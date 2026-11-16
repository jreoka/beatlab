use crate::models::Track;
use crate::sources::MusicSource;
use crate::store::Store;
use crate::taste::classify_moods_str;
use async_trait::async_trait;
use lofty::file::{AudioFile, TaggedFile, TaggedFileExt};
use lofty::probe::Probe;
use lofty::tag::{Accessor, ItemKey};
use sha1::{Digest, Sha1};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use tracing::{info, warn};
use walkdir::WalkDir;

const AUDIO_EXTS: &[&str] = &[
    "flac", "alac", "wav", "aiff", "aif", "mp3", "m4a", "ogg", "opus", "wv", "ape",
];

pub struct LocalSource {
    store: Store,
    dirs: Arc<RwLock<Vec<PathBuf>>>,
}

impl LocalSource {
    pub fn new(store: Store, initial_dirs: Vec<PathBuf>) -> Self {
        let mut dirs = initial_dirs;
        if let Some(saved) = store.kv_get("local_dirs") {
            if let Ok(parsed) = serde_json::from_str::<Vec<String>>(&saved) {
                for p in parsed {
                    let pb = PathBuf::from(p);
                    if !dirs.contains(&pb) {
                        dirs.push(pb);
                    }
                }
            }
        }
        Self {
            store,
            dirs: Arc::new(RwLock::new(dirs)),
        }
    }

    pub fn dirs(&self) -> Vec<PathBuf> {
        self.dirs.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Scans all configured directories and upserts tracks into SQLite.
    pub fn scan(&self) -> usize {
        let dirs = self.dirs();
        if dirs.is_empty() {
            return 0;
        }
        let mut count = 0usize;
        let mut batch = Vec::new();
        let mut loudness_batch: Vec<(String, f64, String)> = Vec::new();
        for root in dirs {
            if !root.exists() {
                continue;
            }
            for entry in WalkDir::new(&root).follow_links(true).into_iter().flatten() {
                if !entry.file_type().is_file() {
                    continue;
                }
                let path = entry.path();
                let ext = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.to_lowercase())
                    .unwrap_or_default();
                if !AUDIO_EXTS.contains(&ext.as_str()) {
                    continue;
                }
                if let Some((track, norm_db)) = read_local_track(path, &ext) {
                    if let Some(db) = norm_db {
                        loudness_batch.push((track.id.clone(), db, "tag".to_string()));
                    }
                    batch.push(track);
                    count += 1;
                    if batch.len() >= 100 {
                        if let Err(e) = self.store.upsert_tracks(&batch) {
                            warn!(error = %e, "upsert_tracks batch failed");
                        }
                        if !loudness_batch.is_empty() {
                            let _ = self.store.upsert_track_loudness_batch(&loudness_batch);
                            loudness_batch.clear();
                        }
                        batch.clear();
                    }
                }
            }
        }
        if !batch.is_empty() {
            if let Err(e) = self.store.upsert_tracks(&batch) {
                warn!(error = %e, "upsert_tracks final batch failed");
            }
        }
        if !loudness_batch.is_empty() {
            let _ = self.store.upsert_track_loudness_batch(&loudness_batch);
        }
        info!(tracks = count, "local library scan complete");
        count
    }
}

#[async_trait]
impl MusicSource for LocalSource {
    fn name(&self) -> &'static str {
        "local"
    }

    fn available(&self) -> bool {
        !self.dirs.read().unwrap_or_else(|e| e.into_inner()).is_empty()
    }

    fn status(&self) -> String {
        let dirs = self.dirs();
        if dirs.is_empty() {
            "no local folders configured (set BEATLAB_MUSIC_DIRS)".to_string()
        } else {
            let joined: Vec<String> = dirs.iter().map(|d| d.display().to_string()).collect();
            format!("watching {} folder(s): {}", dirs.len(), joined.join(", "))
        }
    }

    async fn search(&self, query: &str, limit: usize) -> Vec<Track> {
        self.store
            .search_tracks(query, limit * 2, 0)
            .unwrap_or_default()
            .into_iter()
            .filter(|t| t.source == "local")
            .take(limit)
            .collect()
    }
}

fn read_local_track(path: &Path, ext: &str) -> Option<(Track, Option<f64>)> {
    let meta = std::fs::metadata(path).ok()?;
    let size = meta.len() as i64;
    let abs = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let abs_str = abs.to_string_lossy().to_string();

    let mut hasher = Sha1::new();
    hasher.update(abs_str.as_bytes());
    let id = format!("local:{}", hex::encode(&hasher.finalize()[..10]));

    let fallback_title = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Unknown Track")
        .to_string();

    let mut title = fallback_title;
    let mut artist = "Unknown Artist".to_string();
    let mut album = "Local Library".to_string();
    let mut genre = String::new();
    let mut year = 0i32;
    let mut track_no = 0i32;
    let mut disc_no = 1i32;
    let mut duration = 0i32;
    let mut bitrate = 0i32;
    let mut sample_rate = 0i32;
    let mut bit_depth = 0i32;
    let mut has_cover = false;
    let mut norm_gain_db: Option<f64> = None;

    if let Ok(tagged) = Probe::open(path).and_then(|p| p.read()) {
        let props = tagged.properties();
        duration = props.duration().as_secs() as i32;
        bitrate = props.overall_bitrate().unwrap_or(0) as i32;
        sample_rate = props.sample_rate().unwrap_or(0) as i32;
        bit_depth = props.bit_depth().unwrap_or(0) as i32;
        norm_gain_db = extract_loudness_gain_from_tags(&tagged);

        if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
            if let Some(t) = tag.title() {
                if !t.trim().is_empty() {
                    title = t.trim().to_string();
                }
            }
            if let Some(a) = tag.artist() {
                if !a.trim().is_empty() {
                    artist = a.trim().to_string();
                }
            }
            if let Some(al) = tag.album() {
                if !al.trim().is_empty() {
                    album = al.trim().to_string();
                }
            }
            if let Some(g) = tag.genre() {
                genre = g.trim().to_string();
            }
            if let Some(y) = tag.year() {
                year = y as i32;
            }
            if let Some(tn) = tag.track() {
                track_no = tn as i32;
            }
            if let Some(dn) = tag.disk() {
                if dn > 0 {
                    disc_no = dn as i32;
                }
            }
            has_cover = !tag.pictures().is_empty();
        }
    }

    let moods = classify_moods_str(&title, &album, &genre, "");
    let cover_url = if has_cover {
        format!("local-art:{}", abs_str)
    } else {
        String::new()
    };

    Some((
        Track {
            id,
            source: "local".to_string(),
            title,
            artist,
            album,
            genre,
            moods,
            year,
            track_no,
            disc_no,
            duration,
            bitrate,
            sample_rate,
            bit_depth,
            format: ext.to_string(),
            size,
            stream_url: format!("file:{}", abs_str),
            cover_url,
            meta: String::new(),
            deezer_id: String::new(),
            created: chrono::Utc::now().timestamp(),
        },
        norm_gain_db,
    ))
}

pub fn parse_replaygain_db(raw: &str) -> Option<f64> {
    let trimmed = raw
        .trim()
        .trim_end_matches(|c: char| c.is_ascii_alphabetic() || c.is_whitespace());
    let val: f64 = trimmed.trim().parse().ok()?;
    if !val.is_finite() {
        return None;
    }
    // ReplayGain 1.0/2.0 references -18 LUFS; target is -14 LUFS (+4.0 dB offset).
    Some((val + 4.0).clamp(-12.0, 8.5))
}

pub fn parse_r128_track_gain_db(raw: &str) -> Option<f64> {
    let val: f64 = raw.trim().parse().ok()?;
    if !val.is_finite() {
        return None;
    }
    // RFC 7845 R128_TRACK_GAIN is Q7.8 fixed-point relative to -23 LUFS; target is -14 LUFS (+9.0 dB).
    Some(((val / 256.0) + 9.0).clamp(-12.0, 8.5))
}

pub fn parse_itunnorm_db(raw: &str) -> Option<f64> {
    let mut vals = raw
        .split_whitespace()
        .take(2)
        .filter_map(|tok| u32::from_str_radix(tok.trim(), 16).ok())
        .filter(|&v| v > 0);
    let left = vals.next()?;
    let peak_val = vals.next().map(|right| left.max(right)).unwrap_or(left) as f64;
    // Apple SoundCheck: 1000 == 0 dB relative to -18 LUFS; +4 dB adjusts to -14 LUFS.
    let db_18 = -10.0 * (peak_val / 1000.0).log10();
    if !db_18.is_finite() {
        return None;
    }
    Some((db_18 + 4.0).clamp(-12.0, 8.5))
}

pub fn extract_loudness_gain_from_tags(tagged: &TaggedFile) -> Option<f64> {
    for tag in tagged.tags() {
        for item in tag.items() {
            let Some(val) = item.value().text() else {
                continue;
            };
            match item.key() {
                ItemKey::ReplayGainTrackGain => {
                    if let Some(db) = parse_replaygain_db(val) {
                        return Some(db);
                    }
                }
                ItemKey::Unknown(k) => {
                    let key_low = k.to_ascii_lowercase();
                    if key_low == "replaygain_track_gain"
                        || key_low.ends_with(":replaygain_track_gain")
                    {
                        if let Some(db) = parse_replaygain_db(val) {
                            return Some(db);
                        }
                    } else if key_low == "r128_track_gain"
                        || key_low.ends_with(":r128_track_gain")
                    {
                        if let Some(db) = parse_r128_track_gain_db(val) {
                            return Some(db);
                        }
                    } else if key_low == "itunnorm" || key_low.ends_with(":itunnorm") {
                        if let Some(db) = parse_itunnorm_db(val) {
                            return Some(db);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    None
}

pub fn extract_loudness_gain_from_file(path: &Path) -> Option<f64> {
    let tagged = Probe::open(path).ok()?.read().ok()?;
    extract_loudness_gain_from_tags(&tagged)
}

pub fn extract_embedded_cover(file_path: &Path) -> Option<(Vec<u8>, String)> {
    let tagged = Probe::open(file_path).ok()?.read().ok()?;
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag())?;
    let pic = tag.pictures().first()?;
    let mime = pic
        .mime_type()
        .map(|m| m.as_str().to_string())
        .unwrap_or_else(|| "image/jpeg".to_string());
    Some((pic.data().to_vec(), mime))
}
