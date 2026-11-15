pub mod archive;
pub mod bandcamp;
pub mod local;
pub mod soulseek;
pub mod ytdlp;

use crate::models::Track;
use crate::store::Store;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::warn;

#[async_trait]
pub trait MusicSource: Send + Sync {
    fn name(&self) -> &'static str;
    fn available(&self) -> bool;
    fn status(&self) -> String;
    async fn search(&self, query: &str, limit: usize) -> Vec<Track>;
    async fn resolve_stream(&self, _stream_ref: &str) -> Option<String> {
        None
    }
}

pub struct SourceRegistry {
    sources: Vec<Arc<dyn MusicSource>>,
    store: Store,
}

impl SourceRegistry {
    pub fn new(store: Store, sources: Vec<Arc<dyn MusicSource>>) -> Self {
        Self { sources, store }
    }

    pub fn all(&self) -> &[Arc<dyn MusicSource>] {
        &self.sources
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn MusicSource>> {
        self.sources.iter().find(|s| s.name() == name).cloned()
    }

    /// Searches all available sources in parallel with a soft deadline (5.5s).
    /// Sources that finish later still feed into the SQLite catalog in the background.
    /// Results are deduplicated by `song_key`, keeping the HIGHEST quality version!
    pub async fn search_all(&self, query: &str, limit: usize) -> Vec<Track> {
        self.search_all_inner(query, limit, true).await
    }

    /// Searches all available sources in parallel without automatically inserting
    /// unverified results into SQLite.
    pub async fn search_all_raw(&self, query: &str, limit: usize) -> Vec<Track> {
        self.search_all_inner(query, limit, false).await
    }

    async fn search_all_inner(&self, query: &str, limit: usize, upsert: bool) -> Vec<Track> {
        let query = query.trim().to_string();
        if query.is_empty() || limit == 0 {
            return vec![];
        }

        let avail: Vec<Arc<dyn MusicSource>> = self
            .sources
            .iter()
            .filter(|s| s.available())
            .cloned()
            .collect();

        if avail.is_empty() {
            return vec![];
        }

        let (tx, mut rx) = mpsc::channel::<Vec<Track>>(avail.len());
        let store_bg = self.store.clone();

        for src in &avail {
            let src = src.clone();
            let q = query.clone();
            let tx = tx.clone();
            let store_clone = store_bg.clone();
            tokio::spawn(async move {
                let res = tokio::time::timeout(
                    Duration::from_secs(25),
                    src.search(&q, limit),
                )
                .await;
                match res {
                    Ok(tracks) => {
                        if upsert && !tracks.is_empty() {
                            store_clone.upsert_tracks(&tracks);
                        }
                        let _ = tx.send(tracks).await;
                    }
                    Err(_) => {
                        warn!(source = src.name(), query = %q, "source search timed out");
                        let _ = tx.send(vec![]).await;
                    }
                }
            });
        }
        drop(tx);

        let mut collected = Vec::new();
        let deadline = tokio::time::sleep(Duration::from_millis(5500));
        tokio::pin!(deadline);

        loop {
            tokio::select! {
                maybe = rx.recv() => {
                    match maybe {
                        Some(batch) => collected.extend(batch),
                        None => break,
                    }
                }
                _ = &mut deadline => {
                    if upsert {
                        // Drain any remaining late results into the store in the background
                        tokio::spawn(async move {
                            while let Some(batch) = rx.recv().await {
                                if !batch.is_empty() {
                                    store_bg.upsert_tracks(&batch);
                                }
                            }
                        });
                    }
                    break;
                }
            }
        }

        dedup_and_rank_search(collected, &query, limit)
    }

    pub async fn resolve_stream(&self, source_name: &str, stream_ref: &str) -> Option<String> {
        let src = self.get(source_name)?;
        src.resolve_stream(stream_ref).await
    }
}

/// Deduplicates tracks sharing the same `song_key()`, keeping the highest `quality_score()`,
/// then ranks by query relevance + quality score.
pub fn dedup_and_rank_search(tracks: Vec<Track>, query: &str, limit: usize) -> Vec<Track> {
    let mut best_by_key: HashMap<String, Track> = HashMap::new();
    for t in tracks {
        let key = t.song_key();
        match best_by_key.get(&key) {
            Some(existing) if existing.quality_score() >= t.quality_score() => {}
            _ => {
                best_by_key.insert(key, t);
            }
        }
    }
    let mut out: Vec<Track> = best_by_key.into_values().collect();
    let q = query.trim();
    if q.is_empty() {
        out.sort_by_key(|a| std::cmp::Reverse(a.quality_score()));
    } else {
        out.sort_by_key(|a| {
            std::cmp::Reverse(crate::store::search_match_score(a, q) * 10000 + a.quality_score())
        });
    }
    out.truncate(limit);
    out
}
