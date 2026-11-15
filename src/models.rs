use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub password: String,
    pub is_admin: bool,
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

    pub fn description(&self) -> &'static str {
        match self {
            Mood::Chill => "Laid-back beats, downtempo & warm textures",
            Mood::Focus => "Instrumental, ambient & steady rhythms for concentration",
            Mood::Energy => "Driving tempo, powerful riffs & adrenaline",
            Mood::LateNight => "Atmospheric, nocturnal & smooth after-hours tracks",
            Mood::Upbeat => "Bright, uplifting melodies & positive energy",
            Mood::Melancholy => "Reflective, emotional & bittersweet songs",
            Mood::Party => "Club grooves, electronic & dancefloor anthems",
            Mood::Acoustic => "Unplugged, folk, singer-songwriter & intimate sessions",
            Mood::Groove => "Funk, R&B, jazz & pocket rhythms",
            Mood::RockIndie => "Guitars, alternative, classic & indie rock",
        }
    }

    pub fn from_slug(s: &str) -> Option<Mood> {
        let low = s.trim().to_lowercase().replace('-', "_");
        Self::ALL.iter().copied().find(|m| m.slug() == low)
    }
}

impl fmt::Display for Mood {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
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

impl QualityTier {
    pub fn label(&self) -> &'static str {
        match self {
            QualityTier::HiRes => "Hi-Res Lossless",
            QualityTier::Lossless => "CD Lossless",
            QualityTier::HighLossy => "High Quality",
            QualityTier::Standard => "Standard",
            QualityTier::Low => "Compressed",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Track {
    pub id: String,
    pub source: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub genre: String,
    pub moods: String, // comma-separated mood slugs
    pub year: i32,
    pub track_no: i32,
    pub duration: i32,     // seconds
    pub bitrate: i32,      // kbps
    pub sample_rate: i32,  // Hz (e.g. 44100, 96000)
    pub bit_depth: i32,    // bits (e.g. 16, 24)
    pub format: String,    // flac, wav, alac, mp3, m4a, ogg, opus
    pub size: i64,         // bytes
    pub stream_url: String,
    pub cover_url: String,
    pub meta: String,
    pub play_count: i32,
    pub last_played: i64, // unix timestamp
    pub created: i64,     // unix timestamp
}

impl Track {
    pub fn artist_id(&self) -> String {
        format!("ar:{}", slug(&self.artist))
    }

    pub fn album_id(&self) -> String {
        format!("al:{}", slug(&format!("{}|{}", self.artist, self.album)))
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
        if self.bitrate >= 160 || (self.format.eq_ignore_ascii_case("opus") && self.bitrate >= 128)
        {
            return QualityTier::Standard;
        }
        if self.source == "bandcamp" || self.source == "soundcloud" {
            return QualityTier::Standard;
        }
        QualityTier::Low
    }

    /// Numeric quality score used to prefer lossless / local / high-bitrate over YouTube.
    pub fn quality_score(&self) -> i32 {
        let base = match self.quality_tier() {
            QualityTier::HiRes => 1000,
            QualityTier::Lossless => 800,
            QualityTier::HighLossy => 500,
            QualityTier::Standard => 300,
            QualityTier::Low => 100,
        };
        let source_bonus = match self.source.as_str() {
            "local" => 80,
            "archive" => 40,
            "bandcamp" => 35,
            "soundcloud" => 20,
            "youtube" => 0,
            _ => 10,
        };
        let br_bonus = (self.bitrate.clamp(0, 1500)) / 10;
        base + source_bonus + br_bonus
    }

    pub fn quality_badge(&self) -> String {
        let fmt = if self.format.is_empty() {
            "AUDIO".to_string()
        } else {
            self.format.to_uppercase()
        };
        if self.is_lossless() {
            if self.bit_depth >= 24 || self.sample_rate > 48000 {
                let khz = if self.sample_rate > 0 {
                    format!(" {:.1}kHz", self.sample_rate as f64 / 1000.0)
                } else {
                    String::new()
                };
                let bd = if self.bit_depth > 0 {
                    format!(" {}-bit", self.bit_depth)
                } else {
                    String::new()
                };
                return format!("{}{}{}", fmt, bd, khz);
            }
            return format!("{} Lossless", fmt);
        }
        if self.bitrate > 0 {
            format!("{} {}k", fmt, self.bitrate)
        } else {
            fmt
        }
    }

    pub fn mood_list(&self) -> Vec<Mood> {
        self.moods
            .split(',')
            .filter_map(|s| Mood::from_slug(s))
            .collect()
    }

    /// Canonical song key ignoring upload/version/source: `artist_slug|core_title`.
    pub fn song_key(&self) -> String {
        format!("{}|{}", slug(&self.artist), core_song_title(&self.title))
    }

    pub fn song_aliases(&self) -> Vec<String> {
        let primary = self.song_key();
        let a_slug = slug(&self.artist);
        let mut out = Vec::new();
        for seg in bracket_segments(&self.title) {
            let core = core_song_title(&seg);
            if !core.is_empty() {
                let key = format!("{}|{}", a_slug, core);
                if key != primary {
                    out.push(key);
                }
            }
        }
        out
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Playlist {
    pub id: String,
    pub user_id: i64,
    pub name: String,
    pub comment: String,
    pub public: bool,
    pub is_auto: bool,
    pub kind: String, // "user", "supermix", "daily_mix", "discover", "on_repeat", "forgotten_gems", "lossless", "mood"
    pub created: i64,
    pub changed: i64,
    pub track_count: usize,
    pub duration: i32,
    pub cover_track_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: i64,
    pub user_id: i64,
    pub track_id: String,
    pub kind: String, // "play", "complete", "skip", "star", "unstar", "rate"
    pub value: f64,
    pub at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RadioStation {
    pub id: String,
    pub name: String,
    pub stream_url: String,
    pub home_page_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    pub track_id: String,
    pub path: String,
    pub bytes: i64,
    pub last_hit: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoMixSummary {
    pub id: String,
    pub name: String,
    pub subtitle: String,
    pub kind: String,
    pub tracks: Vec<Track>,
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

pub fn bracket_segments(s: &str) -> Vec<String> {
    let mut segs = Vec::new();
    let mut cur = String::new();
    let mut depth = 0i32;
    for ch in s.chars() {
        match ch {
            '(' | '[' | '\u{3010}' | '\u{300e}' | '\u{300c}' => {
                if depth == 0 {
                    cur.clear();
                }
                depth += 1;
            }
            ')' | ']' | '\u{3011}' | '\u{300f}' | '\u{300d}' => {
                if depth > 0 {
                    depth -= 1;
                    if depth == 0 && !cur.is_empty() {
                        segs.push(cur.clone());
                    }
                }
            }
            _ => {
                if depth > 0 {
                    cur.push(ch);
                }
            }
        }
    }
    segs
}

/// Strips bracketed suffixes, punctuation, and trailing version words ("remaster", "live", "radio edit", year).
pub fn core_song_title(s: &str) -> String {
    let mut stripped = String::with_capacity(s.len());
    let mut depth = 0i32;
    for ch in s.chars() {
        match ch {
            '(' | '[' | '\u{3010}' | '\u{300e}' | '\u{300c}' => depth += 1,
            ')' | ']' | '\u{3011}' | '\u{300f}' | '\u{300d}' => {
                if depth > 0 {
                    depth -= 1;
                }
            }
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
    ]
    .into_iter()
    .collect();

    let mut tokens: Vec<String> = stripped
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .collect();

    while let Some(last) = tokens.last() {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quality_scoring_prefers_lossless_over_youtube() {
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
        let yt = Track {
            id: "youtube:1".into(),
            source: "youtube".into(),
            format: "opus".into(),
            bitrate: 128,
            ..Default::default()
        };
        assert!(flac.quality_score() > cd_flac.quality_score());
        assert!(cd_flac.quality_score() > yt.quality_score());
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
}
