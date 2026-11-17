//! Background quality hunter.
//!
//! Slowly walks the catalog looking for higher-quality versions of tracks
//! (currently via Soulseek FLAC search), then installs the better file under
//! the *original* track ID. Artwork and all other metadata stay untouched —
//! only the audio bytes and their technical fields change — so every existing
//! playlist, queue and mix immediately plays the upgraded file.

use crate::engine::StreamEngine;
use crate::models::{primary_artist, QualityTier, Track};
use crate::sources::MusicSource;
use crate::store::Store;
use chrono::Utc;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Semaphore;
use tracing::{info, warn};

/// Counters surfaced on the admin panel so an operator can see what the
/// background hunter is actually doing.
#[derive(Default)]
struct HunterStats {
    probes: AtomicU64,
    candidates: AtomicU64,
    downloads: AtomicU64,
    installed: AtomicU64,
    failures: AtomicU64,
    bytes_upgraded: AtomicU64,
    last_tick: AtomicI64,
    last_upgrade: AtomicI64,
    last_track: Mutex<String>,
}

impl HunterStats {
    fn inc(counter: &AtomicU64) {
        counter.fetch_add(1, Ordering::Relaxed);
    }
}

const DEFAULT_COOLDOWN_SECS: i64 = 6 * 3600;
const DEFAULT_INTERVAL_SECS: usize = 300;
const DEFAULT_BATCH: usize = 12;
const SEARCH_TIMEOUT_MS: u64 = 5000;
const DOWNLOAD_TIMEOUT_SECS: u64 = 900;

fn env_bool(name: &str, default: bool) -> bool {
    match std::env::var(name) {
        Ok(v) => matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"),
        Err(_) => default,
    }
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|v| *v > 0)
        .unwrap_or(default)
}

/// Compares only the properties of the audio file itself. `quality_score()`
/// includes a source bonus (Soulseek > YTM), which would make an installed
/// upgrade look "better" than itself and cause endless re-hunts.
fn audio_quality(t: &Track) -> i32 {
    let tier = match t.quality_tier() {
        QualityTier::HiRes => 1000,
        QualityTier::Lossless => 800,
        QualityTier::HighLossy => 500,
        QualityTier::Standard => 300,
        QualityTier::Low => 100,
    };
    tier
        + t.bitrate.clamp(0, 1500) / 10
        + (t.bit_depth.clamp(0, 32)) * 2
        + (t.sample_rate.clamp(0, 192000) / 1000)
}

/// Picks the best matching candidate. Matching requires the same canonical
/// song key, no conflicting artist, and a close duration so remixes, edits and
/// live versions are not silently substituted.
pub(crate) fn choose_upgrade_candidate(base: &Track, hits: Vec<Track>) -> Option<Track> {
    let base_quality = audio_quality(base);
    let mut best: Option<(i32, Track)> = None;
    for mut hit in hits {
        if hit.id == base.id || audio_quality(&hit) <= base_quality {
            continue;
        }
        if hit.song_key() != base.song_key() {
            continue;
        }
        if crate::models::tracks_conflict_artist(base, &hit) {
            continue;
        }
        if base.duration > 0 && hit.duration > 0 && (base.duration - hit.duration).abs() > 20 {
            continue;
        }

        // The higher-quality file usually has bare filename tags. Reuse the
        // lower-quality track's art and metadata wholesale.
        crate::taste::enrich_slsk_hit_with_base(&mut hit, base, false);

        let score = audio_quality(&hit);
        if best.as_ref().map_or(true, |(s, _)| score > *s) {
            best = Some((score, hit));
        }
    }
    best.map(|(_, t)| t)
}

#[derive(Clone)]
pub struct QualityHunter {
    store: Store,
    engine: StreamEngine,
    checked: Arc<Mutex<HashMap<String, i64>>>,
    permit: Arc<Semaphore>,
    enabled: bool,
    batch: usize,
    cooldown_secs: i64,
    stats: Arc<HunterStats>,
}

impl QualityHunter {
    pub fn new(store: Store, engine: StreamEngine) -> Self {
        let cooldown_secs = env_usize("BEATLAB_QUALITY_HUNTER_COOLDOWN_SECS", DEFAULT_COOLDOWN_SECS as usize) as i64;
        Self {
            store,
            engine,
            checked: Arc::new(Mutex::new(HashMap::new())),
            permit: Arc::new(Semaphore::new(1)),
            enabled: env_bool("BEATLAB_QUALITY_HUNTER", true),
            batch: env_usize("BEATLAB_QUALITY_HUNTER_BATCH", DEFAULT_BATCH),
            cooldown_secs,
            stats: Arc::new(HunterStats::default()),
        }
    }

    /// Snapshot of hunt activity for the admin panel.
    pub fn stats_json(&self) -> serde_json::Value {
        let s = &self.stats;
        serde_json::json!({
            "enabled": self.enabled,
            "intervalSecs": self.interval().as_secs(),
            "batch": self.batch,
            "cooldownSecs": self.cooldown_secs,
            "probes": s.probes.load(Ordering::Relaxed),
            "candidates": s.candidates.load(Ordering::Relaxed),
            "downloads": s.downloads.load(Ordering::Relaxed),
            "installed": s.installed.load(Ordering::Relaxed),
            "failures": s.failures.load(Ordering::Relaxed),
            "bytesUpgraded": s.bytes_upgraded.load(Ordering::Relaxed),
            "lastTick": s.last_tick.load(Ordering::Relaxed),
            "lastUpgrade": s.last_upgrade.load(Ordering::Relaxed),
            "lastTrack": s.last_track.lock().unwrap_or_else(|p| p.into_inner()).clone(),
        })
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn interval(&self) -> Duration {
        Duration::from_secs(env_usize(
            "BEATLAB_QUALITY_HUNTER_INTERVAL_SECS",
            DEFAULT_INTERVAL_SECS,
        ) as u64)

    }

    fn should_probe(&self, key: &str) -> bool {
        let now = Utc::now().timestamp();
        let mut guard = self.checked.lock().unwrap_or_else(|p| p.into_inner());
        if guard.len() > 4096 {
            guard.retain(|_, ts| now - *ts < self.cooldown_secs);
        }
        if let Some(&ts) = guard.get(key) {
            if now - ts < self.cooldown_secs {
                return false;
            }
        }
        guard.insert(key.to_string(), now);
        true
    }

    async fn hunt_one(&self, base: Track) {
        // HiRes is the ceiling; there is nothing better to hunt for.
        if base.quality_tier() == QualityTier::HiRes || base.duration <= 0 {
            return;
        }
        if !self.engine.sources.soulseek.available_or_connecting() {
            return;
        }
        if !self.should_probe(&base.id) {
            return;
        }
        HunterStats::inc(&self.stats.probes);

        let query = format!("{} {} flac", primary_artist(&base.artist), base.title);
        let hits = match tokio::time::timeout(
            Duration::from_millis(SEARCH_TIMEOUT_MS),
            self.engine.sources.soulseek.search(&query, 25),
        )
        .await
        {
            Ok(v) => v,
            Err(_) => return,
        };
        let Some(candidate) = choose_upgrade_candidate(&base, hits) else {
            return;
        };
        HunterStats::inc(&self.stats.candidates);

        let resolved = match tokio::time::timeout(
            Duration::from_secs(DOWNLOAD_TIMEOUT_SECS),
            self.engine.sources.soulseek.resolve_track(&candidate),
        )
        .await
        {
            Ok(Some(r)) => r,
            _ => return,
        };
        let Some(path) = resolved.strip_prefix("file:") else {
            self.stats.failures.fetch_add(1, Ordering::Relaxed);
            return;
        };
        let path = PathBuf::from(path);
        if !path.exists() {
            self.stats.failures.fetch_add(1, Ordering::Relaxed);
            return;
        }
        HunterStats::inc(&self.stats.downloads);

        // The catalog row may have changed while the download was running.
        let fresh = match self.store.get_track(&base.id) {
            Ok(Some(t)) => t,
            _ => return,
        };
        if audio_quality(&candidate) <= audio_quality(&fresh) {
            return;
        }

        match self
            .engine
            .install_quality_upgrade(&fresh.id, &path, &candidate)
            .await
        {
            Ok(()) => {
                let bytes = tokio::fs::metadata(&path)
                    .await
                    .map(|m| m.len())
                    .unwrap_or(0);
                self.stats.bytes_upgraded.fetch_add(bytes, Ordering::Relaxed);
                self.stats.installed.fetch_add(1, Ordering::Relaxed);
                self.stats.last_upgrade.store(Utc::now().timestamp(), Ordering::Relaxed);
                *self.stats.last_track.lock().unwrap_or_else(|p| p.into_inner()) =
                    format!("{} — {}", primary_artist(&fresh.artist), fresh.title);
                info!(track_id = %fresh.id, "background quality upgrade installed");
            }
            Err(e) => {
                self.stats.failures.fetch_add(1, Ordering::Relaxed);
                warn!(track_id = %fresh.id, error = %e, "quality upgrade install failed");
            }
        }
    }

    /// One gentle pass over a rotating slice of the catalog. Only one pass runs
    /// at a time, and tracks are processed sequentially to avoid hammering
    /// peers.
    pub async fn run_tick(&self) {
        let Ok(_permit) = self.permit.clone().try_acquire_owned() else {
            return;
        };
        let Ok(tracks) = self.store.all_tracks(self.batch, self.rotating_offset()) else {
            return;
        };
        self.stats.last_tick.store(Utc::now().timestamp(), Ordering::Relaxed);
        for track in tracks {
            self.hunt_one(track).await;
            tokio::task::yield_now().await;
        }
    }

    fn rotating_offset(&self) -> usize {
        let bucket = Utc::now().timestamp().max(0) as usize / self.interval().as_secs() as usize;
        bucket.saturating_mul(self.batch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Track;

    fn track(id: &str, artist: &str, title: &str, format: &str, bitrate: i32, duration: i32) -> Track {
        Track {
            id: id.into(),
            source: "ytm".into(),
            artist: artist.into(),
            title: title.into(),
            format: format.into(),
            bitrate,
            duration,
            cover_url: "https://img.example/base.jpg".into(),
            meta: r#"{"deezer_track_id":"123"}"#.into(),
            ..Default::default()
        }
    }

    #[test]
    fn picks_better_quality_and_keeps_base_metadata() {
        let base = track("ytm:1", "Porter Robinson", "Shelter", "m4a", 256, 240);
        let mut weaker = track("slsk:weak", "Porter Robinson", "Shelter", "mp3", 128, 240);
        weaker.source = "soulseek".into();
        let mut better = track("slsk:flac", "Porter Robinson", "Shelter", "flac", 900, 241);
        better.source = "soulseek".into();
        better.cover_url.clear();
        better.meta.clear();

        let chosen = choose_upgrade_candidate(&base, vec![weaker, better]).unwrap();
        assert_eq!(chosen.id, "slsk:flac");
        assert_eq!(chosen.format, "flac");
        assert_eq!(chosen.cover_url, base.cover_url);
        assert_eq!(chosen.meta, base.meta);
    }

    #[test]
    fn rejects_wrong_song_artist_and_long_duration_drift() {
        let base = track("ytm:1", "Porter Robinson", "Shelter", "m4a", 256, 240);
        let mut other_song = track("slsk:2", "Porter Robinson", "Different Song", "flac", 900, 240);
        other_song.source = "soulseek".into();
        let mut other_artist = track("slsk:3", "Somebody Else", "Shelter", "flac", 900, 240);
        other_artist.source = "soulseek".into();
        let mut long_remix = track("slsk:4", "Porter Robinson", "Shelter", "flac", 900, 600);
        long_remix.source = "soulseek".into();

        assert!(choose_upgrade_candidate(
            &base,
            vec![other_song, other_artist, long_remix]
        )
        .is_none());
    }

    #[test]
    fn ignores_upgrades_that_only_improve_the_source_bonus() {
        let base = track("ytm:1", "Porter Robinson", "Shelter", "m4a", 256, 240);
        let mut same_quality = track("slsk:5", "Porter Robinson", "Shelter", "m4a", 256, 240);
        same_quality.source = "soulseek".into();

        assert!(choose_upgrade_candidate(&base, vec![same_quality]).is_none());
    }
}
