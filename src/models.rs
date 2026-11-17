use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone)]
pub struct AppError(pub String);

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl<E: std::error::Error> From<E> for AppError {
    fn from(e: E) -> Self {
        Self(e.to_string())
    }
}

pub type AppResult<T> = std::result::Result<T, AppError>;

pub fn err_msg(msg: impl Into<String>) -> AppError {
    AppError(msg.into())
}

/// Sentinel returned by credential checks when the account exists but has been
/// soft-deleted and is still inside its seven-day recovery window.
pub const ERR_RESTORE_REQUIRED: &str = "__beatlab_restore_required__";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: i64,
    pub username: String,
    #[serde(skip_serializing)]
    pub password: String,
    pub created: i64,
}

/// Editable social profile attached to a user. Kept in its own table so the core
/// auth `users` table stays lean and forward-compatible.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserProfile {
    pub user_id: i64,
    pub display_name: String,
    pub bio: String,
    pub avatar_color: String,
    pub location: String,
    pub updated: i64,
}

impl Default for UserProfile {
    fn default() -> Self {
        Self {
            user_id: 0,
            display_name: String::new(),
            bio: String::new(),
            avatar_color: String::new(),
            location: String::new(),
            updated: 0,
        }
    }
}

/// A user as seen by other people on the platform (search results, profiles,
/// follower lists). Aggregates counts and the viewer's follow relationship.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicUser {
    pub id: i64,
    pub username: String,
    pub display_name: String,
    pub bio: String,
    pub avatar_color: String,
    pub avatar_letter: String,
    /// Relative URL of the user's uploaded picture, or empty when they use the
    /// generated initial/color avatar. Includes a cache-busting version query.
    pub avatar_url: String,
    pub location: String,
    pub created: i64,
    pub followers_count: i64,
    pub following_count: i64,
    pub is_following: bool,
    pub is_self: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mood {
    Chill,
    Focus,
    Energy,
    LateNight,
    Upbeat,
    Melancholy,
    Party,
    Acoustic,
    Groove,
    RockIndie,
}

impl Mood {
    pub const ALL: [Mood; 10] = [
        Mood::Chill,
        Mood::Focus,
        Mood::Energy,
        Mood::LateNight,
        Mood::Upbeat,
        Mood::Melancholy,
        Mood::Party,
        Mood::Acoustic,
        Mood::Groove,
        Mood::RockIndie,
    ];

    pub fn slug(&self) -> &'static str {
        match self {
            Mood::Chill => "chill",
            Mood::Focus => "focus",
            Mood::Energy => "energy",
            Mood::LateNight => "late_night",
            Mood::Upbeat => "upbeat",
            Mood::Melancholy => "melancholy",
            Mood::Party => "party",
            Mood::Acoustic => "acoustic",
            Mood::Groove => "groove",
            Mood::RockIndie => "rock_indie",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Mood::Chill => "Chill & Lo-Fi",
            Mood::Focus => "Deep Focus",
            Mood::Energy => "High Energy",
            Mood::LateNight => "Late Night",
            Mood::Upbeat => "Feel Good",
            Mood::Melancholy => "Melancholy",
            Mood::Party => "Party & Dance",
            Mood::Acoustic => "Acoustic",
            Mood::Groove => "Soul & Groove",
            Mood::RockIndie => "Rock & Indie",
        }
    }

    pub fn subtitle(&self) -> &'static str {
        match self {
            Mood::Chill => "Laid-back beats, dream pop & soft vibes",
            Mood::Focus => "Ambient, instrumental & flow-state music",
            Mood::Energy => "Adrenaline, workout & heavy rhythms",
            Mood::LateNight => "Atmospheric, moody & after-hours tracks",
            Mood::Upbeat => "Bright hooks, pop anthems & sunshine",
            Mood::Melancholy => "Reflective ballads & emotional cuts",
            Mood::Party => "Dancefloor, house, disco & club energy",
            Mood::Acoustic => "Unplugged guitars, folk & warm vocals",
            Mood::Groove => "R&B, soul, funk, jazz & hip-hop bounce",
            Mood::RockIndie => "Alt-rock, indie classics & guitars",
        }
    }

    pub fn gradient(&self) -> (&'static str, &'static str) {
        match self {
            Mood::Chill => ("#0ea5e9", "#1e3a8a"),
            Mood::Focus => ("#14b8a6", "#115e59"),
            Mood::Energy => ("#ef4444", "#7f1d1d"),
            Mood::LateNight => ("#6366f1", "#1e1b4b"),
            Mood::Upbeat => ("#f59e0b", "#b45309"),
            Mood::Melancholy => ("#64748b", "#1e293b"),
            Mood::Party => ("#ec4899", "#831843"),
            Mood::Acoustic => ("#84cc16", "#365314"),
            Mood::Groove => ("#a855f7", "#4c1d95"),
            Mood::RockIndie => ("#f97316", "#7c2d12"),
        }
    }

    /// Short, card-friendly name used by the personalized "Mixed for you" mixes.
    pub fn mix_name(&self) -> &'static str {
        match self {
            Mood::Chill => "Chill",
            Mood::Focus => "Focus",
            Mood::Energy => "Energy",
            Mood::LateNight => "Late Night",
            Mood::Upbeat => "Feel Good",
            Mood::Melancholy => "Melancholy",
            Mood::Party => "Party",
            Mood::Acoustic => "Acoustic",
            Mood::Groove => "Groove",
            Mood::RockIndie => "Indie Rock",
        }
    }

    pub fn from_slug(s: &str) -> Option<Mood> {
        let low = s.trim().to_lowercase().replace('-', "_");
        Self::ALL.iter().copied().find(|m| m.slug() == low)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QualityTier {
    Low = 1,
    Standard = 2,
    HighLossy = 3,
    Lossless = 4,
    HiRes = 5,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Track {
    pub id: String,
    pub source: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub genre: String,
    pub moods: String,
    pub year: i32,
    pub track_no: i32,
    pub disc_no: i32,
    pub duration: i32,     // seconds
    pub bitrate: i32,      // kbps
    pub sample_rate: i32,  // Hz
    pub bit_depth: i32,    // bits
    pub format: String,    // flac, wav, alac, mp3, m4a, ogg, opus
    pub size: i64,         // bytes
    pub stream_url: String,
    pub cover_url: String,
    pub meta: String,
    pub deezer_id: String,
    pub created: i64,
}

impl Track {
    pub fn deezer_artist_id(&self) -> Option<String> {
        extract_meta_id_field(&self.meta, "deezer_artist_id")
    }

    pub fn deezer_album_id(&self) -> Option<String> {
        extract_meta_id_field(&self.meta, "deezer_album_id")
    }

    pub fn ytm_artist_id(&self) -> Option<String> {
        extract_meta_id_field(&self.meta, "ytm_artist_id")
    }

    pub fn set_deezer_artist_id(&mut self, dz_artist_id: &str) {
        let clean = dz_artist_id.trim();
        if clean.is_empty() || clean == "0" {
            return;
        }
        let mut obj = parse_meta_object(&self.meta);
        if let Ok(num) = clean.parse::<u64>() {
            obj.insert("deezer_artist_id".to_string(), serde_json::Value::from(num));
        } else {
            obj.insert(
                "deezer_artist_id".to_string(),
                serde_json::Value::String(clean.to_string()),
            );
        }
        if let Ok(s) = serde_json::to_string(&obj) {
            self.meta = s;
        }
    }

    pub fn artist_id(&self) -> String {
        if let Some(dz_aid) = self.deezer_artist_id() {
            return format!("ar:dz:{}", dz_aid);
        }
        if let Some(ytm_aid) = self.ytm_artist_id() {
            return format!("ar:ytm:{}", ytm_aid);
        }
        let pa = primary_artist(&self.artist);
        let name = if pa.is_empty() { &self.artist } else { &pa };
        format!("ar:{}", slug(name))
    }

    pub fn matches_artist_id(&self, want_id: &str) -> bool {
        if self.artist_id() == want_id {
            return true;
        }
        if let Some(dz_aid) = want_id.strip_prefix("ar:dz:") {
            return self.deezer_artist_id().as_deref() == Some(dz_aid);
        }
        if let Some(ytm_aid) = want_id.strip_prefix("ar:ytm:") {
            return self.ytm_artist_id().as_deref() == Some(ytm_aid);
        }
        false
    }

    pub fn album_id(&self) -> String {
        let alb = if self.album.trim().is_empty() {
            &self.title
        } else {
            &self.album
        };
        if let Some(dz_aid) = self.deezer_artist_id() {
            return format!("al:dz-{}-{}", dz_aid, slug(alb));
        }
        if let Some(ytm_aid) = self.ytm_artist_id() {
            return format!("al:ytm-{}-{}", slug(&ytm_aid), slug(alb));
        }
        let pa = primary_artist(&self.artist);
        let art = if pa.is_empty() || is_generic_folder_name(&pa) {
            &self.artist
        } else {
            &pa
        };
        format!("al:{}", slug(&format!("{}|{}", art, alb)))
    }

    pub fn matches_album_id(&self, want_id: &str) -> bool {
        if self.album_id() == want_id {
            return true;
        }
        let alb = if self.album.trim().is_empty() {
            &self.title
        } else {
            &self.album
        };
        let alb_slug = slug(alb);
        if want_id.starts_with("al:dz-") {
            if let Some(dz_aid) = self.deezer_artist_id() {
                if format!("al:dz-{}-{}", dz_aid, alb_slug) == want_id {
                    return true;
                }
            }
        }
        if want_id.starts_with("al:ytm-") {
            if let Some(ytm_aid) = self.ytm_artist_id() {
                if format!("al:ytm-{}-{}", slug(&ytm_aid), alb_slug) == want_id {
                    return true;
                }
            }
        }
        false
    }

    pub fn is_lossless(&self) -> bool {
        matches!(
            self.format.to_lowercase().as_str(),
            "flac" | "alac" | "wav" | "aiff" | "aif" | "ape" | "wv" | "dsf"
        )
    }

    pub fn quality_tier(&self) -> QualityTier {
        if self.is_lossless() {
            if self.bit_depth >= 24 || self.sample_rate > 48000 {
                return QualityTier::HiRes;
            }
            return QualityTier::Lossless;
        }
        if self.bitrate >= 256 {
            return QualityTier::HighLossy;
        }
        if self.bitrate >= 160
            || (self.format.eq_ignore_ascii_case("opus") && self.bitrate >= 128)
            || self.source == "deezer"
            || self.source == "ytm"
            || self.source == "soundcloud"
        {
            return QualityTier::Standard;
        }
        QualityTier::Low
    }

    pub fn quality_badge(&self) -> String {
        match self.quality_tier() {
            QualityTier::HiRes => {
                let bd = if self.bit_depth > 0 { self.bit_depth } else { 24 };
                let khz = if self.sample_rate > 0 {
                    self.sample_rate as f64 / 1000.0
                } else {
                    96.0
                };
                format!("HI-RES {}/{:.0}k", bd, khz)
            }
            QualityTier::Lossless => {
                let fmt = if self.format.is_empty() {
                    "FLAC".to_string()
                } else {
                    self.format.to_uppercase()
                };
                format!("LOSSLESS {}", fmt)
            }
            QualityTier::HighLossy => {
                let br = if self.bitrate > 0 { self.bitrate } else { 320 };
                format!("HQ {}k", br)
            }
            QualityTier::Standard => {
                if self.format.eq_ignore_ascii_case("m4a") || self.source == "deezer" || self.source == "ytm" {
                    "AAC".to_string()
                } else if !self.format.is_empty() {
                    self.format.to_uppercase()
                } else {
                    "STREAM".to_string()
                }
            }
            QualityTier::Low => "STD".to_string(),
        }
    }

    /// Numeric quality score used to prefer lossless / local / clean catalog over raw videos.
    pub fn quality_score(&self) -> i32 {
        let base = match self.quality_tier() {
            QualityTier::HiRes => 1000,
            QualityTier::Lossless => 800,
            QualityTier::HighLossy => 500,
            QualityTier::Standard => 300,
            QualityTier::Low => 100,
        };
        let source_bonus = match self.source.as_str() {
            "local" => 90,
            "soulseek" => 65,
            "archive" => 45,
            "deezer" => 40,
            "ytm" => 30,
            "soundcloud" => 20,
            "youtube" => 10,
            _ => 10,
        };
        let br_bonus = (self.bitrate.clamp(0, 1500)) / 10;
        base + source_bonus + br_bonus
    }

    pub fn mood_list(&self) -> Vec<Mood> {
        self.moods
            .split(',')
            .filter_map(Mood::from_slug)
            .collect()
    }

    /// Canonical song key ignoring upload/version/source: `primary_artist_slug|core_title`.
    pub fn song_key(&self) -> String {
        let pa = primary_artist(&self.artist);
        let a = if pa.is_empty() { &self.artist } else { &pa };
        format!("{}|{}", slug(a), core_song_title(&self.title))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AlbumSummary {
    pub id: String,
    pub name: String,
    pub artist: String,
    pub artist_id: String,
    pub cover_id: String,
    pub cover_url: String,
    pub genre: String,
    pub year: i32,
    pub label: String,
    pub deezer_id: String,
    pub song_count: usize,
    pub duration: i32,
    pub play_count: i64,
    pub last_played: i64,
    pub created: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ArtistSummary {
    pub id: String,
    pub name: String,
    pub cover_url: String,
    pub bio: String,
    pub deezer_id: String,
    pub album_count: usize,
    pub song_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Playlist {
    pub id: String,
    pub user_id: i64,
    pub owner_name: String,
    pub name: String,
    pub comment: String,
    pub public: bool,
    pub is_auto: bool,
    pub kind: String,
    pub created: i64,
    pub changed: i64,
    pub track_count: usize,
    pub duration: i32,
    pub cover_track_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct UserTrackStats {
    pub play_count: i32,
    pub skip_count: i32,
    pub last_played: i64,
}

#[derive(Debug, Clone)]
pub struct Event {
    pub track_id: String,
    pub kind: String, // "play", "complete", "skip", "star", "unstar", "rate"
    pub value: f64,
    pub at: i64,
}

#[derive(Debug, Clone)]
pub struct UserEventRow {
    pub track_id: String,
    pub kind: String,
    pub value: f64,
    pub at: i64,
    pub track: Option<Track>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShareRecord {
    pub id: String,
    pub user_id: i64,
    pub creator_name: String,
    pub kind: String,
    pub target_id: String,
    pub title: String,
    pub subtitle: String,
    pub track_ids: Vec<String>,
    pub created: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RadioStation {
    pub id: String,
    pub name: String,
    pub stream_url: String,
    pub home_page_url: String,
}

#[derive(Debug, Clone)]
pub struct CacheEntry {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LyricLine {
    pub start_ms: Option<i64>,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LyricsData {
    pub synced: bool,
    pub plain: String,
    pub lines: Vec<LyricLine>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PlayQueueState {
    pub current_id: String,
    pub current_index: usize,
    pub position_ms: i64,
    pub track_ids: Vec<String>,
    pub changed_by: String,
    pub updated: i64,
}

pub fn constant_time_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes()
        .zip(b.bytes())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

pub fn validate_credentials(username: &str, password: &str) -> Result<String, &'static str> {
    let u = username.trim();
    if u.is_empty() || u.len() > 64 {
        return Err("Username must be 1 to 64 characters");
    }
    if u.chars().any(|c| c.is_control() || c == '/' || c == '\\') {
        return Err("Username contains invalid characters");
    }
    if password.is_empty() || password.len() > 256 {
        return Err("Password must be 1 to 256 characters");
    }
    Ok(u.to_string())
}

/// Deterministic URL-safe slug for artist/album/title keys.
pub fn slug(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_dash = false;
    for ch in s.trim().to_lowercase().chars() {
        if ch.is_alphanumeric() {
            out.push(ch);
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "unknown".to_string()
    } else {
        out
    }
}

/// Returns true for common share bucket folder names that are not artist names.
pub fn is_generic_folder_name(s: &str) -> bool {
    let clean = strip_release_tags(s).trim().to_lowercase();
    if clean.is_empty() {
        return true;
    }
    matches!(
        clean.as_str(),
        "complete"
            | "incomplete"
            | "downloads"
            | "download"
            | "incoming"
            | "music"
            | "shared"
            | "share"
            | "shares"
            | "audio"
            | "song"
            | "songs"
            | "track"
            | "tracks"
            | "mp3"
            | "flac"
            | "lossless"
            | "alac"
            | "wav"
            | "albums"
            | "album"
            | "singles"
            | "single"
            | "hi-res"
            | "hires"
            | "rips"
            | "cd"
            | "temp"
            | "tmp"
            | "new"
            | "sorted"
            | "unsorted"
            | "library"
            | "collection"
            | "media"
            | "users"
            | "soulseek"
            | "slsk"
            | "slskd"
            | "v0"
            | "320"
            | "ost"
            | "soundtrack"
            | "anime"
            | "jpop"
            | "j-pop"
            | "kpop"
            | "k-pop"
            | "discography"
            | "complete discography"
            | "unknown artist"
            | "unknown"
            | "various"
            | "various artists"
            | "va"
            | "love"
            | "misc"
            | "other"
            | "favorites"
            | "favourites"
    )
}

/// Strips release/codec/hash bracket tags (e.g. `[2020.07.22]`, `[FLAC]`, `[9a149]`, `(24bit-48kHz)`).
pub fn strip_release_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if ch == '[' || ch == '(' {
            let close = if ch == '[' { ']' } else { ')' };
            let mut j = i + 1;
            let mut depth = 1i32;
            while j < chars.len() && depth > 0 {
                if chars[j] == ch {
                    depth += 1;
                } else if chars[j] == close {
                    depth -= 1;
                }
                j += 1;
            }
            if depth == 0 {
                let inner: String = chars[i + 1..j - 1].iter().collect();
                if is_release_tag(&inner) {
                    i = j;
                    continue;
                }
            }
        }
        out.push(ch);
        i += 1;
    }
    let collapsed = out.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed
        .trim_matches(|c: char| matches!(c, ' ' | '-' | '_' | '|' | '•'))
        .trim()
        .to_string()
}

fn is_release_tag(inner: &str) -> bool {
    let t = inner.trim();
    if t.is_empty() {
        return true;
    }
    let low = t.to_lowercase();
    if (2..=8).contains(&low.len())
        && low.chars().all(|c| c.is_ascii_hexdigit())
        && low.chars().any(|c| c.is_ascii_digit())
    {
        return true;
    }
    if matches!(
        low.as_str(),
        "flac"
            | "alac"
            | "mp3"
            | "wav"
            | "aiff"
            | "aac"
            | "m4a"
            | "opus"
            | "ogg"
            | "web"
            | "cd"
            | "v0"
            | "320"
            | "320kbps"
            | "hq"
            | "hi-res"
            | "hires"
            | "lossless"
            | "jp"
            | "xw"
            | "us"
            | "uk"
            | "eu"
            | "mv"
            | "pv"
            | "single"
            | "ep"
            | "album"
    ) {
        return true;
    }
    let digits_and_sep = low
        .chars()
        .all(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '/'));
    if digits_and_sep && low.chars().filter(|c| c.is_ascii_digit()).count() >= 4 {
        return true;
    }
    const RELEASE_WORDS: &[&str] = &[
        "flac",
        "24bit",
        "24-bit",
        "16bit",
        "16-bit",
        "khz",
        "kbps",
        "320k",
        "web",
        "cd",
        "vinyl",
        "official",
        "lyric",
        "audio",
        "video",
        "visualizer",
        "hi-res",
        "hires",
        "remaster",
    ];
    RELEASE_WORDS.iter().any(|w| low.contains(w))
}

/// Extracts primary artist display name by stripping release tags, channel suffixes, and collab trailers.
pub fn primary_artist(s: &str) -> String {
    let mut cleaned = strip_release_tags(s);
    for suffix in [
        " official youtube channel",
        " official channel",
        " youtube channel",
        " - topic",
        " official",
        " vevo",
    ] {
        if cleaned.to_ascii_lowercase().ends_with(suffix) {
            let keep = cleaned.len().saturating_sub(suffix.len());
            if cleaned.is_char_boundary(keep) {
                cleaned = cleaned[..keep].trim().to_string();
            }
        }
    }
    let low = cleaned.to_ascii_lowercase();
    for sep in [
        " feat. ",
        " feat ",
        " ft. ",
        " ft ",
        " featuring ",
        " vs. ",
        " vs ",
    ] {
        if let Some(pos) = low.find(sep) {
            if cleaned.is_char_boundary(pos) {
                cleaned = cleaned[..pos].trim().to_string();
                break;
            }
        }
    }
    cleaned.trim().to_string()
}

/// Strips parenthetical suffixes, punctuation, and trailing version words ("remaster", "live", "radio edit", year).
pub fn core_song_title(s: &str) -> String {
    let mut stripped = String::with_capacity(s.len());
    let mut depth = 0i32;
    for ch in s.chars() {
        match ch {
            '(' | '[' | '\u{3010}' => depth += 1,
            ')' | ']' | '\u{3011}' => {
                if depth > 0 {
                    depth -= 1;
                }
            }
            '\u{300c}' | '\u{300d}' | '\u{300e}' | '\u{300f}' => stripped.push(' '),
            _ => {
                if depth == 0 {
                    stripped.push(ch);
                }
            }
        }
    }

    let version_words: HashSet<&str> = [
        "remaster",
        "remastered",
        "live",
        "acoustic",
        "unplugged",
        "demo",
        "instrumental",
        "karaoke",
        "version",
        "edit",
        "radio",
        "single",
        "album",
        "explicit",
        "clean",
        "mono",
        "stereo",
        "deluxe",
        "mix",
        "remix",
        "cover",
        "official",
        "audio",
        "video",
        "lyric",
        "lyrics",
        "hd",
        "4k",
        "hq",
        "mv",
        "pv",
        "ver",
        "tv",
        "size",
        "short",
    ]
    .into_iter()
    .collect();

    let mut tokens: Vec<String> = stripped
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .collect();

    while tokens.len() > 1 {
        let Some(last) = tokens.last() else { break };
        let is_year = last.len() == 4
            && last.chars().all(|c| c.is_ascii_digit())
            && (last.starts_with("19") || last.starts_with("20"));
        if version_words.contains(last.as_str()) || is_year {
            tokens.pop();
        } else {
            break;
        }
    }

    tokens.join(" ")
}

/// Upgrades thumbnail URLs from YouTube/Google/Apple CDNs to high-resolution artwork.
pub fn upgrade_cover_url(url: &str) -> String {
    let trimmed = url.trim();
    if trimmed.is_empty() || (!trimmed.starts_with("http://") && !trimmed.starts_with("https://")) {
        return trimmed.to_string();
    }

    let split_idx = trimmed.find(['?', '#']).unwrap_or(trimmed.len());
    let (base, tail) = trimmed.split_at(split_idx);
    let low_base = base.to_lowercase();

    if low_base.contains("googleusercontent.com/") || low_base.contains(".ggpht.com/") {
        if let Some(slash_idx) = base.rfind('/') {
            let segment = &base[slash_idx + 1..];
            if let Some(eq_rel) = segment.rfind('=') {
                let prefix = &base[..slash_idx + 1 + eq_rel];
                let params = &segment[eq_rel + 1..];
                if let Some(after_w) = params.strip_prefix('w') {
                    let w_digits_end = after_w
                        .find(|c: char| !c.is_ascii_digit())
                        .unwrap_or(after_w.len());
                    if let Ok(w) = after_w[..w_digits_end].parse::<u32>() {
                        if let Some(after_h_tag) = after_w[w_digits_end..].strip_prefix("-h") {
                            let h_digits_end = after_h_tag
                                .find(|c: char| !c.is_ascii_digit())
                                .unwrap_or(after_h_tag.len());
                            let rest = &after_h_tag[h_digits_end..];
                            if (rest.is_empty() || rest.starts_with('-'))
                                && h_digits_end > 0
                            {
                                if let Ok(h) = after_h_tag[..h_digits_end].parse::<u32>() {
                                    if w > 0 && h > 0 && w.max(h) < 1000 {
                                        let scale = 1080.0 / (w.max(h) as f64);
                                        let nw = ((w as f64) * scale).round() as u32;
                                        let nh = ((h as f64) * scale).round() as u32;
                                        return format!("{}=w{}-h{}{}{}", prefix, nw, nh, rest, tail);
                                    }
                                }
                            }
                        }
                    }
                } else if let Some(after_s) = params.strip_prefix('s') {
                    let s_digits_end = after_s
                        .find(|c: char| !c.is_ascii_digit())
                        .unwrap_or(after_s.len());
                    let rest = &after_s[s_digits_end..];
                    if (rest.is_empty() || rest.starts_with('-')) && s_digits_end > 0 {
                        if let Ok(s) = after_s[..s_digits_end].parse::<u32>() {
                            if s > 0 && s < 1000 {
                                return format!("{}=s1080{}{}", prefix, rest, tail);
                            }
                        }
                    }
                }
            }
        }
    }

    if (low_base.contains("i.ytimg.com/vi/") || low_base.contains("img.youtube.com/vi/"))
        && (low_base.ends_with("/hqdefault.jpg")
            || low_base.ends_with("/mqdefault.jpg")
            || low_base.ends_with("/default.jpg"))
    {
        if let Some(slash_idx) = base.rfind('/') {
            return format!("{}/maxresdefault.jpg{}", &base[..slash_idx], tail);
        }
    }

    if low_base.contains("mzstatic.com/") {
        for pat in ["/60x60bb", "/100x100bb", "/200x200bb", "/300x300bb"] {
            if base.contains(pat) {
                return format!("{}{}", base.replace(pat, "/1000x1000bb"), tail);
            }
        }
    }

    trimmed.to_string()
}

pub fn extract_meta_id_field(meta: &str, field: &str) -> Option<String> {
    let trimmed = meta.trim();
    if !trimmed.starts_with('{') {
        return None;
    }
    let val = serde_json::from_str::<serde_json::Value>(trimmed).ok()?;
    let field_val = val.get(field)?;
    match field_val {
        serde_json::Value::Number(n) => {
            let u = n.as_u64()?;
            (u > 0).then(|| u.to_string())
        }
        serde_json::Value::String(s) => {
            let c = s.trim();
            (!c.is_empty() && c != "0").then(|| c.to_string())
        }
        _ => None,
    }
}

fn parse_meta_object(meta: &str) -> serde_json::Map<String, serde_json::Value> {
    let trimmed = meta.trim();
    if trimmed.is_empty() {
        return serde_json::Map::new();
    }
    if trimmed.starts_with('{') {
        if let Ok(serde_json::Value::Object(map)) = serde_json::from_str(trimmed) {
            return map;
        }
    }
    let mut map = serde_json::Map::new();
    map.insert(
        "webpage_url".to_string(),
        serde_json::Value::String(trimmed.to_string()),
    );
    map
}

/// Returns true when `a` and `b` carry explicit external artist IDs that refer to two different artists.
pub fn tracks_conflict_artist(a: &Track, b: &Track) -> bool {
    if let (Some(da), Some(db)) = (a.deezer_artist_id(), b.deezer_artist_id()) {
        if da != db {
            return true;
        }
    }
    if let (Some(ya), Some(yb)) = (a.ytm_artist_id(), b.ytm_artist_id()) {
        if ya != yb {
            return true;
        }
    }
    false
}

/// Merges two `Track.meta` strings so external IDs (`deezer_artist_id`, `deezer_album_id`,
/// `deezer_track_id`, `ytm_artist_id`) and popularity `rank` survive cross-source quality upgrades.
pub fn merge_meta_json(winner_meta: &str, loser_meta: &str) -> String {
    let w = winner_meta.trim();
    let l = loser_meta.trim();
    if l.is_empty() {
        return w.to_string();
    }
    if w.is_empty() {
        return l.to_string();
    }
    if !w.starts_with('{') && !l.starts_with('{') {
        return w.to_string();
    }
    let mut w_obj = parse_meta_object(w);
    let l_obj = parse_meta_object(l);
    for (k, lv) in l_obj {
        if k == "rank" {
            let wr = w_obj.get("rank").and_then(|v| v.as_u64()).unwrap_or(0);
            let lr = lv.as_u64().unwrap_or(0);
            if lr > wr {
                w_obj.insert(k, lv);
            }
            continue;
        }
        let needs_fill = match w_obj.get(&k) {
            None | Some(serde_json::Value::Null) => true,
            Some(serde_json::Value::Number(n)) => n.as_u64().unwrap_or(0) == 0,
            Some(serde_json::Value::String(s)) => s.trim().is_empty() || s.trim() == "0",
            _ => false,
        };
        if needs_fill {
            w_obj.insert(k, lv);
        }
    }
    serde_json::to_string(&w_obj).unwrap_or_else(|_| w.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quality_scoring_prefers_lossless_over_streams() {
        let flac = Track {
            id: "local:1".into(),
            source: "local".into(),
            format: "flac".into(),
            bit_depth: 24,
            sample_rate: 96000,
            ..Default::default()
        };
        let cd_flac = Track {
            id: "archive:1".into(),
            source: "archive".into(),
            format: "flac".into(),
            bit_depth: 16,
            sample_rate: 44100,
            ..Default::default()
        };
        let ytm = Track {
            id: "ytm:1".into(),
            source: "ytm".into(),
            format: "m4a".into(),
            bitrate: 128,
            ..Default::default()
        };
        assert!(flac.quality_score() > cd_flac.quality_score());
        assert!(cd_flac.quality_score() > ytm.quality_score());
        assert_eq!(flac.quality_tier(), QualityTier::HiRes);
        assert_eq!(cd_flac.quality_tier(), QualityTier::Lossless);
    }

    #[test]
    fn song_key_normalizes_versions() {
        let t1 = Track {
            artist: "Kansas".into(),
            title: "Carry On Wayward Son (2013 Remaster)".into(),
            ..Default::default()
        };
        let t2 = Track {
            artist: "Kansas".into(),
            title: "Carry On Wayward Son - Live".into(),
            ..Default::default()
        };
        assert_eq!(t1.song_key(), t2.song_key());
        assert_eq!(t1.song_key(), "kansas|carry on wayward son");
    }

    #[test]
    fn core_song_title_preserves_single_word_titles() {
        assert_eq!(core_song_title("Acoustic"), "acoustic");
        assert_eq!(core_song_title("Live"), "live");
    }

    #[test]
    fn upgrade_cover_url_scales_ytm_and_ytimg_art() {
        assert_eq!(
            upgrade_cover_url("https://yt3.googleusercontent.com/abc=w120-h120-l90-rj"),
            "https://yt3.googleusercontent.com/abc=w1080-h1080-l90-rj"
        );
        assert_eq!(
            upgrade_cover_url("https://lh3.googleusercontent.com/xyz=w226-h127-l90-rj"),
            "https://lh3.googleusercontent.com/xyz=w1080-h607-l90-rj"
        );
        assert_eq!(
            upgrade_cover_url("https://yt3.ggpht.com/avatar=s120-c-k-c0x00ffffff-no-rj"),
            "https://yt3.ggpht.com/avatar=s1080-c-k-c0x00ffffff-no-rj"
        );
        assert_eq!(
            upgrade_cover_url("https://i.ytimg.com/vi/R-maTg10QKs/hqdefault.jpg"),
            "https://i.ytimg.com/vi/R-maTg10QKs/maxresdefault.jpg"
        );
        assert_eq!(
            upgrade_cover_url("https://yt3.googleusercontent.com/abc=w1080-h1080-l90-rj"),
            "https://yt3.googleusercontent.com/abc=w1080-h1080-l90-rj"
        );
        assert_eq!(upgrade_cover_url("local-art:/music/a.flac"), "local-art:/music/a.flac");
    }

    #[test]
    fn homonym_artists_produce_distinct_artist_and_album_ids_and_conflict_on_merge() {
        let lisa_kr = Track {
            id: "dz:1".into(),
            source: "deezer".into(),
            title: "MONEY".into(),
            artist: "LISA".into(),
            album: "LALISA".into(),
            meta: r#"{"rank":900000,"deezer_artist_id":14559887,"deezer_album_id":257654}"#.into(),
            ..Default::default()
        };
        let lisa_jp = Track {
            id: "dz:2".into(),
            source: "deezer".into(),
            title: "Gurenge".into(),
            artist: "LiSA".into(),
            album: "LEO-NiNE".into(),
            meta: r#"{"rank":880000,"deezer_artist_id":418196,"deezer_album_id":179000}"#.into(),
            ..Default::default()
        };

        assert_eq!(lisa_kr.artist_id(), "ar:dz:14559887");
        assert_eq!(lisa_jp.artist_id(), "ar:dz:418196");
        assert_ne!(lisa_kr.artist_id(), lisa_jp.artist_id());

        assert_eq!(lisa_kr.album_id(), "al:dz-14559887-lalisa");
        assert_eq!(lisa_jp.album_id(), "al:dz-418196-leo-nine");
        assert_ne!(lisa_kr.album_id(), lisa_jp.album_id());

        assert!(tracks_conflict_artist(&lisa_kr, &lisa_jp));

        // Same song across Deezer and YTM does not conflict and preserves deezer_artist_id on merge.
        let mut ytm_money = Track {
            id: "ytm:money".into(),
            source: "ytm".into(),
            title: "MONEY".into(),
            artist: "LISA".into(),
            meta: r#"{"ytm_artist_id":"UC6-BgjsBa5R3PZQ_kZTGGQw"}"#.into(),
            ..Default::default()
        };
        assert!(!tracks_conflict_artist(&lisa_kr, &ytm_money));
        ytm_money.meta = merge_meta_json(&ytm_money.meta, &lisa_kr.meta);
        assert_eq!(ytm_money.deezer_artist_id().as_deref(), Some("14559887"));
        assert_eq!(
            ytm_money.ytm_artist_id().as_deref(),
            Some("UC6-BgjsBa5R3PZQ_kZTGGQw")
        );
        assert_eq!(ytm_money.artist_id(), "ar:dz:14559887");
    }
}
