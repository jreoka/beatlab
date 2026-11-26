use crate::models::{primary_artist, slug, upgrade_cover_url, AlbumSummary, Track};
use crate::sources::deezer::NEW_RELEASE_WINDOW_DAYS;
use crate::taste::classify_moods_str;
use chrono::Utc;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{RwLock, Semaphore};
use tracing::warn;

/// Apple's public iTunes Search/Lookup API allows ~20 requests/min per IP.
/// Keep bursts small and paced, and batch artist album lookups (up to 12 artists
/// per request) so refreshing dozens of artists only takes a few requests.
const IT_BUCKET_CAPACITY: f64 = 10.0;
const IT_BUCKET_REFILL_PER_SEC: f64 = 0.5;
const IT_BLOCK_COOLDOWN: Duration = Duration::from_secs(5 * 60);
const IT_JSON_CACHE_TTL: Duration = Duration::from_secs(6 * 60 * 60);
const IT_JSON_CACHE_MAX: usize = 200;
const IT_BATCH_SIZE: usize = 12;

struct TokenBucket {
    tokens: f64,
    last: Instant,
}

#[derive(Clone)]
pub struct ItunesSource {
    http: reqwest::Client,
    http_sem: Arc<Semaphore>,
    json_cache: Arc<RwLock<HashMap<String, (Instant, serde_json::Value)>>>,
    bucket: Arc<std::sync::Mutex<TokenBucket>>,
    blocked_until: Arc<std::sync::Mutex<Option<Instant>>>,
}

#[derive(Debug, Clone)]
pub struct ItunesArtist {
    pub id: u64,
    pub name: String,
    #[allow(dead_code)]
    pub genre: String,
}

impl ItunesSource {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent("Beatlab/0.3 (MusicDiscovery)")
            .build()
            .unwrap_or_default();
        Self {
            http,
            http_sem: Arc::new(Semaphore::new(2)),
            json_cache: Arc::new(RwLock::new(HashMap::new())),
            bucket: Arc::new(std::sync::Mutex::new(TokenBucket {
                tokens: IT_BUCKET_CAPACITY,
                last: Instant::now(),
            })),
            blocked_until: Arc::new(std::sync::Mutex::new(None)),
        }
    }

    pub fn is_blocked(&self) -> bool {
        let guard = self.blocked_until.lock().unwrap_or_else(|e| e.into_inner());
        guard.map_or(false, |until| Instant::now() < until)
    }

    fn mark_blocked(&self, url: &str, status: u16) {
        let mut guard = self.blocked_until.lock().unwrap_or_else(|e| e.into_inner());
        let already = guard.map_or(false, |until| Instant::now() < until);
        *guard = Some(Instant::now() + IT_BLOCK_COOLDOWN);
        if !already {
            warn!(
                url,
                status,
                cooldown_secs = IT_BLOCK_COOLDOWN.as_secs(),
                "itunes API rate-limited/refused; pausing itunes calls"
            );
        }
    }

    async fn throttle(&self) {
        self.throttle_with_floor(1.0).await;
    }

    async fn throttle_interactive(&self) {
        // Allow 1-2 user-initiated clicks to go out immediately even right after
        // a background New Releases sweep drained the bucket; background calls
        // still wait until tokens refill to >= 1.0 so the overall rate stays bounded.
        self.throttle_with_floor(-1.0).await;
    }

    async fn throttle_with_floor(&self, min_tokens_before_spend: f64) {
        loop {
            let wait = {
                let mut b = self.bucket.lock().unwrap_or_else(|e| e.into_inner());
                let now = Instant::now();
                let refill = now.duration_since(b.last).as_secs_f64() * IT_BUCKET_REFILL_PER_SEC;
                b.tokens = (b.tokens + refill).min(IT_BUCKET_CAPACITY);
                b.last = now;
                if b.tokens >= min_tokens_before_spend {
                    b.tokens -= 1.0;
                    None
                } else {
                    Some(Duration::from_secs_f64(
                        (min_tokens_before_spend - b.tokens) / IT_BUCKET_REFILL_PER_SEC,
                    ))
                }
            };
            match wait {
                None => return,
                Some(d) => tokio::time::sleep(d).await,
            }
        }
    }

    async fn get_json<T: DeserializeOwned>(&self, url: &str) -> Option<T> {
        self.get_json_inner(url, false).await
    }

    async fn get_json_interactive<T: DeserializeOwned>(&self, url: &str) -> Option<T> {
        self.get_json_inner(url, true).await
    }

    async fn get_json_inner<T: DeserializeOwned>(&self, url: &str, interactive: bool) -> Option<T> {
        {
            let guard = self.json_cache.read().await;
            if let Some((ts, v)) = guard.get(url) {
                if ts.elapsed() < IT_JSON_CACHE_TTL {
                    if let Ok(parsed) = serde_json::from_value::<T>(v.clone()) {
                        return Some(parsed);
                    }
                }
            }
        }
        let val = self.fetch_and_cache(url, interactive).await?;
        serde_json::from_value::<T>(val).ok()
    }

    async fn fetch_and_cache(&self, url: &str, interactive: bool) -> Option<serde_json::Value> {
        for attempt in 0..2 {
            if self.is_blocked() {
                return None;
            }
            if interactive {
                self.throttle_interactive().await;
            } else {
                self.throttle().await;
            }
            let permit = self.http_sem.acquire().await.ok();
            let resp = match self.http.get(url).send().await {
                Ok(r) => r,
                Err(e) => {
                    drop(permit);
                    if attempt == 0 {
                        tokio::time::sleep(Duration::from_millis(300)).await;
                        continue;
                    }
                    warn!(error = %e, url, "itunes request failed");
                    return None;
                }
            };
            let status = resp.status();
            let code = status.as_u16();
            if code == 429 || code == 403 {
                drop(permit);
                self.mark_blocked(url, code);
                return None;
            }
            if !status.is_success() {
                drop(permit);
                warn!(status = %status, url, "itunes HTTP non-success");
                return None;
            }
            let val = match resp.json::<serde_json::Value>().await {
                Ok(v) => v,
                Err(e) => {
                    drop(permit);
                    warn!(error = %e, url, "itunes invalid JSON");
                    return None;
                }
            };
            drop(permit);
            {
                let mut guard = self.json_cache.write().await;
                if guard.len() >= IT_JSON_CACHE_MAX {
                    guard.retain(|_, (ts, _)| ts.elapsed() < IT_JSON_CACHE_TTL);
                    if guard.len() >= IT_JSON_CACHE_MAX {
                        let mut ages: Vec<(Instant, String)> =
                            guard.iter().map(|(k, (ts, _))| (*ts, k.clone())).collect();
                        ages.sort_by_key(|(ts, _)| *ts);
                        for (_, k) in ages.into_iter().take(IT_JSON_CACHE_MAX / 4) {
                            guard.remove(&k);
                        }
                    }
                }
                guard.insert(url.to_string(), (Instant::now(), val.clone()));
            }
            return Some(val);
        }
        None
    }

    /// Searches Apple's iTunes catalog for music artists. Returns `None` on
    /// network/rate-limit failure so callers don't cache transient errors.
    pub async fn search_artists(&self, query: &str, limit: usize) -> Option<Vec<ItunesArtist>> {
        let q = query.trim();
        if q.is_empty() {
            return Some(Vec::new());
        }
        let url = format!(
            "https://itunes.apple.com/search?term={}&entity=musicArtist&limit={}",
            urlencoding::encode(q),
            limit.clamp(3, 15)
        );
        let parsed = self.get_json::<ItResponse>(&url).await?;
        Some(
            parsed
                .results
                .into_iter()
                .filter(|r| r.artist_id > 0 && !r.artist_name.trim().is_empty())
                .map(|r| ItunesArtist {
                    id: r.artist_id,
                    name: r.artist_name.trim().to_string(),
                    genre: r.primary_genre_name.trim().to_string(),
                })
                .collect(),
        )
    }

    /// Fetches sample song titles for an iTunes artist ID (used to disambiguate
    /// homonym artists like "Eve" J-Pop vs "Eve" Hip-Hop).
    pub async fn fetch_artist_top_songs(&self, artist_id: u64, limit: usize) -> Vec<String> {
        if artist_id == 0 {
            return Vec::new();
        }
        let url = format!(
            "https://itunes.apple.com/lookup?id={}&entity=song&limit={}",
            artist_id,
            limit.clamp(5, 30)
        );
        let Some(parsed) = self.get_json::<ItResponse>(&url).await else {
            return Vec::new();
        };
        parsed
            .results
            .into_iter()
            .filter(|r| r.wrapper_type.eq_ignore_ascii_case("track") && !r.track_name.trim().is_empty())
            .map(|r| r.track_name.trim().to_string())
            .collect()
    }

    /// Fetches recent releases within `NEW_RELEASE_WINDOW_DAYS` for a batch of
    /// `(itunes_artist_id, beatlab_artist_id, display_name)` entries. Uses
    /// Apple's multi-ID lookup endpoint (up to 12 artists per HTTP request).
    /// Returns `(answered_artist_count, albums)`.
    pub async fn fetch_artists_recent_releases(
        &self,
        artists: &[(u64, String, String)],
        limit_per_artist: usize,
    ) -> (usize, Vec<AlbumSummary>) {
        if artists.is_empty() {
            return (0, Vec::new());
        }
        let today = Utc::now().date_naive();
        let cutoff = (today - chrono::Duration::days(NEW_RELEASE_WINDOW_DAYS))
            .format("%Y-%m-%d")
            .to_string();
        let max_date = (today + chrono::Duration::days(1))
            .format("%Y-%m-%d")
            .to_string();

        let by_it_id: HashMap<u64, (&str, &str)> = artists
            .iter()
            .map(|(it_id, ar_id, name)| (*it_id, (ar_id.as_str(), name.as_str())))
            .collect();

        let mut answered = 0usize;
        let mut out: Vec<AlbumSummary> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();

        for chunk in artists.chunks(IT_BATCH_SIZE) {
            if self.is_blocked() {
                break;
            }
            let ids_csv = chunk
                .iter()
                .map(|(id, _, _)| id.to_string())
                .collect::<Vec<_>>()
                .join(",");
            let url = format!(
                "https://itunes.apple.com/lookup?id={}&entity=album&sort=recent&limit={}",
                ids_csv,
                limit_per_artist.clamp(2, 10)
            );
            let Some(parsed) = self.get_json::<ItResponse>(&url).await else {
                continue;
            };
            answered += chunk.len();
            for item in parsed.results {
                if let Some(summary) = item.into_recent_album(&by_it_id, &cutoff, &max_date) {
                    if seen.insert(summary.id.clone()) {
                        out.push(summary);
                    }
                }
            }
        }
        (answered, out)
    }

    /// Hydrates an iTunes album (`al:it-<collectionId>`) with its official
    /// tracklist so clicking or playing a New Release card works even when
    /// Deezer is unavailable.
    pub async fn fetch_album(&self, collection_id: u64) -> Option<(AlbumSummary, Vec<Track>)> {
        if collection_id == 0 {
            return None;
        }
        let url = format!(
            "https://itunes.apple.com/lookup?id={}&entity=song",
            collection_id
        );
        let parsed = self.get_json_interactive::<ItResponse>(&url).await?;
        parse_itunes_album_response(collection_id, parsed)
    }
}

/// Strips iTunes's standard `" - Single"` and `" - EP"` title suffixes so release
/// titles match Deezer/YouTube Music cleanly for display, dedup, and playback.
pub fn clean_itunes_collection_name(raw: &str) -> String {
    let trimmed = raw.trim();
    for suffix in [" - Single", " - EP"] {
        if let Some(stripped) = trimmed.strip_suffix(suffix) {
            let s = stripped.trim();
            if !s.is_empty() {
                return s.to_string();
            }
        }
    }
    trimmed.to_string()
}

fn null_to_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

#[derive(Debug, Clone, Default, Deserialize)]
struct ItResponse {
    #[serde(default, deserialize_with = "null_to_default")]
    results: Vec<ItResult>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ItResult {
    #[serde(default, deserialize_with = "null_to_default")]
    wrapper_type: String,
    #[serde(default, deserialize_with = "null_to_default")]
    kind: String,
    #[serde(default, deserialize_with = "null_to_default")]
    collection_type: String,
    #[serde(default, deserialize_with = "null_to_default")]
    artist_id: u64,
    #[serde(default, deserialize_with = "null_to_default")]
    collection_id: u64,
    #[serde(default, deserialize_with = "null_to_default")]
    track_id: u64,
    #[serde(default, deserialize_with = "null_to_default")]
    artist_name: String,
    #[serde(default, deserialize_with = "null_to_default")]
    collection_name: String,
    #[serde(default, deserialize_with = "null_to_default")]
    track_name: String,
    #[serde(default, deserialize_with = "null_to_default")]
    artwork_url100: String,
    #[serde(default, deserialize_with = "null_to_default")]
    artwork_url60: String,
    #[serde(default, deserialize_with = "null_to_default")]
    release_date: String,
    #[serde(default, deserialize_with = "null_to_default")]
    primary_genre_name: String,
    #[serde(default, deserialize_with = "null_to_default")]
    copyright: String,
    #[serde(default, deserialize_with = "null_to_default")]
    track_count: usize,
    #[serde(default, deserialize_with = "null_to_default")]
    track_number: i32,
    #[serde(default, deserialize_with = "null_to_default")]
    disc_number: i32,
    #[serde(default, deserialize_with = "null_to_default")]
    track_time_millis: i64,
}

impl ItResult {
    fn best_cover(&self) -> String {
        let raw = if !self.artwork_url100.trim().is_empty() {
            self.artwork_url100.trim()
        } else {
            self.artwork_url60.trim()
        };
        if raw.is_empty() {
            String::new()
        } else {
            upgrade_cover_url(raw)
        }
    }

    fn into_recent_album(
        self,
        by_it_id: &HashMap<u64, (&str, &str)>,
        cutoff: &str,
        max_date: &str,
    ) -> Option<AlbumSummary> {
        if !self.wrapper_type.eq_ignore_ascii_case("collection") || self.collection_id == 0 {
            return None;
        }
        if !self.collection_type.is_empty() && !self.collection_type.eq_ignore_ascii_case("Album") {
            return None;
        }
        // Only keep releases belonging to one of the requested artists (skip compilations).
        let (beatlab_artist_id, fallback_name) = by_it_id.get(&self.artist_id)?;
        let date = self.release_date.trim().get(..10)?;
        if date < cutoff || date > max_date {
            return None;
        }
        let released_ts = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
            .ok()
            .and_then(|d| d.and_hms_opt(0, 0, 0))
            .map(|dt| dt.and_utc().timestamp())?;
        let name = clean_itunes_collection_name(&self.collection_name);
        if name.is_empty() {
            return None;
        }
        let artist = if !self.artist_name.trim().is_empty() {
            self.artist_name.trim().to_string()
        } else {
            (*fallback_name).to_string()
        };
        let aid = format!("al:it-{}", self.collection_id);
        let year = date.get(..4).and_then(|y| y.parse::<i32>().ok()).unwrap_or(0);
        let artist_id = if !beatlab_artist_id.is_empty() {
            (*beatlab_artist_id).to_string()
        } else {
            format!("ar:{}", slug(&artist))
        };
        Some(AlbumSummary {
            id: aid.clone(),
            name,
            artist_id,
            artist,
            cover_id: aid,
            cover_url: self.best_cover(),
            genre: self.primary_genre_name.trim().to_string(),
            year,
            label: self.copyright.trim().to_string(),
            deezer_id: String::new(),
            song_count: self.track_count,
            duration: 0,
            play_count: 0,
            last_played: 0,
            created: released_ts,
        })
    }
}

fn parse_itunes_album_response(
    collection_id: u64,
    parsed: ItResponse,
) -> Option<(AlbumSummary, Vec<Track>)> {
    let mut coll: Option<ItResult> = None;
    let mut raw_tracks: Vec<ItResult> = Vec::new();
    for item in parsed.results {
        if item.wrapper_type.eq_ignore_ascii_case("collection") && coll.is_none() {
            coll = Some(item);
        } else if item.wrapper_type.eq_ignore_ascii_case("track")
            && (item.kind.is_empty() || item.kind.eq_ignore_ascii_case("song"))
            && item.track_id > 0
            && !item.track_name.trim().is_empty()
        {
            raw_tracks.push(item);
        }
    }
    let c = coll?;
    let album_name = clean_itunes_collection_name(&c.collection_name);
    if album_name.is_empty() {
        return None;
    }
    let album_artist = c.artist_name.trim().to_string();
    let album_cover = c.best_cover();
    let album_genre = c.primary_genre_name.trim().to_string();
    let album_year = c
        .release_date
        .trim()
        .get(..4)
        .and_then(|y| y.parse::<i32>().ok())
        .unwrap_or(0);
    let released_ts = c
        .release_date
        .trim()
        .get(..10)
        .and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map(|dt| dt.and_utc().timestamp())
        .unwrap_or_else(|| Utc::now().timestamp());

    let aid = format!("al:it-{}", collection_id);
    let mut tracks: Vec<Track> = Vec::with_capacity(raw_tracks.len());
    let mut total_dur = 0i32;

    for t in raw_tracks {
        let title = t.track_name.trim().to_string();
        let artist = if !t.artist_name.trim().is_empty() {
            t.artist_name.trim().to_string()
        } else {
            album_artist.clone()
        };
        let dur = ((t.track_time_millis.max(0) + 500) / 1000) as i32;
        total_dur += dur;
        let genre = if !t.primary_genre_name.trim().is_empty() {
            t.primary_genre_name.trim().to_string()
        } else {
            album_genre.clone()
        };
        let cover_url = {
            let tc = t.best_cover();
            if tc.is_empty() {
                album_cover.clone()
            } else {
                tc
            }
        };
        let year = t
            .release_date
            .trim()
            .get(..4)
            .and_then(|y| y.parse::<i32>().ok())
            .unwrap_or(album_year);
        let moods = classify_moods_str(&title, &album_name, &genre, "");
        let meta = serde_json::json!({
            "itunes_track_id": t.track_id,
            "itunes_album_id": collection_id,
            "itunes_artist_id": t.artist_id,
        })
        .to_string();
        tracks.push(Track {
            id: format!("it:{}", t.track_id),
            source: "itunes".to_string(),
            title: title.clone(),
            artist: artist.clone(),
            album: album_name.clone(),
            genre,
            moods,
            year,
            track_no: t.track_number.max(0),
            disc_no: if t.disc_number > 0 { t.disc_number } else { 1 },
            duration: dur,
            bitrate: 256,
            sample_rate: 44100,
            bit_depth: 0,
            format: "m4a".to_string(),
            size: 0,
            stream_url: format!("resolve:ytm:{}|{}|{}", artist, title, dur),
            cover_url,
            meta,
            deezer_id: String::new(),
            created: released_ts,
        });
    }
    tracks.sort_by(|a, b| a.disc_no.cmp(&b.disc_no).then_with(|| a.track_no.cmp(&b.track_no)));

    let pa = primary_artist(&album_artist);
    let primary_alb_artist = if pa.is_empty() {
        album_artist.as_str()
    } else {
        pa.as_str()
    };
    let summary = AlbumSummary {
        id: aid.clone(),
        name: album_name,
        artist_id: format!("ar:{}", slug(primary_alb_artist)),
        artist: album_artist,
        cover_id: aid,
        cover_url: album_cover,
        genre: album_genre,
        year: album_year,
        label: c.copyright.trim().to_string(),
        deezer_id: String::new(),
        song_count: tracks.len().max(c.track_count),
        duration: total_dur,
        play_count: 0,
        last_played: 0,
        created: released_ts,
    };
    Some((summary, tracks))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_itunes_title_strips_single_and_ep_suffixes() {
        assert_eq!(clean_itunes_collection_name("Wake Me Up - Single"), "Wake Me Up");
        assert_eq!(
            clean_itunes_collection_name("Remedy / Double Wings - EP"),
            "Remedy / Double Wings"
        );
        assert_eq!(clean_itunes_collection_name("BLAZE"), "BLAZE");
    }

    #[test]
    fn parse_itunes_album_response_builds_playable_tracks_and_hd_cover() {
        let raw = r#"{
            "resultCount": 3,
            "results": [
                {
                    "wrapperType": "collection",
                    "collectionType": "Album",
                    "artistId": 1300493324,
                    "collectionId": 6831539495,
                    "artistName": "ASCA",
                    "collectionName": "Remedy / Double Wings - EP",
                    "artworkUrl100": "https://is1-ssl.mzstatic.com/image/thumb/x/100x100bb.jpg",
                    "releaseDate": "2026-11-18T08:00:00Z",
                    "primaryGenreName": "Anime",
                    "trackCount": 2
                },
                {
                    "wrapperType": "track",
                    "kind": "song",
                    "artistId": 1300493324,
                    "collectionId": 6831539495,
                    "trackId": 9001,
                    "artistName": "ASCA",
                    "trackName": "Remedy",
                    "artworkUrl100": "https://is1-ssl.mzstatic.com/image/thumb/x/100x100bb.jpg",
                    "releaseDate": "2026-11-18T08:00:00Z",
                    "primaryGenreName": "Anime",
                    "discNumber": 1,
                    "trackNumber": 1,
                    "trackTimeMillis": 246248
                },
                {
                    "wrapperType": "track",
                    "kind": "song",
                    "artistId": 1300493324,
                    "collectionId": 6831539495,
                    "trackId": 9002,
                    "artistName": "ASCA",
                    "trackName": "Double Wings",
                    "artworkUrl100": "https://is1-ssl.mzstatic.com/image/thumb/x/100x100bb.jpg",
                    "releaseDate": "2026-11-18T08:00:00Z",
                    "primaryGenreName": "Anime",
                    "discNumber": 1,
                    "trackNumber": 2,
                    "trackTimeMillis": 218631
                }
            ]
        }"#;
        let parsed: ItResponse = serde_json::from_str(raw).unwrap();
        let (album, tracks) = parse_itunes_album_response(6831539495, parsed).unwrap();
        assert_eq!(album.id, "al:it-6831539495");
        assert_eq!(album.name, "Remedy / Double Wings");
        assert_eq!(album.year, 2026);
        assert!(album.cover_url.contains("1000x1000bb.jpg"));
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[0].album_id(), "al:it-6831539495");
        assert_eq!(tracks[0].stream_url, "resolve:ytm:ASCA|Remedy|246");
        assert_eq!(tracks[1].duration, 219);
    }

    #[tokio::test]
    async fn interactive_throttle_does_not_stall_on_drained_bucket() {
        let src = ItunesSource::new();
        {
            let mut b = src.bucket.lock().unwrap();
            b.tokens = 0.0;
            b.last = Instant::now();
        }
        let t0 = Instant::now();
        src.throttle_interactive().await;
        assert!(
            t0.elapsed() < Duration::from_millis(200),
            "interactive lookup should not stall behind background sweep bucket drain"
        );
    }
}
