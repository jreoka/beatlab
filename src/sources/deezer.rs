use crate::models::{slug, AlbumSummary, ArtistSummary, Track};
use crate::sources::MusicSource;
use crate::taste::classify_moods_str;
use async_trait::async_trait;
use chrono::Utc;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{RwLock, Semaphore};
use tracing::warn;

/// Deezer allows 50 requests per 5 seconds per IP and bans IPs that keep exceeding it.
/// A token bucket keeps us well under that: short bursts of up to 25, then 5/s.
const DZ_BUCKET_CAPACITY: f64 = 25.0;
/// How far back the home "New Releases" shelf looks, in days.
pub const NEW_RELEASE_WINDOW_DAYS: i64 = 30;
const DZ_BUCKET_REFILL_PER_SEC: f64 = 5.0;
/// After Deezer refuses us outright (HTTP 403), stop calling it for a while instead of
/// hammering a blocked endpoint; callers fall back to other sources immediately.
const DZ_BLOCK_COOLDOWN: Duration = Duration::from_secs(15 * 60);
const DZ_JSON_CACHE_MAX: usize = 400;
const DZ_ALBUM_INFO_CACHE_MAX: usize = 8_000;

struct TokenBucket {
    tokens: f64,
    last: Instant,
}

#[derive(Clone)]
pub struct DeezerSource {
    http: reqwest::Client,
    http_sem: Arc<Semaphore>,
    genres_cache: Arc<RwLock<Option<(Instant, Vec<DeezerGenre>)>>>,
    json_cache: Arc<RwLock<HashMap<String, (Instant, serde_json::Value)>>>,
    /// Album genre/year never change, so they are cached for the life of the process.
    album_info_cache: Arc<std::sync::Mutex<HashMap<u64, (String, i32)>>>,
    bucket: Arc<std::sync::Mutex<TokenBucket>>,
    blocked_until: Arc<std::sync::Mutex<Option<Instant>>>,
    /// URLs with a background refresh in flight, so a burst of page loads on a stale
    /// entry triggers one upstream request instead of one per visitor.
    refreshing: Arc<std::sync::Mutex<HashSet<String>>>,
}

/// Search and chart results drift; entity lookups (albums, artists, playlists) barely do.
fn json_cache_ttl(url: &str) -> Duration {
    if url.contains("/search") || url.contains("/chart") || url.contains("/radio") {
        Duration::from_secs(60 * 60)
    } else {
        Duration::from_secs(6 * 60 * 60)
    }
}

/// Past its TTL an entry is still served (stale-while-revalidate) for this long while a
/// background task refreshes it, so pages never block on Deezer for data we already have.
const DZ_JSON_STALE_MAX: Duration = Duration::from_secs(7 * 24 * 60 * 60);

#[derive(Debug, Clone)]
pub struct DeezerGenre {
    pub id: u64,
    pub name: String,
    pub picture_url: String,
}

impl DeezerSource {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(12))
            .user_agent("Beatlab/0.3 (MusicDiscovery)")
            .build()
            .unwrap_or_default();
        Self {
            http,
            http_sem: Arc::new(Semaphore::new(4)),
            genres_cache: Arc::new(RwLock::new(None)),
            json_cache: Arc::new(RwLock::new(HashMap::new())),
            album_info_cache: Arc::new(std::sync::Mutex::new(HashMap::new())),
            bucket: Arc::new(std::sync::Mutex::new(TokenBucket {
                tokens: DZ_BUCKET_CAPACITY,
                last: Instant::now(),
            })),
            blocked_until: Arc::new(std::sync::Mutex::new(None)),
            refreshing: Arc::new(std::sync::Mutex::new(HashSet::new())),
        }
    }

    /// True while Deezer is refusing this server's requests (see `DZ_BLOCK_COOLDOWN`).
    pub fn is_blocked(&self) -> bool {
        let guard = self.blocked_until.lock().unwrap_or_else(|e| e.into_inner());
        guard.map_or(false, |until| Instant::now() < until)
    }

    fn mark_blocked(&self, url: &str) {
        let mut guard = self.blocked_until.lock().unwrap_or_else(|e| e.into_inner());
        let already = guard.map_or(false, |until| Instant::now() < until);
        *guard = Some(Instant::now() + DZ_BLOCK_COOLDOWN);
        if !already {
            warn!(
                url,
                cooldown_secs = DZ_BLOCK_COOLDOWN.as_secs(),
                "deezer is refusing requests (HTTP 403); pausing deezer calls"
            );
        }
    }

    async fn throttle(&self) {
        loop {
            let wait = {
                let mut b = self.bucket.lock().unwrap_or_else(|e| e.into_inner());
                let now = Instant::now();
                let refill = now.duration_since(b.last).as_secs_f64() * DZ_BUCKET_REFILL_PER_SEC;
                b.tokens = (b.tokens + refill).min(DZ_BUCKET_CAPACITY);
                b.last = now;
                if b.tokens >= 1.0 {
                    b.tokens -= 1.0;
                    None
                } else {
                    Some(Duration::from_secs_f64(
                        (1.0 - b.tokens) / DZ_BUCKET_REFILL_PER_SEC,
                    ))
                }
            };
            match wait {
                None => return,
                Some(d) => tokio::time::sleep(d).await,
            }
        }
    }

    async fn get_cached_json<T: DeserializeOwned>(&self, url: &str) -> Option<T> {
        let cached = {
            let guard = self.json_cache.read().await;
            guard
                .get(url)
                .filter(|(ts, _)| ts.elapsed() < DZ_JSON_STALE_MAX)
                .map(|(ts, v)| (ts.elapsed() < json_cache_ttl(url), v.clone()))
        };
        if let Some((fresh, value)) = cached {
            if let Ok(parsed) = serde_json::from_value::<T>(value) {
                if !fresh {
                    self.spawn_refresh(url);
                }
                return Some(parsed);
            }
        }
        None
    }

    async fn get_json<T: DeserializeOwned>(&self, url: &str) -> Option<T> {
        if let Some(parsed) = self.get_cached_json::<T>(url).await {
            return Some(parsed);
        }

        let val = self.fetch_and_cache(url).await?;
        serde_json::from_value::<T>(val).ok()
    }

    /// Re-fetch a stale cache entry in the background (at most one in flight per URL).
    fn spawn_refresh(&self, url: &str) {
        {
            let mut inflight = self.refreshing.lock().unwrap_or_else(|e| e.into_inner());
            if !inflight.insert(url.to_string()) {
                return;
            }
        }
        let this = self.clone();
        let url = url.to_string();
        tokio::spawn(async move {
            let _ = this.fetch_and_cache(&url).await;
            this.refreshing
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&url);
        });
    }

    /// Request `url` from Deezer (throttled, with retries) and store the JSON in the cache.
    async fn fetch_and_cache(&self, url: &str) -> Option<serde_json::Value> {
        for attempt in 0..3 {
            if self.is_blocked() {
                return None;
            }
            self.throttle().await;
            let permit = self.http_sem.acquire().await.ok();
            let resp = match self.http.get(url).send().await {
                Ok(r) => r,
                Err(e) => {
                    drop(permit);
                    if attempt < 1 {
                        tokio::time::sleep(Duration::from_millis(250)).await;
                        continue;
                    }
                    warn!(error = %e, url, "deezer request failed");
                    return None;
                }
            };
            let status = resp.status();
            if status.as_u16() == 429 {
                drop(permit);
                if attempt < 2 {
                    let wait_ms = 420 * (attempt as u64 + 1);
                    warn!(url, attempt, "deezer HTTP 429 rate limited; retrying");
                    tokio::time::sleep(Duration::from_millis(wait_ms)).await;
                    continue;
                }
                return None;
            }
            if status.as_u16() == 403 {
                drop(permit);
                self.mark_blocked(url);
                return None;
            }
            if !status.is_success() {
                drop(permit);
                warn!(status = %status, url, "deezer HTTP non-success");
                return None;
            }
            let val = match resp.json::<serde_json::Value>().await {
                Ok(v) => v,
                Err(e) => {
                    drop(permit);
                    warn!(error = %e, url, "deezer invalid JSON");
                    return None;
                }
            };
            drop(permit);
            if let Some(err_obj) = val.get("error") {
                let code = err_obj.get("code").and_then(|c| c.as_i64()).unwrap_or(0);
                let msg = err_obj
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("unknown deezer error");
                if code == 4 && attempt < 2 {
                    let wait_ms = 450 * (attempt as u64 + 1);
                    warn!(code, msg, url, attempt, "deezer quota limit; backing off");
                    tokio::time::sleep(Duration::from_millis(wait_ms)).await;
                    continue;
                }
                warn!(code, msg, url, "deezer API error");
                return None;
            }
            {
                let mut guard = self.json_cache.write().await;
                if guard.len() >= DZ_JSON_CACHE_MAX {
                    guard.retain(|_, (ts, _)| ts.elapsed() < DZ_JSON_STALE_MAX);
                    if guard.len() >= DZ_JSON_CACHE_MAX {
                        // Evict the oldest quarter, preferring non-chart entries so home/explore charts stay hot.
                        let mut ages: Vec<(bool, Instant, String)> = guard
                            .iter()
                            .map(|(k, (ts, _))| (k.contains("/chart/"), *ts, k.clone()))
                            .collect();
                        ages.sort_by_key(|(is_chart, ts, _)| (*is_chart, *ts));
                        for (_, _, k) in ages.into_iter().take(DZ_JSON_CACHE_MAX / 4) {
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

    async fn fetch_track_list(&self, url: &str, fallback_genre: &str) -> Vec<Track> {
        let Some(parsed) = self.get_json::<DzDataList<DzTrack>>(url).await else {
            return vec![];
        };
        parsed
            .data
            .into_iter()
            .filter_map(|d| d.into_track(None, "", 0, 0, fallback_genre))
            .collect()
    }

    pub async fn fetch_chart_tracks(&self, limit: usize) -> Vec<Track> {
        let url = format!("https://api.deezer.com/chart/0/tracks?limit={}", limit.clamp(5, 100));
        self.fetch_track_list(&url, "").await
    }

    /// Returns cached chart tracks without blocking on HTTP (spawns a background refresh if stale).
    pub async fn cached_chart_tracks(&self, limit: usize) -> Option<Vec<Track>> {
        let url = format!("https://api.deezer.com/chart/0/tracks?limit={}", limit.clamp(5, 100));
        let parsed = self.get_cached_json::<DzDataList<DzTrack>>(&url).await?;
        Some(
            parsed
                .data
                .into_iter()
                .filter_map(|d| d.into_track(None, "", 0, 0, ""))
                .collect(),
        )
    }

    pub async fn fetch_genre_chart_tracks(
        &self,
        genre_id: u64,
        genre_name: &str,
        limit: usize,
    ) -> Vec<Track> {
        let url = format!(
            "https://api.deezer.com/chart/{}/tracks?limit={}",
            genre_id,
            limit.clamp(5, 50)
        );
        self.fetch_track_list(&url, genre_name).await
    }

    pub async fn fetch_chart_albums(&self, limit: usize) -> Vec<AlbumSummary> {
        let url = format!("https://api.deezer.com/chart/0/albums?limit={}", limit.clamp(5, 60));
        let Some(parsed) = self.get_json::<DzDataList<DzAlbum>>(&url).await else {
            return vec![];
        };
        parsed
            .data
            .into_iter()
            .filter_map(|a| a.into_summary())
            .collect()
    }

    /// Returns cached chart albums without blocking on HTTP (spawns a background refresh if stale).
    pub async fn cached_chart_albums(&self, limit: usize) -> Option<Vec<AlbumSummary>> {
        let url = format!("https://api.deezer.com/chart/0/albums?limit={}", limit.clamp(5, 60));
        let parsed = self.get_cached_json::<DzDataList<DzAlbum>>(&url).await?;
        Some(parsed.data.into_iter().filter_map(|a| a.into_summary()).collect())
    }

    pub async fn fetch_chart_artists(&self, limit: usize) -> Vec<ArtistSummary> {
        let url = format!("https://api.deezer.com/chart/0/artists?limit={}", limit.clamp(5, 60));
        let Some(parsed) = self.get_json::<DzDataList<DzArtist>>(&url).await else {
            return vec![];
        };
        parsed
            .data
            .into_iter()
            .filter_map(|a| a.into_summary())
            .collect()
    }

    /// Returns cached chart artists without blocking on HTTP (spawns a background refresh if stale).
    pub async fn cached_chart_artists(&self, limit: usize) -> Option<Vec<ArtistSummary>> {
        let url = format!("https://api.deezer.com/chart/0/artists?limit={}", limit.clamp(5, 60));
        let parsed = self.get_cached_json::<DzDataList<DzArtist>>(&url).await?;
        Some(parsed.data.into_iter().filter_map(|a| a.into_summary()).collect())
    }

    pub async fn search_tracks(&self, query: &str, limit: usize) -> Vec<Track> {
        let url = format!(
            "https://api.deezer.com/search/track?q={}&limit={}",
            urlencoding::encode(query.trim()),
            limit.clamp(5, 50)
        );
        self.fetch_track_list(&url, "").await
    }

    pub async fn search_albums(&self, query: &str, limit: usize) -> Vec<AlbumSummary> {
        let url = format!(
            "https://api.deezer.com/search/album?q={}&limit={}",
            urlencoding::encode(query.trim()),
            limit.clamp(3, 30)
        );
        let Some(parsed) = self.get_json::<DzDataList<DzAlbum>>(&url).await else {
            return vec![];
        };
        parsed
            .data
            .into_iter()
            .filter_map(|a| a.into_summary())
            .collect()
    }

    pub async fn search_artists(&self, query: &str, limit: usize) -> Vec<ArtistSummary> {
        self.search_artists_checked(query, limit)
            .await
            .unwrap_or_default()
    }

    /// Like `search_artists`, but returns `None` when the upstream Deezer request
    /// failed or was blocked, so callers don't cache transient failures as misses.
    pub async fn search_artists_checked(
        &self,
        query: &str,
        limit: usize,
    ) -> Option<Vec<ArtistSummary>> {
        let url = format!(
            "https://api.deezer.com/search/artist?q={}&limit={}",
            urlencoding::encode(query.trim()),
            limit.clamp(3, 30)
        );
        let parsed = self.get_json::<DzDataList<DzArtist>>(&url).await?;
        Some(
            parsed
                .data
                .into_iter()
                .filter_map(|a| a.into_summary())
                .collect(),
        )
    }

    #[allow(dead_code)]
    pub async fn search_all(
        &self,
        query: &str,
        limit: usize,
    ) -> (Vec<Track>, Vec<AlbumSummary>, Vec<ArtistSummary>) {
        let (tracks, albums, artists) = tokio::join!(
            self.search_tracks(query, limit),
            self.search_albums(query, limit.min(12)),
            self.search_artists(query, limit.min(12)),
        );
        (tracks, albums, artists)
    }

    pub async fn fetch_album(&self, deezer_album_id: &str) -> Option<(AlbumSummary, Vec<Track>)> {
        let url = format!("https://api.deezer.com/album/{}", urlencoding::encode(deezer_album_id));
        let album: DzAlbumFull = self.get_json(&url).await?;
        if album.id == 0 || album.title.trim().is_empty() {
            return None;
        }
        let artist_dz_id = album.artist.as_ref().map(|a| a.id).unwrap_or(0);
        let artist_name = album
            .artist
            .as_ref()
            .map(|a| a.name.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Various Artists".to_string());
        let cover_url = album.best_cover();
        let genre = album.resolved_genre();
        let year = parse_year(&album.release_date);

        let fallback_album = DzTrackAlbum {
            id: album.id,
            title: album.title.clone(),
            cover_xl: album.cover_xl.clone(),
            cover_big: album.cover_big.clone(),
            cover_medium: album.cover_medium.clone(),
        };

        let raw_tracks = album.tracks.map(|t| t.data).unwrap_or_default();
        let mut tracks = Vec::with_capacity(raw_tracks.len());
        let mut total_dur = 0i32;
        for (idx, dt) in raw_tracks.into_iter().enumerate() {
            let mut tr = dt.into_track(Some(&fallback_album), &artist_name, artist_dz_id, year, &genre)?;
            if tr.track_no == 0 {
                tr.track_no = (idx + 1) as i32;
            }
            if tr.disc_no == 0 {
                tr.disc_no = 1;
            }
            if tr.cover_url.is_empty() {
                tr.cover_url = cover_url.clone();
            }
            total_dur += tr.duration.max(0);
            tracks.push(tr);
        }

        let artist_id = if artist_dz_id > 0 {
            format!("ar:dz:{}", artist_dz_id)
        } else {
            format!("ar:{}", slug(&artist_name))
        };
        let summary_id = if artist_dz_id > 0 {
            format!("al:dz-{}-{}", artist_dz_id, slug(&album.title))
        } else {
            tracks
                .first()
                .map(|t| t.album_id())
                .unwrap_or_else(|| format!("al:{}", slug(&format!("{}|{}", artist_name, album.title))))
        };
        let cover_id = tracks.first().map(|t| t.id.clone()).unwrap_or_else(|| summary_id.clone());

        let summary = AlbumSummary {
            id: summary_id,
            name: album.title,
            artist_id,
            artist: artist_name,
            cover_id,
            cover_url,
            genre,
            year,
            label: album.label,
            deezer_id: album.id.to_string(),
            song_count: tracks.len(),
            duration: total_dur,
            play_count: 0,
            last_played: 0,
            created: Utc::now().timestamp(),
        };
        Some((summary, tracks))
    }

    pub async fn fetch_artist(
        &self,
        deezer_artist_id: &str,
    ) -> Option<(ArtistSummary, Vec<Track>, Vec<AlbumSummary>, Vec<ArtistSummary>)> {
        let eid = urlencoding::encode(deezer_artist_id);
        let u_info = format!("https://api.deezer.com/artist/{}", eid);
        let u_top = format!("https://api.deezer.com/artist/{}/top?limit=50", eid);
        let u_albums = format!("https://api.deezer.com/artist/{}/albums?limit=50", eid);
        let u_related = format!("https://api.deezer.com/artist/{}/related?limit=12", eid);

        let (r_info, r_top, r_albums, r_related) = tokio::join!(
            self.get_json::<DzArtist>(&u_info),
            self.get_json::<DzDataList<DzTrack>>(&u_top),
            self.get_json::<DzDataList<DzAlbum>>(&u_albums),
            self.get_json::<DzDataList<DzArtist>>(&u_related),
        );

        let artist_obj: DzArtist = r_info?;
        let mut summary = artist_obj.clone().into_summary()?;

        let mut albums: Vec<AlbumSummary> = r_albums
            .map(|d| {
                d.data
                    .into_iter()
                    .filter_map(|mut a| {
                        if a.artist.is_none() {
                            a.artist = Some(artist_obj.clone());
                        }
                        a.into_summary()
                    })
                    .collect()
            })
            .unwrap_or_default();
        albums.sort_by(|a, b| b.year.cmp(&a.year).then_with(|| a.name.cmp(&b.name)));

        let mut album_by_dz_id: HashMap<u64, (i32, String)> = HashMap::new();
        let mut album_by_slug: HashMap<String, (i32, String)> = HashMap::new();
        let mut genre_freq: HashMap<String, usize> = HashMap::new();
        for alb in &albums {
            if let Ok(dz_id) = alb.deezer_id.parse::<u64>() {
                if dz_id > 0 {
                    album_by_dz_id.insert(dz_id, (alb.year, alb.genre.clone()));
                }
            }
            let s = slug(&alb.name);
            if !s.is_empty() {
                album_by_slug
                    .entry(s)
                    .or_insert_with(|| (alb.year, alb.genre.clone()));
            }
            if !alb.genre.trim().is_empty() {
                *genre_freq.entry(alb.genre.clone()).or_insert(0) += 1;
            }
        }
        let artist_fallback_genre = genre_freq
            .into_iter()
            .max_by_key(|(_, c)| *c)
            .map(|(g, _)| g)
            .unwrap_or_default();

        let top_tracks: Vec<Track> = r_top
            .map(|d| {
                d.data
                    .into_iter()
                    .filter_map(|t| {
                        let (alb_year, alb_genre) = t
                            .album
                            .as_ref()
                            .and_then(|a| {
                                album_by_dz_id
                                    .get(&a.id)
                                    .or_else(|| album_by_slug.get(&slug(&a.title)))
                            })
                            .cloned()
                            .unwrap_or((0, String::new()));
                        let use_genre = if !alb_genre.trim().is_empty() {
                            alb_genre.as_str()
                        } else {
                            artist_fallback_genre.as_str()
                        };
                        t.into_track(None, &summary.name, artist_obj.id, alb_year, use_genre)
                    })
                    .collect()
            })
            .unwrap_or_default();

        let related: Vec<ArtistSummary> = r_related
            .map(|d| d.data.into_iter().filter_map(|a| a.into_summary()).collect())
            .unwrap_or_default();

        summary.album_count = albums.len();
        summary.song_count = top_tracks.len();
        Some((summary, top_tracks, albums, related))
    }

    /// Fetches an artist's most recent, genuine releases (albums/EPs/singles),
    /// newest first. Powers the personalised "New Releases" home feed.
    /// Returns `None` when Deezer failed or was blocked, vs `Some(vec)` when
    /// Deezer answered (even if the artist has no releases inside the window).
    pub async fn fetch_artist_releases_checked(
        &self,
        deezer_artist_id: &str,
        artist_name: &str,
        limit: usize,
    ) -> Option<Vec<AlbumSummary>> {
        let Ok(dz_id) = deezer_artist_id.trim().parse::<u64>() else {
            return Some(Vec::new());
        };
        if dz_id == 0 {
            return Some(Vec::new());
        }
        // Deezer groups this list by type (albums, then EPs, then singles), not by
        // date, so read a full 100 to reach recent singles of prolific artists.
        let url = format!("https://api.deezer.com/artist/{}/albums?limit=100", dz_id);
        let parsed = self.get_json::<DzDataList<DzAlbum>>(&url).await?;
        let mut raw_albums = parsed.data;
        // If a prolific artist has >=100 albums/EPs before singles even begin, fetch
        // one extra page so their recent singles aren't cut off.
        if !parsed.next.trim().is_empty()
            && !raw_albums.is_empty()
            && raw_albums
                .last()
                .map(|a| !a.record_type.eq_ignore_ascii_case("single"))
                .unwrap_or(false)
        {
            let page2_url = format!(
                "https://api.deezer.com/artist/{}/albums?limit=100&index=100",
                dz_id
            );
            if let Some(page2) = self.get_json::<DzDataList<DzAlbum>>(&page2_url).await {
                raw_albums.extend(page2.data);
            }
        }

        // Only genuinely new drops: released within the release window (and not
        // far-future pre-announces; allow +1 day so release-day drops from ahead-of-UTC
        // timezones like Japan/Australia appear immediately). ISO dates compare lexicographically.
        let today = Utc::now().date_naive();
        let cutoff = (today - chrono::Duration::days(NEW_RELEASE_WINDOW_DAYS))
            .format("%Y-%m-%d")
            .to_string();
        let max_date = (today + chrono::Duration::days(1))
            .format("%Y-%m-%d")
            .to_string();
        let mut entries: Vec<(String, DzAlbum)> = raw_albums
            .into_iter()
            .filter(|a| a.id > 0)
            // Keep the artist's own releases, not compilations/appearances.
            .filter(|a| {
                a.artist
                    .as_ref()
                    .map(|ar| ar.id == dz_id)
                    .unwrap_or(true)
            })
            .map(|a| (a.release_date.clone(), a))
            .collect();
        // ISO dates (YYYY-MM-DD) sort lexicographically: newest first.
        entries.sort_by(|a, b| b.0.cmp(&a.0));

        let fallback_artist = DzArtist {
            id: dz_id,
            name: artist_name.trim().to_string(),
            ..Default::default()
        };
        let mut out: Vec<AlbumSummary> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for (_, mut album) in entries {
            let Some(date) = album.release_date.trim().get(..10).map(str::to_string) else {
                continue;
            };
            if date < cutoff || date > max_date {
                continue;
            }
            let released_ts = chrono::NaiveDate::parse_from_str(&date, "%Y-%m-%d")
                .ok()
                .and_then(|d| d.and_hms_opt(0, 0, 0))
                .map(|dt| dt.and_utc().timestamp());
            if album.artist.is_none() {
                album.artist = Some(fallback_artist.clone());
            }
            if let Some(mut summary) = album.into_summary() {
                if let Some(ts) = released_ts {
                    // Carry the exact release date so the feed sorts newest first.
                    summary.created = ts;
                }
                if seen.insert(summary.id.clone()) {
                    out.push(summary);
                }
            }
            if out.len() >= limit {
                break;
            }
        }
        Some(out)
    }

    /// Convenience wrapper around `fetch_artist_releases_checked`.
    #[allow(dead_code)]
    pub async fn fetch_artist_releases(
        &self,
        deezer_artist_id: &str,
        artist_name: &str,
        limit: usize,
    ) -> Vec<AlbumSummary> {
        self.fetch_artist_releases_checked(deezer_artist_id, artist_name, limit)
            .await
            .unwrap_or_default()
    }

    pub async fn fetch_genres(&self) -> Vec<DeezerGenre> {
        {
            let guard = self.genres_cache.read().await;
            if let Some((ts, cached)) = guard.as_ref() {
                if ts.elapsed() < Duration::from_secs(3600) && !cached.is_empty() {
                    return cached.clone();
                }
            }
        }
        let Some(parsed) = self
            .get_json::<DzDataList<DzGenreRaw>>("https://api.deezer.com/genre")
            .await
        else {
            return vec![];
        };
        let genres: Vec<DeezerGenre> = parsed
            .data
            .into_iter()
            .filter(|g| g.id > 0 && !g.name.trim().is_empty())
            .map(|g| DeezerGenre {
                id: g.id,
                name: g.name.trim().to_string(),
                picture_url: first_non_empty(&[g.picture_xl, g.picture_big, g.picture_medium]),
            })
            .collect();
        if !genres.is_empty() {
            let mut guard = self.genres_cache.write().await;
            *guard = Some((Instant::now(), genres.clone()));
        }
        genres
    }

    pub async fn fetch_artist_radio(&self, deezer_artist_id: &str) -> Vec<Track> {
        let url = format!(
            "https://api.deezer.com/artist/{}/radio",
            urlencoding::encode(deezer_artist_id)
        );
        self.fetch_track_list(&url, "").await
    }

    pub async fn fetch_artist_top_tracks(&self, deezer_artist_id: &str, limit: usize) -> Vec<Track> {
        let url = format!(
            "https://api.deezer.com/artist/{}/top?limit={}",
            urlencoding::encode(deezer_artist_id),
            limit.clamp(3, 25)
        );
        self.fetch_track_list(&url, "").await
    }

    pub async fn search_playlists(&self, query: &str, limit: usize) -> Vec<DzPlaylist> {
        let q = query.trim();
        if q.is_empty() {
            return vec![];
        }
        let url = format!(
            "https://api.deezer.com/search/playlist?q={}&limit={}",
            urlencoding::encode(q),
            limit.clamp(3, 25)
        );
        let Some(parsed) = self.get_json::<DzDataList<DzPlaylist>>(&url).await else {
            return vec![];
        };
        parsed
            .data
            .into_iter()
            .filter(|p| p.id > 0 && !p.title.trim().is_empty())
            .collect()
    }

    /// Deezer's search/playlist endpoints omit the album genre from track objects, so every
    /// remote candidate arrives with an empty `genre`. That makes genre-clash validation of
    /// curated instant mixes impossible (a mis-tagged "Jrock" playlist full of rap tracks
    /// would be trusted purely because its *title* matched). Fill the missing genre and release
    /// year from each distinct album's Deezer metadata (and artist album search fallback for
    /// non-Deezer tracks), then recompute moods. Results are cached by `get_json`.
    pub async fn enrich_track_genres(&self, tracks: &mut [Track]) {
        self.enrich_track_genres_with_timeout(tracks, Duration::from_secs(5))
            .await;
    }

    /// Bounded variant of `enrich_track_genres` used on interactive paths (radio / play)
    /// where partial results must still be preserved and applied even if one upstream
    /// album lookup is slow.
    pub async fn enrich_track_genres_with_timeout(&self, tracks: &mut [Track], budget: Duration) {
        if self.is_blocked() || tracks.is_empty() {
            return;
        }
        let started = Instant::now();
        let mut album_ids: Vec<u64> = tracks
            .iter()
            .filter(|t| t.genre.trim().is_empty() || t.year <= 0)
            .filter_map(|t| t.deezer_album_id().and_then(|id| id.parse::<u64>().ok()))
            .filter(|&id| id > 0)
            .collect();
        album_ids.sort_unstable();
        album_ids.dedup();

        // Album genre/year never change: answer from the process-wide cache first.
        let mut by_album: HashMap<u64, (String, i32)> = HashMap::new();
        {
            let cache = self.album_info_cache.lock().unwrap_or_else(|e| e.into_inner());
            album_ids.retain(|id| match cache.get(id) {
                Some(info) => {
                    by_album.insert(*id, info.clone());
                    false
                }
                None => true,
            });
        }
        // Bound the network work per call; uncached albums are picked up on later requests.
        album_ids.truncate(24);

        if !album_ids.is_empty() {
            let remaining = budget.saturating_sub(started.elapsed());
            if remaining >= Duration::from_millis(100) {
                let fetched = Arc::new(std::sync::Mutex::new(HashMap::<u64, (String, i32)>::new()));
                let lookup = async {
                    let futs = album_ids.into_iter().map(|id| {
                        let fetched = fetched.clone();
                        async move {
                            let url = format!("https://api.deezer.com/album/{}", id);
                            if let Some(a) = self.get_json::<DzAlbumFull>(&url).await {
                                let info = (a.resolved_genre(), parse_year(&a.release_date));
                                fetched
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner())
                                    .insert(id, info.clone());
                                let mut cache =
                                    self.album_info_cache.lock().unwrap_or_else(|e| e.into_inner());
                                if cache.len() >= DZ_ALBUM_INFO_CACHE_MAX {
                                    cache.clear();
                                }
                                cache.insert(id, info);
                            }
                        }
                    });
                    futures::future::join_all(futs).await;
                };
                let _ = tokio::time::timeout(remaining, lookup).await;
                let completed = std::mem::take(
                    &mut *fetched.lock().unwrap_or_else(|e| e.into_inner()),
                );
                by_album.extend(completed);
            }
        }

        if !by_album.is_empty() {
            for t in tracks.iter_mut() {
                if let Some((genre, year)) = t
                    .deezer_album_id()
                    .and_then(|id| id.parse::<u64>().ok())
                    .and_then(|id| by_album.get(&id))
                {
                    if t.genre.trim().is_empty() && !genre.is_empty() {
                        t.genre = genre.clone();
                    }
                    if t.year <= 0 && *year > 0 {
                        t.year = *year;
                    }
                }
            }
        }

        // Fallback for tracks (e.g. YTM / Soulseek or untagged singles) that still lack a genre
        // or release year: query Deezer's album search by primary artist name so `genre_id` and
        // exact album lookups fill them in.
        let mut missing_artists: Vec<String> = Vec::new();
        let mut seen_artists: HashSet<String> = HashSet::new();
        for t in tracks.iter() {
            // A track can already carry a genre and still be missing its release
            // year — lossless/local rips usually have tags but no DATE.
            if !t.genre.trim().is_empty() && t.year > 0 {
                continue;
            }
            let pa = crate::models::primary_artist(&t.artist);
            let key = slug(&pa);
            if !pa.is_empty() && !key.is_empty() && seen_artists.insert(key) {
                missing_artists.push(pa);
                if missing_artists.len() >= 6 {
                    break;
                }
            }
        }

        if !missing_artists.is_empty() && budget.saturating_sub(started.elapsed()) >= Duration::from_millis(200) {
            let remaining = budget
                .saturating_sub(started.elapsed())
                .min(Duration::from_secs(4));
            let artist_lookup = async {
                let futs = missing_artists.into_iter().map(|art| async move {
                    let albs = self.search_albums(&art, 10).await;
                    (slug(&art), albs)
                });
                futures::future::join_all(futs).await
            };
            let by_artist: HashMap<String, Vec<AlbumSummary>> =
                tokio::time::timeout(remaining, artist_lookup)
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|(_, albs)| !albs.is_empty())
                    .collect();

            // Deezer's `/search/album` omits `release_date`, so collect exact album matches
            // that still need a release year and resolve them via `/album/{id}` (cached).
            let mut extra_album_ids: Vec<u64> = Vec::new();
            let mut track_exact_dz_album: Vec<Option<u64>> = vec![None; tracks.len()];

            for (idx, t) in tracks.iter_mut().enumerate() {
                let needs_genre = t.genre.trim().is_empty();
                let needs_year = t.year <= 0;
                if !needs_genre && !needs_year {
                    continue;
                }
                let art_slug = slug(&crate::models::primary_artist(&t.artist));
                let Some(albs) = by_artist.get(&art_slug) else {
                    continue;
                };
                let matching: Vec<&AlbumSummary> = albs
                    .iter()
                    .filter(|a| slug(&crate::models::primary_artist(&a.artist)) == art_slug)
                    .collect();
                let pool: Vec<&AlbumSummary> = if matching.is_empty() {
                    albs.iter().collect()
                } else {
                    matching
                };
                let want_alb_slug = slug(&t.album);
                let exact_alb = (!want_alb_slug.is_empty())
                    .then(|| pool.iter().copied().find(|a| slug(&a.name) == want_alb_slug))
                    .flatten();
                if let Some(alb) = exact_alb {
                    if needs_genre && !alb.genre.trim().is_empty() {
                        t.genre = alb.genre.clone();
                    }
                    if t.year <= 0 && alb.year > 0 {
                        t.year = alb.year;
                    }
                    if t.year <= 0 {
                        if let Ok(dz_id) = alb.deezer_id.parse::<u64>() {
                            if dz_id > 0 {
                                track_exact_dz_album[idx] = Some(dz_id);
                                extra_album_ids.push(dz_id);
                            }
                        }
                    }
                }
                if t.genre.trim().is_empty() {
                    let mut freq: HashMap<&str, usize> = HashMap::new();
                    for a in &pool {
                        let g = a.genre.trim();
                        if !g.is_empty() {
                            *freq.entry(g).or_insert(0) += 1;
                        }
                    }
                    if let Some((best_g, _)) = freq.into_iter().max_by_key(|(_, c)| *c) {
                        t.genre = best_g.to_string();
                    }
                }
                if t.deezer_artist_id().is_none() {
                    if let Some(dz_aid) = pool
                        .iter()
                        .filter(|a| slug(&crate::models::primary_artist(&a.artist)) == art_slug)
                        .find_map(|a| a.artist_id.strip_prefix("ar:dz:"))
                    {
                        t.set_deezer_artist_id(dz_aid);
                    }
                }
            }

            if !extra_album_ids.is_empty() {
                extra_album_ids.sort_unstable();
                extra_album_ids.dedup();
                let mut resolved_extra: HashMap<u64, (String, i32)> = HashMap::new();
                {
                    let cache = self.album_info_cache.lock().unwrap_or_else(|e| e.into_inner());
                    extra_album_ids.retain(|id| match cache.get(id) {
                        Some(info) => {
                            resolved_extra.insert(*id, info.clone());
                            false
                        }
                        None => true,
                    });
                }
                extra_album_ids.truncate(8);
                let rem = budget.saturating_sub(started.elapsed());
                if !extra_album_ids.is_empty() && rem >= Duration::from_millis(150) {
                    let fetched = Arc::new(std::sync::Mutex::new(HashMap::<u64, (String, i32)>::new()));
                    let lookup = async {
                        let futs = extra_album_ids.into_iter().map(|id| {
                            let fetched = fetched.clone();
                            async move {
                                let url = format!("https://api.deezer.com/album/{}", id);
                                if let Some(a) = self.get_json::<DzAlbumFull>(&url).await {
                                    let info = (a.resolved_genre(), parse_year(&a.release_date));
                                    fetched
                                        .lock()
                                        .unwrap_or_else(|e| e.into_inner())
                                        .insert(id, info.clone());
                                    let mut cache = self
                                        .album_info_cache
                                        .lock()
                                        .unwrap_or_else(|e| e.into_inner());
                                    if cache.len() >= DZ_ALBUM_INFO_CACHE_MAX {
                                        cache.clear();
                                    }
                                    cache.insert(id, info);
                                }
                            }
                        });
                        futures::future::join_all(futs).await;
                    };
                    let _ = tokio::time::timeout(rem, lookup).await;
                    let completed = std::mem::take(
                        &mut *fetched.lock().unwrap_or_else(|e| e.into_inner()),
                    );
                    resolved_extra.extend(completed);
                }
                for (idx, t) in tracks.iter_mut().enumerate() {
                    if let Some(dz_id) = track_exact_dz_album[idx] {
                        if let Some((genre, year)) = resolved_extra.get(&dz_id) {
                            if t.genre.trim().is_empty() && !genre.is_empty() {
                                t.genre = genre.clone();
                            }
                            if t.year <= 0 && *year > 0 {
                                t.year = *year;
                            }
                        }
                    }
                }
            }
        }

        // Single-track / tiny-batch fallback for non-Deezer tracks (e.g. YTM radio items
        // that omitted both album and year): locate the matching Deezer track by song_key
        // and fetch its album's release date.
        if tracks.len() <= 3 && budget.saturating_sub(started.elapsed()) >= Duration::from_millis(250) {
            for t in tracks.iter_mut() {
                if t.year > 0 || t.deezer_album_id().is_some() {
                    continue;
                }
                let rem = budget.saturating_sub(started.elapsed());
                if rem < Duration::from_millis(250) {
                    break;
                }
                let pa = crate::models::primary_artist(&t.artist);
                let clean_title = crate::models::strip_release_tags(&t.title);
                let q = format!("{} {}", pa, clean_title).trim().to_string();
                if q.is_empty() {
                    continue;
                }
                let want_key = t.song_key();
                let candidates = tokio::time::timeout(rem, self.search_tracks(&q, 5))
                    .await
                    .unwrap_or_default();
                let matched_album_id = candidates
                    .into_iter()
                    .find(|c| c.song_key() == want_key)
                    .and_then(|c| c.deezer_album_id())
                    .and_then(|id| id.parse::<u64>().ok())
                    .filter(|&id| id > 0);
                if let Some(dz_id) = matched_album_id {
                    let cached = self
                        .album_info_cache
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .get(&dz_id)
                        .cloned();
                    let info = if let Some(hit) = cached {
                        Some(hit)
                    } else {
                        let rem2 = budget.saturating_sub(started.elapsed());
                        if rem2 >= Duration::from_millis(150) {
                            let url = format!("https://api.deezer.com/album/{}", dz_id);
                            tokio::time::timeout(rem2, self.get_json::<DzAlbumFull>(&url))
                                .await
                                .ok()
                                .flatten()
                                .map(|a| {
                                    let info = (a.resolved_genre(), parse_year(&a.release_date));
                                    let mut cache = self
                                        .album_info_cache
                                        .lock()
                                        .unwrap_or_else(|e| e.into_inner());
                                    if cache.len() >= DZ_ALBUM_INFO_CACHE_MAX {
                                        cache.clear();
                                    }
                                    cache.insert(dz_id, info.clone());
                                    info
                                })
                        } else {
                            None
                        }
                    };
                    if let Some((genre, year)) = info {
                        if t.genre.trim().is_empty() && !genre.is_empty() {
                            t.genre = genre;
                        }
                        if t.year <= 0 && year > 0 {
                            t.year = year;
                        }
                    }
                }
            }
        }

        for t in tracks.iter_mut() {
            if !t.genre.trim().is_empty() {
                let classified = classify_moods_str(&t.title, &t.album, &t.genre, "");
                if !classified.is_empty() || t.moods.trim().is_empty() {
                    t.moods = classified;
                }
            }
        }
    }

    pub async fn fetch_playlist_tracks(&self, playlist_id: u64, limit: usize) -> Vec<Track> {
        if playlist_id == 0 {
            return vec![];
        }
        let url = format!(
            "https://api.deezer.com/playlist/{}/tracks?limit={}",
            playlist_id,
            limit.clamp(5, 60)
        );
        self.fetch_track_list(&url, "").await
    }

    /// Searches public Deezer playlists matching `query` and concurrently fetches their tracks
    /// to fuel ephemeral on-the-fly thematic/era mixes.
    pub async fn fetch_curated_mix_tracks(
        &self,
        query: &str,
        max_playlists: usize,
        per_playlist: usize,
    ) -> Vec<(String, Vec<Track>)> {
        let playlists = self.search_playlists(query, 15).await;
        let q_low = query.to_lowercase();
        let mut scored: Vec<(i32, DzPlaylist)> = playlists
            .into_iter()
            .filter_map(|p| {
                if crate::sources::has_unrequested_scene_qualifier(&p.title, &q_low) {
                    return None;
                }
                if !is_usable_mix_playlist(&p, &q_low) {
                    return None;
                }
                let rel = playlist_title_relevance_score(&p.title, &q_low)?;
                let is_editorial = p
                    .user
                    .as_ref()
                    .map(|u| u.name.trim().to_ascii_lowercase().starts_with("deezer"))
                    .unwrap_or(false);
                let extra = crate::sources::extra_title_words(&p.title, &q_low);
                if !is_editorial && extra > 3 {
                    return None;
                }
                let mut score = rel - 6 * (extra as i32);
                if is_editorial {
                    score += 25;
                }
                if p.nb_tracks > 0 && p.nb_tracks < 12 {
                    score -= 15;
                }
                Some((score, p))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0));
        let mut seen_ids = HashSet::new();
        let mut seen_titles = HashSet::new();
        let chosen: Vec<DzPlaylist> = scored
            .into_iter()
            .map(|(_, p)| p)
            .filter(|p| {
                let norm = p.title.trim().to_lowercase();
                seen_ids.insert(p.id) && seen_titles.insert(norm)
            })
            .take(max_playlists.max(1))
            .collect();
        if chosen.is_empty() {
            return vec![];
        }
        let futs = chosen
            .iter()
            .map(|p| self.fetch_playlist_tracks(p.id, per_playlist));
        let batches = futures::future::join_all(futs).await;
        chosen
            .into_iter()
            .zip(batches)
            .filter(|(_, tracks)| !tracks.is_empty())
            .map(|(p, tracks)| (p.title, tracks))
            .collect()
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct DzPlaylistUser {
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct DzPlaylist {
    #[serde(default)]
    pub id: u64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub nb_tracks: usize,
    #[serde(default)]
    pub user: Option<DzPlaylistUser>,
}

fn is_usable_mix_playlist(p: &DzPlaylist, query_low: &str) -> bool {
    if p.nb_tracks > 0 && p.nb_tracks < 5 {
        return false;
    }
    let title_low = p.title.to_lowercase();
    for bad in [
        "karaoke",
        "tribute",
        "parody",
        "ringtone",
        "8-bit",
        "8 bit",
        "lullaby",
        "lullabies",
        "instrumental cover",
        "music box",
        "type beat",
        "slowed",
        "nightcore",
        "8d audio",
        "sped up",
        "white noise",
        "binaural",
        "asmr",
        "tantric",
        "massage",
        "meditation",
        "meditate",
        "zen ",
        "spa ",
        "sound bath",
        "sound healing",
        "singing bowl",
        "mindfulness",
        "new age",
        "reiki",
        "chakra",
        "yoga",
        "setlist",
        "rockabilly",
        "line dance",
        "square dance",
        "ballroom",
        "polka",
        "waltz",
        "zumba",
        "doo-wop",
        "doowop",
        "disney",
        "nursery",
        "toddler",
        "wedding",
    ] {
        if title_low.contains(bad) && !query_low.contains(bad) {
            return false;
        }
    }
    for term in ["kids", "children", "baby", "twist", "swing", "christmas", "halloween"] {
        if crate::taste::contains_term(&title_low, term)
            && !crate::taste::contains_term(query_low, term)
        {
            return false;
        }
    }
    if (crate::taste::contains_term(&title_low, "cover")
        || crate::taste::contains_term(&title_low, "covers"))
        && !query_low.contains("cover")
    {
        return false;
    }
    if crate::taste::contains_term(&title_low, "instrumental")
        && ![
            "instrumental",
            "ambient",
            "study",
            "jazz",
            "fusion",
            "classical",
            "lofi",
            "lo-fi",
            "soundtrack",
            "post-rock",
            "synthwave",
            "piano",
            "guitar",
            "chillhop",
        ]
        .iter()
        .any(|ok| query_low.contains(ok))
    {
        return false;
    }
    if crate::taste::contains_term(&title_low, "sex") && !query_low.contains("sex") {
        return false;
    }
    true
}

/// Source-agnostic playlist filter + relevance score shared by Deezer and YouTube Music
/// curated-mix lookups. `None` means the playlist must not be used for `query_low`.
pub(crate) fn score_mix_playlist(title: &str, nb_tracks: usize, query_low: &str) -> Option<i32> {
    let probe = DzPlaylist {
        id: 0,
        title: title.to_string(),
        nb_tracks,
        user: None,
    };
    if !is_usable_mix_playlist(&probe, query_low) {
        return None;
    }
    playlist_title_relevance_score(title, query_low)
}

fn normalize_playlist_terms(s: &str) -> String {
    s.to_lowercase()
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
        .replace("fifties", "50s")
}

fn playlist_title_relevance_score(title: &str, query_low: &str) -> Option<i32> {
    let title_norm = normalize_playlist_terms(title);
    let q_norm = normalize_playlist_terms(query_low);

    let decades = ["50s", "60s", "70s", "80s", "90s", "00s", "10s", "20s"];
    let query_decades: Vec<&str> = decades
        .iter()
        .copied()
        .filter(|d| crate::taste::contains_term(&q_norm, d))
        .collect();
    if !query_decades.is_empty() {
        let matches_query_dec = query_decades
            .iter()
            .any(|d| crate::taste::contains_term(&title_norm, d));
        let mentions_other_dec = decades
            .iter()
            .any(|d| !query_decades.contains(d) && crate::taste::contains_term(&title_norm, d));
        if mentions_other_dec && !matches_query_dec {
            return None;
        }
    } else if ["50s", "60s"]
        .iter()
        .any(|d| crate::taste::contains_term(&title_norm, d))
    {
        return None;
    }

    let generic = [
        "mix", "playlist", "radio", "hits", "hit", "best", "top", "greatest", "songs", "tracks",
        "music", "the", "of", "and", "for", "a", "an", "vibes", "vibe", "classic", "classics",
    ];
    let content_tokens: Vec<&str> = q_norm
        .split(|c: char| !c.is_alphanumeric() && c != '-' && c != '&')
        .filter(|tok| !tok.is_empty() && !generic.contains(tok))
        .collect();

    if content_tokens.is_empty() {
        return Some(1);
    }

    let intent = crate::taste::parse_search_intent(query_low);
    if !intent.genres.is_empty() {
        if !intent
            .genres
            .iter()
            .any(|g| crate::taste::genre_matches_text(&title_norm, g))
        {
            return None;
        }
        if crate::taste::has_conflicting_scene_genre(&title_norm, &intent.genres, &q_norm) {
            return None;
        }
    }

    // If the query explicitly asks for a regional/language scene (e.g. "japanese jazz fusion",
    // "korean indie", "brazilian jazz"), reject generic playlists that omit that scene
    // (like "Jazz Classics").
    let required_scenes = crate::taste::requested_scene_families(&q_norm);
    for fam in &required_scenes {
        if !crate::taste::scene_family_matches_text(fam, &title_norm) {
            return None;
        }
    }

    let mut score = 0i32;
    let mut matched = 0usize;
    for tok in &content_tokens {
        if crate::taste::playlist_token_matches(&title_norm, tok) {
            matched += 1;
            score += 20;
        }
    }

    if matched == 0 {
        return None;
    }
    if matched == content_tokens.len() {
        score += 15;
    }
    if title_norm.contains(&q_norm) {
        score += 10;
    }
    Some(score)
}

#[async_trait]
impl MusicSource for DeezerSource {
    fn name(&self) -> &'static str {
        "deezer"
    }

    fn available(&self) -> bool {
        true
    }

    fn status(&self) -> String {
        "deezer catalog — rich metadata, charts, albums & artists".to_string()
    }

    async fn search(&self, query: &str, limit: usize) -> Vec<Track> {
        self.search_tracks(query, limit).await
    }
}

fn null_to_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(bound(deserialize = "T: Deserialize<'de>"))]
struct DzDataList<T> {
    #[serde(default, deserialize_with = "null_to_default")]
    data: Vec<T>,
    #[serde(default, deserialize_with = "null_to_default")]
    next: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct DzGenreRaw {
    #[serde(default)]
    id: u64,
    #[serde(default)]
    name: String,
    #[serde(default)]
    picture_xl: String,
    #[serde(default)]
    picture_big: String,
    #[serde(default)]
    picture_medium: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct DzArtist {
    #[serde(default, deserialize_with = "null_to_default")]
    id: u64,
    #[serde(default, deserialize_with = "null_to_default")]
    name: String,
    #[serde(default, deserialize_with = "null_to_default")]
    picture_xl: String,
    #[serde(default, deserialize_with = "null_to_default")]
    picture_big: String,
    #[serde(default, deserialize_with = "null_to_default")]
    picture_medium: String,
    #[serde(default, deserialize_with = "null_to_default")]
    nb_album: usize,
    #[serde(default, deserialize_with = "null_to_default")]
    nb_fan: usize,
}

impl DzArtist {
    fn into_summary(self) -> Option<ArtistSummary> {
        let name = self.name.trim().to_string();
        if name.is_empty() || self.id == 0 {
            return None;
        }
        let cover_url = first_non_empty(&[self.picture_xl, self.picture_big, self.picture_medium]);
        let bio = if self.nb_fan > 0 {
            format!("{} fans on Deezer", self.nb_fan)
        } else {
            String::new()
        };
        Some(ArtistSummary {
            id: format!("ar:dz:{}", self.id),
            name,
            cover_url,
            bio,
            deezer_id: self.id.to_string(),
            album_count: self.nb_album,
            song_count: 0,
        })
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
struct DzTrackAlbum {
    #[serde(default)]
    id: u64,
    #[serde(default)]
    title: String,
    #[serde(default)]
    cover_xl: String,
    #[serde(default)]
    cover_big: String,
    #[serde(default)]
    cover_medium: String,
}

impl DzTrackAlbum {
    fn best_cover(&self) -> String {
        first_non_empty(&[
            self.cover_xl.clone(),
            self.cover_big.clone(),
            self.cover_medium.clone(),
        ])
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
struct DzAlbum {
    #[serde(default, deserialize_with = "null_to_default")]
    id: u64,
    #[serde(default, deserialize_with = "null_to_default")]
    title: String,
    #[serde(default, deserialize_with = "null_to_default")]
    cover_xl: String,
    #[serde(default, deserialize_with = "null_to_default")]
    cover_big: String,
    #[serde(default, deserialize_with = "null_to_default")]
    cover_medium: String,
    #[serde(default, deserialize_with = "null_to_default")]
    release_date: String,
    #[serde(default, deserialize_with = "null_to_default")]
    record_type: String,
    #[serde(default, deserialize_with = "null_to_default")]
    genre_id: i64,
    #[serde(default, deserialize_with = "null_to_default")]
    nb_tracks: usize,
    #[serde(default)]
    artist: Option<DzArtist>,
}

impl DzAlbum {
    fn into_summary(self) -> Option<AlbumSummary> {
        let name = self.title.trim().to_string();
        if name.is_empty() || self.id == 0 {
            return None;
        }
        let artist_dz_id = self.artist.as_ref().map(|a| a.id).unwrap_or(0);
        let artist = self
            .artist
            .map(|a| a.name.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Unknown Artist".to_string());
        let cover_url = first_non_empty(&[self.cover_xl, self.cover_big, self.cover_medium]);
        let artist_id = if artist_dz_id > 0 {
            format!("ar:dz:{}", artist_dz_id)
        } else {
            format!("ar:{}", slug(&artist))
        };
        let aid = if artist_dz_id > 0 {
            format!("al:dz-{}-{}", artist_dz_id, slug(&name))
        } else {
            format!("al:{}", slug(&format!("{}|{}", artist, name)))
        };
        Some(AlbumSummary {
            id: aid.clone(),
            name,
            artist_id,
            artist,
            cover_id: aid,
            cover_url,
            genre: deezer_genre_name(self.genre_id).to_string(),
            year: parse_year(&self.release_date),
            label: String::new(),
            deezer_id: self.id.to_string(),
            song_count: self.nb_tracks,
            duration: 0,
            play_count: 0,
            last_played: 0,
            created: Utc::now().timestamp(),
        })
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
struct DzAlbumFull {
    #[serde(default)]
    id: u64,
    #[serde(default)]
    title: String,
    #[serde(default)]
    cover_xl: String,
    #[serde(default)]
    cover_big: String,
    #[serde(default)]
    cover_medium: String,
    #[serde(default)]
    label: String,
    #[serde(default)]
    release_date: String,
    #[serde(default)]
    genre_id: i64,
    #[serde(default)]
    artist: Option<DzArtist>,
    #[serde(default)]
    genres: Option<DzDataList<DzGenreName>>,
    #[serde(default)]
    tracks: Option<DzDataList<DzTrack>>,
}

impl DzAlbumFull {
    fn best_cover(&self) -> String {
        first_non_empty(&[
            self.cover_xl.clone(),
            self.cover_big.clone(),
            self.cover_medium.clone(),
        ])
    }

    fn resolved_genre(&self) -> String {
        let mut names: Vec<String> = Vec::new();
        if let Some(ref g) = self.genres {
            for item in &g.data {
                let raw = item.name.trim();
                if raw.is_empty() || raw.eq_ignore_ascii_case("all") {
                    continue;
                }
                let mapped = if raw.eq_ignore_ascii_case("films/games") {
                    "Soundtrack"
                } else {
                    raw
                };
                if !names.iter().any(|existing| existing.eq_ignore_ascii_case(mapped)) {
                    names.push(mapped.to_string());
                }
            }
        }
        if names.is_empty() {
            let fallback = deezer_genre_name(self.genre_id);
            if !fallback.is_empty() {
                return fallback.to_string();
            }
        }
        names.join(", ")
    }
}

pub fn deezer_genre_name(genre_id: i64) -> &'static str {
    match genre_id {
        2 => "Afrobeats",
        16 => "Asian Music",
        65 => "Regional Mexican, Latin",
        67 => "Salsa, Latin",
        71 => "Cumbia, Latin",
        75 => "Brazilian Music",
        81 => "Indian Music",
        84 => "Country",
        85 => "Alternative",
        95 => "Kids",
        98 => "Classical",
        106 => "Electro",
        113 => "Dance",
        116 => "Rap/Hip Hop",
        122 => "Reggaeton",
        129 => "Jazz",
        132 => "Pop",
        144 => "Reggae",
        152 => "Rock",
        153 => "Blues",
        165 => "R&B",
        169 => "Soul & Funk",
        173 => "Soundtrack",
        186 => "Christian",
        197 => "Latin Music",
        464 => "Metal",
        466 => "Folk",
        _ => "",
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
struct DzGenreName {
    #[serde(default)]
    name: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct DzTrack {
    #[serde(default)]
    id: u64,
    #[serde(default)]
    title: String,
    #[serde(default)]
    duration: i32,
    #[serde(default)]
    rank: u64,
    #[serde(default)]
    explicit_lyrics: bool,
    #[serde(default)]
    track_position: i32,
    #[serde(default)]
    disk_number: i32,
    #[serde(default)]
    release_date: String,
    #[serde(default)]
    artist: Option<DzArtist>,
    #[serde(default)]
    album: Option<DzTrackAlbum>,
}

impl DzTrack {
    fn into_track(
        self,
        fallback_album: Option<&DzTrackAlbum>,
        fallback_artist: &str,
        fallback_artist_id: u64,
        fallback_year: i32,
        fallback_genre: &str,
    ) -> Option<Track> {
        let title = self.title.trim().to_string();
        if self.id == 0 || title.is_empty() {
            return None;
        }
        let artist_dz_id = self
            .artist
            .as_ref()
            .map(|a| a.id)
            .filter(|&id| id > 0)
            .unwrap_or(fallback_artist_id);
        let artist = self
            .artist
            .map(|a| a.name.trim().to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| (!fallback_artist.is_empty()).then(|| fallback_artist.to_string()))
            .unwrap_or_else(|| "Unknown Artist".to_string());
        let alb_ref = self.album.as_ref().or(fallback_album);
        let album = alb_ref
            .map(|a| a.title.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| title.clone());
        let cover_url = alb_ref.map(|a| a.best_cover()).unwrap_or_default();
        let year = parse_year(&self.release_date).max(fallback_year);
        let genre = fallback_genre.to_string();
        let moods = classify_moods_str(&title, &album, &genre, "");

        let meta = serde_json::json!({
            "deezer_track_id": self.id,
            "deezer_album_id": alb_ref.map(|a| a.id).unwrap_or(0),
            "deezer_artist_id": artist_dz_id,
            "rank": self.rank,
            "explicit": self.explicit_lyrics,
        })
        .to_string();

        Some(Track {
            id: format!("dz:{}", self.id),
            source: "deezer".to_string(),
            title,
            artist: artist.clone(),
            album,
            genre,
            moods,
            year,
            track_no: self.track_position.max(0),
            disc_no: if self.disk_number > 0 { self.disk_number } else { 1 },
            duration: self.duration.max(0),
            bitrate: 256,
            sample_rate: 44100,
            bit_depth: 0,
            format: "m4a".to_string(),
            size: 0,
            // Always resolve full audio via YouTube Music / Local / Archive, never Deezer's 30s preview!
            stream_url: format!("resolve:ytm:{}|{}|{}", artist, self.title.trim(), self.duration),
            cover_url,
            meta,
            deezer_id: self.id.to_string(),
            created: Utc::now().timestamp(),
        })
    }
}

fn parse_year(s: &str) -> i32 {
    s.trim()
        .get(..4)
        .and_then(|y| y.parse::<i32>().ok())
        .unwrap_or(0)
}

fn first_non_empty(candidates: &[String]) -> String {
    candidates
        .iter()
        .find(|s| !s.trim().is_empty())
        .cloned()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playlist_relevance_rejects_mismatched_eras_and_unrelated_titles() {
        assert!(playlist_title_relevance_score("80s Hits", "80s").is_some());
        assert!(playlist_title_relevance_score("Best of the 80's", "80s").is_some());
        assert!(playlist_title_relevance_score("Best of 2020s", "80s").is_none());
        assert!(playlist_title_relevance_score("90s Rock Classics", "90s rock").is_some());
        assert!(playlist_title_relevance_score("Workout Mix", "90s rock").is_none());
    }

    #[test]
    fn playlist_filter_rejects_covers_lullabies_and_spam() {
        let mk = |title: &str, nb_tracks: usize| DzPlaylist {
            id: 1,
            title: title.into(),
            nb_tracks,
            user: None,
        };
        assert!(!is_usable_mix_playlist(&mk("Sad Covers", 60), "sad songs"));
        assert!(!is_usable_mix_playlist(&mk("Piano lullabies", 40), "ambient"));
        assert!(!is_usable_mix_playlist(
            &mk("BAISE Stimulation Tantric Pure Massage Sex ChillOut DownTempo", 68),
            "chillout downtempo"
        ));
        assert!(!is_usable_mix_playlist(&mk("Upbeat INSTRUMENTAL POP HITS", 45), "upbeat hits"));
        assert!(is_usable_mix_playlist(
            &mk("Ambient Instrumentals - Atmospheric Vibes", 121),
            "ambient instrumental"
        ));
        assert!(is_usable_mix_playlist(&mk("Melancholy Indie Folk", 45), "melancholy indie"));
    }

    #[test]
    fn playlist_relevance_matches_hyphenated_genres_and_rejects_scene_clashes() {
        assert!(playlist_title_relevance_score("Top J-Pop", "jpop").is_some());
        assert!(playlist_title_relevance_score("J-Pop Essentials", "jpop").is_some());
        assert!(playlist_title_relevance_score("KPOP x JPOP mix", "jpop").is_none());
        assert!(playlist_title_relevance_score("MIX J-POP / K-POP", "j-pop").is_none());
        assert!(playlist_title_relevance_score("Top K-Pop", "kpop").is_some());
        assert!(playlist_title_relevance_score("Top J-Rock", "jrock").is_some());
        assert!(playlist_title_relevance_score("Pop Hits", "city pop").is_none());
        assert!(playlist_title_relevance_score("80s Japanese City Pop", "city pop").is_some());

        // Regional jazz queries must reject generic non-Japanese jazz playlists, while
        // accepting equivalent Japan/Japanese/J-Jazz titles.
        assert!(playlist_title_relevance_score("Jazz Classics", "japanese jazz fusion").is_none());
        assert!(playlist_title_relevance_score("Smooth Jazz", "japanese jazz fusion").is_none());
        assert!(playlist_title_relevance_score("Japanese Jazz Fusion", "japanese jazz fusion").is_some());
        assert!(playlist_title_relevance_score("Japan Jazz Fusion", "japanese jazz fusion").is_some());
        assert!(playlist_title_relevance_score("J-Jazz Kissaten Essentials", "japanese jazz fusion").is_some());
        assert!(!crate::sources::has_unrequested_scene_qualifier(
            "Japan Jazz Fusion",
            "japanese jazz fusion"
        ));
        assert!(!crate::sources::has_unrequested_scene_qualifier(
            "Japanese Jazz Fusion",
            "j-jazz"
        ));
        assert!(crate::sources::has_unrequested_scene_qualifier(
            "Japanese 80s Hits",
            "80s hits"
        ));
    }

    #[test]
    fn dz_album_deserializes_with_null_fields() {
        let raw = r#"{
            "data": [
                {
                    "id": 1140167251,
                    "title": "Remedy / Double Wings",
                    "cover_xl": null,
                    "cover_big": "https://cdn.example/big.jpg",
                    "cover_medium": null,
                    "release_date": "2026-11-18",
                    "record_type": null,
                    "genre_id": null,
                    "nb_tracks": null,
                    "artist": null
                }
            ],
            "next": null
        }"#;
        let parsed: DzDataList<DzAlbum> = serde_json::from_str(raw).unwrap();
        assert_eq!(parsed.data.len(), 1);
        assert_eq!(parsed.data[0].id, 1140167251);
        assert_eq!(parsed.data[0].release_date, "2026-11-18");
        assert!(parsed.next.is_empty());
    }

    #[tokio::test]
    async fn enrich_track_genres_with_timeout_fills_year_from_cached_album_info() {
        let dz = DeezerSource::new();
        {
            let mut cache = dz.album_info_cache.lock().unwrap();
            cache.insert(424242, ("J-Pop".to_string(), 2011));
        }
        let mut tracks = vec![Track {
            id: "dz:9001".into(),
            source: "deezer".into(),
            title: "Perfect Day".into(),
            artist: "Supercell".into(),
            album: "Today Is A Beautiful Day".into(),
            year: 0,
            meta: r#"{"deezer_track_id":9001,"deezer_album_id":424242,"deezer_artist_id":55}"#
                .into(),
            ..Default::default()
        }];

        dz.enrich_track_genres_with_timeout(&mut tracks, Duration::from_secs(2))
            .await;

        assert_eq!(tracks[0].year, 2011);
        assert_eq!(tracks[0].genre, "J-Pop");
        assert!(!tracks[0].moods.is_empty());
    }
}
