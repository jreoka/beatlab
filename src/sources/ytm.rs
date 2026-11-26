use crate::models::{core_song_title, err_msg, primary_artist, slug, upgrade_cover_url, AppResult, Track};
use crate::sources::MusicSource;
use crate::taste::classify_moods_str;
use async_trait::async_trait;
use chrono::Utc;
use serde_json::Value;
use std::process::Stdio;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::process::Command;
use tokio::sync::Semaphore;
use tracing::warn;

const INNERTUBE_SEARCH_URL: &str = "https://music.youtube.com/youtubei/v1/search?prettyPrint=false";
const INNERTUBE_NEXT_URL: &str = "https://music.youtube.com/youtubei/v1/next?prettyPrint=false";
const INNERTUBE_PLAYER_URL: &str = "https://www.youtube.com/youtubei/v1/player?prettyPrint=false";
const YTM_SONGS_PARAMS: &str = "EgWKAQIIAWoKEAMQBBAJEAoQBQ%3D%3D";
/// Search filter: playlists curated by YouTube Music itself ("Anime Hits", "Top J-Rock"...).
const YTM_FEATURED_PLAYLISTS_PARAMS: &str = "EgeKAQQoADgBagwQDhAKEAMQBRAJEAQ%3D";
/// Search filter: user-made playlists. Noisier; only used when no featured playlist fits.
const YTM_COMMUNITY_PLAYLISTS_PARAMS: &str = "EgeKAQQoAEABagoQAxAEEAoQCRAF";

#[derive(Debug, Clone)]
pub struct YtmPlaylistHit {
    pub playlist_id: String,
    pub title: String,
    pub song_count: usize,
}

#[derive(Debug, Clone)]
pub struct ResolvedStream {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub ext: String,
    pub source_webpage_url: String,
    pub has_video: bool,
}

#[derive(Debug, Clone, Default)]
pub struct YtmCandidate {
    pub video_id: String,
    pub title: String,
    pub artist: String,
    pub artist_id: String,
    pub album: String,
    pub year: i32,
    pub duration: i32,
    pub cover_url: String,
}

#[derive(Clone)]
pub struct YtmSource {
    http: reqwest::Client,
    ytdlp_bin: String,
    ytdlp_sem: Arc<Semaphore>,
    /// Index into the yt-dlp client attempts that last succeeded. When YouTube
    /// breaks one player client, every play used to wait for it to fail first.
    preferred_client: Arc<AtomicUsize>,
}

impl YtmSource {
    pub fn new(ytdlp_override: Option<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(12))
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36")
            .build()
            .unwrap_or_default();
        let ytdlp_bin = ytdlp_override
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(detect_ytdlp_bin);
        Self {
            http,
            ytdlp_bin,
            ytdlp_sem: Arc::new(Semaphore::new(4)),
            preferred_client: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn ytdlp_bin(&self) -> &str {
        &self.ytdlp_bin
    }

    /// Fetches YouTube's pre-computed ITU-R BS.1770 integrated loudness (LKFS/LUFS)
    /// for a video ID via Innertube player endpoints (~120ms, zero audio bytes
    /// downloaded) so cold streams start at exact normalized volume from 0:00.
    /// Falls back from `ANDROID` to `WEB_REMIX` when a client omits `audioConfig`.
    pub async fn fetch_video_loudness(
        &self,
        video_id: &str,
    ) -> Option<crate::loudness::TrackLoudness> {
        let vid = video_id.trim().trim_start_matches("ytm:").trim();
        if vid.is_empty() || vid.len() > 32 || vid.contains('/') || vid.contains('?') {
            return None;
        }
        let clients: &[(&str, &str, &str)] = &[
            (INNERTUBE_PLAYER_URL, "ANDROID", "20.10.38"),
            (
                "https://music.youtube.com/youtubei/v1/player?prettyPrint=false",
                "WEB_REMIX",
                "1.20250115.01.00",
            ),
        ];
        for &(url, client_name, client_version) in clients {
            let body = serde_json::json!({
                "context": {
                    "client": {
                        "clientName": client_name,
                        "clientVersion": client_version,
                        "hl": "en",
                        "gl": "US"
                    }
                },
                "videoId": vid
            });
            let mut req = self
                .http
                .post(url)
                .header("Content-Type", "application/json");
            if client_name == "WEB_REMIX" {
                req = req
                    .header("Origin", "https://music.youtube.com")
                    .header("Referer", "https://music.youtube.com/");
            }
            let Ok(Ok(resp)) =
                tokio::time::timeout(Duration::from_millis(2500), req.json(&body).send()).await
            else {
                continue;
            };
            if !resp.status().is_success() {
                continue;
            }
            let Ok(Ok(val)) =
                tokio::time::timeout(Duration::from_millis(2000), resp.json::<Value>()).await
            else {
                continue;
            };
            if let Some(m) = parse_innertube_audio_config_loudness(&val) {
                return Some(m);
            }
        }
        None
    }

    async fn innertube_post(&self, url: &str, body: Value) -> Option<Value> {
        let resp = match self
            .http
            .post(url)
            .header("Content-Type", "application/json")
            .header("Origin", "https://music.youtube.com")
            .header("Referer", "https://music.youtube.com/")
            .json(&body)
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => {
                warn!(status = %r.status(), url, "youtube music innertube non-success");
                return None;
            }
            Err(e) => {
                warn!(error = %e, url, "youtube music innertube error");
                return None;
            }
        };
        resp.json::<Value>().await.ok()
    }

    async fn search_innertube_checked(
        &self,
        query: &str,
        songs_only: bool,
        limit: usize,
    ) -> Option<Vec<YtmCandidate>> {
        let q = query.trim();
        if q.is_empty() {
            return Some(vec![]);
        }
        let mut body = serde_json::json!({
            "context": innertube_client_context(),
            "query": q
        });
        if songs_only {
            body["params"] = Value::String(YTM_SONGS_PARAMS.to_string());
        }

        let val = self.innertube_post(INNERTUBE_SEARCH_URL, body).await?;
        let mut out = parse_innertube_results(&val);
        out.truncate(limit);
        Some(out)
    }

    pub async fn search_innertube(
        &self,
        query: &str,
        songs_only: bool,
        limit: usize,
    ) -> Vec<YtmCandidate> {
        self.search_innertube_checked(query, songs_only, limit)
            .await
            .unwrap_or_default()
    }

    /// Search YouTube Music playlists: featured (editorial) ones, or user-made ones.
    pub async fn search_playlists(&self, query: &str, featured: bool, limit: usize) -> Vec<YtmPlaylistHit> {
        let q = query.trim();
        if q.is_empty() {
            return vec![];
        }
        let params = if featured {
            YTM_FEATURED_PLAYLISTS_PARAMS
        } else {
            YTM_COMMUNITY_PLAYLISTS_PARAMS
        };
        let body = serde_json::json!({
            "context": innertube_client_context(),
            "query": q,
            "params": params,
        });
        let Some(val) = self.innertube_post(INNERTUBE_SEARCH_URL, body).await else {
            return vec![];
        };
        let mut out = parse_playlist_search_results(&val);
        out.truncate(limit);
        out
    }

    /// Fetch the tracks of a YouTube Music playlist (first page, ~25 tracks).
    pub async fn fetch_playlist_tracks(&self, playlist_id: &str, limit: usize) -> Vec<Track> {
        let pid = playlist_id.trim().trim_start_matches("VL");
        if pid.is_empty() {
            return vec![];
        }
        let body = serde_json::json!({
            "context": innertube_client_context(),
            "playlistId": pid,
            "isAudioOnly": true
        });
        let Some(val) = self.innertube_post(INNERTUBE_NEXT_URL, body).await else {
            return vec![];
        };
        let now = Utc::now().timestamp();
        parse_innertube_radio_results(&val, "")
            .into_iter()
            .filter_map(|c| candidate_to_track(c, now))
            .take(limit)
            .collect()
    }

    /// Fetch official YouTube Music "Up Next" / Track Radio mix tracks seeded by `video_id`.
    pub async fn fetch_radio_tracks(&self, video_id: &str, limit: usize) -> Vec<Track> {
        let vid = video_id.trim().trim_start_matches("ytm:").trim();
        if vid.is_empty() {
            return vec![];
        }

        let mut cands = Vec::new();
        for playlist_prefix in ["RDAMVM", "RD"] {
            let body = serde_json::json!({
                "context": innertube_client_context(),
                "videoId": vid,
                "playlistId": format!("{}{}", playlist_prefix, vid),
                "isAudioOnly": true
            });
            if let Some(val) = self.innertube_post(INNERTUBE_NEXT_URL, body).await {
                cands = parse_innertube_radio_results(&val, vid);
                if !cands.is_empty() {
                    break;
                }
            }
        }

        let now = Utc::now().timestamp();
        cands
            .into_iter()
            .filter_map(|c| candidate_to_track(c, now))
            .take(limit)
            .collect()
    }

    /// Find the best YouTube Music video ID matching `(artist, title, duration)`.
    pub async fn find_best_video_for_track(
        &self,
        artist: &str,
        title: &str,
        target_duration: i32,
    ) -> Option<String> {
        let pa = primary_artist(artist);
        let clean_artist = if pa.is_empty() { artist.trim() } else { pa.as_str() };
        let clean_title = crate::models::strip_release_tags(title);
        let q = format!("{} {}", clean_artist, clean_title).trim().to_string();

        let mut cands = self.search_innertube(&q, true, 8).await;
        if cands.is_empty() {
            cands = self.search_innertube(&q, false, 8).await;
        }
        if !cands.is_empty() {
            cands.sort_by_key(|c| {
                std::cmp::Reverse(score_ytm_candidate(c, clean_artist, title, target_duration))
            });
            return Some(cands.into_iter().next()?.video_id);
        }

        // Last-resort yt-dlp flat search if Innertube HTTP is temporarily rate-limited.
        self.ytdlp_flat_search(&q, clean_artist, title, target_duration)
            .await
    }

    async fn ytdlp_flat_search(
        &self,
        query: &str,
        artist: &str,
        title: &str,
        target_duration: i32,
    ) -> Option<String> {
        let _permit = tokio::time::timeout(Duration::from_secs(10), self.ytdlp_sem.acquire())
            .await
            .ok()?
            .ok()?;
        let search_arg = format!("ytsearch4:{} audio", query);
        let mut cmd = Command::new(&self.ytdlp_bin);
        cmd.kill_on_drop(true);
        cmd.args([
            "-j",
            "--flat-playlist",
            "--no-warnings",
            "--no-check-certificates",
            &search_arg,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

        let out = tokio::time::timeout(Duration::from_secs(12), cmd.output())
            .await
            .ok()?
            .ok()?;
        let stdout = String::from_utf8_lossy(&out.stdout);
        let mut cands = Vec::new();
        for line in stdout.lines() {
            if let Ok(v) = serde_json::from_str::<Value>(line) {
                let vid = v.get("id").and_then(|s| s.as_str()).unwrap_or("");
                if vid.is_empty() {
                    continue;
                }
                cands.push(YtmCandidate {
                    video_id: vid.to_string(),
                    title: v
                        .get("title")
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string(),
                    artist: v
                        .get("uploader")
                        .or_else(|| v.get("channel"))
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string(),
                    artist_id: v
                        .get("channel_id")
                        .or_else(|| v.get("uploader_id"))
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string(),
                    album: String::new(),
                    year: 0,
                    duration: v.get("duration").and_then(|d| d.as_f64()).unwrap_or(0.0) as i32,
                    cover_url: String::new(),
                });
            }
        }
        cands.sort_by_key(|c| std::cmp::Reverse(score_ytm_candidate(c, artist, title, target_duration)));
        cands.into_iter().next().map(|c| c.video_id)
    }

    /// Resolve a playable HTTP audio URL + headers for a YouTube video ID or URL.
    pub async fn resolve_stream(&self, video_id_or_url: &str) -> AppResult<ResolvedStream> {
        let watch_url = if video_id_or_url.starts_with("http://")
            || video_id_or_url.starts_with("https://")
        {
            video_id_or_url.to_string()
        } else {
            let vid = video_id_or_url.trim_start_matches("ytm:");
            format!("https://www.youtube.com/watch?v={}", vid)
        };

        // Prefer audio-only streams (`vcodec=none`) across every client before ever
        // falling back to a muxed video+audio format (e.g. progressive MP4 itag 18).
        // Chrome on Android pauses `<audio>` elements on screen lock whenever the
        // demuxed container reports a video track (`HasVideo() == true`).
        let clients = [
            "youtube:player_client=android",
            "youtube:player_client=ios",
            "",
        ];
        let audio_only_fmt = "bestaudio[vcodec=none][protocol!=m3u8][protocol!=m3u8_native]/bestaudio[protocol!=m3u8][protocol!=m3u8_native]";
        let fallback_fmt = "bestaudio[protocol!=m3u8][protocol!=m3u8_native]/best[protocol!=m3u8][protocol!=m3u8_native]/best";

        let mut last_err = String::new();
        let order = client_attempt_order(self.preferred_client.load(Ordering::Relaxed), clients.len());

        // Pass 1: try to obtain a pure audio-only direct stream from any client.
        let mut first_muxed: Option<(usize, ResolvedStream)> = None;
        for &idx in &order {
            let extractor_args = clients[idx];
            match self.run_ytdlp_json(&watch_url, extractor_args, audio_only_fmt).await {
                Ok(res) if !res.has_video => {
                    self.preferred_client.store(idx, Ordering::Relaxed);
                    return Ok(res);
                }
                Ok(res) => {
                    if first_muxed.is_none() {
                        first_muxed = Some((idx, res));
                    }
                }
                Err(e) => {
                    warn!(client = extractor_args, error = %e, "yt-dlp audio-only resolution attempt failed");
                    last_err = e.to_string();
                }
            }
        }
        if let Some((idx, res)) = first_muxed {
            self.preferred_client.store(idx, Ordering::Relaxed);
            return Ok(res);
        }

        // Pass 2: allow muxed fallback (tagged `has_video = true` so the engine strips video via ffmpeg `-vn`).
        for &idx in &order {
            let extractor_args = clients[idx];
            let fmt = if extractor_args.is_empty() {
                "bestaudio/best"
            } else {
                fallback_fmt
            };
            match self.run_ytdlp_json(&watch_url, extractor_args, fmt).await {
                Ok(res) => {
                    self.preferred_client.store(idx, Ordering::Relaxed);
                    return Ok(res);
                }
                Err(e) => {
                    warn!(client = extractor_args, error = %e, "yt-dlp fallback resolution attempt failed");
                    last_err = e.to_string();
                }
            }
        }
        Err(err_msg(format!("yt-dlp resolution failed: {}", last_err)))
    }

    async fn run_ytdlp_json(
        &self,
        watch_url: &str,
        extractor_args: &str,
        format_sel: &str,
    ) -> AppResult<ResolvedStream> {
        let _permit = tokio::time::timeout(Duration::from_secs(12), self.ytdlp_sem.acquire())
            .await
            .map_err(|_| err_msg("Timed out waiting for yt-dlp concurrency slot"))?
            .map_err(|_| err_msg("yt-dlp semaphore closed"))?;
        let mut cmd = Command::new(&self.ytdlp_bin);
        cmd.kill_on_drop(true);
        cmd.arg("-j").arg("--no-playlist").arg("--no-warnings");
        if !extractor_args.is_empty() {
            cmd.arg("--extractor-args").arg(extractor_args);
        }
        cmd.arg("-f").arg(format_sel).arg(watch_url);
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        let out = tokio::time::timeout(Duration::from_secs(18), cmd.output())
            .await
            .map_err(|_| err_msg("yt-dlp timed out"))??;

        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            return Err(err_msg(if err.is_empty() {
                "yt-dlp returned non-zero status".to_string()
            } else {
                err
            }));
        }

        let line = String::from_utf8_lossy(&out.stdout);
        let first_json = line
            .lines()
            .find(|l| l.trim_start().starts_with('{'))
            .ok_or_else(|| err_msg("yt-dlp produced empty JSON"))?;
        let v: Value = serde_json::from_str(first_json)?;

        let url = v
            .get("url")
            .and_then(|u| u.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if url.is_empty() || (!url.starts_with("http://") && !url.starts_with("https://")) {
            return Err(err_msg("yt-dlp returned non-HTTP stream URL"));
        }

        let mut headers = Vec::new();
        if let Some(obj) = v.get("http_headers").and_then(|h| h.as_object()) {
            for (k, val) in obj {
                if let Some(s) = val.as_str() {
                    headers.push((k.clone(), s.to_string()));
                }
            }
        }
        let ext = v
            .get("ext")
            .and_then(|s| s.as_str())
            .unwrap_or("m4a")
            .to_lowercase();
        let has_video = parse_ytdlp_has_video(&v);

        Ok(ResolvedStream {
            url,
            headers,
            ext,
            source_webpage_url: watch_url.to_string(),
            has_video,
        })
    }
}

fn parse_ytdlp_has_video(v: &Value) -> bool {
    v.get("vcodec")
        .and_then(|s| s.as_str())
        .map(str::trim)
        .is_some_and(|vc| !vc.is_empty() && !vc.eq_ignore_ascii_case("none"))
}

#[async_trait]
impl MusicSource for YtmSource {
    fn name(&self) -> &'static str {
        "ytm"
    }

    fn available(&self) -> bool {
        true
    }

    fn status(&self) -> String {
        format!("youtube music — fast innertube search + {}", self.ytdlp_bin)
    }

    async fn search(&self, query: &str, limit: usize) -> Vec<Track> {
        let items = match self.search_innertube_checked(query, true, limit).await {
            Some(v) if !v.is_empty() => v,
            Some(_) => self.search_innertube(query, false, limit).await,
            None => return vec![],
        };
        let now = Utc::now().timestamp();
        items
            .into_iter()
            .filter_map(|c| candidate_to_track(c, now))
            .collect()
    }
}

fn parse_innertube_audio_config_loudness(val: &Value) -> Option<crate::loudness::TrackLoudness> {
    let audio_cfg = val.pointer("/playerConfig/audioConfig")?;
    let target_lkfs = audio_cfg
        .get("loudnessTargetLkfs")
        .and_then(|v| v.as_f64())
        .filter(|v| v.is_finite() && *v <= -6.0 && *v >= -30.0)
        .unwrap_or(-14.0);
    let lufs = audio_cfg
        .get("trackAbsoluteLoudnessLkfs")
        .and_then(|v| v.as_f64())
        .filter(|v| v.is_finite() && *v < 0.0 && *v > -70.0)
        .or_else(|| {
            audio_cfg
                .get("perceptualLoudnessDb")
                .and_then(|v| v.as_f64())
                .filter(|v| v.is_finite() && *v < 0.0 && *v > -70.0)
        })
        .or_else(|| {
            audio_cfg
                .get("loudnessDb")
                .and_then(|v| v.as_f64())
                .filter(|v| v.is_finite())
                .map(|d| target_lkfs + d)
        })?;
    let m = crate::loudness::TrackLoudness {
        lufs: (lufs * 100.0).round() / 100.0,
        true_peak_db: 0.0,
        exact: true,
    };
    m.is_valid().then_some(m)
}

fn candidate_to_track(c: YtmCandidate, created: i64) -> Option<Track> {
    let dur = c.duration.max(0);
    if dur > 0 && (dur < 35 || dur > 1500) {
        // Skip tiny snippets and multi-hour mixes.
        return None;
    }
    let artist = if c.artist.is_empty() {
        "Unknown Artist".to_string()
    } else {
        c.artist
    };
    let album = if c.album.is_empty() {
        c.title.clone()
    } else {
        c.album
    };
    let moods = classify_moods_str(&c.title, &album, "", "");
    let cover_url = upgrade_cover_url(&c.cover_url);
    let watch_url = format!("https://music.youtube.com/watch?v={}", c.video_id);
    let meta = if c.artist_id.trim().is_empty() {
        watch_url
    } else {
        serde_json::json!({
            "ytm_video_id": c.video_id,
            "ytm_artist_id": c.artist_id.trim(),
            "webpage_url": watch_url,
        })
        .to_string()
    };
    Some(Track {
        id: format!("ytm:{}", c.video_id),
        source: "ytm".to_string(),
        title: c.title,
        artist,
        album,
        genre: String::new(),
        moods,
        year: c.year,
        track_no: 0,
        disc_no: 1,
        duration: dur,
        bitrate: 192,
        sample_rate: 44100,
        bit_depth: 0,
        format: "m4a".to_string(),
        size: 0,
        stream_url: format!("ytm:{}", c.video_id),
        cover_url,
        meta,
        deezer_id: String::new(),
        created,
    })
}

fn innertube_client_context() -> Value {
    serde_json::json!({
        "client": {
            "clientName": "WEB_REMIX",
            "clientVersion": "1.20241127.01.00",
            "hl": "en",
            "gl": "US"
        }
    })
}

fn detect_ytdlp_bin() -> String {
    if cfg!(windows) {
        for cand in ["yt-dlp.exe", "yt-dlp"] {
            if std::process::Command::new(cand)
                .arg("--version")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok()
            {
                return cand.to_string();
            }
        }
        "yt-dlp".to_string()
    } else {
        "yt-dlp".to_string()
    }
}

pub fn parse_innertube_results(root: &Value) -> Vec<YtmCandidate> {
    let mut raw_renderers = Vec::new();
    collect_objects_by_key(root, "musicResponsiveListItemRenderer", &mut raw_renderers);
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for item in raw_renderers {
        if let Some(c) = extract_candidate(item) {
            if seen.insert(c.video_id.clone()) {
                out.push(c);
            }
        }
    }
    out
}

fn parse_playlist_search_results(root: &Value) -> Vec<YtmPlaylistHit> {
    let mut items = Vec::new();
    collect_objects_by_key(root, "musicResponsiveListItemRenderer", &mut items);
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for item in items {
        let Some(browse_id) = item
            .pointer("/navigationEndpoint/browseEndpoint/browseId")
            .and_then(|b| b.as_str())
        else {
            continue;
        };
        if !browse_id.starts_with("VL") {
            continue;
        }
        let column_text = |idx: usize| {
            item.pointer(&format!(
                "/flexColumns/{}/musicResponsiveListItemFlexColumnRenderer/text/runs",
                idx
            ))
            .and_then(join_runs_text)
            .unwrap_or_default()
        };
        let title = column_text(0);
        if title.trim().is_empty() {
            continue;
        }
        // Subtitle looks like "YouTube Music • 50 songs" (featured) or "Owner • 1.2M views".
        let song_count = column_text(1)
            .split('•')
            .find_map(|part| {
                let p = part.trim().to_lowercase();
                p.strip_suffix(" songs")
                    .or_else(|| p.strip_suffix(" song"))
                    .and_then(|n| n.replace(',', "").trim().parse::<usize>().ok())
            })
            .unwrap_or(0);
        let playlist_id = browse_id.trim_start_matches("VL").to_string();
        if seen.insert(playlist_id.clone()) {
            out.push(YtmPlaylistHit {
                playlist_id,
                title,
                song_count,
            });
        }
    }
    out
}

fn parse_innertube_radio_results(root: &Value, seed_video_id: &str) -> Vec<YtmCandidate> {
    let mut raw_renderers = Vec::new();
    collect_objects_by_key(root, "playlistPanelVideoRenderer", &mut raw_renderers);
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for item in raw_renderers {
        if let Some(c) = extract_panel_candidate(item, seed_video_id) {
            if seen.insert(c.video_id.clone()) {
                out.push(c);
            }
        }
    }
    out
}

fn collect_objects_by_key<'a>(v: &'a Value, key: &str, out: &mut Vec<&'a Value>) {
    match v {
        Value::Object(map) => {
            if let Some(r) = map.get(key) {
                out.push(r);
            }
            for child in map.values() {
                collect_objects_by_key(child, key, out);
            }
        }
        Value::Array(arr) => {
            for child in arr {
                collect_objects_by_key(child, key, out);
            }
        }
        _ => {}
    }
}

fn best_thumbnail_url(thumbs: Option<&Value>, video_id: &str) -> String {
    let raw = thumbs
        .and_then(|t| t.as_array())
        .and_then(|arr| arr.last())
        .and_then(|t| t.get("url"))
        .and_then(|u| u.as_str())
        .unwrap_or("")
        .to_string();
    let candidate = if raw.is_empty() {
        format!("https://i.ytimg.com/vi/{}/hqdefault.jpg", video_id)
    } else {
        raw
    };
    upgrade_cover_url(&candidate)
}

fn extract_panel_candidate(item: &Value, seed_video_id: &str) -> Option<YtmCandidate> {
    let video_id = item
        .get("videoId")
        .or_else(|| item.pointer("/navigationEndpoint/watchEndpoint/videoId"))
        .and_then(|v| v.as_str())?
        .trim()
        .to_string();
    if video_id.is_empty() || video_id == seed_video_id {
        return None;
    }

    let title = item
        .pointer("/title/runs")
        .and_then(join_runs_text)
        .unwrap_or_default();
    if title.is_empty() {
        return None;
    }

    let subtitle_runs = item
        .pointer("/longBylineText/runs")
        .or_else(|| item.pointer("/shortBylineText/runs"))
        .and_then(|r| r.as_array());

    let mut artist = String::new();
    let mut artist_id = String::new();
    let mut album = String::new();
    let mut year = 0i32;
    let mut duration = item
        .pointer("/lengthText/runs")
        .and_then(join_runs_text)
        .and_then(|s| parse_mm_ss(&s))
        .unwrap_or(0);

    if let Some(runs) = subtitle_runs {
        for r in runs {
            let Some(part) = r.get("text").and_then(|t| t.as_str()).map(str::trim) else {
                continue;
            };
            if part.is_empty() || part == "•" || part == "·" || part == "&" || part == "," {
                continue;
            }
            let page_type = r
                .pointer("/navigationEndpoint/browseEndpoint/browseEndpointContextSupportedConfigs/browseEndpointContextMusicConfig/pageType")
                .and_then(|p| p.as_str())
                .unwrap_or("");
            let browse_id = r
                .pointer("/navigationEndpoint/browseEndpoint/browseId")
                .and_then(|b| b.as_str())
                .map(str::trim)
                .unwrap_or("");
            if page_type == "MUSIC_PAGE_TYPE_ARTIST" {
                if artist.is_empty() {
                    artist = part.to_string();
                    if !browse_id.is_empty() {
                        artist_id = browse_id.to_string();
                    }
                } else if !artist.contains(part) {
                    artist = format!("{}, {}", artist, part);
                }
                continue;
            }
            if page_type == "MUSIC_PAGE_TYPE_ALBUM" {
                if album.is_empty() {
                    album = part.to_string();
                }
                continue;
            }
            if let Some(d) = parse_mm_ss(part) {
                if duration == 0 {
                    duration = d;
                }
                continue;
            }
            if let Some(y) = parse_release_year(part) {
                year = y;
                continue;
            }
            let low = part.to_lowercase();
            if low == "song"
                || low == "video"
                || low.ends_with("plays")
                || low.ends_with("views")
                || low.ends_with("likes")
            {
                continue;
            }
            if artist.is_empty() {
                artist = part.to_string();
                if artist_id.is_empty() && browse_id.starts_with("UC") {
                    artist_id = browse_id.to_string();
                }
            } else if album.is_empty() && page_type.is_empty() {
                album = part.to_string();
            }
        }
    }

    let cover_url = best_thumbnail_url(item.pointer("/thumbnail/thumbnails"), &video_id);
    Some(YtmCandidate {
        video_id,
        title,
        artist,
        artist_id,
        album,
        year,
        duration,
        cover_url,
    })
}

fn extract_candidate(item: &Value) -> Option<YtmCandidate> {
    let video_id = item
        .pointer("/playlistItemData/videoId")
        .or_else(|| {
            item.pointer("/overlay/musicItemThumbnailOverlayRenderer/content/musicPlayButtonRenderer/playNavigationEndpoint/watchEndpoint/videoId")
        })
        .and_then(|v| v.as_str())
        .or_else(|| find_watch_video_id(item))?
        .trim()
        .to_string();
    if video_id.is_empty() {
        return None;
    }

    let flex = item.get("flexColumns").and_then(|f| f.as_array())?;
    let title = flex
        .first()
        .and_then(|c| c.pointer("/musicResponsiveListItemFlexColumnRenderer/text/runs"))
        .and_then(join_runs_text)
        .unwrap_or_default();
    if title.is_empty() {
        return None;
    }

    let subtitle_runs = flex
        .get(1)
        .and_then(|c| c.pointer("/musicResponsiveListItemFlexColumnRenderer/text/runs"))
        .and_then(|r| r.as_array());

    let mut artist = String::new();
    let mut artist_id = String::new();
    let mut album = String::new();
    let mut year = 0i32;
    let mut duration = 0i32;

    if let Some(runs) = subtitle_runs {
        for r in runs {
            let Some(part) = r.get("text").and_then(|t| t.as_str()).map(str::trim) else {
                continue;
            };
            if part.is_empty() || part == "•" || part == "·" || part == "&" || part == "," {
                continue;
            }
            let page_type = r
                .pointer("/navigationEndpoint/browseEndpoint/browseEndpointContextSupportedConfigs/browseEndpointContextMusicConfig/pageType")
                .and_then(|p| p.as_str())
                .unwrap_or("");
            let browse_id = r
                .pointer("/navigationEndpoint/browseEndpoint/browseId")
                .and_then(|b| b.as_str())
                .map(str::trim)
                .unwrap_or("");
            if page_type == "MUSIC_PAGE_TYPE_ARTIST" {
                if artist.is_empty() {
                    artist = part.to_string();
                    if !browse_id.is_empty() {
                        artist_id = browse_id.to_string();
                    }
                } else if !artist.contains(part) {
                    artist = format!("{}, {}", artist, part);
                }
                continue;
            }
            if page_type == "MUSIC_PAGE_TYPE_ALBUM" {
                if album.is_empty() {
                    album = part.to_string();
                }
                continue;
            }
            let low = part.to_lowercase();
            if let Some(d) = parse_mm_ss(part) {
                duration = d;
                continue;
            }
            if let Some(y) = parse_release_year(part) {
                year = y;
                continue;
            }
            if low == "song"
                || low == "video"
                || low == "single"
                || low == "ep"
                || low.ends_with("plays")
                || low.ends_with("views")
                || low.ends_with("likes")
            {
                continue;
            }
            if artist.is_empty() {
                artist = part.to_string();
                if artist_id.is_empty() && browse_id.starts_with("UC") {
                    artist_id = browse_id.to_string();
                }
            } else if album.is_empty() {
                album = part.to_string();
            }
        }
    }

    let cover_url = best_thumbnail_url(
        item.pointer("/thumbnail/musicThumbnailRenderer/thumbnail/thumbnails"),
        &video_id,
    );

    Some(YtmCandidate {
        video_id,
        title,
        artist,
        artist_id,
        album,
        year,
        duration,
        cover_url,
    })
}

fn parse_release_year(s: &str) -> Option<i32> {
    let trimmed = s.trim();
    if trimmed.len() == 4 && trimmed.chars().all(|c| c.is_ascii_digit()) {
        let y = trimmed.parse::<i32>().ok()?;
        if (1920..=2035).contains(&y) {
            return Some(y);
        }
    }
    None
}

fn find_watch_video_id(v: &Value) -> Option<&str> {
    match v {
        Value::Object(map) => {
            if let Some(vid) = map.get("watchEndpoint").and_then(|w| w.get("videoId")).and_then(|s| s.as_str()) {
                return Some(vid);
            }
            for child in map.values() {
                if let Some(found) = find_watch_video_id(child) {
                    return Some(found);
                }
            }
            None
        }
        Value::Array(arr) => {
            for child in arr {
                if let Some(found) = find_watch_video_id(child) {
                    return Some(found);
                }
            }
            None
        }
        _ => None,
    }
}

fn join_runs_text(v: &Value) -> Option<String> {
    let arr = v.as_array()?;
    let joined: String = arr
        .iter()
        .filter_map(|r| r.get("text").and_then(|t| t.as_str()))
        .collect::<Vec<_>>()
        .join("")
        .trim()
        .to_string();
    (!joined.is_empty()).then_some(joined)
}

fn parse_mm_ss(s: &str) -> Option<i32> {
    let parts: Vec<&str> = s.trim().split(':').collect();
    if parts.len() == 2 {
        let m = parts[0].parse::<i32>().ok()?;
        let sec = parts[1].parse::<i32>().ok()?;
        if (0..60).contains(&sec) && m >= 0 {
            return Some(m * 60 + sec);
        }
    } else if parts.len() == 3 {
        let h = parts[0].parse::<i32>().ok()?;
        let m = parts[1].parse::<i32>().ok()?;
        let sec = parts[2].parse::<i32>().ok()?;
        if (0..60).contains(&m) && (0..60).contains(&sec) && h >= 0 {
            return Some(h * 3600 + m * 60 + sec);
        }
    }
    None
}

fn score_ytm_candidate(
    c: &YtmCandidate,
    target_artist: &str,
    target_title: &str,
    target_duration: i32,
) -> i32 {
    let mut score = 100;
    let cand_title_core = core_song_title(&c.title);
    let want_title_core = core_song_title(target_title);
    let cand_title_low = c.title.to_lowercase();
    let cand_artist_slug = slug(&c.artist);
    let want_artist_slug = slug(target_artist);

    if cand_title_core == want_title_core && !want_title_core.is_empty() {
        score += 180;
    } else if !want_title_core.is_empty() && cand_title_core.contains(&want_title_core) {
        score += 90;
    }

    if !want_artist_slug.is_empty() && cand_artist_slug.contains(&want_artist_slug) {
        score += 120;
    } else if !want_artist_slug.is_empty() && cand_title_low.contains(&target_artist.to_lowercase()) {
        score += 60;
    }

    if target_duration > 0 && c.duration > 0 {
        let diff = (c.duration - target_duration).abs();
        if diff <= 6 {
            score += 120;
        } else if diff <= 15 {
            score += 65;
        } else if diff > 60 {
            score -= 100;
        }
    }

    let want_low = target_title.to_lowercase();
    for bad in [
        "karaoke",
        "instrumental",
        "cover",
        "slowed",
        "reverb",
        "nightcore",
        "8d audio",
        "reaction",
        "tutorial",
        "live",
    ] {
        if cand_title_low.contains(bad) && !want_low.contains(bad) {
            score -= 140;
        }
    }
    score
}

/// Attempt order starting from the last client that worked, then the rest in
/// their default priority.
fn client_attempt_order(preferred: usize, n: usize) -> Vec<usize> {
    let first = if preferred < n { preferred } else { 0 };
    std::iter::once(first).chain((0..n).filter(move |&i| i != first)).collect()
}

/// Seconds-since-epoch expiry embedded in a googlevideo stream URL (`expire=`).
pub fn stream_url_expiry(url: &str) -> Option<i64> {
    let q = url.split_once('?')?.1;
    q.split('&')
        .find_map(|kv| kv.strip_prefix("expire="))
        .and_then(|v| v.parse::<i64>().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_order_starts_with_last_working_client() {
        assert_eq!(client_attempt_order(0, 3), vec![0, 1, 2]);
        assert_eq!(client_attempt_order(1, 3), vec![1, 0, 2]);
        assert_eq!(client_attempt_order(2, 3), vec![2, 0, 1]);
        assert_eq!(client_attempt_order(9, 3), vec![0, 1, 2]);
    }

    #[test]
    fn stream_url_expiry_reads_expire_param() {
        assert_eq!(
            stream_url_expiry("https://rr1.googlevideo.com/videoplayback?id=x&expire=1795000000&ei=y"),
            Some(1_795_000_000)
        );
        assert_eq!(stream_url_expiry("https://example.com/a.flac"), None);
        assert_eq!(stream_url_expiry("https://x/v?expire=abc"), None);
    }

    #[test]
    fn innertube_parser_extracts_song_fields() {
        let payload = serde_json::json!({
            "contents": {
                "tabbedSearchResultsRenderer": {
                    "tabs": [{
                        "tabRenderer": {
                            "content": {
                                "sectionListRenderer": {
                                    "contents": [{
                                        "musicShelfRenderer": {
                                            "contents": [{
                                                "musicResponsiveListItemRenderer": {
                                                    "playlistItemData": { "videoId": "FGBhQbmPwH8" },
                                                    "flexColumns": [
                                                        {
                                                            "musicResponsiveListItemFlexColumnRenderer": {
                                                                "text": { "runs": [{ "text": "One More Time" }] }
                                                            }
                                                        },
                                                        {
                                                            "musicResponsiveListItemFlexColumnRenderer": {
                                                                "text": {
                                                                    "runs": [
                                                                        { "text": "Daft Punk" },
                                                                        { "text": " • " },
                                                                        { "text": "Discovery" },
                                                                        { "text": " • " },
                                                                        { "text": "5:20" }
                                                                    ]
                                                                }
                                                            }
                                                        }
                                                    ]
                                                }
                                            }]
                                        }
                                    }]
                                }
                            }
                        }
                    }]
                }
            }
        });
        let items = parse_innertube_results(&payload);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].video_id, "FGBhQbmPwH8");
        assert_eq!(items[0].title, "One More Time");
        assert_eq!(items[0].artist, "Daft Punk");
        assert_eq!(items[0].album, "Discovery");
        assert_eq!(items[0].duration, 320);
    }

    #[test]
    fn scorer_prefers_studio_over_karaoke_and_matches_duration() {
        let studio = YtmCandidate {
            video_id: "good".into(),
            title: "One More Time".into(),
            artist: "Daft Punk".into(),
            album: "Discovery".into(),
            duration: 320,
            cover_url: String::new(),
            ..Default::default()
        };
        let bad = YtmCandidate {
            video_id: "bad".into(),
            title: "One More Time (Karaoke Slowed + Reverb)".into(),
            artist: "Random Cover".into(),
            album: String::new(),
            duration: 210,
            cover_url: String::new(),
            ..Default::default()
        };
        assert!(
            score_ytm_candidate(&studio, "Daft Punk", "One More Time", 320)
                > score_ytm_candidate(&bad, "Daft Punk", "One More Time", 320)
        );
    }

    #[test]
    fn radio_panel_parser_extracts_tracks_skips_seed_and_upgrades_thumbnails() {
        let payload = serde_json::json!({
            "contents": {
                "singleColumnMusicWatchNextResultsRenderer": {
                    "tabbedRenderer": {
                        "watchNextTabbedResultsRenderer": {
                            "tabs": [{
                                "tabRenderer": {
                                    "content": {
                                        "musicQueueRenderer": {
                                            "content": {
                                                "playlistPanelRenderer": {
                                                    "contents": [
                                                        {
                                                            "playlistPanelVideoRenderer": {
                                                                "videoId": "R-maTg10QKs",
                                                                "title": { "runs": [{ "text": "ソラゴト - Fake" }] },
                                                                "longBylineText": { "runs": [{ "text": "ASU" }, { "text": " • " }, { "text": "2023" }] },
                                                                "lengthText": { "runs": [{ "text": "3:15" }] }
                                                            }
                                                        },
                                                        {
                                                            "playlistPanelVideoRenderer": {
                                                                "videoId": "BRPQDDAUeMY",
                                                                "title": { "runs": [{ "text": "愛が灯る - The Flame of Love" }] },
                                                                "longBylineText": {
                                                                    "runs": [
                                                                        {
                                                                            "text": "Rokudenashi",
                                                                            "navigationEndpoint": {
                                                                                "browseEndpoint": {
                                                                                    "browseEndpointContextSupportedConfigs": {
                                                                                        "browseEndpointContextMusicConfig": { "pageType": "MUSIC_PAGE_TYPE_ARTIST" }
                                                                                    }
                                                                                }
                                                                            }
                                                                        },
                                                                        { "text": " • " },
                                                                        {
                                                                            "text": "The Flame of Love",
                                                                            "navigationEndpoint": {
                                                                                "browseEndpoint": {
                                                                                    "browseEndpointContextSupportedConfigs": {
                                                                                        "browseEndpointContextMusicConfig": { "pageType": "MUSIC_PAGE_TYPE_ALBUM" }
                                                                                    }
                                                                                }
                                                                            }
                                                                        },
                                                                        { "text": " • " },
                                                                        { "text": "2023" }
                                                                    ]
                                                                },
                                                                "thumbnail": {
                                                                    "thumbnails": [
                                                                        { "url": "https://yt3.googleusercontent.com/abc=w120-h120-l90-rj" }
                                                                    ]
                                                                },
                                                                "lengthText": { "runs": [{ "text": "3:22" }] }
                                                            }
                                                        }
                                                    ]
                                                }
                                            }
                                        }
                                    }
                                }
                            }]
                        }
                    }
                }
            }
        });
        let items = parse_innertube_radio_results(&payload, "R-maTg10QKs");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].video_id, "BRPQDDAUeMY");
        assert_eq!(items[0].artist, "Rokudenashi");
        assert_eq!(items[0].album, "The Flame of Love");
        assert_eq!(items[0].year, 2023);
        assert_eq!(items[0].duration, 202);
        assert_eq!(
            items[0].cover_url,
            "https://yt3.googleusercontent.com/abc=w1080-h1080-l90-rj"
        );
    }

    #[test]
    fn innertube_parser_extracts_release_year_without_confusing_album_1984() {
        let payload = serde_json::json!({
            "contents": {
                "tabbedSearchResultsRenderer": {
                    "tabs": [{
                        "tabRenderer": {
                            "content": {
                                "sectionListRenderer": {
                                    "contents": [{
                                        "musicShelfRenderer": {
                                            "contents": [{
                                                "musicResponsiveListItemRenderer": {
                                                    "playlistItemData": { "videoId": "jump84" },
                                                    "flexColumns": [
                                                        {
                                                            "musicResponsiveListItemFlexColumnRenderer": {
                                                                "text": { "runs": [{ "text": "Jump" }] }
                                                            }
                                                        },
                                                        {
                                                            "musicResponsiveListItemFlexColumnRenderer": {
                                                                "text": {
                                                                    "runs": [
                                                                        {
                                                                            "text": "Van Halen",
                                                                            "navigationEndpoint": {
                                                                                "browseEndpoint": {
                                                                                    "browseEndpointContextSupportedConfigs": {
                                                                                        "browseEndpointContextMusicConfig": { "pageType": "MUSIC_PAGE_TYPE_ARTIST" }
                                                                                    }
                                                                                }
                                                                            }
                                                                        },
                                                                        { "text": " • " },
                                                                        {
                                                                            "text": "1984",
                                                                            "navigationEndpoint": {
                                                                                "browseEndpoint": {
                                                                                    "browseEndpointContextSupportedConfigs": {
                                                                                        "browseEndpointContextMusicConfig": { "pageType": "MUSIC_PAGE_TYPE_ALBUM" }
                                                                                    }
                                                                                }
                                                                            }
                                                                        },
                                                                        { "text": " • " },
                                                                        { "text": "1983" },
                                                                        { "text": " • " },
                                                                        { "text": "4:01" }
                                                                    ]
                                                                }
                                                            }
                                                        }
                                                    ]
                                                }
                                            }]
                                        }
                                    }]
                                }
                            }
                        }
                    }]
                }
            }
        });
        let items = parse_innertube_results(&payload);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].album, "1984");
        assert_eq!(items[0].year, 1983);
        assert_eq!(items[0].duration, 241);
    }

    #[test]
    fn parse_ytdlp_has_video_distinguishes_audio_only_from_muxed_streams() {
        assert!(!parse_ytdlp_has_video(&serde_json::json!({ "vcodec": "none", "acodec": "mp4a.40.2" })));
        assert!(!parse_ytdlp_has_video(&serde_json::json!({ "vcodec": "NONE" })));
        assert!(!parse_ytdlp_has_video(&serde_json::json!({ "vcodec": "" })));
        assert!(!parse_ytdlp_has_video(&serde_json::json!({ "acodec": "opus" })));
        assert!(parse_ytdlp_has_video(&serde_json::json!({ "vcodec": "avc1.42001E", "acodec": "mp4a.40.2" })));
        assert!(parse_ytdlp_has_video(&serde_json::json!({ "vcodec": "vp9", "acodec": "opus" })));
    }
}
