pub mod archive;
pub mod deezer;
pub mod local;
pub mod soulseek;
pub mod ytm;

use crate::models::{AlbumSummary, ArtistSummary, Track};
use async_trait::async_trait;
use futures::future::join_all;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use archive::ArchiveSource;
use deezer::DeezerSource;
use local::LocalSource;
use soulseek::SoulseekSource;
use ytm::YtmSource;

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

#[derive(Clone, Serialize)]
pub struct SourceStatus {
    pub name: String,
    pub available: bool,
    pub status: String,
}

const SEARCH_CACHE_TTL: Duration = Duration::from_secs(90);
const SEARCH_CACHE_MAX_ENTRIES: usize = 128;

#[derive(Clone)]
struct CachedTrackSearch {
    created: Instant,
    tracks: Vec<Track>,
    ytm_hints: HashMap<String, String>,
}

#[derive(Clone)]
struct CachedCatalogSearch {
    created: Instant,
    albums: Vec<AlbumSummary>,
    artists: Vec<ArtistSummary>,
}

#[derive(Clone)]
pub struct SourceRegistry {
    pub deezer: DeezerSource,
    pub ytm: YtmSource,
    pub archive: ArchiveSource,
    pub local: Arc<LocalSource>,
    pub soulseek: Arc<SoulseekSource>,
    track_cache: Arc<Mutex<HashMap<String, CachedTrackSearch>>>,
    catalog_cache: Arc<Mutex<HashMap<String, CachedCatalogSearch>>>,
}

impl SourceRegistry {
    pub fn new(
        deezer: DeezerSource,
        ytm: YtmSource,
        archive: ArchiveSource,
        local: Arc<LocalSource>,
        soulseek: Arc<SoulseekSource>,
    ) -> Self {
        Self {
            deezer,
            ytm,
            archive,
            local,
            soulseek,
            track_cache: Arc::new(Mutex::new(HashMap::new())),
            catalog_cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Curated playlists matching `query` for building topic mixes ("anime", "80s hits").
    /// Deezer's playlists are preferred because their tracks carry genre metadata, but Deezer
    /// can be unreachable (it blocks some server IPs), so YouTube Music's own editorial
    /// playlists are fetched alongside and used whenever Deezer comes back empty.
    pub async fn fetch_curated_mix_tracks(
        &self,
        query: &str,
        max_playlists: usize,
        per_playlist: usize,
    ) -> Vec<(String, Vec<Track>)> {
        let dz_fut = async {
            if self.deezer.is_blocked() {
                return Vec::new();
            }
            tokio::time::timeout(
                Duration::from_secs(3),
                self.deezer
                    .fetch_curated_mix_tracks(query, max_playlists, per_playlist),
            )
            .await
            .unwrap_or_default()
        };
        let ytm_fut = self.ytm_curated_mix_tracks(query, max_playlists, per_playlist);
        let (dz, ytm) = tokio::join!(dz_fut, ytm_fut);
        if !dz.is_empty() {
            dz
        } else {
            ytm
        }
    }

    async fn ytm_curated_mix_tracks(
        &self,
        query: &str,
        max_playlists: usize,
        per_playlist: usize,
    ) -> Vec<(String, Vec<Track>)> {
        let q_low = query.trim().to_lowercase();
        if q_low.is_empty() {
            return vec![];
        }
        // `max_extra`: how many title words beyond the query a playlist may carry
        // ("Uplifting Anime Songs" is fine for "anime", "Asha Bhosle: 80s Hits" is not
        // for "80s hits"). User-made playlists must match the query outright.
        let pick = |hits: Vec<ytm::YtmPlaylistHit>, max_extra: usize| {
            let mut scored: Vec<(i32, ytm::YtmPlaylistHit)> = hits
                .into_iter()
                .filter_map(|h| {
                    if has_unrequested_scene_qualifier(&h.title, &q_low) {
                        return None;
                    }
                    let extra = extra_title_words(&h.title, &q_low);
                    if extra > max_extra {
                        return None;
                    }
                    let s = deezer::score_mix_playlist(&h.title, h.song_count, &q_low)?;
                    Some((s - 6 * extra as i32, h))
                })
                .collect();
            scored.sort_by(|a, b| b.0.cmp(&a.0));
            let mut seen_titles = HashSet::new();
            scored
                .into_iter()
                .map(|(_, h)| h)
                .filter(|h| seen_titles.insert(h.title.trim().to_lowercase()))
                .take(max_playlists.max(1))
                .collect::<Vec<_>>()
        };
        let mut chosen = pick(self.ytm.search_playlists(query, true, 20).await, 1);
        if chosen.is_empty() {
            chosen = pick(self.ytm.search_playlists(query, false, 20).await, 0);
        }
        if chosen.is_empty() {
            return vec![];
        }
        tracing::info!(
            query,
            playlists = ?chosen.iter().map(|h| h.title.as_str()).collect::<Vec<_>>(),
            "using youtube music playlists for curated mix"
        );
        let batches = join_all(
            chosen
                .iter()
                .map(|h| self.ytm.fetch_playlist_tracks(&h.playlist_id, per_playlist)),
        )
        .await;
        chosen
            .into_iter()
            .zip(batches)
            .filter(|(_, tracks)| !tracks.is_empty())
            .map(|(h, tracks)| (h.title, tracks))
            .collect()
    }

    pub fn sources_info(&self) -> Vec<SourceStatus> {
        let all: [&dyn MusicSource; 5] = [
            &*self.local as &dyn MusicSource,
            &self.deezer,
            &self.ytm,
            &self.archive,
            &*self.soulseek,
        ];
        all.into_iter()
            .map(|s| SourceStatus {
                name: s.name().to_string(),
                available: s.available(),
                status: s.status(),
            })
            .collect()
    }

    /// Queries all active sources concurrently and returns a deduplicated, quality-ranked list.
    pub async fn search_all(&self, query: &str, per_source: usize) -> Vec<Track> {
        self.search_all_with_ytm_hints(query, per_source).await.0
    }

    /// Queries all active sources concurrently and returns a deduplicated, quality-ranked list
    /// alongside a `song_key -> ytm_video_id` map captured prior to cross-source quality dedup.
    pub async fn search_all_with_ytm_hints(
        &self,
        query: &str,
        per_source: usize,
    ) -> (Vec<Track>, HashMap<String, String>) {
        let q = query.trim();
        if q.is_empty() {
            return (vec![], HashMap::new());
        }
        let cache_key = format!("all:{}:{}", per_source, q.to_lowercase());
        if let Some(hit) = self.get_cached_tracks(&cache_key) {
            return hit;
        }
        let mut sources: Vec<&dyn MusicSource> = vec![
            &*self.local,
            &self.deezer,
            &self.ytm,
            &self.archive,
        ];
        if self.soulseek.available_or_connecting() {
            sources.push(&*self.soulseek);
        }
        let res = self.run_source_search(sources, q, per_source).await;
        self.put_cached_tracks(cache_key, res.0.clone(), res.1.clone());
        res
    }

    /// Queries only low-latency sources (`local`, `deezer`, `ytm`) so search UI renders immediately.
    pub async fn search_fast_with_ytm_hints(
        &self,
        query: &str,
        per_source: usize,
    ) -> (Vec<Track>, HashMap<String, String>) {
        let q = query.trim();
        if q.is_empty() {
            return (vec![], HashMap::new());
        }
        let cache_key = format!("fast:{}:{}", per_source, q.to_lowercase());
        if let Some(hit) = self.get_cached_tracks(&cache_key) {
            return hit;
        }
        let sources: Vec<&dyn MusicSource> = vec![&*self.local, &self.deezer, &self.ytm];
        let res = self.run_source_search(sources, q, per_source).await;
        self.put_cached_tracks(cache_key, res.0.clone(), res.1.clone());
        res
    }

    /// Queries slower/lossless-heavy sources (`archive`, `soulseek`) in a second phase.
    pub async fn search_slow(&self, query: &str, per_source: usize) -> Vec<Track> {
        let q = query.trim();
        if q.is_empty() {
            return vec![];
        }
        let cache_key = format!("slow:{}:{}", per_source, q.to_lowercase());
        if let Some((tracks, _)) = self.get_cached_tracks(&cache_key) {
            return tracks;
        }
        let mut sources: Vec<&dyn MusicSource> = vec![&self.archive];
        if self.soulseek.available_or_connecting() {
            sources.push(&*self.soulseek);
        }
        let (tracks, hints) = self.run_source_search(sources, q, per_source).await;
        self.put_cached_tracks(cache_key, tracks.clone(), hints);
        tracks
    }

    /// Cached Deezer album & artist search for fast repeated/back-button queries.
    pub async fn search_deezer_catalog(
        &self,
        query: &str,
        limit: usize,
    ) -> (Vec<AlbumSummary>, Vec<ArtistSummary>) {
        let q = query.trim();
        if q.is_empty() {
            return (vec![], vec![]);
        }
        let cache_key = format!("{}:{}", limit, q.to_lowercase());
        if let Ok(guard) = self.catalog_cache.lock() {
            if let Some(entry) = guard.get(&cache_key) {
                if entry.created.elapsed() < SEARCH_CACHE_TTL {
                    return (entry.albums.clone(), entry.artists.clone());
                }
            }
        }
        let (albums, artists) = tokio::join!(
            tokio::time::timeout(
                Duration::from_millis(3500),
                self.deezer.search_albums(q, limit),
            ),
            tokio::time::timeout(
                Duration::from_millis(3500),
                self.deezer.search_artists(q, limit),
            ),
        );
        let albums = albums.unwrap_or_default();
        let artists = artists.unwrap_or_default();
        if let Ok(mut guard) = self.catalog_cache.lock() {
            if guard.len() >= SEARCH_CACHE_MAX_ENTRIES {
                guard.retain(|_, v| v.created.elapsed() < SEARCH_CACHE_TTL);
                if guard.len() >= SEARCH_CACHE_MAX_ENTRIES {
                    guard.clear();
                }
            }
            guard.insert(
                cache_key,
                CachedCatalogSearch {
                    created: Instant::now(),
                    albums: albums.clone(),
                    artists: artists.clone(),
                },
            );
        }
        (albums, artists)
    }

    async fn run_source_search(
        &self,
        sources: Vec<&dyn MusicSource>,
        q: &str,
        per_source: usize,
    ) -> (Vec<Track>, HashMap<String, String>) {
        let futs = sources.into_iter().map(|s| {
            let per_src_timeout = match s.name() {
                "local" => Duration::from_millis(1500),
                "archive" | "soulseek" => Duration::from_millis(3500),
                _ => Duration::from_millis(4000),
            };
            async move {
                tokio::time::timeout(per_src_timeout, s.search(q, per_source))
                    .await
                    .unwrap_or_default()
            }
        });
        let batches = join_all(futs).await;
        let combined: Vec<Track> = batches
            .into_iter()
            .flatten()
            .filter(|t| !t.stream_url.contains("/preview/"))
            .collect();
        let mut ytm_ids_by_key = HashMap::new();
        for t in &combined {
            if t.source == "ytm" || t.id.starts_with("ytm:") {
                let vid = t.id.trim_start_matches("ytm:");
                if !vid.is_empty() {
                    ytm_ids_by_key
                        .entry(t.song_key())
                        .or_insert_with(|| vid.to_string());
                }
            }
        }
        (dedup_and_rank_search(combined, q), ytm_ids_by_key)
    }

    fn get_cached_tracks(&self, key: &str) -> Option<(Vec<Track>, HashMap<String, String>)> {
        let guard = self.track_cache.lock().ok()?;
        let entry = guard.get(key)?;
        if entry.created.elapsed() < SEARCH_CACHE_TTL {
            Some((entry.tracks.clone(), entry.ytm_hints.clone()))
        } else {
            None
        }
    }

    fn put_cached_tracks(
        &self,
        key: String,
        tracks: Vec<Track>,
        ytm_hints: HashMap<String, String>,
    ) {
        if let Ok(mut guard) = self.track_cache.lock() {
            if guard.len() >= SEARCH_CACHE_MAX_ENTRIES {
                guard.retain(|_, v| v.created.elapsed() < SEARCH_CACHE_TTL);
                if guard.len() >= SEARCH_CACHE_MAX_ENTRIES {
                    guard.clear();
                }
            }
            guard.insert(
                key,
                CachedTrackSearch {
                    created: Instant::now(),
                    tracks,
                    ytm_hints,
                },
            );
        }
    }
}

/// Picks the higher-quality track between `a` and `b` while preserving missing metadata
/// (artwork, genre, moods, year, Deezer ID, popularity metadata, album).
pub fn merge_track_pair(a: Track, b: Track) -> Track {
    let (mut winner, loser) = if a.quality_score() > b.quality_score() {
        (a, b)
    } else {
        (b, a)
    };
    if winner.cover_url.is_empty() && !loser.cover_url.is_empty() {
        winner.cover_url = loser.cover_url;
    }
    if winner.genre.is_empty() && !loser.genre.is_empty() {
        winner.genre = loser.genre;
    }
    if winner.moods.is_empty() && !loser.moods.is_empty() {
        winner.moods = loser.moods;
    }
    if winner.year == 0 && loser.year > 0 {
        winner.year = loser.year;
    }
    if winner.deezer_id.is_empty() && !loser.deezer_id.is_empty() {
        winner.deezer_id = loser.deezer_id;
    }
    winner.meta = crate::models::merge_meta_json(&winner.meta, &loser.meta);
    if (winner.album.is_empty() || winner.album == winner.title) && !loser.album.is_empty() {
        winner.album = loser.album;
    }
    winner
}

/// Propagates `deezer_artist_id` across a batch of tracks using:
/// 1) `ytm_artist_id -> deezer_artist_id` learned from cross-source merged tracks,
/// 2) unambiguous exact case-sensitive artist display name (`"LiSA"` vs `"LISA"`),
/// 3) unambiguous case-insensitive artist slug (only when a single Deezer artist has that slug).
pub fn disambiguate_batch_tracks(
    tracks: &mut [Track],
    known_artists: &[crate::models::ArtistSummary],
) {
    let mut ytm_to_dz: HashMap<String, Option<String>> = HashMap::new();
    let mut exact_to_dz: HashMap<String, Option<String>> = HashMap::new();
    let mut slug_to_dz: HashMap<String, Option<String>> = HashMap::new();

    let mut record_mapping = |name: &str, dz_id: &str| {
        let pa = crate::models::primary_artist(name);
        if pa.is_empty() || dz_id.is_empty() || dz_id == "0" {
            return;
        }
        exact_to_dz
            .entry(pa.clone())
            .and_modify(|cur| {
                if cur.as_deref() != Some(dz_id) {
                    *cur = None;
                }
            })
            .or_insert_with(|| Some(dz_id.to_string()));

        let sl = crate::models::slug(&pa);
        if !sl.is_empty() {
            slug_to_dz
                .entry(sl)
                .and_modify(|cur| {
                    if cur.as_deref() != Some(dz_id) {
                        *cur = None;
                    }
                })
                .or_insert_with(|| Some(dz_id.to_string()));
        }
    };

    for ar in known_artists {
        if !ar.deezer_id.is_empty() {
            record_mapping(&ar.name, &ar.deezer_id);
        }
    }

    for t in tracks.iter() {
        if let Some(dz_id) = t.deezer_artist_id() {
            record_mapping(&t.artist, &dz_id);
            if let Some(ytm_id) = t.ytm_artist_id() {
                ytm_to_dz
                    .entry(ytm_id)
                    .and_modify(|cur| {
                        if cur.as_deref() != Some(dz_id.as_str()) {
                            *cur = None;
                        }
                    })
                    .or_insert_with(|| Some(dz_id));
            }
        }
    }

    for t in tracks.iter_mut() {
        if t.deezer_artist_id().is_some() {
            continue;
        }
        if let Some(ytm_id) = t.ytm_artist_id() {
            if let Some(Some(dz_id)) = ytm_to_dz.get(&ytm_id) {
                t.set_deezer_artist_id(dz_id);
                continue;
            }
        }
        let pa = crate::models::primary_artist(&t.artist);
        if pa.is_empty() {
            continue;
        }
        if let Some(Some(dz_id)) = exact_to_dz.get(&pa) {
            t.set_deezer_artist_id(dz_id);
            continue;
        }
        let sl = crate::models::slug(&pa);
        if let Some(Some(dz_id)) = slug_to_dz.get(&sl) {
            t.set_deezer_artist_id(dz_id);
        }
    }
}

/// Deduplicate across sources by canonical `(artist, core_title)` keeping highest quality
/// and clean metadata, while never merging tracks from conflicting external artist IDs.
pub fn dedup_tracks_prefer_quality(tracks: Vec<Track>) -> Vec<Track> {
    let mut index_by_key: HashMap<String, Vec<usize>> = HashMap::new();
    let mut out: Vec<Track> = Vec::with_capacity(tracks.len());
    for t in tracks {
        let key = t.song_key();
        if let Some(indices) = index_by_key.get(&key) {
            if let Some(&idx) = indices
                .iter()
                .find(|&&i| !crate::models::tracks_conflict_artist(&out[i], &t))
            {
                let existing = std::mem::take(&mut out[idx]);
                out[idx] = merge_track_pair(t, existing);
                continue;
            }
        }
        let new_idx = out.len();
        index_by_key.entry(key).or_default().push(new_idx);
        out.push(t);
    }
    disambiguate_batch_tracks(&mut out, &[]);
    out
}

/// Deduplicate across sources by canonical `(artist, core_title)` keeping highest quality
/// and clean metadata, while sorting by relevance + quality.
pub fn dedup_and_rank_search(tracks: Vec<Track>, query: &str) -> Vec<Track> {
    let deduped = dedup_tracks_prefer_quality(tracks);
    let mut ranked: Vec<(i32, Track)> = deduped
        .into_iter()
        .map(|t| {
            let s = crate::store::search_match_score(&t, query);
            let rel = if s > 0 { s } else { t.quality_score() / 20 };
            (rel, t)
        })
        .collect();
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.quality_score().cmp(&a.1.quality_score())));
    ranked.into_iter().map(|(_, t)| t).collect()
}

/// YouTube Music and Deezer playlist searches are global, so a search for "80s hits" or
/// "dance party" also returns "The Turkish Hits: 80s", "Egyptian '80s", or "Malayalam Dance Party".
/// Reject playlists scoped to a regional or language scene the query did not ask for.
pub(crate) fn has_unrequested_scene_qualifier(title: &str, query_low: &str) -> bool {
    const SCENES: &[&str] = &[
        "arabic", "khaleeji", "egyptian", "turkish", "persian", "kurdish", "bollywood",
        "tollywood", "kollywood", "mollywood", "sandalwood", "hindi", "punjabi", "tamil",
        "telugu", "malayalam", "marathi", "kannada", "bengali", "gujarati", "assamese",
        "odia", "bhojpuri", "haryanvi", "desi", "bhangra", "garba", "dandiya", "kuthu",
        "dappankuthu", "filmi", "qawwali", "carnatic", "hindustani", "malayali", "mallu",
        "kerala", "maharashtra", "indonesian", "malaysian", "thai", "vietnamese", "filipino",
        "opm", "mandopop", "cantopop", "chinese", "korean", "japanese", "nigeria", "nigerian",
        "ghanaian", "naija", "argentinian", "mexican", "brazilian", "colombian", "latin",
        "latino", "spanish", "español", "éxitos", "sertanejo", "forró", "forro", "pagode",
        "brega", "corridos", "banda", "norteño", "norteno", "ranchera", "vallenato", "dembow",
        "champeta", "french", "german", "schlager", "volksmusik", "italian", "flamenco",
        "polish", "russian", "ukrainian", "greek", "dutch", "swedish", "chalga", "turbofolk",
        "dabke", "shaabi", "mahraganat", "canrock", "canadian", "australian", "japan", "korea",
        "china", "taiwan", "india", "pakistan", "turkey", "egypt", "iran", "indonesia",
        "malaysia", "thailand", "vietnam", "philippines", "africa", "african", "ghana",
        "kenya", "brazil", "brasil", "mexico", "argentina", "colombia", "france", "germany",
        "italy", "spain", "poland", "russia", "ukraine", "greece", "sweden",
    ];
    let t = title.to_lowercase();
    SCENES.iter().any(|s| {
        crate::taste::contains_term(&t, s) && !crate::taste::contains_term(query_low, s)
    })
}

/// Number of meaningful title words that are not part of the query, used to prefer
/// "The Hits: '80s" over "Relaxing 80s Rock" for the query "80s hits".
pub(crate) fn extra_title_words(title: &str, query_low: &str) -> usize {
    const FILLER: &[&str] = &[
        "the", "of", "and", "a", "an", "in", "for", "to", "hits", "hit", "best", "top",
        "songs", "song", "music", "mix", "playlist", "essentials", "biggest", "greatest", "all",
        "time", "classics", "classic", "anthems", "favorites", "favourites",
    ];
    let q_words: HashSet<String> = query_low
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.trim_end_matches('s').to_string())
        .collect();
    title
        .to_lowercase()
        .replace("'s", "")
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty() && !w.chars().all(|c| c.is_ascii_digit()))
        .filter(|w| !FILLER.contains(w))
        .filter(|w| !q_words.contains(w.trim_end_matches('s')))
        .count()
}
