use crate::models::{slug, AlbumSummary, ArtistSummary, Track};
use crate::sources::MusicSource;
use crate::taste::classify_moods_str;
use async_trait::async_trait;
use chrono::Utc;
use serde::Deserialize;
use std::time::Duration;

#[derive(Clone)]
pub struct DeezerSource {
    http: reqwest::Client,
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
        Self { http }
    }

    async fn fetch_track_list(&self, url: &str, fallback_genre: &str) -> Vec<Track> {
        let Ok(resp) = self.http.get(url).send().await else {
            return vec![];
        };
        let Ok(parsed) = resp.json::<DzDataList<DzTrack>>().await else {
            return vec![];
        };
        parsed
            .data
            .into_iter()
            .filter_map(|d| d.into_track(None, "", 0, fallback_genre))
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
        let Ok(resp) = self.http.get(&url).send().await else {
            return vec![];
        };
        let Ok(parsed) = resp.json::<DzDataList<DzAlbum>>().await else {
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
        let Ok(resp) = self.http.get(&url).send().await else {
            return vec![];
        };
        let Ok(parsed) = resp.json::<DzDataList<DzArtist>>().await else {
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
        let Ok(resp) = self.http.get(&url).send().await else {
            return vec![];
        };
        let Ok(parsed) = resp.json::<DzDataList<DzTrack>>().await else {
            return vec![];
        };
        parsed
            .data
            .into_iter()
            .filter_map(|d| d.into_track(None, "", 0, ""))
            .collect()
    }

    pub async fn search_albums(&self, query: &str, limit: usize) -> Vec<AlbumSummary> {
        let url = format!(
            "https://api.deezer.com/search/album?q={}&limit={}",
            urlencoding::encode(query.trim()),
            limit.clamp(3, 30)
        );
        let Ok(resp) = self.http.get(&url).send().await else {
            return vec![];
        };
        let Ok(parsed) = resp.json::<DzDataList<DzAlbum>>().await else {
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
        let Ok(resp) = self.http.get(&url).send().await else {
            return vec![];
        };
        let Ok(parsed) = resp.json::<DzDataList<DzArtist>>().await else {
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
        let resp = self.http.get(&url).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let album: DzAlbumFull = resp.json().await.ok()?;
        if album.id == 0 || album.title.trim().is_empty() {
            return None;
        }
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
            let mut tr = dt.into_track(Some(&fallback_album), &artist_name, year, &genre)?;
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

        let summary_id = tracks
            .first()
            .map(|t| t.album_id())
            .unwrap_or_else(|| format!("al:{}", slug(&format!("{}|{}", artist_name, album.title))));
        let cover_id = tracks.first().map(|t| t.id.clone()).unwrap_or_else(|| summary_id.clone());

        let summary = AlbumSummary {
            id: summary_id,
            name: album.title,
            artist_id: format!("ar:{}", slug(&artist_name)),
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
        let u_top = format!("https://api.deezer.com/artist/{}/top?limit=20", eid);
        let u_albums = format!("https://api.deezer.com/artist/{}/albums?limit=30", eid);
        let u_related = format!("https://api.deezer.com/artist/{}/related?limit=12", eid);

        let (r_info, r_top, r_albums, r_related) = tokio::join!(
            self.http.get(&u_info).send(),
            self.http.get(&u_top).send(),
            self.http.get(&u_albums).send(),
            self.http.get(&u_related).send(),
        );

        let artist_obj: DzArtist = r_info.ok()?.json().await.ok()?;
        let mut summary = artist_obj.clone().into_summary()?;

        let top_tracks: Vec<Track> = match r_top {
            Ok(resp) => resp
                .json::<DzDataList<DzTrack>>()
                .await
                .map(|d| {
                    d.data
                        .into_iter()
                        .filter_map(|t| t.into_track(None, &summary.name, 0, ""))
                        .collect()
                })
                .unwrap_or_default(),
            Err(_) => vec![],
        };

        let mut albums: Vec<AlbumSummary> = match r_albums {
            Ok(resp) => resp
                .json::<DzDataList<DzAlbum>>()
                .await
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
                .unwrap_or_default(),
            Err(_) => vec![],
        };
        albums.sort_by(|a, b| b.year.cmp(&a.year).then_with(|| a.name.cmp(&b.name)));

        let related: Vec<ArtistSummary> = match r_related {
            Ok(resp) => resp
                .json::<DzDataList<DzArtist>>()
                .await
                .map(|d| d.data.into_iter().filter_map(|a| a.into_summary()).collect())
                .unwrap_or_default(),
            Err(_) => vec![],
        };

        summary.album_count = albums.len();
        summary.song_count = top_tracks.len();
        Some((summary, top_tracks, albums, related))
    }

    pub async fn fetch_genres(&self) -> Vec<DeezerGenre> {
        let Ok(resp) = self.http.get("https://api.deezer.com/genre").send().await else {
            return vec![];
        };
        let Ok(parsed) = resp.json::<DzDataList<DzGenreRaw>>().await else {
            return vec![];
        };
        parsed
            .data
            .into_iter()
            .filter(|g| g.id > 0 && !g.name.trim().is_empty())
            .map(|g| DeezerGenre {
                id: g.id,
                name: g.name.trim().to_string(),
                picture_url: first_non_empty(&[g.picture_xl, g.picture_big, g.picture_medium]),
            })
            .collect()
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
        let Ok(resp) = self.http.get(&url).send().await else {
            return vec![];
        };
        let Ok(parsed) = resp.json::<DzDataList<DzPlaylist>>().await else {
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
        let Ok(resp) = self.http.get(&url).send().await else {
            return vec![];
        };
        let Ok(parsed) = resp.json::<DzDataList<DzTrack>>().await else {
            return vec![];
        };
        parsed
            .data
            .into_iter()
            .filter_map(|d| d.into_track(None, "", 0, ""))
            .collect()
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
        let chosen: Vec<DzPlaylist> = playlists
            .into_iter()
            .filter(|p| is_usable_mix_playlist(&p, &q_low))
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
        "instrumental cover",
        "music box",
        "type beat",
        "slowed",
        "nightcore",
        "8d audio",
    ] {
        if title_low.contains(bad) && !query_low.contains(bad) {
            return false;
        }
    }
    true
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
            id: format!("ar:{}", slug(&name)),
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
        let artist = self
            .artist
            .map(|a| a.name.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Unknown Artist".to_string());
        let cover_url = first_non_empty(&[self.cover_xl, self.cover_big, self.cover_medium]);
        let aid = format!("al:{}", slug(&format!("{}|{}", artist, name)));
        Some(AlbumSummary {
            id: aid.clone(),
            name,
            artist_id: format!("ar:{}", slug(&artist)),
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
        fallback_year: i32,
        fallback_genre: &str,
    ) -> Option<Track> {
        let title = self.title.trim().to_string();
        if self.id == 0 || title.is_empty() {
            return None;
        }
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
