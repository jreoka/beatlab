use crate::models::{
    primary_artist, slug, AlbumSummary, ArtistSummary, Mood, Track, UserTrackStats,
};
use crate::sources::{
    dedup_tracks_prefer_quality, merge_album_tracks, merge_track_pair, SourceRegistry,
};
use crate::store::{search_match_score, Store};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use tracing::info;

pub fn split_genres(raw: &str) -> Vec<String> {
    let normalized = raw
        .replace("Films/Games", "Soundtrack")
        .replace("films/games", "soundtrack");
    normalized
        .split([',', ';', '/'])
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty() && s != "all")
        .collect()
}

pub fn contains_term(hay_low: &str, term: &str) -> bool {
    if term.is_empty() || hay_low.is_empty() {
        return false;
    }
    let mut pos = 0;
    while let Some(rel) = hay_low[pos..].find(term) {
        let start = pos + rel;
        let end = start + term.len();
        let before_ok = hay_low[..start]
            .chars()
            .next_back()
            .map_or(true, |c| !c.is_alphanumeric());
        let after_ok = hay_low[end..]
            .chars()
            .next()
            .map_or(true, |c| !c.is_alphanumeric());
        if before_ok && after_ok {
            return true;
        }
        let step = hay_low[start..]
            .chars()
            .next()
            .map_or(1, char::len_utf8);
        pos = start + step;
    }
    false
}

fn is_cover_or_backing_title(title_album_low: &str) -> bool {
    for pat in [
        "(instrumental)",
        "[instrumental]",
        "- instrumental",
        "instrumental version",
        "instrumental cover",
        "piano cover",
        "piano version",
        "acoustic cover",
        "karaoke",
        "made famous by",
        "ringtone",
        "8-bit",
        "8 bit",
        "lullaby rendition",
        "music box",
        "type beat",
        "tribute to",
        "slowed + reverb",
        "slowed and reverb",
        "nightcore",
        "8d audio",
    ] {
        if title_album_low.contains(pat) {
            return true;
        }
    }
    title_album_low.trim().ends_with(" instrumental")
}

const MOOD_META_RULES: &[(Mood, &[&str])] = &[
    (
        Mood::Chill,
        &[
            "chill",
            "lofi",
            "lo-fi",
            "chillout",
            "chillwave",
            "downtempo",
            "dream pop",
            "dreampop",
            "trip-hop",
            "trip hop",
            "bossa",
            "bossa nova",
            "bedroom pop",
            "easy listening",
            "lounge",
            "reggae",
            "dub",
            "brazilian music",
            "mpb",
            "ambient pop",
            "soft rock",
        ],
    ),
    (
        Mood::Energy,
        &[
            "rock",
            "metal",
            "heavy metal",
            "hard rock",
            "j-rock",
            "jrock",
            "pop punk",
            "alt metal",
            "nu metal",
            "punk",
            "hardcore",
            "drum and bass",
            "drum & bass",
            "dnb",
            "dubstep",
            "hardstyle",
            "workout",
            "trap",
            "phonk",
            "thrash",
            "industrial",
        ],
    ),
    (
        Mood::LateNight,
        &[
            "night",
            "midnight",
            "late night",
            "late-night",
            "after hours",
            "dark",
            "darkwave",
            "dark wave",
            "dark ambient",
            "noir",
            "moody",
            "synthwave",
            "vaporwave",
            "chillwave",
            "trip-hop",
            "trip hop",
        ],
    ),
    (
        Mood::Upbeat,
        &[
            "pop",
            "dance pop",
            "synthpop",
            "synth pop",
            "electropop",
            "k-pop",
            "kpop",
            "j-pop",
            "jpop",
            "c-pop",
            "cpop",
            "city pop",
            "asian music",
            "japanese music",
            "korean music",
            "anime",
            "latin",
            "latin music",
            "afrobeats",
            "afropop",
            "indie pop",
            "pop rock",
            "power pop",
            "happy",
            "sunshine",
            "summer",
            "feel good",
            "feel-good",
            "upbeat",
            "disco",
            "funky",
        ],
    ),
    (
        Mood::Melancholy,
        &[
            "sad",
            "melancholy",
            "emo",
            "slowcore",
            "sadcore",
            "ballad",
            "blues",
            "heartbreak",
        ],
    ),
    (
        Mood::Party,
        &[
            "dance",
            "disco",
            "house",
            "deep house",
            "tech house",
            "club",
            "techno",
            "edm",
            "electro",
            "electronic",
            "trance",
            "party",
            "rave",
            "eurodance",
            "uk garage",
            "reggaeton",
            "salsa",
            "cumbia",
            "bachata",
            "merengue",
            "amapiano",
        ],
    ),
    (
        Mood::Acoustic,
        &[
            "acoustic",
            "folk",
            "indie folk",
            "folk rock",
            "americana",
            "country",
            "singer-songwriter",
            "singer songwriter",
            "singer & songwriter",
            "chanson",
            "traditional",
            "unplugged",
            "bluegrass",
        ],
    ),
    (
        Mood::Groove,
        &[
            "funk",
            "soul",
            "soul & funk",
            "jazz",
            "r&b",
            "rnb",
            "hip-hop",
            "hip hop",
            "rap",
            "groove",
            "neosoul",
            "neo-soul",
            "neo soul",
            "motown",
            "nu-disco",
            "boogie",
            "gospel",
            "african music",
            "afrobeats",
            "afropop",
            "amapiano",
            "reggae",
            "blues",
            "brazilian music",
            "latin",
            "salsa",
            "cumbia",
        ],
    ),
    (
        Mood::RockIndie,
        &[
            "rock",
            "indie",
            "indie rock",
            "indie pop",
            "alt",
            "alternative",
            "alt-rock",
            "alt rock",
            "hard rock",
            "classic rock",
            "pop rock",
            "punk rock",
            "j-rock",
            "jrock",
            "k-rock",
            "prog rock",
            "progressive rock",
            "folk rock",
            "blues rock",
            "shoegaze",
            "grunge",
            "britpop",
            "post-punk",
            "post punk",
            "garage rock",
            "psychedelic rock",
        ],
    ),
];

const MOOD_TITLE_RULES: &[(Mood, &[&str])] = &[
    (
        Mood::Chill,
        &["lofi", "lo-fi", "chillout", "chillwave", "downtempo", "bossa nova", "chillhop", "dream pop"],
    ),
    (
        Mood::Energy,
        &["phonk", "hardstyle", "dubstep", "metalcore", "drum and bass", "thrash"],
    ),
    (
        Mood::LateNight,
        &["synthwave", "darkwave", "dark ambient", "vaporwave"],
    ),
    (Mood::Upbeat, &["synthpop", "electropop", "dance pop"]),
    (Mood::Melancholy, &["slowcore", "sadcore", "heartbreak ballad"]),
    (
        Mood::Party,
        &["techno", "eurodance", "edm", "deep house", "tech house", "rave", "club mix"],
    ),
    (
        Mood::Acoustic,
        &["unplugged", "indie folk", "singer-songwriter", "acoustic session"],
    ),
    (
        Mood::Groove,
        &["neosoul", "neo-soul", "neo soul", "motown", "smooth jazz", "bossa"],
    ),
    (
        Mood::RockIndie,
        &["shoegaze", "post-punk", "post punk", "grunge", "britpop", "indie rock", "alt rock"],
    ),
];

pub fn classify_moods_str(title: &str, album: &str, genre: &str, tags: &str) -> String {
    classify_moods(title, album, genre, tags)
        .into_iter()
        .map(|m| m.slug())
        .collect::<Vec<_>>()
        .join(",")
}

pub fn classify_moods(title: &str, album: &str, genre: &str, tags: &str) -> Vec<Mood> {
    let title_hay = format!("{} {}", title, album).to_lowercase();
    let meta_hay = format!("{} {}", genre, tags).to_lowercase();
    if is_cover_or_backing_title(&title_hay) {
        return Vec::new();
    }

    let mut moods = Vec::new();
    let push_mood = |out: &mut Vec<Mood>, m: Mood| {
        if !out.contains(&m) {
            out.push(m);
        }
    };

    for (mood, keys) in MOOD_META_RULES {
        if keys.iter().any(|k| contains_term(&meta_hay, k)) {
            push_mood(&mut moods, *mood);
        }
    }
    if !is_cover_or_backing_title(&title_hay) {
        for (mood, keys) in MOOD_TITLE_RULES {
            if keys.iter().any(|k| contains_term(&title_hay, k)) {
                push_mood(&mut moods, *mood);
            }
        }
    }

    if moods.is_empty() && contains_term(&meta_hay, "electronic") {
        moods.push(Mood::Party);
        moods.push(Mood::LateNight);
    }
    moods.truncate(3);
    moods
}

#[derive(Debug, Default, Clone)]
pub struct TasteProfile {
    pub artist_affinity: HashMap<String, f64>,
    pub artist_slug_affinity: HashMap<String, f64>,
    pub artist_slug_ext_ids: HashMap<String, HashSet<String>>,
    pub genre_affinity: HashMap<String, f64>,
    pub mood_affinity: HashMap<Mood, f64>,
    pub playlist_artist_affinity: HashMap<String, f64>,
    pub playlist_artist_slug_affinity: HashMap<String, f64>,
    pub playlist_genre_affinity: HashMap<String, f64>,
    pub playlist_mood_affinity: HashMap<Mood, f64>,
    pub related_artist_affinity: HashMap<String, f64>,
    pub related_track_affinity: HashMap<String, f64>,
    pub playlist_track_counts: HashMap<String, usize>,
    pub manual_playlist_ids: HashSet<String>,
    pub manual_playlist_song_keys: HashSet<String>,
    pub manual_playlist_names: Vec<String>,
    pub followed_artists: HashSet<String>,
    pub saved_albums: HashSet<String>,
    pub starred: HashSet<String>,
    pub starred_song_keys: HashSet<String>,
    pub played_song_keys: HashMap<String, i32>,
    pub ratings: HashMap<String, i32>,
    pub hidden_ids: HashSet<String>,
    pub hidden_keys: HashSet<String>,
}

#[derive(Default)]
struct TrackAffinities {
    artist_affinity: f64,
    genre_affinity: f64,
    mood_affinity: f64,
    playlist_artist_affinity: f64,
    playlist_genre_affinity: f64,
    playlist_mood_affinity: f64,
    followed_artist: bool,
    saved_album: bool,
}

struct ScoringContext {
    prof: TasteProfile,
    stats: HashMap<String, UserTrackStats>,
}

const RECENT_HEARD_WINDOW_SECS: i64 = 7200;
const RECENT_HEARD_HALF_LIFE_SECS: f64 = 1800.0;
const RECENT_HEARD_MAX_PENALTY: f64 = 12.0;

struct RecentHeardPenalties {
    now: i64,
    by_id: HashMap<String, i64>,
    by_key: HashMap<String, i64>,
}

impl RecentHeardPenalties {
    fn is_empty(&self) -> bool {
        self.by_id.is_empty() && self.by_key.is_empty()
    }

    fn penalty_for(&self, track: &Track) -> f64 {
        let ts_id = self.by_id.get(&track.id).copied();
        let ts_key = self.by_key.get(&track.song_key()).copied();
        let Some(heard_at) = (match (ts_id, ts_key) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        }) else {
            return 0.0;
        };
        let age = (self.now - heard_at).max(0);
        if age >= RECENT_HEARD_WINDOW_SECS {
            return 0.0;
        }
        RECENT_HEARD_MAX_PENALTY * (-(age as f64) / RECENT_HEARD_HALF_LIFE_SECS).exp()
    }
}

/// One card in the personalized "Mixed for you" rail. The artwork, artist list
/// and track ids are all prepared server-side so the client never has to build
/// a mix itself.
#[derive(Debug, Clone, Serialize)]
pub struct MixedForYouCard {
    pub id: String,
    pub title: String,
    pub art_title: String,
    pub number: String,
    pub is_super: bool,
    pub subtitle: String,
    pub covers: Vec<String>,
    pub c1: String,
    pub c2: String,
    pub track_count: usize,
}

#[derive(Default)]
struct PlaylistCentroid {
    artists: HashMap<String, f64>,
    genres: HashMap<String, f64>,
    moods: HashMap<Mood, f64>,
    keywords: Vec<String>,
    mean_year: Option<f64>,
    mean_duration: Option<f64>,
    existing_ids: HashSet<String>,
    existing_keys: HashSet<String>,
}

#[derive(Clone, Copy)]
enum ScoreMode {
    Supermix,
    Discover,
    PlaylistDna,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RadioMode {
    #[default]
    Default,
    Familiar,
    Discover,
}

impl RadioMode {
    pub fn parse(raw: Option<&str>) -> Self {
        match raw.map(str::trim).map(str::to_lowercase).as_deref() {
            Some("familiar") => Self::Familiar,
            Some("discover") => Self::Discover,
            _ => Self::Default,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SearchMixResult {
    pub tracks: Vec<Track>,
    pub instant_mix: Option<InstantMix>,
    pub refinements: SearchRefinements,
    pub taste_match_ids: HashSet<String>,
}

#[derive(Debug, Clone)]
pub struct InstantMix {
    pub title: String,
    pub subtitle: String,
    pub query: String,
    pub is_theme_mix: bool,
    pub prominent: bool,
    pub tracks: Vec<Track>,
    pub total_duration: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CuratedPoolCache {
    updated_at: i64,
    track_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct ExploreMixSpec {
    pub key: &'static str,
    pub kind: &'static str,
    pub title: &'static str,
    pub subtitle: &'static str,
    pub query: &'static str,
    pub c1: &'static str,
    pub c2: &'static str,
}

pub const EXPLORE_MIX_SPECS: &[ExploreMixSpec] = &[
    ExploreMixSpec {
        key: "70s_classics",
        kind: "decade",
        title: "70s Classics",
        subtitle: "70s rock, soul, disco & golden-era icons",
        query: "70s hits",
        c1: "#ea580c",
        c2: "#7c2d12",
    },
    ExploreMixSpec {
        key: "80s_hits",
        kind: "decade",
        title: "80s Hits",
        subtitle: "80s synthpop, rock, new wave & anthems",
        query: "80s hits",
        c1: "#db2777",
        c2: "#831843",
    },
    ExploreMixSpec {
        key: "90s_anthems",
        kind: "decade",
        title: "90s Anthems",
        subtitle: "90s alternative, R&B, hip-hop & pop",
        query: "90s hits",
        c1: "#7c3aed",
        c2: "#4c1d95",
    },
    ExploreMixSpec {
        key: "2000s_throwbacks",
        kind: "decade",
        title: "2000s Throwbacks",
        subtitle: "2000s pop, indie, hip-hop & R&B hits",
        query: "2000s hits",
        c1: "#2563eb",
        c2: "#1e3a8a",
    },
    ExploreMixSpec {
        key: "2010s_favorites",
        kind: "decade",
        title: "2010s Favorites",
        subtitle: "Modern classics & festival-era hits",
        query: "2010s hits",
        c1: "#059669",
        c2: "#064e3b",
    },
];

pub fn find_explore_mix_spec(key: &str) -> Option<&'static ExploreMixSpec> {
    EXPLORE_MIX_SPECS.iter().find(|s| s.key == key)
}

/// Local hour at which a listener's "Mixed for you" rail turns over each day.
const MIX4U_REFRESH_HOUR: i64 = 4;
/// How often a taste change may trigger a (fairly costly) featured-mood re-check.
const MIX4U_MOOD_RECHECK_SECS: i64 = 6 * 3600;
/// Minimum gap between rebuild attempts, so a failing rebuild is not retried every tick.
const MIX4U_RETRY_SECS: i64 = 30 * 60;
/// The listener's very best picks stay in the mixes day after day; the rest rotate.
const MIX4U_KEEP_TOP: usize = 12;

/// The "mix day" a timestamp falls in for a listener at `tz_offset_min`
/// (minutes east of UTC). Days start at `MIX4U_REFRESH_HOUR` local time.
fn mix4u_day(ts: i64, tz_offset_min: i32) -> i64 {
    (ts + i64::from(tz_offset_min) * 60 - MIX4U_REFRESH_HOUR * 3600).div_euclid(86_400)
}

/// Keep the top `keep_top` tracks where they are, then put tracks that were
/// not in the previous mixes ahead of ones that were (each group keeps its
/// ranked order). With a pool larger than the mixes, yesterday's repeats are
/// the first to fall off the end.
fn rotate_out_recent_mix_tracks(
    pool: Vec<Track>,
    previous_ids: &HashSet<String>,
    keep_top: usize,
) -> Vec<Track> {
    if previous_ids.is_empty() || pool.len() <= keep_top {
        return pool;
    }
    let mut out = Vec::with_capacity(pool.len());
    let mut repeats = Vec::new();
    for (i, t) in pool.into_iter().enumerate() {
        if i < keep_top || !previous_ids.contains(&t.id) {
            out.push(t);
        } else {
            repeats.push(t);
        }
    }
    out.extend(repeats);
    out
}

fn mood_cache_key(mood: Mood) -> String {
    format!("curated:mood:v4:{}", mood.slug())
}

fn mood_user_cache_key(user_id: i64, mood: Mood) -> String {
    format!("curated:mood_user:v6:{}:{}", user_id, mood.slug())
}

fn explore_cache_key(key: &str) -> String {
    format!("curated:explore:{}", key)
}

fn explore_user_cache_key(user_id: i64, key: &str) -> String {
    format!("curated:explore_user:v1:{}:{}", user_id, key)
}

fn query_cache_key(norm_query: &str) -> String {
    format!("curated:query:{}", slug(norm_query))
}

#[derive(Debug, Clone, Default)]
pub struct SearchMixContext {
    pub anchor_artist: Option<String>,
    pub anchor_deezer_artist_id: Option<String>,
    pub anchor_track_key: Option<String>,
    pub anchor_genres: Vec<String>,
    pub is_artist_query: bool,
    pub mode_override: Option<String>,
    pub ytm_radio: Vec<Track>,
    pub dz_artist_top: Vec<Track>,
    pub dz_artist_radio: Vec<Track>,
    pub dz_related_tracks: Vec<Track>,
}

#[derive(Debug, Clone)]
pub struct EntityAnchorPlan {
    pub artist_name: String,
    pub deezer_artist_id: Option<String>,
    pub seed_track: Option<Track>,
    pub is_artist_query: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchRefinements {
    pub detected_intent: String,
    pub filter_chips: Vec<RefinementChip>,
    pub query_suggestions: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefinementChip {
    pub key: String,
    pub label: String,
    pub kind: String,
    pub count: usize,
}

#[derive(Debug, Clone)]
pub struct ThemeSpec {
    pub key: &'static str,
    pub label: &'static str,
    pub keywords: &'static [&'static str],
    pub search_boost: &'static str,
}

#[derive(Debug, Clone, Default)]
pub struct SearchIntent {
    pub raw: String,
    pub normalized: String,
    pub content_tokens: Vec<String>,
    pub entity_query: Option<String>,
    pub explicit_mix_word: bool,
    pub has_trailing_radio_ambiguity: bool,
    pub decade_label: Option<&'static str>,
    pub year_range: Option<(i32, i32)>,
    pub wants_hits: bool,
    pub wants_classic: bool,
    pub moods: Vec<Mood>,
    pub genres: Vec<String>,
    pub themes: Vec<ThemeSpec>,
    pub is_theme_mix: bool,
}

impl SearchIntent {
    pub fn playlist_search_queries(&self) -> Vec<String> {
        if !self.is_theme_mix {
            return vec![];
        }
        let mut out = Vec::new();
        if let Some(dec) = self.decade_label {
            if self.genres.is_empty() && self.themes.is_empty() && self.moods.is_empty() {
                out.push(format!("{} hits", dec));
                out.push(dec.to_string());
            } else {
                out.push(self.normalized.clone());
                out.push(format!("{} hits", self.normalized));
            }
        } else if !self.themes.is_empty() {
            out.push(self.normalized.clone());
            let boost = self.themes[0].search_boost;
            if boost != self.normalized {
                out.push(boost.to_string());
            }
        } else if self.is_theme_mix {
            out.push(self.normalized.clone());
            if self.genres.len() == 1 {
                let canonical = self.genres[0].to_lowercase();
                if canonical != self.normalized {
                    out.push(canonical);
                }
            }
            if out.len() < 2
                && !self.normalized.contains("hits")
                && !self.normalized.contains("mix")
            {
                out.push(format!("{} mix", self.normalized));
            }
        }
        out.truncate(2);
        out
    }

    pub fn expanded_track_queries(&self) -> Vec<String> {
        if !self.is_theme_mix {
            return vec![];
        }
        let mut out = Vec::new();
        if let Some(dec) = self.decade_label {
            if self.genres.is_empty() && self.themes.is_empty() {
                out.push(format!("{} greatest hits", dec));
            } else {
                out.push(format!("{} hits", self.normalized));
            }
        } else if !self.themes.is_empty() {
            out.push(format!("{} hits", self.themes[0].search_boost));
        } else if !self.genres.is_empty() {
            // Genre-only mixes (e.g. "classic rock") previously produced no
            // direct track search at all, leaving only coarse genre charts.
            out.push(format!("{} hits", self.normalized));
        }
        out.retain(|q| q != &self.normalized);
        out.truncate(1);
        out
    }

    pub fn deezer_genre_hints(&self) -> Vec<&'static str> {
        if !self.is_theme_mix {
            return vec![];
        }
        let mut hints: Vec<&'static str> = Vec::new();
        for g in &self.genres {
            let mapped: &[&str] = match g.as_str() {
                "Pop" => &["pop"],
                "Synthpop" | "Dream Pop" => &[],
                "Classic Rock" => &["classic rock", "classic rock hits"],
                "Rock" | "Grunge" | "Punk" | "Post-Punk" | "Pop Punk" => &["rock", "alternative"],
                "Indie" | "Alternative" | "Shoegaze" | "Emo" => &["alternative", "indie", "rock"],
                "Hip-Hop" | "Rap" | "Trap" => &["rap", "hip hop"],
                "R&B" | "Soul" | "Neo-Soul" | "Funk" => &["r&b", "soul", "funk"],
                "Electronic" | "EDM" | "House" | "Techno" | "Trance" | "Synthwave"
                | "Drum & Bass" | "Dubstep" | "Vaporwave" | "Chillwave" | "Disco" => {
                    &["electro", "dance", "electronic"]
                }
                "Jazz" | "Bossa Nova" => &["jazz"],
                "Blues" => &["blues"],
                "Metal" => &["metal"],
                "Folk" | "Acoustic" => &["folk", "singer"],
                "Country" => &["country"],
                "Classical" => &["classical"],
                "Reggae" => &["reggae"],
                "Latin" | "Reggaeton" => &["latin"],
                "Afrobeats" => &["african", "afro"],
                "J-Pop" | "J-Rock" | "City Pop" | "K-Pop" | "C-Pop" => &[],
                _ => &[],
            };
            for &h in mapped {
                if !hints.contains(&h) {
                    hints.push(h);
                }
            }
        }
        for m in &self.moods {
            for &h in mood_deezer_genre_names(*m) {
                if !hints.contains(&h) {
                    hints.push(h);
                }
            }
        }
        hints.truncate(3);
        hints
    }
}

struct ExpandSlotGuard {
    set: std::sync::Arc<std::sync::Mutex<HashSet<i64>>>,
    user_id: i64,
}

impl Drop for ExpandSlotGuard {
    fn drop(&mut self) {
        let mut g = self.set.lock().unwrap_or_else(|e| e.into_inner());
        g.remove(&self.user_id);
    }
}

#[derive(Default)]
struct ExpandedSeedBatch {
    tracks: Vec<Track>,
    artist_metas: Vec<ArtistSummary>,
    track_links: Vec<(String, f64)>,
    artist_links: Vec<(String, f64)>,
}

impl ExpandedSeedBatch {
    fn push_track(&mut self, t: Track, seed_id: &str, track_w: f64, artist_w: f64) {
        if t.id != seed_id {
            self.track_links.push((t.id.clone(), track_w));
        }
        let ak = slug(&primary_artist(&t.artist));
        if !ak.is_empty() {
            self.artist_links.push((ak, artist_w));
        }
        self.tracks.push(t);
    }
}

#[derive(Clone)]
pub struct TasteEngine {
    pub store: Store,
    pub sources: SourceRegistry,
    pub neural: std::sync::Arc<crate::transformer::TransformerEngine>,
    expanding_users: std::sync::Arc<std::sync::Mutex<HashSet<i64>>>,
    slsk_checked: std::sync::Arc<std::sync::Mutex<HashMap<String, i64>>>,
}

impl TasteEngine {
    pub fn new(store: Store, sources: SourceRegistry) -> Self {
        let neural = std::sync::Arc::new(crate::transformer::TransformerEngine::load_or_new(&store));
        Self {
            store,
            sources,
            neural,
            expanding_users: std::sync::Arc::new(std::sync::Mutex::new(HashSet::new())),
            slsk_checked: std::sync::Arc::new(std::sync::Mutex::new(HashMap::new())),
        }
    }

    fn neural_scores(
        &self,
        user_id: i64,
        context_tracks: &[Track],
        query: Option<&str>,
        moods: &[Mood],
        genres: &[String],
        candidates: &[Track],
    ) -> Vec<f64> {
        self.neural
            .score_candidates(
                &self.store,
                user_id,
                context_tracks,
                query,
                moods,
                genres,
                candidates,
            )
            .into_iter()
            .map(|s| s as f64)
            .collect()
    }

    fn neural_blend_weight(&self, base: f64, sctx: &ScoringContext) -> f64 {
        let hist_len =
            sctx.stats.len() + sctx.prof.starred.len() + sctx.prof.manual_playlist_ids.len();
        if hist_len == 0 && !profile_has_taste(&sctx.prof) {
            return 0.0;
        }
        base * self.neural.adaptive_multiplier(hist_len)
    }

    pub fn train_and_distill_background(&self, max_steps: usize) -> crate::transformer::TrainEpochStats {
        self.neural
            .train_background_epoch(&self.store, max_steps, 16, true)
    }

    fn should_probe_slsk(&self, cache_key: &str) -> bool {
        if !self.sources.soulseek.available_or_connecting() {
            return false;
        }
        let now = Utc::now().timestamp();
        let mut guard = self.slsk_checked.lock().unwrap_or_else(|p| p.into_inner());
        if guard.len() > 2048 {
            guard.retain(|_, ts| now - *ts < 1800);
        }
        if let Some(&ts) = guard.get(cache_key) {
            if now - ts < 1800 {
                return false;
            }
        }
        guard.insert(cache_key.to_string(), now);
        true
    }

    pub fn mark_taste_dirty(&self, user_id: i64) {
        if user_id > 0 {
            self.store.kv_set(&format!("taste:dirty:{}", user_id), "1");
        }
    }

    fn auto_playlists_need_refresh(&self, user_id: i64, max_age_secs: i64) -> bool {
        if user_id <= 0 {
            return false;
        }
        let dirty = self
            .store
            .kv_get(&format!("taste:dirty:{}", user_id))
            .map(|v| v == "1")
            .unwrap_or(false);
        let last_rebuilt = self
            .store
            .kv_get(&format!("taste:rebuilt_at:{}", user_id))
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(0);
        let now = Utc::now().timestamp();
        let has_auto = self
            .store
            .list_playlists(user_id, false)
            .map(|pls| pls.iter().any(|p| p.is_auto))
            .unwrap_or(false);

        !has_auto || dirty || (now - last_rebuilt) >= max_age_secs
    }

    /// Async refresh that expands taste-related artists & radio candidates from external sources
    /// (with a bounded latency budget) before rebuilding auto-playlists.
    pub async fn refresh_auto_playlists_if_stale_async(&self, user_id: i64, max_age_secs: i64) {
        if self.auto_playlists_need_refresh(user_id, max_age_secs) {
            self.rebuild_auto_playlists_with_budget(
                user_id,
                std::time::Duration::from_millis(3200),
            )
            .await;
        }
    }

    pub async fn rebuild_auto_playlists_async(&self, user_id: i64) {
        self.rebuild_auto_playlists_with_budget(user_id, std::time::Duration::from_secs(10))
            .await;
    }

    async fn rebuild_auto_playlists_with_budget(
        &self,
        user_id: i64,
        budget: std::time::Duration,
    ) {
        if user_id <= 0 {
            return;
        }
        let _ = self.expand_taste_candidates_async(user_id, budget).await;
        let engine = self.clone();
        let _ = tokio::task::spawn_blocking(move || {
            engine.rebuild_auto_playlists(user_id);
        })
        .await;
    }

    pub fn record_event(&self, user_id: i64, track_id: &str, kind: &str, value: f64) {
        let safe_val = if value.is_finite() { value } else { 0.0 };
        let _ = self.store.record_event(user_id, track_id, kind, safe_val);
        self.learn_signal(user_id, track_id, kind, safe_val);
    }

    /// Marks user taste dirty after an already-persisted interaction event.
    pub fn learn_signal(&self, user_id: i64, _track_id: &str, kind: &str, _value: f64) {
        if user_id > 0
            && matches!(
                kind,
                "star"
                    | "unstar"
                    | "rate"
                    | "playlist_add"
                    | "playlist_remove"
                    | "complete"
                    | "skip"
                    | "search_pick"
                    | "mix_play"
                    | "not_interested"
            )
        {
            self.neural.invalidate_user_cache(user_id);
            self.mark_taste_dirty(user_id);
        }
    }

    pub fn record_playlist_additions(&self, user_id: i64, track_ids: &[String]) {
        if user_id <= 0 || track_ids.is_empty() {
            return;
        }
        for tid in track_ids {
            self.record_event(user_id, tid, "playlist_add", 1.0);
        }
    }

    pub fn record_playlist_removals(&self, user_id: i64, track_ids: &[String]) {
        if user_id <= 0 || track_ids.is_empty() {
            return;
        }
        for tid in track_ids {
            self.record_event(user_id, tid, "playlist_remove", 1.0);
        }
    }

    pub fn mark_not_interested(&self, user_id: i64, track_id: &str) {
        if user_id <= 0 || track_id.trim().is_empty() {
            return;
        }
        let song_key = self
            .store
            .get_track(track_id)
            .ok()
            .flatten()
            .filter(|t| {
                !t.title.trim().is_empty()
                    && !primary_artist(&t.artist).eq_ignore_ascii_case("Unknown Artist")
            })
            .map(|t| t.song_key())
            .unwrap_or_default();
        let _ = self.store.hide_track(user_id, track_id, &song_key);
        self.record_event(user_id, track_id, "not_interested", 1.0);
    }

    pub fn unhide_track(&self, user_id: i64, track_id: &str) -> usize {
        if user_id <= 0 {
            return 0;
        }
        let removed = self.store.unhide_track(user_id, track_id).unwrap_or(0);
        if removed > 0 {
            self.mark_taste_dirty(user_id);
        }
        removed
    }

    pub fn reset_hidden_tracks(&self, user_id: i64) -> usize {
        if user_id <= 0 {
            return 0;
        }
        let removed = self.store.reset_hidden_tracks(user_id).unwrap_or(0);
        self.mark_taste_dirty(user_id);
        removed
    }

    pub fn user_profile(&self, user_id: i64) -> TasteProfile {
        let mut prof = TasteProfile::default();
        if user_id <= 0 {
            return prof;
        }
        let starred = self.store.starred_ids(user_id).unwrap_or_default();
        let ratings = self.store.user_ratings(user_id).unwrap_or_default();
        let followed_artists = self.store.followed_artist_ids(user_id).unwrap_or_default();
        let saved_albums = self.store.saved_album_ids(user_id).unwrap_or_default();

        // 1. Recent events (with recency half-life + explicit playlist signals)
        let events = self.store.user_recent_events(user_id, 900).unwrap_or_default();
        let mut fetch_ids: HashSet<String> = events.iter().map(|e| e.track_id.clone()).collect();
        fetch_ids.extend(starred.iter().cloned());
        fetch_ids.extend(ratings.keys().cloned());
        let id_vec: Vec<String> = fetch_ids.into_iter().collect();
        let track_map: HashMap<String, Track> = self
            .store
            .get_tracks_by_ids(&id_vec)
            .unwrap_or_default()
            .into_iter()
            .map(|t| (t.id.clone(), t))
            .collect();

        let now = Utc::now().timestamp();
        for ev in &events {
            let Some(t) = track_map.get(&ev.track_id) else {
                continue;
            };
            let age_days = ((now - ev.at).max(0) as f64) / 86400.0;
            let decay = (-age_days / 21.0).exp().max(0.25);
            let base_w = match ev.kind.as_str() {
                "star" => 2.4,
                "playlist_add" => 2.1,
                "complete" => 1.5,
                "search_pick" => 1.35,
                "mix_play" => 1.05,
                "play" => 0.65,
                "rate" => (ev.value.clamp(1.0, 5.0) - 2.75) * 1.4,
                "skip" => -1.5,
                "not_interested" => -1.5,
                "unstar" => -1.6,
                "playlist_remove" => -1.0,
                _ => 0.15,
            };
            apply_track_affinity_delta(&mut prof, t, base_w * decay, false);
            if matches!(ev.kind.as_str(), "complete" | "play" | "mix_play" | "search_pick") {
                *prof.played_song_keys.entry(t.song_key()).or_insert(0) += 1;
            }
        }

        // 2. Persistent stars & explicit ratings anchor taste beyond the 900-event window
        for sid in &starred {
            if let Some(t) = track_map.get(sid) {
                prof.starred_song_keys.insert(t.song_key());
                apply_track_affinity_delta(&mut prof, t, 1.8, false);
            }
        }
        for (rid, &rating) in &ratings {
            if let Some(t) = track_map.get(rid) {
                let rw = match rating {
                    5 => 2.4,
                    4 => 1.1,
                    3 => 0.1,
                    2 => -1.4,
                    _ => -3.0,
                };
                apply_track_affinity_delta(&mut prof, t, rw, false);
            }
        }

        // 3. Curated manual playlists teach both playlist DNA and overall user taste
        let pl_entries = self
            .store
            .user_manual_playlist_tracks(user_id)
            .unwrap_or_default();
        if !pl_entries.is_empty() {
            let mut pl_lengths: HashMap<String, usize> = HashMap::new();
            let mut seen_names = HashSet::new();
            for (pid, pname, _) in &pl_entries {
                *pl_lengths.entry(pid.clone()).or_insert(0) += 1;
                if seen_names.insert(pid.clone()) {
                    prof.manual_playlist_names.push(pname.clone());
                }
            }
            for (pid, _, t) in &pl_entries {
                prof.manual_playlist_ids.insert(t.id.clone());
                prof.manual_playlist_song_keys.insert(t.song_key());
                *prof.playlist_track_counts.entry(t.id.clone()).or_insert(0) += 1;

                let n = pl_lengths.get(pid).copied().unwrap_or(1) as f64;
                // Tight curated playlists give stronger per-track signal than huge bulk dumps.
                let cur_w = (2.8 / n.sqrt().max(1.0)).clamp(0.45, 2.2);
                apply_track_affinity_delta(&mut prof, t, cur_w, true);
                apply_track_affinity_delta(&mut prof, t, cur_w * 0.65, false);
            }
        }

        // 4. Followed artists & saved albums boost taste directly
        for artist_id in &followed_artists {
            let norm = artist_id.strip_prefix("ar:").unwrap_or(artist_id);
            *prof.artist_affinity.entry(norm.to_string()).or_insert(0.0) += 2.8;
            let with_prefix = if artist_id.starts_with("ar:") {
                artist_id.clone()
            } else {
                format!("ar:{}", artist_id)
            };
            *prof.artist_affinity.entry(with_prefix).or_insert(0.0) += 2.8;
        }
        for alb_id in &saved_albums {
            for t in self.store.album_tracks(alb_id).unwrap_or_default().iter().take(6) {
                apply_track_affinity_delta(&mut prof, t, 0.55, false);
            }
        }

        let (related_artists, related_tracks) = self
            .store
            .user_taste_links(user_id)
            .unwrap_or_default();
        let (hidden_ids, hidden_keys) = self.store.hidden_track_sets(user_id).unwrap_or_default();

        prof.related_artist_affinity = related_artists;
        prof.related_track_affinity = related_tracks;
        prof.starred = starred;
        prof.ratings = ratings;
        prof.followed_artists = followed_artists;
        prof.saved_albums = saved_albums;
        prof.hidden_ids = hidden_ids;
        prof.hidden_keys = hidden_keys;
        prof
    }

    fn build_scoring_context(&self, user_id: i64) -> ScoringContext {
        let prof = self.user_profile(user_id);
        let stats = if user_id > 0 {
            self.store.track_stats_map(user_id).unwrap_or_default()
        } else {
            HashMap::new()
        };
        ScoringContext { prof, stats }
    }

    pub fn for_you_tracks(&self, user_id: i64, limit: usize) -> Vec<Track> {
        let candidates = self
            .store
            .candidate_tracks_for_user(user_id, 1800)
            .unwrap_or_default();
        if candidates.is_empty() {
            return vec![];
        }
        let sctx = self.build_scoring_context(user_id);
        let filtered: Vec<Track> = candidates
            .into_iter()
            .filter(|t| !is_strongly_disliked(t, &sctx.prof, &sctx.stats))
            .collect();
        let session_ctx: Vec<Track> = if user_id > 0 {
            self.store
                .user_recent_tracks(user_id, 8, 0)
                .unwrap_or_default()
                .into_iter()
                .map(|(t, _)| t)
                .filter(|t| !is_strongly_disliked(t, &sctx.prof, &sctx.stats))
                .take(4)
                .collect()
        } else {
            Vec::new()
        };
        let has_user_taste = !sctx.stats.is_empty() || profile_has_taste(&sctx.prof);
        let (n_scores, n_w) = if has_user_taste {
            (
                self.neural_scores(user_id, &session_ctx, None, &[], &[], &filtered),
                self.neural_blend_weight(0.85, &sctx),
            )
        } else {
            (Vec::new(), 0.0)
        };

        let mut scored: Vec<(f64, Track)> = filtered
            .into_iter()
            .enumerate()
            .map(|(idx, t)| {
                let s = score_track_core(
                    &t,
                    &sctx.prof,
                    &sctx.stats,
                    ScoreMode::Supermix,
                ) + n_scores.get(idx).copied().unwrap_or(0.0) * n_w;
                (s, t)
            })
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let ranked: Vec<Track> = scored.into_iter().map(|(_, t)| t).collect();
        dedup_and_interleave(ranked, limit, 2)
    }

    pub fn quick_picks(&self, user_id: i64, limit: usize) -> Vec<Track> {
        let pool = self.for_you_tracks(user_id, limit * 3);
        dedup_and_interleave(pool, limit, 1)
    }

    pub fn on_repeat_tracks(&self, user_id: i64, limit: usize) -> Vec<Track> {
        if user_id <= 0 {
            return vec![];
        }
        let prof = self.user_profile(user_id);
        let frequent = self
            .store
            .user_frequent_tracks(user_id, (limit * 4).max(40), 0)
            .unwrap_or_default();
        let now = Utc::now().timestamp();

        let mut scored: Vec<(f64, Track)> = frequent
            .into_iter()
            .filter(|(t, st)| {
                let bad_rating = prof.ratings.get(&t.id).copied().unwrap_or(3) <= 2;
                let net_positive = st.play_count > st.skip_count;
                !is_hidden_track(t, &prof)
                    && !bad_rating
                    && (net_positive || prof.starred.contains(&t.id))
            })
            .map(|(t, st)| {
                let age_days = ((now - st.last_played).max(0) as f64) / 86400.0;
                let recency = (-age_days / 30.0).exp().max(0.25);
                let replay_strength = (st.play_count as f64) * 1.5 - (st.skip_count as f64) * 1.6;
                let star_boost = if prof.starred.contains(&t.id) { 1.4 } else { 0.0 };
                let pl_boost = if prof.manual_playlist_ids.contains(&t.id) {
                    1.0
                } else {
                    0.0
                };
                ((replay_strength * recency) + star_boost + pl_boost, t)
            })
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let ranked: Vec<Track> = scored.into_iter().map(|(_, t)| t).collect();
        dedup_and_interleave(ranked, limit, 2)
    }

    pub fn forgotten_gems(&self, user_id: i64, limit: usize) -> Vec<Track> {
        let now = Utc::now().timestamp();
        let cutoff = now - 7 * 86400;
        let sctx = self.build_scoring_context(user_id);
        let starred = self.store.starred_tracks(user_id).unwrap_or_default();

        let mut pool: Vec<(f64, Track)> = Vec::new();
        let mut seen = HashSet::new();

        for (t, at) in starred {
            let lp = sctx.stats.get(&t.id).map(|s| s.last_played).unwrap_or(at);
            if lp < cutoff && !is_strongly_disliked(&t, &sctx.prof, &sctx.stats) && seen.insert(t.id.clone()) {
                pool.push((10.0, t));
            }
        }

        // Also surface older curated manual-playlist tracks & high-rated tracks not played recently.
        let candidates = self
            .store
            .candidate_tracks_for_user(user_id, 1800)
            .unwrap_or_default();
        for t in candidates {
            if !seen.insert(t.id.clone()) || is_strongly_disliked(&t, &sctx.prof, &sctx.stats) {
                continue;
            }
            let st = sctx.stats.get(&t.id).copied().unwrap_or_default();
            let is_curated = sctx.prof.manual_playlist_ids.contains(&t.id)
                || sctx.prof.ratings.get(&t.id).copied().unwrap_or(0) >= 4;
            let aged_play = st.last_played > 0 && st.last_played < cutoff && st.play_count > 0;
            let aged_curated = is_curated && (st.last_played == 0 || st.last_played < cutoff);
            if aged_play || aged_curated {
                let s = score_track_core(
                    &t,
                    &sctx.prof,
                    &sctx.stats,
                    ScoreMode::Supermix,
                );
                pool.push((s, t));
            }
        }

        pool.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let ranked: Vec<Track> = pool.into_iter().map(|(_, t)| t).collect();
        dedup_and_interleave(ranked, limit, 2)
    }

    pub fn discover_weekly(&self, user_id: i64, limit: usize) -> Vec<Track> {
        let candidates = self
            .store
            .candidate_tracks_for_user(user_id, 1800)
            .unwrap_or_default();
        if candidates.is_empty() {
            return vec![];
        }
        let sctx = self.build_scoring_context(user_id);

        // Exclude cross-source duplicates (e.g., Deezer vs YTM) of any track the user already heard.
        let heard_keys: HashSet<String> = candidates
            .iter()
            .filter(|t| {
                let st = sctx.stats.get(&t.id).copied().unwrap_or_default();
                st.play_count > 0 || st.skip_count > 0
            })
            .map(Track::song_key)
            .collect();

        let filtered: Vec<Track> = candidates
            .into_iter()
            .filter(|t| {
                let st = sctx.stats.get(&t.id).copied().unwrap_or_default();
                let key = t.song_key();
                let heard = st.play_count > 0 || st.skip_count > 0 || heard_keys.contains(&key);
                let known_fav = sctx.prof.starred.contains(&t.id)
                    || sctx.prof.manual_playlist_ids.contains(&t.id)
                    || sctx.prof.manual_playlist_song_keys.contains(&key)
                    || sctx.prof.ratings.contains_key(&t.id);
                !heard && !known_fav && !is_strongly_disliked(t, &sctx.prof, &sctx.stats)
            })
            .collect();
        let has_user_taste = !sctx.stats.is_empty() || profile_has_taste(&sctx.prof);
        let (n_scores, n_w) = if has_user_taste {
            (
                self.neural_scores(user_id, &[], None, &[], &[], &filtered),
                self.neural_blend_weight(0.95, &sctx),
            )
        } else {
            (Vec::new(), 0.0)
        };

        let mut scored: Vec<(f64, Track)> = filtered
            .into_iter()
            .enumerate()
            .map(|(idx, t)| {
                let s = score_track_core(
                    &t,
                    &sctx.prof,
                    &sctx.stats,
                    ScoreMode::Discover,
                ) + n_scores.get(idx).copied().unwrap_or(0.0) * n_w;
                (s, t)
            })
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let ranked: Vec<Track> = scored.into_iter().map(|(_, t)| t).collect();
        dedup_and_interleave(ranked, limit, 1)
    }

    /// Returns personalized related tracks drawn from the user's related-artist and radio graph.
    pub fn related_tracks_for_user(&self, user_id: i64, limit: usize) -> Vec<Track> {
        if limit == 0 {
            return vec![];
        }
        let candidates = self
            .store
            .candidate_tracks_for_user(user_id, 900)
            .unwrap_or_default();
        if candidates.is_empty() {
            return vec![];
        }
        let sctx = self.build_scoring_context(user_id);
        let heard_keys: HashSet<String> = candidates
            .iter()
            .filter(|t| {
                let st = sctx.stats.get(&t.id).copied().unwrap_or_default();
                st.play_count > 0 || st.skip_count > 0
            })
            .map(Track::song_key)
            .collect();

        let has_rel_graph = !sctx.prof.related_track_affinity.is_empty()
            || !sctx.prof.related_artist_affinity.is_empty();

        let mut scored: Vec<(f64, Track)> = candidates
            .into_iter()
            .filter(|t| !is_strongly_disliked(t, &sctx.prof, &sctx.stats))
            .filter_map(|t| {
                let key = t.song_key();
                let st = sctx.stats.get(&t.id).copied().unwrap_or_default();
                let heard = st.play_count > 0 || st.skip_count > 0 || heard_keys.contains(&key);
                let known_fav = sctx.prof.starred.contains(&t.id)
                    || sctx.prof.manual_playlist_ids.contains(&t.id)
                    || sctx.prof.manual_playlist_song_keys.contains(&key);
                if heard || known_fav {
                    return None;
                }
                let fctx = build_track_affinities(&t, &sctx.prof);
                let align = compute_taste_alignment(&t, &sctx.prof, &fctx);
                let rel_bonus = align.rel_t * 3.0 + align.rel_a * 2.2;
                if has_rel_graph && rel_bonus <= 0.0 && fctx.genre_affinity <= 0.0 {
                    return None;
                }
                let core = score_track_core(&t, &sctx.prof, &sctx.stats, ScoreMode::Discover);
                Some((core + rel_bonus, t))
            })
            .collect();

        if scored.is_empty() {
            return self.discover_weekly(user_id, limit);
        }
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let ranked: Vec<Track> = scored.into_iter().map(|(_, t)| t).collect();
        dedup_and_interleave(ranked, limit, 2)
    }

    /// Returns personalized "Artists You Might Like" based on related artists, genres, and taste profile.
    pub fn recommended_artists_for_user(&self, user_id: i64, limit: usize) -> Vec<ArtistSummary> {
        if limit == 0 {
            return vec![];
        }
        let sctx = self.build_scoring_context(user_id);
        let followed = self.store.followed_artist_ids(user_id).unwrap_or_default();
        let summaries = self.store.build_artist_summaries().unwrap_or_default();
        if summaries.is_empty() {
            return vec![];
        }
        let artist_names: Vec<String> = summaries.iter().map(|a| a.name.clone()).collect();
        let artist_genres = self
            .store
            .known_artist_genres_for(&artist_names)
            .unwrap_or_default();

        let has_taste = profile_has_taste(&sctx.prof)
            || !sctx.prof.related_artist_affinity.is_empty()
            || !sctx.prof.genre_affinity.is_empty();
        if !has_taste {
            // Fresh accounts only see artists from the external Deezer charts,
            // never local/cached artists from other users' searches or listens.
            return self
                .store
                .popular_chart_artists(limit.saturating_mul(2).max(limit))
                .into_iter()
                .filter(|a| !followed.contains(&a.id))
                .take(limit)
                .collect();
        }

        let prefiltered: Vec<(ArtistSummary, String, f64, f64, &str)> = summaries
            .into_iter()
            .filter(|a| {
                !followed.contains(&a.id)
                    && !crate::store::is_junk_featured_artist_name(&a.name)
            })
            .filter_map(|a| {
                let ak = slug(&primary_artist(&a.name));
                if ak.is_empty() {
                    return None;
                }
                let direct_aff = sctx
                    .prof
                    .artist_affinity
                    .get(&a.id)
                    .or_else(|| sctx.prof.artist_affinity.get(&ak))
                    .copied()
                    .unwrap_or(0.0);
                // Prefer artists the user hasn't already overplayed so recommendations feel fresh.
                if has_taste && direct_aff > 8.0 {
                    return None;
                }
                let rel_aff = sctx
                    .prof
                    .related_artist_affinity
                    .get(&ak)
                    .copied()
                    .unwrap_or(0.0);
                let ag = artist_genres
                    .get(&primary_artist(&a.name).to_lowercase())
                    .map(|s| s.as_str())
                    .unwrap_or("");
                Some((a, ak, direct_aff, rel_aff, ag))
            })
            .collect();

        let artist_pairs: Vec<(String, String)> = prefiltered
            .iter()
            .map(|(a, _, _, _, ag)| (a.name.clone(), (*ag).to_string()))
            .collect();
        let n_artist_scores = if user_id > 0 {
            self.neural
                .score_artist_candidates(&self.store, user_id, &artist_pairs)
        } else {
            Vec::new()
        };
        let n_w = self.neural_blend_weight(0.75, &sctx);

        let mut scored: Vec<(f64, ArtistSummary)> = prefiltered
            .into_iter()
            .enumerate()
            .filter_map(|(idx, (a, _ak, direct_aff, rel_aff, ag))| {
                let mut genre_aff = 0.0f64;
                for g in split_genres(ag) {
                    if let Some(&gw) = sctx.prof.genre_affinity.get(&g) {
                        genre_aff += gw;
                    }
                    if let Some(&pgw) = sctx.prof.playlist_genre_affinity.get(&g) {
                        genre_aff += pgw * 0.7;
                    }
                }
                let n_score = n_artist_scores.get(idx).copied().unwrap_or(0.0) as f64;
                let taste_rel = rel_aff * 6.5 + genre_aff.min(6.0) * 2.2 - direct_aff * 0.4;
                if has_taste && rel_aff <= 0.0 && genre_aff <= 0.0 && n_score <= 0.35 {
                    return None;
                }

                let mut pop_score = 0.0f64;
                if !a.cover_url.trim().is_empty() {
                    pop_score += 2.5;
                }
                if !a.deezer_id.trim().is_empty() {
                    pop_score += 1.5;
                }
                if let Some(fans) = parse_deezer_fans(&a.bio) {
                    if fans > 0 {
                        pop_score += (fans as f64).log10() * 0.8;
                    }
                }
                pop_score += (a.song_count.min(15) as f64) * 0.15;

                Some((taste_rel + pop_score + n_score * n_w, a))
            })
            .collect();

        if scored.is_empty() {
            return self
                .store
                .featured_artist_summaries(limit)
                .unwrap_or_default()
                .into_iter()
                .filter(|a| !followed.contains(&a.id))
                .collect();
        }
        scored.sort_by(|a, b| {
            b.0.partial_cmp(&a.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.1.name.to_lowercase().cmp(&b.1.name.to_lowercase()))
        });
        scored.into_iter().take(limit).map(|(_, a)| a).collect()
    }

    /// Synthesizes a mix built from the musical DNA across all of a user's curated playlists.
    pub fn playlist_dna_mix(&self, user_id: i64, limit: usize) -> Vec<Track> {
        if user_id <= 0 {
            return vec![];
        }
        let entries = self
            .store
            .user_manual_playlist_tracks(user_id)
            .unwrap_or_default();
        if entries.len() < 2 {
            return vec![];
        }
        let sctx = self.build_scoring_context(user_id);
        let pl_tracks: Vec<Track> = entries.iter().map(|(_, _, t)| t.clone()).collect();
        let mut centroid = build_playlist_centroid(&pl_tracks, &sctx.prof.manual_playlist_names.join(" "));
        centroid.existing_ids.clear();
        centroid.existing_keys.clear();

        let candidates = self
            .store
            .candidate_tracks_for_user(user_id, 1800)
            .unwrap_or_default();
        let filtered: Vec<Track> = candidates
            .into_iter()
            .filter(|t| !is_strongly_disliked(t, &sctx.prof, &sctx.stats))
            .collect();
        let sample_ctx: Vec<Track> = pl_tracks.iter().take(12).cloned().collect();
        let n_scores = self.neural_scores(user_id, &sample_ctx, None, &[], &[], &filtered);
        let n_w = self.neural_blend_weight(0.85, &sctx);

        let mut scored: Vec<(f64, Track)> = filtered
            .into_iter()
            .enumerate()
            .map(|(idx, t)| {
                let sim = score_against_centroid(&t, &centroid);
                let taste = score_track_core(
                    &t,
                    &sctx.prof,
                    &sctx.stats,
                    ScoreMode::PlaylistDna,
                );
                let fam_bonus = if sctx.prof.manual_playlist_ids.contains(&t.id) {
                    1.2
                } else {
                    0.5
                };
                let n_boost = n_scores.get(idx).copied().unwrap_or(0.0) * n_w;
                (sim * 2.2 + taste * 0.7 + fam_bonus + n_boost, t)
            })
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let ranked: Vec<Track> = scored.into_iter().map(|(_, t)| t).collect();
        dedup_and_interleave(ranked, limit, 2)
    }

    /// A living mix of what a listener actually plays: their most-played and
    /// starred tracks first, blended with taste-driven discoveries so the mix
    /// evolves as their listening history grows.
    pub fn user_radio_tracks(&self, user_id: i64, limit: usize) -> Vec<Track> {
        if user_id <= 0 {
            return vec![];
        }
        let repeat = self.on_repeat_tracks(user_id, limit.max(12));
        let gems = self.forgotten_gems(user_id, (limit / 2).max(6));
        let fresh = self.for_you_tracks(user_id, limit.saturating_mul(2).max(40));
        let pool: Vec<Track> = repeat
            .into_iter()
            .chain(gems)
            .chain(fresh)
            .collect();
        dedup_and_interleave(pool, limit, 2)
    }

    /// Refresh every saved "someone else's radio" playlist from its target's
    /// current taste. Called by the background curator alongside auto mixes.
    pub fn refresh_saved_user_radios(&self) {
        let Ok(radios) = self.store.all_user_radios() else {
            return;
        };
        for (owner_id, target_id, playlist_id) in radios {
            let key = format!("radio:rebuilt:{}:{}", owner_id, target_id);
            let last = self
                .store
                .kv_get(&key)
                .and_then(|v| v.parse::<i64>().ok())
                .unwrap_or(0);
            if Utc::now().timestamp() - last < 900 {
                continue;
            }
            let ids: Vec<String> = self
                .user_radio_tracks(target_id, 40)
                .into_iter()
                .map(|t| t.id)
                .collect();
            if ids.is_empty() {
                continue;
            }
            let Ok(Some(target)) = self.store.get_public_user(target_id, owner_id) else {
                continue;
            };
            let display = if target.display_name.trim().is_empty() {
                target.username.clone()
            } else {
                target.display_name.clone()
            };
            let _ = self.store.upsert_auto_playlist(
                &playlist_id,
                owner_id,
                &format!("{} Radio", display),
                "Auto-updating mix of what this listener plays",
                &format!("user_radio:{}", target_id),
                &ids,
            );
            self.store.kv_set(&key, &Utc::now().timestamp().to_string());
        }
    }

    pub fn rebuild_auto_playlists(&self, user_id: i64) {
        if user_id <= 0 {
            return;
        }
        let specs: [(&str, &str, &str, &str, Vec<String>); 6] = [
            (
                "supermix",
                "My Supermix",
                "Personalized blend of your favorites, playlists & fresh discoveries",
                "supermix",
                self.for_you_tracks(user_id, 35)
                    .into_iter()
                    .map(|t| t.id)
                    .collect(),
            ),
            (
                "discover",
                "Discover Weekly",
                "Fresh tracks tailored to your taste & playlists",
                "discover",
                self.discover_weekly(user_id, 30)
                    .into_iter()
                    .map(|t| t.id)
                    .collect(),
            ),
            (
                "repeat",
                "On Repeat",
                "Songs you can't stop playing",
                "repeat",
                self.on_repeat_tracks(user_id, 25)
                    .into_iter()
                    .map(|t| t.id)
                    .collect(),
            ),
            (
                "gems",
                "Forgotten Favorites",
                "Loved & curated tracks you haven't spun lately",
                "gems",
                self.forgotten_gems(user_id, 25)
                    .into_iter()
                    .map(|t| t.id)
                    .collect(),
            ),
            (
                "playlist-dna",
                "Playlist DNA",
                "Synthesized from the styles inside your own playlists",
                "playlist_dna",
                self.playlist_dna_mix(user_id, 30)
                    .into_iter()
                    .map(|t| t.id)
                    .collect(),
            ),
            (
                "user-radio",
                "My Radio",
                "An evolving mix of the songs you keep coming back to",
                "user_radio",
                self.user_radio_tracks(user_id, 40)
                    .into_iter()
                    .map(|t| t.id)
                    .collect(),
            ),
        ];

        let llm_exp = crate::llm::LlmCurator::load_experience(&self.store, user_id);
        for (suffix, default_title, default_comment, kind, ids) in specs {
            let pid = format!("auto-u{}-{}", user_id, suffix);
            let (title, comment) = if let Some(ov) = llm_exp
                .as_ref()
                .and_then(|e| e.auto_mix_overrides.get(kind))
            {
                let t = if ov.title.trim().is_empty() {
                    default_title
                } else {
                    ov.title.as_str()
                };
                let c = if ov.comment.trim().is_empty() {
                    default_comment
                } else {
                    ov.comment.as_str()
                };
                (t, c)
            } else {
                (default_title, default_comment)
            };
            if !ids.is_empty() {
                let _ = self
                    .store
                    .upsert_auto_playlist(&pid, user_id, title, comment, kind, &ids);
            } else if suffix == "playlist-dna" {
                let _ = self.store.delete_playlist(&pid);
            }
        }

        self.store.kv_set(&format!("taste:dirty:{}", user_id), "0");
        self.store.kv_set(
            &format!("taste:rebuilt_at:{}", user_id),
            &Utc::now().timestamp().to_string(),
        );
    }

    // -----------------------------------------------------------------------
    // Personalized "Mixed for you" rails (YouTube-Music-style numbered mixes)
    // -----------------------------------------------------------------------

    const MIXED_FOR_YOU_PARTS: [&'static str; 4] = ["1", "2", "3", "supermix"];

    fn mixed_for_you_playlist_id(user_id: i64, mood_slug: &str, part: &str) -> String {
        format!("mixfu-u{}-{}-{}", user_id, mood_slug, part)
    }

    fn mixed_for_you_kind(mood_slug: &str, part: &str) -> String {
        format!("mixed_for_you:{}:{}", mood_slug, part)
    }

    /// The mood the user leans into most, from both listening history and their
    /// own playlists. Cold users get an upbeat starter theme.
    fn featured_mixed_for_you_mood(&self, user_id: i64) -> Mood {
        let sctx = self.build_scoring_context(user_id);
        let prof = &sctx.prof;

        // Tracks from sources without genre tags (e.g. YouTube Music) add nothing to
        // `mood_affinity`, so a library made mostly of them would be judged by its few
        // tagged songs. Borrow the artist's genres from the catalog for those tracks.
        let mut untagged_moods: HashMap<Mood, f64> = HashMap::new();
        let untagged: Vec<Track> = self
            .store
            .candidate_tracks_for_user(user_id, 600)
            .unwrap_or_default()
            .into_iter()
            .filter(|t| {
                t.genre.trim().is_empty()
                    && t.mood_list().is_empty()
                    && is_familiar_track(t, prof, &sctx.stats)
                    && !is_strongly_disliked(t, prof, &sctx.stats)
            })
            .collect();
        if !untagged.is_empty() {
            let artists: Vec<String> = untagged.iter().map(|t| t.artist.clone()).collect();
            let artist_genres = self.store.known_artist_genres_for(&artists).unwrap_or_default();
            for t in &untagged {
                let Some(ag) = artist_genres.get(&primary_artist(&t.artist).to_lowercase()) else {
                    continue;
                };
                for m in classify_moods("", "", ag, "") {
                    *untagged_moods.entry(m).or_insert(0.0) += 0.5;
                }
            }
        }

        let mut best: Option<(f64, Mood)> = None;
        for &mood in Mood::ALL.iter() {
            let affinity = prof.mood_affinity.get(&mood).copied().unwrap_or(0.0)
                + prof
                    .playlist_mood_affinity
                    .get(&mood)
                    .copied()
                    .unwrap_or(0.0)
                    * 1.25
                + untagged_moods.get(&mood).copied().unwrap_or(0.0);
            if !affinity.is_finite() || affinity <= 0.0 {
                continue;
            }
            if best.map_or(true, |(score, _)| affinity > score) {
                best = Some((affinity, mood));
            }
        }
        best.map(|(_, mood)| mood).unwrap_or(Mood::Energy)
    }

    /// Remember the listener's UTC offset (minutes east of UTC, as reported by
    /// their browser) so the daily mix refresh lands overnight in *their* day.
    pub fn set_user_tz_offset(&self, user_id: i64, tz_offset_min: i32) {
        if user_id <= 0 {
            return;
        }
        let clean = tz_offset_min.clamp(-720, 840).to_string();
        let key = format!("user:tz_offset_min:{user_id}");
        if self.store.kv_get(&key).as_deref() != Some(clean.as_str()) {
            self.store.kv_set(&key, &clean);
        }
    }

    pub fn user_tz_offset_min(&self, user_id: i64) -> i32 {
        self.store
            .kv_get(&format!("user:tz_offset_min:{user_id}"))
            .and_then(|v| v.parse::<i32>().ok())
            .map(|v| v.clamp(-720, 840))
            .unwrap_or(0)
    }

    fn has_mixed_for_you(&self, user_id: i64) -> bool {
        self.store
            .list_playlists(user_id, false)
            .map(|pls| pls.iter().any(|p| p.kind.starts_with("mixed_for_you:")))
            .unwrap_or(false)
    }

    /// Mixed for you is a daily product: the mixes stay put all day so a listener
    /// can come back to a song they heard this morning, and turn over once a day
    /// at `MIX4U_REFRESH_HOUR` local time. The only mid-day rebuild is when the
    /// listener's featured mood changes, so the rail's titles never go stale.
    fn mixed_for_you_needs_refresh(&self, user_id: i64) -> bool {
        if user_id <= 0 {
            return false;
        }
        // Brand-new listeners get their mixes as soon as the catalog allows.
        if !self.has_mixed_for_you(user_id) {
            return true;
        }

        let now = Utc::now().timestamp();
        let kv_ts = |key: String| {
            self.store
                .kv_get(&key)
                .and_then(|v| v.parse::<i64>().ok())
                .unwrap_or(0)
        };
        // Don't hammer a rebuild that keeps failing (e.g. a thin catalog).
        if now - kv_ts(format!("mix4u:v4:attempt_at:{user_id}")) < MIX4U_RETRY_SECS {
            return false;
        }

        let tz = self.user_tz_offset_min(user_id);
        // When an overnight LLM edition already built today's custom Daily Mixes,
        // keep them intact instead of overwriting them with generic mood mixes.
        // The curator records when its edition's day ends in the listener's local
        // timezone, which can differ from MIX4U_REFRESH_HOUR when
        // BEATLAB_LLM_REFRESH_HOUR is customized.
        if self.store.kv_get(&format!("mix4u:mood:{user_id}")).as_deref() == Some("llm") {
            let fresh_until = kv_ts(format!("mix4u:llm_fresh_until:{user_id}"));
            if fresh_until > 0 {
                if now < fresh_until {
                    return false;
                }
            } else {
                // Editions materialized before fresh_until existed: fall back to the
                // generic mix-day comparison.
                let last_rebuilt = kv_ts(format!("mix4u:v4:rebuilt_at:{user_id}"));
                if mix4u_day(now, tz) <= mix4u_day(last_rebuilt, tz) {
                    return false;
                }
            }
        }

        let last_rebuilt = kv_ts(format!("mix4u:v4:rebuilt_at:{user_id}"));
        if mix4u_day(now, tz) > mix4u_day(last_rebuilt, tz) {
            return true;
        }

        // Taste changed: occasionally check whether the featured mood moved.
        // Recomputing the mood is not free, so it is rate limited.
        let dirty = self
            .store
            .kv_get(&format!("taste:dirty:{user_id}"))
            .map(|v| v == "1")
            .unwrap_or(false);
        let checked_key = format!("mix4u:mood_checked_at:{user_id}");
        if dirty && now - kv_ts(checked_key.clone()) >= MIX4U_MOOD_RECHECK_SECS {
            self.store.kv_set(&checked_key, &now.to_string());
            let featured = self.featured_mixed_for_you_mood(user_id);
            let current = self.store.kv_get(&format!("mix4u:mood:{user_id}"));
            return current.as_deref() != Some(featured.slug());
        }
        false
    }

    /// Rebuild the four personalized mixes in the background. Called by the
    /// curator and, on first sign-up, by `prime_new_user`.
    pub async fn rebuild_mixed_for_you(&self, user_id: i64) {
        if user_id <= 0 {
            return;
        }
        let mood = self.featured_mixed_for_you_mood(user_id);
        let slug = mood.slug();

        // If the user's dominant mood changed, drop the old theme's cards so a
        // single rail never shows two different moods at once.
        let previous = self.store.kv_get(&format!("mix4u:mood:{user_id}"));
        let same_mood = previous.as_deref() == Some(slug);
        if let Some(old) = previous {
            if old != slug {
                for part in Self::MIXED_FOR_YOU_PARTS {
                    let _ = self
                        .store
                        .delete_playlist(&Self::mixed_for_you_playlist_id(user_id, &old, part));
                }
            }
        }

        // Yesterday's tracks, so today's mixes can lead with something new.
        let mut previous_ids: HashSet<String> = HashSet::new();
        if same_mood {
            for part in Self::MIXED_FOR_YOU_PARTS {
                let pid = Self::mixed_for_you_playlist_id(user_id, slug, part);
                for t in self.store.playlist_tracks(&pid).unwrap_or_default() {
                    previous_ids.insert(t.id);
                }
            }
        }

        // Ask for more than the mixes hold, so there is room to rotate tracks in.
        let mut pool = self.mood_mix_tracks(user_id, mood, 130).await;
        if pool.len() < 24 {
            pool = self.mood_mix_tracks(user_id, mood, 170).await;
        }
        if pool.len() < 12 {
            return;
        }
        let pool = self.personalize_mixed_for_you_pool(user_id, mood, pool);
        let pool = rotate_out_recent_mix_tracks(pool, &previous_ids, MIX4U_KEEP_TOP);
        let pool = dedup_and_interleave(pool, 96, 2);
        if pool.len() < 12 {
            return;
        }

        // Three overlapping but distinct windows over the same ranked pool,
        // plus a best-of "Supermix" — the shape listeners expect from a
        // personalized mix rail.
        let part_len = (pool.len() / 3).max(12).min(40);
        for (idx, part) in ["1", "2", "3"].iter().enumerate() {
            let start = (idx * part_len) % pool.len();
            let ids: Vec<String> = (0..part_len)
                .map(|i| pool[(start + i) % pool.len()].id.clone())
                .collect();
            let _ = self.store.upsert_auto_playlist(
                &Self::mixed_for_you_playlist_id(user_id, slug, part),
                user_id,
                &format!("Daily Mix {}", idx + 1),
                &format!("A {} mix picked for you", mood.label()),
                &Self::mixed_for_you_kind(slug, part),
                &ids,
            );
        }

        let super_ids: Vec<String> = dedup_and_interleave(pool, 50, 2)
            .into_iter()
            .map(|t| t.id)
            .collect();
        let _ = self.store.upsert_auto_playlist(
            &Self::mixed_for_you_playlist_id(user_id, slug, "supermix"),
            user_id,
            "Daily Supermix",
            &format!(
                "The best of your {} mixes",
                mood.label()
            ),
            &Self::mixed_for_you_kind(slug, "supermix"),
            &super_ids,
        );

        self.store
            .kv_set(&format!("mix4u:mood:{user_id}"), slug);
        self.store.kv_set(
            &format!("mix4u:v4:rebuilt_at:{user_id}"),
            &Utc::now().timestamp().to_string(),
        );
    }

    /// Mixed for you is the listener's own rail, so their world leads and the global mood
    /// pool only backfills when their own library + related graph is still small. Moods &
    /// Vibes stay strict (taste alone never admits a track, or every mood would collapse
    /// into one favourites list), but this rail covers a single featured mood, so the
    /// listener's own untagged music — e.g. YouTube Music tracks that carry no genre at
    /// all — is welcome here as long as nothing marks it as off-mood.
    fn personalize_mixed_for_you_pool(&self, user_id: i64, mood: Mood, pool: Vec<Track>) -> Vec<Track> {
        let sctx = self.build_scoring_context(user_id);
        let is_personal = |t: &Track| is_personal_mix_track(t, &sctx.prof, &sctx.stats);

        let mut seen_keys: HashSet<String> = HashSet::new();
        // (has verified mood evidence, track)
        let mut personal: Vec<(bool, Track)> = Vec::new();
        let mut generic: Vec<Track> = Vec::new();
        for t in pool {
            if !seen_keys.insert(t.song_key()) {
                continue;
            }
            if is_personal(&t) {
                personal.push((true, t));
            } else {
                generic.push(t);
            }
        }

        // The listener's own tracks the mood ranker could not judge: no track genre and no
        // known artist genre. Only strongly personal ones (played, starred, in a playlist,
        // a well-loved artist, or a Transformer/related-graph pick) qualify.
        let user_cands = self
            .store
            .candidate_tracks_for_user(user_id, 900)
            .unwrap_or_default();
        let cand_artists: Vec<String> = user_cands.iter().map(|t| t.artist.clone()).collect();
        let artist_genres = self.store.known_artist_genres_for(&cand_artists).unwrap_or_default();
        for t in user_cands {
            if !t.genre.trim().is_empty()
                || artist_genres.contains_key(&primary_artist(&t.artist).to_lowercase())
                || seen_keys.contains(&t.song_key())
            {
                continue;
            }
            // A stored mood tag for a different mood is evidence against this one.
            let tagged = t.mood_list();
            if !tagged.is_empty() && !tagged.contains(&mood) {
                continue;
            }
            if is_hidden_track(&t, &sctx.prof)
                || is_strongly_disliked(&t, &sctx.prof, &sctx.stats)
                || is_mood_clutter_track(&t)
                || is_mood_title_trap(&t, mood)
                || track_matches_topic(&t, "christmas")
                || track_matches_topic(&t, "halloween")
                || ((t.source == "archive" || t.source == "soulseek")
                    && !is_familiar_track(&t, &sctx.prof, &sctx.stats))
            {
                continue;
            }
            let realm = user_realm_score(&t, &sctx.prof, &sctx.stats);
            let fctx = build_track_affinities(&t, &sctx.prof);
            let align = compute_taste_alignment(&t, &sctx.prof, &fctx);
            if realm >= 1.0 || align.rel_t > 0.0 || align.rel_a > 0.3 {
                seen_keys.insert(t.song_key());
                personal.push((false, t));
            }
        }

        if personal.is_empty() {
            return generic;
        }

        let personal_tracks: Vec<Track> = personal.iter().map(|(_, t)| t.clone()).collect();
        let n_scores = self.neural_scores(user_id, &[], None, &[mood], &[], &personal_tracks);
        let n_w = self.neural_blend_weight(0.75, &sctx);
        let mut scored: Vec<(f64, Track)> = personal
            .into_iter()
            .enumerate()
            .map(|(idx, (verified, t))| {
                let s = score_track_core(&t, &sctx.prof, &sctx.stats, ScoreMode::Supermix) * 0.6
                    + user_realm_score(&t, &sctx.prof, &sctx.stats) * 1.2
                    + n_scores.get(idx).copied().unwrap_or(0.0) * n_w
                    // Tracks the mood ranker verified for this mood lead unknown ones.
                    + if verified { 2.0 } else { 0.0 };
                (s, t)
            })
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let personal: Vec<Track> = scored.into_iter().map(|(_, t)| t).collect();

        // Only borrow from the global mood pool when the listener's own world
        // (played/saved tracks, artists, and related-artist graph) is too small
        // to fill the mixes.
        const MIN_POOL: usize = 36;
        let generic_allowed = MIN_POOL.saturating_sub(personal.len());
        generic.truncate(generic_allowed);

        // Spread the generic picks evenly so Mix 1/2/3 share the same personal/discovery balance.
        let mut out = Vec::with_capacity(personal.len() + generic.len());
        let every = if generic.is_empty() {
            usize::MAX
        } else {
            (personal.len() / generic.len()).max(1)
        };
        let mut generic_iter = generic.into_iter();
        for (i, t) in personal.into_iter().enumerate() {
            out.push(t);
            if (i + 1) % every == 0 {
                if let Some(g) = generic_iter.next() {
                    out.push(g);
                }
            }
        }
        out.extend(generic_iter);
        out
    }

    /// Rebuild the mixes if their day is over (or the featured mood moved).
    /// A listener with no mixes yet waits for the first build; everyone else
    /// keeps seeing today's mixes while tomorrow's are built in the background.
    pub async fn refresh_mixed_for_you_if_stale_async(&self, user_id: i64) {
        self.refresh_mixed_for_you_impl(user_id, false).await;
    }

    /// Same as `refresh_mixed_for_you_if_stale_async`, but always waits for the
    /// rebuild. The background curator uses this so a whole user base rolling
    /// over to a new day is rebuilt one listener at a time, not all at once.
    async fn refresh_mixed_for_you_blocking(&self, user_id: i64) {
        self.refresh_mixed_for_you_impl(user_id, true).await;
    }

    async fn refresh_mixed_for_you_impl(&self, user_id: i64, wait: bool) {
        if !self.mixed_for_you_needs_refresh(user_id) {
            return;
        }
        self.store.kv_set(
            &format!("mix4u:v4:attempt_at:{user_id}"),
            &Utc::now().timestamp().to_string(),
        );
        let has_mixes = self.has_mixed_for_you(user_id);
        // First builds sit on a request path (sign-up, home), so keep them short.
        let budget = std::time::Duration::from_secs(if has_mixes { 15 } else { 8 });
        let engine = self.clone();
        let build = async move {
            let _ = tokio::time::timeout(budget, engine.rebuild_mixed_for_you(user_id)).await;
        };
        if has_mixes && !wait {
            tokio::spawn(build);
        } else {
            build.await;
        }
    }

    /// Ready-to-render cards for the home rail, including collage covers and a
    /// short artist line derived from each mix's actual tracks.
    pub fn mixed_for_you_cards(&self, user_id: i64) -> Vec<MixedForYouCard> {
        if user_id <= 0 {
            return Vec::new();
        }
        let Ok(playlists) = self.store.list_playlists(user_id, false) else {
            return Vec::new();
        };
        let mut mixes: Vec<_> = playlists
            .into_iter()
            .filter(|p| p.kind.starts_with("mixed_for_you:"))
            .collect();
        mixes.sort_by_key(|p| {
            let part = p.kind.split(':').nth(2).unwrap_or("");
            if part == "supermix" {
                99
            } else {
                part.parse::<usize>().unwrap_or(50)
            }
        });

        let llm_exp = crate::llm::LlmCurator::load_experience(&self.store, user_id);
        let mut cards = Vec::with_capacity(mixes.len());
        for p in mixes {
            let mut parts = p.kind.split(':');
            let _ = parts.next();
            let mood_slug = parts.next().unwrap_or("");
            let is_llm = mood_slug == "llm";
            let mood = Mood::from_slug(mood_slug).unwrap_or(Mood::Energy);
            let part = parts.next().unwrap_or("");
            let is_super = part == "supermix";
            let llm_spec = if is_llm {
                llm_exp.as_ref().and_then(|e| e.find_daily_mix(part))
            } else {
                None
            };
            let (default_c1, default_c2) = mood.gradient();
            let (c1, c2) = if let Some(spec) = llm_spec {
                (spec.c1.clone(), spec.c2.clone())
            } else {
                (default_c1.to_string(), default_c2.to_string())
            };
            let tracks = self.filter_hidden_tracks_for_user(
                user_id,
                self.store.playlist_tracks(&p.id).unwrap_or_default(),
            );

            let mut covers: Vec<String> = Vec::new();
            for t in &tracks {
                if !t.cover_url.trim().is_empty() && !covers.contains(&t.cover_url) {
                    covers.push(t.cover_url.clone());
                    if covers.len() >= 4 {
                        break;
                    }
                }
            }
            let mut artists: Vec<String> = Vec::new();
            for t in tracks.iter().take(16) {
                let artist = primary_artist(&t.artist);
                if artist.trim().is_empty() || artist.eq_ignore_ascii_case("Unknown Artist") {
                    continue;
                }
                if artists
                    .iter()
                    .any(|a| a.eq_ignore_ascii_case(artist.as_str()))
                {
                    continue;
                }
                artists.push(artist.to_string());
                if artists.len() >= 4 {
                    break;
                }
            }

            let title = if let Some(spec) = llm_spec {
                spec.title.clone()
            } else if is_llm && !p.name.trim().is_empty() {
                p.name.clone()
            } else if is_super {
                "Daily Supermix".to_string()
            } else {
                format!("Daily Mix {}", part.parse::<usize>().unwrap_or(0))
            };
            let art_title = if let Some(spec) = llm_spec {
                spec.art_title.clone()
            } else if is_super {
                "Daily Supermix".to_string()
            } else {
                "Daily Mix".to_string()
            };
            let subtitle = if let Some(spec) = llm_spec {
                if !spec.subtitle.is_empty() && !artists.is_empty() {
                    format!("{} · {}", spec.subtitle, artists.iter().take(2).cloned().collect::<Vec<_>>().join(", "))
                } else if !spec.subtitle.is_empty() {
                    spec.subtitle.clone()
                } else if !artists.is_empty() {
                    artists.join(", ")
                } else {
                    mood.subtitle().to_string()
                }
            } else if artists.is_empty() {
                mood.subtitle().to_string()
            } else {
                artists.join(", ")
            };

            cards.push(MixedForYouCard {
                id: p.id,
                title,
                art_title,
                number: if is_super {
                    String::new()
                } else {
                    format!("{:02}", part.parse::<usize>().unwrap_or(0))
                },
                is_super,
                subtitle,
                covers,
                c1,
                c2,
                track_count: tracks.len(),
            });
        }
        cards
    }

    /// Seeds a brand-new account from external, currently-popular charts only.
    /// No instance-wide listening history, no local files, no cached search or
    /// listen results ever enter a fresh user's starter auto-playlists.
    pub async fn prime_new_user(&self, user_id: i64) {
        if user_id <= 0 || self.store.has_listening_history(user_id).unwrap_or(false) {
            return;
        }
        let dz = &self.sources.deezer;
        let (global, pop, hiphop, chart_albums, chart_artists) = tokio::join!(
            dz.fetch_chart_tracks(50),
            dz.fetch_genre_chart_tracks(132, "Pop", 20),
            dz.fetch_genre_chart_tracks(116, "Hip-Hop", 20),
            dz.fetch_chart_albums(18),
            dz.fetch_chart_artists(24),
        );
        let mut all = global.clone();
        all.extend(pop.iter().cloned());
        all.extend(hiphop.iter().cloned());
        if !all.is_empty() {
            let _ = self.store.upsert_tracks(&all);
        }
        if !global.is_empty() {
            let _ = self.store.record_chart_tracks(&global, 1);
        }
        let mut genre = pop;
        genre.extend(hiphop);
        if !genre.is_empty() {
            let _ = self.store.record_chart_tracks(&genre, 1000);
        }
        for alb in &chart_albums {
            let _ = self.store.upsert_album_meta(alb);
        }
        self.store.record_chart_albums(&chart_albums);
        for art in &chart_artists {
            let _ = self.store.upsert_artist_meta(art);
        }
        self.store.record_chart_artists(&chart_artists);
        // Rebuild from the clean, chart-only slate produced above.
        self.rebuild_auto_playlists(user_id);
        // Bounded so a cold catalog never slows down account creation.
        self.refresh_mixed_for_you_if_stale_async(user_id).await;
    }

    /// Cold-start / periodic background warming using pure HTTP Deezer global + multi-genre charts,
    /// plus initial curation of missing moods and featured Explore mixes.
    pub async fn warm_catalog(&self) {
        let dz = &self.sources.deezer;
        let (
            chart_tracks,
            chart_albums,
            chart_artists,
            pop_tracks,
            hiphop_tracks,
            rock_tracks,
            dance_tracks,
            rnb_tracks,
            alt_tracks,
        ) = tokio::join!(
            dz.fetch_chart_tracks(50),
            dz.fetch_chart_albums(24),
            dz.fetch_chart_artists(24),
            dz.fetch_genre_chart_tracks(132, "Pop", 20),
            dz.fetch_genre_chart_tracks(116, "Hip-Hop", 20),
            dz.fetch_genre_chart_tracks(152, "Rock", 20),
            dz.fetch_genre_chart_tracks(113, "Dance", 20),
            dz.fetch_genre_chart_tracks(165, "R&B", 20),
            dz.fetch_genre_chart_tracks(85, "Alternative", 20),
        );

        let mut chart_tracks = chart_tracks;
        dz.enrich_track_genres(&mut chart_tracks).await;
        let global_chart_tracks = chart_tracks.clone();
        let mut all_tracks = chart_tracks;
        let mut genre_chart_tracks = Vec::new();
        for batch in [
            pop_tracks,
            hiphop_tracks,
            rock_tracks,
            dance_tracks,
            rnb_tracks,
            alt_tracks,
        ] {
            genre_chart_tracks.extend(batch.iter().cloned());
            all_tracks.extend(batch);
        }
        if !all_tracks.is_empty() {
            let _ = self.store.upsert_tracks(&all_tracks);
        }
        if !global_chart_tracks.is_empty() {
            let _ = self.store.record_chart_tracks(&global_chart_tracks, 1);
        }
        if !genre_chart_tracks.is_empty() {
            let _ = self.store.record_chart_tracks(&genre_chart_tracks, 1000);
        }
        for alb in &chart_albums {
            let _ = self.store.upsert_album_meta(alb);
        }
        self.store.record_chart_albums(&chart_albums);
        for art in &chart_artists {
            let _ = self.store.upsert_artist_meta(art);
        }
        self.store.record_chart_artists(&chart_artists);

        // Warm any unpopulated Moods and core Explore mixes gently at startup.
        for &mood in Mood::ALL.iter() {
            if self.load_curated_pool_ids(&mood_cache_key(mood), 6 * 3600).len() < 12 {
                let _ = tokio::time::timeout(
                    std::time::Duration::from_secs(14),
                    self.curate_mood_pool(mood),
                )
                .await;
                tokio::time::sleep(std::time::Duration::from_millis(120)).await;
            }
        }
        for spec in EXPLORE_MIX_SPECS.iter() {
            if self.load_curated_pool_ids(&explore_cache_key(spec.key), 6 * 3600).len() < 10 {
                let _ = tokio::time::timeout(
                    std::time::Duration::from_secs(7),
                    self.curate_explore_mix(spec),
                )
                .await;
                tokio::time::sleep(std::time::Duration::from_millis(120)).await;
            }
        }

        let train_engine = self.clone();
        let train_stats = tokio::task::spawn_blocking(move || train_engine.train_and_distill_background(48))
            .await
            .unwrap_or_default();

        if let Ok(uids) = self.store.all_user_ids() {
            for uid in uids {
                self.enrich_user_listened_tracks(uid).await;
                self.rebuild_auto_playlists_async(uid).await;
            }
        }
        self.update_curator_status();
        info!(
            tracks = self.store.count_tracks().unwrap_or(0),
            model_steps = train_stats.total_steps,
            model_loss = train_stats.avg_loss,
            "catalog warming & neural transformer warm-up complete"
        );
    }

    /// Enriches any tracks in a user's listening history that are missing `genre`, `year`, or `moods`
    /// using Deezer album metadata and artist album lookup, persisting the enriched metadata to SQLite.
    pub async fn enrich_user_listened_tracks(&self, user_id: i64) {
        if user_id <= 0 {
            return;
        }
        let store = self.store.clone();
        let mut missing = tokio::task::spawn_blocking(move || {
            store.user_tracks_missing_metadata(user_id, 64).unwrap_or_default()
        })
        .await
        .unwrap_or_default();
        if missing.is_empty() {
            return;
        }
        self.sources.deezer.enrich_track_genres(&mut missing).await;
        let store = self.store.clone();
        let _ = tokio::task::spawn_blocking(move || {
            let _ = store.upsert_tracks(&missing);
        })
        .await;
    }

    /// Runs one incremental background curation cycle even when no user is actively listening:
    /// rotates through moods, Explore featured mixes/decades, extra genre charts, and per-user taste expansion.
    pub async fn curate_background_step(&self, tick: usize) {
        let now = Utc::now().timestamp();

        // 1. Refresh global top charts occasionally (every ~15 ticks = ~30 mins).
        if tick % 15 == 0 {
            let dz = &self.sources.deezer;
            let (chart_tracks, chart_albums, chart_artists) = tokio::join!(
                dz.fetch_chart_tracks(50),
                dz.fetch_chart_albums(24),
                dz.fetch_chart_artists(24),
            );
            if !chart_tracks.is_empty() {
                let _ = self.store.upsert_tracks(&chart_tracks);
                let _ = self.store.record_chart_tracks(&chart_tracks, 1);
            }
            for alb in &chart_albums {
                let _ = self.store.upsert_album_meta(alb);
            }
            self.store.record_chart_albums(&chart_albums);
            for art in &chart_artists {
                let _ = self.store.upsert_artist_meta(art);
            }
            self.store.record_chart_artists(&chart_artists);
        }

        // 2. Curate 2 moods per cycle, prioritizing missing/oldest pools first.
        let mut mood_order: Vec<(i64, Mood)> = Mood::ALL
            .iter()
            .map(|&m| {
                let ts = self
                    .load_curated_pool_entry(&mood_cache_key(m))
                    .map(|c| c.updated_at)
                    .unwrap_or(0);
                (ts, m)
            })
            .collect();
        mood_order.sort_by_key(|(ts, _)| *ts);
        for (_, mood) in mood_order.into_iter().take(2) {
            let _ = tokio::time::timeout(
                std::time::Duration::from_secs(14),
                self.curate_mood_pool(mood),
            )
            .await;
            tokio::time::sleep(std::time::Duration::from_millis(160)).await;
        }

        // 3. Curate 2 Explore decades per cycle, prioritizing missing/oldest first.
        let mut mix_order: Vec<(i64, &'static ExploreMixSpec)> = EXPLORE_MIX_SPECS
            .iter()
            .map(|spec| {
                let ts = self
                    .load_curated_pool_entry(&explore_cache_key(spec.key))
                    .map(|c| c.updated_at)
                    .unwrap_or(0);
                (ts, spec)
            })
            .collect();
        mix_order.sort_by_key(|(ts, _)| *ts);
        for (_, spec) in mix_order.into_iter().take(2) {
            let _ = tokio::time::timeout(
                std::time::Duration::from_secs(8),
                self.curate_explore_mix(spec),
            )
            .await;
            tokio::time::sleep(std::time::Duration::from_millis(160)).await;
        }

        // 4. Rotate through extra Deezer genres to continuously enrich SQLite discovery pools.
        let dz_genres = self.sources.deezer.fetch_genres().await;
        if !dz_genres.is_empty() {
            let start = (tick * 2) % dz_genres.len();
            for offset in 0..2.min(dz_genres.len()) {
                let g = &dz_genres[(start + offset) % dz_genres.len()];
                if g.id > 0 && !g.name.eq_ignore_ascii_case("all") {
                    let tracks = self
                        .sources
                        .deezer
                        .fetch_genre_chart_tracks(g.id, &g.name, 25)
                        .await;
                    if !tracks.is_empty() {
                        let _ = self.store.upsert_tracks(&tracks);
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(140)).await;
                }
            }
        }

        // 5. Actively train the RoPE-QKVO + XSA + GEGLU Transformer across ALL users' histories &
        // playlists, distill personalized discoveries for every user (even inactive ones), and
        // expand taste graphs & auto-playlists in preparation.
        let train_engine = self.clone();
        let train_stats =
            tokio::task::spawn_blocking(move || train_engine.train_and_distill_background(48))
                .await
                .unwrap_or_default();
        if train_stats.steps_run > 0 && !train_stats.skipped {
            info!(
                tick = tick,
                steps_run = train_stats.steps_run,
                total_steps = train_stats.total_steps,
                avg_loss = train_stats.avg_loss,
                examples = train_stats.examples_used,
                distilled_users = train_stats.distilled_users,
                rejected = train_stats.rejected,
                heldout = train_stats.eval.n,
                model_mrr = train_stats.eval.model_mrr,
                popularity_mrr = train_stats.eval.popularity_mrr,
                content_mrr = train_stats.eval.content_mrr,
                quality = train_stats.eval.quality,
                "transformer background training & distillation tick complete"
            );
        }

        if let Ok(uids) = self.store.all_user_ids() {
            for uid in uids {
                let _ = self
                    .expand_taste_candidates_async(uid, std::time::Duration::from_secs(4))
                    .await;
                // Refresh personalized mixes first: rebuilding the generic auto
                // mixes clears the shared "taste dirty" flag.
                self.refresh_mixed_for_you_blocking(uid).await;
                self.refresh_auto_playlists_if_stale_async(uid, 900).await;
            }
        }

        // Saved user radios are other people's taste, so they live outside the
        // owner-scoped auto-playlist loop above. Rebuild them from their target.
        let radio_engine = self.clone();
        let _ = tokio::task::spawn_blocking(move || {
            radio_engine.refresh_saved_user_radios();
        })
        .await;

        self.store.kv_set("curator:last_tick_at", &now.to_string());
        self.update_curator_status();
    }

    fn load_curated_pool_entry(&self, key: &str) -> Option<CuratedPoolCache> {
        self.store
            .kv_get(key)
            .and_then(|raw| serde_json::from_str::<CuratedPoolCache>(&raw).ok())
    }

    pub fn load_curated_pool_ids(&self, key: &str, max_age_secs: i64) -> Vec<String> {
        let Some(entry) = self.load_curated_pool_entry(key) else {
            return Vec::new();
        };
        if max_age_secs > 0 && (Utc::now().timestamp() - entry.updated_at) > max_age_secs {
            return Vec::new();
        }
        entry.track_ids
    }

    fn save_curated_pool(&self, key: &str, track_ids: &[String]) {
        if track_ids.is_empty() {
            return;
        }
        let mut deduped = Vec::with_capacity(track_ids.len().min(60));
        let mut seen = HashSet::new();
        for id in track_ids {
            if seen.insert(id.clone()) {
                deduped.push(id.clone());
                if deduped.len() >= 60 {
                    break;
                }
            }
        }
        let payload = CuratedPoolCache {
            updated_at: Utc::now().timestamp(),
            track_ids: deduped,
        };
        if let Ok(json_str) = serde_json::to_string(&payload) {
            self.store.kv_set(key, &json_str);
        }
    }

    fn preview_from_tracks(tracks: &[Track]) -> (usize, Vec<String>) {
        let mut covers = Vec::new();
        for t in tracks {
            if !t.cover_url.is_empty() && !covers.contains(&t.cover_url) {
                covers.push(t.cover_url.clone());
                if covers.len() >= 4 {
                    break;
                }
            }
        }
        (tracks.len(), covers)
    }

    #[cfg(test)]
    pub fn curated_mood_preview(&self, mood: Mood) -> (usize, Vec<String>) {
        let ids = self.load_curated_pool_ids(&mood_cache_key(mood), 24 * 3600);
        let tracks = if !ids.is_empty() {
            self.store.get_tracks_by_ids(&ids).unwrap_or_default()
        } else {
            self.store.tracks_by_mood(mood, 30, 0).unwrap_or_default()
        };
        Self::preview_from_tracks(&tracks)
    }

    fn filter_hidden_tracks_for_user(&self, user_id: i64, tracks: Vec<Track>) -> Vec<Track> {
        if user_id <= 0 || tracks.is_empty() {
            return tracks;
        }
        let (hidden_ids, hidden_keys) = self.store.hidden_track_sets(user_id).unwrap_or_default();
        if hidden_ids.is_empty() && hidden_keys.is_empty() {
            return tracks;
        }
        tracks
            .into_iter()
            .filter(|t| {
                !hidden_ids.contains(&t.id)
                    && (hidden_keys.is_empty() || !hidden_keys.contains(&t.song_key()))
            })
            .collect()
    }

    pub fn curated_mood_preview_for_user(&self, user_id: i64, mood: Mood) -> (usize, Vec<String>) {
        if user_id > 0 {
            let global_updated = self
                .load_curated_pool_entry(&mood_cache_key(mood))
                .map(|e| e.updated_at)
                .unwrap_or(0);
            if let Some(entry) = self.load_curated_pool_entry(&mood_user_cache_key(user_id, mood)) {
                let fresh = (Utc::now().timestamp() - entry.updated_at) <= 12 * 3600;
                if fresh && entry.updated_at >= global_updated && !entry.track_ids.is_empty() {
                    let tracks = self
                        .store
                        .get_tracks_by_ids(&entry.track_ids)
                        .unwrap_or_default();
                    let tracks = self.filter_hidden_tracks_for_user(user_id, tracks);
                    if !tracks.is_empty() {
                        return Self::preview_from_tracks(&tracks);
                    }
                }
            }
        }
        let is_fresh_user =
            user_id > 0 && !self.store.has_listening_history(user_id).unwrap_or(false);
        let cached_curated_ids = self.load_curated_pool_ids(&mood_cache_key(mood), 24 * 3600);
        let cached_curated_tracks = if !cached_curated_ids.is_empty() {
            self.store
                .get_tracks_by_ids(&cached_curated_ids)
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let local_mood = if is_fresh_user {
            Vec::new()
        } else {
            self.store.tracks_by_mood(mood, 140, 0).unwrap_or_default()
        };
        let user_pool = if user_id > 0 {
            self.store
                .candidate_tracks_for_user(user_id, 900)
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let mut candidates = Vec::new();
        let mut seen_ids = HashSet::new();
        for t in cached_curated_tracks
            .into_iter()
            .chain(local_mood)
            .chain(user_pool.clone())
        {
            if seen_ids.insert(t.id.clone()) {
                candidates.push(t);
            }
        }
        if candidates.is_empty() {
            return (0, Vec::new());
        }
        let curated_ids: HashSet<String> = cached_curated_ids.into_iter().collect();
        let mut ranked = self.rank_mood_mix(user_id, mood, candidates, &curated_ids, 35);
        if ranked.len() < 12 && user_id > 0 && !is_fresh_user {
            let sctx = self.build_scoring_context(user_id);
            let mut have_keys: HashSet<String> = ranked.iter().map(Track::song_key).collect();
            let mut have_ids: HashSet<String> = ranked.iter().map(|t| t.id.clone()).collect();
            let extra_artists: Vec<String> = user_pool.iter().map(|t| t.artist.clone()).collect();
            let artist_genres = self
                .store
                .known_artist_genres_for(&extra_artists)
                .unwrap_or_default();
            for t in user_pool {
                if ranked.len() >= 35 {
                    break;
                }
                if have_ids.contains(&t.id)
                    || have_keys.contains(&t.song_key())
                    || is_strongly_disliked(&t, &sctx.prof, &sctx.stats)
                    || is_mood_clutter_track(&t)
                    || ((t.source == "archive" || t.source == "soulseek")
                        && !is_familiar_track(&t, &sctx.prof, &sctx.stats))
                {
                    continue;
                }
                let ag = artist_genres
                    .get(&primary_artist(&t.artist).to_lowercase())
                    .map(|s| s.as_str())
                    .unwrap_or("");
                let meta_low = format!("{} {}", t.genre, ag).to_lowercase();
                let has_evidence = has_mood_meta_match(&meta_low, mood)
                    || classify_moods(&t.title, &t.album, &t.genre, ag).contains(&mood);
                if has_evidence
                    && is_track_compatible_with_mood(&t, ag, mood)
                    && !is_mood_title_trap(&t, mood)
                    && !track_matches_topic(&t, "christmas")
                    && !track_matches_topic(&t, "halloween")
                {
                    have_ids.insert(t.id.clone());
                    have_keys.insert(t.song_key());
                    ranked.push(t);
                }
            }
        }
        Self::preview_from_tracks(&ranked)
    }

    pub fn curated_explore_preview_for_user(
        &self,
        user_id: i64,
        spec: &ExploreMixSpec,
    ) -> (usize, Vec<String>) {
        if user_id > 0 {
            let global_updated = self
                .load_curated_pool_entry(&explore_cache_key(spec.key))
                .map(|e| e.updated_at)
                .unwrap_or(0);
            if let Some(entry) =
                self.load_curated_pool_entry(&explore_user_cache_key(user_id, spec.key))
            {
                let fresh = (Utc::now().timestamp() - entry.updated_at) <= 12 * 3600;
                if fresh && entry.updated_at >= global_updated && !entry.track_ids.is_empty() {
                    let tracks = self
                        .store
                        .get_tracks_by_ids(&entry.track_ids)
                        .unwrap_or_default();
                    let tracks = self.filter_hidden_tracks_for_user(user_id, tracks);
                    if !tracks.is_empty() {
                        return Self::preview_from_tracks(&tracks);
                    }
                }
            }
        }
        let ids = self.load_curated_pool_ids(&explore_cache_key(spec.key), 24 * 3600);
        let curated_tracks = if !ids.is_empty() {
            self.store.get_tracks_by_ids(&ids).unwrap_or_default()
        } else {
            Vec::new()
        };
        if curated_tracks.is_empty() && user_id <= 0 {
            return (0, Vec::new());
        }
        let ranked = self.rank_explore_mix_tracks(user_id, spec, curated_tracks, 35);
        Self::preview_from_tracks(&ranked)
    }

    pub fn curator_status_snapshot(&self) -> serde_json::Value {
        let last_run = self
            .store
            .kv_get("curator:last_tick_at")
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0);
        let ready_moods = Mood::ALL
            .iter()
            .filter(|&&m| {
                !self
                    .load_curated_pool_ids(&mood_cache_key(m), 24 * 3600)
                    .is_empty()
                    || !self
                        .store
                        .tracks_by_mood(m, 1, 0)
                        .unwrap_or_default()
                        .is_empty()
            })
            .count();
        let ready_mixes = EXPLORE_MIX_SPECS
            .iter()
            .filter(|s| !self.load_curated_pool_ids(&explore_cache_key(s.key), 24 * 3600).is_empty())
            .count();
        serde_json::json!({
            "active": true,
            "lastRunAt": last_run,
            "catalogTracks": self.store.count_tracks().unwrap_or(0),
            "readyMoods": ready_moods,
            "totalMoods": Mood::ALL.len(),
            "readyMixes": ready_mixes,
            "totalMixes": EXPLORE_MIX_SPECS.len(),
            "transformer": self.neural.status_snapshot(),
        })
    }

    fn update_curator_status(&self) {
        let now = Utc::now().timestamp();
        self.store.kv_set("curator:last_tick_at", &now.to_string());
    }
}

impl TasteEngine {
    pub async fn curate_explore_mix(&self, spec: &ExploreMixSpec) -> Vec<Track> {
        let intent = parse_search_intent(spec.query);
        let mut pl_queries: Vec<String> = if intent.is_theme_mix {
            let mut q = intent.playlist_search_queries();
            if q.is_empty() {
                q.push(spec.query.to_string());
            }
            q
        } else {
            vec![spec.query.to_string()]
        };
        if intent.genres.is_empty()
            && intent.decade_label.is_none()
            && intent.themes.is_empty()
            && intent.moods.len() == 1
        {
            for mq in mood_playlist_queries(intent.moods[0]) {
                if !pl_queries.iter().any(|existing| existing.eq_ignore_ascii_case(mq)) {
                    pl_queries.push((*mq).to_string());
                }
            }
            pl_queries.truncate(3);
        }

        let mut curated_batches = Vec::new();
        for pq in &pl_queries {
            let batches = self.sources.fetch_curated_mix_tracks(pq, 3, 25).await;
            for (pl_title, batch) in batches {
                if !batch.is_empty() {
                    curated_batches.push((pl_title, batch));
                }
            }
        }

        let mut raw_tracks = Vec::new();
        if curated_batches.is_empty() {
            // Avoid `deezer.search_tracks` for mood-only queries,
            // since Deezer track search mainly returns literal title-collision tracks.
            let is_mood_only = !intent.moods.is_empty()
                && intent.genres.is_empty()
                && intent.decade_label.is_none()
                && intent.themes.is_empty();
            if is_mood_only {
                let hints = intent.deezer_genre_hints();
                if !hints.is_empty() {
                    let dz_genres = self.sources.deezer.fetch_genres().await;
                    let matched: Vec<_> = dz_genres
                        .into_iter()
                        .filter(|g| {
                            let gl = g.name.to_lowercase();
                            hints.iter().any(|want| contains_term(&gl, want))
                        })
                        .take(2)
                        .collect();
                    for g in matched {
                        let chart = self
                            .sources
                            .deezer
                            .fetch_genre_chart_tracks(g.id, &g.name, 20)
                            .await;
                        raw_tracks.extend(chart);
                    }
                }
            } else {
                raw_tracks = self.sources.deezer.search_tracks(spec.query, 20).await;
            }
            if raw_tracks.len() < 8 {
                use crate::sources::MusicSource;
                let ytm_tracks = self.sources.ytm.search(spec.query, 15).await;
                raw_tracks.extend(ytm_tracks);
            }
        }

        let mut to_upsert = raw_tracks.clone();
        for (_, batch) in &curated_batches {
            to_upsert.extend(batch.iter().cloned());
        }
        if !to_upsert.is_empty() {
            let _ = self.store.upsert_tracks(&to_upsert);
        }

        let has_curated = !curated_batches.is_empty();
        let res = self.search_and_refine_with_context(
            0,
            spec.query,
            raw_tracks,
            curated_batches,
            SearchMixContext::default(),
        );
        let mut tracks = res
            .instant_mix
            .map(|m| m.tracks)
            .unwrap_or_else(|| res.tracks);
        tracks.retain(|t| !is_mood_clutter_track(t));
        if has_curated && tracks.len() >= 8 {
            let ids: Vec<String> = tracks.iter().map(|t| t.id.clone()).collect();
            self.save_curated_pool(&explore_cache_key(spec.key), &ids);
            self.save_curated_pool(&query_cache_key(&intent.normalized), &ids);
        }
        tracks
    }

    pub async fn get_curated_explore_mix_tracks(
        &self,
        user_id: i64,
        spec: &ExploreMixSpec,
        limit: usize,
    ) -> Vec<Track> {
        let cached_ids = self.load_curated_pool_ids(&explore_cache_key(spec.key), 12 * 3600);
        let mut curated_tracks = if cached_ids.len() >= 8 {
            self.store.get_tracks_by_ids(&cached_ids).unwrap_or_default()
        } else {
            Vec::new()
        };
        if curated_tracks.len() < 8 {
            curated_tracks = self.curate_explore_mix(spec).await;
        }
        let ranked = self.rank_explore_mix_tracks(user_id, spec, curated_tracks, limit);
        if user_id > 0 && !ranked.is_empty() {
            let ids: Vec<String> = ranked.iter().map(|t| t.id.clone()).collect();
            self.save_curated_pool(&explore_user_cache_key(user_id, spec.key), &ids);
        }
        ranked
    }

    /// Preview (track count + up to 4 cover URLs) for a user's LLM-curated Explore mix.
    pub fn curated_llm_explore_preview(
        &self,
        user_id: i64,
        spec: &crate::llm::LlmExploreMixSpec,
    ) -> (usize, Vec<String>) {
        if user_id <= 0 {
            return (0, Vec::new());
        }
        let ids = self.load_curated_pool_ids(
            &crate::llm::llm_explore_pool_key(user_id, &spec.key),
            48 * 3600,
        );
        if ids.is_empty() {
            return (0, Vec::new());
        }
        let tracks = self.store.get_tracks_by_ids(&ids).unwrap_or_default();
        let tracks = self.filter_hidden_tracks_for_user(user_id, tracks);
        Self::preview_from_tracks(&tracks)
    }

    /// Return tracks for an LLM-curated Explore mix, using its overnight cached pool
    /// when ready or curating on demand if not yet cached.
    pub async fn get_llm_explore_mix_tracks(
        &self,
        user_id: i64,
        spec: &crate::llm::LlmExploreMixSpec,
        limit: usize,
    ) -> Vec<Track> {
        let pool_key = crate::llm::llm_explore_pool_key(user_id, &spec.key);
        let cached_ids = self.load_curated_pool_ids(&pool_key, 36 * 3600);
        if cached_ids.len() >= 8 {
            let tracks = self.store.get_tracks_by_ids(&cached_ids).unwrap_or_default();
            let visible = self.filter_hidden_tracks_for_user(user_id, tracks);
            if visible.len() >= 8 {
                return visible.into_iter().take(limit).collect();
            }
        }
        let queries = vec![spec.query.clone()];
        let empty_excl = HashSet::new();
        let tracks = self
            .curate_llm_mix_tracks(
                user_id,
                &queries,
                &spec.seed_artists,
                &spec.genres,
                spec.mood.as_deref(),
                spec.year_range,
                "balanced",
                &empty_excl,
                limit,
            )
            .await;
        if !tracks.is_empty() {
            let ids: Vec<String> = tracks.iter().map(|t| t.id.clone()).collect();
            self.save_curated_pool(&pool_key, &ids);
        }
        tracks
    }

    /// Materialize a freshly generated `DailyLlmExperience` into real playlists and
    /// cached Explore category pools in SQLite so every screen loads instantaneously.
    pub async fn materialize_llm_experience(
        &self,
        user_id: i64,
        exp: &crate::llm::DailyLlmExperience,
    ) {
        if user_id <= 0 {
            return;
        }

        // 1. Materialize the 4 custom-named Daily Mixes ("Mixed for you" rail).
        if !exp.daily_mixes.is_empty() {
            // Clean up any legacy non-llm mixed_for_you playlists so the rail never mixes old & new.
            if let Some(old_mood) = self.store.kv_get(&format!("mix4u:mood:{user_id}")) {
                if old_mood != "llm" {
                    for part in Self::MIXED_FOR_YOU_PARTS {
                        let _ = self.store.delete_playlist(&Self::mixed_for_you_playlist_id(
                            user_id, &old_mood, part,
                        ));
                    }
                }
            }

            let mut used_keys: HashSet<String> = HashSet::new();
            let mut built_any = false;
            for spec in &exp.daily_mixes {
                let is_super = spec.part == "supermix";
                let target_len = if is_super { 40 } else { 30 };
                let excl = if is_super {
                    HashSet::new()
                } else {
                    used_keys.clone()
                };
                let tracks = self
                    .curate_llm_mix_tracks(
                        user_id,
                        &spec.seed_queries,
                        &spec.seed_artists,
                        &spec.genres,
                        spec.mood.as_deref(),
                        spec.year_range,
                        &spec.familiarity,
                        &excl,
                        target_len,
                    )
                    .await;
                if tracks.is_empty() {
                    continue;
                }
                if !is_super {
                    for t in &tracks {
                        used_keys.insert(t.song_key());
                    }
                }
                let ids: Vec<String> = tracks.iter().map(|t| t.id.clone()).collect();
                let comment = if spec.subtitle.trim().is_empty() {
                    format!("Curated for {}", exp.date_label)
                } else {
                    spec.subtitle.clone()
                };
                let _ = self.store.upsert_auto_playlist(
                    &Self::mixed_for_you_playlist_id(user_id, "llm", &spec.part),
                    user_id,
                    &spec.title,
                    &comment,
                    &Self::mixed_for_you_kind("llm", &spec.part),
                    &ids,
                );
                built_any = true;
            }
            if built_any {
                self.store
                    .kv_set(&format!("mix4u:mood:{user_id}"), "llm");
                self.store.kv_set(
                    &format!("mix4u:v4:rebuilt_at:{user_id}"),
                    &Utc::now().timestamp().to_string(),
                );
            }
        }

        // 2. Rebuild standard auto-playlists so they pick up today's LLM custom titles & comments.
        let engine_for_auto = self.clone();
        let _ = tokio::task::spawn_blocking(move || {
            engine_for_auto.rebuild_auto_playlists(user_id);
        })
        .await;

        // 3. Pre-populate each custom Explore category mix overnight so cards have real
        //    cover collages, track counts, and instant playback in the morning.
        for cat in &exp.explore_categories {
            for mix in &cat.mixes {
                let queries = vec![mix.query.clone()];
                let empty_excl = HashSet::new();
                let tracks = self
                    .curate_llm_mix_tracks(
                        user_id,
                        &queries,
                        &mix.seed_artists,
                        &mix.genres,
                        mix.mood.as_deref(),
                        mix.year_range,
                        "balanced",
                        &empty_excl,
                        30,
                    )
                    .await;
                if !tracks.is_empty() {
                    let ids: Vec<String> = tracks.iter().map(|t| t.id.clone()).collect();
                    self.save_curated_pool(
                        &crate::llm::llm_explore_pool_key(user_id, &mix.key),
                        &ids,
                    );
                }
                tokio::time::sleep(std::time::Duration::from_millis(80)).await;
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn curate_llm_mix_tracks(
        &self,
        user_id: i64,
        seed_queries: &[String],
        seed_artists: &[String],
        genres: &[String],
        mood_slug: Option<&str>,
        year_range: Option<(i32, i32)>,
        familiarity: &str,
        exclude_song_keys: &HashSet<String>,
        limit: usize,
    ) -> Vec<Track> {
        let mut raw_candidates: Vec<Track> = Vec::new();
        let mut curated_source_ids: HashSet<String> = HashSet::new();

        // 1. Fetch curated playlists & search tracks for LLM seed queries (bounded timeouts).
        for q in seed_queries.iter().take(2) {
            let trimmed = q.trim();
            if trimmed.is_empty() {
                continue;
            }
            let batches = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                self.sources.fetch_curated_mix_tracks(trimmed, 2, 20),
            )
            .await
            .unwrap_or_default();
            let mut got_batch = false;
            for (_, batch) in batches {
                if !batch.is_empty() {
                    got_batch = true;
                    for t in batch {
                        curated_source_ids.insert(t.id.clone());
                        raw_candidates.push(t);
                    }
                }
            }
            if !got_batch {
                let dz = tokio::time::timeout(
                    std::time::Duration::from_secs(3),
                    self.sources.deezer.search_tracks(trimmed, 16),
                )
                .await
                .unwrap_or_default();
                for t in dz {
                    curated_source_ids.insert(t.id.clone());
                    raw_candidates.push(t);
                }
            }
            if let Ok(local_hits) = self.store.search_tracks(trimmed, 16, 0) {
                raw_candidates.extend(local_hits);
            }
        }

        // 2. Pull tracks for LLM seed artists from SQLite + Deezer.
        for artist in seed_artists.iter().take(4) {
            let clean = artist.trim();
            if clean.is_empty() {
                continue;
            }
            let local_artist_tracks = self.store.search_tracks(clean, 10, 0).unwrap_or_default();
            let local_cnt = local_artist_tracks.len();
            raw_candidates.extend(local_artist_tracks);
            if local_cnt < 4 {
                let remote = tokio::time::timeout(
                    std::time::Duration::from_secs(3),
                    self.sources.deezer.search_tracks(clean, 8),
                )
                .await
                .unwrap_or_default();
                raw_candidates.extend(remote);
            }
        }

        if !raw_candidates.is_empty() {
            let _ = self.store.upsert_tracks(&raw_candidates);
        }

        // 3. Also pull matching tracks from the listener's own catalog & mood/genre pools.
        let mood_opt = mood_slug.and_then(Mood::from_slug);
        if let Some(m) = mood_opt {
            let mood_ids = self.load_curated_pool_ids(&mood_cache_key(m), 24 * 3600);
            if !mood_ids.is_empty() {
                raw_candidates.extend(
                    self.store
                        .get_tracks_by_ids(&mood_ids[..mood_ids.len().min(30)])
                        .unwrap_or_default(),
                );
            }
        }

        let user_cands = self
            .store
            .candidate_tracks_for_user(user_id, 600)
            .unwrap_or_default();

        let sctx = self.build_scoring_context(user_id);
        let seed_artist_lows: Vec<String> = seed_artists
            .iter()
            .map(|a| primary_artist(a).to_lowercase())
            .filter(|a| !a.is_empty())
            .collect();
        let genre_lows: Vec<String> = genres
            .iter()
            .map(|g| g.trim().to_lowercase())
            .filter(|g| !g.is_empty())
            .collect();

        let mut seen_ids: HashSet<String> = HashSet::new();
        let mut seen_keys: HashSet<String> = HashSet::new();
        let mut pool: Vec<Track> = Vec::new();

        let mut admit = |t: Track, require_match: bool| {
            if is_hidden_track(&t, &sctx.prof)
                || is_strongly_disliked(&t, &sctx.prof, &sctx.stats)
                || is_mood_clutter_track(&t)
            {
                return;
            }
            if require_match {
                let pa = primary_artist(&t.artist).to_lowercase();
                let artist_hit = seed_artist_lows
                    .iter()
                    .any(|sa| !pa.is_empty() && (pa == *sa || pa.contains(sa.as_str())));
                let g_low = t.genre.to_lowercase();
                let genre_hit = !genre_lows.is_empty()
                    && genre_lows.iter().any(|want| {
                        genre_variants(want)
                            .iter()
                            .any(|alias| contains_term(&g_low, alias))
                    });
                let mood_hit = mood_opt.map_or(false, |m| {
                    t.mood_list().contains(&m)
                        || classify_moods(&t.title, &t.album, &t.genre, "").contains(&m)
                });
                let year_hit = year_range
                    .map_or(false, |(lo, hi)| t.year >= lo && t.year <= hi);
                if !(artist_hit || genre_hit || mood_hit || year_hit) {
                    return;
                }
            }
            if seen_ids.insert(t.id.clone()) && seen_keys.insert(t.song_key()) {
                pool.push(t);
            }
        };

        for t in raw_candidates {
            admit(t, false);
        }
        for t in user_cands {
            admit(t, true);
        }
        drop(admit);

        // Graceful fallback when sources are offline or the catalog is very small.
        if pool.len() < 12 {
            let fallback = if familiarity == "discovery" {
                self.discover_weekly(user_id, limit)
            } else {
                self.for_you_tracks(user_id, limit)
            };
            for t in fallback {
                if !is_hidden_track(&t, &sctx.prof)
                    && !is_strongly_disliked(&t, &sctx.prof, &sctx.stats)
                    && !is_mood_clutter_track(&t)
                    && seen_ids.insert(t.id.clone())
                    && seen_keys.insert(t.song_key())
                {
                    pool.push(t);
                }
            }
        }
        if pool.is_empty() {
            return Vec::new();
        }

        let score_mode = match familiarity {
            "discovery" => ScoreMode::Discover,
            _ => ScoreMode::Supermix,
        };
        let mood_cues: Vec<Mood> = mood_opt.into_iter().collect();
        let primary_q = seed_queries.first().map(String::as_str);
        let n_w = self.neural_blend_weight(0.65, &sctx);
        let n_scores = if n_w > 0.0 {
            self.neural_scores(user_id, &[], primary_q, &mood_cues, genres, &pool)
        } else {
            Vec::new()
        };

        let mut scored: Vec<(f64, Track)> = pool
            .into_iter()
            .enumerate()
            .map(|(idx, t)| {
                let mut s = score_track_core(&t, &sctx.prof, &sctx.stats, score_mode);
                if let Some(&ns) = n_scores.get(idx) {
                    s += ns * n_w;
                }
                let pa = primary_artist(&t.artist).to_lowercase();
                if seed_artist_lows
                    .iter()
                    .any(|sa| !pa.is_empty() && (pa == *sa || pa.contains(sa.as_str())))
                {
                    s += 3.2;
                }
                let g_low = t.genre.to_lowercase();
                if genre_lows.iter().any(|want| {
                    genre_variants(want)
                        .iter()
                        .any(|alias| contains_term(&g_low, alias))
                }) {
                    s += 1.6;
                }
                if let Some(m) = mood_opt {
                    if t.mood_list().contains(&m)
                        || classify_moods(&t.title, &t.album, &t.genre, "").contains(&m)
                    {
                        s += 1.2;
                    }
                }
                if let Some((lo, hi)) = year_range {
                    if t.year >= lo && t.year <= hi {
                        s += 1.8;
                    } else if t.year > 0 {
                        s -= 1.2;
                    }
                }
                if curated_source_ids.contains(&t.id) {
                    s += 0.9;
                }
                let familiar = is_familiar_track(&t, &sctx.prof, &sctx.stats);
                match familiarity {
                    "familiar" => {
                        if familiar {
                            s += 2.4;
                        }
                    }
                    "discovery" => {
                        if familiar {
                            s -= 2.2;
                        } else {
                            s += 1.4;
                        }
                    }
                    _ => {
                        if familiar {
                            s += 0.6;
                        }
                    }
                }
                // Strongly prefer tracks not already placed in another Daily Mix today.
                if exclude_song_keys.contains(&t.song_key()) {
                    s -= 8.0;
                }
                (s, t)
            })
            .collect();

        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let ranked: Vec<Track> = scored.into_iter().map(|(_, t)| t).collect();
        dedup_and_interleave(ranked, limit, 2)
    }

    fn rank_explore_mix_tracks(
        &self,
        user_id: i64,
        spec: &ExploreMixSpec,
        curated_tracks: Vec<Track>,
        limit: usize,
    ) -> Vec<Track> {
        let intent = parse_search_intent(spec.query);
        let sctx = self.build_scoring_context(user_id);
        let category_mood = intent.moods.first().copied();

        let mut pool: Vec<(Option<usize>, Track)> = Vec::new();
        let mut seen_ids = HashSet::new();
        let mut seen_keys = HashSet::new();

        if user_id > 0 {
            let user_cands = self
                .store
                .candidate_tracks_for_user(user_id, 900)
                .unwrap_or_default();
            let cand_artists: Vec<String> = user_cands.iter().map(|t| t.artist.clone()).collect();
            let artist_genres = self
                .store
                .known_artist_genres_for(&cand_artists)
                .unwrap_or_default();

            for t in user_cands {
                if is_hidden_track(&t, &sctx.prof)
                    || is_strongly_disliked(&t, &sctx.prof, &sctx.stats)
                    || is_mood_clutter_track(&t)
                    || track_matches_topic(&t, "christmas")
                    || track_matches_topic(&t, "halloween")
                {
                    continue;
                }
                let ag = artist_genres
                    .get(&primary_artist(&t.artist).to_lowercase())
                    .map(|s| s.as_str())
                    .unwrap_or("");
                let g_low = format!("{} {}", t.genre, ag).to_lowercase();

                let mood_ok = category_mood.map_or(true, |m| {
                    is_track_compatible_with_mood(&t, ag, m)
                        && (has_mood_meta_match(&g_low, m)
                            || classify_moods(&t.title, &t.album, &t.genre, ag).contains(&m)
                            || (!g_low.trim().is_empty() && t.mood_list().contains(&m)))
                });
                let genre_ok = intent.genres.is_empty()
                    || intent.genres.iter().any(|want| {
                        let wl = want.to_lowercase();
                        genre_variants(&wl)
                            .iter()
                            .any(|alias| contains_term(&g_low, alias))
                    });
                let year_ok = intent
                    .year_range
                    .map_or(true, |(min_y, max_y)| t.year >= min_y && t.year <= max_y);

                if mood_ok && genre_ok && year_ok {
                    if seen_ids.insert(t.id.clone()) && seen_keys.insert(t.song_key()) {
                        pool.push((None, t));
                    }
                }
            }
        }

        for (idx, t) in curated_tracks.into_iter().enumerate() {
            if is_hidden_track(&t, &sctx.prof)
                || is_strongly_disliked(&t, &sctx.prof, &sctx.stats)
                || is_mood_clutter_track(&t)
                || track_matches_topic(&t, "christmas")
                || track_matches_topic(&t, "halloween")
            {
                continue;
            }
            // Cached pools can drift from a category; re-check compatibility.
            if let Some(m) = category_mood {
                if !is_track_compatible_with_mood(&t, "", m) {
                    continue;
                }
            }
            if seen_ids.insert(t.id.clone()) && seen_keys.insert(t.song_key()) {
                pool.push((Some(idx), t));
            }
        }

        if pool.is_empty() {
            return Vec::new();
        }

        let pool_tracks: Vec<Track> = pool.iter().map(|(_, t)| t.clone()).collect();
        let mut mood_cues: Vec<Mood> = intent.moods.clone();
        if let Some(m) = category_mood {
            if !mood_cues.contains(&m) {
                mood_cues.push(m);
            }
        }
        let n_w = self.neural_blend_weight(0.75, &sctx);
        let n_scores = if n_w > 0.0 {
            self.neural_scores(
                user_id,
                &[],
                Some(spec.query),
                &mood_cues,
                &intent.genres,
                &pool_tracks,
            )
        } else {
            Vec::new()
        };
        let all_artists: Vec<String> = pool.iter().map(|(_, t)| t.artist.clone()).collect();
        let all_artist_genres = self
            .store
            .known_artist_genres_for(&all_artists)
            .unwrap_or_default();
        let mut scored: Vec<(f64, Track)> = pool
            .into_iter()
            .enumerate()
            .map(|(idx, (curated_idx, t))| {
                let taste_s = score_track_core(&t, &sctx.prof, &sctx.stats, ScoreMode::Supermix);
                let rank_s = curated_idx
                    .map(|ci| (25usize.saturating_sub(ci) as f64) * 0.07)
                    .unwrap_or(0.0);
                let realm = if user_id > 0 {
                    user_realm_score(&t, &sctx.prof, &sctx.stats)
                } else {
                    0.0
                };
                let ag = all_artist_genres
                    .get(&primary_artist(&t.artist).to_lowercase())
                    .map(|s| s.as_str())
                    .unwrap_or("");
                let g_low = format!("{} {}", t.genre, ag).to_lowercase();
                let mut evidence = if curated_idx.is_some() { 0.6 } else { 0.0 };
                if let Some(m) = category_mood {
                    if has_mood_meta_match(&g_low, m) {
                        evidence += 1.8;
                    }
                    if classify_moods(&t.title, &t.album, &t.genre, ag).contains(&m) {
                        evidence += 1.4;
                    }
                    if !g_low.trim().is_empty() && t.mood_list().contains(&m) {
                        evidence += 1.6;
                    }
                } else {
                    for want in &intent.genres {
                        let wl = want.to_lowercase();
                        if genre_variants(&wl).iter().any(|alias| contains_term(&g_low, alias)) {
                            evidence += 1.8;
                            break;
                        }
                    }
                }
                let n_boost = n_scores.get(idx).copied().unwrap_or(0.0) * n_w;
                (
                    rank_s + taste_s * 0.95 + parse_rank_bonus(&t.meta) * 0.08
                        + evidence + realm * 0.9 + n_boost,
                    t,
                )
            })
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let ordered: Vec<Track> = scored.into_iter().map(|(_, t)| t).collect();
        dedup_and_interleave(ordered, limit, 2)
    }

    pub fn load_curated_query_tracks(&self, normalized_query: &str) -> Vec<Track> {
        let ids = self.load_curated_pool_ids(&query_cache_key(normalized_query), 24 * 3600);
        if ids.is_empty() {
            return Vec::new();
        }
        self.store.get_tracks_by_ids(&ids).unwrap_or_default()
    }

    pub fn save_curated_query_tracks(&self, normalized_query: &str, tracks: &[Track]) {
        let ids: Vec<String> = tracks.iter().map(|t| t.id.clone()).collect();
        self.save_curated_pool(&query_cache_key(normalized_query), &ids);
    }

    /// Expands a user's SQLite candidate pool and `user_taste_links` using live related-artist and
    /// radio graphs seeded from their favorites, frequent plays, recent listens, and manual playlists.
    pub async fn expand_taste_candidates_async(
        &self,
        user_id: i64,
        budget: std::time::Duration,
    ) -> usize {
        if user_id <= 0 {
            return 0;
        }
        {
            let mut active = self
                .expanding_users
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if !active.insert(user_id) {
                return 0;
            }
        }
        let _guard = ExpandSlotGuard {
            set: self.expanding_users.clone(),
            user_id,
        };

        let now = Utc::now().timestamp();
        let expand_ttl = 6 * 3600;
        let kv_key = format!("taste:expanded_artists:{}", user_id);
        let mut expanded_map: HashMap<String, i64> = self
            .store
            .kv_get(&kv_key)
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        expanded_map.retain(|_, ts| now - *ts < expand_ttl);

        let ratings = self.store.user_ratings(user_id).unwrap_or_default();
        let stats = self.store.track_stats_map(user_id).unwrap_or_default();
        let starred = self.store.starred_tracks(user_id).unwrap_or_default();
        let frequent = self.store.user_frequent_tracks(user_id, 24, 0).unwrap_or_default();
        let recent = self.store.user_recent_tracks(user_id, 24, 0).unwrap_or_default();
        let pl_entries = self.store.user_manual_playlist_tracks(user_id).unwrap_or_default();
        let followed = self.store.followed_artist_ids(user_id).unwrap_or_default();

        let mut seed_by_artist: HashMap<String, (f64, Track)> = HashMap::new();
        let mut add_seed = |t: Track, bonus: f64| {
            let ak = slug(&primary_artist(&t.artist));
            if ak.is_empty() || ak == "unknown-artist" {
                return;
            }
            let st = stats.get(&t.id).copied().unwrap_or_default();
            let bad_rating = ratings.get(&t.id).copied().unwrap_or(3) <= 2;
            let over_skipped = st.skip_count >= st.play_count + 2;
            if bad_rating || over_skipped {
                return;
            }
            let total_w = bonus + (st.play_count.min(10) as f64) * 0.4;
            match seed_by_artist.get_mut(&ak) {
                Some((cur_w, cur_t)) => {
                    if total_w > *cur_w {
                        *cur_w = total_w;
                        *cur_t = t;
                    }
                }
                None => {
                    seed_by_artist.insert(ak, (total_w, t));
                }
            }
        };

        for (t, _) in starred {
            add_seed(t, 5.0);
        }
        for (t, st) in frequent {
            add_seed(t, 2.5 + (st.play_count.min(12) as f64) * 0.8);
        }
        for (idx, (t, _)) in recent.into_iter().enumerate() {
            add_seed(t, (3.2 - (idx as f64) * 0.1).max(1.2));
        }
        for (_, _, t) in pl_entries {
            add_seed(t, 3.5);
        }

        let mut ranked_seeds: Vec<(String, f64, Track)> = seed_by_artist
            .into_iter()
            .map(|(ak, (w, t))| (ak, w, t))
            .collect();
        ranked_seeds.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let unexpanded_tracks: Vec<(String, Track)> = ranked_seeds
            .iter()
            .filter(|(ak, _, _)| !expanded_map.contains_key(ak))
            .take(4)
            .map(|(ak, _, t)| (ak.clone(), t.clone()))
            .collect();

        let seed_artist_slugs: HashSet<String> = ranked_seeds.iter().map(|(ak, _, _)| ak.clone()).collect();
        let mut unexpanded_followed: Vec<(String, String)> = followed
            .into_iter()
            .filter_map(|fid| {
                let ak = slug(fid.strip_prefix("ar:").unwrap_or(&fid));
                if ak.is_empty() || seed_artist_slugs.contains(&ak) || expanded_map.contains_key(&ak) {
                    return None;
                }
                let name = self
                    .store
                    .artist_summary_by_id(&fid)
                    .ok()
                    .flatten()
                    .map(|a| a.name)
                    .unwrap_or_else(|| ak.replace('-', " "));
                Some((ak, name))
            })
            .take(2)
            .collect();

        // Also let Transformer-distilled artist discoveries drive external catalog expansion.
        let mut seen_exp: HashSet<String> = unexpanded_followed.iter().map(|(ak, _)| ak.clone()).collect();
        if let Ok(neural_artists) = self.store.load_neural_artist_links(user_id, 10) {
            let mut added_neural = 0usize;
            for (ak, _w) in neural_artists {
                if added_neural >= 2 {
                    break;
                }
                if ak.is_empty()
                    || seed_artist_slugs.contains(&ak)
                    || expanded_map.contains_key(&ak)
                    || !seen_exp.insert(ak.clone())
                {
                    continue;
                }
                unexpanded_followed.push((ak.clone(), ak.replace('-', " ")));
                added_neural += 1;
            }
        }

        if unexpanded_tracks.is_empty() && unexpanded_followed.is_empty() {
            return 0;
        }

        let snapshot = expanded_map.clone();
        let track_futs = unexpanded_tracks.into_iter().map(|(ak, seed)| {
            let snap = snapshot.clone();
            async move {
                let res = tokio::time::timeout(budget, self.expand_seed_track(&seed, &snap)).await;
                (ak, res.ok())
            }
        });
        let artist_futs = unexpanded_followed.into_iter().map(|(ak, name)| async move {
            let res = tokio::time::timeout(budget, self.expand_seed_artist_name(&name)).await;
            (ak, res.ok())
        });

        let (track_outcomes, artist_outcomes) = tokio::join!(
            futures::future::join_all(track_futs),
            futures::future::join_all(artist_futs),
        );

        let mut all_tracks = Vec::new();
        let mut all_artists = Vec::new();
        let mut link_entries: Vec<(String, String, f64)> = Vec::new();
        let mut succeeded_artists = Vec::new();

        for (ak, maybe_res) in track_outcomes.into_iter().chain(artist_outcomes) {
            if let Some(exp) = maybe_res {
                if !exp.tracks.is_empty() || !exp.artist_metas.is_empty() {
                    succeeded_artists.push(ak);
                    all_tracks.extend(exp.tracks);
                    all_artists.extend(exp.artist_metas);
                    for (tid, w) in exp.track_links {
                        link_entries.push(("related_track".to_string(), tid, w));
                    }
                    for (aslug, w) in exp.artist_links {
                        link_entries.push(("related_artist".to_string(), aslug, w));
                    }
                }
            }
        }

        let mut deduped_tracks = Vec::new();
        let mut seen_ids = HashSet::new();
        for t in all_tracks {
            if seen_ids.insert(t.id.clone()) {
                deduped_tracks.push(t);
            }
        }
        let added = deduped_tracks.len();
        if !deduped_tracks.is_empty() {
            let _ = self.store.upsert_tracks(&deduped_tracks);
        }
        for art in &all_artists {
            let _ = self.store.upsert_artist_meta(art);
        }
        if !link_entries.is_empty() {
            let refs: Vec<(&str, &str, f64)> = link_entries
                .iter()
                .map(|(k, v, w)| (k.as_str(), v.as_str(), *w))
                .collect();
            let _ = self.store.upsert_taste_links(user_id, &refs);
        }
        if !succeeded_artists.is_empty() {
            for ak in succeeded_artists {
                expanded_map.insert(ak, now);
            }
            if expanded_map.len() > 80 {
                let mut pairs: Vec<(String, i64)> = expanded_map.into_iter().collect();
                pairs.sort_by(|a, b| b.1.cmp(&a.1));
                pairs.truncate(80);
                expanded_map = pairs.into_iter().collect();
            }
            if let Ok(json_str) = serde_json::to_string(&expanded_map) {
                self.store.kv_set(&kv_key, &json_str);
            }
        }
        added
    }

    async fn expand_seed_track(
        &self,
        seed: &Track,
        already_expanded: &HashMap<String, i64>,
    ) -> ExpandedSeedBatch {
        let mut out = ExpandedSeedBatch::default();
        let pa = primary_artist(&seed.artist);
        let pa_slug = slug(&pa);
        let is_ytm = seed.source == "ytm" || seed.id.starts_with("ytm:");

        let ytm_fut = async {
            if is_ytm {
                let vid = seed.id.trim_start_matches("ytm:");
                self.sources.ytm.fetch_radio_tracks(vid, 24).await
            } else {
                vec![]
            }
        };
        let seed_dz_artist_id = seed.deezer_artist_id();
        let dz_search_fut = async {
            if seed_dz_artist_id.is_none() && !pa.is_empty() {
                self.sources.deezer.search_artists(&pa, 4).await
            } else {
                vec![]
            }
        };
        let (ytm_radio, dz_artists) = tokio::join!(ytm_fut, dz_search_fut);

        let resolved_dz_artist_id = seed_dz_artist_id.or_else(|| {
            let exact = dz_artists.iter().find(|a| a.name == pa);
            let matched_dz = if !is_ytm {
                exact
                    .or_else(|| dz_artists.iter().find(|a| slug(&a.name) == pa_slug))
                    .or_else(|| dz_artists.first())
            } else {
                exact.or_else(|| dz_artists.iter().find(|a| slug(&a.name) == pa_slug))
            };
            matched_dz
                .map(|a| a.deezer_id.clone())
                .filter(|id| !id.is_empty())
        });

        let mut dz_direct_count = 0usize;
        if let Some(dz_artist_id) = resolved_dz_artist_id {
            if !dz_artist_id.is_empty() {
                let (artist_res, radio_res) = tokio::join!(
                    self.sources.deezer.fetch_artist(&dz_artist_id),
                    self.sources.deezer.fetch_artist_radio(&dz_artist_id),
                );
                if let Some((_meta, top_tracks, _albums, related)) = artist_res {
                    dz_direct_count += top_tracks.len();
                    for t in top_tracks {
                        out.push_track(t, &seed.id, 1.5, 1.25);
                    }
                    let hop_targets: Vec<ArtistSummary> = related
                        .iter()
                        .filter(|r| {
                            let rs = slug(&r.name);
                            !r.deezer_id.is_empty()
                                && rs != pa_slug
                                && !already_expanded.contains_key(&rs)
                        })
                        .take(2)
                        .cloned()
                        .collect();
                    for rel in related {
                        let rs = slug(&rel.name);
                        if !rs.is_empty() && rs != pa_slug {
                            out.artist_links.push((rs, 1.45));
                        }
                        out.artist_metas.push(rel);
                    }
                    let hop_futs = hop_targets.into_iter().map(|rel| async move {
                        let (top, radio) = tokio::join!(
                            self.sources.deezer.fetch_artist_top_tracks(&rel.deezer_id, 6),
                            self.sources.deezer.fetch_artist_radio(&rel.deezer_id),
                        );
                        (top, radio)
                    });
                    for (top, radio) in futures::future::join_all(hop_futs).await {
                        for t in top.into_iter().chain(radio) {
                            out.push_track(t, &seed.id, 1.15, 1.1);
                        }
                    }
                }
                dz_direct_count += radio_res.len();
                for t in radio_res {
                    out.push_track(t, &seed.id, 1.45, 1.3);
                }
            }
        }

        // Expand YTM radio neighbors too (especially crucial for YTM-only/indie scenes absent on Deezer).
        let mut hop_ytm_vids = Vec::new();
        let mut seen_hop_artists = HashSet::from([pa_slug.clone()]);
        for t in ytm_radio {
            let ak = slug(&primary_artist(&t.artist));
            if dz_direct_count < 10
                && hop_ytm_vids.len() < 2
                && !ak.is_empty()
                && !already_expanded.contains_key(&ak)
                && seen_hop_artists.insert(ak)
            {
                let vid = t.id.trim_start_matches("ytm:").to_string();
                if !vid.is_empty() {
                    hop_ytm_vids.push(vid);
                }
            }
            out.push_track(t, &seed.id, 1.55, 1.35);
        }
        if !hop_ytm_vids.is_empty() {
            let ytm_hops = hop_ytm_vids
                .into_iter()
                .map(|vid| async move { self.sources.ytm.fetch_radio_tracks(&vid, 18).await });
            for batch in futures::future::join_all(ytm_hops).await {
                for t in batch {
                    out.push_track(t, &seed.id, 1.15, 1.05);
                }
            }
        }

        out
    }

    async fn expand_seed_artist_name(&self, artist_name: &str) -> ExpandedSeedBatch {
        let mut out = ExpandedSeedBatch::default();
        let clean = artist_name.trim();
        if clean.is_empty() {
            return out;
        }
        let target_slug = slug(clean);
        let found = self.sources.deezer.search_artists(clean, 3).await;
        let matched = found
            .iter()
            .find(|a| slug(&a.name) == target_slug)
            .or_else(|| found.first());
        if let Some(art) = matched {
            if !art.deezer_id.is_empty() {
                let (artist_res, radio_res) = tokio::join!(
                    self.sources.deezer.fetch_artist(&art.deezer_id),
                    self.sources.deezer.fetch_artist_radio(&art.deezer_id),
                );
                if let Some((_meta, top_tracks, _albums, related)) = artist_res {
                    for t in top_tracks {
                        out.push_track(t, "", 1.5, 1.25);
                    }
                    for rel in related {
                        let rs = slug(&rel.name);
                        if !rs.is_empty() && rs != target_slug {
                            out.artist_links.push((rs, 1.45));
                        }
                        out.artist_metas.push(rel);
                    }
                }
                for t in radio_res {
                    out.push_track(t, "", 1.4, 1.3);
                }
            }
        }
        out
    }

    /// Ensures a full album is loaded into SQLite (hydrating from Deezer if it only has stubs).
    /// Finds the Deezer album ID for an album known only by name/artist, first via
    /// album search, then by locating one of its songs and taking that song's album.
    async fn find_deezer_album(
        &self,
        hint_pairs: &[(String, String)],
        titles: &[String],
    ) -> Option<String> {
        for (artist, album) in hint_pairs.iter().take(3) {
            let pa = primary_artist(artist);
            let alb_clean = album.trim();
            let q = if pa.trim().is_empty() {
                alb_clean.to_string()
            } else {
                format!("{} {}", pa, alb_clean)
            };
            let want_alb = album_match_key(alb_clean);
            let want_inst = alb_clean.to_lowercase().contains("instrumental");
            let found = self
                .sources
                .deezer
                .search_albums(&q, 15)
                .await
                .into_iter()
                .find(|a| {
                    !a.deezer_id.is_empty()
                        && a.name.to_lowercase().contains("instrumental") == want_inst
                        && album_match_key(&a.name) == want_alb
                        && album_artists_match(&pa, &a.artist)
                });
            if let Some(a) = found {
                return Some(a.deezer_id);
            }
        }

        let (artist, _) = hint_pairs.first()?;
        let pa = primary_artist(artist);
        let want_albs: HashSet<String> = hint_pairs.iter().map(|(_, alb)| album_match_key(alb)).collect();
        let mut seen_titles = HashSet::new();
        for title in titles {
            let core = crate::models::core_song_title(title);
            if core.trim().is_empty() || !seen_titles.insert(core.clone()) || seen_titles.len() > 3 {
                continue;
            }
            let q = if pa.trim().is_empty() {
                title.trim().to_string()
            } else {
                format!("{} {}", pa, title.trim())
            };
            let hit = self
                .sources
                .deezer
                .search_tracks(&q, 10)
                .await
                .into_iter()
                .find(|t| {
                    album_artists_match(&pa, &t.artist)
                        && want_albs.contains(&album_match_key(&t.album))
                        && t.deezer_album_id().is_some()
                });
            if let Some(t) = hit {
                return t.deezer_album_id();
            }
        }
        None
    }

    pub async fn get_or_hydrate_album(&self, album_id: &str) -> Option<(AlbumSummary, Vec<Track>)> {
        self.get_or_hydrate_album_inner(album_id, None, None, None, false)
            .await
    }

    /// Resolves an album to its full, authoritative tracklist. The local library
    /// only holds songs that were played or cached, so whenever the album can be
    /// matched on Deezer its complete tracklist is used, with local copies kept
    /// for songs Deezer also lists (so better-quality versions still win).
    pub async fn get_or_hydrate_album_with_hints(
        &self,
        album_id: &str,
        name_hint: Option<&str>,
        artist_hint: Option<&str>,
        track_hint: Option<&str>,
    ) -> Option<(AlbumSummary, Vec<Track>)> {
        self.get_or_hydrate_album_inner(album_id, name_hint, artist_hint, track_hint, false)
            .await
    }

    /// Fast-path album hydration for direct play actions: returns immediately
    /// from SQLite when the official tracklist is already stored, and skips
    /// blocking up to 3.2s on speculative Soulseek searches (callers can invoke
    /// `upgrade_album_from_soulseek_background` asynchronously).
    pub async fn get_or_hydrate_album_fast(
        &self,
        album_id: &str,
        name_hint: Option<&str>,
        artist_hint: Option<&str>,
        track_hint: Option<&str>,
    ) -> Option<(AlbumSummary, Vec<Track>)> {
        self.get_or_hydrate_album_inner(album_id, name_hint, artist_hint, track_hint, true)
            .await
    }

    pub async fn upgrade_album_from_soulseek_background(
        &self,
        meta: &AlbumSummary,
        tracks: Vec<Track>,
    ) {
        let _ = self.upgrade_album_from_soulseek(meta, tracks).await;
    }

    async fn get_or_hydrate_album_inner(
        &self,
        album_id: &str,
        name_hint: Option<&str>,
        artist_hint: Option<&str>,
        track_hint: Option<&str>,
        fast: bool,
    ) -> Option<(AlbumSummary, Vec<Track>)> {
        let existing_tracks = self.store.album_tracks(album_id).unwrap_or_default();
        let existing_meta = self.store.get_album_meta(album_id).ok().flatten();

        if fast {
            if let Some(ref meta) = existing_meta {
                if meta.song_count > 0 && existing_tracks.len() >= meta.song_count {
                    return Some((meta.clone(), existing_tracks));
                }
            }
        }

        let local_summary = self.store.album_summary_by_id(0, album_id).ok().flatten();

        // Hydrate iTunes-discovered albums (e.g. from New Releases when Deezer is
        // blocked) directly via Apple's Lookup API, preserving full tracklists and
        // YTM/Soulseek playback resolution.
        if let Some(it_id) = album_id
            .strip_prefix("al:it-")
            .and_then(|s| s.parse::<u64>().ok())
        {
            if let Some((mut meta, it_tracks)) = self.sources.itunes.fetch_album(it_id).await {
                if let Some(ref em) = existing_meta {
                    if !em.artist_id.is_empty() {
                        meta.artist_id = em.artist_id.clone();
                    }
                }
                let _ = self.store.upsert_album_meta(&meta);
                let _ = self.store.upsert_tracks(&it_tracks);
                let official_keys: HashSet<String> =
                    it_tracks.iter().map(|t| t.album_track_key()).collect();
                let local_same: Vec<Track> = existing_tracks
                    .into_iter()
                    .filter(|t| official_keys.contains(&t.album_track_key()))
                    .collect();
                let merged = merge_album_tracks(it_tracks, local_same);
                let final_tracks = if fast {
                    merged
                } else {
                    self.upgrade_album_from_soulseek(&meta, merged).await
                };
                if !final_tracks.is_empty() {
                    meta.song_count = final_tracks.len();
                    let dur: i32 = final_tracks.iter().map(|t| t.duration.max(0)).sum();
                    if dur > 0 {
                        meta.duration = dur;
                    }
                }
                return Some((meta, final_tracks));
            }
        }

        // Known Deezer album ID, if any local data already links one.
        let mut dz_album_id = existing_meta
            .as_ref()
            .map(|m| m.deezer_id.clone())
            .filter(|id| !id.is_empty())
            .or_else(|| {
                local_summary
                    .as_ref()
                    .map(|s| s.deezer_id.clone())
                    .filter(|id| !id.is_empty())
            })
            .or_else(|| album_id.strip_prefix("dz:").map(|s| s.to_string()))
            .or_else(|| existing_tracks.iter().find_map(|t| t.deezer_album_id()))
            .filter(|id| !id.is_empty() && id != "0");

        // (artist, album name) pairs to search with. The client's hint comes from
        // the exact song the user clicked, so it is the most trustworthy.
        let mut hint_pairs: Vec<(String, String)> = Vec::new();
        if let Some(nh) = name_hint.map(str::trim).filter(|s| !s.is_empty()) {
            let ah = artist_hint.map(str::trim).unwrap_or("");
            hint_pairs.push((ah.to_string(), nh.to_string()));
        }
        if let Some(m) = existing_meta.as_ref() {
            hint_pairs.push((m.artist.clone(), m.name.clone()));
        }
        if let Some(s) = local_summary.as_ref() {
            hint_pairs.push((s.artist.clone(), s.name.clone()));
        }
        if let Some(t) = existing_tracks.first() {
            hint_pairs.push((t.artist.clone(), t.album.clone()));
        }
        hint_pairs.retain(|(_, alb)| !alb.trim().is_empty());
        {
            let mut seen = HashSet::new();
            hint_pairs.retain(|(art, alb)| {
                seen.insert((slug(&primary_artist(art)), album_match_key(alb)))
            });
        }
        let hint_pair = hint_pairs.first().cloned();

        if dz_album_id.is_none() {
            let mut titles: Vec<String> = Vec::new();
            if let Some(th) = track_hint.map(str::trim).filter(|s| !s.is_empty()) {
                titles.push(th.to_string());
            }
            titles.extend(existing_tracks.iter().take(3).map(|t| t.title.clone()));
            dz_album_id = self.find_deezer_album(&hint_pairs, &titles).await;
        }
        if dz_album_id.is_none() {
            if let Some(rest) = album_id.strip_prefix("al:dz-") {
                if let Some((dz_aid, alb_slug)) = rest.split_once('-') {
                    if !dz_aid.is_empty() && !alb_slug.is_empty() {
                        if let Some((_, _, albs, _)) = self.sources.deezer.fetch_artist(dz_aid).await {
                            dz_album_id = albs
                                .into_iter()
                                .find(|a| slug(&a.name) == alb_slug && !a.deezer_id.is_empty())
                                .map(|a| a.deezer_id);
                        }
                    }
                }
            }
        }

        if let Some(ref dz_id) = dz_album_id {
            if let Some((mut meta, dz_tracks)) = self.sources.deezer.fetch_album(dz_id).await {
                let _ = self.store.upsert_album_meta(&meta);
                let _ = self.store.upsert_tracks(&dz_tracks);
                let stored = self
                    .store
                    .album_tracks(&meta.id)
                    .ok()
                    .filter(|v| !v.is_empty())
                    .unwrap_or(dz_tracks);
                // Fold in local copies (e.g. the version being played) of songs on
                // the official tracklist; don't append unrelated local strays.
                let official_keys: HashSet<String> =
                    stored.iter().map(|t| t.album_track_key()).collect();
                let local_same: Vec<Track> = existing_tracks
                    .into_iter()
                    .filter(|t| official_keys.contains(&t.album_track_key()))
                    .collect();
                let merged = merge_album_tracks(stored, local_same);
                let final_tracks = if fast {
                    merged
                } else {
                    self.upgrade_album_from_soulseek(&meta, merged).await
                };
                if !final_tracks.is_empty() {
                    meta.song_count = final_tracks.len();
                    let dur: i32 = final_tracks.iter().map(|t| t.duration.max(0)).sum();
                    if dur > 0 {
                        meta.duration = dur;
                    }
                }
                return Some((meta, final_tracks));
            }
        }

        // Deezer couldn't place it: search every source for the album's songs and
        // merge them with what's stored locally.
        let mut tracks = existing_tracks;
        if let Some((ref artist_name, ref album_name)) = hint_pair {
            let alb_clean = album_name.trim();
            let pa = primary_artist(artist_name);
            let q = if pa.trim().is_empty() {
                alb_clean.to_string()
            } else {
                format!("{} {}", pa, alb_clean)
            };
            let want_alb = album_match_key(alb_clean);
            let found: Vec<Track> = self
                .sources
                .search_all(&q, 25)
                .await
                .into_iter()
                .filter(|t| {
                    album_artists_match(&pa, &t.artist)
                        && (album_match_key(&t.album) == want_alb
                            || (tracks.is_empty()
                                && slug(&crate::models::core_song_title(&t.title)) == want_alb))
                })
                .collect();
            if !found.is_empty() {
                let _ = self.store.upsert_tracks(&found);
                tracks = merge_album_tracks(tracks, found);
            }
        }

        if let Some(summary) = local_summary.or_else(|| self.store.album_summary_by_id(0, album_id).ok().flatten()) {
            let tracks = if tracks.is_empty() {
                self.store.album_tracks(&summary.id).unwrap_or_default()
            } else {
                tracks
            };
            let upgraded = if fast {
                tracks
            } else {
                self.upgrade_album_from_soulseek(&summary, tracks).await
            };
            return Some((summary, upgraded));
        }

        if let Some((_, album_name)) = hint_pair {
            let alb_clean = album_name.trim();
            if !alb_clean.is_empty() && !tracks.is_empty() {
                let first = &tracks[0];
                let summary = AlbumSummary {
                    id: first.album_id(),
                    name: alb_clean.to_string(),
                    artist: first.artist.clone(),
                    artist_id: first.artist_id(),
                    cover_id: first.id.clone(),
                    cover_url: first.cover_url.clone(),
                    genre: first.genre.clone(),
                    year: first.year,
                    label: String::new(),
                    deezer_id: String::new(),
                    song_count: tracks.len(),
                    duration: tracks.iter().map(|t| t.duration.max(0)).sum(),
                    play_count: 0,
                    last_played: 0,
                    created: first.created,
                };
                let _ = self.store.upsert_album_meta(&summary);
                return Some((summary, tracks));
            }
        }
        None
    }

    async fn upgrade_album_from_soulseek(
        &self,
        meta: &AlbumSummary,
        tracks: Vec<Track>,
    ) -> Vec<Track> {
        if tracks.is_empty()
            || tracks.iter().all(|t| t.is_lossless())
            || !self.should_probe_slsk(&format!("al:{}", meta.id))
        {
            return tracks;
        }
        use crate::sources::MusicSource;
        let q = format!("{} {} flac", primary_artist(&meta.artist), meta.name);
        let hits = match tokio::time::timeout(
            std::time::Duration::from_millis(3200),
            self.sources.soulseek.search(&q, 25),
        )
        .await
        {
            Ok(v) => v,
            Err(_) => return tracks,
        };

        let mut base_by_key: HashMap<String, Track> = HashMap::new();
        for t in &tracks {
            base_by_key
                .entry(t.album_track_key())
                .or_insert_with(|| t.clone());
        }

        let mut upgraded = Vec::new();
        for mut h in hits {
            if !h.is_lossless() {
                continue;
            }
            if let Some(base) = base_by_key.get(&h.album_track_key()) {
                enrich_slsk_hit_with_base(&mut h, base, true);
                upgraded.push(h);
            }
        }
        if upgraded.is_empty() {
            return tracks;
        }
        let _ = self.store.upsert_tracks(&upgraded);
        merge_album_tracks(tracks, upgraded)
    }

    async fn upgrade_artist_from_soulseek(
        &self,
        meta: &ArtistSummary,
        tracks: Vec<Track>,
    ) -> Vec<Track> {
        if tracks.iter().any(|t| t.is_lossless())
            || !self.should_probe_slsk(&format!("ar:{}", meta.id))
        {
            return tracks;
        }
        use crate::sources::MusicSource;
        let q = format!("{} flac", meta.name);
        let hits = match tokio::time::timeout(
            std::time::Duration::from_millis(3200),
            self.sources.soulseek.search(&q, 25),
        )
        .await
        {
            Ok(v) => v,
            Err(_) => return tracks,
        };

        let mut base_by_key: HashMap<String, Track> = HashMap::new();
        for t in &tracks {
            base_by_key.entry(t.song_key()).or_insert_with(|| t.clone());
        }

        let mut matched = Vec::new();
        for mut h in hits {
            if !h.is_lossless() || !is_same_or_featured_artist(&h.artist, &meta.name) {
                continue;
            }
            if let Some(base) = base_by_key.get(&h.song_key()) {
                enrich_slsk_hit_with_base(&mut h, base, false);
                matched.push(h);
            } else if tracks.is_empty() {
                if !meta.deezer_id.is_empty() {
                    h.set_deezer_artist_id(&meta.deezer_id);
                }
                matched.push(h);
            }
        }
        if matched.is_empty() {
            return tracks;
        }
        let _ = self.store.upsert_tracks(&matched);
        merge_tracks_dedup(tracks, matched)
    }

    /// Loads full artist profile + top tracks + albums + related artists, hydrating live from Deezer & YTM when available.
    pub async fn get_or_hydrate_artist(
        &self,
        artist_id: &str,
        user_id: i64,
    ) -> Option<(ArtistSummary, Vec<Track>, Vec<AlbumSummary>, Vec<ArtistSummary>)> {
        self.get_or_hydrate_artist_with_hint(artist_id, user_id, None).await
    }

    pub async fn get_or_hydrate_artist_with_hint(
        &self,
        artist_id: &str,
        user_id: i64,
        name_hint: Option<&str>,
    ) -> Option<(ArtistSummary, Vec<Track>, Vec<AlbumSummary>, Vec<ArtistSummary>)> {
        let local_summary = self.store.artist_summary_by_id(artist_id).ok().flatten();
        let stored_for_id = self.store.artist_tracks(artist_id).unwrap_or_default();
        let hint_clean = name_hint
            .map(primary_artist)
            .filter(|n| !n.trim().is_empty());
        let local_summary_name = local_summary
            .as_ref()
            .map(|a| a.name.trim().to_string())
            .filter(|n| !n.is_empty());
        let stored_solo_name = stored_for_id
            .iter()
            .map(|t| primary_artist(&t.artist))
            .find(|n| !n.trim().is_empty() && crate::models::is_solo_artist_name(n));
        let stored_any_name = stored_for_id
            .first()
            .map(|t| primary_artist(&t.artist))
            .filter(|n| !n.trim().is_empty());
        let search_name = local_summary_name
            .as_ref()
            .filter(|n| crate::models::is_solo_artist_name(n))
            .cloned()
            .or_else(|| {
                hint_clean
                    .as_ref()
                    .filter(|n| crate::models::is_solo_artist_name(n))
                    .cloned()
            })
            .or(stored_solo_name)
            .or_else(|| {
                local_summary_name
                    .as_deref()
                    .map(primary_artist)
                    .filter(|n| !n.trim().is_empty())
            })
            .or(stored_any_name)
            .or(hint_clean)
            .unwrap_or_else(|| {
                if artist_id.starts_with("ar:dz:") || artist_id.starts_with("ar:ytm:") {
                    String::new()
                } else {
                    artist_id.trim_start_matches("ar:").replace('-', " ")
                }
            });

        let mut dz_artist_id = artist_id
            .strip_prefix("ar:dz:")
            .or_else(|| artist_id.strip_prefix("dz:"))
            .map(|s| s.trim().to_string())
            .filter(|id| !id.is_empty())
            .or_else(|| {
                local_summary
                    .as_ref()
                    .map(|a| a.deezer_id.clone())
                    .filter(|id| !id.is_empty())
            })
            .or_else(|| stored_for_id.iter().find_map(Track::deezer_artist_id));

        let pick_best_dz_candidate = |candidates: Vec<ArtistSummary>, want_name: &str| -> Option<String> {
            let want_trimmed = want_name.trim();
            let want_slug = slug(want_trimmed);
            let allow_ci = crate::models::allow_case_insensitive_artist_merge(want_trimmed);
            let mut matching: Vec<(bool, u64, usize, bool, ArtistSummary)> = candidates
                .into_iter()
                .filter(|a| {
                    !a.deezer_id.is_empty()
                        && (a.id == artist_id
                            || a.name.trim() == want_trimmed
                            || (allow_ci && !want_slug.is_empty() && slug(&a.name) == want_slug))
                })
                .map(|a| {
                    let exact = a.name.trim() == want_trimmed;
                    let fans = parse_deezer_fans(&a.bio).unwrap_or(0);
                    let albs = a.album_count;
                    let has_art = !a.cover_url.trim().is_empty();
                    (exact, fans, albs, has_art, a)
                })
                .collect();
            matching.sort_by(|a, b| {
                b.0.cmp(&a.0)
                    .then_with(|| b.1.cmp(&a.1))
                    .then_with(|| b.2.cmp(&a.2))
                    .then_with(|| b.3.cmp(&a.3))
            });
            matching.into_iter().next().map(|(_, _, _, _, a)| a.deezer_id)
        };

        let mut searched_by_name = false;
        if dz_artist_id.is_none() && !search_name.is_empty() {
            searched_by_name = true;
            let candidates = self.sources.deezer.search_artists(&search_name, 8).await;
            dz_artist_id = pick_best_dz_candidate(candidates, &search_name);
        }

        let mut hydrated = if let Some(ref dz_id) = dz_artist_id {
            self.sources.deezer.fetch_artist(dz_id).await
        } else {
            None
        };

        // Recover if an explicit/cached Deezer ID pointed at an empty stub profile (0 fans / 0 tracks / 0 albums).
        let is_empty_dz_stub = hydrated
            .as_ref()
            .map_or(true, |(_, tops, albs, _)| tops.is_empty() && albs.is_empty());
        if is_empty_dz_stub && !searched_by_name && !search_name.is_empty() {
            let candidates = self.sources.deezer.search_artists(&search_name, 8).await;
            if let Some(better_id) = pick_best_dz_candidate(candidates, &search_name) {
                if dz_artist_id.as_deref() != Some(better_id.as_str()) {
                    if let Some(better_res) = self.sources.deezer.fetch_artist(&better_id).await {
                        if !better_res.1.is_empty() || !better_res.2.is_empty() {
                            hydrated = Some(better_res);
                        }
                    }
                }
            }
        }

        let mut meta = if let Some((ref m, _, _, _)) = hydrated {
            m.clone()
        } else if let Some(mut s) = local_summary.clone() {
            if s.deezer_id.is_empty()
                && !crate::models::is_solo_artist_name(&s.name)
                && !search_name.trim().is_empty()
            {
                s.name = search_name.clone();
            }
            s
        } else if !stored_for_id.is_empty() || !search_name.trim().is_empty() {
            let fallback_name = if !search_name.trim().is_empty() {
                search_name.clone()
            } else {
                primary_artist(&stored_for_id[0].artist)
            };
            let fallback_id = if let Some(ref dz_id) = dz_artist_id {
                format!("ar:dz:{}", dz_id)
            } else if let Some(first) = stored_for_id.first() {
                first.artist_id()
            } else {
                artist_id.to_string()
            };
            ArtistSummary {
                id: fallback_id,
                name: fallback_name,
                cover_url: stored_for_id
                    .iter()
                    .find_map(|t| (!t.cover_url.trim().is_empty()).then(|| t.cover_url.clone()))
                    .unwrap_or_default(),
                bio: String::new(),
                deezer_id: dz_artist_id.clone().unwrap_or_default(),
                album_count: 0,
                song_count: stored_for_id.len(),
            }
        } else {
            return None;
        };

        let had_hydrated = hydrated.is_some();
        let mut dz_top_tracks = Vec::new();
        let mut dz_albums = Vec::new();
        let mut related = Vec::new();
        if let Some((_, top_tracks, albs, rel)) = hydrated {
            dz_top_tracks = top_tracks;
            dz_albums = albs;
            related = rel;
            let _ = self.store.upsert_artist_meta(&meta);
            if !dz_top_tracks.is_empty() {
                let _ = self.store.upsert_tracks(&dz_top_tracks);
            }
            for a in &dz_albums {
                let _ = self.store.upsert_album_meta(a);
            }
            for r in &related {
                let _ = self.store.upsert_artist_meta(r);
            }
        }

        let mut combined_local = self.store.artist_tracks(&meta.id).unwrap_or_default();
        if !stored_for_id.is_empty() {
            combined_local.extend(stored_for_id);
        }
        let mut merged_tracks = merge_tracks_dedup(dz_top_tracks, combined_local);

        // Deezer's artist/top endpoint caps at 50 tracks, so an artist page
        // would only show a handful of songs until the user happened to search
        // for the rest (which is how they entered the local catalog). Walk the
        // artist's albums and collect their tracks so the full discography is
        // present on the first visit. Results are persisted below, making
        // subsequent loads instant.
        let album_ids = artist_album_ids(&dz_albums, 60);
        if !album_ids.is_empty() {
            let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(8));
            let mut tasks = Vec::with_capacity(album_ids.len());
            for id in album_ids {
                let dz = self.sources.deezer.clone();
                let sem = sem.clone();
                tasks.push(tokio::spawn(async move {
                    let _permit = sem.acquire().await.ok()?;
                    let (_album, tracks) = dz.fetch_album(&id).await?;
                    Some(tracks)
                }));
            }
            let mut album_tracks = Vec::new();
            for task in tasks {
                if let Ok(Some(tracks)) = task.await {
                    album_tracks.extend(tracks);
                }
            }
            if !album_tracks.is_empty() {
                let _ = self.store.upsert_tracks(&album_tracks);
                merged_tracks = merge_tracks_dedup(merged_tracks, album_tracks);
            }
        }

        // When Deezer returned few/no tracks (or was offline/rate-limited), supplement from YTM & Deezer track search.
        let artist_name_for_search = if !meta.name.trim().is_empty() {
            meta.name.clone()
        } else {
            search_name
        };
        if merged_tracks.len() < 15 && !artist_name_for_search.trim().is_empty() {
            use crate::sources::MusicSource;
            let want_dz = (!meta.deezer_id.is_empty()).then(|| meta.deezer_id.as_str());
            let songs_query = format!("{} songs", artist_name_for_search);
            let (ytm_hits, dz_hits, ytm_songs, dz_songs) = tokio::join!(
                tokio::time::timeout(
                    std::time::Duration::from_millis(3500),
                    self.sources.ytm.search(&artist_name_for_search, 50),
                ),
                tokio::time::timeout(
                    std::time::Duration::from_millis(3500),
                    self.sources.deezer.search_tracks(&artist_name_for_search, 50),
                ),
                tokio::time::timeout(
                    std::time::Duration::from_millis(3500),
                    self.sources.ytm.search(&songs_query, 50),
                ),
                tokio::time::timeout(
                    std::time::Duration::from_millis(3500),
                    self.sources.deezer.search_tracks(&songs_query, 50),
                ),
            );
            let mut extra = Vec::new();
            let meta_pa = primary_artist(&meta.name);
            for mut t in dz_hits
                .unwrap_or_default()
                .into_iter()
                .chain(ytm_hits.unwrap_or_default())
                .chain(dz_songs.unwrap_or_default())
                .chain(ytm_songs.unwrap_or_default())
            {
                if !is_same_or_featured_artist(&t.artist, &artist_name_for_search) {
                    continue;
                }
                if let (Some(want), Some(got)) = (want_dz, t.deezer_artist_id().as_deref()) {
                    if want != got {
                        continue;
                    }
                }
                if let Some(want) = want_dz {
                    if t.deezer_artist_id().is_none()
                        && !meta_pa.is_empty()
                        && primary_artist(&t.artist) == meta_pa
                    {
                        t.set_deezer_artist_id(want);
                    }
                }
                extra.push(t);
            }
            if !extra.is_empty() {
                let _ = self.store.upsert_tracks(&extra);
                merged_tracks = merge_tracks_dedup(merged_tracks, extra);
            }
        }

        let upgraded_tracks = self
            .upgrade_artist_from_soulseek(&meta, merged_tracks)
            .await;
        let final_tracks = merge_tracks_dedup(upgraded_tracks, vec![]);
        let merged_albums = merge_albums_dedup(
            dz_albums,
            self.store.artist_albums(user_id, &meta.id).unwrap_or_default(),
        );
        if meta.song_count < final_tracks.len() {
            meta.song_count = final_tracks.len();
        }
        if meta.album_count < merged_albums.len() {
            meta.album_count = merged_albums.len();
        }
        if meta.cover_url.trim().is_empty() {
            if let Some(c) = final_tracks
                .iter()
                .find_map(|t| (!t.cover_url.trim().is_empty()).then(|| t.cover_url.clone()))
            {
                meta.cover_url = c;
            }
        }
        if final_tracks.is_empty() && merged_albums.is_empty() && local_summary.is_none() && !had_hydrated {
            return None;
        }
        Some((meta, final_tracks, merged_albums, related))
    }

    /// Collects related tracks for an artist page from related artists' catalogs and track radio.
    pub async fn artist_related_tracks(
        &self,
        anchor: &ArtistSummary,
        related_artists: &[ArtistSummary],
        anchor_tracks: &[Track],
        limit: usize,
    ) -> Vec<Track> {
        if limit == 0 {
            return vec![];
        }
        let anchor_slug = slug(&primary_artist(&anchor.name));
        let mut pool: Vec<Track> = Vec::new();

        for rel in related_artists.iter().take(6) {
            let local_rel = self.store.artist_tracks(&rel.id).unwrap_or_default();
            pool.extend(local_rel.into_iter().take(4));
        }

        if pool.len() < limit && !related_artists.is_empty() {
            let fetch_targets: Vec<String> = related_artists
                .iter()
                .filter(|r| !r.deezer_id.is_empty())
                .take(4)
                .map(|r| r.deezer_id.clone())
                .collect();
            let futs = fetch_targets.into_iter().map(|dz_id| {
                let dz = self.sources.deezer.clone();
                async move {
                    tokio::time::timeout(
                        std::time::Duration::from_millis(2500),
                        dz.fetch_artist_top_tracks(&dz_id, 6),
                    )
                    .await
                    .unwrap_or_default()
                }
            });
            let batches: Vec<Vec<Track>> = futures::future::join_all(futs).await;
            let fetched: Vec<Track> = batches.into_iter().flatten().collect();
            if !fetched.is_empty() {
                let _ = self.store.upsert_tracks(&fetched);
                pool.extend(fetched);
            }
        }

        if pool.len() < 4 && !anchor.deezer_id.is_empty() {
            if let Ok(radio) = tokio::time::timeout(
                std::time::Duration::from_millis(2500),
                self.sources.deezer.fetch_artist_radio(&anchor.deezer_id),
            )
            .await
            {
                if !radio.is_empty() {
                    let _ = self.store.upsert_tracks(&radio);
                    pool.extend(radio);
                }
            }
        }

        let filtered: Vec<Track> = pool
            .into_iter()
            .filter(|t| {
                let ts = slug(&primary_artist(&t.artist));
                !ts.is_empty()
                    && ts != anchor_slug
                    && !is_same_or_featured_artist(&t.artist, &anchor.name)
                    && !anchor_tracks.iter().any(|at| at.song_key() == t.song_key())
            })
            .collect();
        dedup_and_interleave(filtered, limit, 2)
    }

    pub async fn similar_songs_async(&self, seed: &Track, count: usize) -> Vec<Track> {
        let mut out = Vec::new();
        let mut seen_keys: HashSet<String> = HashSet::new();
        seen_keys.insert(seed.song_key());

        let pa = primary_artist(&seed.artist);
        let pa_slug = slug(&pa);
        let is_ytm_seed = seed.source == "ytm" || seed.id.starts_with("ytm:");
        let target_fetch = (count * 2).max(24);

        // 1. For YouTube Music tracks, seed directly from YouTube Music's song radio mix.
        if is_ytm_seed {
            let vid = seed.id.trim_start_matches("ytm:");
            let radio = self.sources.ytm.fetch_radio_tracks(vid, target_fetch).await;
            self.push_radio_tracks(&mut seen_keys, &mut out, radio);
        }

        // 2. For Deezer/local/archive seeds (or when YTM radio yielded almost nothing),
        //    match an exact Deezer artist and pull artist radio.
        if (!is_ytm_seed || out.len() < 4) && !pa.is_empty() {
            let resolved_dz_id = match seed.deezer_artist_id() {
                Some(id) => Some(id),
                None => {
                    let artists = self.sources.deezer.search_artists(&pa, 5).await;
                    let exact = artists.iter().find(|a| a.name == pa);
                    let matched = if !is_ytm_seed {
                        exact
                            .or_else(|| artists.iter().find(|a| slug(&a.name) == pa_slug))
                            .or_else(|| artists.first())
                    } else {
                        exact.or_else(|| artists.iter().find(|a| slug(&a.name) == pa_slug))
                    };
                    matched
                        .map(|a| a.deezer_id.clone())
                        .filter(|id| !id.is_empty())
                }
            };
            if let Some(dz_id) = resolved_dz_id {
                let radio = self.sources.deezer.fetch_artist_radio(&dz_id).await;
                self.push_radio_tracks(&mut seen_keys, &mut out, radio);
            }
        }

        // 3. If non-YTM seed had insufficient Deezer radio, resolve on YTM for track-level radio.
        if !is_ytm_seed && out.len() < 6 {
            if let Some(vid) = self
                .sources
                .ytm
                .find_best_video_for_track(&pa, &seed.title, seed.duration)
                .await
            {
                let radio = self.sources.ytm.fetch_radio_tracks(&vid, target_fetch).await;
                self.push_radio_tracks(&mut seen_keys, &mut out, radio);
            }
        }

        // 4. Supplement from local catalog matching artist, genre, or moods.
        if out.len() < count {
            self.supplement_radio_from_catalog(seed, count, &mut seen_keys, &mut out);
        }

        dedup_and_interleave(out, count, 3)
    }

    fn push_radio_tracks(
        &self,
        seen_keys: &mut HashSet<String>,
        out: &mut Vec<Track>,
        radio: Vec<Track>,
    ) {
        if radio.is_empty() {
            return;
        }
        let _ = self.store.upsert_tracks(&radio);
        for t in radio {
            if seen_keys.insert(t.song_key()) {
                out.push(t);
            }
        }
    }

    fn supplement_radio_from_catalog(
        &self,
        seed: &Track,
        count: usize,
        seen_keys: &mut HashSet<String>,
        out: &mut Vec<Track>,
    ) {
        let seed_moods: HashSet<Mood> = seed.mood_list().into_iter().collect();
        let seed_genres: HashSet<String> = split_genres(&seed.genre).into_iter().collect();
        let pa_slug = slug(&primary_artist(&seed.artist));
        let require_affinity = !out.is_empty();
        let all = self.store.all_tracks(500, 0).unwrap_or_default();

        let mut scored: Vec<(i32, Track)> = all
            .into_iter()
            .filter(|t| !seen_keys.contains(&t.song_key()))
            .filter_map(|t| {
                let mut aff = 0i32;
                let same_artist = !pa_slug.is_empty() && slug(&primary_artist(&t.artist)) == pa_slug;
                if same_artist {
                    aff += 45;
                }
                let t_genres = split_genres(&t.genre);
                let genre_overlap = !seed_genres.is_empty()
                    && t_genres.iter().any(|g| seed_genres.contains(g));
                if genre_overlap {
                    aff += 60;
                }
                let conflicting_explicit_genres = !same_artist
                    && !seed_genres.is_empty()
                    && !t_genres.is_empty()
                    && !genre_overlap;
                if !conflicting_explicit_genres {
                    let shared_m = t
                        .mood_list()
                        .iter()
                        .filter(|m| seed_moods.contains(m))
                        .count() as i32;
                    let legacy_fallback_collision = !same_artist
                        && seed.genre.trim().is_empty()
                        && t.genre.trim().is_empty()
                        && seed.moods == "upbeat,chill"
                        && t.moods == "upbeat,chill";
                    aff += shared_m * if legacy_fallback_collision { 4 } else { 22 };
                }
                if require_affinity && aff == 0 {
                    return None;
                }
                Some((aff + t.quality_score() / 50, t))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0));
        for (_, t) in scored {
            if out.len() >= count * 2 {
                break;
            }
            if seen_keys.insert(t.song_key()) {
                out.push(t);
            }
        }
    }

fn is_title_word_playlist(tracks: &[Track], mood: Mood) -> bool {
    let needle = mood.label().to_lowercase();
    if tracks.len() < 3 || needle.is_empty() {
        return false;
    }
    let hits = tracks
        .iter()
        .filter(|t| contains_term(&t.title.to_lowercase(), &needle))
        .count();
    hits >= 3 && hits * 10 >= tracks.len() * 3
}

    /// Curates and persists verified tracks for a Moods & Vibes category from curated playlists
    /// and compatible genre charts, returning the freshly curated tracks.
    pub async fn curate_mood_pool(&self, mood: Mood) -> Vec<Track> {
        let queries = mood_playlist_queries(mood);
        let genre_names = mood_deezer_genre_names(mood);
        let pl_futs = queries
            .iter()
            .map(|q| self.sources.fetch_curated_mix_tracks(q, 3, 25));
        let genres_fut = async {
            if genre_names.is_empty() {
                Vec::new()
            } else {
                self.sources.deezer.fetch_genres().await
            }
        };
        let remote_res = tokio::time::timeout(
            std::time::Duration::from_secs(7),
            futures::future::join(futures::future::join_all(pl_futs), genres_fut),
        )
        .await;

        let mut remote_batches: Vec<(Option<String>, String, Vec<Track>)> = Vec::new();
        if let Ok((pl_results, dz_genres)) = remote_res {
            for (pl_title, batch) in pl_results.into_iter().flatten() {
                if is_curated_playlist_for_mood(&pl_title, mood)
                    && !batch.is_empty()
                    && !Self::is_title_word_playlist(&batch, mood)
                {
                    remote_batches.push((Some(pl_title), String::new(), batch));
                }
            }
            let matching_genres: Vec<_> = dz_genres
                .into_iter()
                .filter(|g| {
                    let gl = g.name.to_lowercase();
                    genre_names.iter().any(|want| contains_term(&gl, want))
                })
                .take(2)
                .collect();
            if !matching_genres.is_empty() {
                let g_futs = matching_genres.into_iter().map(|g| {
                    let dz = self.sources.deezer.clone();
                    async move {
                        let chart = tokio::time::timeout(
                            std::time::Duration::from_secs(4),
                            dz.fetch_genre_chart_tracks(g.id, &g.name, 25),
                        )
                        .await
                        .unwrap_or_default();
                        (g.name, chart)
                    }
                });
                for (g_name, chart) in futures::future::join_all(g_futs).await {
                    if !chart.is_empty() {
                        remote_batches.push((None, g_name, chart));
                    }
                }
            }
        }

        let mut curated_tracks: Vec<Track> = Vec::new();
        let mut seen_ids = HashSet::new();
        if !remote_batches.is_empty() {
            // Deezer playlist tracks arrive without album genres; enrich them before
            // mood-clash validation so off-mood tracks (e.g. Country/Soundtrack/50s Rock)
            // cannot slip through an empty-genre check and get stamped with this mood.
            let mut flat_for_enrich: Vec<Track> = remote_batches
                .iter()
                .flat_map(|(_, genre_label, batch)| {
                    if genre_label.is_empty() {
                        batch.clone()
                    } else {
                        Vec::new()
                    }
                })
                .collect();
            if !flat_for_enrich.is_empty() {
                let _ = tokio::time::timeout(
                    std::time::Duration::from_millis(2500),
                    self.sources.deezer.enrich_track_genres(&mut flat_for_enrich),
                )
                .await;
                let enriched_by_id: HashMap<String, (String, i32)> = flat_for_enrich
                    .into_iter()
                    .filter(|t| !t.genre.trim().is_empty() || t.year > 0)
                    .map(|t| (t.id, (t.genre, t.year)))
                    .collect();
                for (_, _, batch) in &mut remote_batches {
                    for t in batch.iter_mut() {
                        if let Some((g, y)) = enriched_by_id.get(&t.id) {
                            if t.genre.trim().is_empty() && !g.trim().is_empty() {
                                t.genre = g.clone();
                            }
                            if t.year <= 0 && *y > 0 {
                                t.year = *y;
                            }
                        }
                    }
                }
            }

            let remote_artists: Vec<String> = remote_batches
                .iter()
                .flat_map(|(_, _, batch)| batch.iter().map(|t| t.artist.clone()))
                .collect();
            let artist_genres = self
                .store
                .known_artist_genres_for(&remote_artists)
                .unwrap_or_default();

            let mut to_upsert = Vec::new();
            for (pl_title, genre_label, batch) in remote_batches {
                for mut t in batch {
                    if !genre_label.is_empty() && t.genre.trim().is_empty() {
                        t.genre = genre_label.clone();
                    }
                    let ag = artist_genres
                        .get(&primary_artist(&t.artist).to_lowercase())
                        .map(|s| s.as_str())
                        .unwrap_or("");
                    if !is_track_compatible_with_mood(&t, ag, mood) {
                        continue;
                    }
                    let base_moods = classify_moods_str(&t.title, &t.album, &t.genre, ag);
                    let meta_low = format!("{} {}", t.genre, ag).to_lowercase();
                    let strict_genre_match = has_mood_meta_match(&meta_low, mood);
                    if !strict_genre_match && is_mood_title_trap(&t, mood) {
                        continue;
                    }
                    // Global Moods & Vibes pools are built from English queries and shared
                    // across all listeners; keep non-Latin-script tracks (e.g. Arabic, Indic,
                    // Cyrillic, CJK) out of the global pool so they don't leak into unrelated
                    // listeners' mixes (scene-specific listeners get theirs via personal history
                    // and `curate_user_mood_tracks`).
                    if has_any_non_latin_script(&t) {
                        continue;
                    }
                    // Reject obscure / AI-slop Deezer uploads from user playlists when
                    // they lack a direct genre match for this mood.
                    if !strict_genre_match && is_low_rank_deezer_track(&t, 30_000) {
                        continue;
                    }
                    if pl_title.is_some() || strict_genre_match {
                        t.moods = append_mood_slug(&base_moods, mood);
                        to_upsert.push(t.clone());
                        if seen_ids.insert(t.id.clone()) {
                            curated_tracks.push(t);
                        }
                    } else {
                        t.moods = base_moods;
                        to_upsert.push(t);
                    }
                }
            }
            if !to_upsert.is_empty() {
                let _ = self.store.upsert_tracks(&to_upsert);
            }
        }

        // Preserve any previously verified chart/catalog matches if remote returned sparse results,
        // and only persist the curated pool when we have a solid verified set.
        if curated_tracks.len() < 6 {
            let chart_fallback = self
                .store
                .popular_chart_tracks_for_new_user(80)
                .unwrap_or_default();
            let fallback_pool: Vec<Track> = if !chart_fallback.is_empty() {
                chart_fallback
            } else {
                self.store.tracks_by_mood(mood, 60, 0).unwrap_or_default()
            };
            for t in fallback_pool {
                if is_mood_clutter_track(&t)
                    || is_mood_title_trap(&t, mood)
                    || track_matches_topic(&t, "christmas")
                    || track_matches_topic(&t, "halloween")
                {
                    continue;
                }
                let meta_low = t.genre.to_lowercase();
                if !has_mood_meta_match(&meta_low, mood)
                    && !classify_moods(&t.title, &t.album, &t.genre, "").contains(&mood)
                {
                    continue;
                }
                if seen_ids.insert(t.id.clone()) {
                    curated_tracks.push(t);
                    if curated_tracks.len() >= 35 {
                        break;
                    }
                }
            }
        }
        if curated_tracks.len() >= 6 {
            let ids: Vec<String> = curated_tracks.iter().map(|t| t.id.clone()).collect();
            self.save_curated_pool(&mood_cache_key(mood), &ids);
        }
        curated_tracks
    }

    async fn curate_user_mood_tracks(&self, user_id: i64, mood: Mood) -> Vec<Track> {
        let sctx = self.build_scoring_context(user_id);
        let mut genres: Vec<(String, f64)> = sctx
            .prof
            .genre_affinity
            .iter()
            .filter(|(g, v)| **v > 0.5 && is_genre_fit_for_mood_query(g, mood))
            .map(|(g, v)| (g.clone(), *v))
            .collect();
        genres.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let top_genres = pick_user_mood_query_genres(&genres);
        if top_genres.is_empty() {
            return Vec::new();
        }
        let hint = match mood {
            Mood::Chill => "chill",
            Mood::Energy => "workout",
            Mood::LateNight => "late night",
            Mood::Upbeat => "upbeat",
            Mood::Melancholy => "sad",
            Mood::Party => "dance",
            Mood::Acoustic => "acoustic",
            Mood::Groove => "groove",
            Mood::RockIndie => "indie",
        };
        let queries: Vec<String> = top_genres
            .into_iter()
            .map(|g| format!("{} {}", g, hint))
            .collect();
        let futs = queries
            .iter()
            .map(|q| self.sources.fetch_curated_mix_tracks(q, 2, 20));
        let Ok(batches) = tokio::time::timeout(
            std::time::Duration::from_secs(4),
            futures::future::join_all(futs),
        )
        .await
        else {
            return Vec::new();
        };
        let mut accepted_tracks: Vec<Track> = Vec::new();
        for batch_list in batches {
            for (pl_title, tracks) in batch_list {
                if !is_curated_playlist_for_mood(&pl_title, mood)
                    || Self::is_title_word_playlist(&tracks, mood)
                {
                    continue;
                }
                accepted_tracks.extend(tracks);
            }
        }
        if accepted_tracks.is_empty() {
            return Vec::new();
        }
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            self.sources.deezer.enrich_track_genres(&mut accepted_tracks),
        )
        .await;

        let mut out = Vec::new();
        let mut seen_ids = HashSet::new();
        let mut to_upsert = Vec::new();
        for mut t in accepted_tracks {
            if is_mood_clutter_track(&t)
                || !is_track_compatible_with_mood(&t, "", mood)
                || is_mood_title_trap(&t, mood)
            {
                continue;
            }
            let strict_genre_match = has_mood_meta_match(&t.genre.to_lowercase(), mood);
            if !strict_genre_match && is_low_rank_deezer_track(&t, 30_000) {
                continue;
            }
            t.moods = append_mood_slug(
                &classify_moods_str(&t.title, &t.album, &t.genre, ""),
                mood,
            );
            if seen_ids.insert(t.id.clone()) {
                to_upsert.push(t.clone());
                out.push(t);
            }
        }
        if !to_upsert.is_empty() {
            let _ = self.store.upsert_tracks(&to_upsert);
        }
        out
    }

    /// Collects stored candidates for a Moods & Vibes category and ranks/curates them against
    /// verified mood rules, genre compatibility, clutter filtering, and user taste.
    pub async fn mood_mix_tracks(&self, user_id: i64, mood: Mood, count: usize) -> Vec<Track> {
        let cached_curated_ids = self.load_curated_pool_ids(&mood_cache_key(mood), 12 * 3600);
        let cached_curated_tracks = if !cached_curated_ids.is_empty() {
            self.store
                .get_tracks_by_ids(&cached_curated_ids)
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let is_fresh_user =
            user_id > 0 && !self.store.has_listening_history(user_id).unwrap_or(false);
        let local_mood = if is_fresh_user {
            Vec::new()
        } else {
            self.store.tracks_by_mood(mood, 140, 0).unwrap_or_default()
        };
        let user_pool = if user_id > 0 {
            self.store
                .candidate_tracks_for_user(user_id, 900)
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        let mut candidates = Vec::new();
        let mut seen_ids = HashSet::new();
        for t in cached_curated_tracks
            .into_iter()
            .chain(local_mood)
            .chain(user_pool.clone())
        {
            if seen_ids.insert(t.id.clone()) {
                candidates.push(t);
            }
        }

        let mut curated_ids: HashSet<String> = cached_curated_ids.into_iter().collect();
        let mut initial = self.rank_mood_mix(user_id, mood, candidates.clone(), &curated_ids, count);
        let needs_personal_curation = if user_id > 0 {
            let sctx = self.build_scoring_context(user_id);
            !sctx.prof.genre_affinity.is_empty()
                && initial
                    .iter()
                    .filter(|t| is_personal_mix_track(t, &sctx.prof, &sctx.stats))
                    .count()
                    < count.min(10)
        } else {
            false
        };
        if needs_personal_curation {
            let personal = self.curate_user_mood_tracks(user_id, mood).await;
            if !personal.is_empty() {
                for t in personal {
                    curated_ids.insert(t.id.clone());
                    if seen_ids.insert(t.id.clone()) {
                        candidates.push(t);
                    }
                }
                initial = self.rank_mood_mix(user_id, mood, candidates.clone(), &curated_ids, count);
            }
        }
        let ranked = if initial.len() >= count.min(18) && !curated_ids.is_empty() {
            initial
        } else {
            // Replenish sparse or uncurated mood categories from curated playlists and genre charts.
            let fresh = self.curate_mood_pool(mood).await;
            for t in fresh {
                curated_ids.insert(t.id.clone());
                if seen_ids.insert(t.id.clone()) {
                    candidates.push(t);
                }
            }

            let mut ranked = self.rank_mood_mix(user_id, mood, candidates, &curated_ids, count);
            // Never collapse to a 1-song mix if compatible user/catalog tracks exist.
            if ranked.len() < count.min(12) && user_id > 0 {
                let sctx = self.build_scoring_context(user_id);
                let mut have_keys: HashSet<String> = ranked.iter().map(Track::song_key).collect();
                let mut have_ids: HashSet<String> = ranked.iter().map(|t| t.id.clone()).collect();
                let extra_pool: Vec<Track> = user_pool
                    .into_iter()
                    .chain(self.for_you_tracks(user_id, count * 2))
                    .collect();
                let extra_artists: Vec<String> = extra_pool.iter().map(|t| t.artist.clone()).collect();
                let artist_genres = self
                    .store
                    .known_artist_genres_for(&extra_artists)
                    .unwrap_or_default();
                for t in extra_pool {
                    if ranked.len() >= count {
                        break;
                    }
                    if have_ids.contains(&t.id)
                        || have_keys.contains(&t.song_key())
                        || is_strongly_disliked(&t, &sctx.prof, &sctx.stats)
                        || is_mood_clutter_track(&t)
                        || ((t.source == "archive" || t.source == "soulseek")
                            && !is_familiar_track(&t, &sctx.prof, &sctx.stats))
                    {
                        continue;
                    }
                    let ag = artist_genres
                        .get(&primary_artist(&t.artist).to_lowercase())
                        .map(|s| s.as_str())
                        .unwrap_or("");
                    let meta_low = format!("{} {}", t.genre, ag).to_lowercase();
                    let has_evidence = has_mood_meta_match(&meta_low, mood)
                        || classify_moods(&t.title, &t.album, &t.genre, ag).contains(&mood);
                    if has_evidence
                        && is_track_compatible_with_mood(&t, ag, mood)
                        && !is_mood_title_trap(&t, mood)
                        && !track_matches_topic(&t, "christmas")
                        && !track_matches_topic(&t, "halloween")
                    {
                        have_ids.insert(t.id.clone());
                        have_keys.insert(t.song_key());
                        ranked.push(t);
                    }
                }
            }
            ranked
        };
        if user_id > 0 && !ranked.is_empty() {
            let ids: Vec<String> = ranked.iter().map(|t| t.id.clone()).collect();
            self.save_curated_pool(&mood_user_cache_key(user_id, mood), &ids);
        }
        ranked
    }

    pub fn rank_mood_mix(
        &self,
        user_id: i64,
        mood: Mood,
        candidates: Vec<Track>,
        curated_ids: &HashSet<String>,
        limit: usize,
    ) -> Vec<Track> {
        if candidates.is_empty() || limit == 0 {
            return Vec::new();
        }
        let sctx = self.build_scoring_context(user_id);
        let artists: Vec<String> = candidates.iter().map(|t| t.artist.clone()).collect();
        let artist_genres = self.store.known_artist_genres_for(&artists).unwrap_or_default();
        let allowed_scripts = allowed_non_latin_scripts_for_user(&sctx.prof, &sctx.stats, &candidates);

        let mut eligible: Vec<(Track, bool, bool, Vec<Mood>, f64, bool, bool)> = Vec::new();
        for t in candidates {
            if is_strongly_disliked(&t, &sctx.prof, &sctx.stats)
                || is_mood_clutter_track(&t)
                || track_matches_topic(&t, "christmas")
                || track_matches_topic(&t, "halloween")
            {
                continue;
            }
            let ag = artist_genres
                .get(&primary_artist(&t.artist).to_lowercase())
                .map(|s| s.as_str())
                .unwrap_or("");
            if !is_track_compatible_with_mood(&t, ag, mood) {
                continue;
            }

            let is_curated = curated_ids.contains(&t.id);
            let clean_moods = classify_moods(&t.title, &t.album, &t.genre, ag);
            let combined_genre_low = format!("{} {}", t.genre, ag).to_lowercase();
            let meta_match = has_mood_meta_match(&combined_genre_low, mood);
            let has_known_genre = !t.genre.trim().is_empty() || !ag.trim().is_empty();
            let clean_match = clean_moods.contains(&mood) && (has_known_genre || t.source == "local");
            let realm = user_realm_score(&t, &sctx.prof, &sctx.stats);
            let is_familiar = is_familiar_track(&t, &sctx.prof, &sctx.stats);
            let is_personal = is_personal_mix_track(&t, &sctx.prof, &sctx.stats);

            // Never leak unplayed Archive.org / Soulseek search-cache leftovers (e.g.
            // homonym albums or P2P fan mashups) into Moods & Vibes mixes.
            if (t.source == "archive" || t.source == "soulseek") && !is_familiar {
                continue;
            }
            if !is_familiar && t.source != "local" && is_live_concert_title(&t.title) {
                continue;
            }
            // Reject non-personal tracks in unrequested non-Latin scripts (e.g. Arabic
            // songs leaking from global YouTube Music "Sad Songs" playlists for a listener
            // who only listens to English and Japanese music).
            if !is_personal && !track_scripts_allowed(&t, &allowed_scripts) {
                continue;
            }

            if !meta_match && !is_familiar && is_mood_title_trap(&t, mood) {
                continue;
            }
            // Reject obscure / AI-slop Deezer uploads unless they have a direct genre match
            // or already belong to the listener's own realm.
            if !meta_match && !is_familiar && realm == 0.0 && is_low_rank_deezer_track(&t, 30_000) {
                continue;
            }
            // Every track needs real evidence for THIS mood. Taste alone never admits a track,
            // otherwise each mood collapses into the same "favorites" playlist. Do not let
            // stale stored mood tags resurrect unrelated remote tracks whose genre doesn't match.
            let stored_match = t.mood_list().contains(&mood)
                && (meta_match
                    || is_curated
                    || (has_known_genre && (is_familiar || realm > 0.0 || t.source == "local")));
            if !is_curated && !meta_match && !clean_match && !stored_match {
                continue;
            }
            let strong_evidence = meta_match || clean_match || is_familiar || realm > 0.0;
            eligible.push((
                t,
                is_curated,
                meta_match,
                clean_moods,
                realm,
                strong_evidence,
                is_personal,
            ));
        }

        let eligible_tracks: Vec<Track> =
            eligible.iter().map(|(t, _, _, _, _, _, _)| t.clone()).collect();
        let has_user_taste = !sctx.stats.is_empty() || profile_has_taste(&sctx.prof);
        let user_has_metal_taste = [
            "metal",
            "heavy metal",
            "thrash",
            "death metal",
            "black metal",
            "nu metal",
            "alt metal",
            "metalcore",
        ]
        .iter()
        .any(|g| {
            sctx.prof.genre_affinity.get(*g).copied().unwrap_or(0.0) > 0.2
                || sctx
                    .prof
                    .playlist_genre_affinity
                    .get(*g)
                    .copied()
                    .unwrap_or(0.0)
                    > 0.2
        });
        let n_w = self.neural_blend_weight(0.65, &sctx);
        let n_scores = if has_user_taste && n_w > 0.0 {
            self.neural_scores(user_id, &[], None, &[mood], &[], &eligible_tracks)
        } else {
            Vec::new()
        };

        let mut scored: Vec<(f64, bool, bool, Track)> = eligible
            .into_iter()
            .enumerate()
            .map(
                |(idx, (t, is_curated, meta_match, clean_moods, realm, strong_evidence, is_personal))| {
                    let taste_s = score_track_core(
                        &t,
                        &sctx.prof,
                        &sctx.stats,
                        ScoreMode::Supermix,
                    );

                    // Category evidence leads; the listener's realm only personalizes within it.
                    let mut evidence = if meta_match { 3.0 } else { 0.0 };
                    if is_curated {
                        evidence += 1.5;
                    }
                    if clean_moods.contains(&mood) {
                        evidence += 1.2;
                    }
                    if t.mood_list().contains(&mood) && (!t.genre.trim().is_empty() || is_curated) {
                        evidence += 1.0;
                    }
                    // Prefer tracks whose strongest mood is this one, so overlapping genres
                    // (e.g. rock in both Energy and Rock/Indie) don't make every mood identical.
                    if clean_moods.first() == Some(&mood) {
                        evidence += 1.0;
                    } else if clean_moods.len() >= 3 && !is_curated {
                        evidence -= 0.6;
                    }

                    let mut score = taste_s * 0.6
                        + n_scores.get(idx).copied().unwrap_or(0.0) * n_w * 0.6;
                    score += evidence + realm * 1.2;
                    if is_personal {
                        score += 2.2;
                    }
                    // Avoid leaking unrequested Western heavy metal (e.g. Metallica) into
                    // a J-Rock / indie listener's mood mix solely via an incidental "rock" tag.
                    if has_user_taste && !is_personal && !user_has_metal_taste {
                        let gl = t.genre.to_lowercase();
                        if ["metal", "heavy metal", "thrash", "death metal", "black metal"]
                            .iter()
                            .any(|m| contains_term(&gl, m))
                        {
                            score -= 4.5;
                        }
                    }
                    if has_user_taste {
                        if t.source == "local" && realm == 0.0 {
                            score += 0.6;
                        }
                        score +=
                            (t.quality_score() as f64 / 120.0) + parse_rank_bonus(&t.meta) * 0.12;
                    } else {
                        score += parse_rank_bonus(&t.meta) * 0.25;
                    }

                    let solid = strong_evidence
                        || parse_deezer_rank(&t.meta).map_or(false, |r| r >= 80_000);
                    (score, solid, is_personal, t)
                },
            )
            .collect();

        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        // Avoid tail divergence: when the listener already has a rich personal pool for
        // this mood, keep the mix in their musical world instead of padding the bottom
        // with unrelated global chart tracks; otherwise drop weak unverified filler once
        // we have enough solid tracks.
        let personal_count = scored.iter().filter(|(_, _, p, _)| *p).count();
        let solid_count = scored.iter().filter(|(_, solid, _, _)| *solid).count();
        let ordered: Vec<Track> = if personal_count >= limit.min(20) {
            scored
                .into_iter()
                .filter(|(_, _, p, _)| *p)
                .map(|(_, _, _, t)| t)
                .collect()
        } else if solid_count >= limit.min(18) {
            scored
                .into_iter()
                .filter(|(_, solid, _, _)| *solid)
                .map(|(_, _, _, t)| t)
                .collect()
        } else {
            scored.into_iter().map(|(_, _, _, t)| t).collect()
        };
        dedup_and_interleave(ordered, limit, 3)
    }
}

fn strict_topic_keywords(topic: &str) -> &'static [&'static str] {
    match topic {
        "christmas" => &[
            "christmas",
            "xmas",
            "holiday",
            "holidays",
            "santa",
            "noel",
            "yuletide",
            "jingle",
            "silent night",
            "white christmas",
            "last christmas",
            "let it snow",
            "winter wonderland",
            "rudolph",
            "frosty",
            "feliz navidad",
            "mistletoe",
            "sleigh",
            "holly jolly",
            "carol",
            "all i want for christmas",
            "rockin' around",
            "it's beginning to look",
            "wonderful christmastime",
            "happy xmas",
            "o holy night",
            "silver bells",
            "little drummer boy",
            "deck the halls",
            "joy to the world",
            "blue christmas",
            "christmastime",
            "hanukkah",
            "kwanzaa",
        ],
        "halloween" => &[
            "halloween",
            "spooky",
            "monster mash",
            "ghostbusters",
            "witch",
            "zombie",
            "nightmare before",
        ],
        _ => &[],
    }
}

fn strict_topic_for_text(text: &str) -> Option<&'static str> {
    let low = text.to_lowercase();
    for topic in ["christmas", "halloween"] {
        if strict_topic_keywords(topic)
            .iter()
            .any(|kw| contains_term(&low, kw))
        {
            return Some(topic);
        }
    }
    None
}

fn track_matches_topic(t: &Track, topic: &str) -> bool {
    let hay = format!("{} {} {} {}", t.title, t.album, t.genre, t.moods).to_lowercase();
    strict_topic_keywords(topic)
        .iter()
        .any(|kw| contains_term(&hay, kw))
}

#[derive(Debug, Clone, Default)]
struct RadioScope {
    topic_key: Option<&'static str>,
    genres: Vec<String>,
    artists: HashSet<String>,
    related_artists: HashSet<String>,
}

impl RadioScope {
    fn from_seed(seed: &Track, sims: &[Track]) -> Self {
        let mut scope = Self::default();
        let pa = slug(&primary_artist(&seed.artist));
        if !pa.is_empty() {
            scope.artists.insert(pa);
        }

        let seed_text = format!("{} {} {} {}", seed.title, seed.album, seed.genre, seed.moods);
        scope.topic_key = strict_topic_for_text(&seed_text).or_else(|| {
            ["christmas", "halloween"].into_iter().find(|topic| {
                let hits = sims.iter().filter(|t| track_matches_topic(t, topic)).count();
                hits >= 3 && hits * 2 >= sims.len().max(1)
            })
        });

        let mut genres = split_genres(&seed.genre);
        if genres.is_empty() {
            let mut counts: HashMap<String, usize> = HashMap::new();
            for t in sims {
                for g in split_genres(&t.genre) {
                    *counts.entry(g).or_insert(0) += 1;
                }
            }
            let mut ranked: Vec<(String, usize)> = counts.into_iter().collect();
            ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            genres = ranked
                .into_iter()
                .filter(|(_, c)| *c >= 2)
                .take(2)
                .map(|(g, _)| g)
                .collect();
        }
        scope.genres = genres;

        // Track on-scope related artists from the seed's radio neighborhood so
        // untagged tracks (e.g. YTM) from those same artists can still qualify.
        for t in sims {
            if !scope.is_off_topic_or_clashing(t) {
                let ak = slug(&primary_artist(&t.artist));
                if !ak.is_empty() {
                    scope.related_artists.insert(ak);
                }
            }
        }
        scope
    }

    fn from_playlist(pl_name: &str, pl_comment: &str, pl_tracks: &[Track]) -> Self {
        let mut scope = Self::default();
        for t in pl_tracks {
            let pa = slug(&primary_artist(&t.artist));
            if !pa.is_empty() {
                scope.artists.insert(pa);
            }
        }

        let hint_text = format!("{} {}", pl_name, pl_comment);
        scope.topic_key = strict_topic_for_text(&hint_text).or_else(|| {
            if pl_tracks.is_empty() {
                return None;
            }
            ["christmas", "halloween"].into_iter().find(|topic| {
                let hits = pl_tracks
                    .iter()
                    .filter(|t| track_matches_topic(t, topic))
                    .count();
                hits > 0 && hits * 3 >= pl_tracks.len()
            })
        });

        let mut g_counts: HashMap<String, usize> = HashMap::new();
        for t in pl_tracks {
            for g in split_genres(&t.genre) {
                *g_counts.entry(g).or_insert(0) += 1;
            }
        }
        let mut genres: Vec<String> = if !g_counts.is_empty() {
            let threshold = (pl_tracks.len() / 5).max(1);
            let mut ranked: Vec<(String, usize)> = g_counts.into_iter().collect();
            ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            ranked
                .into_iter()
                .filter(|(_, c)| *c >= threshold)
                .take(4)
                .map(|(g, _)| g)
                .collect()
        } else {
            Vec::new()
        };
        let intent = parse_search_intent(&hint_text);
        for g in intent.genres {
            let gl = g.to_lowercase();
            if !genres.contains(&gl) {
                genres.push(gl);
            }
        }
        scope.genres = genres;
        scope
    }

    fn record_related_artist(&mut self, t: &Track) {
        if !self.is_off_topic_or_clashing(t) {
            let ak = slug(&primary_artist(&t.artist));
            if !ak.is_empty() {
                self.related_artists.insert(ak);
            }
        }
    }

    fn is_off_topic_or_clashing(&self, t: &Track) -> bool {
        if let Some(topic) = self.topic_key {
            return !track_matches_topic(t, topic);
        }
        !self.genres.is_empty() && has_genre_clash(t, "", &self.genres)
    }

    fn matches_external_candidate(&self, t: &Track) -> bool {
        if let Some(topic) = self.topic_key {
            return track_matches_topic(t, topic);
        }
        let pa = slug(&primary_artist(&t.artist));
        let in_neighborhood = !pa.is_empty()
            && (self.artists.contains(&pa) || self.related_artists.contains(&pa));
        if !self.genres.is_empty() {
            if has_genre_clash(t, "", &self.genres) {
                return false;
            }
            return !split_genres(&t.genre).is_empty() || in_neighborhood;
        }
        in_neighborhood
    }
}

impl TasteEngine {
    fn scoped_external_candidates(
        &self,
        user_id: i64,
        sctx: &ScoringContext,
        scope: &RadioScope,
        exclude_ids: &HashSet<String>,
        exclude_keys: &HashSet<String>,
        want_familiar: bool,
        count: usize,
    ) -> Vec<Track> {
        let mut primary = if want_familiar {
            let mut fam_pool = self.for_you_tracks(user_id, (count * 5).max(90));
            if user_id > 0 {
                for (t, _) in self.store.starred_tracks(user_id).unwrap_or_default() {
                    fam_pool.push(t);
                }
                for (t, _) in self
                    .store
                    .user_frequent_tracks(user_id, 120, 0)
                    .unwrap_or_default()
                {
                    fam_pool.push(t);
                }
            }
            fam_pool
        } else {
            self.discover_weekly(user_id, (count * 5).max(90))
        };
        let catalog = self
            .store
            .candidate_tracks_for_user(user_id, 1800)
            .unwrap_or_default();
        let mut seen_ids = HashSet::new();
        let mut seen_keys = exclude_keys.clone();
        let mut out = Vec::new();
        for t in primary.drain(..).chain(catalog) {
            if exclude_ids.contains(&t.id) || !seen_ids.insert(t.id.clone()) {
                continue;
            }
            if is_strongly_disliked(&t, &sctx.prof, &sctx.stats) {
                continue;
            }
            if is_familiar_track(&t, &sctx.prof, &sctx.stats) != want_familiar {
                continue;
            }
            if !scope.matches_external_candidate(&t) {
                continue;
            }
            if !seen_keys.insert(t.song_key()) {
                continue;
            }
            out.push(t);
            if out.len() >= count * 4 {
                break;
            }
        }
        out
    }

    fn resolve_excluded_song_keys(&self, exclude_ids: &HashSet<String>) -> HashSet<String> {
        if exclude_ids.is_empty() {
            return HashSet::new();
        }
        let ids: Vec<String> = exclude_ids.iter().take(120).cloned().collect();
        self.store
            .get_tracks_by_ids(&ids)
            .unwrap_or_default()
            .into_iter()
            .map(|t| t.song_key())
            .collect()
    }

    /// Expands a radio candidate pool via up to 2 parallel live-radio hop rounds
    /// on distinct on-scope related artists until the pool has enough artist-diverse
    /// candidates (unheard-only for Discover, or all eligible for Default/Familiar)
    /// to fill `target_count` after `dedup_and_interleave(..., 2)`.
    async fn expand_radio_pool_with_hops(
        &self,
        scope: &mut RadioScope,
        sctx: &ScoringContext,
        pool: &mut Vec<Track>,
        fallback_seed_pool: &[Track],
        seen_ids: &mut HashSet<String>,
        seen_keys: &mut HashSet<String>,
        hopped_artists: &mut HashSet<String>,
        unheard_only: bool,
        target_count: usize,
    ) {
        let capacity_of = |items: &[Track]| -> usize {
            if unheard_only {
                let unheard: Vec<Track> = items
                    .iter()
                    .filter(|t| !is_familiar_track(t, &sctx.prof, &sctx.stats))
                    .cloned()
                    .collect();
                interleaved_capacity(&unheard, target_count, 2)
            } else {
                interleaved_capacity(items, target_count, 2)
            }
        };

        for _round in 0..2 {
            if capacity_of(pool) >= target_count {
                break;
            }

            let mut hop_seeds: Vec<Track> = Vec::new();
            // For Discover, prefer hopping from unheard related-artist tracks first
            // so the next radio neighborhood skews toward fresh discoveries.
            if unheard_only {
                for t in pool.iter() {
                    if is_familiar_track(t, &sctx.prof, &sctx.stats)
                        || scope.is_off_topic_or_clashing(t)
                    {
                        continue;
                    }
                    let ak = slug(&primary_artist(&t.artist));
                    if !ak.is_empty() && hopped_artists.insert(ak) {
                        hop_seeds.push(t.clone());
                        if hop_seeds.len() >= 2 {
                            break;
                        }
                    }
                }
            }
            if hop_seeds.len() < 2 {
                for t in pool.iter().chain(fallback_seed_pool.iter()) {
                    if scope.is_off_topic_or_clashing(t) {
                        continue;
                    }
                    let ak = slug(&primary_artist(&t.artist));
                    if !ak.is_empty() && hopped_artists.insert(ak) {
                        hop_seeds.push(t.clone());
                        if hop_seeds.len() >= 2 {
                            break;
                        }
                    }
                }
            }
            if hop_seeds.is_empty() {
                break;
            }

            let per_hop = (target_count * 2).max(24);
            let futs = hop_seeds.into_iter().map(|hs| async move {
                tokio::time::timeout(
                    std::time::Duration::from_secs(4),
                    self.similar_songs_async(&hs, per_hop),
                )
                .await
                .unwrap_or_default()
            });
            let batches = futures::future::join_all(futs).await;
            let mut added_any = false;
            for batch in batches {
                for t in batch {
                    if scope.is_off_topic_or_clashing(&t)
                        || is_strongly_disliked(&t, &sctx.prof, &sctx.stats)
                    {
                        continue;
                    }
                    if seen_ids.contains(&t.id) {
                        continue;
                    }
                    let key = t.song_key();
                    if seen_keys.contains(&key) {
                        continue;
                    }
                    seen_ids.insert(t.id.clone());
                    seen_keys.insert(key);
                    scope.record_related_artist(&t);
                    pool.push(t);
                    added_any = true;
                }
            }
            if !added_any {
                break;
            }
        }
    }

    pub async fn recommend_for_playlist(
        &self,
        user_id: i64,
        playlist_id: &str,
        limit: usize,
    ) -> Vec<Track> {
        let Some(pl) = self.store.get_playlist(playlist_id).ok().flatten() else {
            return vec![];
        };
        let pl_tracks = self.store.playlist_tracks(playlist_id).unwrap_or_default();
        let query_hint = playlist_query_hint(&pl.name, &pl.comment);
        let centroid = build_playlist_centroid(&pl_tracks, &query_hint);
        let scope = RadioScope::from_playlist(&pl.name, &pl.comment, &pl_tracks);
        let sctx = self.build_scoring_context(user_id);

        // Sample up to 3 diverse artists from the playlist to pull fresh related candidates.
        let mut seeds = Vec::new();
        let mut seen_artists = HashSet::new();
        for t in &pl_tracks {
            let ak = slug(&primary_artist(&t.artist));
            if seen_artists.insert(ak) {
                seeds.push(t.clone());
                if seeds.len() >= 3 {
                    break;
                }
            }
        }

        let live_futs = seeds.iter().map(|s| self.similar_songs_async(s, 8));
        let mut live_pool: Vec<Track> = futures::future::join_all(live_futs)
            .await
            .into_iter()
            .flatten()
            .collect();

        if pl_tracks.len() < 3 && !query_hint.is_empty() {
            let found = self.sources.search_all(&query_hint, 10).await;
            if !found.is_empty() {
                let _ = self.store.upsert_tracks(&found);
                live_pool.extend(found);
            }
        }

        let catalog = self
            .store
            .candidate_tracks_for_user(user_id, 1800)
            .unwrap_or_default();

        let mut dedup_candidates = Vec::new();
        let mut seen_ids = HashSet::new();
        for t in live_pool.into_iter().chain(catalog) {
            if seen_ids.insert(t.id.clone())
                && !centroid.existing_ids.contains(&t.id)
                && !centroid.existing_keys.contains(&t.song_key())
                && !is_strongly_disliked(&t, &sctx.prof, &sctx.stats)
            {
                dedup_candidates.push(t);
            }
        }
        if scope.topic_key.is_some() || !scope.genres.is_empty() {
            let on_scope: Vec<Track> = dedup_candidates
                .iter()
                .filter(|t| !scope.is_off_topic_or_clashing(t))
                .cloned()
                .collect();
            if !on_scope.is_empty() {
                dedup_candidates = on_scope;
            }
        }

        let n_scores = self.neural_scores(
            user_id,
            &pl_tracks,
            Some(&pl.name),
            &[],
            &[],
            &dedup_candidates,
        );
        let n_w = self.neural_blend_weight(0.75, &sctx);

        let mut scored: Vec<(f64, Track)> = dedup_candidates
            .into_iter()
            .enumerate()
            .map(|(idx, t)| {
                let c_sim = score_against_centroid(&t, &centroid);
                let u_taste = score_track_core(
                    &t,
                    &sctx.prof,
                    &sctx.stats,
                    ScoreMode::PlaylistDna,
                );
                let n_boost = n_scores.get(idx).copied().unwrap_or(0.0) * n_w;
                let combined = if pl_tracks.is_empty() {
                    c_sim * 2.0 + u_taste * 0.9 + n_boost
                } else {
                    c_sim * 2.6 + u_taste * 0.65 + n_boost
                };
                (combined, t)
            })
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let ranked: Vec<Track> = scored.into_iter().map(|(_, t)| t).collect();
        dedup_and_interleave(ranked, limit, 2)
    }

    pub async fn playlist_radio_tracks_with_mode(
        &self,
        user_id: i64,
        playlist_id: &str,
        exclude_ids: &HashSet<String>,
        client_recent: &[(String, i64)],
        count: usize,
        mode: RadioMode,
    ) -> Vec<Track> {
        let recs = self
            .recommend_for_playlist(user_id, playlist_id, (count * 3).max(48))
            .await;
        let pl = self.store.get_playlist(playlist_id).ok().flatten();
        let pl_tracks = self.store.playlist_tracks(playlist_id).unwrap_or_default();
        let (pl_name, pl_comment) = pl
            .as_ref()
            .map(|p| (p.name.as_str(), p.comment.as_str()))
            .unwrap_or(("", ""));
        let mut scope = RadioScope::from_playlist(pl_name, pl_comment, &pl_tracks);
        let sctx = self.build_scoring_context(user_id);
        let recent_penalties = self.build_recent_heard_penalties(user_id, &sctx, client_recent);
        let excluded_keys = self.resolve_excluded_song_keys(exclude_ids);

        let mut familiar: Vec<Track> = pl_tracks
            .iter()
            .filter(|t| {
                !exclude_ids.contains(&t.id)
                    && !excluded_keys.contains(&t.song_key())
                    && !is_strongly_disliked(t, &sctx.prof, &sctx.stats)
            })
            .cloned()
            .collect();
        familiar.sort_by(|a, b| {
            let sa = score_track_core(a, &sctx.prof, &sctx.stats, ScoreMode::Supermix)
                - recent_penalties.penalty_for(a);
            let sb = score_track_core(b, &sctx.prof, &sctx.stats, ScoreMode::Supermix)
                - recent_penalties.penalty_for(b);
            sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut fresh_scored: Vec<(f64, Track)> = recs
            .into_iter()
            .filter(|t| !exclude_ids.contains(&t.id) && !excluded_keys.contains(&t.song_key()))
            .enumerate()
            .map(|(idx, t)| {
                scope.record_related_artist(&t);
                let order_prior = -(idx as f64) * 0.2;
                let fam_adj = if mode == RadioMode::Discover
                    && is_familiar_track(&t, &sctx.prof, &sctx.stats)
                {
                    -6.0
                } else {
                    0.0
                };
                let s = order_prior + fam_adj - recent_penalties.penalty_for(&t);
                (s, t)
            })
            .collect();
        fresh_scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let fresh: Vec<Track> = fresh_scored.into_iter().map(|(_, t)| t).collect();

        let mut combined = Vec::new();
        let mut seen_ids: HashSet<String> = exclude_ids.clone();
        let mut seen_keys: HashSet<String> = excluded_keys.clone();
        match mode {
            RadioMode::Familiar => {
                for t in familiar {
                    let key = t.song_key();
                    if seen_ids.insert(t.id.clone()) && seen_keys.insert(key) {
                        combined.push(t);
                    }
                }
                if interleaved_capacity(&combined, count, 2) < count {
                    for t in self.scoped_external_candidates(
                        user_id,
                        &sctx,
                        &scope,
                        &seen_ids,
                        &seen_keys,
                        true,
                        count * 2,
                    ) {
                        let key = t.song_key();
                        if seen_ids.insert(t.id.clone()) && seen_keys.insert(key) {
                            combined.push(t);
                        }
                    }
                }
                for t in fresh {
                    let key = t.song_key();
                    if seen_ids.insert(t.id.clone()) && seen_keys.insert(key) {
                        combined.push(t);
                    }
                }
                if interleaved_capacity(&combined, count, 2) < count {
                    let mut hopped = HashSet::new();
                    self.expand_radio_pool_with_hops(
                        &mut scope,
                        &sctx,
                        &mut combined,
                        &pl_tracks,
                        &mut seen_ids,
                        &mut seen_keys,
                        &mut hopped,
                        false,
                        count,
                    )
                    .await;
                }
            }
            RadioMode::Discover => {
                for t in fresh
                    .iter()
                    .filter(|t| !is_familiar_track(t, &sctx.prof, &sctx.stats))
                {
                    let key = t.song_key();
                    if seen_ids.insert(t.id.clone()) && seen_keys.insert(key) {
                        combined.push(t.clone());
                    }
                }
                // Expand via parallel live radio hops on diverse on-scope seeds whenever
                // Discover doesn't yet have enough artist-diverse unheard tracks.
                if interleaved_capacity(&combined, count, 2) < count {
                    let mut hopped = HashSet::new();
                    self.expand_radio_pool_with_hops(
                        &mut scope,
                        &sctx,
                        &mut combined,
                        &pl_tracks,
                        &mut seen_ids,
                        &mut seen_keys,
                        &mut hopped,
                        true,
                        count,
                    )
                    .await;
                    combined.retain(|t| !is_familiar_track(t, &sctx.prof, &sctx.stats));
                }
                if interleaved_capacity(&combined, count, 2) < count {
                    for t in self.scoped_external_candidates(
                        user_id,
                        &sctx,
                        &scope,
                        &seen_ids,
                        &seen_keys,
                        false,
                        count * 2,
                    ) {
                        let key = t.song_key();
                        if seen_ids.insert(t.id.clone()) && seen_keys.insert(key) {
                            combined.push(t);
                        }
                    }
                }
                // Only fall back to familiar tracks if Discover found literally zero unheard tracks.
                if combined.is_empty() {
                    let mut fb_ids = exclude_ids.clone();
                    let mut fb_keys = excluded_keys.clone();
                    for t in fresh.into_iter().chain(familiar) {
                        let key = t.song_key();
                        if fb_ids.insert(t.id.clone()) && fb_keys.insert(key) {
                            combined.push(t);
                        }
                    }
                }
            }
            RadioMode::Default => {
                // Interleave ~3 fresh discoveries per 1 familiar playlist track so radio feels cohesive yet new.
                let mut fi = fresh.into_iter();
                let mut ki = familiar.into_iter();
                while combined.len() < count * 3 {
                    let mut pushed = false;
                    for _ in 0..3 {
                        if let Some(t) = fi.next() {
                            let key = t.song_key();
                            if seen_ids.insert(t.id.clone()) && seen_keys.insert(key) {
                                combined.push(t);
                                pushed = true;
                            }
                        }
                    }
                    if let Some(t) = ki.next() {
                        let key = t.song_key();
                        if seen_ids.insert(t.id.clone()) && seen_keys.insert(key) {
                            combined.push(t);
                            pushed = true;
                        }
                    }
                    if !pushed {
                        break;
                    }
                }
                if interleaved_capacity(&combined, count, 2) < count {
                    let mut hopped = HashSet::new();
                    self.expand_radio_pool_with_hops(
                        &mut scope,
                        &sctx,
                        &mut combined,
                        &pl_tracks,
                        &mut seen_ids,
                        &mut seen_keys,
                        &mut hopped,
                        false,
                        count,
                    )
                    .await;
                }
                if interleaved_capacity(&combined, count, 2) < count {
                    for t in self
                        .scoped_external_candidates(
                            user_id,
                            &sctx,
                            &scope,
                            &seen_ids,
                            &seen_keys,
                            false,
                            count * 2,
                        )
                        .into_iter()
                        .chain(self.scoped_external_candidates(
                            user_id,
                            &sctx,
                            &scope,
                            &seen_ids,
                            &seen_keys,
                            true,
                            count,
                        ))
                    {
                        let key = t.song_key();
                        if seen_ids.insert(t.id.clone()) && seen_keys.insert(key) {
                            combined.push(t);
                        }
                    }
                }
            }
        }
        if combined.is_empty() {
            let fallback: Vec<Track> = match mode {
                RadioMode::Discover => self.scoped_external_candidates(
                    user_id,
                    &sctx,
                    &scope,
                    exclude_ids,
                    &excluded_keys,
                    false,
                    count * 2,
                ),
                RadioMode::Familiar => self.scoped_external_candidates(
                    user_id,
                    &sctx,
                    &scope,
                    exclude_ids,
                    &excluded_keys,
                    true,
                    count * 2,
                ),
                RadioMode::Default => self
                    .for_you_tracks(user_id, (count * 5).max(80))
                    .into_iter()
                    .filter(|t| {
                        !exclude_ids.contains(&t.id)
                            && !excluded_keys.contains(&t.song_key())
                            && ((scope.topic_key.is_none() && scope.genres.is_empty())
                                || scope.matches_external_candidate(t))
                    })
                    .collect(),
            };
            return self.rank_fallback_with_recent(fallback, &recent_penalties, count);
        }
        dedup_and_interleave_fill(combined, count)
    }

    #[cfg(test)]
    pub async fn radio_next_tracks_with_exclusions_and_recent(
        &self,
        user_id: i64,
        seed: Option<&Track>,
        exclude_ids: &HashSet<String>,
        client_recent: &[(String, i64)],
        count: usize,
    ) -> Vec<Track> {
        self.radio_next_tracks_with_mode(
            user_id,
            seed,
            exclude_ids,
            client_recent,
            count,
            RadioMode::Default,
        )
        .await
    }

    pub async fn radio_next_tracks_with_mode(
        &self,
        user_id: i64,
        seed: Option<&Track>,
        exclude_ids: &HashSet<String>,
        client_recent: &[(String, i64)],
        count: usize,
        mode: RadioMode,
    ) -> Vec<Track> {
        let sctx = self.build_scoring_context(user_id);
        let recent_penalties = self.build_recent_heard_penalties(user_id, &sctx, client_recent);
        let excluded_keys = self.resolve_excluded_song_keys(exclude_ids);

        if let Some(s) = seed {
            let sims = self.similar_songs_async(s, (count * 2).max(24)).await;
            let mut scope = RadioScope::from_seed(s, &sims);
            let mut filtered: Vec<Track> = Vec::new();
            let mut seen_ids: HashSet<String> = exclude_ids.clone();
            let mut seen_keys: HashSet<String> = excluded_keys.clone();
            seen_ids.insert(s.id.clone());
            seen_keys.insert(s.song_key());

            for t in &sims {
                if is_strongly_disliked(t, &sctx.prof, &sctx.stats) {
                    continue;
                }
                if seen_ids.contains(&t.id) {
                    continue;
                }
                let key = t.song_key();
                if seen_keys.contains(&key) {
                    continue;
                }
                seen_ids.insert(t.id.clone());
                seen_keys.insert(key);
                filtered.push(t.clone());
            }

            if scope.topic_key.is_some() || !scope.genres.is_empty() {
                let on_scope: Vec<Track> = filtered
                    .iter()
                    .filter(|t| !scope.is_off_topic_or_clashing(t))
                    .cloned()
                    .collect();
                if scope.topic_key.is_some() || !on_scope.is_empty() {
                    filtered = on_scope;
                }
            }

            // Expand via parallel live radio hops on distinct related artists whenever the
            // mode-specific eligible pool doesn't yet have enough artist-diverse tracks to
            // fill `count` (crucial for Discover, where familiar tracks are stripped out).
            let mut hopped_artists = HashSet::new();
            let seed_artist = slug(&primary_artist(&s.artist));
            if !seed_artist.is_empty() {
                hopped_artists.insert(seed_artist);
            }
            self.expand_radio_pool_with_hops(
                &mut scope,
                &sctx,
                &mut filtered,
                &sims,
                &mut seen_ids,
                &mut seen_keys,
                &mut hopped_artists,
                mode == RadioMode::Discover,
                count,
            )
            .await;

            if mode == RadioMode::Familiar {
                for t in self.scoped_external_candidates(
                    user_id,
                    &sctx,
                    &scope,
                    &seen_ids,
                    &seen_keys,
                    true,
                    count * 2,
                ) {
                    let key = t.song_key();
                    if seen_ids.insert(t.id.clone()) && seen_keys.insert(key) {
                        filtered.push(t);
                    }
                }
            } else if mode == RadioMode::Default && interleaved_capacity(&filtered, count, 2) < count {
                for t in self
                    .scoped_external_candidates(
                        user_id,
                        &sctx,
                        &scope,
                        &seen_ids,
                        &seen_keys,
                        false,
                        count * 2,
                    )
                    .into_iter()
                    .chain(self.scoped_external_candidates(
                        user_id,
                        &sctx,
                        &scope,
                        &seen_ids,
                        &seen_keys,
                        true,
                        count,
                    ))
                {
                    let key = t.song_key();
                    if seen_ids.insert(t.id.clone()) && seen_keys.insert(key) {
                        filtered.push(t);
                    }
                }
            }

            if !filtered.is_empty() {
                if user_id <= 0 && recent_penalties.is_empty() && mode == RadioMode::Default {
                    return dedup_and_interleave_fill(filtered, count);
                }
                let mut eligible = filtered;
                if mode == RadioMode::Discover {
                    let mut unheard: Vec<Track> = eligible
                        .iter()
                        .filter(|t| !is_familiar_track(t, &sctx.prof, &sctx.stats))
                        .cloned()
                        .collect();
                    if interleaved_capacity(&unheard, count, 2) < count {
                        for t in self.scoped_external_candidates(
                            user_id,
                            &sctx,
                            &scope,
                            &seen_ids,
                            &seen_keys,
                            false,
                            count * 2,
                        ) {
                            let key = t.song_key();
                            if seen_ids.insert(t.id.clone()) && seen_keys.insert(key) {
                                unheard.push(t);
                            }
                        }
                    }
                    if !unheard.is_empty() {
                        eligible = unheard;
                    }
                }
                let total = eligible.len().max(1) as f64;
                let n_w = self.neural_blend_weight(0.55, &sctx);
                let n_scores = if user_id > 0 && n_w > 0.0 {
                    self.neural_scores(user_id, &[s.clone()], None, &[], &[], &eligible)
                } else {
                    Vec::new()
                };
                let score_mode = match mode {
                    RadioMode::Discover => ScoreMode::Discover,
                    RadioMode::Default | RadioMode::Familiar => ScoreMode::Supermix,
                };
                let mut scored: Vec<(f64, Track)> = eligible
                    .into_iter()
                    .enumerate()
                    .map(|(idx, t)| {
                        let sim_prior = ((total - idx as f64) / total) * 4.5;
                        let user_fit = if user_id > 0 {
                            score_track_core(&t, &sctx.prof, &sctx.stats, score_mode) * 0.45
                                + n_scores.get(idx).copied().unwrap_or(0.0) * n_w
                        } else {
                            0.0
                        };
                        let mode_adj = match mode {
                            RadioMode::Familiar => {
                                let st = sctx.stats.get(&t.id).copied().unwrap_or_default();
                                let mut b = 0.0;
                                if is_familiar_track(&t, &sctx.prof, &sctx.stats) {
                                    b += 12.0;
                                }
                                if st.play_count > 0 {
                                    b += (st.play_count as f64).min(5.0) * 0.4;
                                }
                                b
                            }
                            RadioMode::Discover => {
                                if is_familiar_track(&t, &sctx.prof, &sctx.stats) {
                                    -8.0
                                } else {
                                    1.2
                                }
                            }
                            RadioMode::Default => 0.0,
                        };
                        let temp_recent_penalty = recent_penalties.penalty_for(&t);
                        (sim_prior + user_fit + mode_adj - temp_recent_penalty, t)
                    })
                    .collect();
                if !scored.is_empty() {
                    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
                    let ranked: Vec<Track> = scored.into_iter().map(|(_, t)| t).collect();
                    return dedup_and_interleave_fill(ranked, count);
                }
            }
            let scoped_fallback: Vec<Track> = match mode {
                RadioMode::Discover => self.scoped_external_candidates(
                    user_id,
                    &sctx,
                    &scope,
                    exclude_ids,
                    &excluded_keys,
                    false,
                    count * 2,
                ),
                RadioMode::Familiar => self.scoped_external_candidates(
                    user_id,
                    &sctx,
                    &scope,
                    exclude_ids,
                    &excluded_keys,
                    true,
                    count * 2,
                ),
                RadioMode::Default => self
                    .for_you_tracks(user_id, (count * 5).max(80))
                    .into_iter()
                    .filter(|t| {
                        !exclude_ids.contains(&t.id)
                            && !excluded_keys.contains(&t.song_key())
                            && scope.matches_external_candidate(t)
                    })
                    .collect(),
            };
            if !scoped_fallback.is_empty() || scope.topic_key.is_some() || !scope.genres.is_empty() {
                return self.rank_fallback_with_recent(scoped_fallback, &recent_penalties, count);
            }
        }
        let picks: Vec<Track> = match mode {
            RadioMode::Discover => {
                let mut disc: Vec<Track> = self
                    .discover_weekly(user_id, (count * 4).max(72))
                    .into_iter()
                    .filter(|t| {
                        !exclude_ids.contains(&t.id) && !excluded_keys.contains(&t.song_key())
                    })
                    .collect();
                if interleaved_capacity(&disc, count, 2) < count {
                    let mut seen_ids: HashSet<String> = exclude_ids.clone();
                    let mut seen_keys: HashSet<String> = excluded_keys.clone();
                    for t in &disc {
                        seen_ids.insert(t.id.clone());
                        seen_keys.insert(t.song_key());
                    }
                    let extra = self
                        .store
                        .candidate_tracks_for_user(user_id, 1800)
                        .unwrap_or_default();
                    let mut extra_scored: Vec<(f64, Track)> = extra
                        .into_iter()
                        .filter(|t| {
                            !seen_ids.contains(&t.id)
                                && !seen_keys.contains(&t.song_key())
                                && !is_familiar_track(t, &sctx.prof, &sctx.stats)
                                && !is_strongly_disliked(t, &sctx.prof, &sctx.stats)
                        })
                        .map(|t| {
                            let s = score_track_core(&t, &sctx.prof, &sctx.stats, ScoreMode::Discover);
                            (s, t)
                        })
                        .collect();
                    extra_scored.sort_by(|a, b| {
                        b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal)
                    });
                    for (_, t) in extra_scored {
                        let key = t.song_key();
                        if seen_ids.insert(t.id.clone()) && seen_keys.insert(key) {
                            disc.push(t);
                            if disc.len() >= count * 4 {
                                break;
                            }
                        }
                    }
                }
                disc
            }
            RadioMode::Familiar => {
                let mut seen_ids: HashSet<String> = exclude_ids.clone();
                let mut seen_keys: HashSet<String> = excluded_keys.clone();
                let mut fam: Vec<Track> = Vec::new();
                let base_for_you = self.for_you_tracks(user_id, (count * 4).max(72));
                for t in base_for_you
                    .iter()
                    .filter(|t| is_familiar_track(t, &sctx.prof, &sctx.stats))
                {
                    let key = t.song_key();
                    if seen_ids.insert(t.id.clone()) && seen_keys.insert(key) {
                        fam.push(t.clone());
                    }
                }
                if user_id > 0 && interleaved_capacity(&fam, count, 2) < count {
                    for (t, _) in self
                        .store
                        .starred_tracks(user_id)
                        .unwrap_or_default()
                        .into_iter()
                        .chain(
                            self.store
                                .user_frequent_tracks(user_id, 120, 0)
                                .unwrap_or_default()
                                .into_iter()
                                .map(|(t, _)| (t, 0)),
                        )
                    {
                        if is_strongly_disliked(&t, &sctx.prof, &sctx.stats) {
                            continue;
                        }
                        let key = t.song_key();
                        if seen_ids.insert(t.id.clone()) && seen_keys.insert(key) {
                            fam.push(t);
                        }
                    }
                }
                if interleaved_capacity(&fam, count, 2) < count {
                    for t in base_for_you {
                        let key = t.song_key();
                        if seen_ids.insert(t.id.clone()) && seen_keys.insert(key) {
                            fam.push(t);
                        }
                    }
                }
                fam
            }
            RadioMode::Default => self
                .for_you_tracks(user_id, (count * 4).max(72))
                .into_iter()
                .filter(|t| !exclude_ids.contains(&t.id) && !excluded_keys.contains(&t.song_key()))
                .collect(),
        };
        self.rank_fallback_with_recent(picks, &recent_penalties, count)
    }

    fn rank_fallback_with_recent(
        &self,
        picks: Vec<Track>,
        recent_penalties: &RecentHeardPenalties,
        count: usize,
    ) -> Vec<Track> {
        if picks.is_empty() || recent_penalties.is_empty() {
            return dedup_and_interleave_fill(picks, count);
        }
        let total = picks.len().max(1) as f64;
        let mut scored: Vec<(f64, Track)> = picks
            .into_iter()
            .enumerate()
            .map(|(idx, t)| {
                let base = ((total - idx as f64) / total) * 4.5;
                (base - recent_penalties.penalty_for(&t), t)
            })
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let ranked: Vec<Track> = scored.into_iter().map(|(_, t)| t).collect();
        dedup_and_interleave_fill(ranked, count)
    }

    fn build_recent_heard_penalties(
        &self,
        user_id: i64,
        sctx: &ScoringContext,
        client_recent: &[(String, i64)],
    ) -> RecentHeardPenalties {
        let now = Utc::now().timestamp();
        let mut by_id: HashMap<String, i64> = HashMap::new();
        let mut by_key: HashMap<String, i64> = HashMap::new();

        if user_id > 0 {
            for (tid, st) in &sctx.stats {
                if st.last_played > 0 && (now - st.last_played) < RECENT_HEARD_WINDOW_SECS {
                    by_id
                        .entry(tid.clone())
                        .and_modify(|ts| *ts = (*ts).max(st.last_played))
                        .or_insert(st.last_played);
                }
            }
            if let Ok(recent_tracks) = self.store.user_recent_tracks(user_id, 60, 0) {
                for (t, st) in recent_tracks {
                    if st.last_played > 0 && (now - st.last_played) < RECENT_HEARD_WINDOW_SECS {
                        let key = t.song_key();
                        by_key
                            .entry(key)
                            .and_modify(|ts| *ts = (*ts).max(st.last_played))
                            .or_insert(st.last_played);
                    }
                }
            }
        }

        if !client_recent.is_empty() {
            let mut client_ids: Vec<String> = Vec::with_capacity(client_recent.len());
            let mut client_ts_by_id: HashMap<String, i64> = HashMap::new();
            for (tid, age_secs) in client_recent {
                let clean_id = tid.trim();
                if clean_id.is_empty() {
                    continue;
                }
                let clamped_age = (*age_secs).clamp(0, RECENT_HEARD_WINDOW_SECS);
                if clamped_age >= RECENT_HEARD_WINDOW_SECS {
                    continue;
                }
                let heard_at = now - clamped_age;
                by_id
                    .entry(clean_id.to_string())
                    .and_modify(|ts| *ts = (*ts).max(heard_at))
                    .or_insert(heard_at);
                client_ts_by_id
                    .entry(clean_id.to_string())
                    .and_modify(|ts| *ts = (*ts).max(heard_at))
                    .or_insert(heard_at);
                client_ids.push(clean_id.to_string());
            }
            if !client_ids.is_empty() {
                if let Ok(resolved) = self.store.get_tracks_by_ids(&client_ids) {
                    for t in resolved {
                        if let Some(&heard_at) = client_ts_by_id.get(&t.id) {
                            let key = t.song_key();
                            by_key
                                .entry(key)
                                .and_modify(|ts| *ts = (*ts).max(heard_at))
                                .or_insert(heard_at);
                        }
                    }
                }
            }
        }

        RecentHeardPenalties {
            now,
            by_id,
            by_key,
        }
    }

    /// Pulls local & cached SQLite tracks matching a parsed search intent (e.g., 1980s year tags
    /// or seasonal/genre keywords) so offline libraries and cached gems fuel instant mixes.
    pub fn matching_catalog_tracks(&self, intent: &SearchIntent, limit: usize) -> Vec<Track> {
        if limit == 0 {
            return vec![];
        }
        let all = self.store.all_tracks(1500, 0).unwrap_or_default();
        let artists: Vec<String> = all.iter().map(|t| t.artist.clone()).collect();
        let artist_genres = self.store.known_artist_genres_for(&artists).unwrap_or_default();
        let lex_q = intent.entity_query.as_deref().unwrap_or(&intent.normalized);

        let mut matched: Vec<(f64, Track)> = all
            .into_iter()
            .filter_map(|t| {
                if is_mix_clutter_track(&t, &intent.normalized) {
                    return None;
                }
                let ag = artist_genres
                    .get(&primary_artist(&t.artist).to_lowercase())
                    .map(|s| s.as_str())
                    .unwrap_or("");
                let era = compute_query_era_match(&t, intent);
                let theme = compute_query_theme_match_with_artist(&t, ag, intent);
                let lex = search_match_score(&t, lex_q) as f64;
                if intent.is_theme_mix {
                    if let Some((min_y, max_y)) = intent.year_range {
                        let in_era = t.year > 1920 && t.year >= min_y && t.year <= max_y;
                        if !in_era {
                            return None;
                        }
                    } else if !intent.themes.is_empty() {
                        if theme < 0.35 && lex == 0.0 {
                            return None;
                        }
                    } else if theme < 0.25 {
                        return None;
                    }
                    if !intent.moods.is_empty()
                        && !intent.moods.iter().all(|m| is_track_compatible_with_mood(&t, ag, *m))
                    {
                        return None;
                    }
                } else if lex == 0.0 {
                    return None;
                }
                let pop = parse_rank_bonus(&t.meta);
                let qual = (t.quality_score() as f64) / 500.0;
                let s = era.max(0.0) * 3.0 + theme * 3.5 + (lex / 200.0).min(2.0) + pop + qual;
                Some((s, t))
            })
            .collect();
        matched.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        matched.into_iter().take(limit).map(|(_, t)| t).collect()
    }

    /// Selects the primary artist/song anchor from initial search results so `webapp::search`
    /// can fetch real YTM/Deezer radio & related-artist tracks for non-theme instant mixes.
    pub fn plan_entity_mix_anchor(
        &self,
        user_id: i64,
        query: &str,
        raw_tracks: &[Track],
        dz_artists: &[ArtistSummary],
    ) -> Option<EntityAnchorPlan> {
        let intent = parse_search_intent(query);
        if intent.is_theme_mix {
            return None;
        }
        let entity_q = intent
            .entity_query
            .as_deref()
            .unwrap_or(&intent.raw)
            .trim();
        let entity_slug = slug(entity_q);
        let entity_low = entity_q.to_lowercase();
        if entity_slug.is_empty() && entity_low.is_empty() {
            return None;
        }

        let sctx = self.build_scoring_context(user_id);
        let deduped = dedup_tracks_prefer_quality(raw_tracks.to_vec());
        let evals = evaluate_search_candidates(
            &intent,
            deduped,
            &HashMap::new(),
            &HashMap::new(),
            &sctx,
            &HashMap::new(),
        );
        let top_track = evals
            .iter()
            .find(|e| {
                !e.is_junk
                    && !e.disliked
                    && (e.lex > 0.0
                        || (!entity_slug.is_empty()
                            && slug(&format!("{} {}", e.track.artist, e.track.title))
                                .contains(&entity_slug)))
            })
            .map(|e| &e.track);

        let top_matches_artist = top_track
            .map(|t| !entity_slug.is_empty() && slug(&primary_artist(&t.artist)) == entity_slug)
            .unwrap_or(false);
        let top_dz_artist_id = top_track
            .filter(|_| top_matches_artist)
            .and_then(Track::deezer_artist_id);

        let matched_dz_artist = dz_artists
            .iter()
            .find(|a| a.name == entity_q)
            .or_else(|| {
                top_dz_artist_id.as_ref().and_then(|dz_id| {
                    dz_artists
                        .iter()
                        .find(|a| !a.deezer_id.is_empty() && &a.deezer_id == dz_id)
                })
            })
            .or_else(|| {
                dz_artists
                    .iter()
                    .find(|a| !entity_slug.is_empty() && slug(&a.name) == entity_slug)
            });

        let entity_words = crate::store::normalize_search_words(&entity_low);
        let top_exact_title = top_track
            .map(|t| {
                let tl = t.title.to_lowercase();
                let core_tl = crate::models::core_song_title(&t.title);
                tl == entity_low
                    || (!entity_slug.is_empty() && slug(&core_tl) == entity_slug)
                    || (!entity_words.is_empty()
                        && crate::store::title_alias_word_variants(&t.title)
                            .iter()
                            .any(|a| a == &entity_words))
            })
            .unwrap_or(false);

        let is_artist_query = if let Some(_art) = matched_dz_artist {
            top_track.is_none() || top_matches_artist || !top_exact_title
        } else {
            top_matches_artist && !top_exact_title
        };

        if is_artist_query {
            let artist_name = matched_dz_artist
                .map(|a| a.name.clone())
                .or_else(|| top_track.map(|t| primary_artist(&t.artist)))
                .filter(|a| !a.is_empty() && !a.eq_ignore_ascii_case("Unknown Artist"))?;
            let deezer_artist_id = matched_dz_artist
                .map(|a| a.deezer_id.clone())
                .filter(|id| !id.is_empty())
                .or(top_dz_artist_id);
            let seed_track = evals
                .iter()
                .find(|e| {
                    !e.is_junk
                        && !e.disliked
                        && is_same_or_featured_artist(&e.track.artist, &artist_name)
                        && deezer_artist_id.as_ref().map_or(true, |want_dz| {
                            e.track
                                .deezer_artist_id()
                                .as_ref()
                                .map_or(true, |tid| tid == want_dz)
                        })
                        && !is_mix_clutter_track(&e.track, &intent.normalized)
                })
                .map(|e| e.track.clone())
                .or_else(|| {
                    top_track
                        .filter(|t| {
                            deezer_artist_id.as_ref().map_or(true, |want_dz| {
                                t.deezer_artist_id()
                                    .as_ref()
                                    .map_or(true, |tid| tid == want_dz)
                            })
                        })
                        .cloned()
                });
            return Some(EntityAnchorPlan {
                artist_name,
                deezer_artist_id,
                seed_track,
                is_artist_query: true,
            });
        }

        // Without an artist match, never hijack a mix onto a partial-title collision
        // (e.g. "christian" -> "Christian's Inferno") or a catalog-genre / obscure 1-word title.
        if !top_exact_title {
            return None;
        }
        let seed = top_track?;
        if self.store.has_catalog_genre(&entity_low) {
            return None;
        }
        let is_single_word = entity_low.split_whitespace().count() == 1;
        let has_conjunction = entity_low.contains(" and ") || entity_low.contains(" & ");
        let is_unranked_local = seed.source == "local" || seed.source == "soulseek";
        let user_knows_seed = sctx.prof.starred.contains(&seed.id)
            || sctx.prof.manual_playlist_ids.contains(&seed.id);
        if !user_knows_seed
            && parse_rank_bonus(&seed.meta) < 1.2
            && (is_single_word || is_unranked_local || has_conjunction)
        {
            return None;
        }
        let artist_name = primary_artist(&seed.artist);
        if artist_name.is_empty() || artist_name.eq_ignore_ascii_case("Unknown Artist") {
            return None;
        }
        let anchor_slug = slug(&artist_name);
        let deezer_artist_id = seed.deezer_artist_id().or_else(|| {
            dz_artists
                .iter()
                .find(|a| a.name == artist_name)
                .or_else(|| {
                    dz_artists
                        .iter()
                        .find(|a| !anchor_slug.is_empty() && slug(&a.name) == anchor_slug)
                })
                .map(|a| a.deezer_id.clone())
                .filter(|id| !id.is_empty())
        });

        Some(EntityAnchorPlan {
            artist_name,
            deezer_artist_id,
            seed_track: Some(seed.clone()),
            is_artist_query: false,
        })
    }

    /// Actively sorts and refines search results using listening taste,
    /// lexical anchors, and query intent—and builds an ephemeral on-the-fly custom mix
    /// without persisting a playlist.
    pub fn search_and_refine(
        &self,
        user_id: i64,
        query: &str,
        raw_tracks: Vec<Track>,
        curated_batches: Vec<(String, Vec<Track>)>,
    ) -> SearchMixResult {
        self.search_and_refine_with_context(
            user_id,
            query,
            raw_tracks,
            curated_batches,
            SearchMixContext::default(),
        )
    }

    pub fn search_and_refine_with_context(
        &self,
        user_id: i64,
        query: &str,
        raw_tracks: Vec<Track>,
        curated_batches: Vec<(String, Vec<Track>)>,
        mix_ctx: SearchMixContext,
    ) -> SearchMixResult {
        let mut intent = parse_search_intent(query);
        let sctx = self.build_scoring_context(user_id);
        let has_user_taste = profile_has_taste(&sctx.prof);
        let has_cat_genre = self.store.has_catalog_genre(&intent.normalized);
        let force_topic = mix_ctx.mode_override.as_deref() == Some("topic");
        let force_entity = mix_ctx.mode_override.as_deref() == Some("entity");
        let promoted_topic = if force_entity {
            intent.is_theme_mix = false;
            false
        } else {
            force_topic
                || (!intent.is_theme_mix
                    && mix_ctx.anchor_artist.is_none()
                    && (!curated_batches.is_empty() || has_cat_genre))
        };
        if promoted_topic {
            intent.is_theme_mix = true;
            if has_cat_genre && intent.genres.is_empty() && !intent.normalized.is_empty() {
                intent.genres.push(title_case_query(&intent.normalized));
            }
        }

        // Track provenance from curated playlists & intent-expanded catalog hits before dedup.
        let mut curated_prior_by_key: HashMap<String, f64> = HashMap::new();
        let mut search_pool = raw_tracks;
        for (pl_title, batch) in curated_batches {
            let Some(base_prior) = curated_playlist_title_score(&pl_title, &intent) else {
                continue;
            };
            for t in batch {
                let k = t.song_key();
                let entry = curated_prior_by_key.entry(k).or_insert(0.0);
                if base_prior > *entry {
                    *entry = base_prior;
                }
                search_pool.push(t);
            }
        }

        if intent.is_theme_mix || search_pool.len() < 12 {
            for t in self.matching_catalog_tracks(&intent, 65) {
                search_pool.push(t);
            }
        } else {
            let lex_q = intent.entity_query.as_deref().unwrap_or(&intent.normalized);
            if !lex_q.trim().is_empty() {
                if let Ok(cat_hits) = self.store.search_tracks(lex_q, 50, 0) {
                    search_pool.extend(cat_hits);
                }
            }
        }

        if intent.is_theme_mix && !intent.genres.is_empty() {
            for t in search_pool.iter_mut() {
                normalize_scene_genre(t, &intent);
            }
        }

        let deduped_search = dedup_tracks_prefer_quality(search_pool);
        if deduped_search.is_empty()
            && mix_ctx.ytm_radio.is_empty()
            && mix_ctx.dz_artist_top.is_empty()
            && mix_ctx.dz_artist_radio.is_empty()
            && mix_ctx.dz_related_tracks.is_empty()
        {
            let empty_matches = HashSet::new();
            return SearchMixResult {
                tracks: vec![],
                instant_mix: None,
                refinements: build_search_refinements(&intent, &[], &empty_matches, &sctx, None),
                taste_match_ids: empty_matches,
            };
        }

        let search_artists: Vec<String> =
            deduped_search.iter().map(|t| t.artist.clone()).collect();
        let artist_genres = self
            .store
            .known_artist_genres_for(&search_artists)
            .unwrap_or_default();

        // Transformer opinion on every candidate (search hits + mix feeds), keyed by song so the
        // instant-mix composer can use it for feed tracks too. Pre-weighted by model trust.
        let n_w = self.neural_blend_weight(0.6, &sctx);
        let neural_by_key: HashMap<String, f64> = if user_id > 0 && n_w > 0.0 {
            let mut cands: Vec<Track> = deduped_search.clone();
            let mut seen_keys: HashSet<String> = cands.iter().map(Track::song_key).collect();
            for feed in [
                &mix_ctx.ytm_radio,
                &mix_ctx.dz_artist_top,
                &mix_ctx.dz_artist_radio,
                &mix_ctx.dz_related_tracks,
            ] {
                for t in feed {
                    if seen_keys.insert(t.song_key()) {
                        cands.push(t.clone());
                    }
                }
            }
            let scores = self.neural_scores(
                user_id,
                &[],
                Some(query),
                &intent.moods,
                &intent.genres,
                &cands,
            );
            cands
                .iter()
                .zip(scores)
                .filter(|(_, s)| *s != 0.0)
                .map(|(t, s)| (t.song_key(), (s * n_w).clamp(-4.0, 4.0)))
                .collect()
        } else {
            HashMap::new()
        };

        let evals = evaluate_search_candidates(
            &intent,
            deduped_search,
            &curated_prior_by_key,
            &artist_genres,
            &sctx,
            &neural_by_key,
        );

        let mut taste_match_ids = HashSet::new();
        for e in &evals {
            if e.is_taste_match {
                taste_match_ids.insert(e.track.id.clone());
            }
        }
        let ranked_tracks: Vec<Track> = evals.iter().map(|e| e.track.clone()).collect();

        // ---------------------------------------------------------------------
        // Instant mix composition.
        //
        // A mix is built from a small set of tracks that verifiably match the
        // query ("seeds"), then expanded only with tracks that are musically
        // similar to those seeds. Source rank, playlist provenance and personal
        // taste are tie-breakers: none of them can make an unrelated track
        // eligible. A small eligible pool produces a small mix — it is never
        // padded with generic filler just to reach an arbitrary track count.
        // ---------------------------------------------------------------------
        let mut entity_anchor_name: Option<String> = None;
        let instant_mix = if intent.is_theme_mix {
            compose_coherent_mix(
                &intent,
                &mix_ctx,
                &evals,
                &artist_genres,
                &sctx,
                &neural_by_key,
                None,
                has_user_taste,
            )
        } else {
            let entity_q = intent
                .entity_query
                .as_deref()
                .unwrap_or(&intent.raw)
                .trim();
            let entity_slug = slug(entity_q);
            let entity_low = entity_q.to_lowercase();
            let fallback_anchor = ranked_tracks.iter().find_map(|t| {
                if let Some(ref want_dz) = mix_ctx.anchor_deezer_artist_id {
                    if !want_dz.is_empty() {
                        if let Some(ref t_dz) = t.deezer_artist_id() {
                            if t_dz != want_dz {
                                return None;
                            }
                        }
                    }
                }
                if is_mix_clutter_track(t, &intent.normalized) {
                    return None;
                }
                let exact_art =
                    !entity_slug.is_empty() && slug(&primary_artist(&t.artist)) == entity_slug;
                let entity_words = crate::store::normalize_search_words(&entity_low);
                let exact_title = t.title.to_lowercase() == entity_low
                    || (!entity_slug.is_empty()
                        && slug(&crate::models::core_song_title(&t.title)) == entity_slug)
                    || (!entity_words.is_empty()
                        && crate::store::title_alias_word_variants(&t.title)
                            .iter()
                            .any(|a| a == &entity_words));
                if mix_ctx.anchor_artist.is_some() || force_entity {
                    let lex = search_match_score(t, entity_q);
                    let slug_match = !entity_slug.is_empty()
                        && slug(&format!("{} {}", t.artist, t.title)).contains(&entity_slug);
                    if lex > 0 || slug_match {
                        return Some(t.clone());
                    }
                } else if exact_art {
                    return Some(t.clone());
                } else if exact_title && !has_cat_genre {
                    let is_single_word = entity_low.split_whitespace().count() == 1;
                    let has_conj = entity_low.contains(" and ") || entity_low.contains(" & ");
                    let is_local_slsk = t.source == "local" || t.source == "soulseek";
                    let user_knows = sctx.prof.starred.contains(&t.id)
                        || sctx.prof.manual_playlist_ids.contains(&t.id);
                    if user_knows
                        || parse_rank_bonus(&t.meta) >= 1.2
                        || (!is_single_word && !is_local_slsk && !has_conj)
                    {
                        return Some(t.clone());
                    }
                }
                None
            });
            let anchor_artist = mix_ctx
                .anchor_artist
                .clone()
                .filter(|a| !a.trim().is_empty() && !a.eq_ignore_ascii_case("Unknown Artist"))
                .or_else(|| {
                    fallback_anchor
                        .as_ref()
                        .map(|t| primary_artist(&t.artist))
                        .filter(|a| !a.is_empty() && !a.eq_ignore_ascii_case("Unknown Artist"))
                });
            if let Some(art) = anchor_artist {
                entity_anchor_name = Some(art.clone());
                let anchor_dz_id = mix_ctx
                    .anchor_deezer_artist_id
                    .clone()
                    .filter(|id| !id.trim().is_empty())
                    .or_else(|| fallback_anchor.as_ref().and_then(Track::deezer_artist_id))
                    .or_else(|| {
                        mix_ctx
                            .dz_artist_top
                            .iter()
                            .find_map(Track::deezer_artist_id)
                    });
                let anchor_track_key = mix_ctx
                    .anchor_track_key
                    .clone()
                    .or_else(|| fallback_anchor.as_ref().map(Track::song_key));
                let anchor = MixAnchor {
                    artist: art,
                    deezer_id: anchor_dz_id,
                    track_key: anchor_track_key,
                    genres: mix_ctx.anchor_genres.clone(),
                    is_artist_query: mix_ctx.is_artist_query,
                };
                compose_coherent_mix(
                    &intent,
                    &mix_ctx,
                    &evals,
                    &artist_genres,
                    &sctx,
                    &neural_by_key,
                    Some(&anchor),
                    has_user_taste,
                )
            } else {
                None
            }
        };

        let refinements = build_search_refinements(
            &intent,
            &ranked_tracks,
            &taste_match_ids,
            &sctx,
            entity_anchor_name.as_deref(),
        );

        SearchMixResult {
            tracks: ranked_tracks,
            instant_mix,
            refinements,
            taste_match_ids,
        }
    }
}

/// Resolved identity of an artist/song mix anchor. Kept separate from
/// `SearchMixContext` so the composer can treat entity and theme mixes
/// through one code path.
#[derive(Clone, Default)]
struct MixAnchor {
    artist: String,
    deezer_id: Option<String>,
    track_key: Option<String>,
    genres: Vec<String>,
    is_artist_query: bool,
}

fn mix_artist_matches(t: &Track, anchor: &MixAnchor) -> bool {
    if let Some(ref want_dz) = anchor.deezer_id {
        if !want_dz.is_empty() {
            if let Some(t_dz) = t.deezer_artist_id() {
                if t_dz != *want_dz {
                    return false;
                }
            }
        }
    }
    is_same_or_featured_artist(&t.artist, &anchor.artist)
}

fn mix_artist_genres<'a>(
    t: &Track,
    artist_genres: &'a HashMap<String, String>,
) -> &'a str {
    artist_genres
        .get(&primary_artist(&t.artist).to_lowercase())
        .map(String::as_str)
        .unwrap_or("")
}

/// Content-based evidence that a track really matches the query. Source rank
/// is never consulted; a theme's own curated playlists may contribute weak
/// evidence, but every other filter (junk, genre clash, era, dislikes) still
/// applies to those tracks.
fn mix_query_evidence(
    t: &Track,
    intent: &SearchIntent,
    artist_genres: &HashMap<String, String>,
    curated_prior: f64,
) -> f64 {
    let ag = mix_artist_genres(t, artist_genres);
    let meta_low = format!("{} {}", t.genre, ag).to_lowercase();
    let mut ev = 0.0f64;

    if !intent.genres.is_empty() {
        if intent
            .genres
            .iter()
            .any(|g| genre_matches_text(&meta_low, g))
        {
            ev += 0.55;
        } else if has_genre_clash(t, ag, &intent.genres) {
            return 0.0;
        }
    }
    if !intent.moods.is_empty() {
        if !intent
            .moods
            .iter()
            .all(|m| is_track_compatible_with_mood(t, ag, *m))
        {
            return 0.0;
        }
        let classified = classify_moods(&t.title, &t.album, &t.genre, ag);
        let has_known_genre = !meta_low.trim().is_empty();
        let real_mood = intent.moods.iter().any(|m| {
            has_mood_meta_match(&meta_low, *m)
                || classified.contains(m)
                || (has_known_genre && t.mood_list().contains(m))
        });
        if real_mood {
            ev += 0.5;
        }
    }
    if let Some((lo, hi)) = intent.year_range {
        if t.year > 0 {
            if t.year >= lo && t.year <= hi {
                ev += 0.55;
            } else {
                return 0.0;
            }
        }
    }
    if !intent.themes.is_empty() {
        let theme = compute_query_theme_match_with_artist(t, ag, intent);
        if theme >= 0.35 {
            ev += 0.4 + theme * 0.4;
        }
    }
    // An exact title match is only meaningful for entity queries. For theme
    // queries it is a known trap ("praise and worship" by an obscure local
    // artist, "Christian's Inferno" for "christian"), so it is ignored here.
    if !intent.is_theme_mix {
        let q = intent.entity_query.as_deref().unwrap_or(&intent.normalized);
        let q_slug = slug(q);
        let q_words = crate::store::normalize_search_words(q);
        if !q_slug.is_empty() || !q_words.is_empty() {
            let a_slug = slug(&primary_artist(&t.artist));
            let t_slug = slug(&crate::models::core_song_title(&t.title));
            let alias_match = !q_words.is_empty()
                && crate::store::title_alias_word_variants(&t.title)
                    .iter()
                    .any(|a| a == &q_words);
            if (!q_slug.is_empty() && (a_slug == q_slug || t_slug == q_slug)) || alias_match {
                ev += 0.7;
            }
        }
    }
    if curated_prior > 0.0 {
        ev += (curated_prior * 0.3).min(0.9);
    }
    ev
}

/// Musical similarity between a candidate and one seed. Only content and
/// artist identity contribute; popularity and personal taste are applied later
/// as tie-breakers and cannot make an unrelated track eligible.
fn mix_track_similarity(
    c: &Track,
    seed: &Track,
    intent: &SearchIntent,
    anchor: Option<&MixAnchor>,
    artist_genres: &HashMap<String, String>,
    candidate_curated_prior: f64,
    related_feed: bool,
) -> f64 {
    if c.song_key() == seed.song_key() {
        return 1.0;
    }
    let c_artist = slug(&primary_artist(&c.artist));
    let s_artist = slug(&primary_artist(&seed.artist));
    if !c_artist.is_empty() && c_artist == s_artist {
        return 0.85;
    }
    if let Some(a) = anchor {
        if mix_artist_matches(c, a) {
            return 0.95;
        }
        if mix_artist_matches(seed, a) && related_feed {
            return 0.6;
        }
    }

    let mut sim = 0.0f64;
    let cg = split_genres(&c.genre);
    let sg = split_genres(&seed.genre);
    let genre_overlap = cg.iter().any(|g| {
        sg.iter()
            .any(|s| g == s || (genre_family(g).is_some() && genre_family(g) == genre_family(s)))
    });
    if genre_overlap {
        sim += 0.4;
    } else if !cg.is_empty() && !sg.is_empty() {
        sim -= 0.12;
    }
    let cm = c.mood_list();
    if !cm.is_empty() && cm.iter().any(|m| seed.mood_list().contains(m)) {
        sim += 0.18;
    }
    if c.year > 0 && seed.year > 0 && (c.year - seed.year).abs() <= 12 {
        sim += 0.18;
    }
    sim += (mix_query_evidence(c, intent, artist_genres, candidate_curated_prior) * 0.45).min(0.6);
    if related_feed {
        sim += if anchor.is_some() { 0.45 } else { 0.12 };
    }
    sim
}

/// Builds an instant mix from verified seeds plus strictly similar neighbours.
fn compose_coherent_mix(
    intent: &SearchIntent,
    mix_ctx: &SearchMixContext,
    evals: &[EvaluatedSearchTrack],
    artist_genres: &HashMap<String, String>,
    sctx: &ScoringContext,
    neural_by_key: &HashMap<String, f64>,
    anchor: Option<&MixAnchor>,
    has_user_taste: bool,
) -> Option<InstantMix> {
    let is_entity = anchor.is_some();

    let curated_priors: HashMap<String, f64> = evals
        .iter()
        .filter(|e| e.curated_prior > 0.0)
        .map(|e| (e.track.song_key(), e.curated_prior))
        .collect();

    // Gather every candidate we were given, keyed by (song, artist) so two
    // different artists with the same song title are never collapsed.
    let mut all: Vec<Track> = evals.iter().map(|e| e.track.clone()).collect();
    for feed in [
        &mix_ctx.ytm_radio,
        &mix_ctx.dz_artist_top,
        &mix_ctx.dz_artist_radio,
        &mix_ctx.dz_related_tracks,
    ] {
        all.extend(feed.iter().cloned());
    }
    let mut by_identity: HashMap<(String, String), Track> = HashMap::new();
    for mut t in all {
        if !is_entity && !intent.genres.is_empty() {
            normalize_scene_genre(&mut t, intent);
        }
        let key = (t.song_key(), t.artist_id());
        match by_identity.get_mut(&key) {
            Some(existing) if t.quality_score() > existing.quality_score() => {
                let mut winner = t;
                merge_track_metadata(&mut winner, existing);
                *existing = winner;
            }
            Some(existing) => merge_track_metadata(existing, &t),
            None => {
                by_identity.insert(key, t);
            }
        }
    }
    let mut pool: Vec<Track> = by_identity.into_values().collect();

    // A track is only allowed into a mix if it belongs to a trusted related
    // feed for entity mixes; for theme mixes content evidence is mandatory.
    let mut related_keys: HashSet<String> = HashSet::new();
    for feed in [
        &mix_ctx.ytm_radio,
        &mix_ctx.dz_artist_radio,
        &mix_ctx.dz_related_tracks,
    ] {
        for t in feed {
            related_keys.insert(t.song_key());
        }
    }
    let mut top_keys: HashSet<String> = HashSet::new();
    for t in &mix_ctx.dz_artist_top {
        top_keys.insert(t.song_key());
    }

    // Topic radio is seeded from verified tracks, so it is legitimate (if weak)
    // evidence for a theme. Raw YTM keyword search hits are much noisier: they
    // may fill a mix but can never define what it is about (seed it).
    let ytm_radio_keys: HashSet<String> = mix_ctx
        .ytm_radio
        .iter()
        .map(Track::song_key)
        .collect();
    let mut ytm_query_keys: HashSet<String> = ytm_radio_keys.clone();
    for e in evals {
        if e.track.source == "ytm" {
            ytm_query_keys.insert(e.track.song_key());
        }
    }

    pool.retain(|t| {
        if is_hidden_track(t, &sctx.prof) || is_strongly_disliked(t, &sctx.prof, &sctx.stats) {
            return false;
        }
        if is_junk_track_for_query(t, &intent.normalized)
            || is_mix_clutter_track(t, &intent.normalized)
        {
            return false;
        }
        if let Some((lo, hi)) = intent.year_range {
            if t.year > 0 && (t.year < lo || t.year > hi) {
                return false;
            }
        }
        // "Classic"/"retro" queries reject definitely-modern tracks even when
        // no explicit decade was given.
        if !is_entity && compute_query_era_match(t, intent) <= -0.35 {
            return false;
        }
        // A decade mix cannot be padded with tracks whose era is unknown unless
        // they carry independent query evidence (curated batch or YTM radio).
        if !is_entity && intent.year_range.is_some() && t.year == 0 {
            let prior = curated_priors.get(&t.song_key()).copied().unwrap_or(0.0);
            let ag_unknown = mix_artist_genres(t, artist_genres);
            let theme = compute_query_theme_match_with_artist(t, ag_unknown, intent);
            if prior <= 0.0 && theme < 0.35 && !ytm_query_keys.contains(&t.song_key()) {
                return false;
            }
        }
        let ag = mix_artist_genres(t, artist_genres);
        if !intent.genres.is_empty() && has_genre_clash(t, ag, &intent.genres) {
            return false;
        }
        if let Some(a) = anchor {
            // Two artists with the same display name but different external
            // identities must never share a mix (LISA vs LiSA).
            if slug(&primary_artist(&t.artist)) == slug(&primary_artist(&a.artist))
                && !mix_artist_matches(t, a)
            {
                return false;
            }
            if !a.genres.is_empty() && has_genre_clash(t, ag, &a.genres) {
                return false;
            }
        }
        true
    });

    let mut scored: Vec<(f64, Track)> = pool
        .into_iter()
        .map(|t| {
            let key = t.song_key();
            let prior = curated_priors.get(&key).copied().unwrap_or(0.0);
            let mut evidence = mix_query_evidence(&t, intent, artist_genres, prior);
            if evidence <= 0.0 && !is_entity {
                if ytm_radio_keys.contains(&key) {
                    evidence = 0.45;
                } else if ytm_query_keys.contains(&key) && t.genre.trim().is_empty() {
                    evidence = 0.25;
                }
            }
            (evidence, t)
        })
        .collect();

    // Pick seeds: exact anchor artist/song for entity mixes, verified content
    // matches for theme mixes. These define what the mix is *about*.
    let mut seeds: Vec<Track> = Vec::new();
    if let Some(a) = anchor {
        if let Some(ref key) = a.track_key {
            if let Some(pos) = scored
                .iter()
                .position(|(_, t)| &t.song_key() == key && mix_artist_matches(t, a))
            {
                seeds.push(scored.remove(pos).1);
            }
        }
        let mut idx = 0;
        while seeds.len() < 8 && idx < scored.len() {
            if mix_artist_matches(&scored[idx].1, a) {
                seeds.push(scored.remove(idx).1);
            } else {
                idx += 1;
            }
        }
    } else {
        scored.sort_by(|x, y| y.0.partial_cmp(&x.0).unwrap_or(std::cmp::Ordering::Equal));
        let mut idx = 0;
        while seeds.len() < 12 && idx < scored.len() {
            if scored[idx].0 >= 0.4 {
                seeds.push(scored.remove(idx).1);
            } else {
                idx += 1;
            }
        }
        // Last resort when no curated/metadata evidence exists at all: let the
        // query's own YTM hits define the mix rather than returning nothing.
        if seeds.len() < 2 {
            let mut idx = 0;
            while seeds.len() < 4 && idx < scored.len() {
                if scored[idx].0 > 0.0 && ytm_query_keys.contains(&scored[idx].1.song_key()) {
                    seeds.push(scored.remove(idx).1);
                } else {
                    idx += 1;
                }
            }
        }
    }
    if seeds.is_empty() || (!is_entity && seeds.len() < 2) {
        return None;
    }

    let threshold = if is_entity { 0.35 } else { 0.4 };
    let mut neighbours: Vec<(f64, Track)> = Vec::new();
    for (own_evidence, t) in scored {
        let related = related_keys.contains(&t.song_key())
            || (is_entity && top_keys.contains(&t.song_key()));
        // Eligibility gate. Entity mixes only take the anchor's own catalogue or
        // tracks from its radio / related-artist feeds; theme mixes only take
        // tracks with independent query evidence. Generic search noise that
        // merely shares a broad genre with a seed is the source of off-topic junk.
        let eligible = if let Some(a) = anchor {
            related || mix_artist_matches(&t, a)
        } else {
            own_evidence > 0.0
        };
        if !eligible {
            continue;
        }
        let similarity = seeds
            .iter()
            .map(|s| {
                let prior = curated_priors.get(&t.song_key()).copied().unwrap_or(0.0);
                mix_track_similarity(&t, s, intent, anchor, artist_genres, prior, related)
            })
            .fold(0.0f64, f64::max);
        if similarity < threshold {
            continue;
        }

        // Tie-breakers: they rank eligible tracks, they never grant eligibility.
        let aff = build_track_affinities(&t, &sctx.prof);
        let taste = (aff.artist_affinity * 0.3
            + aff.genre_affinity * 0.3
            + aff.mood_affinity * 0.2
            + if aff.followed_artist { 0.3 } else { 0.0 }
            + if sctx.prof.starred.contains(&t.id) { 0.4 } else { 0.0 })
        .clamp(-1.0, 1.0);
        let stats = sctx.stats.get(&t.id).copied().unwrap_or_default();
        let neural = neural_by_key.get(&t.song_key()).copied().unwrap_or(0.0);
        let tie = similarity
            + (parse_rank_bonus(&t.meta) * 0.012).min(0.06)
            + taste * 0.03
            + neural * 0.012
            + ((t.quality_score() as f64 / 800.0) * 0.01).min(0.02)
            - (stats.skip_count as f64) * 0.15;
        neighbours.push((tie, t));
    }
    neighbours.sort_by(|x, y| y.0.partial_cmp(&x.0).unwrap_or(std::cmp::Ordering::Equal));

    let mut ordered = seeds;
    ordered.extend(neighbours.into_iter().map(|(_, t)| t));

    let (anchor_slug, cap) = match anchor {
        Some(a) => (
            slug(&primary_artist(&a.artist)),
            if a.is_artist_query { 15 } else { 4 },
        ),
        None => (String::new(), 3),
    };
    let mut mix_tracks = if anchor.is_some() {
        dedup_and_interleave_with_anchor(ordered, 30, 3, Some((&anchor_slug, cap)))
    } else {
        dedup_and_interleave(ordered, 30, 3)
    };

    // An entity mix always opens with the track that seeded it.
    if let Some(a) = anchor {
        if let Some(ref key) = a.track_key {
            if let Some(pos) = mix_tracks.iter().position(|t| &t.song_key() == key) {
                let seed = mix_tracks.remove(pos);
                mix_tracks.insert(0, seed);
            }
        }
    }

    let min_tracks = if is_entity { 3 } else { 2 };
    if mix_tracks.len() < min_tracks {
        return None;
    }

    let seed_title = if is_entity && !anchor.unwrap().is_artist_query {
        mix_tracks.first().map(|t| t.title.as_str())
    } else {
        None
    };
    let single_artist_only = is_entity
        && mix_tracks
            .iter()
            .map(|t| slug(&primary_artist(&t.artist)))
            .filter(|s| !s.is_empty())
            .collect::<HashSet<_>>()
            .len()
            <= 1;
    let (title, subtitle) = build_instant_mix_title(
        intent,
        &mix_tracks,
        has_user_taste,
        anchor.map(|a| a.artist.as_str()),
        seed_title,
        single_artist_only,
    );
    let total_duration = mix_tracks.iter().map(|t| t.duration.max(0)).sum();
    Some(InstantMix {
        title,
        subtitle,
        query: intent.raw.clone(),
        is_theme_mix: !is_entity,
        prominent: if is_entity {
            intent.explicit_mix_word
        } else {
            true
        },
        tracks: mix_tracks,
        total_duration,
    })
}

struct EvaluatedSearchTrack {
    track: Track,
    search_score: f64,
    curated_prior: f64,
    lex: f64,
    is_taste_match: bool,
    is_junk: bool,
    disliked: bool,
}

fn signed_sat(v: f64, scale: f64, cap: f64) -> f64 {
    if v == 0.0 || scale <= 0.0 {
        0.0
    } else {
        (v / scale).tanh() * cap
    }
}

fn evaluate_search_candidates(
    intent: &SearchIntent,
    deduped: Vec<Track>,
    curated_prior_by_key: &HashMap<String, f64>,
    artist_genres: &HashMap<String, String>,
    sctx: &ScoringContext,
    neural_by_key: &HashMap<String, f64>,
) -> Vec<EvaluatedSearchTrack> {
    let lex_query = intent.entity_query.as_deref().unwrap_or(&intent.normalized);
    let lex_query_low = lex_query.to_lowercase();
    let lex_query_words = crate::store::normalize_search_words(&lex_query_low);
    let mut evals: Vec<EvaluatedSearchTrack> = Vec::with_capacity(deduped.len());

    for t in deduped {
        if is_hidden_track(&t, &sctx.prof) {
            continue;
        }
        let curated_prior = curated_prior_by_key
            .get(&t.song_key())
            .copied()
            .unwrap_or(0.0);
        let ag = artist_genres
            .get(&primary_artist(&t.artist).to_lowercase())
            .map(|s| s.as_str())
            .unwrap_or("");
        let lex = search_match_score(&t, &lex_query_low) as f64;
        let era_match = compute_query_era_match(&t, intent);
        let theme_match = compute_query_theme_match_with_artist(&t, ag, intent);
        let is_junk = is_junk_track_for_query(&t, &intent.normalized);
        let disliked = is_strongly_disliked(&t, &sctx.prof, &sctx.stats);

        let st = sctx.stats.get(&t.id).copied().unwrap_or_default();
        let song_key = t.song_key();
        let is_starred =
            sctx.prof.starred.contains(&t.id) || sctx.prof.starred_song_keys.contains(&song_key);
        let in_manual_playlist = sctx.prof.manual_playlist_ids.contains(&t.id)
            || sctx.prof.manual_playlist_song_keys.contains(&song_key);
        let net_plays = if st.play_count > st.skip_count {
            st.play_count - st.skip_count
        } else if st.skip_count == 0 {
            sctx.prof
                .played_song_keys
                .get(&song_key)
                .copied()
                .unwrap_or(0)
        } else {
            0
        };
        let is_personal_track = is_starred || in_manual_playlist || net_plays > 0;
        let mut fctx = build_track_affinities(&t, &sctx.prof);
        // Fresh remote search results often lack track-level genre tags; fall back to known
        // artist genres so user genre taste still prioritizes the kind of music they listen to.
        if fctx.genre_affinity == 0.0 && !ag.is_empty() {
            let ag_list = split_genres(ag);
            fctx.genre_affinity = ag_list
                .iter()
                .filter_map(|g| sctx.prof.genre_affinity.get(g))
                .fold(0.0f64, |acc, v| if v.abs() > acc.abs() { *v } else { acc });
            if fctx.playlist_genre_affinity == 0.0 {
                fctx.playlist_genre_affinity = ag_list
                    .iter()
                    .filter_map(|g| sctx.prof.playlist_genre_affinity.get(g))
                    .fold(0.0f64, |a, b| a.max(*b));
            }
        }
        let align = compute_taste_alignment(&t, &sctx.prof, &fctx);

        // Smoothly bounded personal taste signal: strong enough that within the same lexical
        // tier a user's favorite artist or preferred genre decisively beats a more popular
        // mainstream song, while capped so it can never lift an unrelated song above a direct
        // title/artist match.
        let play_history_term = if net_plays > 0 {
            ((net_plays as f64) / 3.0).tanh() * 2.5
        } else {
            0.0
        };
        let raw_taste_aff = signed_sat(fctx.artist_affinity, 3.5, 9.0)
            + signed_sat(fctx.playlist_artist_affinity, 2.0, 3.5)
            + signed_sat(fctx.genre_affinity, 5.0, 4.5)
            + signed_sat(fctx.playlist_genre_affinity, 4.0, 2.0)
            + signed_sat(fctx.mood_affinity, 6.0, 2.0)
            + if align.rel_a > 0.0 {
                (align.rel_a / 1.8).tanh() * 2.5
            } else {
                0.0
            }
            + if align.rel_t > 0.0 {
                (align.rel_t / 1.5).tanh() * 2.0
            } else {
                0.0
            }
            + if fctx.followed_artist { 3.5 } else { 0.0 }
            + if fctx.saved_album { 1.8 } else { 0.0 }
            + if is_starred { 3.5 } else { 0.0 }
            + if in_manual_playlist { 2.0 } else { 0.0 }
            + play_history_term;
        let taste_aff = raw_taste_aff.clamp(-14.0, 18.0);

        // Already scaled by model trust and clamped to ±4, so it can reorder within a
        // lexical tier but never lift a loose hit above an exact song/artist match.
        let neural_term = if disliked {
            0.0
        } else {
            neural_by_key.get(&song_key).copied().unwrap_or(0.0)
        };
        let pop_bonus = parse_rank_bonus(&t.meta);
        let quality_bonus = (t.quality_score() as f64) / 360.0;
        let skip_penalty = if disliked {
            12.0
        } else {
            (st.skip_count as f64) * 1.4
        };
        let junk_penalty = if is_junk { 14.0 } else { 0.0 };

        let is_taste_match = !disliked
            && !is_junk
            && (fctx.artist_affinity >= 1.2
                || fctx.genre_affinity >= 1.2
                || fctx.followed_artist
                || is_personal_track
                || align.rel_a >= 1.2
                || align.rel_t >= 1.2);

        let tl = t.title.to_lowercase();
        let al = primary_artist(&t.artist).to_lowercase();
        let tl_words = crate::store::normalize_search_words(&t.title);
        let core_tl_words =
            crate::store::normalize_search_words(&crate::models::core_song_title(&t.title));
        let title_aliases = crate::store::title_alias_word_variants(&t.title);
        let core_title_variants = crate::store::core_title_word_variants(&t.title);
        let is_unrequested_cover =
            crate::store::is_explicit_cover_title(&t.title) && !lex_query_low.contains("cover");
        let cover_penalty = if is_unrequested_cover && !is_personal_track {
            8.0
        } else {
            0.0
        };
        let al_words = crate::store::normalize_search_words(&primary_artist(&t.artist));

        let is_exact_entity = !is_junk
            && !is_unrequested_cover
            && (tl == lex_query_low
                || al == lex_query_low
                || (!lex_query_words.is_empty()
                    && (tl_words == lex_query_words
                        || al_words == lex_query_words
                        || title_aliases.iter().any(|a| a == &lex_query_words))));
        let is_exact_core_title = !is_junk
            && !is_unrequested_cover
            && !is_exact_entity
            && !lex_query_words.is_empty()
            && (core_tl_words == lex_query_words
                || core_title_variants.iter().any(|v| v == &lex_query_words));

        // Pin exact song/artist matches at the top for entity searches while letting
        // listening taste decisively win within the same lexical match tier.
        let lex_anchor = if intent.is_theme_mix {
            if is_exact_entity {
                28.0
            } else if lex > 0.0 {
                (lex / 110.0).min(4.0)
            } else {
                0.0
            }
        } else if is_exact_entity {
            56.0 + (lex - 250.0).max(0.0) / 50.0
        } else if is_exact_core_title {
            54.0 + (lex - 250.0).max(0.0) / 50.0
        } else if lex >= 450.0 {
            44.0 + (lex - 450.0) / 50.0
        } else if lex >= 280.0 {
            30.0 + (lex - 280.0) / 35.0
        } else if lex > 0.0 {
            12.0 + (lex / 25.0).min(12.0)
        } else {
            -6.0
        };

        let intent_fit = era_match * 4.5
            + theme_match * 4.5
            + curated_prior * 1.4
            + if intent.wants_hits {
                pop_bonus * 0.95
            } else {
                pop_bonus * 0.65
            };

        let search_score = lex_anchor
            + if intent.is_theme_mix {
                intent_fit
            } else {
                era_match * 1.5 + theme_match * 1.5 + curated_prior * 0.6 + pop_bonus * 0.40
            }
            + taste_aff
            + neural_term
            + quality_bonus
            - skip_penalty
            - junk_penalty
            - cover_penalty;

        evals.push(EvaluatedSearchTrack {
            track: t,
            search_score: if search_score.is_finite() { search_score } else { 0.0 },
            curated_prior,
            lex,
            is_taste_match,
            is_junk,
            disliked,
        });
    }

    evals.sort_by(|a, b| {
        b.search_score
            .partial_cmp(&a.search_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.track.quality_score().cmp(&a.track.quality_score()))
    });
    evals
}

fn collab_artist_slugs(artist: &str) -> Vec<String> {
    let low = artist.to_lowercase();
    let has_comma_list = artist.contains(", ")
        && !low.contains(" & ")
        && !low.contains(" and ");
    let has_collab_sep = has_comma_list
        || low.contains(" feat. ")
        || low.contains(" feat ")
        || low.contains(" ft. ")
        || low.contains(" ft ")
        || low.contains(" featuring ")
        || artist.contains(" × ");
    if !has_collab_sep {
        return Vec::new();
    }
    let normalized = artist
        .replace(" feat. ", "\u{001F}")
        .replace(" Feat. ", "\u{001F}")
        .replace(" FEAT. ", "\u{001F}")
        .replace(" feat ", "\u{001F}")
        .replace(" ft. ", "\u{001F}")
        .replace(" Ft. ", "\u{001F}")
        .replace(" ft ", "\u{001F}")
        .replace(" featuring ", "\u{001F}")
        .replace(" × ", "\u{001F}");
    let mut slugs = Vec::new();
    for chunk in normalized.split('\u{001F}') {
        let parts: Vec<&str> = if has_comma_list {
            chunk.split(", ").collect()
        } else {
            vec![chunk]
        };
        for p in parts {
            let s = slug(&primary_artist(p));
            if s.len() >= 2 && !slugs.contains(&s) {
                slugs.push(s);
            }
        }
    }
    if slugs.len() >= 2 {
        slugs
    } else {
        Vec::new()
    }
}

fn apply_track_affinity_delta(prof: &mut TasteProfile, t: &Track, w: f64, playlist_layer: bool) {
    let artist_id = t.artist_id();
    let has_ext_id = artist_id.starts_with("ar:dz:") || artist_id.starts_with("ar:ytm:");
    let a_key = slug(&primary_artist(&t.artist));
    let collab_slugs = collab_artist_slugs(&t.artist);
    if playlist_layer {
        if has_ext_id {
            *prof.playlist_artist_affinity.entry(artist_id.clone()).or_insert(0.0) += w;
        } else if !a_key.is_empty() {
            *prof.playlist_artist_affinity.entry(a_key.clone()).or_insert(0.0) += w;
        }
        if !a_key.is_empty() {
            *prof.playlist_artist_slug_affinity.entry(a_key.clone()).or_insert(0.0) += w;
            if has_ext_id {
                prof.artist_slug_ext_ids
                    .entry(a_key.clone())
                    .or_default()
                    .insert(artist_id);
            }
        }
        for cs in collab_slugs {
            if cs != a_key {
                *prof.playlist_artist_slug_affinity.entry(cs).or_insert(0.0) += w * 0.7;
            }
        }
    } else {
        if has_ext_id {
            *prof.artist_affinity.entry(artist_id.clone()).or_insert(0.0) += w;
        } else if !a_key.is_empty() {
            *prof.artist_affinity.entry(a_key.clone()).or_insert(0.0) += w;
        }
        if !a_key.is_empty() {
            *prof.artist_slug_affinity.entry(a_key.clone()).or_insert(0.0) += w;
            if has_ext_id {
                prof.artist_slug_ext_ids
                    .entry(a_key.clone())
                    .or_default()
                    .insert(artist_id);
            }
        }
        for cs in collab_slugs {
            if cs != a_key {
                *prof.artist_slug_affinity.entry(cs).or_insert(0.0) += w * 0.7;
            }
        }
    }

    let target_genre = if playlist_layer {
        &mut prof.playlist_genre_affinity
    } else {
        &mut prof.genre_affinity
    };
    for gl in split_genres(&t.genre) {
        *target_genre.entry(gl).or_insert(0.0) += w;
    }

    let target_mood = if playlist_layer {
        &mut prof.playlist_mood_affinity
    } else {
        &mut prof.mood_affinity
    };
    for m in t.mood_list() {
        *target_mood.entry(m).or_insert(0.0) += w;
    }
}

fn build_track_affinities(t: &Track, prof: &TasteProfile) -> TrackAffinities {
    let artist_id = t.artist_id();
    let has_ext_id = artist_id.starts_with("ar:dz:") || artist_id.starts_with("ar:ytm:");
    let artist_slug = slug(&primary_artist(&t.artist));
    let ext_provider_conflict = |ext_id: &str, slug_key: &str| -> bool {
        let Some(known) = prof.artist_slug_ext_ids.get(slug_key) else {
            return false;
        };
        let prefix = if ext_id.starts_with("ar:dz:") {
            Some("ar:dz:")
        } else if ext_id.starts_with("ar:ytm:") {
            Some("ar:ytm:")
        } else {
            None
        };
        if let Some(p) = prefix {
            known.iter().any(|k| k.starts_with(p) && k != ext_id)
        } else {
            false
        }
    };
    let mut artist_affinity = if has_ext_id {
        prof.artist_affinity
            .get(&artist_id)
            .copied()
            .unwrap_or_else(|| {
                if ext_provider_conflict(&artist_id, &artist_slug) {
                    0.0
                } else {
                    prof.artist_slug_affinity
                        .get(&artist_slug)
                        .or_else(|| prof.artist_affinity.get(&artist_slug))
                        .copied()
                        .unwrap_or(0.0)
                }
            })
    } else {
        prof.artist_affinity
            .get(&artist_id)
            .or_else(|| prof.artist_slug_affinity.get(&artist_slug))
            .or_else(|| prof.artist_affinity.get(&artist_slug))
            .copied()
            .unwrap_or(0.0)
    };
    let mut playlist_artist_affinity = if has_ext_id {
        prof.playlist_artist_affinity
            .get(&artist_id)
            .copied()
            .unwrap_or_else(|| {
                if ext_provider_conflict(&artist_id, &artist_slug) {
                    0.0
                } else {
                    prof.playlist_artist_slug_affinity
                        .get(&artist_slug)
                        .or_else(|| prof.playlist_artist_affinity.get(&artist_slug))
                        .copied()
                        .unwrap_or(0.0)
                }
            })
    } else {
        prof.playlist_artist_affinity
            .get(&artist_id)
            .or_else(|| prof.playlist_artist_slug_affinity.get(&artist_slug))
            .or_else(|| prof.playlist_artist_affinity.get(&artist_slug))
            .copied()
            .unwrap_or(0.0)
    };
    let collab_slugs = collab_artist_slugs(&t.artist);
    if !collab_slugs.is_empty() {
        let best_collab = collab_slugs
            .iter()
            .filter_map(|cs| {
                prof.artist_slug_affinity
                    .get(cs)
                    .or_else(|| prof.artist_affinity.get(cs))
                    .copied()
            })
            .fold(0.0f64, f64::max);
        if best_collab > 0.0 {
            artist_affinity = artist_affinity.max(best_collab * 0.85);
        }
        let best_pl_collab = collab_slugs
            .iter()
            .filter_map(|cs| {
                prof.playlist_artist_slug_affinity
                    .get(cs)
                    .or_else(|| prof.playlist_artist_affinity.get(cs))
                    .copied()
            })
            .fold(0.0f64, f64::max);
        if best_pl_collab > 0.0 {
            playlist_artist_affinity = playlist_artist_affinity.max(best_pl_collab * 0.85);
        }
    }

    let genres = split_genres(&t.genre);
    let genre_affinity = genres
        .iter()
        .filter_map(|g| prof.genre_affinity.get(g))
        .fold(0.0f64, |acc, v| if v.abs() > acc.abs() { *v } else { acc });
    let playlist_genre_affinity = genres
        .iter()
        .filter_map(|g| prof.playlist_genre_affinity.get(g))
        .fold(0.0f64, |a, b| a.max(*b));

    let moods = t.mood_list();
    let mood_affinity = if moods.is_empty() {
        0.0
    } else {
        moods
            .iter()
            .filter_map(|m| prof.mood_affinity.get(m))
            .sum::<f64>()
            / (moods.len() as f64)
    };
    let playlist_mood_affinity = if moods.is_empty() {
        0.0
    } else {
        moods
            .iter()
            .filter_map(|m| prof.playlist_mood_affinity.get(m))
            .sum::<f64>()
            / (moods.len() as f64)
    };

    let followed_artist = if has_ext_id {
        prof.followed_artists.contains(&artist_id)
    } else {
        prof.followed_artists.contains(&artist_id)
            || prof.followed_artists.contains(&format!("ar:{}", artist_slug))
            || prof.followed_artists.contains(&artist_slug)
    } || collab_slugs.iter().any(|cs| {
        prof.followed_artists.contains(&format!("ar:{}", cs))
            || prof.followed_artists.contains(cs)
    });
    let saved_album = prof.saved_albums.contains(&t.album_id());

    TrackAffinities {
        artist_affinity,
        genre_affinity,
        mood_affinity,
        playlist_artist_affinity,
        playlist_genre_affinity,
        playlist_mood_affinity,
        followed_artist,
        saved_album,
    }
}

pub fn parse_search_intent(raw: &str) -> SearchIntent {
    let trimmed = raw.trim();
    let normalized = trimmed.to_lowercase();
    let clean_norm = normalized.replace('\'', "");
    let tokens: Vec<String> = clean_norm
        .split(|c: char| !c.is_alphanumeric() && c != '-' && c != '&')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();

    let mut decade_label = None;
    let mut year_range = None;
    let mut wants_hits = false;
    let mut wants_classic = false;
    let mut explicit_mix_word = false;
    let mut moods: Vec<Mood> = Vec::new();
    let mut genres: Vec<String> = Vec::new();
    let mut themes: Vec<ThemeSpec> = Vec::new();
    let mut matched_token_flags = vec![false; tokens.len()];

    let mut similarity_prefix_entity: Option<String> = None;
    for pref in [
        "songs like ",
        "tracks like ",
        "music like ",
        "artists like ",
        "similar to ",
        "more like ",
        "radio for ",
        "mix like ",
    ] {
        if normalized.starts_with(pref) {
            let rest = trimmed[pref.len()..].trim();
            if !rest.is_empty() {
                similarity_prefix_entity = Some(rest.to_string());
                explicit_mix_word = true;
            }
            break;
        }
    }

    let theme_catalog: &[(&[&str], ThemeSpec)] = &[
        (
            &["christmas", "xmas", "holiday", "holidays", "santa", "noel", "yuletide"],
            ThemeSpec {
                key: "christmas",
                label: "Christmas",
                keywords: &[
                    "christmas", "xmas", "santa", "jingle", "silent night", "white christmas",
                    "last christmas", "let it snow", "winter wonderland", "rudolph", "frosty",
                    "feliz navidad", "mistletoe", "sleigh", "holly jolly", "noel", "carol",
                    "all i want for christmas", "rockin' around", "it's beginning to look",
                    "wonderful christmastime", "happy xmas", "o holy night",
                ],
                search_boost: "classic christmas",
            },
        ),
        (
            &["halloween", "spooky"],
            ThemeSpec {
                key: "halloween",
                label: "Halloween",
                keywords: &["halloween", "spooky", "ghost", "monster", "thriller", "witch", "zombie", "nightmare"],
                search_boost: "halloween hits",
            },
        ),
        (
            &["summer", "beach", "sunny", "tropical"],
            ThemeSpec {
                key: "summer",
                label: "Summer",
                keywords: &["summer", "sun", "sunshine", "beach", "island", "ocean", "california", "tropical", "heat"],
                search_boost: "summer hits",
            },
        ),
        (
            &["workout", "gym", "cardio", "fitness", "running"],
            ThemeSpec {
                key: "workout",
                label: "Workout",
                keywords: &["workout", "power", "stronger", "energy", "run", "hype", "pump", "beast"],
                search_boost: "workout hits",
            },
        ),
        (
            &["sleep", "bedtime", "rainy", "cozy"],
            ThemeSpec {
                key: "sleep",
                label: "Sleep & Cozy",
                keywords: &["sleep", "rain", "ambient", "soft", "dream", "acoustic", "calm", "lullaby"],
                search_boost: "chill sleep ambient",
            },
        ),
        (
            &["roadtrip", "driving", "cruise"],
            ThemeSpec {
                key: "roadtrip",
                label: "Road Trip",
                keywords: &["drive", "highway", "road", "ride", "cruise", "nightcall"],
                search_boost: "road trip hits",
            },
        ),
    ];

    let genre_map: &[(&str, &str)] = &[
        ("hip hop", "Hip-Hop"),
        ("hip-hop", "Hip-Hop"),
        ("hiphop", "Hip-Hop"),
        ("rap", "Rap"),
        ("trap", "Trap"),
        ("r&b", "R&B"),
        ("rnb", "R&B"),
        ("neo soul", "Neo-Soul"),
        ("neo-soul", "Neo-Soul"),
        ("neosoul", "Neo-Soul"),
        ("new wave", "New Wave"),
        ("synthpop", "Synthpop"),
        ("synth-pop", "Synthpop"),
        ("synth pop", "Synthpop"),
        ("synthwave", "Synthwave"),
        ("dream pop", "Dream Pop"),
        ("dream-pop", "Dream Pop"),
        ("dreampop", "Dream Pop"),
        ("japanese city pop", "City Pop"),
        ("city pop", "City Pop"),
        ("city-pop", "City Pop"),
        ("citypop", "City Pop"),
        ("japanese pop", "J-Pop"),
        ("j pop", "J-Pop"),
        ("j-pop", "J-Pop"),
        ("jpop", "J-Pop"),
        ("japanese rock", "J-Rock"),
        ("j rock", "J-Rock"),
        ("j-rock", "J-Rock"),
        ("jrock", "J-Rock"),
        ("korean pop", "K-Pop"),
        ("k pop", "K-Pop"),
        ("k-pop", "K-Pop"),
        ("kpop", "K-Pop"),
        ("chinese pop", "C-Pop"),
        ("c pop", "C-Pop"),
        ("c-pop", "C-Pop"),
        ("cpop", "C-Pop"),
        ("mandopop", "C-Pop"),
        ("cantopop", "C-Pop"),
        ("lo-fi", "Lo-Fi"),
        ("lofi", "Lo-Fi"),
        ("drum and bass", "Drum & Bass"),
        ("drum & bass", "Drum & Bass"),
        ("dnb", "Drum & Bass"),
        ("dubstep", "Dubstep"),
        ("phonk", "Phonk"),
        ("vaporwave", "Vaporwave"),
        ("chillwave", "Chillwave"),
        ("shoegaze", "Shoegaze"),
        ("post-punk", "Post-Punk"),
        ("post punk", "Post-Punk"),
        ("postpunk", "Post-Punk"),
        ("pop punk", "Pop Punk"),
        ("pop-punk", "Pop Punk"),
        ("alt rock", "Alternative"),
        ("alt-rock", "Alternative"),
        ("alternative", "Alternative"),
        ("emo", "Emo"),
        ("bossa nova", "Bossa Nova"),
        ("bossa", "Bossa Nova"),
        ("afrobeats", "Afrobeats"),
        ("afrobeat", "Afrobeats"),
        ("reggaeton", "Reggaeton"),
        ("soundtrack", "Soundtrack"),
        ("classic rock", "Classic Rock"),
        ("rock", "Rock"),
        ("pop", "Pop"),
        ("jazz", "Jazz"),
        ("blues", "Blues"),
        ("soul", "Soul"),
        ("funk", "Funk"),
        ("disco", "Disco"),
        ("electronic", "Electronic"),
        ("edm", "EDM"),
        ("house", "House"),
        ("techno", "Techno"),
        ("trance", "Trance"),
        ("metal", "Metal"),
        ("punk", "Punk"),
        ("grunge", "Grunge"),
        ("indie", "Indie"),
        ("folk", "Folk"),
        ("country", "Country"),
        ("classical", "Classical"),
        ("reggae", "Reggae"),
        ("latin", "Latin"),
        ("ambient", "Ambient"),
        ("acoustic", "Acoustic"),
    ];

    let mood_tokens: &[(&str, Mood)] = &[
        ("chill", Mood::Chill),
        ("lofi", Mood::Chill),
        ("lo-fi", Mood::Chill),
        ("relax", Mood::Chill),
        ("energy", Mood::Energy),
        ("workout", Mood::Energy),
        ("hype", Mood::Energy),
        ("night", Mood::LateNight),
        ("midnight", Mood::LateNight),
        ("upbeat", Mood::Upbeat),
        ("happy", Mood::Upbeat),
        ("sad", Mood::Melancholy),
        ("melancholy", Mood::Melancholy),
        ("party", Mood::Party),
        ("dance", Mood::Party),
        ("club", Mood::Party),
        ("acoustic", Mood::Acoustic),
        ("groove", Mood::Groove),
    ];

    let stopwords: HashSet<&str> = [
        "the", "of", "and", "for", "in", "to", "a", "an", "songs", "tracks", "music", "era",
        "from", "with", "all", "&",
    ]
    .into_iter()
    .collect();

    if similarity_prefix_entity.is_none() {
        if clean_norm.contains("late night") {
            moods.push(Mood::LateNight);
        }
        if clean_norm.contains("feel good") {
            moods.push(Mood::Upbeat);
        }
        if clean_norm.contains("rainy day") {
            moods.push(Mood::Melancholy);
        }
        if clean_norm.contains("road trip") {
            if let Some(roadtrip) = theme_catalog.iter().find(|(keys, _)| keys.contains(&"roadtrip")) {
                themes.push(roadtrip.1.clone());
            }
        }

        // Check multi-word genre phrases first so e.g. "city pop" or "pop punk"
        // does not also register generic "Pop" / "Punk".
        let mut phrase_genre_tokens = vec![false; tokens.len()];
        for &(phrase, g_lbl) in genre_map {
            if phrase.contains(' ') && contains_term(&clean_norm, phrase) {
                if !genres.iter().any(|g| g == g_lbl) {
                    genres.push(g_lbl.to_string());
                }
                for (idx, tok) in tokens.iter().enumerate() {
                    if phrase.split_whitespace().any(|p| p == tok) {
                        matched_token_flags[idx] = true;
                        phrase_genre_tokens[idx] = true;
                    }
                }
            }
        }

        for (idx, tok) in tokens.iter().enumerate() {
            if let Some((lbl, min_y, max_y)) = parse_decade_token(tok) {
                decade_label = Some(lbl);
                year_range = Some((min_y, max_y));
                matched_token_flags[idx] = true;
                continue;
            }
            if matches!(
                tok.as_str(),
                "classic" | "classics" | "oldies" | "retro" | "vintage" | "golden" | "traditional" | "throwback"
            ) {
                wants_classic = true;
                matched_token_flags[idx] = true;
                continue;
            }
            if matches!(
                tok.as_str(),
                "hits" | "hit" | "best" | "top" | "greatest" | "essential" | "essentials" | "anthems" | "popular"
            ) {
                wants_hits = true;
                matched_token_flags[idx] = true;
                continue;
            }
            if matches!(tok.as_str(), "playlist" | "mix" | "radio" | "vibes" | "vibe") {
                explicit_mix_word = true;
                matched_token_flags[idx] = true;
                continue;
            }
            if stopwords.contains(tok.as_str()) {
                matched_token_flags[idx] = true;
            }

            for (keys, spec) in theme_catalog {
                if keys.contains(&tok.as_str()) {
                    if !themes.iter().any(|t| t.key == spec.key) {
                        themes.push(spec.clone());
                    }
                    matched_token_flags[idx] = true;
                }
            }
            for &(m_tok, m) in mood_tokens {
                if tok == m_tok {
                    if !moods.contains(&m) {
                        moods.push(m);
                    }
                    matched_token_flags[idx] = true;
                }
            }
            if !phrase_genre_tokens[idx] {
                for &(g_tok, g_lbl) in genre_map {
                    if !g_tok.contains(' ') && tok == g_tok {
                        if !genres.iter().any(|g| g == g_lbl) {
                            genres.push(g_lbl.to_string());
                        }
                        matched_token_flags[idx] = true;
                    }
                }
            }
        }
        if clean_norm.contains("late night")
            || clean_norm.contains("feel good")
            || clean_norm.contains("road trip")
            || clean_norm.contains("rainy day")
        {
            for (idx, tok) in tokens.iter().enumerate() {
                if matches!(
                    tok.as_str(),
                    "late" | "night" | "feel" | "good" | "road" | "trip" | "rainy" | "day"
                ) {
                    matched_token_flags[idx] = true;
                }
            }
        }

        // Treat filler words and regional/style modifiers as mix-query cues when paired with a genre/mood/theme/decade.
        let has_style_cue =
            decade_label.is_some() || !themes.is_empty() || !genres.is_empty() || !moods.is_empty();
        if has_style_cue {
            for (idx, tok) in tokens.iter().enumerate() {
                if matches!(
                    tok.as_str(),
                    "beats"
                        | "tunes"
                        | "jam"
                        | "jams"
                        | "british"
                        | "uk"
                        | "english"
                        | "american"
                        | "us"
                        | "canadian"
                        | "australian"
                        | "irish"
                        | "scottish"
                        | "french"
                        | "german"
                        | "spanish"
                        | "italian"
                        | "portuguese"
                        | "brazilian"
                        | "mexican"
                        | "latin"
                        | "japanese"
                        | "korean"
                        | "chinese"
                        | "taiwanese"
                        | "indian"
                        | "swedish"
                        | "norwegian"
                        | "nordic"
                        | "scandinavian"
                        | "dutch"
                        | "polish"
                        | "russian"
                        | "turkish"
                        | "arabic"
                        | "african"
                        | "nigerian"
                        | "instrumental"
                        | "vocal"
                        | "dark"
                        | "heavy"
                        | "soft"
                        | "smooth"
                        | "underground"
                        | "modern"
                        | "old"
                        | "new"
                        | "early"
                        | "late"
                        | "90s"
                        | "80s"
                ) {
                    matched_token_flags[idx] = true;
                }
            }
        }
    }

    // Add default mood hints from themes when helpful
    for th in &themes {
        match th.key {
            "workout" if !moods.contains(&Mood::Energy) => moods.push(Mood::Energy),
            "sleep" if !moods.contains(&Mood::Chill) => moods.push(Mood::Chill),
            _ => {}
        }
    }

    if decade_label.is_some() && !clean_norm.contains("deep") && !clean_norm.contains("obscure") {
        wants_hits = true;
    }

    let unknown_tokens = matched_token_flags.iter().filter(|&&m| !m).count();
    let has_cue = decade_label.is_some()
        || !themes.is_empty()
        || !genres.is_empty()
        || !moods.is_empty()
        || wants_classic
        || wants_hits
        || explicit_mix_word;
    let is_theme_mix =
        similarity_prefix_entity.is_none() && has_cue && !tokens.is_empty() && unknown_tokens == 0;
    if !is_theme_mix {
        // Avoid single-word genre/mood/theme collisions inside artist/track names (e.g. "Daft Punk", "Pop Smoke", "Pop Mix").
        genres.clear();
        moods.clear();
        themes.clear();
    }

    let mut entity_query = similarity_prefix_entity;
    let mut has_trailing_radio_ambiguity = false;
    if entity_query.is_none() && !is_theme_mix && explicit_mix_word && tokens.len() >= 2 {
        let raw_words: Vec<&str> = trimmed.split_whitespace().collect();
        if raw_words.len() >= 2 {
            let last_clean = raw_words
                .last()
                .copied()
                .unwrap_or_default()
                .trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase();
            if matches!(last_clean.as_str(), "mix" | "playlist" | "radio" | "vibe" | "vibes") {
                let prev_clean = raw_words[raw_words.len() - 2]
                    .trim_matches(|c: char| !c.is_alphanumeric())
                    .to_lowercase();
                let dangling_prep = matches!(
                    prev_clean.as_str(),
                    "the" | "a" | "an" | "on" | "in" | "of" | "to" | "for" | "with" | "by" | "my" | "at" | "from"
                );
                if !dangling_prep {
                    let stripped = raw_words[..raw_words.len() - 1].join(" ");
                    if !stripped.trim().is_empty() {
                        has_trailing_radio_ambiguity = last_clean == "radio";
                        entity_query = Some(stripped);
                    }
                }
            }
        }
    }

    let wrapper_words: HashSet<&str> = [
        "mix", "playlist", "radio", "vibes", "vibe", "hits", "hit", "best", "top", "greatest",
        "essential", "essentials", "anthems", "popular", "like", "similar",
    ]
    .into_iter()
    .collect();
    let content_tokens: Vec<String> = tokens
        .iter()
        .filter(|t| !stopwords.contains(t.as_str()) && !wrapper_words.contains(t.as_str()))
        .cloned()
        .collect();

    SearchIntent {
        raw: trimmed.to_string(),
        normalized,
        content_tokens,
        entity_query,
        explicit_mix_word,
        has_trailing_radio_ambiguity,
        decade_label,
        year_range,
        wants_hits,
        wants_classic,
        moods,
        genres,
        themes,
        is_theme_mix,
    }
}

fn parse_decade_token(tok: &str) -> Option<(&'static str, i32, i32)> {
    match tok {
        "50s" | "1950s" | "fifties" => Some(("50s", 1950, 1959)),
        "60s" | "1960s" | "sixties" => Some(("60s", 1960, 1969)),
        "70s" | "1970s" | "seventies" => Some(("70s", 1970, 1979)),
        "80s" | "1980s" | "eighties" => Some(("80s", 1980, 1989)),
        "90s" | "1990s" | "nineties" => Some(("90s", 1990, 1999)),
        "00s" | "2000s" => Some(("2000s", 2000, 2009)),
        "10s" | "2010s" => Some(("2010s", 2010, 2019)),
        "20s" | "2020s" => Some(("2020s", 2020, 2029)),
        _ => None,
    }
}

fn compute_query_era_match(t: &Track, intent: &SearchIntent) -> f64 {
    if let Some((min_y, max_y)) = intent.year_range {
        if t.year > 1920 {
            if t.year >= min_y && t.year <= max_y {
                return 1.0;
            }
            let dist = if t.year < min_y {
                min_y - t.year
            } else {
                t.year - max_y
            };
            if dist <= 3 {
                return 0.35;
            }
            if dist <= 8 {
                return -0.45;
            }
            return -1.0;
        }
        if let Some(dec) = intent.decade_label {
            let g_low = t
                .genre
                .to_lowercase()
                .replace("'s", "s")
                .replace("1950s", "50s")
                .replace("1960s", "60s")
                .replace("1970s", "70s")
                .replace("1980s", "80s")
                .replace("1990s", "90s")
                .replace("2000s", "00s")
                .replace("2010s", "10s")
                .replace("2020s", "20s");
            if contains_term(&g_low, dec) {
                return 0.75;
            }
        }
        return 0.0;
    }

    if intent.wants_classic && t.year > 1920 {
        if t.year <= 1985 {
            return 0.85;
        }
        if t.year <= 1996 {
            return 0.4;
        }
        if t.year >= 2015 {
            return -0.4;
        }
    }
    0.0
}

fn compute_query_theme_match_with_artist(
    t: &Track,
    artist_genres: &str,
    intent: &SearchIntent,
) -> f64 {
    let title_album_hay = format!("{} {} {} {}", t.title, t.album, t.genre, t.moods).to_lowercase();
    let meta_hay = format!("{} {} {}", t.genre, artist_genres, t.moods).to_lowercase();
    let genre_hay = format!("{} {}", t.genre, artist_genres).to_lowercase();
    let mut score = 0.0f64;

    if !intent.themes.is_empty() {
        let mut hits = 0usize;
        for th in &intent.themes {
            hits += th
                .keywords
                .iter()
                .filter(|kw| contains_term(&title_album_hay, kw))
                .count();
        }
        if hits > 0 {
            score += (0.55 + (hits as f64 - 1.0) * 0.15).min(0.95);
        }
    }

    if !intent.genres.is_empty() {
        let g_hits = intent
            .genres
            .iter()
            .filter(|g| genre_matches_text(&genre_hay, g))
            .count();
        if g_hits > 0 {
            score += (0.45 * g_hits as f64).min(0.75);
        }
    }

    if !intent.moods.is_empty() {
        let tm = t.mood_list();
        let m_hits = intent
            .moods
            .iter()
            .filter(|m| {
                let genre_matched = has_mood_meta_match(&genre_hay, **m);
                is_track_compatible_with_mood(t, artist_genres, **m)
                    && (genre_matched
                        || (!is_mood_title_trap(t, **m)
                            && (tm.contains(m) || has_mood_meta_match(&meta_hay, **m))))
            })
            .count();
        if m_hits > 0 {
            score += (0.35 * m_hits as f64).min(0.6);
        }
    }

    score.clamp(0.0, 1.0)
}

fn is_junk_track_for_query(t: &Track, query_low: &str) -> bool {
    let hay = format!("{} {} {}", t.title, t.artist, t.album).to_lowercase();
    for junk in [
        "karaoke",
        "parody",
        "tribute",
        "made famous by",
        "ringtone",
        "8-bit",
        "8 bit",
        "lullaby rendition",
        "music box",
    ] {
        if hay.contains(junk) && !query_low.contains(junk) {
            return true;
        }
    }
    let artist_low = t.artist.to_lowercase();
    if contains_term(&artist_low, "covers")
        || artist_low.contains("cover band")
        || artist_low.contains("tribute band")
        || (artist_low.ends_with(" hits") && !query_low.contains(&artist_low))
    {
        return true;
    }
    for spam_artist in [
        "hits guys",
        "super hits",
        "greatest hits",
        "party hits",
        "unforgettable hits",
        "dj gotta",
        "kiggo",
        "original music",
        "rock 'n' roll music",
        "rock n roll music",
        "rock & roll music",
        "dance music",
        "party music",
        "chill music",
        "workout music",
        "study music",
        "sleep music",
        "relaxing music",
        "relaxation music",
        "background music",
        "club music",
        "gym music",
        "running music",
        "lofi beats",
        "lo-fi beats",
        "chill beats",
        "study beats",
        "sleep sounds",
        "nature sounds",
        "country music",
        "rock music",
        "pop music",
    ] {
        if artist_low.contains(spam_artist) && !query_low.contains(spam_artist) {
            return true;
        }
    }
    false
}

fn is_mix_clutter_track(t: &Track, query_low: &str) -> bool {
    if is_junk_track_for_query(t, query_low) {
        return true;
    }
    if t.duration > 0 && (t.duration < 35 || t.duration > 1200) {
        return true;
    }
    if t.artist.trim().is_empty() || t.artist.eq_ignore_ascii_case("Unknown Artist") {
        return true;
    }
    let title_album = format!("{} {}", t.title, t.album).to_lowercase();
    if is_cover_or_backing_title(&title_album) {
        let wants_backing = ["instrumental", "cover", "piano", "karaoke", "nightcore", "slowed", "8d"]
            .iter()
            .any(|k| query_low.contains(k));
        if !wants_backing {
            return true;
        }
    }
    for bad in ["sped up", "speed up", "slowed", "nightcore", "8d audio"] {
        if title_album.contains(bad) && !query_low.contains(bad) {
            return true;
        }
    }
    if (title_album.contains("(remix)") || title_album.contains("[remix]") || t.title.to_lowercase().ends_with(" remix"))
        && !["remix", "club", "dance", "edm", "house", "party"]
            .iter()
            .any(|k| query_low.contains(k))
    {
        return true;
    }
    false
}

fn genre_variants(term_low: &str) -> &'static [&'static str] {
    match term_low {
        "classic rock" | "classicrock" => &["classic rock", "classicrock"],
        "j-pop" | "jpop" | "j pop" | "japanese pop" => {
            &["jpop", "j-pop", "j pop", "japanese pop"]
        }
        "j-rock" | "jrock" | "j rock" | "japanese rock" => {
            &["jrock", "j-rock", "j rock", "japanese rock"]
        }
        "k-pop" | "kpop" | "k pop" | "korean pop" => {
            &["kpop", "k-pop", "k pop", "korean pop"]
        }
        "c-pop" | "cpop" | "c pop" | "mandopop" | "cantopop" => {
            &["cpop", "c-pop", "c pop", "mandopop", "cantopop"]
        }
        "city pop" | "city-pop" | "citypop" => &["citypop", "city-pop", "city pop"],
        "synthpop" | "synth-pop" | "synth pop" => &["synthpop", "synth-pop", "synth pop"],
        "dream pop" | "dream-pop" | "dreampop" => &["dreampop", "dream-pop", "dream pop"],
        "neo-soul" | "neo soul" | "neosoul" => &["neosoul", "neo-soul", "neo soul"],
        "post-punk" | "post punk" | "postpunk" => &["postpunk", "post-punk", "post punk"],
        "pop punk" | "pop-punk" => &["pop-punk", "pop punk"],
        "lo-fi" | "lofi" => &["lofi", "lo-fi"],
        "hip-hop" | "hiphop" | "hip hop" => &["hip-hop", "hip hop", "hiphop"],
        "r&b" | "rnb" => &["r&b", "rnb"],
        "drum & bass" | "drum and bass" | "dnb" => {
            &["dnb", "drum and bass", "drum & bass", "drum n bass"]
        }
        "alternative" | "alt-rock" | "alt rock" | "altrock" => {
            &["alternative", "alt-rock", "alt rock", "alternative rock"]
        }
        _ => &[],
    }
}

pub(crate) fn playlist_token_matches(pl_norm: &str, tok: &str) -> bool {
    match tok {
        "christmas" | "xmas" => ["christmas", "xmas", "holiday", "noel"]
            .iter()
            .any(|s| contains_term(pl_norm, s)),
        "classic" => ["classic", "classics", "oldies", "greatest", "throwback", "retro"]
            .iter()
            .any(|s| contains_term(pl_norm, s)),
        "chill" => ["chill", "chillout", "lofi", "lo-fi", "downtempo", "relax"]
            .iter()
            .any(|s| contains_term(pl_norm, s)),
        "workout" => ["workout", "gym", "fitness", "cardio", "running"]
            .iter()
            .any(|s| contains_term(pl_norm, s)),
        "sleep" => ["sleep", "ambient", "calm", "relax", "rain"]
            .iter()
            .any(|s| contains_term(pl_norm, s)),
        "lofi" | "lo-fi" => ["lofi", "lo-fi", "chillhop", "chill"]
            .iter()
            .any(|s| contains_term(pl_norm, s)),
        "hip-hop" | "hiphop" => ["hip-hop", "hip hop", "hiphop", "rap"]
            .iter()
            .any(|s| contains_term(pl_norm, s)),
        "r&b" | "rnb" => ["r&b", "rnb", "soul"]
            .iter()
            .any(|s| contains_term(pl_norm, s)),
        _ => {
            let variants = genre_variants(tok);
            if !variants.is_empty() {
                variants.iter().any(|s| contains_term(pl_norm, s))
            } else {
                contains_term(pl_norm, tok)
            }
        }
    }
}

pub(crate) fn genre_matches_text(hay_low: &str, genre_label: &str) -> bool {
    let g_low = genre_label.to_lowercase();
    let variants = genre_variants(&g_low);
    if !variants.is_empty() {
        variants.iter().any(|v| contains_term(hay_low, v))
    } else {
        contains_term(hay_low, &g_low)
    }
}

pub(crate) fn has_conflicting_scene_genre(
    hay_low: &str,
    wanted_genres: &[String],
    query_low: &str,
) -> bool {
    if wanted_genres.is_empty() {
        return false;
    }
    const SCENE_GROUPS: &[(&[&str], &[&str])] = &[
        (
            &["J-Pop", "J-Rock", "City Pop"],
            &[
                "jpop",
                "j-pop",
                "j pop",
                "jrock",
                "j-rock",
                "j rock",
                "city pop",
                "city-pop",
                "citypop",
                "japanese pop",
                "japanese rock",
            ],
        ),
        (&["K-Pop"], &["kpop", "k-pop", "k pop", "korean pop"]),
        (
            &["C-Pop"],
            &["cpop", "c-pop", "c pop", "mandopop", "cantopop"],
        ),
    ];

    let wants_any_scene = wanted_genres.iter().any(|g| {
        SCENE_GROUPS
            .iter()
            .any(|(labels, _)| labels.iter().any(|l| l.eq_ignore_ascii_case(g)))
    });
    if !wants_any_scene {
        return false;
    }

    for (labels, terms) in SCENE_GROUPS {
        let group_wanted = wanted_genres
            .iter()
            .any(|g| labels.iter().any(|l| l.eq_ignore_ascii_case(g)))
            || terms.iter().any(|t| contains_term(query_low, t));
        if !group_wanted && terms.iter().any(|t| contains_term(hay_low, t)) {
            return true;
        }
    }
    false
}

fn curated_playlist_title_score(pl_title: &str, intent: &SearchIntent) -> Option<f64> {
    if !intent.is_theme_mix {
        return None;
    }
    let pl_norm = pl_title
        .to_lowercase()
        .replace("'s", "s")
        .replace('\'', "")
        .replace("1950s", "50s")
        .replace("1960s", "60s")
        .replace("1970s", "70s")
        .replace("1980s", "80s")
        .replace("1990s", "90s")
        .replace("2000s", "00s")
        .replace("2010s", "10s")
        .replace("2020s", "20s")
        .replace("eighties", "80s")
        .replace("nineties", "90s")
        .replace("seventies", "70s")
        .replace("sixties", "60s")
        .replace("fifties", "50s");

    if let Some(dec) = intent.decade_label {
        let all_decades = ["50s", "60s", "70s", "80s", "90s", "00s", "10s", "20s"];
        let has_wanted = contains_term(&pl_norm, dec);
        let has_other = all_decades
            .iter()
            .any(|&d| d != dec && contains_term(&pl_norm, d));
        if has_other && !has_wanted {
            return None;
        }
    }

    if intent.moods.contains(&Mood::Chill)
        && ["metal", "workout", "gym", "party", "hardstyle"]
            .iter()
            .any(|w| contains_term(&pl_norm, w))
    {
        return None;
    }

    if !intent.genres.is_empty() {
        if !intent.genres.iter().any(|g| genre_matches_text(&pl_norm, g)) {
            return None;
        }
        if has_conflicting_scene_genre(&pl_norm, &intent.genres, &intent.normalized) {
            return None;
        }
    }

    if intent.content_tokens.is_empty() {
        return Some(1.6);
    }

    let matches = intent
        .content_tokens
        .iter()
        .filter(|tok| playlist_token_matches(&pl_norm, tok))
        .count();

    if matches == 0 {
        return None;
    }
    Some(2.0 + 0.35 * (matches.min(3) as f64))
}

fn genre_family(g_low: &str) -> Option<u8> {
    if [
        "christmas",
        "xmas",
        "holiday",
        "holidays",
        "festive",
        "yuletide",
    ]
    .iter()
    .any(|k| contains_term(g_low, k))
    {
        return Some(14);
    }
    if ["k-pop", "kpop", "korean pop"]
        .iter()
        .any(|k| contains_term(g_low, k))
    {
        return Some(9);
    }
    if [
        "j-pop",
        "jpop",
        "japanese pop",
        "city pop",
        "city-pop",
        "citypop",
    ]
    .iter()
    .any(|k| contains_term(g_low, k))
    {
        return Some(10);
    }
    if ["j-rock", "jrock", "japanese rock"]
        .iter()
        .any(|k| contains_term(g_low, k))
    {
        return Some(11);
    }
    if ["c-pop", "cpop", "mandopop", "cantopop"]
        .iter()
        .any(|k| contains_term(g_low, k))
    {
        return Some(12);
    }
    if g_low.contains("asian") {
        return Some(13);
    }
    if ["rock", "alt", "indie", "punk", "metal", "grunge", "emo", "shoegaze"]
        .iter()
        .any(|k| g_low.contains(k))
    {
        return Some(1);
    }
    if ["pop", "synthpop", "disco", "dance pop"]
        .iter()
        .any(|k| g_low.contains(k))
    {
        return Some(2);
    }
    if ["electronic", "edm", "house", "techno", "trance", "synthwave", "dnb", "drum", "dubstep", "ambient", "vaporwave", "chillwave", "electro"]
        .iter()
        .any(|k| g_low.contains(k))
    {
        return Some(3);
    }
    if ["hip-hop", "hip hop", "rap", "trap", "phonk", "r&b", "rnb", "soul", "funk"]
        .iter()
        .any(|k| g_low.contains(k))
    {
        return Some(4);
    }
    if ["jazz", "blues", "bossa"]
        .iter()
        .any(|k| g_low.contains(k))
    {
        return Some(5);
    }
    if ["classical", "soundtrack", "orchestra", "piano", "neoclassical"]
        .iter()
        .any(|k| g_low.contains(k))
    {
        return Some(6);
    }
    if ["folk", "acoustic", "country", "americana", "bluegrass", "singer"]
        .iter()
        .any(|k| g_low.contains(k))
    {
        return Some(7);
    }
    if ["reggae", "latin", "reggaeton", "afro", "salsa"]
        .iter()
        .any(|k| g_low.contains(k))
    {
        return Some(8);
    }
    None
}

/// Deezer's genre taxonomy is coarse: an actual J-Rock or J-Pop track is tagged simply
/// "Rock" / "Pop" on its album. When the user asks for a scene genre, treat that broad
/// parent bucket as the requested scene so a real track is not rejected as off-genre.
/// Other families (rap, latin, k-pop, ...) are left untouched and still clash normally.
pub(crate) fn normalize_scene_genre(t: &mut Track, intent: &SearchIntent) {
    if intent.genres.len() != 1 {
        return;
    }
    let scene = intent.genres[0].as_str();
    let parent_family = match scene {
        "J-Rock" => genre_family("rock"),
        "J-Pop" | "K-Pop" | "C-Pop" | "City Pop" => genre_family("pop"),
        _ => return,
    };
    let Some(parent_family) = parent_family else {
        return;
    };
    let g = t.genre.trim().to_lowercase();
    if g.is_empty() || g == scene.to_lowercase() {
        return;
    }
    if genre_family(&g) != Some(parent_family) {
        return;
    }
    if has_conflicting_scene_genre(&g, &intent.genres, &intent.normalized) {
        return;
    }
    t.genre = scene.to_string();
}

pub(crate) fn has_genre_clash(t: &Track, artist_genres: &str, wanted_genres: &[String]) -> bool {
    if wanted_genres.is_empty() {
        return false;
    }
    let meta = format!("{} {}", t.genre, artist_genres).to_lowercase();
    if meta.trim().is_empty() {
        return false;
    }

    // `contains_term` uses non-alphanumeric boundaries, so "j-rock" is treated
    // as a "rock" match. That let Japanese/Korean/Chinese scenes satisfy plain
    // Western genre queries like "classic rock" (e.g. Mr.Children). Reject
    // scene markers up front unless the query explicitly wants that scene.
    let wants_scene = wanted_genres
        .iter()
        .any(|g| matches!(genre_family(&g.to_lowercase()), Some(f) if f >= 10));
    if !wants_scene {
        const SCENE_MARKERS: &[&str] = &[
            "jpop", "j-pop", "j pop", "japanese pop", "jrock", "j-rock", "j rock",
            "japanese rock", "kpop", "k-pop", "k pop", "korean pop", "cpop", "c-pop",
            "c pop", "mandopop", "cantopop", "city pop", "city-pop", "citypop",
        ];
        if SCENE_MARKERS.iter().any(|s| contains_term(&meta, s)) {
            return true;
        }
    }

    if wanted_genres.iter().any(|g| genre_matches_text(&meta, g)) {
        return false;
    }
    let wanted_families: HashSet<u8> = wanted_genres
        .iter()
        .filter_map(|g| genre_family(&g.to_lowercase()))
        .collect();
    if wanted_families.is_empty() {
        return false;
    }
    let track_families: Vec<u8> = split_genres(&meta)
        .into_iter()
        .filter_map(|g| genre_family(&g))
        .collect();
    if track_families.is_empty() {
        return false;
    }
    !track_families.iter().any(|f| wanted_families.contains(f))
}

fn is_same_or_featured_artist(track_artist: &str, anchor_artist: &str) -> bool {
    let anchor_slug = slug(&primary_artist(anchor_artist));
    if anchor_slug.is_empty() {
        return false;
    }
    if slug(&primary_artist(track_artist)) == anchor_slug {
        return true;
    }
    let low = track_artist.to_lowercase();
    for sep in [
        " feat. ",
        " feat ",
        " ft. ",
        " ft ",
        " featuring ",
        " vs. ",
        " vs ",
        " × ",
        " x ",
        ", ",
        " / ",
        " & ",
    ] {
        if low.contains(sep) {
            for part in low.split(sep) {
                if slug(part.trim()) == anchor_slug {
                    return true;
                }
            }
        }
    }
    false
}

fn merge_track_metadata(winner: &mut Track, loser: &Track) {
    if winner.cover_url.is_empty() && !loser.cover_url.is_empty() {
        winner.cover_url = loser.cover_url.clone();
    }
    if winner.genre.is_empty() && !loser.genre.is_empty() {
        winner.genre = loser.genre.clone();
    }
    if winner.moods.is_empty() && !loser.moods.is_empty() {
        winner.moods = loser.moods.clone();
    }
    if winner.year == 0 && loser.year > 0 {
        winner.year = loser.year;
    }
    if winner.deezer_id.is_empty() && !loser.deezer_id.is_empty() {
        winner.deezer_id = loser.deezer_id.clone();
    }
    if !loser.meta.is_empty() {
        winner.meta = crate::models::merge_meta_json(&winner.meta, &loser.meta);
    }
    if (winner.album.is_empty() || winner.album == winner.title) && !loser.album.is_empty() {
        winner.album = loser.album.clone();
    }
}

struct TasteAlignment {
    has_taste: bool,
    on_taste: bool,
    total_pos_genre: f64,
    total_pos_mood: f64,
    total_pos_pl_mood: f64,
    rel_a: f64,
    rel_t: f64,
    has_explicit_genre: bool,
    has_pos_genre_overlap: bool,
    has_direct_or_related: bool,
}

fn profile_has_taste(prof: &TasteProfile) -> bool {
    let total_pos_artist: f64 = prof.artist_affinity.values().filter(|&&v| v > 0.0).sum();
    let total_pos_genre: f64 = prof.genre_affinity.values().filter(|&&v| v > 0.0).sum::<f64>()
        + prof.playlist_genre_affinity.values().filter(|&&v| v > 0.0).sum::<f64>() * 0.6;
    total_pos_artist > 0.4
        || total_pos_genre > 0.4
        || !prof.starred.is_empty()
        || !prof.manual_playlist_ids.is_empty()
        || !prof.followed_artists.is_empty()
        || !prof.saved_albums.is_empty()
        || prof.ratings.values().any(|&r| r >= 4 || r <= 2)
}

fn compute_taste_alignment(
    t: &Track,
    prof: &TasteProfile,
    fctx: &TrackAffinities,
) -> TasteAlignment {
    let artist_slug = slug(&primary_artist(&t.artist));
    let (rel_a, rel_t) = if fctx.artist_affinity < -0.2 {
        (0.0, 0.0)
    } else {
        (
            prof.related_artist_affinity.get(&artist_slug).copied().unwrap_or(0.0),
            prof.related_track_affinity.get(&t.id).copied().unwrap_or(0.0),
        )
    };

    let total_pos_genre: f64 = prof.genre_affinity.values().filter(|&&v| v > 0.0).sum::<f64>()
        + prof.playlist_genre_affinity.values().filter(|&&v| v > 0.0).sum::<f64>() * 0.6;
    let total_pos_mood: f64 = prof.mood_affinity.values().filter(|&&v| v > 0.0).sum();
    let total_pos_pl_mood: f64 = prof.playlist_mood_affinity.values().filter(|&&v| v > 0.0).sum();

    let has_taste = profile_has_taste(prof);

    let t_genres = split_genres(&t.genre);
    let has_explicit_genre = !t_genres.is_empty();
    let has_pos_genre_overlap = t_genres.iter().any(|g| {
        prof.genre_affinity.get(g).copied().unwrap_or(0.0) > 0.1
            || prof.playlist_genre_affinity.get(g).copied().unwrap_or(0.0) > 0.1
    });

    let has_direct_or_related = fctx.artist_affinity > 0.1
        || fctx.playlist_artist_affinity > 0.1
        || rel_a > 0.0
        || rel_t > 0.0
        || fctx.followed_artist
        || fctx.saved_album
        || prof.starred.contains(&t.id)
        || prof.manual_playlist_ids.contains(&t.id);
    let on_taste = has_direct_or_related
        || has_pos_genre_overlap
        || fctx.genre_affinity > 0.1
        || fctx.playlist_genre_affinity > 0.1;

    TasteAlignment {
        has_taste,
        on_taste,
        total_pos_genre,
        total_pos_mood,
        total_pos_pl_mood,
        rel_a,
        rel_t,
        has_explicit_genre,
        has_pos_genre_overlap,
        has_direct_or_related,
    }
}

pub fn mood_playlist_queries(mood: Mood) -> &'static [&'static str] {
    match mood {
        Mood::Chill => &["lofi chill", "chill vibes"],
        Mood::Energy => &["workout motivation", "high energy rock"],
        Mood::LateNight => &["late night vibes", "after hours"],
        Mood::Upbeat => &["feel good pop", "upbeat hits"],
        Mood::Melancholy => &["sad songs", "melancholy indie"],
        Mood::Party => &["dance party", "club hits"],
        Mood::Acoustic => &["acoustic chill", "folk acoustic"],
        Mood::Groove => &["soul groove", "neo soul"],
        Mood::RockIndie => &["indie rock", "alternative rock"],
    }
}

pub fn mood_deezer_genre_names(mood: Mood) -> &'static [&'static str] {
    match mood {
        Mood::Chill => &["chill", "lofi", "ambient", "jazz"],
        Mood::Energy => &["rock", "metal", "rap"],
        Mood::LateNight => &["electro", "r&b", "jazz"],
        Mood::Upbeat => &["pop", "dance"],
        Mood::Melancholy => &["alternative", "folk", "blues"],
        Mood::Party => &["dance", "electro", "electronic"],
        Mood::Acoustic => &["folk", "country", "singer"],
        Mood::Groove => &["r&b", "soul", "funk", "jazz"],
        Mood::RockIndie => &["alternative", "indie", "rock"],
    }
}

pub fn is_curated_playlist_for_mood(playlist_title: &str, mood: Mood) -> bool {
    let low = playlist_title.to_lowercase();
    for bad in [
        "cover",
        "covers",
        "karaoke",
        "tribute",
        "lullaby",
        "lullabies",
        "white noise",
        "binaural",
        "asmr",
        "tantric",
        "massage",
        "ringtone",
        "8-bit",
        "8d audio",
        "sped up",
        "slowed",
        "nightcore",
    ] {
        if contains_term(&low, bad) {
            return false;
        }
    }

    let clash_hints: &[&str] = match mood {
        Mood::Chill => &["workout", "gym", "metal", "hardstyle", "party", "club", "rave"],
        Mood::Energy => &["sleep", "lullaby", "meditation", "sad", "acoustic"],
        Mood::LateNight => &["workout", "gym", "kids", "happy", "morning"],
        Mood::Upbeat => &["sad", "melancholy", "heartbreak", "cry", "gloom", "funeral"],
        Mood::Melancholy => &["party", "workout", "gym", "happy", "upbeat", "dancefloor", "club"],
        Mood::Party => &["sad", "melancholy", "sleep", "study", "acoustic", "folk"],
        Mood::Acoustic => &["techno", "edm", "club", "metal", "dubstep", "hardstyle"],
        Mood::Groove => &["metal", "hardstyle", "classical", "opera"],
        Mood::RockIndie => &["classical", "meditation", "eurodance", "folk essentials"],
    };
    if clash_hints.iter().any(|c| contains_term(&low, c)) {
        return false;
    }

    let hints: &[&str] = match mood {
        Mood::Chill => &["chill", "lofi", "lo-fi", "downtempo", "relax", "dream pop", "bossa", "evening"],
        Mood::Energy => &["energy", "workout", "gym", "power", "rock", "metal", "cardio", "running", "hype"],
        Mood::LateNight => &["late night", "late-night", "midnight", "after hours", "afterhours", "night", "noir", "synthwave", "dark"],
        Mood::Upbeat => &["feel good", "upbeat", "happy", "summer", "sunshine", "pop", "good vibes"],
        Mood::Melancholy => &["sad", "melancholy", "heartbreak", "ballad", "ballads", "rainy", "emotional", "cry", "gloom"],
        Mood::Party => &["party", "dance", "club", "disco", "house", "techno", "edm", "rave"],
        Mood::Acoustic => &["acoustic", "folk", "unplugged", "singer", "country", "camp"],
        Mood::Groove => &["soul", "funk", "groove", "jazz", "r&b", "rnb", "neo soul", "neo-soul", "motown"],
        Mood::RockIndie => &["indie", "alternative", "alt rock", "rock", "shoegaze", "grunge", "britpop"],
    };
    hints.iter().any(|h| contains_term(&low, h))
}

pub fn is_mood_clutter_track(t: &Track) -> bool {
    if is_junk_track_for_query(t, "") {
        return true;
    }
    if t.duration > 0 && t.duration < 30 {
        return true;
    }
    // Reject emoji-stuffed SEO/AI-slop titles (e.g. "Twist the Night - 50s Rock Twist 💃🌙")
    // from remote catalogs while keeping CJK/non-Latin scripts intact.
    if t.source != "local" && t.title.chars().any(|c| (c as u32) >= 0x1F300) {
        return true;
    }
    let title_album = format!("{} {}", t.title, t.album).to_lowercase();
    if is_cover_or_backing_title(&title_album) {
        return true;
    }
    // Reject unofficial mashups / fan bootlegs (e.g. "Gimpson/MC Sobieski/Kessoku Band · kessoku band X gimpson").
    if title_album.contains("mashup")
        || title_album.contains("bootleg")
        || title_album.contains("ai cover")
        || title_album.contains("fan cover")
        || (t.artist.contains('/') && t.album.to_lowercase().contains(" x "))
    {
        return true;
    }
    for noise in [
        "white noise",
        "brown noise",
        "pink noise",
        "rain sounds",
        "binaural",
        "432 hz",
        "432hz",
        "528hz",
        "sleep frequency",
        "asmr",
    ] {
        if title_album.contains(noise) {
            return true;
        }
    }
    false
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ScriptFamily {
    Arabic,
    Indic,
    Cyrillic,
    Thai,
    Hebrew,
    Greek,
    Japanese,
    Korean,
    Cjk,
}

fn non_latin_scripts_in_str(s: &str) -> HashSet<ScriptFamily> {
    let mut out = HashSet::new();
    for ch in s.chars() {
        let cp = ch as u32;
        let fam = match cp {
            0x0600..=0x06FF | 0x0750..=0x077F | 0x08A0..=0x08FF => Some(ScriptFamily::Arabic),
            0x0900..=0x0D7F => Some(ScriptFamily::Indic),
            0x0400..=0x04FF | 0x0500..=0x052F => Some(ScriptFamily::Cyrillic),
            0x0E00..=0x0E7F => Some(ScriptFamily::Thai),
            0x0590..=0x05FF => Some(ScriptFamily::Hebrew),
            0x0370..=0x03FF => Some(ScriptFamily::Greek),
            0x3040..=0x309F | 0x30A0..=0x30FF => Some(ScriptFamily::Japanese),
            0xAC00..=0xD7AF | 0x1100..=0x11FF | 0x3130..=0x318F => Some(ScriptFamily::Korean),
            0x4E00..=0x9FFF | 0x3400..=0x4DBF => Some(ScriptFamily::Cjk),
            _ => None,
        };
        if let Some(f) = fam {
            out.insert(f);
        }
    }
    out
}

fn has_any_non_latin_script(t: &Track) -> bool {
    !non_latin_scripts_in_str(&t.title).is_empty()
        || !non_latin_scripts_in_str(&t.artist).is_empty()
}

fn allowed_non_latin_scripts_for_user(
    prof: &TasteProfile,
    stats: &HashMap<String, UserTrackStats>,
    candidates: &[Track],
) -> HashSet<ScriptFamily> {
    let mut allowed = HashSet::new();
    for t in candidates {
        if is_personal_mix_track(t, prof, stats) {
            for f in non_latin_scripts_in_str(&t.title) {
                allowed.insert(f);
            }
            for f in non_latin_scripts_in_str(&t.artist) {
                allowed.insert(f);
            }
        }
    }
    if allowed.contains(&ScriptFamily::Japanese) {
        allowed.insert(ScriptFamily::Cjk);
    }
    for (g, &aff) in prof
        .genre_affinity
        .iter()
        .chain(prof.playlist_genre_affinity.iter())
    {
        if aff <= 0.2 {
            continue;
        }
        let gl = g.to_lowercase();
        if ["j-pop", "jpop", "j-rock", "jrock", "anime", "japanese", "city pop"]
            .iter()
            .any(|k| contains_term(&gl, k))
        {
            allowed.insert(ScriptFamily::Japanese);
            allowed.insert(ScriptFamily::Cjk);
        }
        if ["k-pop", "kpop", "k-rock", "krock", "korean"]
            .iter()
            .any(|k| contains_term(&gl, k))
        {
            allowed.insert(ScriptFamily::Korean);
        }
        if ["c-pop", "cpop", "mandopop", "cantopop", "chinese"]
            .iter()
            .any(|k| contains_term(&gl, k))
        {
            allowed.insert(ScriptFamily::Cjk);
        }
        if ["arabic", "khaleeji", "egyptian", "dabke", "mahraganat"]
            .iter()
            .any(|k| contains_term(&gl, k))
        {
            allowed.insert(ScriptFamily::Arabic);
        }
        if [
            "indian",
            "bollywood",
            "hindi",
            "punjabi",
            "tamil",
            "telugu",
            "malayalam",
            "marathi",
            "kannada",
            "bengali",
            "desi",
        ]
        .iter()
        .any(|k| contains_term(&gl, k))
        {
            allowed.insert(ScriptFamily::Indic);
        }
        if ["russian", "ukrainian"].iter().any(|k| contains_term(&gl, k)) {
            allowed.insert(ScriptFamily::Cyrillic);
        }
        if contains_term(&gl, "thai") {
            allowed.insert(ScriptFamily::Thai);
        }
        if contains_term(&gl, "greek") {
            allowed.insert(ScriptFamily::Greek);
        }
        if contains_term(&gl, "hebrew") {
            allowed.insert(ScriptFamily::Hebrew);
        }
    }
    allowed
}

fn track_scripts_allowed(t: &Track, allowed: &HashSet<ScriptFamily>) -> bool {
    let mut scripts = non_latin_scripts_in_str(&t.title);
    scripts.extend(non_latin_scripts_in_str(&t.artist));
    if scripts.is_empty() {
        return true;
    }
    scripts.iter().all(|f| allowed.contains(f))
}

fn is_live_concert_title(title: &str) -> bool {
    let low = title.to_lowercase();
    [
        "(live with ",
        "(live at ",
        "(live from ",
        "(live in ",
        "[live with ",
        "[live at ",
        "[live from ",
        "[live in ",
        " - live at ",
        " - live in ",
        " - live from ",
    ]
    .iter()
    .any(|p| low.contains(p))
}

pub fn has_mood_meta_match(meta_low: &str, mood: Mood) -> bool {
    if meta_low.trim().is_empty() {
        return false;
    }
    MOOD_META_RULES
        .iter()
        .find(|(m, _)| *m == mood)
        .map_or(false, |(_, keys)| keys.iter().any(|k| contains_term(meta_low, k)))
}

pub fn is_track_compatible_with_mood(t: &Track, artist_genres: &str, mood: Mood) -> bool {
    if is_mood_clutter_track(t) {
        return false;
    }
    let g_low = format!("{} {}", t.genre, artist_genres).to_lowercase();
    if g_low.trim().is_empty() {
        return true;
    }
    if has_mood_meta_match(&g_low, mood) {
        return true;
    }
    let clashes: &[&str] = match mood {
        Mood::Chill => &[
            "metal",
            "heavy metal",
            "death metal",
            "thrash",
            "hardcore",
            "punk",
            "hardstyle",
            "phonk",
            "dubstep",
            "rave",
            "eurodance",
            "techno",
        ],
        Mood::Acoustic => &[
            "edm",
            "techno",
            "dubstep",
            "hardstyle",
            "trap",
            "phonk",
            "trance",
            "house",
            "eurodance",
            "club",
            "electro",
            "electronic",
            "synthwave",
            "heavy metal",
            "death metal",
        ],
        Mood::Melancholy => &[
            "hardstyle",
            "rave",
            "edm",
            "club",
            "eurodance",
            "party",
            "phonk",
            "thrash",
            "disco",
            "dance",
            "house",
            "techno",
            "reggaeton",
            "salsa",
            "cumbia",
            "amapiano",
            "happy",
            "upbeat",
        ],
        Mood::Energy => &[
            "lullaby",
            "meditation",
            "new age",
            "ambient",
            "classical",
            "easy listening",
            "slowcore",
            "lounge",
            "bossa nova",
            "lo-fi",
            "lofi",
        ],
        Mood::Party => &[
            "classical",
            "lullaby",
            "ambient",
            "meditation",
            "slowcore",
            "acoustic",
            "bluegrass",
            "country",
            "folk",
            "indie folk",
            "americana",
            "singer-songwriter",
            "singer songwriter",
            "rockabilly",
            "rock & roll",
            "rock 'n' roll",
            "rock n roll",
            "metal",
            "heavy metal",
            "death metal",
            "black metal",
            "thrash",
            "grunge",
            "emo",
            "sad",
            "melancholy",
            "ballad",
            "soundtrack",
            "films/games",
            "opera",
            "blues",
            "jazz",
            "gospel",
            "kids",
            "children",
            "nursery",
        ],
        Mood::Upbeat => &[
            "death metal",
            "black metal",
            "doom metal",
            "funeral",
            "slowcore",
            "dark ambient",
            "drone",
            "melancholy",
            "sad",
            "emo",
        ],
        Mood::LateNight => &[
            "kids",
            "children",
            "nursery",
            "christian",
            "gospel",
            "bluegrass",
            "country",
            "polka",
            "eurodance",
            "happy",
        ],
        Mood::Groove => &["death metal", "black metal", "hardstyle", "bluegrass", "opera", "classical"],
        Mood::RockIndie => &[
            "classical",
            "kids",
            "meditation",
            "new age",
            "eurodance",
            "reggaeton",
            "country",
            "salsa",
            "bachata",
            "cumbia",
            "merengue",
            "opera",
            "trance",
            "hardstyle",
        ],
    };
    !clashes.iter().any(|c| contains_term(&g_low, c))
}

/// Whether a listener's affinity genre is a sensible modifier to pair with `mood` when
/// fetching personalized mood playlists (e.g. `"pop dance"` is great, while `"rock dance"`
/// or `"country dance"` pulls in 1950s rockabilly / line-dance AI slop).
fn is_genre_fit_for_mood_query(genre: &str, mood: Mood) -> bool {
    let g_low = genre.trim().to_lowercase();
    if g_low.is_empty() {
        return false;
    }
    if !is_track_compatible_with_mood(
        &Track {
            genre: genre.to_string(),
            ..Default::default()
        },
        "",
        mood,
    ) {
        return false;
    }
    if has_mood_meta_match(&g_low, mood) {
        return true;
    }
    // Avoid combining generic/non-mood umbrella genres with mood hints where playlist
    // search diverges into novelty/retro playlists (e.g. "rock dance" -> 50s twist).
    match mood {
        Mood::Party => [
            "pop",
            "hip-hop",
            "hip hop",
            "rap",
            "r&b",
            "rnb",
            "funk",
            "latin",
            "afrobeats",
            "afropop",
            "k-pop",
            "kpop",
            "j-pop",
            "jpop",
            "synthpop",
            "electropop",
            "indie pop",
        ]
        .iter()
        .any(|k| contains_term(&g_low, k)),
        Mood::Chill | Mood::LateNight | Mood::Melancholy | Mood::Acoustic => {
            !matches!(g_low.as_str(), "soundtrack" | "films/games" | "rock" | "metal")
        }
        _ => !matches!(g_low.as_str(), "soundtrack" | "films/games"),
    }
}

fn is_mood_title_trap(t: &Track, mood: Mood) -> bool {
    let tl = crate::models::core_song_title(&t.title).to_lowercase();
    let traps: &[&str] = match mood {
        Mood::Chill => &["chill vibes", "lofi chill", "chill music"],
        Mood::Energy => &["workout", "workout hits", "high energy", "gym motivation"],
        Mood::LateNight => &["late night", "late night vibes", "late-night vibes", "after hours"],
        Mood::Upbeat => &["feel good", "upbeat hits", "happy hits"],
        Mood::Melancholy => &["sad songs", "melancholy"],
        Mood::Party => &["dance party", "club hits", "party hits"],
        Mood::Acoustic => &["acoustic chill", "folk acoustic"],
        Mood::Groove => &["soul groove", "neo soul"],
        Mood::RockIndie => &["indie rock", "alternative rock"],
    };
    traps.iter().any(|p| tl == *p || contains_term(&tl, p))
}

pub fn append_mood_slug(existing: &str, mood: Mood) -> String {
    let mut list: Vec<Mood> = existing
        .split(',')
        .filter_map(Mood::from_slug)
        .collect();
    if !list.contains(&mood) {
        if list.len() >= 3 {
            list.pop();
        }
        list.insert(0, mood);
    }
    list.into_iter()
        .map(|m| m.slug())
        .collect::<Vec<_>>()
        .join(",")
}

fn build_instant_mix_title(
    intent: &SearchIntent,
    tracks: &[Track],
    has_taste: bool,
    anchor_artist_override: Option<&str>,
    seed_track_title: Option<&str>,
    single_artist_only: bool,
) -> (String, String) {
    if intent.is_theme_mix {
        let mut parts: Vec<String> = Vec::new();
        if intent.wants_classic
            && !intent.genres.iter().any(|g| g.to_lowercase().contains("classic"))
        {
            parts.push("Classic".to_string());
        }
        if let Some(dec) = intent.decade_label {
            parts.push(dec.to_string());
        }
        for th in intent.themes.iter().take(1) {
            parts.push(th.label.to_string());
        }
        if intent.themes.is_empty() && !intent.wants_classic {
            if let Some(m) = intent.moods.first() {
                if !intent.genres.is_empty() {
                    let ml = m.label();
                    if !intent.genres.iter().any(|g| ml.to_lowercase().contains(&g.to_lowercase())) {
                        parts.push(ml.to_string());
                    }
                }
            }
        }
        for g in intent.genres.iter().take(2) {
            if !parts.iter().any(|p| p.eq_ignore_ascii_case(g)) {
                parts.push(g.clone());
            }
        }
        if intent.themes.is_empty() && intent.genres.is_empty() {
            if let Some(m) = intent.moods.first() {
                parts.push(m.label().to_string());
            } else if intent.wants_hits || intent.decade_label.is_some() {
                parts.push("Hits".to_string());
            }
        } else if !intent.wants_classic
            && intent.genres.is_empty()
            && (intent.wants_hits || intent.themes.len() == 1)
        {
            parts.push("Hits".to_string());
        }

        if parts.is_empty() {
            parts.push(title_case_query(&intent.raw));
        }
        let title = format!("{} Mix", parts.join(" "));

        // Describe dominant vibe & taste personalization in subtitle.
        let mut genre_counts: HashMap<String, usize> = HashMap::new();
        for t in tracks {
            for g in split_genres(&t.genre) {
                *genre_counts.entry(title_case_query(&g)).or_insert(0) += 1;
            }
        }
        let mut top_genres: Vec<(String, usize)> = genre_counts.into_iter().collect();
        top_genres.sort_by_key(|a| std::cmp::Reverse(a.1));
        let vibe_hint = if !top_genres.is_empty() {
            let names: Vec<String> = top_genres.into_iter().take(2).map(|(g, _)| g).collect();
            names.join(" & ")
        } else {
            "Curated Classics & Hits".to_string()
        };
        let subtitle = if has_taste {
            format!("On-the-fly custom mix · {} · Tuned to your taste", vibe_hint)
        } else {
            format!("On-the-fly custom mix · {}", vibe_hint)
        };
        return (title, subtitle);
    }

    let anchor_artist = anchor_artist_override
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .map(str::to_string)
        .or_else(|| {
            tracks
                .first()
                .map(|t| primary_artist(&t.artist))
                .filter(|a| !a.is_empty())
        })
        .unwrap_or_else(|| title_case_query(&intent.raw));

    if single_artist_only {
        let title = format!("{} Mix", anchor_artist);
        let subtitle = if has_taste {
            format!("Essential tracks by {} · Tuned to your taste", anchor_artist)
        } else {
            format!("Essential tracks by {}", anchor_artist)
        };
        return (title, subtitle);
    }

    let title = format!("{} & Similar Mix", anchor_artist);
    let subtitle = match (seed_track_title, has_taste) {
        (Some(seed_t), true) => format!("Seeded from {} · Similar artists & your taste", seed_t),
        (Some(seed_t), false) => format!("Seeded from {} · Radio & similar artists", seed_t),
        (None, true) => "Artist radio & similar artists · Tuned to your taste".to_string(),
        (None, false) => "Artist radio & similar artists".to_string(),
    };
    (title, subtitle)
}

fn title_case_query(s: &str) -> String {
    s.split_whitespace()
        .map(|w| {
            let mut ch = w.chars();
            match ch.next() {
                Some(first) => first.to_uppercase().to_string() + &ch.as_str().to_lowercase(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn build_search_refinements(
    intent: &SearchIntent,
    tracks: &[Track],
    taste_match_ids: &HashSet<String>,
    sctx: &ScoringContext,
    anchor_artist: Option<&str>,
) -> SearchRefinements {
    let mut filter_chips = Vec::new();
    let taste_count = tracks
        .iter()
        .filter(|t| taste_match_ids.contains(&t.id))
        .count();
    if taste_count > 0 {
        filter_chips.push(RefinementChip {
            key: "taste".into(),
            label: "For Your Taste".into(),
            kind: "mode".into(),
            count: taste_count,
        });
    }
    let hits_count = tracks
        .iter()
        .filter(|t| parse_rank_bonus(&t.meta) >= 1.5)
        .count();
    if hits_count > 0 {
        filter_chips.push(RefinementChip {
            key: "hits".into(),
            label: "Popular Hits".into(),
            kind: "mode".into(),
            count: hits_count,
        });
    }
    let lossless_count = tracks
        .iter()
        .filter(|t| t.is_lossless() || t.source == "local")
        .count();
    if lossless_count > 0 {
        filter_chips.push(RefinementChip {
            key: "lossless".into(),
            label: "Lossless / Local".into(),
            kind: "mode".into(),
            count: lossless_count,
        });
    }

    let mut mood_counts: HashMap<Mood, usize> = HashMap::new();
    for t in tracks {
        for m in t.mood_list() {
            *mood_counts.entry(m).or_insert(0) += 1;
        }
    }
    let mut top_moods: Vec<(Mood, usize)> = mood_counts.into_iter().collect();
    top_moods.sort_by(|a, b| {
        let aff_a = sctx.prof.mood_affinity.get(&a.0).copied().unwrap_or(0.0);
        let aff_b = sctx.prof.mood_affinity.get(&b.0).copied().unwrap_or(0.0);
        (b.1 as f64 + aff_b * 2.0)
            .partial_cmp(&(a.1 as f64 + aff_a * 2.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for (m, c) in top_moods.into_iter().take(3) {
        if c >= 2 {
            filter_chips.push(RefinementChip {
                key: format!("mood:{}", m.slug()),
                label: m.label().to_string(),
                kind: "mood".into(),
                count: c,
            });
        }
    }

    // Suggest refined sub-queries ordered by user's genre affinities.
    let mut suggestions = Vec::new();
    if let Some(dec) = intent.decade_label {
        let mut sub_genres = vec!["Pop", "Rock", "Synthpop", "R&B", "Hip-Hop", "Dance"];
        sub_genres.sort_by(|a, b| {
            let fa = sctx.prof.genre_affinity.get(&a.to_lowercase()).copied().unwrap_or(0.0);
            let fb = sctx.prof.genre_affinity.get(&b.to_lowercase()).copied().unwrap_or(0.0);
            fb.partial_cmp(&fa).unwrap_or(std::cmp::Ordering::Equal)
        });
        for g in sub_genres.into_iter().take(4) {
            let cand = format!("{} {}", dec, g);
            if !cand.eq_ignore_ascii_case(&intent.raw) {
                suggestions.push(cand);
            }
        }
    } else if intent.themes.iter().any(|t| t.key == "christmas") {
        for cand in ["Classic Christmas", "Christmas Jazz", "Christmas Pop", "Upbeat Christmas"] {
            if !cand.eq_ignore_ascii_case(&intent.raw) {
                suggestions.push(cand.to_string());
            }
        }
    } else if !tracks.is_empty() && !intent.is_theme_mix {
        let base_q = intent.entity_query.as_deref().unwrap_or(&intent.raw);
        if let Some(art) = anchor_artist {
            if !art.eq_ignore_ascii_case(base_q) {
                suggestions.push(art.to_string());
                suggestions.push(format!("{} mix", art));
            }
        }
        if !intent.explicit_mix_word && suggestions.len() < 2 {
            suggestions.push(format!("{} mix", base_q));
        }
    }

    let detected_intent = if intent.is_theme_mix {
        if let Some(dec) = intent.decade_label {
            format!("{} Era Mix", dec)
        } else if let Some(th) = intent.themes.first() {
            format!("{} Theme Mix", th.label)
        } else if !intent.genres.is_empty() {
            format!("{} Mix", intent.genres[0])
        } else {
            "Vibe Mix".to_string()
        }
    } else if intent.explicit_mix_word {
        "Similar Artists Mix".to_string()
    } else {
        "Taste Ranked".to_string()
    };

    SearchRefinements {
        detected_intent,
        filter_chips,
        query_suggestions: suggestions,
    }
}

fn is_hidden_track(t: &Track, prof: &TasteProfile) -> bool {
    prof.hidden_ids.contains(&t.id)
        || (!prof.hidden_keys.is_empty() && prof.hidden_keys.contains(&t.song_key()))
}

fn is_strongly_disliked(t: &Track, prof: &TasteProfile, stats: &HashMap<String, UserTrackStats>) -> bool {
    if is_hidden_track(t, prof) || prof.ratings.get(&t.id).copied() == Some(1) {
        return true;
    }
    let st = stats.get(&t.id).copied().unwrap_or_default();
    st.skip_count >= st.play_count + 2 && !prof.starred.contains(&t.id) && !prof.manual_playlist_ids.contains(&t.id)
}

fn is_familiar_track(
    t: &Track,
    prof: &TasteProfile,
    stats: &HashMap<String, UserTrackStats>,
) -> bool {
    let st = stats.get(&t.id).copied().unwrap_or_default();
    let key = t.song_key();
    st.play_count > 0
        || prof.played_song_keys.get(&key).copied().unwrap_or(0) > 0
        || prof.starred.contains(&t.id)
        || prof.starred_song_keys.contains(&key)
        || prof.playlist_track_counts.get(&t.id).copied().unwrap_or(0) > 0
        || prof.manual_playlist_song_keys.contains(&key)
        || prof.ratings.contains_key(&t.id)
}

/// How strongly a track belongs to this listener's own musical world. Curated
/// Explore/Mood pools are global, so without this signal they read as random
/// chart filler. Callers still apply their category rules first — this only
/// decides which of the category-correct tracks feel personally relevant.
fn user_realm_score(t: &Track, prof: &TasteProfile, stats: &HashMap<String, UserTrackStats>) -> f64 {
    let mut score = 0.0;
    if is_familiar_track(t, prof, stats) {
        score += 1.2;
    }
    let aff = build_track_affinities(t, prof);
    if aff.artist_affinity > 0.6 || aff.playlist_artist_affinity > 0.6 {
        score += 1.0;
    } else if aff.artist_affinity > 0.15 || aff.genre_affinity > 0.4 {
        score += 0.5;
    }
    if aff.followed_artist || aff.saved_album {
        score += 0.6;
    }
    score
}

/// True only when a track actually belongs to the listener's personal world
/// (familiar track, listened/followed/playlist artist, saved album, or
/// related-artist/related-track graph). Broad genre overlap alone (e.g. a
/// global Western "Pop" chart track matching a J-Pop listener's incidental
/// "pop" tag) does NOT qualify.
fn is_personal_mix_track(
    t: &Track,
    prof: &TasteProfile,
    stats: &HashMap<String, UserTrackStats>,
) -> bool {
    if is_familiar_track(t, prof, stats) {
        return true;
    }
    let fctx = build_track_affinities(t, prof);
    if fctx.artist_affinity > 0.15
        || fctx.playlist_artist_affinity > 0.15
        || fctx.followed_artist
        || fctx.saved_album
    {
        return true;
    }
    let align = compute_taste_alignment(t, prof, &fctx);
    align.rel_a > 0.0 || align.rel_t > 0.0
}

/// Picks up to two mood-curation query genres from a listener's affinity-sorted
/// genres, suppressing overly broad umbrella labels ("pop", "rock", "asian music")
/// when the listener has specific scene genres (e.g. "j-pop", "j-rock", "k-pop").
fn pick_user_mood_query_genres(genres: &[(String, f64)]) -> Vec<String> {
    const SCENE_POP: &[&str] = &[
        "j-pop",
        "jpop",
        "city pop",
        "k-pop",
        "kpop",
        "c-pop",
        "cpop",
        "mandopop",
        "cantopop",
        "anime",
        "japanese music",
        "korean music",
    ];
    const SCENE_ROCK: &[&str] = &["j-rock", "jrock", "k-rock", "krock"];

    let has_scene_pop = genres
        .iter()
        .any(|(g, _)| SCENE_POP.contains(&g.as_str()));
    let has_scene_rock = genres
        .iter()
        .any(|(g, _)| SCENE_ROCK.contains(&g.as_str()));

    let filtered: Vec<String> = genres
        .iter()
        .filter(|(g, _)| {
            let gl = g.as_str();
            if has_scene_pop && matches!(gl, "pop" | "asian music") {
                return false;
            }
            if has_scene_rock && matches!(gl, "rock" | "asian music") {
                return false;
            }
            true
        })
        .take(2)
        .map(|(g, _)| g.clone())
        .collect();

    if !filtered.is_empty() {
        filtered
    } else {
        genres.iter().take(2).map(|(g, _)| g.clone()).collect()
    }
}

fn score_track_core(
    t: &Track,
    prof: &TasteProfile,
    stats: &HashMap<String, UserTrackStats>,
    mode: ScoreMode,
) -> f64 {
    let st = stats.get(&t.id).copied().unwrap_or_default();
    let fctx = build_track_affinities(t, prof);

    let a_aff = fctx.artist_affinity;
    let g_aff = fctx.genre_affinity;
    let m_aff = fctx.mood_affinity;
    let pl_a = fctx.playlist_artist_affinity;
    let pl_g = fctx.playlist_genre_affinity;
    let pl_m = fctx.playlist_mood_affinity;

    let align = compute_taste_alignment(t, prof, &fctx);
    let has_taste = align.has_taste;
    let on_taste = align.on_taste;
    let total_pos_genre = align.total_pos_genre;
    let total_pos_mood = align.total_pos_mood;
    let total_pos_pl_mood = align.total_pos_pl_mood;
    let rel_a = align.rel_a;
    let rel_t = align.rel_t;
    let has_explicit_genre = align.has_explicit_genre;
    let has_pos_genre_overlap = align.has_pos_genre_overlap;
    let has_direct_or_related = align.has_direct_or_related;

    let artist_term = if a_aff >= 0.0 {
        (a_aff / 2.5).tanh() * 5.2
    } else {
        (a_aff / 2.5).tanh() * 5.0
    };
    let rel_term = if rel_a > 0.0 { (rel_a / 1.8).tanh() * 3.4 } else { 0.0 }
        + if rel_t > 0.0 { (rel_t / 1.5).tanh() * 2.6 } else { 0.0 };

    let genre_norm = (total_pos_genre * 0.35).clamp(2.5, 8.0);
    let genre_term = (g_aff / genre_norm).tanh() * 3.0;
    let genre_mismatch_penalty = if total_pos_genre >= 1.2
        && has_explicit_genre
        && !has_pos_genre_overlap
        && !has_direct_or_related
    {
        2.6 * (total_pos_genre / 3.0).clamp(0.5, 1.0)
    } else if has_taste && !on_taste && !has_explicit_genre {
        1.4
    } else {
        0.0
    };

    let mood_norm = (total_pos_mood * 0.35).clamp(2.5, 12.0);
    let mood_term = (m_aff / mood_norm).tanh() * 1.2;

    let pl_mood_norm = (total_pos_pl_mood * 0.35).clamp(2.0, 10.0);
    let pl_term = (pl_a / 2.0).tanh() * 2.2
        + (pl_g / 3.0).tanh() * 1.6
        + (pl_m / pl_mood_norm).tanh() * 0.8;

    let raw_pop = parse_rank_bonus(&t.meta);
    let pop_bonus = if !has_taste {
        raw_pop
    } else if on_taste {
        raw_pop * 0.25
    } else {
        0.0
    };
    let stranger_chart_penalty = if has_taste && !on_taste && raw_pop > 0.5 {
        (raw_pop * 0.45).min(2.2)
    } else {
        0.0
    };

    let quality_bonus = (t.quality_score() as f64) / 400.0;
    let follow_bonus = if fctx.followed_artist { 1.6 } else { 0.0 };
    let album_bonus = if fctx.saved_album { 1.1 } else { 0.0 };

    let identity_bonus = match mode {
        ScoreMode::Supermix => {
            let star = if prof.starred.contains(&t.id) { 3.2 } else { 0.0 };
            let pl = (*prof.playlist_track_counts.get(&t.id).unwrap_or(&0) as f64).min(3.0) * 1.4;
            let rating = prof
                .ratings
                .get(&t.id)
                .map(|r| (*r as f64 - 2.5) * 1.5)
                .unwrap_or(0.0);
            star + pl + rating
        }
        ScoreMode::Discover | ScoreMode::PlaylistDna => 0.0,
    };

    let now = Utc::now().timestamp();
    let recency_penalty = match mode {
        ScoreMode::Supermix if st.last_played > 0 => {
            let age = (now - st.last_played).max(0);
            if age < 1800 {
                -2.0
            } else if age < 7200 {
                -0.8
            } else {
                0.0
            }
        }
        _ => 0.0,
    };
    let skip_penalty = (st.skip_count as f64) * 1.8;

    let raw = pop_bonus
        + artist_term
        + rel_term
        + genre_term
        + mood_term
        + pl_term
        + follow_bonus
        + album_bonus
        + identity_bonus
        + quality_bonus
        + recency_penalty
        - genre_mismatch_penalty
        - stranger_chart_penalty
        - skip_penalty;

    if raw.is_finite() {
        raw
    } else {
        0.0
    }
}

fn playlist_query_hint(name: &str, comment: &str) -> String {
    let stop: HashSet<&str> = [
        "my", "the", "a", "an", "playlist", "mix", "songs", "tracks", "music", "vibes", "new",
    ]
    .into_iter()
    .collect();
    let combined = format!("{} {}", name, comment);
    combined
        .split(|c: char| !c.is_alphanumeric() && c != '-' && c != '&')
        .map(str::trim)
        .filter(|w| w.len() >= 3 && !stop.contains(&w.to_lowercase().as_str()))
        .take(5)
        .collect::<Vec<_>>()
        .join(" ")
}

fn build_playlist_centroid(tracks: &[Track], query_hint: &str) -> PlaylistCentroid {
    let mut c = PlaylistCentroid::default();
    let mut year_sum = 0.0;
    let mut year_n = 0.0;
    let mut dur_sum = 0.0;
    let mut dur_n = 0.0;

    for t in tracks {
        c.existing_ids.insert(t.id.clone());
        c.existing_keys.insert(t.song_key());

        let ak = slug(&primary_artist(&t.artist));
        if !ak.is_empty() {
            *c.artists.entry(ak).or_insert(0.0) += 1.0;
        }
        for g in split_genres(&t.genre) {
            *c.genres.entry(g).or_insert(0.0) += 1.0;
        }
        for m in t.mood_list() {
            *c.moods.entry(m).or_insert(0.0) += 1.0;
        }
        if t.year > 1920 {
            year_sum += t.year as f64;
            year_n += 1.0;
        }
        if t.duration > 30 {
            dur_sum += t.duration as f64;
            dur_n += 1.0;
        }
    }

    let n = (tracks.len().max(1)) as f64;
    for v in c.artists.values_mut() {
        *v /= n;
    }
    for v in c.genres.values_mut() {
        *v /= n;
    }
    for v in c.moods.values_mut() {
        *v /= n;
    }
    if year_n > 0.0 {
        c.mean_year = Some(year_sum / year_n);
    }
    if dur_n > 0.0 {
        c.mean_duration = Some(dur_sum / dur_n);
    }
    c.keywords = query_hint
        .split_whitespace()
        .map(|w| w.to_lowercase())
        .filter(|w| w.len() >= 3)
        .collect();
    c
}

fn score_against_centroid(t: &Track, c: &PlaylistCentroid) -> f64 {
    let mut score = 0.0;

    let ak = slug(&primary_artist(&t.artist));
    if let Some(&aw) = c.artists.get(&ak) {
        score += 3.0 * aw.sqrt().min(1.0);
    }

    let genres = split_genres(&t.genre);
    if !genres.is_empty() && !c.genres.is_empty() {
        let mut g_match = 0.0;
        for g in &genres {
            if let Some(&w) = c.genres.get(g) {
                g_match += w;
            }
        }
        if g_match > 0.0 {
            score += 2.6 * g_match.min(1.5);
        } else if !c.artists.contains_key(&ak) {
            score -= 1.0;
        }
    }

    let moods = t.mood_list();
    if !moods.is_empty() && !c.moods.is_empty() {
        let mut m_match = 0.0;
        for m in &moods {
            if let Some(&w) = c.moods.get(m) {
                m_match += w;
            }
        }
        score += 1.8 * m_match.min(1.5);
    }

    if let (Some(my), true) = (c.mean_year, t.year > 1920) {
        let diff = (t.year as f64 - my).abs();
        score += (1.0 - (diff / 25.0).min(1.0)) * 0.75;
    }

    if let (Some(md), true) = (c.mean_duration, t.duration > 30) {
        let diff = (t.duration as f64 - md).abs();
        score += (1.0 - (diff / 240.0).min(1.0)) * 0.35;
    }

    if !c.keywords.is_empty() {
        let hay = format!("{} {} {} {}", t.title, t.artist, t.genre, t.moods).to_lowercase();
        let hits = c.keywords.iter().filter(|kw| hay.contains(kw.as_str())).count();
        score += (hits as f64) * 1.4;
    }

    score
}

/// Deduplicates tracks by `song_key()`, limits tracks per primary artist to `max_per_artist`
/// Counts how many tracks will survive `dedup_and_interleave(tracks, limit, max_per_artist)`.
/// Used by radio modes to decide whether more live radio hops or catalog fallbacks are needed
/// BEFORE final ranking, avoiding short queues (e.g. Discover returning 8-9 instead of 18).
fn interleaved_capacity(tracks: &[Track], limit: usize, max_per_artist: usize) -> usize {
    if limit == 0 || max_per_artist == 0 {
        return 0;
    }
    let mut seen_keys = HashSet::new();
    let mut artist_counts: HashMap<String, usize> = HashMap::new();
    let mut title_counts: HashSet<String> = HashSet::new();
    let mut total = 0usize;
    for t in tracks {
        if !seen_keys.insert(t.song_key()) {
            continue;
        }
        let tk = slug(&crate::models::core_song_title(&t.title));
        if !tk.is_empty() && title_counts.contains(&tk) {
            continue;
        }
        let ak = slug(&primary_artist(&t.artist));
        let cnt = artist_counts.entry(ak).or_insert(0);
        if *cnt >= max_per_artist {
            continue;
        }
        *cnt += 1;
        if !tk.is_empty() {
            title_counts.insert(tk);
        }
        total += 1;
        if total >= limit {
            break;
        }
    }
    total
}

/// Interleaves tracks with `max_per_artist = 2` for artist variety, relaxing to 3 (then 4)
/// only if the pool has enough unique songs to fill `limit` but fewer distinct artists.
fn dedup_and_interleave_fill(tracks: Vec<Track>, limit: usize) -> Vec<Track> {
    if limit == 0 || tracks.is_empty() {
        return vec![];
    }
    let strict = dedup_and_interleave(tracks.clone(), limit, 2);
    if strict.len() >= limit || tracks.len() <= strict.len() {
        return strict;
    }
    let relaxed = dedup_and_interleave(tracks.clone(), limit, 3);
    if relaxed.len() >= limit || relaxed.len() == strict.len() {
        return relaxed;
    }
    dedup_and_interleave(tracks, limit, 4)
}

/// (with an optional higher cap for an anchor artist), and interleaves artists using a lookahead
/// cooldown window so identical artists never cluster back-to-back.
fn dedup_and_interleave(tracks: Vec<Track>, limit: usize, max_per_artist: usize) -> Vec<Track> {
    dedup_and_interleave_with_anchor(tracks, limit, max_per_artist, None)
}

fn dedup_and_interleave_with_anchor(
    tracks: Vec<Track>,
    limit: usize,
    max_per_artist: usize,
    anchor_cap: Option<(&str, usize)>,
) -> Vec<Track> {
    if limit == 0 || max_per_artist == 0 {
        return vec![];
    }

    let mut seen_keys = HashSet::new();
    let mut pool: Vec<(String, String, Track)> = Vec::with_capacity(tracks.len());
    for t in tracks {
        if seen_keys.insert(t.song_key()) {
            let ak = slug(&primary_artist(&t.artist));
            let tk = slug(&crate::models::core_song_title(&t.title));
            pool.push((ak, tk, t));
        }
    }

    let cap_for = |ak: &str| -> usize {
        if let Some((anchor_slug, cap)) = anchor_cap {
            if !anchor_slug.is_empty() && ak == anchor_slug {
                return cap.max(max_per_artist);
            }
        }
        max_per_artist
    };

    let cooldown = if limit <= 10 { 2 } else { 3 };
    let mut artist_counts: HashMap<String, usize> = HashMap::new();
    let mut title_counts: HashMap<String, usize> = HashMap::new();
    let mut recent_artists: Vec<String> = Vec::with_capacity(limit);
    let mut out = Vec::with_capacity(limit.min(pool.len()));

    while out.len() < limit && !pool.is_empty() {
        let window_start = recent_artists.len().saturating_sub(cooldown);
        let window = &recent_artists[window_start..];

        let title_ok = |tk: &str, counts: &HashMap<String, usize>| -> bool {
            tk.is_empty() || counts.get(tk).copied().unwrap_or(0) == 0
        };

        // Prefer top-ranked track respecting per-artist cap, unique core title, and cooldown spacing.
        let best_spaced = pool.iter().position(|(ak, tk, _)| {
            artist_counts.get(ak).copied().unwrap_or(0) < cap_for(ak)
                && title_ok(tk, &title_counts)
                && !window.contains(ak)
        });

        let chosen_idx = best_spaced.or_else(|| {
            pool.iter().position(|(ak, tk, _)| {
                artist_counts.get(ak).copied().unwrap_or(0) < cap_for(ak)
                    && title_ok(tk, &title_counts)
            })
        });

        let Some(idx) = chosen_idx else {
            break;
        };

        let (ak, tk, track) = pool.remove(idx);
        *artist_counts.entry(ak.clone()).or_insert(0) += 1;
        if !tk.is_empty() {
            *title_counts.entry(tk).or_insert(0) += 1;
        }
        recent_artists.push(ak);
        out.push(track);
    }

    out
}

pub(crate) fn enrich_slsk_hit_with_base(
    hit: &mut Track,
    base: &Track,
    lock_album_track_pos: bool,
) {
    hit.artist = base.artist.clone();
    if lock_album_track_pos || !base.album.is_empty() {
        hit.album = base.album.clone();
    }
    if lock_album_track_pos {
        if base.track_no > 0 {
            hit.track_no = base.track_no;
        }
        if base.disc_no > 0 {
            hit.disc_no = base.disc_no;
        }
    }
    if hit.year == 0 && base.year > 0 {
        hit.year = base.year;
    }
    if hit.genre.is_empty() && !base.genre.is_empty() {
        hit.genre = base.genre.clone();
    }
    if hit.moods.is_empty() && !base.moods.is_empty() {
        hit.moods = base.moods.clone();
    }
    if hit.cover_url.is_empty() && !base.cover_url.is_empty() {
        hit.cover_url = base.cover_url.clone();
    }
    if hit.deezer_id.is_empty() && !base.deezer_id.is_empty() {
        hit.deezer_id = base.deezer_id.clone();
    }
    if !base.meta.is_empty() {
        hit.meta = base.meta.clone();
    }
}

/// Normalizes an album title for cross-source comparison: drops release tags
/// ("(Deluxe)", "[Remastered]") and store suffixes like " - Single" / " - EP".
fn album_match_key(name: &str) -> String {
    let mut s = crate::models::strip_release_tags(name).trim().to_string();
    for suffix in [" - single", " - ep", " (single)", " (ep)"] {
        if s.to_lowercase().ends_with(suffix) {
            let keep = s.len() - suffix.len();
            if s.is_char_boundary(keep) {
                s.truncate(keep);
            }
        }
    }
    slug(s.trim())
}

/// Loose artist comparison across sources (handles "Artist - Topic", features, etc).
fn album_artists_match(want_primary: &str, candidate: &str) -> bool {
    if want_primary.trim().is_empty() {
        return true;
    }
    let a = slug(want_primary);
    let b = slug(&primary_artist(candidate));
    a == b || (a.len() >= 3 && b.len() >= 3 && (a.contains(&b) || b.contains(&a)))
}

fn merge_tracks_dedup(primary: Vec<Track>, secondary: Vec<Track>) -> Vec<Track> {
    let mut indices_by_key: HashMap<String, Vec<usize>> = HashMap::new();
    let mut out: Vec<Track> = Vec::new();
    for t in primary.into_iter().chain(secondary) {
        let key = t.song_key();
        let existing_idx = indices_by_key.get(&key).and_then(|indices| {
            indices
                .iter()
                .copied()
                .find(|&idx| !crate::models::tracks_conflict_artist(&out[idx], &t))
        });
        if let Some(idx) = existing_idx {
            let existing = std::mem::take(&mut out[idx]);
            out[idx] = merge_track_pair(existing, t);
        } else {
            let idx = out.len();
            indices_by_key.entry(key).or_default().push(idx);
            out.push(t);
        }
    }
    out
}

fn parse_deezer_fans(bio: &str) -> Option<u64> {
    let first = bio.split_whitespace().next()?;
    first.parse::<u64>().ok()
}

fn parse_deezer_rank(meta: &str) -> Option<u64> {
    if meta.is_empty() {
        return None;
    }
    serde_json::from_str::<serde_json::Value>(meta)
        .ok()
        .and_then(|v| v.get("rank").and_then(|r| r.as_u64()))
}

fn is_low_rank_deezer_track(t: &Track, min_rank: u64) -> bool {
    if t.source != "deezer" {
        return false;
    }
    match parse_deezer_rank(&t.meta) {
        Some(r) => r < min_rank,
        None => false,
    }
}

fn parse_rank_bonus(meta: &str) -> f64 {
    let rank = parse_deezer_rank(meta).unwrap_or(0);
    if rank == 0 {
        0.0
    } else {
        ((rank as f64) / 200_000.0).clamp(0.2, 5.0)
    }
}

/// Deezer album IDs worth fetching full track lists for, deduplicated and
/// bounded so a large discography cannot spawn unbounded requests.
fn artist_album_ids(albums: &[AlbumSummary], limit: usize) -> Vec<String> {
    let mut seen = HashSet::new();
    albums
        .iter()
        .filter_map(|a| {
            let id = a.deezer_id.trim();
            (!id.is_empty() && id != "0" && seen.insert(id.to_string())).then(|| id.to_string())
        })
        .take(limit)
        .collect()
}

fn merge_albums_dedup(primary: Vec<AlbumSummary>, secondary: Vec<AlbumSummary>) -> Vec<AlbumSummary> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for a in primary.into_iter().chain(secondary) {
        if seen.insert(a.id.clone()) {
            out.push(a);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn cold_start_scoring_favors_popular_rank_over_random_untrained_noise() {
        let store = Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("cold", "pw").unwrap();
        let sources = SourceRegistry::new(
            crate::sources::deezer::DeezerSource::new(),
            crate::sources::ytm::YtmSource::new(None),
            crate::sources::archive::ArchiveSource::new(),
            Arc::new(crate::sources::local::LocalSource::new(store.clone(), vec![])),
            crate::sources::soulseek::SoulseekSource::new(
                store.clone(),
                std::env::temp_dir(),
            ),
        );
        let taste = TasteEngine::new(store.clone(), sources);

        let hit = Track {
            id: "dz:hit".into(),
            source: "deezer".into(),
            title: "Dracula".into(),
            artist: "Tame Impala".into(),
            meta: r#"{"rank":980000}"#.into(),
            bitrate: 256,
            ..Default::default()
        };
        let obscure = Track {
            id: "dz:obscure".into(),
            source: "deezer".into(),
            title: "Random Deep Cut".into(),
            artist: "Obscure Artist".into(),
            meta: "{}".into(),
            bitrate: 256,
            ..Default::default()
        };
        store.upsert_tracks(&[obscure.clone(), hit.clone()]).unwrap();
        // Cold users are seeded from external charts only, so register these
        // catalog tracks as chart entries before asking for quick picks.
        store.record_chart_tracks(&[hit.clone(), obscure.clone()], 1).unwrap();
        let picks = taste.quick_picks(u.id, 5);
        assert_eq!(picks[0].id, "dz:hit");
    }

    #[test]
    fn radio_catalog_supplement_excludes_unrelated_noise_when_radio_already_present() {
        let store = Store::open_memory().unwrap();
        let sources = SourceRegistry::new(
            crate::sources::deezer::DeezerSource::new(),
            crate::sources::ytm::YtmSource::new(None),
            crate::sources::archive::ArchiveSource::new(),
            Arc::new(crate::sources::local::LocalSource::new(store.clone(), vec![])),
            crate::sources::soulseek::SoulseekSource::new(store.clone(), std::env::temp_dir()),
        );
        let taste = TasteEngine::new(store.clone(), sources);

        let seed = Track {
            id: "ytm:R-maTg10QKs".into(),
            source: "ytm".into(),
            title: "ソラゴト - Fake".into(),
            artist: "ASU".into(),
            moods: "late_night".into(),
            ..Default::default()
        };
        let radio_item = Track {
            id: "ytm:1nyZA4kqyYM".into(),
            source: "ytm".into(),
            title: "Spirits of the Sea".into(),
            artist: "ASU".into(),
            ..Default::default()
        };
        let unrelated = Track {
            id: "dz:unrelated".into(),
            source: "deezer".into(),
            title: "Fantasy".into(),
            artist: "Mariah Carey".into(),
            genre: "Pop".into(),
            bitrate: 1411,
            format: "flac".into(),
            ..Default::default()
        };
        let related = Track {
            id: "ytm:related".into(),
            source: "ytm".into(),
            title: "Bitter Etude".into(),
            artist: "ASU".into(),
            ..Default::default()
        };
        store
            .upsert_tracks(&[seed.clone(), radio_item.clone(), unrelated, related.clone()])
            .unwrap();

        let mut seen: HashSet<String> = HashSet::from([seed.song_key(), radio_item.song_key()]);
        let mut out = vec![radio_item];
        taste.supplement_radio_from_catalog(&seed, 10, &mut seen, &mut out);

        assert!(out.iter().any(|t| t.id == "ytm:related"));
        assert!(!out.iter().any(|t| t.id == "dz:unrelated"));
    }

    #[test]
    fn mix4u_day_turns_over_at_local_refresh_hour() {
        // 2026-01-10 03:59:59 UTC and 04:00:00 UTC, listener in UTC.
        let before = 1_768_017_599;
        let at = before + 1;
        assert_eq!(mix4u_day(at, 0), mix4u_day(before, 0) + 1);
        // Same instant, listener at UTC-5: still the previous local evening.
        assert_eq!(mix4u_day(at, -300), mix4u_day(before, -300));
        // ...and their day turns over five hours later, at 04:00 local.
        assert_eq!(mix4u_day(at + 5 * 3600, -300), mix4u_day(before, -300) + 1);
        // Never-rebuilt (timestamp 0) is always an earlier day.
        assert!(mix4u_day(at, 840) > mix4u_day(0, 840));
    }

    #[test]
    fn rotate_out_recent_mix_tracks_keeps_favourites_and_demotes_repeats() {
        let mk = |id: &str| Track {
            id: id.into(),
            title: id.into(),
            artist: id.into(),
            ..Default::default()
        };
        let pool: Vec<Track> = ["t1", "t2", "t3", "t4", "t5", "t6"].iter().map(|s| mk(s)).collect();
        let previous: HashSet<String> = ["t1", "t3", "t4"].iter().map(|s| s.to_string()).collect();
        let ids: Vec<String> = rotate_out_recent_mix_tracks(pool.clone(), &previous, 2)
            .into_iter()
            .map(|t| t.id)
            .collect();
        // t1 is a top favourite and stays put; t3/t4 drop behind the new t5/t6.
        assert_eq!(ids, vec!["t1", "t2", "t5", "t6", "t3", "t4"]);

        let unchanged: Vec<String> = rotate_out_recent_mix_tracks(pool, &HashSet::new(), 2)
            .into_iter()
            .map(|t| t.id)
            .collect();
        assert_eq!(unchanged, vec!["t1", "t2", "t3", "t4", "t5", "t6"]);
    }

    #[test]
    fn dedup_and_interleave_spaces_repeated_artists_apart() {
        let mk = |id: &str, artist: &str| Track {
            id: id.into(),
            title: id.into(),
            artist: artist.into(),
            ..Default::default()
        };
        let input = vec![
            mk("a1", "Alpha"),
            mk("a2", "Alpha"),
            mk("b1", "Beta"),
            mk("c1", "Gamma"),
            mk("a3", "Alpha"),
        ];
        let out = dedup_and_interleave(input, 4, 2);
        let artists: Vec<&str> = out.iter().map(|t| t.artist.as_str()).collect();
        assert_eq!(artists, vec!["Alpha", "Beta", "Gamma", "Alpha"]);
    }

    #[test]
    fn playlists_drive_learning_dna_mix_and_discover_exclusions() {
        let store = Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("curator", "pw").unwrap();
        let sources = SourceRegistry::new(
            crate::sources::deezer::DeezerSource::new(),
            crate::sources::ytm::YtmSource::new(None),
            crate::sources::archive::ArchiveSource::new(),
            Arc::new(crate::sources::local::LocalSource::new(store.clone(), vec![])),
            crate::sources::soulseek::SoulseekSource::new(store.clone(), std::env::temp_dir()),
        );
        let taste = TasteEngine::new(store.clone(), sources);

        let mk = |id: &str, title: &str, artist: &str, genre: &str, moods: &str| Track {
            id: id.into(),
            source: "deezer".into(),
            title: title.into(),
            artist: artist.into(),
            album: format!("{} Album", artist),
            genre: genre.into(),
            moods: moods.into(),
            year: 2022,
            bitrate: 320,
            ..Default::default()
        };

        let in_pl_1 = mk("s1", "Midnight Pulse", "Synth Hero", "Electronic", "late_night,energy");
        let in_pl_2 = mk("s2", "Neon Horizon", "Cyber Muse", "Electronic", "late_night");
        let fresh_match = mk("s3", "Starlight Grid", "Cyber Muse", "Electronic", "late_night");
        let hated = mk("s4", "Harsh Static", "Cyber Muse", "Electronic", "late_night");
        let unrelated = mk("s5", "Country Road", "Banjo Sam", "Country", "acoustic");
        store
            .upsert_tracks(&[
                in_pl_1.clone(),
                in_pl_2.clone(),
                fresh_match.clone(),
                hated.clone(),
                unrelated.clone(),
            ])
            .unwrap();

        let pl = store
            .create_playlist(u.id, "Late Night Synth", "", false, &[in_pl_1.id.clone(), in_pl_2.id.clone()])
            .unwrap();
        taste.record_playlist_additions(u.id, &[in_pl_1.id.clone(), in_pl_2.id.clone()]);
        store.set_rating(u.id, &hated.id, 1).unwrap();
        taste.record_event(u.id, &hated.id, "rate", 1.0);

        let prof = taste.user_profile(u.id);
        assert!(prof.playlist_artist_affinity.get("synth-hero").copied().unwrap_or(0.0) > 0.5);
        assert!(prof.playlist_genre_affinity.get("electronic").copied().unwrap_or(0.0) > 0.5);

        let discover = taste.discover_weekly(u.id, 10);
        let discover_ids: Vec<&str> = discover.iter().map(|t| t.id.as_str()).collect();
        // Discover must suggest matching fresh track, exclude tracks already in playlist, and skip 1-star tracks
        assert!(discover_ids.contains(&"s3"), "discover={:?}", discover_ids);
        assert!(!discover_ids.contains(&"s1"));
        assert!(!discover_ids.contains(&"s2"));

        taste.rebuild_auto_playlists(u.id);
        let dna = store.get_playlist(&format!("auto-u{}-playlist-dna", u.id)).unwrap();
        assert!(dna.is_some());
        assert!(store.get_playlist(&pl.id).unwrap().is_some());
        assert!(taste.for_you_tracks(u.id, 10).iter().all(|t| t.id != "s4"));
    }

    #[test]
    fn playlist_centroid_recommends_coherent_unadded_tracks() {
        let store = Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("jazzer", "pw").unwrap();
        let seed_a = Track {
            id: "j1".into(),
            source: "local".into(),
            title: "Blue Train".into(),
            artist: "John Coltrane".into(),
            genre: "Jazz".into(),
            moods: "chill,late_night".into(),
            year: 1958,
            duration: 420,
            bitrate: 1411,
            format: "flac".into(),
            ..Default::default()
        };
        let match_b = Track {
            id: "j2".into(),
            source: "local".into(),
            title: "So What".into(),
            artist: "Miles Davis".into(),
            genre: "Jazz".into(),
            moods: "chill,late_night".into(),
            year: 1959,
            duration: 540,
            bitrate: 1411,
            format: "flac".into(),
            ..Default::default()
        };
        let mismatch = Track {
            id: "m1".into(),
            source: "local".into(),
            title: "Sludge Pit".into(),
            artist: "Doom Hammer".into(),
            genre: "Metal".into(),
            moods: "energy".into(),
            year: 2021,
            duration: 180,
            bitrate: 1411,
            format: "flac".into(),
            ..Default::default()
        };
        store
            .upsert_tracks(&[seed_a.clone(), match_b.clone(), mismatch])
            .unwrap();

        let pl_tracks = vec![seed_a.clone()];
        let centroid = build_playlist_centroid(&pl_tracks, "Late Night Jazz");
        assert!(!centroid.existing_ids.contains(&match_b.id));
        let sim_match = score_against_centroid(&match_b, &centroid);
        let sim_mismatch = score_against_centroid(&Track {
            id: "m1".into(),
            genre: "Metal".into(),
            moods: "energy".into(),
            ..Default::default()
        }, &centroid);
        assert!(sim_match > sim_mismatch + 2.0, "match={} mismatch={}", sim_match, sim_mismatch);

        // Ensure store lookup & playlist creation work end to end
        let pl = store.create_playlist(u.id, "Late Night Jazz", "", false, &[seed_a.id]).unwrap();
        let stored_tracks = store.playlist_tracks(&pl.id).unwrap();
        assert_eq!(stored_tracks.len(), 1);
    }

    #[test]
    fn parse_search_intent_detects_eras_themes_and_entities() {
        let eighties = parse_search_intent("80s");
        assert_eq!(eighties.decade_label, Some("80s"));
        assert_eq!(eighties.year_range, Some((1980, 1989)));
        assert!(eighties.wants_hits);
        assert!(eighties.is_theme_mix);

        let xmas = parse_search_intent("classic christmas");
        assert!(xmas.wants_classic);
        assert!(xmas.themes.iter().any(|t| t.key == "christmas"));
        assert!(xmas.is_theme_mix);

        let pop_smoke = parse_search_intent("Pop Smoke");
        assert!(!pop_smoke.is_theme_mix);

        let daft = parse_search_intent("Daft Punk");
        assert!(!daft.explicit_mix_word);
        assert!(!daft.is_theme_mix);

        let jrock = parse_search_intent("japanese rock");
        assert!(jrock.genres.iter().any(|g| g == "J-Rock"));
        assert!(jrock.is_theme_mix);

        let jpop = parse_search_intent("japanese pop");
        assert!(jpop.genres.iter().any(|g| g == "J-Pop"));
        assert!(jpop.is_theme_mix);

        let brit = parse_search_intent("british rock");
        assert!(brit.genres.iter().any(|g| g == "Rock"));
        assert!(brit.is_theme_mix);

        let fr_house = parse_search_intent("french house");
        assert!(fr_house.genres.iter().any(|g| g == "House"));
        assert!(fr_house.is_theme_mix);

        let fr_montana = parse_search_intent("French Montana");
        assert!(!fr_montana.is_theme_mix);

        let jp_breakfast = parse_search_intent("Japanese Breakfast");
        assert!(!jp_breakfast.is_theme_mix);
    }

    #[test]
    fn non_hardcoded_topic_queries_avoid_false_entity_hijack() {
        let store = Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("topicuser", "pw").unwrap();
        let sources = SourceRegistry::new(
            crate::sources::deezer::DeezerSource::new(),
            crate::sources::ytm::YtmSource::new(None),
            crate::sources::archive::ArchiveSource::new(),
            Arc::new(crate::sources::local::LocalSource::new(store.clone(), vec![])),
            crate::sources::soulseek::SoulseekSource::new(store.clone(), std::env::temp_dir()),
        );
        let taste = TasteEngine::new(store.clone(), sources);

        let mk = |id: &str, title: &str, artist: &str, genre: &str, meta: &str| Track {
            id: id.into(),
            source: "deezer".into(),
            title: title.into(),
            artist: artist.into(),
            genre: genre.into(),
            duration: 210,
            meta: meta.into(),
            ..Default::default()
        };

        // 1. "christian" should NOT hijack onto Green Day's "Christian's Inferno"
        let gd = mk("gd1", "Christian's Inferno", "Green Day", "Punk Rock", "{\"deezer_rank\":500000}");
        let ch1 = mk("ch1", "Oceans", "Hillsong United", "Christian", "{\"deezer_rank\":400000}");
        let ch2 = mk("ch2", "Reckless Love", "Cory Asbury", "Christian", "{\"deezer_rank\":350000}");
        store.upsert_tracks(&[gd.clone(), ch1.clone(), ch2.clone()]).unwrap();

        assert!(
            taste.plan_entity_mix_anchor(u.id, "christian", &[gd.clone()], &[]).is_none(),
            "partial title match must not hijack topic query"
        );
        // Even with empty curated_batches, "christian" must NOT build "Green Day Mix".
        let res_ch_empty = taste.search_and_refine(u.id, "christian", vec![gd.clone()], vec![]);
        assert!(
            res_ch_empty
                .instant_mix
                .as_ref()
                .map_or(true, |m| !m.title.contains("Green Day")),
            "christian must not hijack onto Green Day"
        );
        let res_ch = taste.search_and_refine_with_context(
            u.id,
            "christian",
            vec![gd],
            vec![("Christian Hits".into(), vec![ch1, ch2])],
            SearchMixContext::default(),
        );
        let mix_ch = res_ch.instant_mix.expect("christian should build topic mix");
        assert_eq!(mix_ch.title, "Christian Mix");
        assert!(mix_ch.is_theme_mix);

        // 3. "praise and worship" should NOT hijack onto local/soulseek "Escovedo. Pete - praise and worship"
        let mut pw_obscure = mk("pw0", "praise and worship", "Escovedo. Pete", "Latin Jazz", "");
        pw_obscure.source = "local".into();
        let pw1 = mk("pw1", "What A Beautiful Name", "Hillsong Worship", "Praise & Worship", "{\"deezer_rank\":400000}");
        let pw2 = mk("pw2", "Goodness of God", "Bethel Music", "Worship", "{\"deezer_rank\":380000}");
        store.upsert_tracks(&[pw_obscure.clone(), pw1.clone(), pw2.clone()]).unwrap();
        assert!(
            taste.plan_entity_mix_anchor(u.id, "praise and worship", &[pw_obscure.clone()], &[]).is_none(),
            "local/conjunction phrase must not hijack onto Escovedo. Pete"
        );
        let res_pw = taste.search_and_refine_with_context(
            u.id,
            "praise and worship",
            vec![pw_obscure],
            vec![("Praise and Worship Hits".into(), vec![pw1, pw2])],
            SearchMixContext::default(),
        );
        let mix_pw = res_pw.instant_mix.expect("praise and worship should build topic mix");
        assert_eq!(mix_pw.title, "Praise And Worship Mix");
        assert!(mix_pw.is_theme_mix);

        // 2. "western" should NOT hijack onto obscure 1-word "Music3 - Western"
        let w_obscure = mk("w0", "Western", "Music3", "Experimental", "");
        let w1 = mk("w1", "Big Iron", "Marty Robbins", "Country Western", "{\"deezer_rank\":400000}");
        let w2 = mk("w2", "Ghost Riders", "Johnny Cash", "Western", "{\"deezer_rank\":450000}");
        store.upsert_tracks(&[w_obscure.clone(), w1.clone(), w2.clone()]).unwrap();

        assert!(
            taste.plan_entity_mix_anchor(u.id, "western", &[w_obscure.clone()], &[]).is_none(),
            "catalog genre / obscure 1-word title must not hijack topic query"
        );
        let res_w = taste.search_and_refine_with_context(
            u.id,
            "western",
            vec![w_obscure],
            vec![("Country Western Classics".into(), vec![w1, w2])],
            SearchMixContext::default(),
        );
        let mix_w = res_w.instant_mix.expect("western should build topic mix");
        assert_eq!(mix_w.title, "Western Mix");
        assert!(mix_w.is_theme_mix);
    }

    #[test]
    fn instant_mix_builds_named_80s_and_christmas_mixes_without_saving_playlists() {
        let store = Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("mixuser", "pw").unwrap();
        let sources = SourceRegistry::new(
            crate::sources::deezer::DeezerSource::new(),
            crate::sources::ytm::YtmSource::new(None),
            crate::sources::archive::ArchiveSource::new(),
            Arc::new(crate::sources::local::LocalSource::new(store.clone(), vec![])),
            crate::sources::soulseek::SoulseekSource::new(store.clone(), std::env::temp_dir()),
        );
        let taste = TasteEngine::new(store.clone(), sources);

        let mk_track = |id: &str, title: &str, artist: &str, genre: &str, year: i32, rank: u64| Track {
            id: id.into(),
            source: "deezer".into(),
            title: title.into(),
            artist: artist.into(),
            album: format!("{} Album", title),
            genre: genre.into(),
            moods: "upbeat".into(),
            year,
            bitrate: 320,
            meta: format!(r#"{{"rank":{}}}"#, rank),
            ..Default::default()
        };

        let a = mk_track("t80_1", "Take On Me", "a-ha", "Synthpop", 1985, 950000);
        let b = mk_track("t80_2", "Billie Jean", "Michael Jackson", "Pop", 1982, 990000);
        let c = mk_track("t80_3", "Sweet Dreams", "Eurythmics", "Synthpop", 1983, 910000);
        let modern_trap = mk_track("t22", "80s Baby", "Modern Artist", "Pop", 2022, 120000);
        let karaoke = mk_track("t_kar", "Take On Me (Karaoke Version)", "Karaoke Pros", "Pop", 2018, 50000);

        store
            .upsert_tracks(&[a.clone(), b.clone(), c.clone(), modern_trap.clone(), karaoke.clone()])
            .unwrap();

        let res_80s = taste.search_and_refine(
            u.id,
            "80s",
            vec![modern_trap.clone(), karaoke.clone()],
            vec![("80s Hits".to_string(), vec![a.clone(), b.clone(), c.clone()])],
        );
        let mix = res_80s.instant_mix.expect("80s instant mix should be built");
        assert_eq!(mix.title, "80s Hits Mix");
        assert!(mix.is_theme_mix);
        let mix_ids: Vec<&str> = mix.tracks.iter().map(|t| t.id.as_str()).collect();
        assert!(mix_ids.contains(&"t80_1"));
        assert!(mix_ids.contains(&"t80_2"));
        assert!(!mix_ids.contains(&"t22"));
        assert!(!mix_ids.contains(&"t_kar"));

        // Must NOT save any playlist into SQLite!
        assert!(store.list_playlists(u.id, false).unwrap().is_empty());

        // Now test "classic christmas"
        let x1 = mk_track("x1", "White Christmas", "Bing Crosby", "Vocal", 1942, 940000);
        let x2 = mk_track("x2", "Jingle Bell Rock", "Bobby Helms", "Rock", 1957, 920000);
        let x3 = mk_track("x3", "Last Christmas", "Wham!", "Pop", 1984, 980000);
        store.upsert_tracks(&[x1.clone(), x2.clone(), x3.clone()]).unwrap();

        let res_xmas = taste.search_and_refine(
            u.id,
            "classic christmas",
            vec![x1, x2, x3, modern_trap],
            vec![],
        );
        let xmas_mix = res_xmas.instant_mix.expect("christmas mix should be built");
        assert_eq!(xmas_mix.title, "Classic Christmas Mix");
        assert_eq!(xmas_mix.tracks.len(), 3);
        assert!(store.list_playlists(u.id, false).unwrap().is_empty());
    }

    #[test]
    fn instant_mix_never_pads_with_popular_or_starred_unrelated_tracks() {
        let store = Store::open_memory().unwrap();
        let (u, _) = store
            .authenticate_or_create_user("coherence", "pw")
            .unwrap();
        let sources = SourceRegistry::new(
            crate::sources::deezer::DeezerSource::new(),
            crate::sources::ytm::YtmSource::new(None),
            crate::sources::archive::ArchiveSource::new(),
            Arc::new(crate::sources::local::LocalSource::new(store.clone(), vec![])),
            crate::sources::soulseek::SoulseekSource::new(store.clone(), std::env::temp_dir()),
        );
        let taste = TasteEngine::new(store.clone(), sources);

        let mk = |id: &str, title: &str, artist: &str, genre: &str, year: i32, rank: u64| Track {
            id: id.into(),
            source: "deezer".into(),
            title: title.into(),
            artist: artist.into(),
            album: format!("{} Album", title),
            genre: genre.into(),
            year,
            duration: 220,
            bitrate: 320,
            meta: format!(r#"{{"rank":{}}}"#, rank),
            ..Default::default()
        };

        // Three genuinely 80s tracks from a title-matching curated playlist.
        let s1 = mk("s1", "Take On Me", "a-ha", "Synthpop", 1985, 900_000);
        let s2 = mk("s2", "Billie Jean", "Michael Jackson", "Pop", 1982, 950_000);
        let s3 = mk("s3", "Sweet Dreams", "Eurythmics", "Synthpop", 1983, 880_000);
        // Unrelated modern chart hits with no year metadata, one of which the
        // listener has starred and replayed heavily.
        let filler_starred = mk("f1", "Blinding Lights", "The Weeknd", "Pop", 0, 1_200_000);
        let filler_popular = mk("f2", "As It Was", "Harry Styles", "Pop", 0, 1_100_000);

        store
            .upsert_tracks(&[
                s1.clone(),
                s2.clone(),
                s3.clone(),
                filler_starred.clone(),
                filler_popular.clone(),
            ])
            .unwrap();
        store.star_track(u.id, &filler_starred.id).unwrap();
        for _ in 0..6 {
            taste.record_event(u.id, &filler_starred.id, "complete", 1.0);
        }

        let res = taste.search_and_refine_with_context(
            u.id,
            "80s hits",
            vec![filler_starred.clone(), filler_popular.clone()],
            vec![("80s Hits".into(), vec![s1.clone(), s2.clone(), s3.clone()])],
            SearchMixContext::default(),
        );
        let mix = res
            .instant_mix
            .expect("three verified 80s tracks should still build a mix");
        let ids: Vec<&str> = mix.tracks.iter().map(|t| t.id.as_str()).collect();
        assert!(ids.contains(&s1.id.as_str()));
        assert!(ids.contains(&s2.id.as_str()));
        assert!(ids.contains(&s3.id.as_str()));
        assert!(
            !ids.contains(&filler_starred.id.as_str()),
            "starred but unrelated track must not buy membership: {:?}",
            ids
        );
        assert!(
            !ids.contains(&filler_popular.id.as_str()),
            "popular but unrelated track must not buy membership: {:?}",
            ids
        );
        assert_eq!(
            mix.tracks.len(),
            3,
            "an honest small mix beats padded filler: {:?}",
            ids
        );
    }

    #[test]
    fn taste_engine_reranks_search_results_while_pinning_exact_matches() {
        let store = Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("searchlearner", "pw").unwrap();
        let sources = SourceRegistry::new(
            crate::sources::deezer::DeezerSource::new(),
            crate::sources::ytm::YtmSource::new(None),
            crate::sources::archive::ArchiveSource::new(),
            Arc::new(crate::sources::local::LocalSource::new(store.clone(), vec![])),
            crate::sources::soulseek::SoulseekSource::new(store.clone(), std::env::temp_dir()),
        );
        let taste = TasteEngine::new(store.clone(), sources);

        let exact_track = Track {
            id: "exact".into(),
            source: "deezer".into(),
            title: "Midnight".into(),
            artist: "Coldplay".into(),
            genre: "Rock".into(),
            moods: "melancholy".into(),
            year: 2014,
            bitrate: 256,
            ..Default::default()
        };
        let synth_match = Track {
            id: "synth".into(),
            source: "deezer".into(),
            title: "Midnight City".into(),
            artist: "M83".into(),
            genre: "Electronic, Synthpop".into(),
            moods: "late_night,energy".into(),
            year: 2011,
            bitrate: 256,
            meta: r#"{"rank":500000}"#.into(),
            ..Default::default()
        };
        let jazz_match = Track {
            id: "jazz".into(),
            source: "deezer".into(),
            title: "Midnight Blue".into(),
            artist: "Kenny Burrell".into(),
            genre: "Jazz".into(),
            moods: "chill,late_night".into(),
            year: 1963,
            bitrate: 256,
            meta: r#"{"rank":500000}"#.into(),
            ..Default::default()
        };
        store
            .upsert_tracks(&[exact_track.clone(), synth_match.clone(), jazz_match.clone()])
            .unwrap();

        // Learn affinity toward Electronic/Synthpop search picks and away from skipped Jazz
        for _ in 0..8 {
            taste.record_event(u.id, &synth_match.id, "search_pick", 1.0);
            taste.record_event(u.id, &jazz_match.id, "skip", 1.0);
        }

        let ranked = taste.search_and_refine(
            u.id,
            "midnight",
            vec![jazz_match.clone(), exact_track.clone(), synth_match.clone()],
            vec![],
        );
        let ids: Vec<&str> = ranked.tracks.iter().map(|t| t.id.as_str()).collect();
        // Exact title match "Midnight" stays pinned #1, and Synthpop beats skipped Jazz!
        assert_eq!(ids[0], "exact", "ids={:?}", ids);
        let pos_synth = ids.iter().position(|&id| id == "synth").unwrap();
        let pos_jazz = ids.iter().position(|&id| id == "jazz").unwrap();
        assert!(pos_synth < pos_jazz, "ids={:?}", ids);
        assert!(ranked.taste_match_ids.contains("synth"));
        assert!(!ranked.taste_match_ids.contains("jazz"));
    }

    #[test]
    fn not_interested_hides_tracks_across_recommendations_and_reset_restores_them() {
        let store = Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("hideuser", "pw").unwrap();
        let sources = SourceRegistry::new(
            crate::sources::deezer::DeezerSource::new(),
            crate::sources::ytm::YtmSource::new(None),
            crate::sources::archive::ArchiveSource::new(),
            Arc::new(crate::sources::local::LocalSource::new(store.clone(), vec![])),
            crate::sources::soulseek::SoulseekSource::new(store.clone(), std::env::temp_dir()),
        );
        let taste = TasteEngine::new(store.clone(), sources);

        let keep = Track {
            id: "dz:keep".into(),
            source: "deezer".into(),
            title: "Starlight".into(),
            artist: "Muse".into(),
            genre: "Rock".into(),
            moods: "rock_indie".into(),
            meta: r#"{"rank":900000}"#.into(),
            ..Default::default()
        };
        let hide_dz = Track {
            id: "dz:hide".into(),
            source: "deezer".into(),
            title: "Skip This Song".into(),
            artist: "Some Pop".into(),
            genre: "Pop".into(),
            moods: "upbeat".into(),
            meta: r#"{"rank":950000}"#.into(),
            ..Default::default()
        };
        let hide_ytm_dupe = Track {
            id: "ytm:hide".into(),
            source: "ytm".into(),
            title: "Skip This Song (Official Video)".into(),
            artist: "Some Pop".into(),
            genre: "Pop".into(),
            moods: "upbeat".into(),
            ..Default::default()
        };
        store
            .upsert_tracks(&[keep.clone(), hide_dz.clone(), hide_ytm_dupe.clone()])
            .unwrap();
        // Cold users only receive chart-backed tracks; the ytm duplicate must
        // stay out of the starter pool regardless of the hide action.
        store.record_chart_tracks(&[keep.clone(), hide_dz.clone()], 1).unwrap();

        taste.mark_not_interested(u.id, &hide_dz.id);
        assert_eq!(store.hidden_tracks_list(u.id).unwrap().len(), 1);

        let for_you: Vec<String> = taste
            .for_you_tracks(u.id, 10)
            .into_iter()
            .map(|t| t.id)
            .collect();
        assert!(for_you.contains(&keep.id), "for_you={:?}", for_you);
        assert!(!for_you.contains(&hide_dz.id), "for_you={:?}", for_you);
        assert!(!for_you.contains(&hide_ytm_dupe.id), "for_you={:?}", for_you);

        let search = taste.search_and_refine(
            u.id,
            "skip this song",
            vec![hide_dz.clone(), hide_ytm_dupe.clone(), keep.clone()],
            vec![],
        );
        assert!(search.tracks.iter().all(|t| t.id != hide_dz.id && t.id != hide_ytm_dupe.id));

        let cleared = taste.reset_hidden_tracks(u.id);
        assert_eq!(cleared, 1);
        assert!(store.hidden_tracks_list(u.id).unwrap().is_empty());

        let restored: Vec<String> = taste
            .for_you_tracks(u.id, 10)
            .into_iter()
            .map(|t| t.id)
            .collect();
        assert!(restored.contains(&hide_dz.id), "restored={:?}", restored);
    }

    #[test]
    fn untagged_tracks_do_not_synthesize_upbeat_chill_pop_collision() {
        assert!(classify_moods("Kurogane", "Nocturne", "", "").is_empty());
        assert!(classify_moods("Starlight", "Neon", "Electronic", "").contains(&Mood::Party));
        assert!(classify_moods("Summer Hit", "Album", "Pop", "").contains(&Mood::Upbeat));
    }

    #[test]
    fn supermix_and_discover_prefer_related_taste_over_chart_pop_past_20_tracks() {
        let store = Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("ytmlistener", "pw").unwrap();
        let sources = SourceRegistry::new(
            crate::sources::deezer::DeezerSource::new(),
            crate::sources::ytm::YtmSource::new(None),
            crate::sources::archive::ArchiveSource::new(),
            Arc::new(crate::sources::local::LocalSource::new(store.clone(), vec![])),
            crate::sources::soulseek::SoulseekSource::new(store.clone(), std::env::temp_dir()),
        );
        let taste = TasteEngine::new(store.clone(), sources);

        let mut tracks = Vec::new();
        // 10 listened artists x 2 tracks each = 20 max tracks under Supermix's per-artist cap of 2.
        for a in 0..10 {
            for t in 0..2 {
                tracks.push(Track {
                    id: format!("ytm:played:{}:{}", a, t),
                    source: "ytm".into(),
                    title: format!("Played Song {} {}", a, t),
                    artist: format!("Indie Act {}", a),
                    album: format!("Indie Album {}", a),
                    genre: String::new(),
                    moods: String::new(),
                    bitrate: 192,
                    duration: 210,
                    created: 1000 + a * 10 + t,
                    ..Default::default()
                });
            }
        }
        // Duplicate Deezer version of Played Song 0 0 (must not surface in Discover Weekly).
        tracks.push(Track {
            id: "dz:dup0".into(),
            source: "deezer".into(),
            title: "Played Song 0 0".into(),
            artist: "Indie Act 0".into(),
            album: "Indie Album 0".into(),
            genre: String::new(),
            meta: r#"{"rank":450000}"#.into(),
            bitrate: 256,
            duration: 210,
            created: 1500,
            ..Default::default()
        });

        // 22 fresh related-artist discoveries (no Pop rank, empty YTM genre).
        let mut links: Vec<(String, String, f64)> = Vec::new();
        for r in 0..22 {
            let tid = format!("ytm:rel:{}", r);
            let artist_name = format!("Related Scene {}", r);
            tracks.push(Track {
                id: tid.clone(),
                source: "ytm".into(),
                title: format!("Hidden Gem {}", r),
                artist: artist_name,
                album: format!("Gem LP {}", r),
                genre: String::new(),
                moods: String::new(),
                bitrate: 192,
                duration: 225,
                created: 2000 + r,
                ..Default::default()
            });
            links.push(("related_track".into(), tid, 1.5));
            links.push(("related_artist".into(), format!("related-scene-{}", r), 1.4));
        }

        // Flood catalog with 30 newer high-rank Deezer Pop chart hits (rank=980000).
        for p in 0..30 {
            tracks.push(Track {
                id: format!("dz:pop:{}", p),
                source: "deezer".into(),
                title: format!("Global Pop Smash {}", p),
                artist: format!("Pop Chart Star {}", p),
                album: format!("Charts {}", p),
                genre: "Pop".into(),
                moods: "upbeat,chill".into(),
                bitrate: 256,
                duration: 200,
                meta: r#"{"rank":980000}"#.into(),
                created: 9000 + p,
                ..Default::default()
            });
        }

        store.upsert_tracks(&tracks).unwrap();

        // Complete and play all 20 played tracks without negative pop skips.
        for a in 0..10 {
            for t in 0..2 {
                let tid = format!("ytm:played:{}:{}", a, t);
                taste.record_event(u.id, &tid, "complete", 1.0);
                taste.record_event(u.id, &tid, "play", 1.0);
            }
        }
        let link_refs: Vec<(&str, &str, f64)> = links
            .iter()
            .map(|(k, v, w)| (k.as_str(), v.as_str(), *w))
            .collect();
        store.upsert_taste_links(u.id, &link_refs).unwrap();

        // My Supermix (35 tracks): slots 20..35 must NOT fall back to Global Pop chart hits!
        let supermix = taste.for_you_tracks(u.id, 35);
        assert_eq!(supermix.len(), 35);
        let pop_in_supermix: Vec<&str> = supermix
            .iter()
            .filter(|t| t.id.starts_with("dz:pop:"))
            .map(|t| t.id.as_str())
            .collect();
        assert!(
            pop_in_supermix.is_empty(),
            "unexpected chart pop tracks in supermix back half: {:?}",
            pop_in_supermix
        );

        // Discover Weekly (25 tracks): must pick related discoveries, exclude chart pop & dupes.
        let discover = taste.discover_weekly(u.id, 22);
        let discover_ids: Vec<&str> = discover.iter().map(|t| t.id.as_str()).collect();
        assert!(!discover_ids.contains(&"dz:dup0"), "discover={:?}", discover_ids);
        let pop_in_discover: Vec<&str> = discover
            .iter()
            .filter(|t| t.id.starts_with("dz:pop:"))
            .map(|t| t.id.as_str())
            .collect();
        assert!(
            pop_in_discover.is_empty(),
            "unexpected chart pop tracks in discover weekly: {:?}",
            pop_in_discover
        );
    }

    #[test]
    fn classify_moods_avoids_substring_and_searched_title_traps() {
        assert!(classify_moods("Waltz in C", "Default Salt", "", "").is_empty());
        assert!(classify_moods("Crazy Train", "Crystal Crusader", "", "").is_empty());
        assert!(classify_moods("High Score", "Deep Focus", "", "").is_empty());
        assert!(classify_moods("Piano Man", "Minimal Drone", "", "").is_empty());
        assert!(classify_moods("Bohemian Rhapsody (Instrumental)", "Hits", "", "").is_empty());
        assert!(classify_moods("Afterlife Warehouse", "Pop Smoke", "", "").is_empty());

        let chill_lounge = classify_moods("Sunset Lounge", "", "Lounge", "");
        assert!(chill_lounge.contains(&Mood::Chill), "{:?}", chill_lounge);
        let chill_lofi = classify_moods("Midnight Study", "Lofi Hip Hop Beats", "", "");
        assert!(chill_lofi.contains(&Mood::Chill), "{:?}", chill_lofi);
    }

    #[test]
    fn mood_mix_prefers_listener_familiar_compatible_tracks() {
        let store = Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("vibeuser", "pw").unwrap();
        let sources = SourceRegistry::new(
            crate::sources::deezer::DeezerSource::new(),
            crate::sources::ytm::YtmSource::new(None),
            crate::sources::archive::ArchiveSource::new(),
            Arc::new(crate::sources::local::LocalSource::new(store.clone(), vec![])),
            crate::sources::soulseek::SoulseekSource::new(store.clone(), std::env::temp_dir()),
        );
        let taste = TasteEngine::new(store.clone(), sources);

        let familiar = Track {
            id: "dz:familiar".into(),
            source: "deezer".into(),
            title: "Skinny Love".into(),
            artist: "Bon Iver".into(),
            album: "For Emma".into(),
            genre: "Indie, Folk".into(),
            moods: "melancholy".into(),
            duration: 230,
            bitrate: 256,
            stream_url: "dz:familiar".into(),
            ..Default::default()
        };
        let generic = Track {
            id: "dz:generic".into(),
            source: "deezer".into(),
            title: "Quiet Moments".into(),
            artist: "Compilation Artist".into(),
            album: "Mood Collection".into(),
            genre: "Indie".into(),
            moods: "melancholy".into(),
            duration: 220,
            bitrate: 256,
            stream_url: "dz:generic".into(),
            ..Default::default()
        };
        store.upsert_tracks(&[familiar.clone(), generic.clone()]).unwrap();
        let _ = store.record_event(u.id, &familiar.id, "play", 1.0);

        let curated = HashSet::from([generic.id.clone()]);
        let ranked = taste.rank_mood_mix(
            u.id,
            Mood::Melancholy,
            vec![familiar.clone(), generic.clone()],
            &curated,
            10,
        );
        let ids: Vec<&str> = ranked.iter().map(|t| t.id.as_str()).collect();
        assert!(
            ids.contains(&"dz:familiar"),
            "listener-familiar compatible track was excluded: {:?}",
            ids
        );
        assert_eq!(
            ids.first().copied(),
            Some("dz:familiar"),
            "familiar compatible track should outrank generic curated filler: {:?}",
            ids
        );
    }

    #[test]
    fn parse_search_intent_extracts_entity_mix_and_similarity_queries() {
        let m1 = parse_search_intent("9Lana mix");
        assert!(!m1.is_theme_mix);
        assert!(m1.explicit_mix_word);
        assert_eq!(m1.entity_query.as_deref(), Some("9Lana"));

        let m2 = parse_search_intent("songs like dyna mind");
        assert!(!m2.is_theme_mix);
        assert!(m2.explicit_mix_word);
        assert_eq!(m2.entity_query.as_deref(), Some("dyna mind"));

        let m3 = parse_search_intent("DYNA MIND radio");
        assert!(!m3.is_theme_mix);
        assert!(m3.has_trailing_radio_ambiguity);
        assert_eq!(m3.entity_query.as_deref(), Some("DYNA MIND"));

        let on_radio = parse_search_intent("On the Radio");
        assert!(!on_radio.is_theme_mix);
        assert!(on_radio.entity_query.is_none());

        let jpop = parse_search_intent("j-pop");
        assert!(jpop.is_theme_mix);
        assert_eq!(jpop.genres, vec!["J-Pop".to_string()]);

        let rock90 = parse_search_intent("90s rock");
        assert!(rock90.is_theme_mix);
        assert_eq!(rock90.decade_label, Some("90s"));
        assert_eq!(rock90.genres, vec!["Rock".to_string()]);
    }

    #[test]
    fn entity_instant_mix_excludes_keyword_collisions_and_uses_radio_and_related() {
        let store = Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("lana_fan", "pw").unwrap();
        let sources = SourceRegistry::new(
            crate::sources::deezer::DeezerSource::new(),
            crate::sources::ytm::YtmSource::new(None),
            crate::sources::archive::ArchiveSource::new(),
            Arc::new(crate::sources::local::LocalSource::new(store.clone(), vec![])),
            crate::sources::soulseek::SoulseekSource::new(store.clone(), std::env::temp_dir()),
        );
        let taste = TasteEngine::new(store.clone(), sources);

        let mk_dz = |id: &str, title: &str, artist: &str, genre: &str, rank: u64| Track {
            id: id.into(),
            source: "deezer".into(),
            title: title.into(),
            artist: artist.into(),
            album: title.into(),
            genre: genre.into(),
            duration: 205,
            bitrate: 256,
            meta: format!(r#"{{"rank":{}}}"#, rank),
            ..Default::default()
        };
        let mk_ytm = |id: &str, title: &str, artist: &str| Track {
            id: id.into(),
            source: "ytm".into(),
            title: title.into(),
            artist: artist.into(),
            album: title.into(),
            duration: 210,
            bitrate: 192,
            ..Default::default()
        };

        // Train user profile heavily on unrelated Indie Rock so artist/genre taste has positive affinity overall.
        let indie_seed = mk_dz("dz:indie", "Fluorescent Adolescent", "Arctic Monkeys", "Indie", 850000);
        let seed = mk_dz("dz:dyna", "DYNA MIND", "9Lana", "J-Pop", 550000);

        // Keyword-collision garbage returned by raw text search for "dyna mind"
        let bts = mk_dz("dz:bts", "Dynamite", "BTS", "Pop", 990000);
        let taio = mk_dz("dz:taio", "Dynamite", "Taio Cruz", "Pop", 950000);
        let kent = mk_dz("dz:kent", "Don't Mind", "Kent Jones", "Hip-Hop", 920000);
        let sean = mk_dz("dz:sean", "Dynamite", "Sean Paul", "Dance", 880000);
        let gordo = mk_dz("dz:gordo", "Do You Mind", "Gordo", "Electronic", 840000);

        // Real similar tracks from YTM radio, Deezer artist top, Deezer artist radio, and related artists
        let lana_top = mk_dz("dz:battle", "Let me battle", "9Lana", "J-Pop", 620000);
        let lana_turn = mk_dz("dz:turn", "TURN OVER", "9Lana", "J-Pop", 480000);
        let ado = mk_ytm("ytm:ado1", "Show", "Ado");
        let yoshino = mk_ytm("ytm:yoshino1", "Bavarois", "Yoshino");
        let rosu = mk_ytm("ytm:rosu1", "Chikichiki", "Rosu");
        let giga = mk_dz("dz:giga1", "Ready Steady", "Giga", "J-Pop", 520000);
        let syudou = mk_dz("dz:syudou1", "Bitter Choco Decoration", "syudou", "J-Pop", 540000);
        let karaoke_junk = mk_ytm("ytm:kar", "DYNA MIND (Karaoke Version)", "Karaoke Channel");

        store
            .upsert_tracks(&[
                indie_seed.clone(),
                seed.clone(),
                bts.clone(),
                taio.clone(),
                kent.clone(),
                sean.clone(),
                gordo.clone(),
                karaoke_junk.clone(),
            ])
            .unwrap();

        for _ in 0..6 {
            taste.record_event(u.id, &indie_seed.id, "star", 1.0);
            taste.record_event(u.id, &indie_seed.id, "complete", 1.0);
        }

        let raw_search = vec![
            seed.clone(),
            bts.clone(),
            taio.clone(),
            kent.clone(),
            sean.clone(),
            gordo.clone(),
            karaoke_junk,
        ];

        // 1. Offline / empty radio context must NOT hallucinate a mix out of BTS/Taio/Kent Jones.
        let offline_res = taste.search_and_refine(u.id, "dyna mind", raw_search.clone(), vec![]);
        assert!(
            offline_res.instant_mix.is_none(),
            "unexpected garbage mix: {:?}",
            offline_res.instant_mix.map(|m| m.tracks.into_iter().map(|t| t.title).collect::<Vec<_>>())
        );
        // Unrelated Pop chart hits must NOT get stamped FOR YOU from positive bias.
        assert!(!offline_res.taste_match_ids.contains(&bts.id));
        assert!(!offline_res.taste_match_ids.contains(&taio.id));
        assert!(!offline_res.taste_match_ids.contains(&kent.id));

        store
            .upsert_tracks(&[
                lana_top.clone(),
                lana_turn.clone(),
                ado.clone(),
                yoshino.clone(),
                rosu.clone(),
                giga.clone(),
                syudou.clone(),
            ])
            .unwrap();

        // 2. With radio & related-artist context, mix is coherent, starts with DYNA MIND, and excludes keyword junk.
        let dz_artists = vec![ArtistSummary {
            id: "ar:9lana".into(),
            name: "9Lana".into(),
            deezer_id: "12345".into(),
            ..Default::default()
        }];
        let plan = taste
            .plan_entity_mix_anchor(u.id, "dyna mind", &raw_search, &dz_artists)
            .expect("anchor plan should resolve to 9Lana - DYNA MIND");
        assert_eq!(plan.artist_name, "9Lana");
        assert!(!plan.is_artist_query);

        let mix_ctx = SearchMixContext {
            anchor_artist: Some(plan.artist_name),
            anchor_deezer_artist_id: plan.deezer_artist_id,
            anchor_track_key: plan.seed_track.as_ref().map(Track::song_key),
            anchor_genres: vec!["J-Pop".into()],
            is_artist_query: plan.is_artist_query,
            mode_override: None,
            ytm_radio: vec![ado.clone(), yoshino.clone(), rosu.clone()],
            dz_artist_top: vec![lana_top.clone(), lana_turn.clone()],
            dz_artist_radio: vec![giga.clone(), ado.clone()],
            dz_related_tracks: vec![syudou.clone()],
        };
        let res = taste.search_and_refine_with_context(
            u.id,
            "dyna mind",
            raw_search,
            vec![],
            mix_ctx,
        );
        let mix = res.instant_mix.expect("coherent entity mix should be built");
        assert_eq!(mix.title, "9Lana & Similar Mix");
        assert_eq!(mix.tracks[0].id, seed.id);

        let mix_ids: Vec<&str> = mix.tracks.iter().map(|t| t.id.as_str()).collect();
        for bad_id in [bts.id.as_str(), taio.id.as_str(), kent.id.as_str(), sean.id.as_str(), gordo.id.as_str(), "ytm:kar"] {
            assert!(!mix_ids.contains(&bad_id), "mix leaked keyword junk {}: {:?}", bad_id, mix_ids);
        }
        assert!(mix_ids.contains(&ado.id.as_str()), "mix={:?}", mix_ids);
        assert!(mix_ids.contains(&yoshino.id.as_str()), "mix={:?}", mix_ids);
        assert!(mix_ids.contains(&giga.id.as_str()), "mix={:?}", mix_ids);
        assert!(mix_ids.contains(&syudou.id.as_str()), "mix={:?}", mix_ids);
    }

    #[test]
    fn theme_instant_mix_rejects_unrelated_playlists_and_title_word_traps() {
        let store = Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("rocker", "pw").unwrap();
        let sources = SourceRegistry::new(
            crate::sources::deezer::DeezerSource::new(),
            crate::sources::ytm::YtmSource::new(None),
            crate::sources::archive::ArchiveSource::new(),
            Arc::new(crate::sources::local::LocalSource::new(store.clone(), vec![])),
            crate::sources::soulseek::SoulseekSource::new(store.clone(), std::env::temp_dir()),
        );
        let taste = TasteEngine::new(store.clone(), sources);

        let mk = |id: &str, title: &str, artist: &str, genre: &str, year: i32| Track {
            id: id.into(),
            source: "deezer".into(),
            title: title.into(),
            artist: artist.into(),
            album: title.into(),
            genre: genre.into(),
            year,
            duration: 220,
            bitrate: 256,
            meta: r#"{"rank":750000}"#.into(),
            ..Default::default()
        };

        let rockstar_trap = mk("dz:rockstar", "Rockstar", "Post Malone", "Hip-Hop", 2017);
        let party_rock = mk("dz:partyrock", "Party Rock Anthem", "LMFAO", "Pop", 2011);
        let undated_80s = mk("dz:80sbaby", "80s Baby", "Modern Rap", "", 0);
        let slowed_junk = mk("ytm:slowed", "Smells Like Teen Spirit (Slowed + Reverb)", "Uploader", "Rock", 1991);
        let valid_90s_1 = mk("dz:nirvana", "Smells Like Teen Spirit", "Nirvana", "Rock", 1991);
        let valid_90s_2 = mk("dz:pj", "Alive", "Pearl Jam", "Rock", 1991);
        let unrelated_pl_track = mk("dz:trap24", "Club Banger 2024", "Trap Star", "Trap", 2024);

        let res = taste.search_and_refine(
            u.id,
            "90s rock",
            vec![rockstar_trap, party_rock, undated_80s, slowed_junk],
            vec![
                ("Top Mix 2024".to_string(), vec![unrelated_pl_track]),
                ("90s Rock Classics".to_string(), vec![valid_90s_1.clone(), valid_90s_2.clone()]),
            ],
        );
        let mix = res.instant_mix.expect("90s rock mix should exist");
        assert_eq!(mix.title, "90s Rock Mix");
        let ids: Vec<&str> = mix.tracks.iter().map(|t| t.id.as_str()).collect();
        assert!(ids.contains(&valid_90s_1.id.as_str()), "ids={:?}", ids);
        assert!(ids.contains(&valid_90s_2.id.as_str()), "ids={:?}", ids);
        assert!(!ids.contains(&"dz:rockstar"), "ids={:?}", ids);
        assert!(!ids.contains(&"dz:partyrock"), "ids={:?}", ids);
        assert!(!ids.contains(&"dz:80sbaby"), "ids={:?}", ids);
        assert!(!ids.contains(&"ytm:slowed"), "ids={:?}", ids);
        assert!(!ids.contains(&"dz:trap24"), "ids={:?}", ids);
    }

    #[test]
    fn classic_rock_intent_is_a_first_class_genre_with_real_search_queries() {
        let intent = parse_search_intent("classic rock");
        assert!(intent.is_theme_mix);
        assert_eq!(intent.genres, vec!["Classic Rock".to_string()]);
        assert!(intent.wants_classic);
        assert!(intent.expanded_track_queries().contains(&"classic rock hits".to_string()));
        assert!(intent.deezer_genre_hints().contains(&"classic rock"));
        assert!(genre_matches_text("classic rock", "Classic Rock"));
        assert!(!genre_matches_text("j-rock", "Classic Rock"));
    }

    #[test]
    fn scene_genres_are_rejected_for_western_genre_queries() {
        let jrock = Track {
            genre: "J-Rock".into(),
            ..Default::default()
        };
        assert!(has_genre_clash(&jrock, "", &["Classic Rock".into()]));
        assert!(has_genre_clash(&jrock, "", &["Rock".into()]));
        // An explicit J-Rock query must still accept the same track.
        assert!(!has_genre_clash(&jrock, "", &["J-Rock".into()]));
    }

    #[test]
    fn classic_rock_mix_rejects_jrock_and_is_named_from_real_tracks() {
        let store = Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("classic_rocker", "pw").unwrap();
        let sources = SourceRegistry::new(
            crate::sources::deezer::DeezerSource::new(),
            crate::sources::ytm::YtmSource::new(None),
            crate::sources::archive::ArchiveSource::new(),
            Arc::new(crate::sources::local::LocalSource::new(store.clone(), vec![])),
            crate::sources::soulseek::SoulseekSource::new(store.clone(), std::env::temp_dir()),
        );
        let taste = TasteEngine::new(store.clone(), sources);
        let mk = |id: &str, title: &str, artist: &str, genre: &str, year: i32| Track {
            id: id.into(),
            source: "deezer".into(),
            title: title.into(),
            artist: artist.into(),
            album: title.into(),
            genre: genre.into(),
            year,
            duration: 240,
            bitrate: 256,
            meta: r#"{"rank":850000}"#.into(),
            ..Default::default()
        };

        let zep = mk("dz:zep", "Stairway to Heaven", "Led Zeppelin", "classic rock", 1971);
        let floyd = mk("dz:floyd", "Comfortably Numb", "Pink Floyd", "classic rock", 1979);
        let beatles = mk("dz:beatles", "Hey Jude", "The Beatles", "Rock", 1968);
        let jrock_a = mk("dz:mc1", "toriko", "Mr.Children", "J-Rock", 1996);
        let jrock_b = mk("dz:mc2", "tegami", "Mr.Children", "J-Rock", 1996);

        let res = taste.search_and_refine(
            u.id,
            "classic rock",
            vec![zep.clone(), floyd.clone(), beatles.clone(), jrock_a, jrock_b],
            vec![(
                "Classic Rock Essentials".to_string(),
                vec![zep.clone(), floyd.clone(), beatles.clone()],
            )],
        );
        let mix = res.instant_mix.expect("classic rock mix should exist");
        assert_eq!(mix.title, "Classic Rock Mix");
        let ids: Vec<&str> = mix.tracks.iter().map(|t| t.id.as_str()).collect();
        assert!(ids.contains(&"dz:zep"), "ids={:?}", ids);
        assert!(!ids.contains(&"dz:mc1"), "ids={:?}", ids);
        assert!(!ids.contains(&"dz:mc2"), "ids={:?}", ids);
    }

    #[test]
    fn theme_mix_fallback_never_promotes_unrelated_tracks_when_no_evidence_exists() {
        let store = Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("jrock_fallback", "pw").unwrap();
        let sources = SourceRegistry::new(
            crate::sources::deezer::DeezerSource::new(),
            crate::sources::ytm::YtmSource::new(None),
            crate::sources::archive::ArchiveSource::new(),
            Arc::new(crate::sources::local::LocalSource::new(store.clone(), vec![])),
            crate::sources::soulseek::SoulseekSource::new(store.clone(), std::env::temp_dir()),
        );
        let taste = TasteEngine::new(store.clone(), sources);

        // A fuzzy "jrock" search returns rap tracks (Deezer also has rappers
        // literally named "J-Rock"/"Jrock"). Like all curated/search tracks,
        // they have no genre metadata, so has_genre_clash() cannot reject them.
        let mk = |id: &str, title: &str, artist: &str, rank: u64| Track {
            id: id.into(),
            source: "deezer".into(),
            title: title.into(),
            artist: artist.into(),
            album: title.into(),
            genre: String::new(),
            duration: 200,
            bitrate: 256,
            meta: format!(r#"{{"rank":{}}}"#, rank),
            ..Default::default()
        };
        let unrelated: Vec<Track> = vec![
            mk("dz:rap1", "Fuck Love", "XXXTENTACION, Trippie Redd", 950000),
            mk("dz:rap2", "Stick Up", "Rag baby", 940000),
            mk("dz:rap3", "Palm Angels, Palms Itching", "Babyface Ray", 930000),
            mk("dz:rap4", "Still Hustlin'", "FCG Heem", 920000),
            mk("dz:rap5", "Party With The Boys", "Don Toliver", 910000),
            mk("dz:rap6", "Ya Hit", "Future", 900000),
        ];

        // No curated playlists and no radio: the degraded/offline path.
        let res = taste.search_and_refine_with_context(
            u.id,
            "jrock",
            unrelated.clone(),
            vec![],
            SearchMixContext::default(),
        );
        let leaked = res.instant_mix.as_ref().map(|m| {
            m.tracks.iter().map(|t| t.title.clone()).collect::<Vec<_>>()
        });
        assert!(
            res.instant_mix.is_none(),
            "unrelated rap tracks must not be labelled a J-Rock mix: {:?}",
            leaked
        );
    }

    #[test]
    fn jrock_curated_playlist_with_unrelated_genres_is_not_labelled_a_jrock_mix() {
        let store = Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("jrock_curated_repro", "pw").unwrap();
        let sources = SourceRegistry::new(
            crate::sources::deezer::DeezerSource::new(),
            crate::sources::ytm::YtmSource::new(None),
            crate::sources::archive::ArchiveSource::new(),
            Arc::new(crate::sources::local::LocalSource::new(store.clone(), vec![])),
            crate::sources::soulseek::SoulseekSource::new(store.clone(), std::env::temp_dir()),
        );
        let taste = TasteEngine::new(store.clone(), sources);
        // After Deezer album-genre enrichment these real tracks carry genres that clash with J-Rock.
        let mk = |id: &str, title: &str, artist: &str, genre: &str, rank: u64| Track {
            id: id.into(),
            source: "deezer".into(),
            title: title.into(),
            artist: artist.into(),
            album: title.into(),
            genre: genre.into(),
            duration: 210,
            bitrate: 256,
            meta: format!(r#"{{"rank":{}}}"#, rank),
            ..Default::default()
        };
        let tracks = vec![
            mk("dz:1", "Me Uste", "Sky G", "Rap", 950000),
            mk("dz:2", "Touch the Sky", "TOMORROW X TOGETHER", "K-Pop", 940000),
            mk("dz:3", "BANKS HOUSE", "Fatt Smack", "Hip-Hop", 930000),
            mk("dz:4", "Love 4 Me", "Babyface Ray", "Rap", 920000),
            mk("dz:5", "Gracias", "Poeta Callejero", "Reggaeton", 910000),
            mk("dz:6", "Storm is...", "TOMORROW X TOGETHER", "K-Pop", 900000),
            mk("dz:7", "Party With The Boys", "Don Toliver", "Rap", 890000),
        ];
        let res = taste.search_and_refine_with_context(
            u.id,
            "jrock",
            tracks.clone(),
            vec![("Jrock Mix".into(), tracks)],
            SearchMixContext::default(),
        );
        let leaked = res.instant_mix.as_ref().map(|m| {
            m.tracks.iter().map(|t| t.title.clone()).collect::<Vec<_>>()
        });
        assert!(
            res.instant_mix.is_none(),
            "unrelated curated tracks must not be labelled a J-Rock mix: {:?}",
            leaked
        );
    }

    #[test]
    fn jrock_mix_keeps_real_jrock_tracks_tagged_with_broad_rock_genre() {
        let store = Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("jrock_good", "pw").unwrap();
        let sources = SourceRegistry::new(
            crate::sources::deezer::DeezerSource::new(),
            crate::sources::ytm::YtmSource::new(None),
            crate::sources::archive::ArchiveSource::new(),
            Arc::new(crate::sources::local::LocalSource::new(store.clone(), vec![])),
            crate::sources::soulseek::SoulseekSource::new(store.clone(), std::env::temp_dir()),
        );
        let taste = TasteEngine::new(store.clone(), sources);
        let mk = |id: &str, title: &str, artist: &str, rank: u64| Track {
            id: id.into(),
            source: "deezer".into(),
            title: title.into(),
            artist: artist.into(),
            album: title.into(),
            genre: "Rock".into(), // Deezer's coarse album genre for real J-Rock
            duration: 220,
            bitrate: 256,
            meta: format!(r#"{{"rank":{}}}"#, rank),
            ..Default::default()
        };
        let tracks = vec![
            mk("dz:j1", "Gurenge", "LiSA", 950000),
            mk("dz:j2", "The Beginning", "ONE OK ROCK", 940000),
            mk("dz:j3", "Ignite", "Eir Aoi", 930000),
        ];
        let res = taste.search_and_refine_with_context(
            u.id,
            "jrock",
            tracks.clone(),
            vec![("Top J-Rock".into(), tracks)],
            SearchMixContext::default(),
        );
        let mix = res.instant_mix.expect("real J-Rock tracks should build a mix");
        assert_eq!(mix.title, "J-Rock Mix");
        assert!(mix.tracks.iter().all(|t| t.genre == "J-Rock"));
    }

    #[test]
    fn homonym_artists_lisa_kr_and_lisa_jp_never_cross_contaminate_mix_or_affinities() {
        let store = Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("lisa_fan", "pw").unwrap();
        let sources = SourceRegistry::new(
            crate::sources::deezer::DeezerSource::new(),
            crate::sources::ytm::YtmSource::new(None),
            crate::sources::archive::ArchiveSource::new(),
            Arc::new(crate::sources::local::LocalSource::new(store.clone(), vec![])),
            crate::sources::soulseek::SoulseekSource::new(store.clone(), std::env::temp_dir()),
        );
        let taste = TasteEngine::new(store.clone(), sources);

        let kr_money = Track {
            id: "dz:kr_money".into(),
            source: "deezer".into(),
            title: "MONEY".into(),
            artist: "LISA".into(),
            album: "LALISA".into(),
            genre: "K-Pop".into(),
            duration: 168,
            bitrate: 256,
            meta: r#"{"rank":920000,"deezer_artist_id":14559887}"#.into(),
            ..Default::default()
        };
        let kr_lalisa = Track {
            id: "dz:kr_lalisa".into(),
            source: "deezer".into(),
            title: "LALISA".into(),
            artist: "LISA".into(),
            album: "LALISA".into(),
            genre: "K-Pop".into(),
            duration: 200,
            bitrate: 256,
            meta: r#"{"rank":880000,"deezer_artist_id":14559887}"#.into(),
            ..Default::default()
        };
        let jp_gurenge = Track {
            id: "dz:jp_gurenge".into(),
            source: "deezer".into(),
            title: "Gurenge".into(),
            artist: "LiSA".into(),
            album: "LEO-NiNE".into(),
            genre: "J-Rock".into(),
            duration: 238,
            bitrate: 256,
            meta: r#"{"rank":910000,"deezer_artist_id":418196}"#.into(),
            ..Default::default()
        };
        let jp_homura = Track {
            id: "dz:jp_homura".into(),
            source: "deezer".into(),
            title: "Homura".into(),
            artist: "LiSA".into(),
            album: "Homura".into(),
            genre: "J-Rock".into(),
            duration: 275,
            bitrate: 256,
            meta: r#"{"rank":895000,"deezer_artist_id":418196}"#.into(),
            ..Default::default()
        };

        let jp_crossing = Track {
            id: "dz:jp_crossing".into(),
            source: "deezer".into(),
            title: "Crossing Field".into(),
            artist: "LiSA".into(),
            album: "LANDSPACE".into(),
            genre: "J-Rock".into(),
            duration: 249,
            bitrate: 256,
            meta: r#"{"rank":870000,"deezer_artist_id":418196}"#.into(),
            ..Default::default()
        };
        let rel_aimer = Track {
            id: "dz:aimer".into(),
            source: "deezer".into(),
            title: "Zankyousanka".into(),
            artist: "Aimer".into(),
            album: "Walpurgis".into(),
            genre: "J-Rock".into(),
            duration: 182,
            bitrate: 256,
            meta: r#"{"rank":860000,"deezer_artist_id":999001}"#.into(),
            ..Default::default()
        };
        let rel_yoasobi = Track {
            id: "dz:yoasobi".into(),
            source: "deezer".into(),
            title: "Idol".into(),
            artist: "YOASOBI".into(),
            album: "THE BOOK".into(),
            genre: "J-Pop".into(),
            duration: 213,
            bitrate: 256,
            meta: r#"{"rank":930000,"deezer_artist_id":999002}"#.into(),
            ..Default::default()
        };

        store
            .upsert_tracks(&[
                kr_money.clone(),
                kr_lalisa.clone(),
                jp_gurenge.clone(),
                jp_homura.clone(),
                jp_crossing.clone(),
                rel_aimer.clone(),
                rel_yoasobi.clone(),
            ])
            .unwrap();

        // Starring Korean LISA's track must not give artist affinity to Japanese LiSA.
        store.star_track(u.id, &kr_money.id).unwrap();
        let prof = taste.user_profile(u.id);
        let aff_kr = build_track_affinities(&kr_lalisa, &prof);
        let aff_jp = build_track_affinities(&jp_gurenge, &prof);
        assert!(aff_kr.artist_affinity > 0.0, "kr affinity should be positive");
        assert_eq!(aff_jp.artist_affinity, 0.0, "jp affinity should remain 0.0");

        let dz_artists = vec![
            ArtistSummary {
                id: "ar:dz:14559887".into(),
                name: "LISA".into(),
                deezer_id: "14559887".into(),
                ..Default::default()
            },
            ArtistSummary {
                id: "ar:dz:418196".into(),
                name: "LiSA".into(),
                deezer_id: "418196".into(),
                ..Default::default()
            },
        ];

        // Entity anchor for Japanese "LiSA" selects deezer_artist_id 418196 and her seed track.
        let raw_tracks = vec![
            kr_money.clone(),
            kr_lalisa.clone(),
            jp_gurenge.clone(),
            jp_homura.clone(),
        ];
        let jp_plan = taste
            .plan_entity_mix_anchor(u.id, "LiSA", &raw_tracks, &dz_artists)
            .expect("LiSA anchor plan should resolve");
        assert_eq!(jp_plan.deezer_artist_id.as_deref(), Some("418196"));
        assert_eq!(
            jp_plan.seed_track.as_ref().and_then(Track::deezer_artist_id).as_deref(),
            Some("418196")
        );

        let mix_ctx = SearchMixContext {
            anchor_artist: Some(jp_plan.artist_name),
            anchor_deezer_artist_id: jp_plan.deezer_artist_id,
            anchor_track_key: jp_plan.seed_track.as_ref().map(Track::song_key),
            anchor_genres: vec!["J-Rock".into()],
            is_artist_query: true,
            mode_override: None,
            ytm_radio: vec![rel_aimer.clone(), rel_yoasobi.clone()],
            dz_artist_top: vec![jp_gurenge.clone(), jp_homura.clone(), jp_crossing.clone()],
            dz_artist_radio: vec![rel_aimer.clone()],
            dz_related_tracks: vec![rel_yoasobi.clone()],
        };
        let res = taste.search_and_refine_with_context(u.id, "LiSA", raw_tracks, vec![], mix_ctx);
        let mix = res.instant_mix.expect("LiSA mix should be created");
        let mix_ids: Vec<&str> = mix.tracks.iter().map(|t| t.id.as_str()).collect();
        assert!(mix_ids.contains(&jp_gurenge.id.as_str()), "mix_ids={:?}", mix_ids);
        assert!(mix_ids.contains(&jp_homura.id.as_str()), "mix_ids={:?}", mix_ids);
        assert!(
            !mix_ids.contains(&kr_money.id.as_str()),
            "Korean LISA track leaked into Japanese LiSA mix: {:?}",
            mix_ids
        );
        assert!(
            !mix_ids.contains(&kr_lalisa.id.as_str()),
            "Korean LISA track leaked into Japanese LiSA mix: {:?}",
            mix_ids
        );
    }

    #[tokio::test]
    async fn recently_heard_songs_get_temporary_negative_weight_in_upcoming_radio() {
        let store = Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("radiouser", "pw").unwrap();
        let sources = SourceRegistry::new(
            crate::sources::deezer::DeezerSource::new(),
            crate::sources::ytm::YtmSource::new(None),
            crate::sources::archive::ArchiveSource::new(),
            Arc::new(crate::sources::local::LocalSource::new(store.clone(), vec![])),
            crate::sources::soulseek::SoulseekSource::new(store.clone(), std::env::temp_dir()),
        );
        let taste = TasteEngine::new(store.clone(), sources);

        let heard_loc = Track {
            id: "loc:heard".into(),
            source: "local".into(),
            title: "Midnight City".into(),
            artist: "M83".into(),
            album: "Hurry Up, We're Dreaming".into(),
            genre: "Synthpop".into(),
            moods: "night,upbeat".into(),
            bitrate: 900,
            duration: 243,
            meta: r#"{"rank":950000}"#.into(),
            ..Default::default()
        };
        let heard_dup_ytm = Track {
            id: "ytm:heard-dup".into(),
            source: "ytm".into(),
            title: "Midnight City (Official Video)".into(),
            artist: "M83".into(),
            album: "Hurry Up, We're Dreaming".into(),
            genre: "Synthpop".into(),
            moods: "night,upbeat".into(),
            bitrate: 192,
            duration: 243,
            ..Default::default()
        };
        let fresh_1 = Track {
            id: "loc:fresh1".into(),
            source: "local".into(),
            title: "Wait".into(),
            artist: "M83".into(),
            album: "Hurry Up, We're Dreaming".into(),
            genre: "Synthpop".into(),
            moods: "night,chill".into(),
            bitrate: 900,
            duration: 343,
            ..Default::default()
        };
        let fresh_2 = Track {
            id: "loc:fresh2".into(),
            source: "local".into(),
            title: "Running Out Of Time".into(),
            artist: "Paramore".into(),
            album: "This Is Why".into(),
            genre: "Synthpop".into(),
            moods: "upbeat".into(),
            bitrate: 900,
            duration: 192,
            ..Default::default()
        };
        store
            .upsert_tracks(&[
                heard_loc.clone(),
                heard_dup_ytm.clone(),
                fresh_1.clone(),
                fresh_2.clone(),
            ])
            .unwrap();

        // User just clicked/heard `loc:heard`
        taste.record_event(u.id, &heard_loc.id, "play", 1.0);

        let sctx = taste.build_scoring_context(u.id);
        let penalties = taste.build_recent_heard_penalties(
            u.id,
            &sctx,
            &[("loc:heard".to_string(), 15)],
        );
        // Just-heard track and its cross-source song_key sibling both get strong temporary penalty
        assert!(penalties.penalty_for(&heard_loc) > 10.0);
        assert!(penalties.penalty_for(&heard_dup_ytm) > 10.0);
        assert_eq!(penalties.penalty_for(&fresh_1), 0.0);

        // Penalty expires cleanly after the temporary window (> 2 hours)
        let expired_penalties = RecentHeardPenalties {
            now: Utc::now().timestamp() + RECENT_HEARD_WINDOW_SECS + 60,
            by_id: penalties.by_id.clone(),
            by_key: penalties.by_key.clone(),
        };
        assert_eq!(expired_penalties.penalty_for(&heard_loc), 0.0);
        assert_eq!(expired_penalties.penalty_for(&heard_dup_ytm), 0.0);

        // Fallback radio upcoming list places fresh tracks ahead of recently heard songs
        let upcoming = taste
            .radio_next_tracks_with_exclusions_and_recent(
                u.id,
                None,
                &HashSet::new(),
                &[("loc:heard".to_string(), 10)],
                2,
            )
            .await;
        let upcoming_ids: Vec<&str> = upcoming.iter().map(|t| t.id.as_str()).collect();
        assert!(
            !upcoming_ids.contains(&"loc:heard") && !upcoming_ids.contains(&"ytm:heard-dup"),
            "recently heard song should be deprioritized below fresh tracks in upcoming: {:?}",
            upcoming_ids
        );
    }

    #[test]
    fn theme_instant_mix_recovers_with_ytm_fallback_when_curated_playlists_unavailable() {
        let store = Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("fallback80s", "pw").unwrap();
        let sources = SourceRegistry::new(
            crate::sources::deezer::DeezerSource::new(),
            crate::sources::ytm::YtmSource::new(None),
            crate::sources::archive::ArchiveSource::new(),
            Arc::new(crate::sources::local::LocalSource::new(store.clone(), vec![])),
            crate::sources::soulseek::SoulseekSource::new(store.clone(), std::env::temp_dir()),
        );
        let taste = TasteEngine::new(store.clone(), sources);

        let ytm_1 = Track {
            id: "ytm:takeonme".into(),
            source: "ytm".into(),
            title: "Take On Me".into(),
            artist: "a-ha".into(),
            duration: 225,
            ..Default::default()
        };
        let ytm_2 = Track {
            id: "ytm:ebtrtw".into(),
            source: "ytm".into(),
            title: "Everybody Wants To Rule The World".into(),
            artist: "Tears For Fears".into(),
            duration: 251,
            ..Default::default()
        };
        let ytm_radio_1 = Track {
            id: "ytm:sweetdreams".into(),
            source: "ytm".into(),
            title: "Sweet Dreams (Are Made of This)".into(),
            artist: "Eurythmics".into(),
            duration: 216,
            ..Default::default()
        };
        let spam_cover = Track {
            id: "dz:spam".into(),
            source: "deezer".into(),
            title: "Nothing's Gonna Stop Us Now".into(),
            artist: "80s Hits Guys".into(),
            duration: 240,
            ..Default::default()
        };
        let modern_wrong_era = Track {
            id: "dz:2022".into(),
            source: "deezer".into(),
            title: "80s Vibes".into(),
            artist: "Modern Artist".into(),
            year: 2022,
            duration: 200,
            ..Default::default()
        };

        let mix_ctx = SearchMixContext {
            ytm_radio: vec![ytm_radio_1.clone()],
            ..Default::default()
        };
        let res = taste.search_and_refine_with_context(
            u.id,
            "80s hits",
            vec![ytm_1.clone(), ytm_2.clone(), spam_cover.clone(), modern_wrong_era.clone()],
            vec![],
            mix_ctx,
        );
        let mix = res.instant_mix.expect("80s hits mix should recover when curated playlists are empty");
        assert_eq!(mix.title, "80s Hits Mix");
        let ids: Vec<&str> = mix.tracks.iter().map(|t| t.id.as_str()).collect();
        assert!(ids.contains(&ytm_radio_1.id.as_str()), "ids={:?}", ids);
        assert!(ids.contains(&ytm_1.id.as_str()), "ids={:?}", ids);
        assert!(ids.contains(&ytm_2.id.as_str()), "ids={:?}", ids);
        assert!(!ids.contains(&spam_cover.id.as_str()), "ids={:?}", ids);
        assert!(!ids.contains(&modern_wrong_era.id.as_str()), "ids={:?}", ids);
    }

    #[test]
    fn entity_instant_mix_builds_single_artist_essential_mix_when_radio_is_empty() {
        let store = Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("daftfan", "pw").unwrap();
        let sources = SourceRegistry::new(
            crate::sources::deezer::DeezerSource::new(),
            crate::sources::ytm::YtmSource::new(None),
            crate::sources::archive::ArchiveSource::new(),
            Arc::new(crate::sources::local::LocalSource::new(store.clone(), vec![])),
            crate::sources::soulseek::SoulseekSource::new(store.clone(), std::env::temp_dir()),
        );
        let taste = TasteEngine::new(store.clone(), sources);

        let mk = |id: &str, title: &str| Track {
            id: id.into(),
            source: "deezer".into(),
            title: title.into(),
            artist: "Daft Punk".into(),
            album: "Discovery".into(),
            genre: "Electronic".into(),
            duration: 240,
            bitrate: 256,
            meta: r#"{"deezer_artist_id":27,"rank":900000}"#.into(),
            ..Default::default()
        };
        let raw = vec![
            mk("dz:1", "One More Time"),
            mk("dz:2", "Harder, Better, Faster, Stronger"),
            mk("dz:3", "Around the World"),
            mk("dz:4", "Digital Love"),
            mk("dz:5", "Get Lucky"),
            mk("dz:6", "Instant Crush"),
        ];
        let dz_artists = vec![ArtistSummary {
            id: "ar:dz:27".into(),
            name: "Daft Punk".into(),
            deezer_id: "27".into(),
            ..Default::default()
        }];
        let plan = taste
            .plan_entity_mix_anchor(u.id, "Daft Punk", &raw, &dz_artists)
            .expect("anchor plan should match Daft Punk");
        let mix_ctx = SearchMixContext {
            anchor_artist: Some(plan.artist_name),
            anchor_deezer_artist_id: plan.deezer_artist_id,
            anchor_track_key: plan.seed_track.as_ref().map(Track::song_key),
            is_artist_query: plan.is_artist_query,
            ..Default::default()
        };
        let res = taste.search_and_refine_with_context(u.id, "Daft Punk", raw, vec![], mix_ctx);
        let mix = res
            .instant_mix
            .expect("single-artist mix should succeed when >= 5 anchor tracks exist");
        assert_eq!(mix.title, "Daft Punk Mix");
        assert!(mix.tracks.len() >= 5, "tracks={}", mix.tracks.len());
    }

    #[test]
    fn melancholy_rejects_title_word_traps_and_prefers_user_music() {
        assert!(!classify_moods("Melancholy Kitchen", "Yankee", "J-Pop", "").contains(&Mood::Melancholy));
        assert!(!classify_moods("Mining Melancholy", "Arrangement Archives", "", "").contains(&Mood::Melancholy));
        assert!(!classify_moods("November Melancholy", "November Melancholy", "", "").contains(&Mood::Melancholy));
        assert!(!classify_moods("Melancholy Man", "Here Comes Everybody", "", "").contains(&Mood::Melancholy));

        let store = Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("jpop_mel", "pw").unwrap();
        let sources = SourceRegistry::new(
            crate::sources::deezer::DeezerSource::new(),
            crate::sources::ytm::YtmSource::new(None),
            crate::sources::archive::ArchiveSource::new(),
            Arc::new(crate::sources::local::LocalSource::new(store.clone(), vec![])),
            crate::sources::soulseek::SoulseekSource::new(store.clone(), std::env::temp_dir()),
        );
        let taste = TasteEngine::new(store.clone(), sources);

        let mk = |id: &str, title: &str, artist: &str, genre: &str, moods: &str| Track {
            id: id.into(),
            source: "deezer".into(),
            title: title.into(),
            artist: artist.into(),
            album: "Album".into(),
            genre: genre.into(),
            moods: moods.into(),
            duration: 220,
            bitrate: 256,
            stream_url: id.into(),
            ..Default::default()
        };

        let trap_dk = mk("dz:dk", "Mining Melancholy", "Joshua Morse", "", "melancholy");
        let trap_man = mk("dz:man", "Melancholy Man", "The Wake", "", "melancholy");
        let trap_box = mk("dz:box", "Bright Box of Melancholy", "Laura Cooper", "", "melancholy");
        let user_fav = mk("dz:fav", "Lemon", "Kenshi Yonezu", "J-Pop", "");
        let user_ballad = mk("dz:bal", "First Love", "Utada Hikaru", "J-Pop, Ballad", "");
        let generic_curated = mk("dz:gen", "Quiet Rain", "Random Indie", "Indie", "melancholy");

        store
            .upsert_tracks(&[
                trap_dk.clone(),
                trap_man.clone(),
                trap_box.clone(),
                user_fav.clone(),
                user_ballad.clone(),
                generic_curated.clone(),
            ])
            .unwrap();
        let _ = store.record_event(u.id, &user_fav.id, "play", 1.0);
        let _ = store.record_event(u.id, &user_fav.id, "like", 1.0);

        let curated_ids = HashSet::from([trap_dk.id.clone(), trap_man.id.clone(), generic_curated.id.clone()]);
        let ranked = taste.rank_mood_mix(
            u.id,
            Mood::Melancholy,
            vec![trap_dk, trap_man, trap_box, user_fav, user_ballad, generic_curated],
            &curated_ids,
            20,
        );
        let ids: Vec<&str> = ranked.iter().map(|t| t.id.as_str()).collect();
        assert!(!ids.contains(&"dz:dk"), "title trap leaked: {:?}", ids);
        assert!(!ids.contains(&"dz:man"), "title trap leaked: {:?}", ids);
        assert!(!ids.contains(&"dz:box"), "title trap leaked: {:?}", ids);
        // A favourite with no melancholy evidence must not leak in just because the
        // listener loves it — otherwise every mood collapses into the same playlist.
        assert!(!ids.contains(&"dz:fav"), "off-mood favourite leaked into mood mix: {:?}", ids);
        assert!(ids.contains(&"dz:bal"), "user's genre-aligned ballad missing: {:?}", ids);
        assert!(ids.contains(&"dz:gen"), "curated melancholy track missing: {:?}", ids);
    }

    #[test]
    fn mood_mixes_differ_for_a_listener_with_a_mixed_library() {
        let store = Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("mixedlib", "pw").unwrap();
        let sources = SourceRegistry::new(
            crate::sources::deezer::DeezerSource::new(),
            crate::sources::ytm::YtmSource::new(None),
            crate::sources::archive::ArchiveSource::new(),
            Arc::new(crate::sources::local::LocalSource::new(store.clone(), vec![])),
            crate::sources::soulseek::SoulseekSource::new(store.clone(), std::env::temp_dir()),
        );
        let taste = TasteEngine::new(store.clone(), sources);

        let mut lib = Vec::new();
        for (prefix, genre) in [
            ("rock", "Indie Rock"),
            ("soul", "Soul & Funk"),
            ("folk", "Folk"),
            ("edm", "Deep House"),
            ("lofi", "Lo-Fi, Chillhop"),
        ] {
            for i in 0..8 {
                lib.push(Track {
                    id: format!("dz:{prefix}{i}"),
                    source: "deezer".into(),
                    title: format!("{prefix} song {i}"),
                    artist: format!("{prefix} artist {i}"),
                    album: "Album".into(),
                    genre: genre.into(),
                    duration: 210,
                    bitrate: 320,
                    stream_url: format!("dz:{prefix}{i}"),
                    ..Default::default()
                });
            }
        }
        store.upsert_tracks(&lib).unwrap();
        for t in &lib {
            let _ = store.record_event(u.id, &t.id, "play", 1.0);
            let _ = store.record_event(u.id, &t.id, "like", 1.0);
        }

        let none = HashSet::new();
        let run = |m: Mood| -> HashSet<String> {
            taste
                .rank_mood_mix(u.id, m, lib.clone(), &none, 35)
                .into_iter()
                .map(|t| t.id)
                .collect()
        };
        let rock = run(Mood::RockIndie);
        let groove = run(Mood::Groove);
        let acoustic = run(Mood::Acoustic);
        let party = run(Mood::Party);
        let chill = run(Mood::Chill);

        assert!(rock.iter().all(|id| id.starts_with("dz:rock")), "rock: {:?}", rock);
        assert!(groove.iter().all(|id| id.starts_with("dz:soul")), "groove: {:?}", groove);
        assert!(acoustic.iter().all(|id| id.starts_with("dz:folk")), "acoustic: {:?}", acoustic);
        assert!(party.iter().all(|id| id.starts_with("dz:edm")), "party: {:?}", party);
        assert!(chill.iter().all(|id| id.starts_with("dz:lofi")), "chill: {:?}", chill);
        for set in [&rock, &groove, &acoustic, &party, &chill] {
            assert!(set.len() >= 6, "mood mix too sparse: {:?}", set);
        }
    }

    #[test]
    fn mood_query_instant_mix_excludes_untagged_and_off_mood_filler() {
        let store = Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("chillq", "pw").unwrap();
        let sources = SourceRegistry::new(
            crate::sources::deezer::DeezerSource::new(),
            crate::sources::ytm::YtmSource::new(None),
            crate::sources::archive::ArchiveSource::new(),
            Arc::new(crate::sources::local::LocalSource::new(store.clone(), vec![])),
            crate::sources::soulseek::SoulseekSource::new(store.clone(), std::env::temp_dir()),
        );
        let taste = TasteEngine::new(store.clone(), sources);

        let mk = |id: &str, artist: &str, genre: &str| Track {
            id: id.into(),
            source: "deezer".into(),
            title: format!("Song {id}"),
            artist: artist.into(),
            album: "Album".into(),
            genre: genre.into(),
            duration: 200,
            bitrate: 320,
            stream_url: id.into(),
            ..Default::default()
        };
        let mut raw = Vec::new();
        for i in 0..6 {
            raw.push(mk(&format!("dz:lofi{i}"), &format!("Lofi Artist {i}"), "Lo-Fi, Chillhop"));
        }
        for i in 0..6 {
            raw.push(mk(&format!("dz:untagged{i}"), &format!("Random {i}"), ""));
        }
        raw.push(mk("dz:pop", "Pop Star", "Pop"));

        let res = taste.search_and_refine_with_context(
            u.id,
            "chill",
            raw,
            vec![],
            SearchMixContext {
                mode_override: Some("topic".into()),
                ..Default::default()
            },
        );
        let mix = res.instant_mix.expect("chill mix should build from verified lofi tracks");
        let ids: Vec<&str> = mix.tracks.iter().map(|t| t.id.as_str()).collect();
        assert!(ids.iter().all(|id| id.starts_with("dz:lofi")), "off-topic filler leaked: {:?}", ids);
        assert!(ids.len() >= 5, "ids={:?}", ids);
    }

    #[test]
    fn rank_mood_mix_rejects_lone_unverified_stored_track_and_clashes() {
        let store = Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("meluser", "pw").unwrap();
        let sources = SourceRegistry::new(
            crate::sources::deezer::DeezerSource::new(),
            crate::sources::ytm::YtmSource::new(None),
            crate::sources::archive::ArchiveSource::new(),
            Arc::new(crate::sources::local::LocalSource::new(store.clone(), vec![])),
            crate::sources::soulseek::SoulseekSource::new(store.clone(), std::env::temp_dir()),
        );
        let taste = TasteEngine::new(store.clone(), sources);

        let lone_bogus = Track {
            id: "ytm:bogus".into(),
            source: "ytm".into(),
            title: "Random Upbeat Dance Track".into(),
            artist: "Random DJ".into(),
            genre: String::new(),
            moods: "melancholy".into(),
            duration: 200,
            ..Default::default()
        };
        let verified_sad = Track {
            id: "dz:sad1".into(),
            source: "deezer".into(),
            title: "Skinny Love".into(),
            artist: "Bon Iver".into(),
            genre: "Indie, Folk".into(),
            moods: "melancholy".into(),
            duration: 238,
            ..Default::default()
        };
        store.upsert_tracks(&[lone_bogus.clone(), verified_sad.clone()]).unwrap();

        let curated_only_verified = HashSet::from([verified_sad.id.clone()]);
        let ranked = taste.rank_mood_mix(
            u.id,
            Mood::Melancholy,
            vec![lone_bogus, verified_sad],
            &curated_only_verified,
            20,
        );
        let ids: Vec<&str> = ranked.iter().map(|t| t.id.as_str()).collect();
        assert!(!ids.contains(&"ytm:bogus"), "unverified lone track leaked: {:?}", ids);
        assert!(ids.contains(&"dz:sad1"), "curated track missing: {:?}", ids);

        assert!(!is_curated_playlist_for_mood("Sad Covers", Mood::Melancholy));
        assert!(!is_curated_playlist_for_mood("Party Workout Mix", Mood::Melancholy));
        assert!(is_curated_playlist_for_mood("Melancholy Indie Folk", Mood::Melancholy));
    }

    #[test]
    fn party_mood_rejects_ai_slop_regional_playlists_and_tail_divergence() {
        // Regional & retro playlists must not hijack generic "dance party" / "club hits" queries.
        assert!(crate::sources::has_unrequested_scene_qualifier(
            "Malayalam Dance Party",
            "dance party"
        ));
        assert!(crate::sources::has_unrequested_scene_qualifier(
            "Marathi Party Hits",
            "club hits"
        ));
        assert!(crate::sources::deezer::score_mix_playlist(
            "50s Rock & Roll Dance Party",
            40,
            "dance party"
        )
        .is_none());
        assert!(crate::sources::deezer::score_mix_playlist(
            "Dance Party Hits",
            40,
            "dance party"
        )
        .is_some());

        // User-mood queries must not combine Rock or Country with "dance".
        assert!(!is_genre_fit_for_mood_query("rock", Mood::Party));
        assert!(!is_genre_fit_for_mood_query("country", Mood::Party));
        assert!(!is_genre_fit_for_mood_query("films/games", Mood::Party));
        assert!(is_genre_fit_for_mood_query("pop", Mood::Party));
        assert!(is_genre_fit_for_mood_query("house", Mood::Party));

        // AI-slop spam artists and emoji-stuffed titles are rejected as mood clutter.
        let mk_slop = |id: &str, title: &str, artist: &str, genre: &str, rank: u64| Track {
            id: id.into(),
            source: "deezer".into(),
            title: title.into(),
            artist: artist.into(),
            album: title.into(),
            genre: genre.into(),
            moods: "party".into(),
            duration: 190,
            bitrate: 256,
            meta: format!(r#"{{"rank":{}}}"#, rank),
            ..Default::default()
        };
        assert!(is_mood_clutter_track(&mk_slop(
            "dz:slop1",
            "Hot Rod Hustle",
            "Rock n Roll Original Music",
            "",
            5000
        )));
        assert!(is_mood_clutter_track(&mk_slop(
            "dz:slop2",
            "Quit acting shy",
            "Voltage vault rock 'n' roll music",
            "",
            4000
        )));
        assert!(is_mood_clutter_track(&mk_slop(
            "dz:slop3",
            "Twist the Night - Gaetano La Rocca - 50s Rock Twist 💃🌙",
            "Gaetano La Rocca",
            "",
            3000
        )));

        // Country & Soundtrack tracks are incompatible with Party & Dance.
        let country = mk_slop("dz:cntry", "Raised on dust", "Old Tennessee Country", "Country", 5000);
        let ost = mk_slop("dz:ost", "Armadham", "Sushin Shyam", "Films/Games", 500_000);
        assert!(!is_track_compatible_with_mood(&country, "", Mood::Party));
        assert!(!is_track_compatible_with_mood(&ost, "", Mood::Party));

        // Tail-divergence guard: when enough real Party tracks exist, weak/obscure
        // curated filler is dropped instead of padding the bottom of the playlist.
        let store = Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("partyuser", "pw").unwrap();
        let sources = SourceRegistry::new(
            crate::sources::deezer::DeezerSource::new(),
            crate::sources::ytm::YtmSource::new(None),
            crate::sources::archive::ArchiveSource::new(),
            Arc::new(crate::sources::local::LocalSource::new(store.clone(), vec![])),
            crate::sources::soulseek::SoulseekSource::new(store.clone(), std::env::temp_dir()),
        );
        let taste = TasteEngine::new(store.clone(), sources);

        let mut cands = Vec::new();
        let mut curated = HashSet::new();
        for i in 0..20 {
            let t = Track {
                id: format!("dz:club{i}"),
                source: "deezer".into(),
                title: format!("Club Anthem {i}"),
                artist: format!("DJ Real {i}"),
                album: "Club Album".into(),
                genre: "Dance, House".into(),
                moods: "party".into(),
                duration: 210,
                bitrate: 256,
                meta: r#"{"rank":650000}"#.into(),
                ..Default::default()
            };
            curated.insert(t.id.clone());
            cands.push(t);
        }
        let obscure_filler = Track {
            id: "dz:obscure".into(),
            source: "deezer".into(),
            title: "Random Filler".into(),
            artist: "Unknown Indie Guy".into(),
            album: "Filler".into(),
            genre: String::new(),
            moods: "party".into(),
            duration: 180,
            bitrate: 256,
            meta: r#"{"rank":12000}"#.into(),
            ..Default::default()
        };
        let stale_wrong_genre = Track {
            id: "dz:stale_rock".into(),
            source: "deezer".into(),
            title: "Old Rock Song".into(),
            artist: "Classic Rocker".into(),
            album: "Rock Album".into(),
            genre: "Rock".into(),
            moods: "party".into(),
            duration: 200,
            bitrate: 256,
            meta: r#"{"rank":400000}"#.into(),
            ..Default::default()
        };
        curated.insert(obscure_filler.id.clone());
        cands.push(obscure_filler);
        cands.push(stale_wrong_genre);
        cands.push(country);
        cands.push(ost);
        store.upsert_tracks(&cands).unwrap();

        let ranked = taste.rank_mood_mix(u.id, Mood::Party, cands, &curated, 35);
        let ids: Vec<&str> = ranked.iter().map(|t| t.id.as_str()).collect();
        assert!(!ids.contains(&"dz:obscure"), "obscure filler leaked into tail: {:?}", ids);
        assert!(!ids.contains(&"dz:stale_rock"), "stale off-genre track leaked: {:?}", ids);
        assert!(!ids.contains(&"dz:cntry"), "country track leaked: {:?}", ids);
        assert!(!ids.contains(&"dz:ost"), "soundtrack leaked: {:?}", ids);
        assert_eq!(ids.len(), 20, "expected only the 20 verified dance tracks: {:?}", ids);
    }

    #[test]
    fn mood_mix_excludes_unplayed_archive_leftovers_mashups_and_western_metal_for_jrock_listener() {
        let store = Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("jrock_fan", "pw").unwrap();
        let sources = SourceRegistry::new(
            crate::sources::deezer::DeezerSource::new(),
            crate::sources::ytm::YtmSource::new(None),
            crate::sources::archive::ArchiveSource::new(),
            Arc::new(crate::sources::local::LocalSource::new(store.clone(), vec![])),
            crate::sources::soulseek::SoulseekSource::new(store.clone(), std::env::temp_dir()),
        );
        let taste = TasteEngine::new(store.clone(), sources);

        // Listener has a rich J-Rock world (Kessoku Band, Eve, etc.) tagged "Rock, J-Rock".
        let mut cands = Vec::new();
        for i in 0..22 {
            let t = Track {
                id: format!("dz:jrock{i}"),
                source: "deezer".into(),
                title: format!("JRock Song {i}"),
                artist: if i < 6 {
                    "Kessoku Band".into()
                } else {
                    format!("JRock Act {i}")
                },
                album: "Kessoku Album".into(),
                genre: "Rock, J-Rock".into(),
                moods: "energy,rock_indie".into(),
                duration: 220,
                bitrate: 256,
                meta: r#"{"rank":500000}"#.into(),
                ..Default::default()
            };
            cands.push(t);
        }

        // Unplayed Archive.org / Soulseek leftovers from past searches ("shinkai", "kessoku band").
        let mr_children_shinkai = Track {
            id: "arc:shinkai1".into(),
            source: "archive".into(),
            title: "tegami".into(),
            artist: "Mr.Children".into(),
            album: "shinkai".into(),
            genre: "Rock, J-Rock".into(),
            moods: "energy,rock_indie".into(),
            duration: 169,
            bitrate: 320,
            format: "mp3".into(),
            ..Default::default()
        };
        let kessoku_mashup = Track {
            id: "slsk:mashup1".into(),
            source: "soulseek".into(),
            title: "Sekretna Chłosta".into(),
            artist: "Gimpson/MC Sobieski/Kessoku Band".into(),
            album: "kessoku band X gimpson".into(),
            genre: "Rock".into(),
            moods: "energy,rock_indie".into(),
            duration: 233,
            bitrate: 320,
            format: "mp3".into(),
            ..Default::default()
        };
        // Even if a mashup came from Deezer, is_mood_clutter_track must reject it.
        let mut dz_mashup = kessoku_mashup.clone();
        dz_mashup.id = "dz:mashup1".into();
        dz_mashup.source = "deezer".into();
        assert!(is_mood_clutter_track(&dz_mashup));

        // Global Western Metal chart tracks.
        let metallica_one = Track {
            id: "dz:met1".into(),
            source: "deezer".into(),
            title: "One".into(),
            artist: "Metallica".into(),
            album: "...And Justice For All".into(),
            genre: "Metal, Rock".into(),
            moods: "energy,rock_indie".into(),
            duration: 448,
            bitrate: 256,
            meta: r#"{"rank":920000}"#.into(),
            ..Default::default()
        };
        let metallica_live = Track {
            id: "dz:met2".into(),
            source: "deezer".into(),
            title: "Enter Sandman (Live with the SFSO)".into(),
            artist: "Metallica".into(),
            album: "Enter Sandman (Live with the SFSO)".into(),
            genre: "Metal, Rock".into(),
            moods: "energy,rock_indie".into(),
            duration: 331,
            bitrate: 256,
            meta: r#"{"rank":880000}"#.into(),
            ..Default::default()
        };

        let mut all = cands.clone();
        all.extend([
            mr_children_shinkai.clone(),
            kessoku_mashup.clone(),
            metallica_one.clone(),
            metallica_live.clone(),
        ]);
        store.upsert_tracks(&all).unwrap();
        for t in &cands {
            let _ = store.record_event(u.id, &t.id, "play", 1.0);
        }

        // candidate_tracks_for_user & tracks_by_mood must not pull unplayed archive/soulseek leftovers.
        let user_pool_ids: HashSet<String> = store
            .candidate_tracks_for_user(u.id, 100)
            .unwrap()
            .into_iter()
            .map(|t| t.id)
            .collect();
        assert!(!user_pool_ids.contains("arc:shinkai1"));
        assert!(!user_pool_ids.contains("slsk:mashup1"));

        let mood_db_ids: HashSet<String> = store
            .tracks_by_mood(Mood::Energy, 100, 0)
            .unwrap()
            .into_iter()
            .map(|t| t.id)
            .collect();
        assert!(!mood_db_ids.contains("arc:shinkai1"));
        assert!(!mood_db_ids.contains("slsk:mashup1"));

        // rank_mood_mix excludes unplayed archive/soulseek, mashups, live concert tracks, and Metallica.
        let curated = HashSet::from([metallica_one.id.clone(), metallica_live.id.clone()]);
        let ranked = taste.rank_mood_mix(u.id, Mood::Energy, all, &curated, 35);
        let ids: Vec<&str> = ranked.iter().map(|t| t.id.as_str()).collect();
        assert!(!ids.contains(&"arc:shinkai1"), "unplayed archive leftover leaked: {:?}", ids);
        assert!(!ids.contains(&"slsk:mashup1"), "mashup leaked: {:?}", ids);
        assert!(!ids.contains(&"dz:met1"), "Western metal leaked into J-Rock mix: {:?}", ids);
        assert!(!ids.contains(&"dz:met2"), "live concert metal leaked: {:?}", ids);
        assert!(ids.iter().all(|id| id.starts_with("dz:jrock")), "ids={:?}", ids);
    }

    #[test]
    fn mood_mix_excludes_unrequested_arabic_ytm_tracks_while_keeping_listener_japanese_scripts() {
        let store = Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("jp_mel_fan", "pw").unwrap();
        let sources = SourceRegistry::new(
            crate::sources::deezer::DeezerSource::new(),
            crate::sources::ytm::YtmSource::new(None),
            crate::sources::archive::ArchiveSource::new(),
            Arc::new(crate::sources::local::LocalSource::new(store.clone(), vec![])),
            crate::sources::soulseek::SoulseekSource::new(store.clone(), std::env::temp_dir()),
        );
        let taste = TasteEngine::new(store.clone(), sources);

        let jp_seed = Track {
            id: "ytm:jp1".into(),
            source: "ytm".into(),
            title: "まちがいさがし - Machigaisagashi".into(),
            artist: "Masaki Suda".into(),
            album: "LOVE".into(),
            genre: "J-Pop".into(),
            moods: "melancholy".into(),
            duration: 224,
            ..Default::default()
        };
        let jp_curated = Track {
            id: "ytm:jp2".into(),
            source: "ytm".into(),
            title: "青い栞 - Aoi Shiori".into(),
            artist: "Galileo Galilei".into(),
            album: "Aoi Shiori".into(),
            genre: String::new(),
            moods: "melancholy".into(),
            duration: 340,
            ..Default::default()
        };
        let en_curated = Track {
            id: "ytm:en1".into(),
            source: "ytm".into(),
            title: "Another Love (Official Video)".into(),
            artist: "Tom Odell".into(),
            album: "Another Love".into(),
            genre: String::new(),
            moods: "melancholy".into(),
            duration: 248,
            ..Default::default()
        };
        let arabic_1 = Track {
            id: "ytm:ar1".into(),
            source: "ytm".into(),
            title: "لا يا قلب".into(),
            artist: "Raad And Methaq".into(),
            album: "لا يا قلب".into(),
            genre: String::new(),
            moods: "melancholy".into(),
            duration: 286,
            ..Default::default()
        };
        let arabic_2 = Track {
            id: "ytm:ar2".into(),
            source: "ytm".into(),
            title: "شوارع فارغة".into(),
            artist: "Ghaith Sabah".into(),
            album: "شوارع فارغة".into(),
            genre: String::new(),
            moods: "melancholy".into(),
            duration: 137,
            ..Default::default()
        };

        let all = vec![
            jp_seed.clone(),
            jp_curated.clone(),
            en_curated.clone(),
            arabic_1.clone(),
            arabic_2.clone(),
        ];
        store.upsert_tracks(&all).unwrap();
        let _ = store.record_event(u.id, &jp_seed.id, "play", 1.0);

        let curated = HashSet::from([
            jp_curated.id.clone(),
            en_curated.id.clone(),
            arabic_1.id.clone(),
            arabic_2.id.clone(),
        ]);
        let ranked = taste.rank_mood_mix(u.id, Mood::Melancholy, all, &curated, 35);
        let ids: Vec<&str> = ranked.iter().map(|t| t.id.as_str()).collect();
        assert!(ids.contains(&"ytm:jp1"), "ids={:?}", ids);
        assert!(ids.contains(&"ytm:jp2"), "Japanese track should be kept: {:?}", ids);
        assert!(ids.contains(&"ytm:en1"), "English track should be kept: {:?}", ids);
        assert!(!ids.contains(&"ytm:ar1"), "Arabic track leaked: {:?}", ids);
        assert!(!ids.contains(&"ytm:ar2"), "Arabic track leaked: {:?}", ids);
    }

    #[test]
    fn eighties_hits_ytm_fallback_excludes_off_era_tracks_and_hits_spam() {
        let store = Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("era80s", "pw").unwrap();
        let sources = SourceRegistry::new(
            crate::sources::deezer::DeezerSource::new(),
            crate::sources::ytm::YtmSource::new(None),
            crate::sources::archive::ArchiveSource::new(),
            Arc::new(crate::sources::local::LocalSource::new(store.clone(), vec![])),
            crate::sources::soulseek::SoulseekSource::new(store.clone(), std::env::temp_dir()),
        );
        let taste = TasteEngine::new(store.clone(), sources);

        let mk_ytm = |id: &str, title: &str, artist: &str, year: i32| Track {
            id: id.into(),
            source: "ytm".into(),
            title: title.into(),
            artist: artist.into(),
            year,
            duration: 230,
            ..Default::default()
        };

        let mut ytm_radio = Vec::new();
        for i in 0..8 {
            ytm_radio.push(mk_ytm(
                &format!("ytm:80s_{}", i),
                &format!("80s Classic {}", i),
                &format!("80s Band {}", i),
                1981 + (i % 8),
            ));
        }
        let wrong_1958 = mk_ytm("ytm:1958", "O Mere Dil Ke Chain", "Kishore Kumar", 1958);
        let wrong_1970 = mk_ytm("ytm:1970", "Gulabi Ankhen", "Mohammed Rafi", 1970);
        let wrong_1994 = mk_ytm("ytm:1994", "Waada Raha Sanam", "Abhijeet", 1994);
        let spam_artist = Track {
            id: "dz:spam2".into(),
            source: "deezer".into(),
            title: "Don't Stop Believin'".into(),
            artist: "80s Unforgettable Hits".into(),
            duration: 240,
            ..Default::default()
        };
        ytm_radio.extend([wrong_1958, wrong_1970, wrong_1994]);

        let mix_ctx = SearchMixContext {
            ytm_radio,
            ..Default::default()
        };
        let res = taste.search_and_refine_with_context(
            u.id,
            "80s hits",
            vec![spam_artist],
            vec![],
            mix_ctx,
        );
        let mix = res.instant_mix.expect("80s hits mix should build from verified 80s tracks");
        let ids: Vec<&str> = mix.tracks.iter().map(|t| t.id.as_str()).collect();
        assert!(ids.iter().all(|id| id.starts_with("ytm:80s_")), "ids={:?}", ids);
        assert!(!ids.contains(&"ytm:1958"));
        assert!(!ids.contains(&"ytm:1970"));
        assert!(!ids.contains(&"ytm:1994"));
        assert!(!ids.contains(&"dz:spam2"));
    }

    #[test]
    fn curated_pool_roundtrip_and_curator_status_snapshot() {
        let store = Store::open_memory().unwrap();
        let sources = SourceRegistry::new(
            crate::sources::deezer::DeezerSource::new(),
            crate::sources::ytm::YtmSource::new(None),
            crate::sources::archive::ArchiveSource::new(),
            Arc::new(crate::sources::local::LocalSource::new(store.clone(), vec![])),
            crate::sources::soulseek::SoulseekSource::new(store.clone(), std::env::temp_dir()),
        );
        let taste = TasteEngine::new(store.clone(), sources);

        let t1 = Track {
            id: "dz:m1".into(),
            source: "deezer".into(),
            title: "Holocene".into(),
            artist: "Bon Iver".into(),
            cover_url: "https://example.com/c1.jpg".into(),
            ..Default::default()
        };
        let t2 = Track {
            id: "dz:m2".into(),
            source: "deezer".into(),
            title: "Motion Picture Soundtrack".into(),
            artist: "Radiohead".into(),
            cover_url: "https://example.com/c2.jpg".into(),
            ..Default::default()
        };
        store.upsert_tracks(&[t1.clone(), t2.clone()]).unwrap();
        taste.save_curated_pool(&mood_cache_key(Mood::Melancholy), &[t1.id.clone(), t2.id.clone()]);
        taste.save_curated_query_tracks("80s hits", &[t1.clone(), t2.clone()]);

        let (cnt, covers) = taste.curated_mood_preview(Mood::Melancholy);
        assert_eq!(cnt, 2);
        assert_eq!(covers.len(), 2);
        assert_eq!(taste.load_curated_query_tracks("80s hits").len(), 2);

        let snap = taste.curator_status_snapshot();
        assert_eq!(snap["active"], true);
        assert_eq!(snap["readyMoods"], 1);
        assert_eq!(snap["totalMoods"], 9);
    }

    #[tokio::test]
    async fn curated_mood_preview_for_user_updates_after_mood_mix_tracks() {
        let store = Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("moodfan", "pw").unwrap();
        let sources = SourceRegistry::new(
            crate::sources::deezer::DeezerSource::new(),
            crate::sources::ytm::YtmSource::new(None),
            crate::sources::archive::ArchiveSource::new(),
            Arc::new(crate::sources::local::LocalSource::new(
                store.clone(),
                vec![],
            )),
            crate::sources::soulseek::SoulseekSource::new(store.clone(), std::env::temp_dir()),
        );
        let taste = TasteEngine::new(store.clone(), sources);

        let mut tracks = Vec::new();
        for i in 0..14 {
            tracks.push(Track {
                id: format!("dz:ln{i}"),
                source: "deezer".into(),
                title: format!("Midnight Drive {i}"),
                artist: format!("Night Artist {i}"),
                album: "After Hours".into(),
                genre: "Synthwave".into(),
                moods: if i == 0 { "late-night".into() } else { String::new() },
                cover_url: format!("https://example.com/ln{i}.jpg"),
                duration: 210,
                bitrate: 320,
                stream_url: format!("dz:ln{i}"),
                ..Default::default()
            });
        }
        store.upsert_tracks(&tracks).unwrap();
        for t in &tracks {
            let _ = store.record_event(u.id, &t.id, "play", 1.0);
        }

        let (before_cnt, _) = taste.curated_mood_preview_for_user(u.id, Mood::LateNight);
        assert_eq!(before_cnt, 14);

        let mix = taste.mood_mix_tracks(u.id, Mood::LateNight, 35).await;
        assert!(mix.len() >= 14);

        let (after_cnt, after_covers) = taste.curated_mood_preview_for_user(u.id, Mood::LateNight);
        assert_eq!(after_cnt, mix.len());
        assert_eq!(after_covers.len(), 4);
    }

    #[tokio::test]
    async fn curated_mood_and_explore_previews_match_opened_mix_count_before_opening() {
        let store = Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("countcheck", "pw").unwrap();
        let sources = SourceRegistry::new(
            crate::sources::deezer::DeezerSource::new(),
            crate::sources::ytm::YtmSource::new(None),
            crate::sources::archive::ArchiveSource::new(),
            Arc::new(crate::sources::local::LocalSource::new(
                store.clone(),
                vec![],
            )),
            crate::sources::soulseek::SoulseekSource::new(store.clone(), std::env::temp_dir()),
        );
        let taste = TasteEngine::new(store.clone(), sources);

        // Build 48 curated tracks for Chill (exceeding the 35-track mix limit) plus 2 hidden tracks.
        let mut mood_tracks = Vec::new();
        let mut mood_ids = Vec::new();
        for i in 0..48 {
            let id = format!("dz:chill_pool_{i}");
            mood_ids.push(id.clone());
            mood_tracks.push(Track {
                id: id.clone(),
                source: "deezer".into(),
                title: format!("Soft Breeze {i}"),
                artist: format!("LoFi Artist {i}"),
                album: "Chill Sessions".into(),
                genre: "Lo-Fi".into(),
                moods: "chill".into(),
                cover_url: format!("https://example.com/chill{i}.jpg"),
                duration: 195,
                bitrate: 320,
                stream_url: id,
                meta: r#"{"rank":500000}"#.into(),
                ..Default::default()
            });
        }
        store.upsert_tracks(&mood_tracks).unwrap();
        taste.save_curated_pool(&mood_cache_key(Mood::Chill), &mood_ids);
        // Hide one track from the curated pool.
        taste.mark_not_interested(u.id, &mood_ids[0]);

        let (mood_before_cnt, _) = taste.curated_mood_preview_for_user(u.id, Mood::Chill);
        let mood_opened = taste.mood_mix_tracks(u.id, Mood::Chill, 35).await;
        let (mood_after_cnt, _) = taste.curated_mood_preview_for_user(u.id, Mood::Chill);
        assert_eq!(mood_opened.len(), 35);
        assert_eq!(mood_before_cnt, mood_opened.len());
        assert_eq!(mood_after_cnt, mood_opened.len());

        // Build 45 curated tracks for the 80s Hits explore mix (also exceeding the 35-track limit).
        let spec = find_explore_mix_spec("80s_hits").unwrap();
        let mut era_tracks = Vec::new();
        let mut era_ids = Vec::new();
        for i in 0..45 {
            let id = format!("dz:80s_pool_{i}");
            era_ids.push(id.clone());
            era_tracks.push(Track {
                id: id.clone(),
                source: "deezer".into(),
                title: format!("Neon Anthem {i}"),
                artist: format!("80s Band {i}"),
                album: "1985 Classics".into(),
                genre: "Synthpop".into(),
                year: 1985,
                cover_url: format!("https://example.com/80s{i}.jpg"),
                duration: 220,
                bitrate: 320,
                stream_url: id,
                meta: r#"{"rank":500000}"#.into(),
                ..Default::default()
            });
        }
        store.upsert_tracks(&era_tracks).unwrap();
        taste.save_curated_pool(&explore_cache_key(spec.key), &era_ids);

        let (era_before_cnt, _) = taste.curated_explore_preview_for_user(u.id, spec);
        let era_opened = taste.get_curated_explore_mix_tracks(u.id, spec, 35).await;
        let (era_after_cnt, _) = taste.curated_explore_preview_for_user(u.id, spec);
        assert_eq!(era_opened.len(), 35);
        assert_eq!(era_before_cnt, era_opened.len());
        assert_eq!(era_after_cnt, era_opened.len());
    }

    #[tokio::test]
    async fn mixed_for_you_builds_numbered_mixes_supermix_and_cards() {
        let store = Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("mixfan", "pw").unwrap();
        let sources = SourceRegistry::new(
            crate::sources::deezer::DeezerSource::new(),
            crate::sources::ytm::YtmSource::new(None),
            crate::sources::archive::ArchiveSource::new(),
            Arc::new(crate::sources::local::LocalSource::new(
                store.clone(),
                vec![],
            )),
            crate::sources::soulseek::SoulseekSource::new(store.clone(), std::env::temp_dir()),
        );
        let taste = TasteEngine::new(store.clone(), sources);

        let tracks: Vec<Track> = (0..30)
            .map(|i| Track {
                id: format!("dz:e{i}"),
                source: "deezer".into(),
                title: format!("Loud Track {i}"),
                artist: format!("Energy Artist {i}"),
                album: "Loud".into(),
                genre: "Rock".into(),
                moods: "energy".into(),
                cover_url: format!("https://example.com/e{i}.jpg"),
                duration: 200,
                bitrate: 256,
                stream_url: format!("dz:e{i}"),
                ..Default::default()
            })
            .collect();
        store.upsert_tracks(&tracks).unwrap();
        let ids: Vec<String> = tracks.iter().map(|t| t.id.clone()).collect();
        taste.save_curated_pool(&mood_cache_key(Mood::Energy), &ids);

        taste.rebuild_mixed_for_you(u.id).await;

        let cards = taste.mixed_for_you_cards(u.id);
        assert_eq!(cards.len(), 4, "cards: {:?}", cards.iter().map(|c| &c.title).collect::<Vec<_>>());
        assert_eq!(cards[0].title, "Daily Mix 1");
        assert_eq!(cards[0].art_title, "Daily Mix");
        assert_eq!(cards[0].number, "01");
        assert_eq!(cards[3].title, "Daily Supermix");
        assert!(cards[3].is_super);
        assert!(cards[3].number.is_empty());
        assert_eq!(cards[0].covers.len(), 4);
        assert!(cards[0].subtitle.contains("Energy Artist"));

        let p1 = store.playlist_tracks(&cards[0].id).unwrap();
        let p2 = store.playlist_tracks(&cards[1].id).unwrap();
        assert!(p1.len() >= 12);
        assert_ne!(
            p1.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            p2.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            "numbered mixes should not be identical"
        );

        // The rail is separate from the generic auto-playlist list.
        let generic_auto = store
            .list_playlists(u.id, false)
            .unwrap()
            .into_iter()
            .filter(|p| p.is_auto && !p.kind.starts_with("mixed_for_you:"))
            .count();
        assert_eq!(generic_auto, 0);
    }

    #[tokio::test]
    async fn mixed_for_you_keeps_untagged_ytm_listener_music_over_global_pool() {
        let store = Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("jpopytm", "pw").unwrap();
        let sources = SourceRegistry::new(
            crate::sources::deezer::DeezerSource::new(),
            crate::sources::ytm::YtmSource::new(None),
            crate::sources::archive::ArchiveSource::new(),
            Arc::new(crate::sources::local::LocalSource::new(store.clone(), vec![])),
            crate::sources::soulseek::SoulseekSource::new(store.clone(), std::env::temp_dir()),
        );
        let taste = TasteEngine::new(store.clone(), sources);

        let mk = |id: String, source: &str, artist: String, genre: &str, moods: &str| Track {
            id: id.clone(),
            source: source.into(),
            title: format!("Song {id}"),
            artist,
            album: "Album".into(),
            genre: genre.into(),
            moods: moods.into(),
            cover_url: format!("https://example.com/{id}.jpg"),
            duration: 220,
            bitrate: 256,
            stream_url: id,
            ..Default::default()
        };
        // Global, popular Upbeat pool the curator would cache.
        let global: Vec<Track> = (0..40)
            .map(|i| mk(format!("dz:pop{i}"), "deezer", format!("Chart Star {i}"), "Pop", "upbeat"))
            .collect();
        // A few J-pop tracks with tags, and lots of untagged YouTube Music listening.
        let tagged: Vec<Track> = (0..12)
            .map(|i| mk(format!("dz:jp{i}"), "deezer", format!("JP Artist {i}"), "J-Pop", "upbeat"))
            .collect();
        let untagged: Vec<Track> = (0..20)
            .map(|i| mk(format!("ytm:jp{i}"), "ytm", format!("YTM JP Artist {i}"), "", ""))
            .collect();
        store.upsert_tracks(&global).unwrap();
        store.upsert_tracks(&tagged).unwrap();
        store.upsert_tracks(&untagged).unwrap();
        for t in tagged.iter().chain(untagged.iter()) {
            let _ = store.record_event(u.id, &t.id, "play", 1.0);
            let _ = store.record_event(u.id, &t.id, "complete", 1.0);
        }
        let ids: Vec<String> = global.iter().map(|t| t.id.clone()).collect();
        taste.save_curated_pool(&mood_cache_key(Mood::Upbeat), &ids);

        taste.rebuild_mixed_for_you(u.id).await;

        let cards = taste.mixed_for_you_cards(u.id);
        assert_eq!(cards.len(), 4);
        assert_eq!(cards[0].title, "Daily Mix 1");
        assert_eq!(store.kv_get(&format!("mix4u:mood:{}", u.id)).as_deref(), Some(Mood::Upbeat.slug()));
        let mut all_ids: Vec<String> = Vec::new();
        for c in &cards {
            all_ids.extend(store.playlist_tracks(&c.id).unwrap().into_iter().map(|t| t.id));
        }
        let ytm = all_ids.iter().filter(|id| id.starts_with("ytm:jp")).count();
        let jp = all_ids.iter().filter(|id| id.starts_with("dz:jp")).count();
        let pop = all_ids.iter().filter(|id| id.starts_with("dz:pop")).count();
        assert!(ytm > 0, "untagged listener tracks missing: {:?}", all_ids);
        assert!(jp > 0, "tagged listener tracks missing: {:?}", all_ids);
        assert!(
            pop * 3 < ytm + jp,
            "global pool dominates: pop={pop} ytm={ytm} jp={jp} {:?}",
            all_ids
        );
    }

    #[tokio::test]
    async fn mixed_for_you_does_not_leak_western_pop_into_jpop_feel_good_mixes() {
        let store = Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("jpop_only", "pw").unwrap();
        let sources = SourceRegistry::new(
            crate::sources::deezer::DeezerSource::new(),
            crate::sources::ytm::YtmSource::new(None),
            crate::sources::archive::ArchiveSource::new(),
            Arc::new(crate::sources::local::LocalSource::new(store.clone(), vec![])),
            crate::sources::soulseek::SoulseekSource::new(store.clone(), std::env::temp_dir()),
        );
        let taste = TasteEngine::new(store.clone(), sources);

        let mk = |id: String, artist: String, genre: &str| Track {
            id: id.clone(),
            source: "deezer".into(),
            title: format!("Song {id}"),
            artist,
            album: "Album".into(),
            genre: genre.into(),
            moods: "upbeat".into(),
            cover_url: format!("https://example.com/{id}.jpg"),
            duration: 210,
            bitrate: 256,
            stream_url: id,
            ..Default::default()
        };

        // Global Western Pop chart pool (e.g. Dua Lipa, Justin Bieber).
        let western_pop: Vec<Track> = (0..30)
            .map(|i| mk(format!("dz:west{i}"), format!("Western Pop Star {i}"), "Pop"))
            .collect();
        // Listener's Japanese tracks, tagged "Pop, J-Pop" (which builds genre_affinity["pop"] > 0.4).
        let jpop: Vec<Track> = (0..40)
            .map(|i| mk(format!("dz:jp{i}"), format!("JP Artist {i}"), "Pop, J-Pop"))
            .collect();
        store.upsert_tracks(&western_pop).unwrap();
        store.upsert_tracks(&jpop).unwrap();
        for t in &jpop {
            let _ = store.record_event(u.id, &t.id, "play", 1.0);
            let _ = store.record_event(u.id, &t.id, "complete", 1.0);
        }
        let ids: Vec<String> = western_pop.iter().map(|t| t.id.clone()).collect();
        taste.save_curated_pool(&mood_cache_key(Mood::Upbeat), &ids);

        taste.rebuild_mixed_for_you(u.id).await;

        let cards = taste.mixed_for_you_cards(u.id);
        assert_eq!(cards.len(), 4);
        assert_eq!(cards[0].title, "Daily Mix 1");
        assert_eq!(store.kv_get(&format!("mix4u:mood:{}", u.id)).as_deref(), Some(Mood::Upbeat.slug()));
        for c in &cards {
            let tracks = store.playlist_tracks(&c.id).unwrap();
            assert!(!tracks.is_empty());
            let leaked: Vec<&str> = tracks
                .iter()
                .filter(|t| t.id.starts_with("dz:west"))
                .map(|t| t.id.as_str())
                .collect();
            assert!(
                leaked.is_empty(),
                "Western pop leaked into {}: {:?}",
                c.title,
                leaked
            );
        }
    }

    #[test]
    fn pick_user_mood_query_genres_prefers_scene_genres_over_broad_pop() {
        let jpop_user = vec![
            ("pop".to_string(), 14.0),
            ("j-pop".to_string(), 11.0),
            ("asian music".to_string(), 9.0),
            ("j-rock".to_string(), 5.0),
        ];
        assert_eq!(
            pick_user_mood_query_genres(&jpop_user),
            vec!["j-pop".to_string(), "j-rock".to_string()]
        );

        let mainstream_pop_user = vec![
            ("pop".to_string(), 12.0),
            ("dance".to_string(), 6.0),
        ];
        assert_eq!(
            pick_user_mood_query_genres(&mainstream_pop_user),
            vec!["pop".to_string(), "dance".to_string()]
        );
    }

    #[test]
    fn jpop_instant_mix_accepts_hyphenated_playlists_and_rejects_pop_and_kpop_drift() {
        let intent = parse_search_intent("jpop");
        assert!(intent.is_theme_mix);
        assert_eq!(intent.genres, vec!["J-Pop".to_string()]);
        assert!(intent.deezer_genre_hints().is_empty());
        let pl_q = intent.playlist_search_queries();
        assert!(pl_q.contains(&"jpop".to_string()), "pl_q={:?}", pl_q);
        assert!(pl_q.contains(&"j-pop".to_string()), "pl_q={:?}", pl_q);

        let store = Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("rnb_kpop_user", "pw").unwrap();
        let sources = SourceRegistry::new(
            crate::sources::deezer::DeezerSource::new(),
            crate::sources::ytm::YtmSource::new(None),
            crate::sources::archive::ArchiveSource::new(),
            Arc::new(crate::sources::local::LocalSource::new(store.clone(), vec![])),
            crate::sources::soulseek::SoulseekSource::new(store.clone(), std::env::temp_dir()),
        );
        let taste = TasteEngine::new(store.clone(), sources);

        let mk = |id: &str, title: &str, artist: &str, genre: &str, rank: u64| Track {
            id: id.into(),
            source: "deezer".into(),
            title: title.into(),
            artist: artist.into(),
            album: title.into(),
            genre: genre.into(),
            duration: 210,
            bitrate: 256,
            meta: format!(r#"{{"rank":{}}}"#, rank),
            ..Default::default()
        };

        let snoh = mk("dz:snoh", "Warmest Shade Of Blue", "Snoh Aalegra", "R&B", 950000);
        let wiz = mk("dz:wiz", "Highs and Lows", "Wiz Khalifa", "Hip-Hop", 940000);
        let taeyeon = mk("dz:taeyeon", "Panorama", "TAEYEON", "K-Pop", 930000);
        let iu = mk("dz:iu", "Dear my crazy soulmate", "IU", "K-Pop", 920000);
        let weeknd_kpop = mk("dz:weeknd", "K-POP", "The Weeknd", "", 980000);

        // Real curated J-Pop tracks carry (enriched) J-Pop/Pop genre metadata.
        let yoasobi = mk("dz:yoasobi", "New me", "YOASOBI", "J-Pop", 880000);
        let fujii = mk("dz:fujii", "Hachiko", "Fujii Kaze", "Pop", 870000);
        let yonezu = mk("dz:yonezu", "Azalea", "Kenshi Yonezu", "J-Pop", 860000);
        let anri = mk("dz:anri", "SHYNESS BOY", "Anri", "City Pop", 800000);

        store
            .upsert_tracks(&[
                snoh.clone(),
                wiz.clone(),
                taeyeon.clone(),
                iu.clone(),
                weeknd_kpop.clone(),
                yoasobi.clone(),
                fujii.clone(),
                yonezu.clone(),
                anri.clone(),
            ])
            .unwrap();
        // Give user strong personal affinity to Snoh Aalegra & TAEYEON so taste_aff can't leak them.
        store.star_track(u.id, &snoh.id).unwrap();
        store.star_track(u.id, &taeyeon.id).unwrap();
        taste.record_event(u.id, &snoh.id, "complete", 1.0);
        taste.record_event(u.id, &taeyeon.id, "complete", 1.0);

        let res = taste.search_and_refine_with_context(
            u.id,
            "jpop",
            vec![snoh.clone(), wiz.clone(), weeknd_kpop.clone()],
            vec![
                ("Top J-Pop".into(), vec![yoasobi.clone(), fujii.clone()]),
                ("J-Pop Essentials".into(), vec![yonezu.clone()]),
                ("KPOP x JPOP mix".into(), vec![taeyeon.clone(), iu.clone()]),
            ],
            SearchMixContext::default(),
        );
        let mix = res.instant_mix.expect("J-Pop mix should be created");
        assert_eq!(mix.title, "J-Pop Mix");
        let ids: Vec<&str> = mix.tracks.iter().map(|t| t.id.as_str()).collect();
        assert!(ids.contains(&yoasobi.id.as_str()), "ids={:?}", ids);
        assert!(ids.contains(&fujii.id.as_str()), "ids={:?}", ids);
        assert!(ids.contains(&yonezu.id.as_str()), "ids={:?}", ids);
        assert!(!ids.contains(&snoh.id.as_str()), "R&B leaked: {:?}", ids);
        assert!(!ids.contains(&wiz.id.as_str()), "Hip-Hop leaked: {:?}", ids);
        assert!(!ids.contains(&taeyeon.id.as_str()), "K-Pop leaked: {:?}", ids);
        assert!(!ids.contains(&iu.id.as_str()), "K-Pop leaked: {:?}", ids);
        assert!(!ids.contains(&weeknd_kpop.id.as_str()), "Title trap leaked: {:?}", ids);

        // Even in offline/fallback mode without curated playlists, K-Pop and Pop must clash with J-Pop.
        assert!(has_genre_clash(&taeyeon, "", &["J-Pop".into()]));
        assert!(has_genre_clash(
            &mk("dz:pop", "Pop Hit", "Western Pop", "Pop", 900000),
            "",
            &["J-Pop".into()]
        ));
        assert!(!has_genre_clash(&anri, "", &["J-Pop".into()]));
    }

    #[test]
    fn late_night_mood_and_explore_reject_title_traps_and_duplicate_titles() {
        assert!(classify_moods("Late Night Vibes", "Cozy Deluxe, Vol. 15", "", "").is_empty());
        assert!(classify_moods("After Hours", "Album", "", "").is_empty());
        assert!(classify_moods("Nightcall", "Outrun", "Synthwave", "").contains(&Mood::LateNight));

        let store = Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("nightowl", "pw").unwrap();
        let sources = SourceRegistry::new(
            crate::sources::deezer::DeezerSource::new(),
            crate::sources::ytm::YtmSource::new(None),
            crate::sources::archive::ArchiveSource::new(),
            Arc::new(crate::sources::local::LocalSource::new(store.clone(), vec![])),
            crate::sources::soulseek::SoulseekSource::new(store.clone(), std::env::temp_dir()),
        );
        let taste = TasteEngine::new(store.clone(), sources);

        let mk = |id: &str, title: &str, artist: &str, genre: &str, moods: &str| Track {
            id: id.into(),
            source: "deezer".into(),
            title: title.into(),
            artist: artist.into(),
            album: title.into(),
            genre: genre.into(),
            moods: moods.into(),
            duration: 210,
            bitrate: 256,
            ..Default::default()
        };

        let trap_1 = mk("dz:trap1", "Late Night Vibes", "Vincent Price", "", "late_night");
        let trap_2 = mk("dz:trap2", "Late Night Vibes", "Darius Silk", "Pop", "late_night");
        let trap_3 = mk("dz:trap3", "Late Night", "Flow Sound", "", "late_night");
        let real_synth = mk("dz:real1", "Sunset", "The Midnight", "Synthwave", "late_night");
        let real_curated = mk("dz:real2", "the day she left", "inertia.", "", "");

        store
            .upsert_tracks(&[
                trap_1.clone(),
                trap_2.clone(),
                trap_3.clone(),
                real_synth.clone(),
                real_curated.clone(),
            ])
            .unwrap();

        let curated_ids = HashSet::from([real_curated.id.clone()]);
        let ranked = taste.rank_mood_mix(
            u.id,
            Mood::LateNight,
            vec![trap_1.clone(), trap_2.clone(), trap_3.clone(), real_synth.clone(), real_curated.clone()],
            &curated_ids,
            20,
        );
        let ids: Vec<&str> = ranked.iter().map(|t| t.id.as_str()).collect();
        assert!(ids.contains(&real_synth.id.as_str()), "ids={:?}", ids);
        assert!(ids.contains(&real_curated.id.as_str()), "ids={:?}", ids);
        assert!(!ids.contains(&trap_1.id.as_str()), "title trap leaked: {:?}", ids);
        assert!(!ids.contains(&trap_2.id.as_str()), "title trap leaked: {:?}", ids);
        assert!(!ids.contains(&trap_3.id.as_str()), "title trap leaked: {:?}", ids);

        // dedup_and_interleave caps duplicate identical core song titles to 1.
        let dup_titles = vec![
            mk("dz:d1", "Late Night Vibes", "Artist A", "Synthwave", ""),
            mk("dz:d2", "Late Night Vibes", "Artist B", "Synthwave", ""),
            mk("dz:d3", "Midnight City", "M83", "Synthwave", ""),
        ];
        let interleaved = dedup_and_interleave(dup_titles, 10, 2);
        assert_eq!(interleaved.len(), 2, "interleaved={:?}", interleaved.iter().map(|t| &t.id).collect::<Vec<_>>());
    }

    #[tokio::test]
    async fn artist_recommendations_and_related_tracks_work_and_dedup_songs() {
        let store = Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("jpop_fan", "pw").unwrap();
        let sources = SourceRegistry::new(
            crate::sources::deezer::DeezerSource::new(),
            crate::sources::ytm::YtmSource::new(None),
            crate::sources::archive::ArchiveSource::new(),
            Arc::new(crate::sources::local::LocalSource::new(store.clone(), vec![])),
            crate::sources::soulseek::SoulseekSource::new(store.clone(), std::env::temp_dir()),
        );
        let taste = TasteEngine::new(store.clone(), sources);

        let hires_dup = Track {
            id: "slsk:beaflower".into(),
            source: "soulseek".into(),
            title: "Be a flower".into(),
            artist: "Ryokuoushoku Shakai".into(),
            album: "Be a flower".into(),
            format: "flac".into(),
            bitrate: 1411,
            bit_depth: 24,
            sample_rate: 48000,
            duration: 199,
            ..Default::default()
        };
        let aac_dup = Track {
            id: "ytm:beaflower".into(),
            source: "ytm".into(),
            title: "Be a flower".into(),
            artist: "Ryokuoushoku Shakai".into(),
            album: "Be a flower".into(),
            format: "m4a".into(),
            bitrate: 128,
            duration: 200,
            ..Default::default()
        };
        let other_song = Track {
            id: "ytm:toumei".into(),
            source: "ytm".into(),
            title: "Toumeidatta Sekai".into(),
            artist: "Ryokuoushoku Shakai".into(),
            album: "Toumeidatta Sekai".into(),
            format: "m4a".into(),
            bitrate: 128,
            duration: 213,
            ..Default::default()
        };
        let rel_yorushika = Track {
            id: "dz:yorushika1".into(),
            source: "deezer".into(),
            title: "Sunny".into(),
            artist: "Yorushika".into(),
            album: "Sunny".into(),
            genre: "J-Pop".into(),
            duration: 240,
            bitrate: 256,
            ..Default::default()
        };
        let rel_kinggnu = Track {
            id: "dz:kinggnu1".into(),
            source: "deezer".into(),
            title: "Hakujitsu".into(),
            artist: "King Gnu".into(),
            album: "CEREMONY".into(),
            genre: "J-Rock".into(),
            duration: 276,
            bitrate: 256,
            ..Default::default()
        };
        store
            .upsert_tracks(&[
                hires_dup.clone(),
                aac_dup.clone(),
                other_song.clone(),
                rel_yorushika.clone(),
                rel_kinggnu.clone(),
            ])
            .unwrap();

        let anchor_meta = ArtistSummary {
            id: "ar:ryokuoushoku-shakai".into(),
            name: "Ryokuoushoku Shakai".into(),
            ..Default::default()
        };
        let yorushika_meta = ArtistSummary {
            id: "ar:dz:67898492".into(),
            name: "Yorushika".into(),
            cover_url: "https://cdn.deezer.com/yorushika.jpg".into(),
            bio: "80000 fans on Deezer".into(),
            deezer_id: "67898492".into(),
            ..Default::default()
        };
        let kinggnu_meta = ArtistSummary {
            id: "ar:dz:12944623".into(),
            name: "King Gnu".into(),
            cover_url: "https://cdn.deezer.com/kinggnu.jpg".into(),
            bio: "95000 fans on Deezer".into(),
            deezer_id: "12944623".into(),
            ..Default::default()
        };
        store.upsert_artist_meta(&anchor_meta).unwrap();
        store.upsert_artist_meta(&yorushika_meta).unwrap();
        store.upsert_artist_meta(&kinggnu_meta).unwrap();

        // Duplicate versions of "Be a flower" merge into the single lossless HI-RES track.
        let deduped = merge_tracks_dedup(vec![hires_dup.clone(), aac_dup.clone(), other_song.clone()], vec![]);
        assert_eq!(deduped.len(), 2, "deduped={:?}", deduped.iter().map(|t| &t.id).collect::<Vec<_>>());
        assert_eq!(deduped[0].id, "slsk:beaflower");

        // Artist related tracks surface Yorushika / King Gnu tracks while excluding anchor tracks.
        let art_rel = taste
            .artist_related_tracks(&anchor_meta, &[yorushika_meta.clone(), kinggnu_meta.clone()], &deduped, 10)
            .await;
        let art_rel_ids: Vec<&str> = art_rel.iter().map(|t| t.id.as_str()).collect();
        assert!(art_rel_ids.contains(&"dz:yorushika1"), "art_rel={:?}", art_rel_ids);
        assert!(art_rel_ids.contains(&"dz:kinggnu1"), "art_rel={:?}", art_rel_ids);
        assert!(!art_rel_ids.contains(&"slsk:beaflower"), "anchor leaked: {:?}", art_rel_ids);

        // User taste links surface both "Related Tracks" and "Artists You Might Like" on Home/Explore.
        store.follow_artist(u.id, &anchor_meta.id).unwrap();
        store
            .upsert_taste_links(
                u.id,
                &[
                    ("related_track", rel_yorushika.id.as_str(), 2.2),
                    ("related_artist", "yorushika", 2.0),
                    ("related_artist", "king-gnu", 1.8),
                ],
            )
            .unwrap();
        taste.mark_taste_dirty(u.id);

        let rec_tracks = taste.related_tracks_for_user(u.id, 10);
        let rec_track_ids: Vec<&str> = rec_tracks.iter().map(|t| t.id.as_str()).collect();
        assert!(rec_track_ids.contains(&"dz:yorushika1"), "rec_tracks={:?}", rec_track_ids);

        let rec_artists = taste.recommended_artists_for_user(u.id, 10);
        let rec_artist_names: Vec<&str> = rec_artists.iter().map(|a| a.name.as_str()).collect();
        assert!(rec_artist_names.contains(&"Yorushika"), "rec_artists={:?}", rec_artist_names);
        assert!(rec_artist_names.contains(&"King Gnu"), "rec_artists={:?}", rec_artist_names);
        assert!(
            !rec_artist_names.contains(&"Ryokuoushoku Shakai"),
            "already followed artist leaked: {:?}",
            rec_artist_names
        );
    }

    #[tokio::test]
    async fn familiar_and_discover_radio_stay_on_topic_and_genre() {
        let store = Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("xmas_jpop_user", "pw").unwrap();
        let sources = SourceRegistry::new(
            crate::sources::deezer::DeezerSource::new(),
            crate::sources::ytm::YtmSource::new(None),
            crate::sources::archive::ArchiveSource::new(),
            Arc::new(crate::sources::local::LocalSource::new(store.clone(), vec![])),
            crate::sources::soulseek::SoulseekSource::new(store.clone(), std::env::temp_dir()),
        );
        let taste = TasteEngine::new(store.clone(), sources);

        let mut tracks = Vec::new();
        // Heavy J-Pop user history so global for_you_tracks / discover_weekly are dominated by J-Pop.
        for i in 0..8 {
            tracks.push(Track {
                id: format!("loc:jpop-fam-{}", i),
                source: "local".into(),
                title: format!("Tokyo Neon {}", i),
                artist: format!("J-Pop Star {}", i),
                album: "Shibuya Nights".into(),
                genre: "J-Pop".into(),
                moods: "upbeat,energetic".into(),
                bitrate: 900,
                duration: 220,
                ..Default::default()
            });
            tracks.push(Track {
                id: format!("loc:jpop-new-{}", i),
                source: "local".into(),
                title: format!("Osaka Sunrise {}", i),
                artist: format!("J-Pop Newcomer {}", i),
                album: "City Lights".into(),
                genre: "J-Pop".into(),
                moods: "upbeat,energetic".into(),
                bitrate: 900,
                duration: 215,
                ..Default::default()
            });
        }

        // Christmas tracks: 1 seed, 2 familiar, 3 unheard discoveries.
        let xmas_seed = Track {
            id: "loc:xmas-seed".into(),
            source: "local".into(),
            title: "All I Want for Christmas Is You".into(),
            artist: "Mariah Carey".into(),
            album: "Merry Christmas".into(),
            genre: "Pop".into(),
            moods: "upbeat,happy".into(),
            bitrate: 900,
            duration: 241,
            ..Default::default()
        };
        let xmas_fam_1 = Track {
            id: "loc:xmas-fam-1".into(),
            source: "local".into(),
            title: "Last Christmas".into(),
            artist: "Wham!".into(),
            album: "Music from the Edge of Heaven".into(),
            genre: "Pop".into(),
            moods: "upbeat,chill".into(),
            bitrate: 900,
            duration: 262,
            ..Default::default()
        };
        let xmas_fam_2 = Track {
            id: "loc:xmas-fam-2".into(),
            source: "local".into(),
            title: "White Christmas".into(),
            artist: "Bing Crosby".into(),
            album: "Holiday Inn".into(),
            genre: "Holiday".into(),
            moods: "chill".into(),
            bitrate: 900,
            duration: 185,
            ..Default::default()
        };
        let xmas_new_1 = Track {
            id: "loc:xmas-new-1".into(),
            source: "local".into(),
            title: "Jingle Bell Rock".into(),
            artist: "Bobby Helms".into(),
            album: "Christmas Classics".into(),
            genre: "Holiday".into(),
            moods: "upbeat".into(),
            bitrate: 900,
            duration: 131,
            ..Default::default()
        };
        let xmas_new_2 = Track {
            id: "loc:xmas-new-2".into(),
            source: "local".into(),
            title: "Let It Snow! Let It Snow!".into(),
            artist: "Dean Martin".into(),
            album: "A Winter Romance".into(),
            genre: "Jazz".into(),
            moods: "chill".into(),
            bitrate: 900,
            duration: 117,
            ..Default::default()
        };
        let xmas_new_3 = Track {
            id: "loc:xmas-new-3".into(),
            source: "local".into(),
            title: "Feliz Navidad".into(),
            artist: "Jose Feliciano".into(),
            album: "Feliz Navidad".into(),
            genre: "Pop".into(),
            moods: "upbeat".into(),
            bitrate: 900,
            duration: 182,
            ..Default::default()
        };

        // Jazz tracks to verify non-seasonal genre scoping too.
        let jazz_seed = Track {
            id: "loc:jazz-seed".into(),
            source: "local".into(),
            title: "So What".into(),
            artist: "Miles Davis".into(),
            album: "Kind of Blue".into(),
            genre: "Jazz".into(),
            moods: "chill,night".into(),
            bitrate: 900,
            duration: 545,
            ..Default::default()
        };
        let jazz_fam = Track {
            id: "loc:jazz-fam".into(),
            source: "local".into(),
            title: "Take Five".into(),
            artist: "Dave Brubeck".into(),
            album: "Time Out".into(),
            genre: "Jazz".into(),
            moods: "chill".into(),
            bitrate: 900,
            duration: 324,
            ..Default::default()
        };
        let jazz_new = Track {
            id: "loc:jazz-new".into(),
            source: "local".into(),
            title: "Blue Train".into(),
            artist: "John Coltrane".into(),
            album: "Blue Train".into(),
            genre: "Jazz".into(),
            moods: "night".into(),
            bitrate: 900,
            duration: 643,
            ..Default::default()
        };

        tracks.extend([
            xmas_seed.clone(),
            xmas_fam_1.clone(),
            xmas_fam_2.clone(),
            xmas_new_1.clone(),
            xmas_new_2.clone(),
            xmas_new_3.clone(),
            jazz_seed.clone(),
            jazz_fam.clone(),
            jazz_new.clone(),
        ]);
        store.upsert_tracks(&tracks).unwrap();

        // Build strong J-Pop preference, plus familiarity with two Christmas tracks and one Jazz track.
        for i in 0..8 {
            let tid = format!("loc:jpop-fam-{}", i);
            for _ in 0..4 {
                taste.record_event(u.id, &tid, "play", 1.0);
                taste.record_event(u.id, &tid, "complete", 1.0);
            }
            store.star_track(u.id, &tid).unwrap();
            taste.record_event(u.id, &tid, "star", 1.0);
        }
        store.star_track(u.id, &xmas_fam_1.id).unwrap();
        store.star_track(u.id, &xmas_fam_2.id).unwrap();
        taste.record_event(u.id, &xmas_fam_1.id, "star", 1.0);
        taste.record_event(u.id, &xmas_fam_2.id, "star", 1.0);
        store.star_track(u.id, &jazz_fam.id).unwrap();
        taste.record_event(u.id, &jazz_fam.id, "star", 1.0);
        taste.mark_taste_dirty(u.id);

        // 1. Track-seeded Christmas radio in Familiar mode: must NOT play J-Pop.
        let xmas_familiar = taste
            .radio_next_tracks_with_mode(
                u.id,
                Some(&xmas_seed),
                &HashSet::new(),
                &[],
                6,
                RadioMode::Familiar,
            )
            .await;
        let xmas_fam_ids: Vec<&str> = xmas_familiar.iter().map(|t| t.id.as_str()).collect();
        assert!(!xmas_fam_ids.is_empty());
        assert!(
            !xmas_fam_ids.iter().any(|id| id.contains("jpop")),
            "Familiar Christmas radio leaked J-Pop: {:?}",
            xmas_fam_ids
        );
        assert!(
            xmas_fam_ids.contains(&xmas_fam_1.id.as_str())
                && xmas_fam_ids.contains(&xmas_fam_2.id.as_str()),
            "Familiar Christmas radio should include familiar Christmas tracks: {:?}",
            xmas_fam_ids
        );

        // 2. Track-seeded Christmas radio in Discover mode: must return unheard Christmas tracks, no J-Pop.
        let xmas_discover = taste
            .radio_next_tracks_with_mode(
                u.id,
                Some(&xmas_seed),
                &HashSet::new(),
                &[],
                6,
                RadioMode::Discover,
            )
            .await;
        let xmas_disc_ids: Vec<&str> = xmas_discover.iter().map(|t| t.id.as_str()).collect();
        assert!(!xmas_disc_ids.is_empty());
        assert!(
            !xmas_disc_ids.iter().any(|id| id.contains("jpop")),
            "Discover Christmas radio leaked J-Pop: {:?}",
            xmas_disc_ids
        );
        assert!(
            !xmas_disc_ids.contains(&xmas_fam_1.id.as_str())
                && !xmas_disc_ids.contains(&xmas_fam_2.id.as_str()),
            "Discover Christmas radio should exclude familiar Christmas tracks: {:?}",
            xmas_disc_ids
        );
        assert!(
            xmas_discover.iter().all(|t| track_matches_topic(t, "christmas")),
            "Discover Christmas radio should stay on-topic: {:?}",
            xmas_discover
                .iter()
                .map(|t| format!("{}: {}", t.id, t.title))
                .collect::<Vec<_>>()
        );

        // 3. Track-seeded Jazz radio in Familiar & Discover modes: must NOT play J-Pop.
        let jazz_familiar = taste
            .radio_next_tracks_with_mode(
                u.id,
                Some(&jazz_seed),
                &HashSet::new(),
                &[],
                4,
                RadioMode::Familiar,
            )
            .await;
        let jazz_fam_ids: Vec<&str> = jazz_familiar.iter().map(|t| t.id.as_str()).collect();
        assert!(
            !jazz_fam_ids.iter().any(|id| id.contains("jpop")),
            "Familiar Jazz radio leaked J-Pop: {:?}",
            jazz_fam_ids
        );
        assert!(jazz_fam_ids.contains(&jazz_fam.id.as_str()), "jazz_fam={:?}", jazz_fam_ids);

        let jazz_discover = taste
            .radio_next_tracks_with_mode(
                u.id,
                Some(&jazz_seed),
                &HashSet::new(),
                &[],
                4,
                RadioMode::Discover,
            )
            .await;
        let jazz_disc_ids: Vec<&str> = jazz_discover.iter().map(|t| t.id.as_str()).collect();
        assert!(
            !jazz_disc_ids.iter().any(|id| id.contains("jpop")),
            "Discover Jazz radio leaked J-Pop: {:?}",
            jazz_disc_ids
        );
        assert!(!jazz_disc_ids.is_empty());
        assert!(!jazz_disc_ids.contains(&jazz_fam.id.as_str()), "jazz_disc={:?}", jazz_disc_ids);
        assert!(
            jazz_discover
                .iter()
                .all(|t| !has_genre_clash(t, "", &["jazz".to_string()])),
            "Discover Jazz radio should stay on Jazz: {:?}",
            jazz_disc_ids
        );

        // 4. Playlist-seeded Christmas radio in Familiar & Discover modes: must NOT play J-Pop.
        let pl = store
            .create_playlist(
                u.id,
                "Christmas Classics",
                "Holiday favorites",
                false,
                &[xmas_seed.id.clone()],
            )
            .unwrap();

        let pl_familiar = taste
            .playlist_radio_tracks_with_mode(
                u.id,
                &pl.id,
                &HashSet::from([xmas_seed.id.clone()]),
                &[],
                6,
                RadioMode::Familiar,
            )
            .await;
        let pl_fam_ids: Vec<&str> = pl_familiar.iter().map(|t| t.id.as_str()).collect();
        assert!(!pl_fam_ids.is_empty());
        assert!(
            !pl_fam_ids.iter().any(|id| id.contains("jpop")),
            "Playlist Familiar Christmas radio leaked J-Pop: {:?}",
            pl_fam_ids
        );
        assert!(
            pl_fam_ids.contains(&xmas_fam_1.id.as_str())
                && pl_fam_ids.contains(&xmas_fam_2.id.as_str()),
            "Playlist Familiar should surface familiar Christmas tracks: {:?}",
            pl_fam_ids
        );

        let pl_discover = taste
            .playlist_radio_tracks_with_mode(
                u.id,
                &pl.id,
                &HashSet::from([xmas_seed.id.clone()]),
                &[],
                6,
                RadioMode::Discover,
            )
            .await;
        let pl_disc_ids: Vec<&str> = pl_discover.iter().map(|t| t.id.as_str()).collect();
        assert!(!pl_disc_ids.is_empty());
        assert!(
            !pl_disc_ids.iter().any(|id| id.contains("jpop")),
            "Playlist Discover Christmas radio leaked J-Pop: {:?}",
            pl_disc_ids
        );
    }

    #[tokio::test]
    async fn default_familiar_and_discover_all_fill_18_tracks_even_with_heavy_listening_history() {
        let store = Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("radio18", "pw").unwrap();
        let tmp = std::env::temp_dir().join(format!("beatlab-radio18-{}", uuid::Uuid::new_v4()));
        let sources = SourceRegistry::new(
            crate::sources::deezer::DeezerSource::new(),
            crate::sources::ytm::YtmSource::new(None),
            crate::sources::archive::ArchiveSource::new(),
            Arc::new(crate::sources::local::LocalSource::new(store.clone(), vec![])),
            crate::sources::soulseek::SoulseekSource::new(store.clone(), tmp.join("slsk")),
        );
        let taste = TasteEngine::new(store.clone(), sources);

        let mk = |id: &str, title: &str, artist: &str, genre: &str| Track {
            id: id.into(),
            source: "local".into(),
            title: title.into(),
            artist: artist.into(),
            album: format!("{artist} Album"),
            genre: genre.into(),
            moods: "upbeat".into(),
            duration: 210,
            format: "flac".into(),
            bitrate: 900,
            ..Default::default()
        };

        let seed = mk("loc:seed", "Neon Drive", "Synth Artist 0", "Electronic");
        let mut tracks = vec![seed.clone()];
        // 20 familiar Electronic tracks across 10 artists (including a cross-source duplicate).
        for i in 0..20 {
            tracks.push(mk(
                &format!("loc:fam-{i}"),
                &format!("Familiar Track {i}"),
                &format!("Synth Artist {}", (i % 10) + 1),
                "Electronic",
            ));
        }
        // Cross-source copy of Familiar Track 0 (same song_key, different ID) — Discover must skip it!
        tracks.push(mk(
            "loc:fam-0-alt",
            "Familiar Track 0",
            "Synth Artist 1",
            "Electronic",
        ));
        // 22 unheard Electronic tracks across 8 artists (2-3 songs per artist).
        for i in 0..22 {
            tracks.push(mk(
                &format!("loc:new-{i}"),
                &format!("Fresh Discovery {i}"),
                &format!("New Synth {}", (i % 8) + 1),
                "Electronic",
            ));
        }
        // Off-genre tracks that must NOT leak into Electronic radio.
        for i in 0..10 {
            tracks.push(mk(
                &format!("loc:metal-{i}"),
                &format!("Brutal Riff {i}"),
                &format!("Metal Band {i}"),
                "Heavy Metal",
            ));
        }
        store.upsert_tracks(&tracks).unwrap();

        for i in 0..20 {
            let tid = format!("loc:fam-{i}");
            taste.record_event(u.id, &tid, "play", 1.0);
            taste.record_event(u.id, &tid, "complete", 1.0);
            store.star_track(u.id, &tid).unwrap();
        }
        taste.mark_taste_dirty(u.id);

        // 1. Track-seeded Default, Familiar, and Discover MUST all return 18 tracks.
        let def = taste
            .radio_next_tracks_with_mode(u.id, Some(&seed), &HashSet::new(), &[], 18, RadioMode::Default)
            .await;
        let fam = taste
            .radio_next_tracks_with_mode(u.id, Some(&seed), &HashSet::new(), &[], 18, RadioMode::Familiar)
            .await;
        let disc = taste
            .radio_next_tracks_with_mode(u.id, Some(&seed), &HashSet::new(), &[], 18, RadioMode::Discover)
            .await;

        assert_eq!(def.len(), 18, "Default should fill 18 upcoming tracks");
        assert_eq!(fam.len(), 18, "Familiar should fill 18 upcoming tracks");
        assert_eq!(disc.len(), 18, "Discover should fill 18 upcoming tracks");

        // Discover must exclude familiar tracks, cross-source duplicates of familiar tracks, and Heavy Metal.
        for t in &disc {
            assert!(
                !t.id.starts_with("loc:fam-"),
                "Discover leaked familiar track (or cross-source dup): {}",
                t.id
            );
            assert!(
                !t.id.starts_with("loc:metal-"),
                "Discover leaked off-genre track: {}",
                t.id
            );
        }
        // Familiar must surface familiar tracks at the top.
        let fam_familiar_count = fam.iter().filter(|t| t.id.starts_with("loc:fam-")).count();
        assert!(
            fam_familiar_count >= 12,
            "Familiar should prioritize familiar tracks, got {fam_familiar_count}"
        );

        // 2. Playlist-seeded Default, Familiar, and Discover MUST all return 18 tracks.
        let pl = store
            .create_playlist(
                u.id,
                "Synth Nights",
                "Electronic vibes",
                false,
                &[seed.id.clone(), "loc:fam-0".into(), "loc:fam-1".into()],
            )
            .unwrap();
        let excl = HashSet::from([seed.id.clone()]);
        let pl_def = taste
            .playlist_radio_tracks_with_mode(u.id, &pl.id, &excl, &[], 18, RadioMode::Default)
            .await;
        let pl_fam = taste
            .playlist_radio_tracks_with_mode(u.id, &pl.id, &excl, &[], 18, RadioMode::Familiar)
            .await;
        let pl_disc = taste
            .playlist_radio_tracks_with_mode(u.id, &pl.id, &excl, &[], 18, RadioMode::Discover)
            .await;

        assert_eq!(pl_def.len(), 18, "Playlist Default should fill 18 tracks");
        assert_eq!(pl_fam.len(), 18, "Playlist Familiar should fill 18 tracks");
        assert_eq!(pl_disc.len(), 18, "Playlist Discover should fill 18 tracks");
        for t in &pl_disc {
            assert!(
                !t.id.starts_with("loc:fam-"),
                "Playlist Discover leaked familiar track: {}",
                t.id
            );
        }

        // 3. Seedless (no active seed) Default, Familiar, and Discover MUST also fill 18 tracks.
        let sl_def = taste
            .radio_next_tracks_with_mode(u.id, None, &HashSet::new(), &[], 18, RadioMode::Default)
            .await;
        let sl_fam = taste
            .radio_next_tracks_with_mode(u.id, None, &HashSet::new(), &[], 18, RadioMode::Familiar)
            .await;
        let sl_disc = taste
            .radio_next_tracks_with_mode(u.id, None, &HashSet::new(), &[], 18, RadioMode::Discover)
            .await;
        assert_eq!(sl_def.len(), 18, "Seedless Default should fill 18 tracks");
        assert_eq!(sl_fam.len(), 18, "Seedless Familiar should fill 18 tracks");
        assert_eq!(sl_disc.len(), 18, "Seedless Discover should fill 18 tracks");
        for t in &sl_disc {
            assert!(
                !t.id.starts_with("loc:fam-"),
                "Seedless Discover leaked familiar track: {}",
                t.id
            );
        }
    }

    #[test]
    fn artist_album_ids_are_deduplicated_and_bounded() {
        let mk = |id: &str| AlbumSummary {
            id: id.to_string(),
            deezer_id: id.to_string(),
            name: id.to_string(),
            ..Default::default()
        };
        let albums = vec![
            mk("10"),
            mk("0"),
            mk(" "),
            mk("10"),
            mk("22"),
            mk("33"),
        ];
        assert_eq!(artist_album_ids(&albums, 2), vec!["10", "22"]);
        assert_eq!(artist_album_ids(&albums, 10), vec!["10", "22", "33"]);
    }

    #[tokio::test]
    async fn fresh_account_is_isolated_from_cache_and_other_users() {
        let store = Store::open_memory().unwrap();
        let (veteran, _) = store.authenticate_or_create_user("veteran", "pw").unwrap();
        let (fresh, _) = store.authenticate_or_create_user("fresh_test", "pw").unwrap();

        let sources = SourceRegistry::new(
            crate::sources::deezer::DeezerSource::new(),
            crate::sources::ytm::YtmSource::new(None),
            crate::sources::archive::ArchiveSource::new(),
            Arc::new(crate::sources::local::LocalSource::new(store.clone(), vec![])),
            crate::sources::soulseek::SoulseekSource::new(store.clone(), std::env::temp_dir()),
        );
        let taste = TasteEngine::new(store.clone(), sources);

        // 1. Seed external Deezer charts (tracks + artists + curated mood pool).
        let mut chart_tracks = Vec::new();
        let mut chart_artists = Vec::new();
        for i in 0..20 {
            chart_tracks.push(Track {
                id: format!("dz:chart_{i}"),
                source: "deezer".into(),
                title: format!("Popular Hit {i}"),
                artist: format!("Chart Star {i}"),
                album: format!("Top Album {i}"),
                genre: "Pop".into(),
                moods: "upbeat".into(),
                cover_url: format!("https://example.com/chart_{i}.jpg"),
                duration: 200,
                bitrate: 256,
                stream_url: format!("resolve:ytm:Chart Star {i}|Popular Hit {i}|200"),
                meta: format!(r#"{{"rank":{},"deezer_artist_id":{}}}"#, 900_000 - i * 1000, 100 + i),
                ..Default::default()
            });
            chart_artists.push(ArtistSummary {
                id: format!("ar:dz:{}", 100 + i),
                name: format!("Chart Star {i}"),
                cover_url: format!("https://example.com/artist_{i}.jpg"),
                bio: "500000 fans on Deezer".into(),
                deezer_id: format!("{}", 100 + i),
                album_count: 1,
                song_count: 1,
            });
        }
        store.upsert_tracks(&chart_tracks).unwrap();
        store.record_chart_tracks(&chart_tracks, 1).unwrap();
        for a in &chart_artists {
            store.upsert_artist_meta(a).unwrap();
        }
        store.record_chart_artists(&chart_artists);

        // External curated Energy pool (separate from veteran's cached rock/metal tracks).
        let curated_energy: Vec<Track> = (0..18)
            .map(|i| Track {
                id: format!("dz:cur_energy_{i}"),
                source: "deezer".into(),
                title: format!("Anthem {i}"),
                artist: format!("Stadium Band {i}"),
                album: "Anthems".into(),
                genre: "Rock".into(),
                moods: "energy".into(),
                cover_url: format!("https://example.com/cur_e_{i}.jpg"),
                duration: 210,
                bitrate: 256,
                stream_url: format!("dz:cur_energy_{i}"),
                meta: format!(r#"{{"rank":{}}}"#, 800_000 - i * 1000),
                ..Default::default()
            })
            .collect();
        store.upsert_tracks(&curated_energy).unwrap();
        let curated_energy_ids: Vec<String> = curated_energy.iter().map(|t| t.id.clone()).collect();
        taste.save_curated_pool(&mood_cache_key(Mood::Energy), &curated_energy_ids);

        // 2. Veteran user heavily listens to & caches niche local/Deezer tracks and hydrates an artist
        //    with lots of songs/albums (which previously dominated featured_artist_summaries & tracks_by_mood).
        let veteran_artist = ArtistSummary {
            id: "ar:dz:999999".into(),
            name: "Veteran Niche Metal Act".into(),
            cover_url: "https://example.com/vet_artist.jpg".into(),
            bio: "9000000 fans on Deezer".into(),
            deezer_id: "999999".into(),
            album_count: 12,
            song_count: 30,
        };
        store.upsert_artist_meta(&veteran_artist).unwrap();
        let mut veteran_tracks = Vec::new();
        for i in 0..25 {
            let t = Track {
                id: format!("local:vet_{i}"),
                source: "local".into(),
                title: format!("Niche Riff {i}"),
                artist: "Veteran Niche Metal Act".into(),
                album: format!("Niche Album {}", i % 10),
                genre: "Heavy Metal, Rock".into(),
                moods: "energy".into(),
                cover_url: format!("https://example.com/vet_{i}.jpg"),
                duration: 240,
                bitrate: 1411,
                format: "flac".into(),
                stream_url: format!("file:/music/vet_{i}.flac"),
                meta: r#"{"deezer_artist_id":999999,"rank":999999}"#.into(),
                created: 999_999 + i as i64,
                ..Default::default()
            };
            veteran_tracks.push(t);
        }
        store.upsert_tracks(&veteran_tracks).unwrap();
        for t in &veteran_tracks {
            store.record_event(veteran.id, &t.id, "play", 1.0).unwrap();
            store.record_event(veteran.id, &t.id, "complete", 1.0).unwrap();
        }
        // Train the shared neural model on the veteran user's history.
        let _ = taste.train_and_distill_background(12);

        // 3. Verify fresh account is completely isolated from cache & veteran user's preferences.
        assert!(!store.has_listening_history(fresh.id).unwrap());

        // 3a. Neural blend weight must be 0 for a fresh account.
        let fresh_sctx = taste.build_scoring_context(fresh.id);
        assert_eq!(taste.neural_blend_weight(0.85, &fresh_sctx), 0.0);

        // 3b. Recommended artists must come strictly from external charts, never the veteran's hydrated artist.
        let rec_artists = taste.recommended_artists_for_user(fresh.id, 12);
        assert!(!rec_artists.is_empty());
        assert!(
            !rec_artists.iter().any(|a| a.id == veteran_artist.id || a.name == veteran_artist.name),
            "fresh account recommended_artists leaked veteran's cached artist: {:?}",
            rec_artists.iter().map(|a| &a.name).collect::<Vec<_>>()
        );

        // 3c. Mood mix & Mixed for you must never pull veteran's cached local/tracks_by_mood items.
        let energy_mix = taste.mood_mix_tracks(fresh.id, Mood::Energy, 30).await;
        assert!(!energy_mix.is_empty());
        assert!(
            !energy_mix.iter().any(|t| t.id.starts_with("local:vet_")),
            "fresh account mood_mix_tracks leaked veteran's cached tracks: {:?}",
            energy_mix.iter().map(|t| &t.id).collect::<Vec<_>>()
        );

        taste.rebuild_mixed_for_you(fresh.id).await;
        let cards = taste.mixed_for_you_cards(fresh.id);
        assert!(!cards.is_empty());
        for card in &cards {
            let tracks = store.playlist_tracks(&card.id).unwrap();
            assert!(
                !tracks.iter().any(|t| t.id.starts_with("local:vet_")),
                "fresh account Mixed for you ({}) leaked veteran's cached tracks",
                card.title
            );
        }

        // 3d. Mood preview for a mood without a curated pool must not leak cached tracks_by_mood.
        let (preview_cnt, preview_covers) =
            taste.curated_mood_preview_for_user(fresh.id, Mood::LateNight);
        assert_eq!(preview_cnt, 0);
        assert!(preview_covers.is_empty());

        // 3e. Quick picks, Discover Weekly, Related Tracks must only return popular chart tracks.
        for list in [
            taste.quick_picks(fresh.id, 12),
            taste.discover_weekly(fresh.id, 12),
            taste.related_tracks_for_user(fresh.id, 12),
        ] {
            assert!(!list.is_empty());
            assert!(
                list.iter().all(|t| t.id.starts_with("dz:chart_")),
                "fresh account feed leaked non-chart tracks: {:?}",
                list.iter().map(|t| &t.id).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn search_ranks_exact_title_phrase_first_even_when_user_heavily_listens_to_query_artist() {
        let store = Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("reona_fan", "pw").unwrap();
        let sources = SourceRegistry::new(
            crate::sources::deezer::DeezerSource::new(),
            crate::sources::ytm::YtmSource::new(None),
            crate::sources::archive::ArchiveSource::new(),
            Arc::new(crate::sources::local::LocalSource::new(store.clone(), vec![])),
            crate::sources::soulseek::SoulseekSource::new(store.clone(), std::env::temp_dir()),
        );
        let taste = TasteEngine::new(store.clone(), sources);

        let amore = Track {
            id: "ytm:amore".into(),
            source: "ytm".into(),
            title: "Amore".into(),
            artist: "ReoNa".into(),
            album: "Amore".into(),
            genre: "J-Pop".into(),
            bitrate: 256,
            meta: r#"{"rank":820000}"#.into(),
            ..Default::default()
        };
        let lost = Track {
            id: "ytm:lost".into(),
            source: "ytm".into(),
            title: "Lost -Game size-".into(),
            artist: "ReoNa".into(),
            album: "TSUKIHIME".into(),
            genre: "J-Pop".into(),
            bitrate: 256,
            meta: r#"{"rank":700000}"#.into(),
            ..Default::default()
        };
        let game_of_love = Track {
            id: "ytm:gol".into(),
            source: "ytm".into(),
            title: "Game of Love".into(),
            artist: "Elza Kanzaki".into(),
            album: "ELZA2".into(),
            genre: "J-Pop".into(),
            bitrate: 256,
            meta: r#"{"rank":120000}"#.into(),
            ..Default::default()
        };
        store
            .upsert_tracks(&[amore.clone(), lost.clone(), game_of_love.clone()])
            .unwrap();

        // User heavily listens to ReoNa (even stars Amore).
        store.star_track(u.id, &amore.id).unwrap();
        for _ in 0..25 {
            store.record_event(u.id, &amore.id, "complete", 1.0).unwrap();
            store.record_event(u.id, &lost.id, "complete", 1.0).unwrap();
        }

        let res = taste.search_and_refine(
            u.id,
            "game of love reona",
            vec![amore.clone(), lost.clone(), game_of_love.clone()],
            vec![],
        );
        assert_eq!(
            res.tracks.first().map(|t| t.id.as_str()),
            Some("ytm:gol"),
            "exact title match 'Game of Love' must rank #1, got: {:?}",
            res.tracks
                .iter()
                .map(|t| (&t.id, &t.title, &t.artist))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn search_prioritizes_user_listened_artist_and_genre_over_more_popular_pop_artist() {
        let store = Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("listener", "pw").unwrap();
        let sources = SourceRegistry::new(
            crate::sources::deezer::DeezerSource::new(),
            crate::sources::ytm::YtmSource::new(None),
            crate::sources::archive::ArchiveSource::new(),
            Arc::new(crate::sources::local::LocalSource::new(store.clone(), vec![])),
            crate::sources::soulseek::SoulseekSource::new(store.clone(), std::env::temp_dir()),
        );
        let taste = TasteEngine::new(store.clone(), sources);

        // User listens a ton to Artist 123 (on YTM) and J-Rock in general.
        let past_track = Track {
            id: "ytm:a123_prev".into(),
            source: "ytm".into(),
            title: "Previous Favorite".into(),
            artist: "Artist 123".into(),
            album: "First Album".into(),
            genre: "J-Rock".into(),
            meta: r#"{"ytm_artist_id":"UC_a123"}"#.into(),
            ..Default::default()
        };
        // Search result for Artist 123 comes from Deezer (different source ID) and is NOT popular.
        let a123_stay = Track {
            id: "dz:a123_stay".into(),
            source: "deezer".into(),
            title: "Stay".into(),
            artist: "Artist 123".into(),
            album: "Second Album".into(),
            genre: "J-Rock".into(),
            bitrate: 256,
            meta: r#"{"deezer_artist_id":"99123","rank":80000}"#.into(),
            ..Default::default()
        };
        // Mega-popular mainstream pop song with the exact same title.
        let pop_stay = Track {
            id: "dz:pop876_stay".into(),
            source: "deezer".into(),
            title: "Stay".into(),
            artist: "Pop Artist 876".into(),
            album: "Top 40 Hits".into(),
            genre: "Pop".into(),
            bitrate: 320,
            meta: r#"{"deezer_artist_id":"99876","rank":995000}"#.into(),
            ..Default::default()
        };
        // Another unheard J-Rock artist with the same title (tests genre-taste preference).
        let jrock_other = Track {
            id: "dz:jrock_new".into(),
            source: "deezer".into(),
            title: "Stay".into(),
            artist: "Unheard J-Rock Band".into(),
            album: "Indie EP".into(),
            genre: "J-Rock".into(),
            bitrate: 256,
            meta: r#"{"deezer_artist_id":"99555","rank":100000}"#.into(),
            ..Default::default()
        };
        store
            .upsert_tracks(&[
                past_track.clone(),
                a123_stay.clone(),
                pop_stay.clone(),
                jrock_other.clone(),
            ])
            .unwrap();

        for _ in 0..8 {
            store
                .record_event(u.id, &past_track.id, "complete", 1.0)
                .unwrap();
        }

        // 1. User's listened artist ranks #1 over mega-popular pop artist.
        let res = taste.search_and_refine(
            u.id,
            "stay",
            vec![pop_stay.clone(), a123_stay.clone()],
            vec![],
        );
        assert_eq!(
            res.tracks.first().map(|t| t.id.as_str()),
            Some("dz:a123_stay"),
            "user's heavily listened artist should outrank popular pop artist, got: {:?}",
            res.tracks
                .iter()
                .map(|t| (&t.id, &t.artist))
                .collect::<Vec<_>>()
        );

        // 2. Even when remote search only returns an unheard J-Rock artist and a mega-popular
        //    pop hit, the catalog pulls in the user's listened artist (#1) and the preferred
        //    J-Rock genre (#2) still outranks the unrelated popular pop hit (#3).
        let res_genre = taste.search_and_refine(
            u.id,
            "stay",
            vec![pop_stay.clone(), jrock_other.clone()],
            vec![],
        );
        let ids: Vec<&str> = res_genre.tracks.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["dz:a123_stay", "dz:jrock_new", "dz:pop876_stay"],
            "expected listened artist #1, preferred genre #2, popular pop #3, got: {:?}",
            res_genre
                .tracks
                .iter()
                .map(|t| (&t.id, &t.artist, &t.genre))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn search_prioritizes_listened_dual_script_title_over_unrelated_exact_matches() {
        let store = Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("cross", "pw").unwrap();
        let sources = SourceRegistry::new(
            crate::sources::deezer::DeezerSource::new(),
            crate::sources::ytm::YtmSource::new(None),
            crate::sources::archive::ArchiveSource::new(),
            Arc::new(crate::sources::local::LocalSource::new(store.clone(), vec![])),
            crate::sources::soulseek::SoulseekSource::new(store.clone(), std::env::temp_dir()),
        );
        let taste = TasteEngine::new(store.clone(), sources);

        let mk = |id: &str, title: &str, artist: &str, album: &str| Track {
            id: id.into(),
            source: "ytm".into(),
            title: title.into(),
            artist: artist.into(),
            album: album.into(),
            bitrate: 256,
            ..Default::default()
        };

        let mfs_mugen = mk("ytm:mfs_mugen", "夢幻 - MUGEN", "MY FIRST STORY, HYDE", "MUGEN");
        let hyde_tokoshie = mk(
            "ytm:tokoshie",
            "永久 -トコシエ- - Tokoshie",
            "HYDE, MY FIRST STORY",
            "Tokoshie",
        );
        let genfukei = mk("ytm:gen", "Mugen", "Genfukei", "Mugen");
        let danie = mk("ytm:danie", "Mugen", "Danie Green", "Mugen");
        let cover1 = mk(
            "ytm:cov1",
            "Mugen (Cover)",
            "Brianna Knickerbocker",
            "Mugen (Cover)",
        );
        let cover2 = mk(
            "ytm:cov2",
            "MUGEN (English Cover)",
            "Seishirou Scarlatti",
            "MUGEN (English Cover)",
        );
        let mfs_solo = mk("ytm:mfs_solo", "I'm a mess", "MY FIRST STORY", "I'm a mess");
        store
            .upsert_tracks(&[
                mfs_mugen.clone(),
                hyde_tokoshie.clone(),
                genfukei.clone(),
                danie.clone(),
                cover1.clone(),
                cover2.clone(),
                mfs_solo.clone(),
            ])
            .unwrap();

        // Scenario A: User starred and listens to 夢幻 - MUGEN.
        store.star_track(u.id, &mfs_mugen.id).unwrap();
        store.star_track(u.id, &hyde_tokoshie.id).unwrap();
        for _ in 0..5 {
            store.record_event(u.id, &mfs_mugen.id, "complete", 1.0).unwrap();
        }

        let res = taste.search_and_refine(
            u.id,
            "mugen",
            vec![
                genfukei.clone(),
                danie.clone(),
                cover1.clone(),
                cover2.clone(),
                mfs_mugen.clone(),
                hyde_tokoshie.clone(),
            ],
            vec![],
        );
        let ids: Vec<&str> = res.tracks.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(
            ids.first().copied(),
            Some("ytm:mfs_mugen"),
            "listened dual-script song '夢幻 - MUGEN' must rank #1 for 'mugen', got: {:?}",
            ids
        );
        let pos_gen = ids.iter().position(|&id| id == "ytm:gen").unwrap();
        let pos_cov1 = ids.iter().position(|&id| id == "ytm:cov1").unwrap();
        let pos_cov2 = ids.iter().position(|&id| id == "ytm:cov2").unwrap();
        assert!(
            pos_gen < pos_cov1 && pos_gen < pos_cov2,
            "original 'Mugen' must outrank unrequested covers: {:?}",
            ids
        );

        // Searching "tokoshie" also puts 永久 -トコシエ- - Tokoshie #1.
        let tok_res = taste.search_and_refine(
            u.id,
            "tokoshie",
            vec![genfukei.clone(), hyde_tokoshie.clone()],
            vec![],
        );
        assert_eq!(
            tok_res.tracks.first().map(|t| t.id.as_str()),
            Some("ytm:tokoshie")
        );

        // Scenario B: A user who only listens to solo "MY FIRST STORY" (never played the collab)
        // still gets "夢幻 - MUGEN" by "MY FIRST STORY, HYDE" at #1 via collab artist affinity.
        let (u2, _) = store.authenticate_or_create_user("mfs_fan", "pw").unwrap();
        for _ in 0..6 {
            store.record_event(u2.id, &mfs_solo.id, "complete", 1.0).unwrap();
        }
        let res_collab = taste.search_and_refine(
            u2.id,
            "mugen",
            vec![genfukei.clone(), danie.clone(), mfs_mugen.clone()],
            vec![],
        );
        assert_eq!(
            res_collab.tracks.first().map(|t| t.id.as_str()),
            Some("ytm:mfs_mugen"),
            "solo artist affinity should carry over to 'MY FIRST STORY, HYDE' collab: {:?}",
            res_collab
                .tracks
                .iter()
                .map(|t| (&t.id, &t.title, &t.artist))
                .collect::<Vec<_>>()
        );
    }
}
