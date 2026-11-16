use crate::models::{primary_artist, Mood, QualityTier, Track, User, UserEventRow};
use crate::store::Store;
use crate::taste::{classify_moods_str, split_genres};
use chrono::{Datelike, DateTime, Duration, FixedOffset, NaiveDate, TimeZone, Timelike, Utc};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatsRange {
    Days7,
    Days30,
    Days90,
    Days365,
    All,
}

impl StatsRange {
    pub fn from_str(raw: Option<&str>) -> Self {
        match raw.unwrap_or("all").trim().to_ascii_lowercase().as_str() {
            "7d" | "7" | "week" => Self::Days7,
            "30d" | "30" | "month" => Self::Days30,
            "90d" | "90" | "quarter" => Self::Days90,
            "365d" | "365" | "1y" | "year" => Self::Days365,
            _ => Self::All,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Days7 => "7d",
            Self::Days30 => "30d",
            Self::Days90 => "90d",
            Self::Days365 => "365d",
            Self::All => "all",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Days7 => "Last 7 Days",
            Self::Days30 => "Last 30 Days",
            Self::Days90 => "Last 90 Days",
            Self::Days365 => "Last 12 Months",
            Self::All => "All Time",
        }
    }

    pub fn days(self) -> Option<i64> {
        match self {
            Self::Days7 => Some(7),
            Self::Days30 => Some(30),
            Self::Days90 => Some(90),
            Self::Days365 => Some(365),
            Self::All => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ListenRecord {
    pub song_key: String,
    pub start_at: i64,
    pub end_at: Option<i64>,
    pub completed: bool,
    pub skipped: bool,
    pub listened_secs: i64,
    pub track: Option<Track>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryStats {
    pub total_streams: usize,
    pub completed_streams: usize,
    pub skipped_streams: usize,
    pub partial_streams: usize,
    pub completion_rate: f64,
    pub skip_rate: f64,
    pub total_listening_secs: i64,
    pub avg_daily_secs: i64,
    pub avg_streams_per_active_day: f64,
    pub avg_track_duration_secs: i64,
    pub unique_tracks: usize,
    pub unique_artists: usize,
    pub unique_albums: usize,
    pub repeat_ratio: f64,
    pub active_days: usize,
    pub span_days: usize,
    pub new_tracks_discovered: usize,
    pub new_artists_discovered: usize,
    pub discovery_rate_pct: f64,
    pub lossless_streams: usize,
    pub lossless_pct: f64,
    pub avg_bitrate: i64,
    pub favorite_source: String,
    pub favorite_source_label: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BestDayStat {
    pub date: String,
    pub streams: usize,
    pub secs: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreakStats {
    pub current_days: usize,
    pub longest_days: usize,
    pub streak_active_today: bool,
    pub active_days_all_time: usize,
    pub best_day: Option<BestDayStat>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignalsInRange {
    pub stars: usize,
    pub playlist_adds: usize,
    pub search_picks: usize,
    pub mix_plays: usize,
    pub ratings: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryStats {
    pub liked_tracks: usize,
    pub saved_albums: usize,
    pub followed_artists: usize,
    pub manual_playlists: usize,
    pub auto_playlists: usize,
    pub curated_tracks: usize,
    pub hidden_tracks: usize,
    pub rated_tracks: usize,
    pub avg_rating: f64,
    pub ratings_dist: [usize; 5],
    pub signals_in_range: SignalsInRange,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonaStats {
    pub title: String,
    pub chronotype: String,
    pub chronotype_desc: String,
    pub style: String,
    pub headline: String,
    pub badges: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelinePoint {
    pub key: String,
    pub label: String,
    pub short_label: String,
    pub streams: usize,
    pub completed: usize,
    pub skipped: usize,
    pub secs: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineStats {
    pub granularity: String,
    pub points: Vec<TimelinePoint>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeatmapDay {
    pub date: String,
    pub streams: usize,
    pub secs: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeatmapStats {
    pub start_date: String,
    pub end_date: String,
    pub max_streams: usize,
    pub days: Vec<HeatmapDay>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HourStat {
    pub hour: usize,
    pub label: String,
    pub streams: usize,
    pub secs: i64,
    pub pct: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WeekdayStat {
    pub day: String,
    pub full_day: String,
    pub streams: usize,
    pub secs: i64,
    pub pct: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MoodStat {
    pub slug: String,
    pub label: String,
    pub c1: String,
    pub c2: String,
    pub streams: usize,
    pub secs: i64,
    pub pct: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenreStat {
    pub name: String,
    pub streams: usize,
    pub secs: i64,
    pub pct: f64,
    pub unique_tracks: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QualityTierStat {
    pub tier: String,
    pub label: String,
    pub color: String,
    pub streams: usize,
    pub secs: i64,
    pub pct: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyCountStat {
    pub key: String,
    pub label: String,
    pub streams: usize,
    pub secs: i64,
    pub pct: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DecadeStat {
    pub decade: i32,
    pub label: String,
    pub streams: usize,
    pub secs: i64,
    pub pct: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct YearTrackRef {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub year: i32,
}

#[derive(Debug, Clone)]
pub struct TopTrackStat {
    pub track: Track,
    pub streams: usize,
    pub completed: usize,
    pub skips: usize,
    pub listened_secs: i64,
    pub last_played: i64,
}

#[derive(Debug, Clone)]
pub struct TopArtistStat {
    pub id: String,
    pub name: String,
    pub streams: usize,
    pub listened_secs: i64,
    pub unique_tracks: usize,
    pub pct: f64,
    pub is_followed: bool,
}

#[derive(Debug, Clone)]
pub struct TopAlbumStat {
    pub id: String,
    pub name: String,
    pub artist: String,
    pub artist_id: String,
    pub year: i32,
    pub streams: usize,
    pub listened_secs: i64,
    pub unique_tracks: usize,
    pub is_saved: bool,
}

#[derive(Debug, Clone)]
pub struct SkippedTrackStat {
    pub track: Track,
    pub skips: usize,
    pub streams: usize,
}

#[derive(Debug, Clone)]
pub struct HighlightsData {
    pub longest_track: Option<Track>,
    pub shortest_track: Option<Track>,
    pub first_track_ever: Option<(Track, i64)>,
    pub most_completed_track: Option<(Track, usize)>,
}

#[derive(Debug, Clone)]
pub struct RecentActivityItem {
    pub kind: String,
    pub label: String,
    pub at: i64,
    pub value: f64,
    pub track: Track,
}

#[derive(Debug, Clone)]
pub struct ComputedStats {
    pub range: StatsRange,
    pub tz_offset_min: i32,
    pub generated_at: i64,
    pub member_since: i64,
    pub first_listen_at: Option<i64>,
    pub last_listen_at: Option<i64>,
    pub summary: SummaryStats,
    pub streaks: StreakStats,
    pub library: LibraryStats,
    pub persona: PersonaStats,
    pub timeline: TimelineStats,
    pub heatmap: HeatmapStats,
    pub hourly: Vec<HourStat>,
    pub peak_hour: Option<usize>,
    pub weekdays: Vec<WeekdayStat>,
    pub peak_weekday: Option<String>,
    pub moods: Vec<MoodStat>,
    pub genres: Vec<GenreStat>,
    pub quality_tiers: Vec<QualityTierStat>,
    pub formats: Vec<KeyCountStat>,
    pub sources: Vec<KeyCountStat>,
    pub decades: Vec<DecadeStat>,
    pub oldest_track: Option<YearTrackRef>,
    pub newest_track: Option<YearTrackRef>,
    pub top_tracks: Vec<TopTrackStat>,
    pub top_artists: Vec<TopArtistStat>,
    pub top_albums: Vec<TopAlbumStat>,
    pub most_skipped: Vec<SkippedTrackStat>,
    pub highlights: HighlightsData,
    pub recent_events: Vec<RecentActivityItem>,
}

pub fn build_user_stats(
    store: &Store,
    user: &User,
    range: StatsRange,
    tz_offset_min: i32,
    now_ts: i64,
) -> ComputedStats {
    let clamped_tz = tz_offset_min.clamp(-720, 840);
    let offset = FixedOffset::east_opt(clamped_tz * 60)
        .unwrap_or_else(|| FixedOffset::east_opt(0).unwrap());

    let events = store.user_events_with_tracks(user.id).unwrap_or_default();
    let starred = store.starred_ids(user.id).unwrap_or_default();
    let saved_albums = store.saved_album_ids(user.id).unwrap_or_default();
    let followed_artists = store.followed_artist_ids(user.id).unwrap_or_default();
    let ratings = store.user_ratings(user.id).unwrap_or_default();
    let playlists = store.list_playlists(user.id, false).unwrap_or_default();
    let (hidden_ids, _) = store.hidden_track_sets(user.id).unwrap_or_default();

    compute_stats_from_data(
        user,
        &events,
        &starred,
        &saved_albums,
        &followed_artists,
        &ratings,
        &playlists,
        hidden_ids.len(),
        range,
        clamped_tz,
        offset,
        now_ts,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn compute_stats_from_data(
    user: &User,
    events: &[UserEventRow],
    starred: &HashSet<String>,
    saved_albums: &HashSet<String>,
    followed_artists: &HashSet<String>,
    ratings: &HashMap<String, i32>,
    playlists: &[crate::models::Playlist],
    hidden_count: usize,
    range: StatsRange,
    tz_offset_min: i32,
    offset: FixedOffset,
    now_ts: i64,
) -> ComputedStats {
    let now_local = ts_to_local(now_ts, offset);
    let today_local = now_local.date_naive();

    let range_start_date = range
        .days()
        .map(|d| today_local - Duration::days(d - 1));
    let range_start_ts = range_start_date.and_then(|d| {
        d.and_hms_opt(0, 0, 0)
            .and_then(|ndt| offset.from_local_datetime(&ndt).single())
            .map(|dt| dt.timestamp())
    });

    // Build canonical track lookup across all events so any referenced track retains metadata.
    let mut track_lookup: HashMap<String, Track> = HashMap::new();
    for ev in events {
        if let Some(ref t) = ev.track {
            track_lookup.entry(ev.track_id.clone()).or_insert_with(|| t.clone());
        }
    }

    // Reconstruct chronological streams/listens across ALL history so play->complete pairing
    // and all-time streaks / discovery baselines are accurate.
    let (listens, standalone_skips) = reconstruct_listens(events, &track_lookup);

    let first_listen_at = listens.first().map(|l| l.start_at);
    let last_listen_at = listens.last().map(|l| l.start_at);

    let mut first_heard_track: HashMap<String, i64> = HashMap::new();
    let mut first_heard_artist: HashMap<String, i64> = HashMap::new();
    let mut all_days: BTreeMap<NaiveDate, (usize, i64)> = BTreeMap::new();

    for l in &listens {
        first_heard_track.entry(l.song_key.clone()).or_insert(l.start_at);
        if let Some(ref t) = l.track {
            first_heard_artist.entry(t.artist_id()).or_insert(l.start_at);
        }
        let d = ts_to_local(l.start_at, offset).date_naive();
        let entry = all_days.entry(d).or_insert((0, 0));
        entry.0 += 1;
        entry.1 += l.listened_secs;
    }

    let streaks = compute_streaks(&all_days, today_local);
    let heatmap = compute_heatmap(&all_days, today_local);

    // Filter listens & standalone skips to the selected range.
    let filtered_listens: Vec<&ListenRecord> = listens
        .iter()
        .filter(|l| range_start_ts.map_or(true, |start| l.start_at >= start))
        .collect();
    let filtered_standalone_skips: Vec<&(String, i64)> = standalone_skips
        .iter()
        .filter(|(_, at)| range_start_ts.map_or(true, |start| *at >= start))
        .collect();

    // Library counts + range interaction signals.
    let mut manual_playlists = 0usize;
    let mut auto_playlists = 0usize;
    let mut curated_tracks = 0usize;
    for pl in playlists {
        if pl.is_auto {
            auto_playlists += 1;
        } else {
            manual_playlists += 1;
            curated_tracks += pl.track_count;
        }
    }
    let mut ratings_dist = [0usize; 5];
    let mut rating_sum = 0i64;
    for &r in ratings.values() {
        if (1..=5).contains(&r) {
            ratings_dist[(r - 1) as usize] += 1;
            rating_sum += r as i64;
        }
    }
    let rated_tracks = ratings_dist.iter().sum::<usize>();
    let avg_rating = if rated_tracks > 0 {
        round1(rating_sum as f64 / rated_tracks as f64)
    } else {
        0.0
    };

    let mut signals = SignalsInRange {
        stars: 0,
        playlist_adds: 0,
        search_picks: 0,
        mix_plays: 0,
        ratings: 0,
    };
    for ev in events {
        if range_start_ts.map_or(false, |start| ev.at < start) {
            continue;
        }
        match ev.kind.as_str() {
            "star" => signals.stars += 1,
            "playlist_add" => signals.playlist_adds += 1,
            "search_pick" => signals.search_picks += 1,
            "mix_play" => signals.mix_plays += 1,
            "rate" => signals.ratings += 1,
            _ => {}
        }
    }

    let library = LibraryStats {
        liked_tracks: starred.len(),
        saved_albums: saved_albums.len(),
        followed_artists: followed_artists.len(),
        manual_playlists,
        auto_playlists,
        curated_tracks,
        hidden_tracks: hidden_count,
        rated_tracks,
        avg_rating,
        ratings_dist,
        signals_in_range: signals,
    };

    // Aggregate metrics for filtered listens.
    let total_streams = filtered_listens.len();
    let mut completed_streams = 0usize;
    let mut skipped_listens = 0usize;
    let mut partial_streams = 0usize;
    let mut total_listening_secs = 0i64;
    let mut track_dur_sum = 0i64;
    let mut track_dur_count = 0i64;
    let mut bitrate_sum = 0i64;
    let mut bitrate_count = 0i64;
    let mut lossless_streams = 0usize;

    let mut unique_track_keys: HashSet<String> = HashSet::new();
    let mut unique_artist_ids: HashSet<String> = HashSet::new();
    let mut unique_album_ids: HashSet<String> = HashSet::new();
    let mut active_day_set: HashSet<NaiveDate> = HashSet::new();

    let mut hour_counts = vec![(0usize, 0i64); 24];
    let mut weekday_counts = vec![(0usize, 0i64); 7]; // 0 = Mon .. 6 = Sun
    let mut mood_counts: HashMap<String, (usize, i64)> = HashMap::new();
    let mut genre_counts: HashMap<String, GenreAccum> = HashMap::new();
    let mut quality_counts: HashMap<&'static str, (usize, i64)> = HashMap::new();
    let mut format_counts: HashMap<String, (usize, i64)> = HashMap::new();
    let mut source_counts: HashMap<String, (usize, i64)> = HashMap::new();
    let mut decade_counts: BTreeMap<i32, (usize, i64)> = BTreeMap::new();

    struct TrackAccum {
        candidates: Vec<Track>,
        streams: usize,
        completed: usize,
        skips: usize,
        listened_secs: i64,
        last_played: i64,
    }
    struct ArtistAccum {
        name: String,
        streams: usize,
        listened_secs: i64,
        song_keys: HashSet<String>,
    }
    struct AlbumAccum {
        name: String,
        artist: String,
        artist_id: String,
        year: i32,
        streams: usize,
        listened_secs: i64,
        song_keys: HashSet<String>,
    }
    struct GenreAccum {
        display_name: String,
        streams: usize,
        secs: i64,
        song_keys: HashSet<String>,
    }

    let mut track_acc: HashMap<String, TrackAccum> = HashMap::new();
    let mut artist_acc: HashMap<String, ArtistAccum> = HashMap::new();
    let mut album_acc: HashMap<String, AlbumAccum> = HashMap::new();

    let mut oldest_track: Option<YearTrackRef> = None;
    let mut newest_track: Option<YearTrackRef> = None;
    let mut longest_track: Option<Track> = None;
    let mut shortest_track: Option<Track> = None;

    for l in &filtered_listens {
        if l.completed {
            completed_streams += 1;
        } else if l.skipped {
            skipped_listens += 1;
        } else {
            partial_streams += 1;
        }
        total_listening_secs += l.listened_secs;
        unique_track_keys.insert(l.song_key.clone());

        let local_dt = ts_to_local(l.start_at, offset);
        let local_date = local_dt.date_naive();
        active_day_set.insert(local_date);

        let hr = local_dt.hour() as usize;
        if hr < 24 {
            hour_counts[hr].0 += 1;
            hour_counts[hr].1 += l.listened_secs;
        }
        let wd = local_dt.weekday().num_days_from_monday() as usize;
        if wd < 7 {
            weekday_counts[wd].0 += 1;
            weekday_counts[wd].1 += l.listened_secs;
        }

        if let Some(ref t) = l.track {
            if t.duration > 0 {
                track_dur_sum += t.duration as i64;
                track_dur_count += 1;
                if longest_track
                    .as_ref()
                    .map_or(true, |cur| t.duration > cur.duration)
                {
                    longest_track = Some(t.clone());
                }
                if shortest_track
                    .as_ref()
                    .map_or(true, |cur| t.duration < cur.duration)
                {
                    shortest_track = Some(t.clone());
                }
            }
            if t.bitrate > 0 {
                bitrate_sum += t.bitrate as i64;
                bitrate_count += 1;
            }
            if t.is_lossless() {
                lossless_streams += 1;
            }

            let tier_key = match t.quality_tier() {
                QualityTier::HiRes => "hi_res",
                QualityTier::Lossless => "lossless",
                QualityTier::HighLossy => "high_lossy",
                QualityTier::Standard => "standard",
                QualityTier::Low => "low",
            };
            let qt = quality_counts.entry(tier_key).or_insert((0, 0));
            qt.0 += 1;
            qt.1 += l.listened_secs;

            let fmt_key = infer_format_key(t);
            let fe = format_counts.entry(fmt_key).or_insert((0, 0));
            fe.0 += 1;
            fe.1 += l.listened_secs;

            let src_key = if t.source.trim().is_empty() {
                "unknown".to_string()
            } else {
                t.source.trim().to_lowercase()
            };
            let se = source_counts.entry(src_key).or_insert((0, 0));
            se.0 += 1;
            se.1 += l.listened_secs;

            if (1900..=2099).contains(&t.year) {
                let dec = (t.year / 10) * 10;
                let de = decade_counts.entry(dec).or_insert((0, 0));
                de.0 += 1;
                de.1 += l.listened_secs;

                if oldest_track.as_ref().map_or(true, |o| t.year < o.year) {
                    oldest_track = Some(YearTrackRef {
                        id: t.id.clone(),
                        title: t.title.clone(),
                        artist: t.artist.clone(),
                        year: t.year,
                    });
                }
                if newest_track.as_ref().map_or(true, |n| t.year > n.year) {
                    newest_track = Some(YearTrackRef {
                        id: t.id.clone(),
                        title: t.title.clone(),
                        artist: t.artist.clone(),
                        year: t.year,
                    });
                }
            }

            let moods_raw = if t.moods.trim().is_empty() {
                classify_moods_str(&t.title, &t.album, &t.genre, "")
            } else {
                t.moods.clone()
            };
            let mut seen_moods = HashSet::new();
            for m in moods_raw.split(',').filter_map(Mood::from_slug) {
                if seen_moods.insert(m.slug()) {
                    let me = mood_counts.entry(m.slug().to_string()).or_insert((0, 0));
                    me.0 += 1;
                    me.1 += l.listened_secs;
                }
            }

            let mut seen_genres = HashSet::new();
            for g in split_genres(&t.genre) {
                let clean = g.trim();
                if clean.is_empty() || !seen_genres.insert(clean.to_string()) {
                    continue;
                }
                let ge = genre_counts
                    .entry(clean.to_string())
                    .or_insert_with(|| GenreAccum {
                        display_name: title_case_genre(clean),
                        streams: 0,
                        secs: 0,
                        song_keys: HashSet::new(),
                    });
                ge.streams += 1;
                ge.secs += l.listened_secs;
                ge.song_keys.insert(l.song_key.clone());
            }

            let ta = track_acc.entry(l.song_key.clone()).or_insert_with(|| TrackAccum {
                candidates: Vec::new(),
                streams: 0,
                completed: 0,
                skips: 0,
                listened_secs: 0,
                last_played: 0,
            });
            ta.candidates.push(t.clone());
            ta.streams += 1;
            if l.completed {
                ta.completed += 1;
            }
            if l.skipped {
                ta.skips += 1;
            }
            ta.listened_secs += l.listened_secs;
            ta.last_played = ta.last_played.max(l.start_at);

            let aid = t.artist_id();
            unique_artist_ids.insert(aid.clone());
            let disp_artist = {
                let pa = primary_artist(&t.artist);
                if pa.is_empty() {
                    t.artist.trim().to_string()
                } else {
                    pa
                }
            };
            if !disp_artist.is_empty() {
                let aa = artist_acc.entry(aid).or_insert_with(|| ArtistAccum {
                    name: disp_artist,
                    streams: 0,
                    listened_secs: 0,
                    song_keys: HashSet::new(),
                });
                aa.streams += 1;
                aa.listened_secs += l.listened_secs;
                aa.song_keys.insert(l.song_key.clone());
            }

            if !t.album.trim().is_empty() {
                let alid = t.album_id();
                unique_album_ids.insert(alid.clone());
                let alb_artist = {
                    let pa = primary_artist(&t.artist);
                    if pa.is_empty() {
                        t.artist.trim().to_string()
                    } else {
                        pa
                    }
                };
                let ala = album_acc.entry(alid).or_insert_with(|| AlbumAccum {
                    name: t.album.trim().to_string(),
                    artist: alb_artist,
                    artist_id: t.artist_id(),
                    year: t.year,
                    streams: 0,
                    listened_secs: 0,
                    song_keys: HashSet::new(),
                });
                if ala.year <= 0 && t.year > 0 {
                    ala.year = t.year;
                }
                ala.streams += 1;
                ala.listened_secs += l.listened_secs;
                ala.song_keys.insert(l.song_key.clone());
            }
        }
    }

    // Attribute standalone skips (if any) to track_acc for skip leaderboard accuracy.
    for (tid, _) in &filtered_standalone_skips {
        if let Some(t) = track_lookup.get(tid.as_str()) {
            let key = canonical_song_key(t, tid);
            if let Some(ta) = track_acc.get_mut(&key) {
                ta.skips += 1;
            } else {
                track_acc.insert(
                    key,
                    TrackAccum {
                        candidates: vec![t.clone()],
                        streams: 0,
                        completed: 0,
                        skips: 1,
                        listened_secs: 0,
                        last_played: 0,
                    },
                );
            }
        }
    }

    let skipped_streams = skipped_listens + filtered_standalone_skips.len();
    let skip_denom = (total_streams + filtered_standalone_skips.len()).max(1) as f64;
    let completion_rate = if total_streams > 0 {
        round1((completed_streams as f64 / total_streams as f64) * 100.0)
    } else {
        0.0
    };
    let skip_rate = if total_streams + filtered_standalone_skips.len() > 0 {
        round1((skipped_streams as f64 / skip_denom) * 100.0)
    } else {
        0.0
    };

    let active_days = active_day_set.len();
    let span_days = match range_start_date {
        Some(sd) => ((today_local - sd).num_days() + 1).max(1) as usize,
        None => match (first_listen_at, last_listen_at) {
            (Some(f), _) => {
                let fd = ts_to_local(f, offset).date_naive();
                ((today_local - fd).num_days() + 1).max(1) as usize
            }
            (None, _) => 0,
        },
    };
    let avg_daily_secs = if active_days > 0 {
        total_listening_secs / active_days as i64
    } else {
        0
    };
    let avg_streams_per_active_day = if active_days > 0 {
        round1(total_streams as f64 / active_days as f64)
    } else {
        0.0
    };
    let avg_track_duration_secs = if track_dur_count > 0 {
        track_dur_sum / track_dur_count
    } else {
        0
    };
    let unique_tracks = unique_track_keys.len();
    let unique_artists = unique_artist_ids.len();
    let unique_albums = unique_album_ids.len();
    let repeat_ratio = if unique_tracks > 0 {
        (total_streams as f64 / unique_tracks as f64 * 100.0).round() / 100.0
    } else {
        0.0
    };

    let new_tracks_discovered = unique_track_keys
        .iter()
        .filter(|k| {
            first_heard_track
                .get(*k)
                .map_or(false, |&first_at| range_start_ts.map_or(true, |s| first_at >= s))
        })
        .count();
    let new_artists_discovered = unique_artist_ids
        .iter()
        .filter(|a| {
            first_heard_artist
                .get(*a)
                .map_or(false, |&first_at| range_start_ts.map_or(true, |s| first_at >= s))
        })
        .count();
    let discovery_rate_pct = if unique_tracks > 0 {
        round1((new_tracks_discovered as f64 / unique_tracks as f64) * 100.0)
    } else {
        0.0
    };

    let lossless_pct = if total_streams > 0 {
        round1((lossless_streams as f64 / total_streams as f64) * 100.0)
    } else {
        0.0
    };
    let avg_bitrate = if bitrate_count > 0 {
        bitrate_sum / bitrate_count
    } else {
        0
    };

    // Build sorted sources, formats, quality tiers, genres, moods, decades.
    let mut sources: Vec<KeyCountStat> = source_counts
        .into_iter()
        .map(|(key, (streams, secs))| KeyCountStat {
            label: source_label(&key),
            key,
            streams,
            secs,
            pct: pct(streams, total_streams),
        })
        .collect();
    sources.sort_by(|a, b| b.streams.cmp(&a.streams).then(b.secs.cmp(&a.secs)));
    let favorite_source = sources.first().map(|s| s.key.clone()).unwrap_or_default();
    let favorite_source_label = sources
        .first()
        .map(|s| s.label.clone())
        .unwrap_or_else(|| "None yet".to_string());

    let mut formats: Vec<KeyCountStat> = format_counts
        .into_iter()
        .map(|(key, (streams, secs))| KeyCountStat {
            label: key.to_ascii_uppercase(),
            key,
            streams,
            secs,
            pct: pct(streams, total_streams),
        })
        .collect();
    formats.sort_by(|a, b| b.streams.cmp(&a.streams).then(b.secs.cmp(&a.secs)));

    let quality_order = [
        ("hi_res", "Hi-Res Lossless", "#fbbf24"),
        ("lossless", "CD Lossless", "#1ed760"),
        ("high_lossy", "High (256k+)", "#38bdf8"),
        ("standard", "Standard Stream", "#a855f7"),
        ("low", "Low Bitrate", "#6b7485"),
    ];
    let quality_tiers: Vec<QualityTierStat> = quality_order
        .iter()
        .filter_map(|&(tier, label, color)| {
            let &(streams, secs) = quality_counts.get(tier)?;
            if streams == 0 {
                return None;
            }
            Some(QualityTierStat {
                tier: tier.to_string(),
                label: label.to_string(),
                color: color.to_string(),
                streams,
                secs,
                pct: pct(streams, total_streams),
            })
        })
        .collect();

    let total_mood_hits: usize = mood_counts.values().map(|v| v.0).sum();
    let moods: Vec<MoodStat> = Mood::ALL
        .iter()
        .map(|m| {
            let (c1, c2) = m.gradient();
            let (streams, secs) = mood_counts.get(m.slug()).copied().unwrap_or((0, 0));
            MoodStat {
                slug: m.slug().to_string(),
                label: m.label().to_string(),
                c1: c1.to_string(),
                c2: c2.to_string(),
                streams,
                secs,
                pct: pct(streams, total_mood_hits),
            }
        })
        .collect();

    let mut genres: Vec<GenreStat> = genre_counts
        .into_values()
        .map(|g| GenreStat {
            name: g.display_name,
            streams: g.streams,
            secs: g.secs,
            pct: pct(g.streams, total_streams),
            unique_tracks: g.song_keys.len(),
        })
        .collect();
    genres.sort_by(|a, b| b.streams.cmp(&a.streams).then(b.secs.cmp(&a.secs)));
    genres.truncate(12);

    let decades: Vec<DecadeStat> = decade_counts
        .into_iter()
        .map(|(dec, (streams, secs))| DecadeStat {
            decade: dec,
            label: format!("{}s", dec),
            streams,
            secs,
            pct: pct(streams, total_streams),
        })
        .collect();

    let hourly: Vec<HourStat> = hour_counts
        .iter()
        .enumerate()
        .map(|(h, &(streams, secs))| HourStat {
            hour: h,
            label: format_hour_short(h),
            streams,
            secs,
            pct: pct(streams, total_streams),
        })
        .collect();
    let peak_hour = if total_streams > 0 {
        hourly
            .iter()
            .max_by(|a, b| a.streams.cmp(&b.streams).then(a.secs.cmp(&b.secs)))
            .filter(|h| h.streams > 0)
            .map(|h| h.hour)
    } else {
        None
    };

    const WEEKDAY_SHORT: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    const WEEKDAY_FULL: [&str; 7] = [
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
        "Sunday",
    ];
    let weekdays: Vec<WeekdayStat> = weekday_counts
        .iter()
        .enumerate()
        .map(|(i, &(streams, secs))| WeekdayStat {
            day: WEEKDAY_SHORT[i].to_string(),
            full_day: WEEKDAY_FULL[i].to_string(),
            streams,
            secs,
            pct: pct(streams, total_streams),
        })
        .collect();
    let peak_weekday = if total_streams > 0 {
        weekdays
            .iter()
            .max_by(|a, b| a.streams.cmp(&b.streams).then(a.secs.cmp(&b.secs)))
            .filter(|w| w.streams > 0)
            .map(|w| w.full_day.clone())
    } else {
        None
    };

    let timeline = build_timeline(
        &filtered_listens,
        range,
        range_start_date,
        today_local,
        offset,
    );

    // Top tracks, artists, albums, most skipped, highlights.
    let mut top_tracks_all: Vec<TopTrackStat> = track_acc
        .values()
        .filter(|ta| ta.streams > 0 && !ta.candidates.is_empty())
        .map(|ta| {
            let best_track = pick_representative_track(&ta.candidates, starred);
            TopTrackStat {
                track: best_track,
                streams: ta.streams,
                completed: ta.completed,
                skips: ta.skips,
                listened_secs: ta.listened_secs,
                last_played: ta.last_played,
            }
        })
        .collect();
    top_tracks_all.sort_by(|a, b| {
        b.streams
            .cmp(&a.streams)
            .then(b.listened_secs.cmp(&a.listened_secs))
            .then(b.last_played.cmp(&a.last_played))
    });

    let most_completed_track = top_tracks_all
        .iter()
        .filter(|t| t.completed > 0)
        .max_by(|a, b| a.completed.cmp(&b.completed).then(a.streams.cmp(&b.streams)))
        .map(|t| (t.track.clone(), t.completed));

    let mut most_skipped: Vec<SkippedTrackStat> = track_acc
        .values()
        .filter(|ta| ta.skips > 0 && !ta.candidates.is_empty())
        .map(|ta| SkippedTrackStat {
            track: pick_representative_track(&ta.candidates, starred),
            skips: ta.skips,
            streams: ta.streams,
        })
        .collect();
    most_skipped.sort_by(|a, b| b.skips.cmp(&a.skips).then(b.streams.cmp(&a.streams)));
    most_skipped.truncate(6);

    top_tracks_all.truncate(20);

    let mut top_artists: Vec<TopArtistStat> = artist_acc
        .into_iter()
        .map(|(id, aa)| TopArtistStat {
            is_followed: followed_artists.contains(&id),
            id,
            name: aa.name,
            streams: aa.streams,
            listened_secs: aa.listened_secs,
            unique_tracks: aa.song_keys.len(),
            pct: pct(aa.streams, total_streams),
        })
        .collect();
    top_artists.sort_by(|a, b| b.streams.cmp(&a.streams).then(b.listened_secs.cmp(&a.listened_secs)));
    top_artists.truncate(15);

    let mut top_albums: Vec<TopAlbumStat> = album_acc
        .into_iter()
        .map(|(id, ala)| TopAlbumStat {
            is_saved: saved_albums.contains(&id),
            id,
            name: ala.name,
            artist: ala.artist,
            artist_id: ala.artist_id,
            year: ala.year,
            streams: ala.streams,
            listened_secs: ala.listened_secs,
            unique_tracks: ala.song_keys.len(),
        })
        .collect();
    top_albums.sort_by(|a, b| b.streams.cmp(&a.streams).then(b.listened_secs.cmp(&a.listened_secs)));
    top_albums.truncate(15);

    let first_track_ever = listens
        .iter()
        .find_map(|l| l.track.as_ref().map(|t| (t.clone(), l.start_at)));

    let highlights = HighlightsData {
        longest_track,
        shortest_track,
        first_track_ever,
        most_completed_track,
    };

    let recent_events = build_recent_activity(events, &track_lookup);

    let summary = SummaryStats {
        total_streams,
        completed_streams,
        skipped_streams,
        partial_streams,
        completion_rate,
        skip_rate,
        total_listening_secs,
        avg_daily_secs,
        avg_streams_per_active_day,
        avg_track_duration_secs,
        unique_tracks,
        unique_artists,
        unique_albums,
        repeat_ratio,
        active_days,
        span_days,
        new_tracks_discovered,
        new_artists_discovered,
        discovery_rate_pct,
        lossless_streams,
        lossless_pct,
        avg_bitrate,
        favorite_source,
        favorite_source_label,
    };

    let persona = build_persona(
        &summary,
        &streaks,
        &library,
        &hourly,
        peak_hour,
        &moods,
        &genres,
    );

    ComputedStats {
        range,
        tz_offset_min,
        generated_at: now_ts,
        member_since: user.created,
        first_listen_at,
        last_listen_at,
        summary,
        streaks,
        library,
        persona,
        timeline,
        heatmap,
        hourly,
        peak_hour,
        weekdays,
        peak_weekday,
        moods,
        genres,
        quality_tiers,
        formats,
        sources,
        decades,
        oldest_track,
        newest_track,
        top_tracks: top_tracks_all,
        top_artists,
        top_albums,
        most_skipped,
        highlights,
        recent_events,
    }
}

fn ts_to_local(ts: i64, offset: FixedOffset) -> DateTime<FixedOffset> {
    DateTime::from_timestamp(ts, 0)
        .unwrap_or_else(Utc::now)
        .with_timezone(&offset)
}

fn canonical_song_key(track: &Track, fallback_id: &str) -> String {
    if !track.title.trim().is_empty() && !track.artist.trim().is_empty() {
        track.song_key()
    } else {
        fallback_id.to_string()
    }
}

fn reconstruct_listens(
    events: &[UserEventRow],
    track_lookup: &HashMap<String, Track>,
) -> (Vec<ListenRecord>, Vec<(String, i64)>) {
    let mut listens: Vec<ListenRecord> = Vec::new();
    let mut pending_play: HashMap<String, usize> = HashMap::new();
    let mut standalone_skips: Vec<(String, i64)> = Vec::new();

    for ev in events {
        let tr = ev.track.as_ref().or_else(|| track_lookup.get(&ev.track_id)).cloned();
        let dur = tr.as_ref().map(|t| t.duration.max(0) as i64).unwrap_or(0);
        let song_key = tr
            .as_ref()
            .map(|t| canonical_song_key(t, &ev.track_id))
            .unwrap_or_else(|| ev.track_id.clone());

        match ev.kind.as_str() {
            "play" => {
                let idx = listens.len();
                listens.push(ListenRecord {
                    song_key,
                    start_at: ev.at,
                    end_at: None,
                    completed: false,
                    skipped: false,
                    listened_secs: 0,
                    track: tr,
                });
                pending_play.insert(ev.track_id.clone(), idx);
            }
            "complete" => {
                let max_window = (dur * 2 + 300).max(3600);
                let paired = pending_play.get(&ev.track_id).copied().filter(|&idx| {
                    let l = &listens[idx];
                    !l.completed && !l.skipped && (ev.at - l.start_at) <= max_window
                });
                if let Some(idx) = paired {
                    listens[idx].completed = true;
                    listens[idx].end_at = Some(ev.at);
                    pending_play.remove(&ev.track_id);
                } else {
                    listens.push(ListenRecord {
                        song_key,
                        start_at: ev.at,
                        end_at: Some(ev.at),
                        completed: true,
                        skipped: false,
                        listened_secs: 0,
                        track: tr,
                    });
                }
            }
            "skip" => {
                let paired = pending_play.get(&ev.track_id).copied().filter(|&idx| {
                    let l = &listens[idx];
                    !l.completed && !l.skipped && (ev.at - l.start_at) <= 1800
                });
                if let Some(idx) = paired {
                    listens[idx].skipped = true;
                    listens[idx].end_at = Some(ev.at);
                    pending_play.remove(&ev.track_id);
                } else {
                    standalone_skips.push((ev.track_id.clone(), ev.at));
                }
            }
            _ => {}
        }
    }

    for i in 0..listens.len() {
        let dur = listens[i]
            .track
            .as_ref()
            .map(|t| t.duration.max(0) as i64)
            .unwrap_or(0);
        let secs = if listens[i].completed {
            dur
        } else if listens[i].skipped {
            let dt = listens[i].end_at.unwrap_or(listens[i].start_at) - listens[i].start_at;
            if dt > 0 {
                let cap = if dur > 0 { dur.min(30) } else { 30 };
                dt.min(cap)
            } else if dur > 0 {
                dur.min(15)
            } else {
                15
            }
        } else {
            let next_start = listens.get(i + 1).map(|n| n.start_at);
            match next_start {
                Some(ns) if ns >= listens[i].start_at => {
                    let gap = ns - listens[i].start_at;
                    if gap <= dur.max(120) + 60 {
                        if dur > 0 {
                            gap.min(dur)
                        } else {
                            gap.min(180)
                        }
                    } else if dur > 0 {
                        dur / 2
                    } else {
                        0
                    }
                }
                _ => {
                    if dur > 0 {
                        dur / 2
                    } else {
                        0
                    }
                }
            }
        };
        listens[i].listened_secs = secs.max(0);
    }

    (listens, standalone_skips)
}

fn compute_streaks(
    all_days: &BTreeMap<NaiveDate, (usize, i64)>,
    today_local: NaiveDate,
) -> StreakStats {
    let active_days_all_time = all_days.len();
    let mut longest_days = 0usize;
    let mut run = 0usize;
    let mut prev: Option<NaiveDate> = None;

    for &d in all_days.keys() {
        if let Some(p) = prev {
            if d == p + Duration::days(1) {
                run += 1;
            } else {
                run = 1;
            }
        } else {
            run = 1;
        }
        longest_days = longest_days.max(run);
        prev = Some(d);
    }

    let streak_active_today = all_days.contains_key(&today_local);
    let yesterday = today_local - Duration::days(1);
    let mut current_days = 0usize;
    let mut cursor = if streak_active_today {
        Some(today_local)
    } else if all_days.contains_key(&yesterday) {
        Some(yesterday)
    } else {
        None
    };

    while let Some(d) = cursor {
        if all_days.contains_key(&d) {
            current_days += 1;
            cursor = Some(d - Duration::days(1));
        } else {
            break;
        }
    }

    let best_day = all_days
        .iter()
        .max_by(|a, b| a.1 .0.cmp(&b.1 .0).then(a.1 .1.cmp(&b.1 .1)))
        .map(|(d, &(streams, secs))| BestDayStat {
            date: d.format("%Y-%m-%d").to_string(),
            streams,
            secs,
        });

    StreakStats {
        current_days,
        longest_days,
        streak_active_today,
        active_days_all_time,
        best_day,
    }
}

fn compute_heatmap(
    all_days: &BTreeMap<NaiveDate, (usize, i64)>,
    today_local: NaiveDate,
) -> HeatmapStats {
    let start_date = today_local - Duration::days(364);
    let mut days = Vec::new();
    let mut max_streams = 0usize;

    for (&d, &(streams, secs)) in all_days.range(start_date..=today_local) {
        if streams > 0 {
            max_streams = max_streams.max(streams);
            days.push(HeatmapDay {
                date: d.format("%Y-%m-%d").to_string(),
                streams,
                secs,
            });
        }
    }

    HeatmapStats {
        start_date: start_date.format("%Y-%m-%d").to_string(),
        end_date: today_local.format("%Y-%m-%d").to_string(),
        max_streams,
        days,
    }
}

fn build_timeline(
    filtered_listens: &[&ListenRecord],
    range: StatsRange,
    range_start_date: Option<NaiveDate>,
    today_local: NaiveDate,
    offset: FixedOffset,
) -> TimelineStats {
    let first_date = filtered_listens
        .first()
        .map(|l| ts_to_local(l.start_at, offset).date_naive())
        .unwrap_or(today_local);
    let start_date = range_start_date.unwrap_or_else(|| {
        let min_window = today_local - Duration::days(6);
        if first_date < min_window {
            first_date
        } else {
            min_window
        }
    });
    let span_days = (today_local - start_date).num_days() + 1;

    let granularity = match range {
        StatsRange::Days7 | StatsRange::Days30 | StatsRange::Days90 => "day",
        StatsRange::Days365 => "week",
        StatsRange::All => {
            if span_days <= 90 {
                "day"
            } else if span_days <= 400 {
                "week"
            } else {
                "month"
            }
        }
    };

    let mut buckets: BTreeMap<String, TimelinePoint> = BTreeMap::new();

    match granularity {
        "day" => {
            let mut cur = start_date;
            while cur <= today_local {
                let key = cur.format("%Y-%m-%d").to_string();
                let label = cur.format("%b %-d").to_string();
                let short_label = if span_days <= 7 {
                    cur.format("%a").to_string()
                } else {
                    cur.format("%-m/%-d").to_string()
                };
                buckets.insert(
                    key.clone(),
                    TimelinePoint {
                        key,
                        label,
                        short_label,
                        streams: 0,
                        completed: 0,
                        skipped: 0,
                        secs: 0,
                    },
                );
                cur += Duration::days(1);
            }
            for l in filtered_listens {
                let d = ts_to_local(l.start_at, offset).date_naive();
                let key = d.format("%Y-%m-%d").to_string();
                if let Some(p) = buckets.get_mut(&key) {
                    p.streams += 1;
                    if l.completed {
                        p.completed += 1;
                    }
                    if l.skipped {
                        p.skipped += 1;
                    }
                    p.secs += l.listened_secs;
                }
            }
        }
        "week" => {
            let start_mon = start_date
                - Duration::days(start_date.weekday().num_days_from_monday() as i64);
            let mut cur = start_mon;
            while cur <= today_local {
                let key = cur.format("%Y-%m-%d").to_string();
                let end_wk = (cur + Duration::days(6)).min(today_local);
                let label = format!("{} – {}", cur.format("%b %-d"), end_wk.format("%b %-d"));
                let short_label = cur.format("%b %-d").to_string();
                buckets.insert(
                    key.clone(),
                    TimelinePoint {
                        key,
                        label,
                        short_label,
                        streams: 0,
                        completed: 0,
                        skipped: 0,
                        secs: 0,
                    },
                );
                cur += Duration::days(7);
            }
            for l in filtered_listens {
                let d = ts_to_local(l.start_at, offset).date_naive();
                let mon = d - Duration::days(d.weekday().num_days_from_monday() as i64);
                let key = mon.format("%Y-%m-%d").to_string();
                if let Some(p) = buckets.get_mut(&key) {
                    p.streams += 1;
                    if l.completed {
                        p.completed += 1;
                    }
                    if l.skipped {
                        p.skipped += 1;
                    }
                    p.secs += l.listened_secs;
                }
            }
        }
        _ => {
            let mut y = start_date.year();
            let mut m = start_date.month();
            let end_y = today_local.year();
            let end_m = today_local.month();
            while (y, m) <= (end_y, end_m) {
                let first = NaiveDate::from_ymd_opt(y, m, 1).unwrap_or(today_local);
                let key = format!("{:04}-{:02}", y, m);
                let label = first.format("%b %Y").to_string();
                let short_label = first.format("%b '%y").to_string();
                buckets.insert(
                    key.clone(),
                    TimelinePoint {
                        key,
                        label,
                        short_label,
                        streams: 0,
                        completed: 0,
                        skipped: 0,
                        secs: 0,
                    },
                );
                if m == 12 {
                    y += 1;
                    m = 1;
                } else {
                    m += 1;
                }
            }
            for l in filtered_listens {
                let d = ts_to_local(l.start_at, offset).date_naive();
                let key = format!("{:04}-{:02}", d.year(), d.month());
                if let Some(p) = buckets.get_mut(&key) {
                    p.streams += 1;
                    if l.completed {
                        p.completed += 1;
                    }
                    if l.skipped {
                        p.skipped += 1;
                    }
                    p.secs += l.listened_secs;
                }
            }
        }
    }

    TimelineStats {
        granularity: granularity.to_string(),
        points: buckets.into_values().collect(),
    }
}

fn pick_representative_track(candidates: &[Track], starred: &HashSet<String>) -> Track {
    candidates
        .iter()
        .max_by(|a, b| {
            let a_star = starred.contains(&a.id);
            let b_star = starred.contains(&b.id);
            a_star
                .cmp(&b_star)
                .then(a.quality_score().cmp(&b.quality_score()))
                .then(a.created.cmp(&b.created))
        })
        .cloned()
        .unwrap_or_default()
}

fn build_recent_activity(
    events: &[UserEventRow],
    track_lookup: &HashMap<String, Track>,
) -> Vec<RecentActivityItem> {
    let mut out = Vec::new();
    for ev in events.iter().rev() {
        let Some(track) = ev
            .track
            .as_ref()
            .or_else(|| track_lookup.get(&ev.track_id))
            .cloned()
        else {
            continue;
        };
        let label = match ev.kind.as_str() {
            "complete" => "Completed".to_string(),
            "play" => "Started".to_string(),
            "skip" => "Skipped".to_string(),
            "star" => "Liked".to_string(),
            "playlist_add" => "Added to Playlist".to_string(),
            "search_pick" => "Search Pick".to_string(),
            "mix_play" => "Mix Play".to_string(),
            "rate" => format!("Rated ★{}", ev.value.round().clamp(1.0, 5.0) as i32),
            _ => continue,
        };
        out.push(RecentActivityItem {
            kind: ev.kind.clone(),
            label,
            at: ev.at,
            value: ev.value,
            track,
        });
        if out.len() >= 25 {
            break;
        }
    }
    out
}

fn build_persona(
    summary: &SummaryStats,
    streaks: &StreakStats,
    library: &LibraryStats,
    hourly: &[HourStat],
    peak_hour: Option<usize>,
    moods: &[MoodStat],
    genres: &[GenreStat],
) -> PersonaStats {
    if summary.total_streams == 0 {
        return PersonaStats {
            title: "Fresh Turntable".to_string(),
            chronotype: "Ready to Spin".to_string(),
            chronotype_desc: "Play a few songs to unlock your listening DNA".to_string(),
            style: "New Explorer".to_string(),
            headline: "Your personal listening stats, graphs, and taste DNA will light up automatically as you play music on Beatlab.".to_string(),
            badges: vec![],
        };
    }

    // Chronotype based on time-of-day buckets
    let night: usize = [22, 23, 0, 1, 2, 3].iter().map(|&h| hourly[h].streams).sum();
    let morning: usize = (5..=10).map(|h| hourly[h].streams).sum();
    let afternoon: usize = (11..=16).map(|h| hourly[h].streams).sum();
    let evening: usize = (17..=21).map(|h| hourly[h].streams).sum();

    let (chronotype, chronotype_prefix) = if night >= morning && night >= afternoon && night >= evening {
        ("Night Owl", "Late-Night")
    } else if morning >= afternoon && morning >= evening {
        ("Early Bird", "Morning")
    } else if afternoon >= evening {
        ("Daytime Groover", "Daytime")
    } else {
        ("Evening Listener", "Evening")
    };

    let chronotype_desc = match peak_hour {
        Some(h) => format!("Peak listening around {}", format_hour_long(h)),
        None => "Listening rhythm emerging".to_string(),
    };

    let (style, style_noun) = if summary.lossless_pct >= 55.0 && summary.total_streams >= 5 {
        ("Audiophile", "Audiophile")
    } else if summary.discovery_rate_pct >= 65.0 && summary.unique_tracks >= 8 {
        ("Crate Digger", "Crate Digger")
    } else if summary.repeat_ratio >= 2.2 && summary.total_streams >= 6 {
        ("Loyal Repeater", "Repeater")
    } else if library.manual_playlists >= 3 && library.curated_tracks >= 15 {
        ("Playlist Curator", "Curator")
    } else if genres.len() >= 5 {
        ("Genre Hopper", "Explorer")
    } else {
        ("Music Lover", "Selector")
    };

    let title = format!("{} {}", chronotype_prefix, style_noun);
    let top_genre = genres.first().map(|g| g.name.as_str());
    let top_mood = moods
        .iter()
        .filter(|m| m.streams > 0)
        .max_by_key(|m| m.streams)
        .map(|m| m.label.as_str());

    let flavor = match (top_genre, top_mood) {
        (Some(g), Some(m)) => format!(" with {} and {} vibes leading your rotation", g, m),
        (Some(g), None) => format!(" with {} leading your rotation", g),
        (None, Some(m)) => format!(" leaning heavily into {}", m),
        (None, None) => String::new(),
    };

    let time_str = format_duration_words(summary.total_listening_secs);
    let peak_str = peak_hour
        .map(|h| format!(", peaking around {}", format_hour_long(h)))
        .unwrap_or_default();
    let headline = format!(
        "You logged {} across {} tracks and {} artists for {}{}{}.",
        plural(summary.total_streams, "stream", "streams"),
        summary.unique_tracks,
        summary.unique_artists,
        time_str,
        peak_str,
        flavor,
    );

    let mut badges = vec![chronotype.to_string()];
    if summary.lossless_pct >= 25.0 {
        badges.push(format!("{:.0}% Lossless", summary.lossless_pct));
    }
    if summary.total_streams >= 3 {
        badges.push(format!("{:.0}% Completion", summary.completion_rate));
    }
    if streaks.current_days >= 2 {
        badges.push(format!("{}-Day Streak", streaks.current_days));
    } else if summary.unique_artists >= 3 {
        badges.push(format!("{} Artists", summary.unique_artists));
    }
    if summary.discovery_rate_pct >= 50.0 && summary.unique_tracks >= 5 {
        badges.push(format!("{:.0}% Fresh Picks", summary.discovery_rate_pct));
    }

    PersonaStats {
        title,
        chronotype: chronotype.to_string(),
        chronotype_desc,
        style: style.to_string(),
        headline,
        badges,
    }
}

fn infer_format_key(t: &Track) -> String {
    let f = t.format.trim().to_lowercase();
    if !f.is_empty() {
        return f;
    }
    match t.source.as_str() {
        "deezer" | "ytm" => "aac".to_string(),
        _ => "stream".to_string(),
    }
}

fn source_label(src: &str) -> String {
    match src {
        "local" => "Local Library".to_string(),
        "deezer" => "Deezer".to_string(),
        "ytm" => "YouTube Music".to_string(),
        "soulseek" => "Soulseek".to_string(),
        "archive" => "Internet Archive".to_string(),
        "soundcloud" => "SoundCloud".to_string(),
        "youtube" => "YouTube".to_string(),
        "" | "unknown" => "Unknown".to_string(),
        other => {
            let mut chars = other.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
                None => "Unknown".to_string(),
            }
        }
    }
}

fn title_case_genre(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "edm" => "EDM".to_string(),
        "idm" => "IDM".to_string(),
        "r&b" | "rnb" => "R&B".to_string(),
        "uk garage" => "UK Garage".to_string(),
        "dnb" => "Drum & Bass".to_string(),
        "k-pop" | "kpop" => "K-Pop".to_string(),
        "j-pop" | "jpop" => "J-Pop".to_string(),
        "lo-fi" | "lofi" => "Lo-Fi".to_string(),
        "hip-hop" | "hip hop" => "Hip-Hop".to_string(),
        "synth-pop" | "synthpop" => "Synth-Pop".to_string(),
        other => other
            .split_whitespace()
            .map(|word| {
                let mut ch = word.chars();
                match ch.next() {
                    Some(c) => format!("{}{}", c.to_uppercase(), ch.as_str()),
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
    }
}

fn format_hour_short(h: usize) -> String {
    match h {
        0 => "12a".to_string(),
        1..=11 => format!("{}a", h),
        12 => "12p".to_string(),
        _ => format!("{}p", h - 12),
    }
}

fn format_hour_long(h: usize) -> String {
    match h {
        0 => "12 AM".to_string(),
        1..=11 => format!("{} AM", h),
        12 => "12 PM".to_string(),
        _ => format!("{} PM", h - 12),
    }
}

fn format_duration_words(secs: i64) -> String {
    if secs <= 0 {
        return "0m".to_string();
    }
    let mins = secs / 60;
    let hrs = mins / 60;
    let rem_m = mins % 60;
    if hrs > 0 {
        format!("{}h {}m", hrs, rem_m)
    } else if mins > 0 {
        format!("{}m", mins)
    } else {
        format!("{}s", secs)
    }
}

fn plural(n: usize, singular: &str, plural_word: &str) -> String {
    if n == 1 {
        format!("1 {}", singular)
    } else {
        format!("{} {}", n, plural_word)
    }
}

fn pct(part: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        round1((part as f64 / total as f64) * 100.0)
    }
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Playlist;

    fn mk_track(
        id: &str,
        title: &str,
        artist: &str,
        album: &str,
        genre: &str,
        year: i32,
        duration: i32,
        format: &str,
        source: &str,
    ) -> Track {
        Track {
            id: id.into(),
            source: source.into(),
            title: title.into(),
            artist: artist.into(),
            album: album.into(),
            genre: genre.into(),
            year,
            duration,
            format: format.into(),
            bitrate: if format == "flac" { 950 } else { 256 },
            ..Default::default()
        }
    }

    #[test]
    fn empty_history_produces_zeroed_stats_safely() {
        let store = Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("emptyuser", "pw123").unwrap();
        let stats = build_user_stats(&store, &u, StatsRange::All, 0, 1_700_000_000);
        assert_eq!(stats.summary.total_streams, 0);
        assert_eq!(stats.summary.total_listening_secs, 0);
        assert_eq!(stats.streaks.current_days, 0);
        assert_eq!(stats.hourly.len(), 24);
        assert_eq!(stats.weekdays.len(), 7);
        assert_eq!(stats.moods.len(), 10);
        assert_eq!(stats.persona.title, "Fresh Turntable");
    }

    #[test]
    fn paired_play_and_complete_counts_as_single_completed_stream_and_respects_tz_and_streaks() {
        let user = User {
            id: 1,
            username: "alex".into(),
            password: "pw".into(),
            created: 1_700_000_000,
        };
        let t1 = mk_track(
            "loc:1",
            "Midnight City",
            "M83",
            "Hurry Up, We're Dreaming",
            "Synth-Pop, Indie",
            2011,
            243,
            "flac",
            "local",
        );
        let t2 = mk_track(
            "dz:2",
            "Digital Love",
            "Daft Punk",
            "Discovery",
            "Electronic, House",
            2001,
            301,
            "m4a",
            "deezer",
        );

        // Nov 16, 2026 04:00 UTC = Nov 15, 2026 23:00 UTC-5 (-300 min)
        let nov16_04utc = Utc.with_ymd_and_hms(2026, 11, 16, 4, 0, 0).unwrap().timestamp();
        let nov15_04utc = nov16_04utc - 86400; // Nov 14 23:00 UTC-5
        let nov14_04utc = nov16_04utc - 2 * 86400; // Nov 13 23:00 UTC-5
        let now_ts = Utc.with_ymd_and_hms(2026, 11, 16, 15, 0, 0).unwrap().timestamp(); // Nov 16 10:00 UTC-5

        let events = vec![
            // Day 1 (Nov 13 local): completed t1
            UserEventRow {
                track_id: t1.id.clone(),
                kind: "play".into(),
                value: 1.0,
                at: nov14_04utc,
                track: Some(t1.clone()),
            },
            UserEventRow {
                track_id: t1.id.clone(),
                kind: "complete".into(),
                value: 1.0,
                at: nov14_04utc + 200,
                track: Some(t1.clone()),
            },
            // Day 2 (Nov 14 local): skipped t2 after 12s
            UserEventRow {
                track_id: t2.id.clone(),
                kind: "play".into(),
                value: 1.0,
                at: nov15_04utc,
                track: Some(t2.clone()),
            },
            UserEventRow {
                track_id: t2.id.clone(),
                kind: "skip".into(),
                value: 1.0,
                at: nov15_04utc + 12,
                track: Some(t2.clone()),
            },
            // Day 3 (Nov 15 local, yesterday relative to Nov 16): completed t1 again
            UserEventRow {
                track_id: t1.id.clone(),
                kind: "play".into(),
                value: 1.0,
                at: nov16_04utc,
                track: Some(t1.clone()),
            },
            UserEventRow {
                track_id: t1.id.clone(),
                kind: "complete".into(),
                value: 1.0,
                at: nov16_04utc + 190,
                track: Some(t1.clone()),
            },
        ];

        let mut starred = HashSet::new();
        starred.insert(t1.id.clone());
        let ratings = HashMap::from([(t1.id.clone(), 5)]);
        let pls = vec![Playlist {
            id: "pl-1".into(),
            user_id: 1,
            owner_name: "alex".into(),
            name: "Night Drive".into(),
            comment: "".into(),
            public: false,
            is_auto: false,
            kind: "user".into(),
            created: now_ts,
            changed: now_ts,
            track_count: 10,
            duration: 2400,
            cover_track_id: None,
        }];

        let offset = FixedOffset::east_opt(-300 * 60).unwrap();
        let stats = compute_stats_from_data(
            &user,
            &events,
            &starred,
            &HashSet::new(),
            &HashSet::new(),
            &ratings,
            &pls,
            0,
            StatsRange::All,
            -300,
            offset,
            now_ts,
        );

        assert_eq!(stats.summary.total_streams, 3);
        assert_eq!(stats.summary.completed_streams, 2);
        assert_eq!(stats.summary.skipped_streams, 1);
        assert_eq!(stats.summary.total_listening_secs, 243 + 12 + 243);
        assert_eq!(stats.summary.unique_tracks, 2);
        assert_eq!(stats.summary.unique_artists, 2);
        // All 3 events started at 23:00 local (UTC-5)
        assert_eq!(stats.hourly[23].streams, 3);
        assert_eq!(stats.peak_hour, Some(23));
        // Streak covers Nov 13, 14, 15 (yesterday relative to Nov 16)
        assert_eq!(stats.streaks.current_days, 3);
        assert_eq!(stats.streaks.longest_days, 3);
        assert!(!stats.streaks.streak_active_today);
        assert_eq!(stats.top_tracks[0].track.id, "loc:1");
        assert_eq!(stats.top_tracks[0].streams, 2);
        assert_eq!(stats.library.avg_rating, 5.0);
        assert_eq!(stats.decades.len(), 2);
    }

    #[test]
    fn discovery_tracks_first_listen_before_selected_window() {
        let user = User {
            id: 1,
            username: "alex".into(),
            password: "pw".into(),
            created: 1_700_000_000,
        };
        let old_track = mk_track(
            "t:old",
            "Old Favorite",
            "Artist A",
            "Album A",
            "Rock",
            1999,
            200,
            "flac",
            "local",
        );
        let new_track = mk_track(
            "t:new",
            "Fresh Find",
            "Artist B",
            "Album B",
            "Indie",
            2025,
            180,
            "flac",
            "local",
        );
        let now_ts = Utc.with_ymd_and_hms(2026, 11, 16, 12, 0, 0).unwrap().timestamp();
        let twenty_days_ago = now_ts - 20 * 86400;
        let two_days_ago = now_ts - 2 * 86400;

        let events = vec![
            UserEventRow {
                track_id: old_track.id.clone(),
                kind: "complete".into(),
                value: 1.0,
                at: twenty_days_ago,
                track: Some(old_track.clone()),
            },
            UserEventRow {
                track_id: old_track.id.clone(),
                kind: "complete".into(),
                value: 1.0,
                at: two_days_ago,
                track: Some(old_track.clone()),
            },
            UserEventRow {
                track_id: new_track.id.clone(),
                kind: "complete".into(),
                value: 1.0,
                at: two_days_ago + 300,
                track: Some(new_track.clone()),
            },
        ];

        let offset = FixedOffset::east_opt(0).unwrap();
        let stats_7d = compute_stats_from_data(
            &user,
            &events,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashMap::new(),
            &[],
            0,
            StatsRange::Days7,
            0,
            offset,
            now_ts,
        );

        assert_eq!(stats_7d.summary.total_streams, 2);
        assert_eq!(stats_7d.summary.unique_tracks, 2);
        assert_eq!(stats_7d.summary.new_tracks_discovered, 1);
        assert_eq!(stats_7d.summary.new_artists_discovered, 1);
        assert_eq!(stats_7d.summary.discovery_rate_pct, 50.0);
        assert_eq!(stats_7d.timeline.points.len(), 7);
    }
}
