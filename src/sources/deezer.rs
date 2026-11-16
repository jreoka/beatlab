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

#[derive(Clone)]
pub struct DeezerSource {
    http: reqwest::Client,
    http_sem: Arc<Semaphore>,
    genres_cache: Arc<RwLock<Option<(Instant, Vec<DeezerGenre>)>>>,
    json_cache: Arc<RwLock<HashMap<String, (Instant, serde_json::Value)>>>,
}

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
        }
    }

    async fn get_json<T: DeserializeOwned>(&self, url: &str) -> Option<T> {
        {
            let guard = self.json_cache.read().await;
            if let Some((ts, cached)) = guard.get(url) {
                if ts.elapsed() < Duration::from_secs(600) {
                    if let Ok(parsed) = serde_json::from_value::<T>(cached.clone()) {
                        return Some(parsed);
                    }
                }
            }
        }

        for attempt in 0..3 {
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
            if let Ok(parsed) = serde_json::from_value::<T>(val.clone()) {
                let mut guard = self.json_cache.write().await;
                if guard.len() >= 300 {
                    guard.retain(|_, (ts, _)| ts.elapsed() < Duration::from_secs(600));
                    if guard.len() >= 300 {
                        guard.clear();
                    }
                }
                guard.insert(url.to_string(), (Instant::now(), val));
                return Some(parsed);
            }
            return None;
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
        let url = format!(
            "https://api.deezer.com/search/artist?q={}&limit={}",
            urlencoding::encode(query.trim()),
            limit.clamp(3, 30)
        );
        let Some(parsed) = self.get_json::<DzDataList<DzArtist>>(&url).await else {
            return vec![];
        };
        parsed
            .data
            .into_iter()
            .filter_map(|a| a.into_summary())
            .collect()
    }

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
        let genre = album
            .genres
            .as_ref()
            .and_then(|g| g.data.first())
            .map(|g| g.name.trim().to_string())
            .unwrap_or_default();
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

        let top_tracks: Vec<Track> = r_top
            .map(|d| {
                d.data
                    .into_iter()
                    .filter_map(|t| t.into_track(None, &summary.name, artist_obj.id, 0, ""))
                    .collect()
            })
            .unwrap_or_default();

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

        let related: Vec<ArtistSummary> = r_related
            .map(|d| d.data.into_iter().filter_map(|a| a.into_summary()).collect())
            .unwrap_or_default();

        summary.album_count = albums.len();
        summary.song_count = top_tracks.len();
        Some((summary, top_tracks, albums, related))
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
        let playlists = self.search_playlists(query, 10).await;
        let q_low = query.to_lowercase();
        let mut scored: Vec<(i32, DzPlaylist)> = playlists
            .into_iter()
            .filter_map(|p| {
                if !is_usable_mix_playlist(&p, &q_low) {
                    return None;
                }
                let rel = playlist_title_relevance_score(&p.title, &q_low)?;
                Some((rel, p))
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
pub struct DzPlaylist {
    #[serde(default)]
    pub id: u64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub nb_tracks: usize,
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
        "setlist",
    ] {
        if title_low.contains(bad) && !query_low.contains(bad) {
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
        && !query_low.contains("instrumental")
        && !query_low.contains("focus")
        && !query_low.contains("ambient")
        && !query_low.contains("study")
    {
        return false;
    }
    if crate::taste::contains_term(&title_low, "sex") && !query_low.contains("sex") {
        return false;
    }
    true
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

#[derive(Debug, Clone, Default, Deserialize)]
struct DzDataList<T> {
    #[serde(default)]
    data: Vec<T>,
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
    #[serde(default)]
    nb_album: usize,
    #[serde(default)]
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
    release_date: String,
    #[serde(default)]
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
            genre: String::new(),
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
        };
        assert!(!is_usable_mix_playlist(&mk("Sad Covers", 60), "sad songs"));
        assert!(!is_usable_mix_playlist(&mk("Piano lullabies", 40), "deep focus"));
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
    }
}
