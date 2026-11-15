use crate::models::{err_msg, primary_artist, upgrade_cover_url, AppResult, LyricLine, LyricsData, Track};
use crate::sources::local::extract_embedded_cover;
use crate::sources::{MusicSource, SourceRegistry};
use crate::store::Store;
use axum::body::Body;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use futures::StreamExt;
use serde::Deserialize;
use sha1::{Digest, Sha1};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::sync::{Mutex, Notify, RwLock};
use tokio_util::io::ReaderStream;
use tracing::{debug, warn};

#[derive(Clone)]
pub struct CachedResolved {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub ext: String,
    pub webpage_url: String,
    pub expires_at: i64,
}

pub enum PlayTarget {
    LocalFile {
        path: PathBuf,
        content_type: String,
    },
    RemoteHttp {
        url: String,
        headers: Vec<(String, String)>,
        content_type: String,
        ext: String,
        webpage_url: String,
        cacheable: bool,
    },
}

#[derive(Clone)]
pub struct StreamEngine {
    pub store: Store,
    pub sources: SourceRegistry,
    pub http: reqwest::Client,
    pub cache_dir: PathBuf,
    pub cover_dir: PathBuf,
    pub max_cache_bytes: i64,
    pub ffmpeg_available: bool,
    resolved_cache: Arc<RwLock<HashMap<String, CachedResolved>>>,
    in_flight_resolutions: Arc<Mutex<HashMap<String, Arc<Notify>>>>,
    in_flight_downloads: Arc<Mutex<HashSet<String>>>,
}

impl StreamEngine {
    pub fn new(
        store: Store,
        sources: SourceRegistry,
        data_dir: &Path,
        max_cache_gb: f64,
    ) -> Self {
        let cache_dir = data_dir.join("cache").join("audio");
        let cover_dir = data_dir.join("cache").join("covers");
        let _ = std::fs::create_dir_all(&cache_dir);
        let _ = std::fs::create_dir_all(&cover_dir);

        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::limited(8))
            .build()
            .unwrap_or_default();

        let ffmpeg_available = std::process::Command::new("ffmpeg")
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

        Self {
            store,
            sources,
            http,
            cache_dir,
            cover_dir,
            max_cache_bytes: ((max_cache_gb.max(0.5)) * 1024.0 * 1024.0 * 1024.0) as i64,
            ffmpeg_available,
            resolved_cache: Arc::new(RwLock::new(HashMap::new())),
            in_flight_resolutions: Arc::new(Mutex::new(HashMap::new())),
            in_flight_downloads: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    pub async fn resolve_target(&self, track: &Track) -> AppResult<PlayTarget> {
        // 1. Check disk cache for this exact track.
        if let Ok(Some(entry)) = self.store.get_cache_entry(&track.id) {
            let p = PathBuf::from(&entry.path);
            if p.exists() && std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0) > 4096 {
                let _ = self.store.touch_cache_entry(&track.id);
                let ext = p
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or(&track.format);
                return Ok(PlayTarget::LocalFile {
                    content_type: mime_for_audio_ext(ext).to_string(),
                    path: p,
                });
            }
            let _ = self.store.delete_cache_entry(&track.id);
        }

        // 2. Local filesystem source.
        if let Some(local_path) = track.stream_url.strip_prefix("file:") {
            let p = PathBuf::from(local_path);
            if p.exists() {
                return Ok(PlayTarget::LocalFile {
                    content_type: mime_for_audio_ext(&track.format).to_string(),
                    path: p,
                });
            }
            return Err(err_msg("Local audio file missing"));
        }

        // 3. Soulseek P2P source -> downloads into cache directory via MusicSource::resolve_stream.
        if track.stream_url.starts_with("slsk://") {
            let resolved = self
                .sources
                .soulseek
                .resolve_stream(&track.stream_url)
                .await
                .ok_or_else(|| err_msg("Soulseek download failed"))?;
            let local_path = PathBuf::from(resolved.strip_prefix("file:").unwrap_or(&resolved));
            if !local_path.exists() {
                return Err(err_msg("Soulseek download failed: cached file missing"));
            }
            let bytes = std::fs::metadata(&local_path)
                .map(|m| m.len() as i64)
                .unwrap_or(0);
            let _ = self
                .store
                .upsert_cache_entry(&track.id, &local_path.to_string_lossy(), bytes);
            let _ = self.store.prune_cache(self.max_cache_bytes);
            let ext = local_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or(&track.format);
            return Ok(PlayTarget::LocalFile {
                content_type: mime_for_audio_ext(ext).to_string(),
                path: local_path,
            });
        }

        // 4. Local/cached sibling upgrade for streaming metadata tracks (`resolve:ytm:` / `ytm:`).
        if track.stream_url.starts_with("resolve:") || track.stream_url.starts_with("ytm:") {
            if let Some(local) = self.find_local_or_cached_sibling(track) {
                return Ok(local);
            }
            let resolved = self.resolve_ytm_singleflight(track, false).await?;
            return Ok(PlayTarget::RemoteHttp {
                content_type: mime_for_audio_ext(&resolved.ext).to_string(),
                url: resolved.url,
                headers: resolved.headers,
                ext: resolved.ext,
                webpage_url: resolved.webpage_url,
                cacheable: true,
            });
        }

        // 5. Direct HTTP stream (Archive.org FLAC/MP3, radio, etc.).
        if track.stream_url.starts_with("http://") || track.stream_url.starts_with("https://") {
            return Ok(PlayTarget::RemoteHttp {
                url: track.stream_url.clone(),
                headers: vec![(
                    "User-Agent".to_string(),
                    "Beatlab/0.3 (HighQualityAudio)".to_string(),
                )],
                content_type: mime_for_audio_ext(&track.format).to_string(),
                ext: if track.format.is_empty() {
                    "mp3".to_string()
                } else {
                    track.format.clone()
                },
                webpage_url: track.stream_url.clone(),
                cacheable: !track.stream_url.contains("/preview/") && track.source != "radio",
            });
        }

        Err(err_msg(format!("Unsupported stream URL: {}", track.stream_url)))
    }

    fn find_local_or_cached_sibling(&self, track: &Track) -> Option<PlayTarget> {
        let key = track.song_key();
        let candidates = self.store.search_tracks(&track.title, 40, 0).ok()?;
        for c in candidates {
            if c.id == track.id || c.song_key() != key {
                continue;
            }
            if track.duration > 0 && c.duration > 0 && (track.duration - c.duration).abs() > 20 {
                continue;
            }
            if let Some(path_str) = c.stream_url.strip_prefix("file:") {
                let p = PathBuf::from(path_str);
                if p.exists() {
                    return Some(PlayTarget::LocalFile {
                        content_type: mime_for_audio_ext(&c.format).to_string(),
                        path: p,
                    });
                }
            }
            if let Ok(Some(entry)) = self.store.get_cache_entry(&c.id) {
                let p = PathBuf::from(&entry.path);
                if p.exists() && std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0) > 4096 {
                    return Some(PlayTarget::LocalFile {
                        content_type: mime_for_audio_ext(&c.format).to_string(),
                        path: p,
                    });
                }
            }
        }
        None
    }

    async fn resolve_ytm_singleflight(
        &self,
        track: &Track,
        force_refresh: bool,
    ) -> AppResult<CachedResolved> {
        let now = Utc::now().timestamp();
        if !force_refresh {
            let guard = self.resolved_cache.read().await;
            if let Some(cached) = guard.get(&track.id) {
                if cached.expires_at > now + 30 {
                    return Ok(cached.clone());
                }
            }
        }

        // Coordinate concurrent requests for the same track ID.
        loop {
            let (notify, is_leader) = {
                let mut map = self.in_flight_resolutions.lock().await;
                if let Some(existing) = map.get(&track.id) {
                    (Arc::clone(existing), false)
                } else {
                    let n = Arc::new(Notify::new());
                    map.insert(track.id.clone(), Arc::clone(&n));
                    (n, true)
                }
            };

            if !is_leader {
                notify.notified().await;
                let guard = self.resolved_cache.read().await;
                if let Some(cached) = guard.get(&track.id) {
                    if cached.expires_at > Utc::now().timestamp() {
                        return Ok(cached.clone());
                    }
                }
                continue;
            }

            let res = self.perform_ytm_resolve(track).await;
            {
                let mut map = self.in_flight_resolutions.lock().await;
                map.remove(&track.id);
            }
            notify.notify_waiters();
            return res;
        }
    }

    async fn perform_ytm_resolve(&self, track: &Track) -> AppResult<CachedResolved> {
        let video_id = if let Some(vid) = track.stream_url.strip_prefix("ytm:") {
            vid.trim().to_string()
        } else if let Some(spec) = track.stream_url.strip_prefix("resolve:ytm:") {
            let mut parts = spec.splitn(3, '|');
            let art = parts.next().unwrap_or(&track.artist);
            let title = parts.next().unwrap_or(&track.title);
            let dur = parts
                .next()
                .and_then(|d| d.parse::<i32>().ok())
                .unwrap_or(track.duration);
            self.sources
                .ytm
                .find_best_video_for_track(art, title, dur)
                .await
                .ok_or_else(|| {
                    err_msg(format!("No playable match found on YouTube Music for {} - {}", art, title))
                })?
        } else {
            return Err(err_msg("Unsupported resolution specifier"));
        };

        let resolved = self.sources.ytm.resolve_stream(&video_id).await?;
        let item = CachedResolved {
            url: resolved.url,
            headers: resolved.headers,
            ext: resolved.ext,
            webpage_url: resolved.source_webpage_url,
            expires_at: Utc::now().timestamp() + 3600,
        };
        {
            let mut guard = self.resolved_cache.write().await;
            guard.insert(track.id.clone(), item.clone());
        }
        Ok(item)
    }

    /// Pre-resolves stream metadata & kicks off background caching so next-track transitions feel instant.
    pub async fn warm_track(&self, track: &Track) -> AppResult<()> {
        match self.resolve_target(track).await? {
            PlayTarget::LocalFile { .. } => Ok(()),
            PlayTarget::RemoteHttp {
                url,
                headers,
                ext,
                cacheable,
                ..
            } => {
                if cacheable {
                    self.spawn_background_cache(&track.id, &url, &headers, &ext);
                }
                Ok(())
            }
        }
    }

    pub async fn serve_track_stream(
        &self,
        track: &Track,
        req_headers: &HeaderMap,
        max_bitrate: Option<i32>,
        target_format: Option<&str>,
        time_offset: f64,
    ) -> Response {
        let want_fmt = target_format
            .map(|f| f.trim().to_lowercase())
            .filter(|f| !f.is_empty() && f != "raw");

        let resolved = match self.resolve_target(track).await {
            Ok(t) => t,
            Err(e) => {
                warn!(track_id = %track.id, error = %e, "stream resolution failed");
                return (StatusCode::BAD_GATEWAY, format!("Stream error: {}", e)).into_response();
            }
        };

        let need_transcode = self.ffmpeg_available
            && (time_offset > 0.5
                || want_fmt.is_some()
                || max_bitrate.map(|b| b > 0).unwrap_or(false));

        if need_transcode {
            let (fmt, br) = choose_transcode_params(
                want_fmt.as_deref(),
                max_bitrate,
                track,
            );
            return self
                .serve_ffmpeg_transcode(track, &resolved, &fmt, br, time_offset)
                .await;
        }

        let range_hdr = req_headers
            .get(header::RANGE)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        match resolved {
            PlayTarget::LocalFile { path, content_type } => {
                serve_local_file_range(&path, &content_type, range_hdr.as_deref()).await
            }
            PlayTarget::RemoteHttp {
                url,
                headers,
                content_type,
                ext,
                webpage_url,
                cacheable,
            } => {
                if cacheable {
                    self.spawn_background_cache(&track.id, &url, &headers, &ext);
                }
                self.serve_remote_http_range(
                    track,
                    &url,
                    &headers,
                    &content_type,
                    &webpage_url,
                    range_hdr.as_deref(),
                )
                .await
            }
        }
    }

    async fn serve_remote_http_range(
        &self,
        track: &Track,
        url: &str,
        headers: &[(String, String)],
        content_type: &str,
        webpage_url: &str,
        range_hdr: Option<&str>,
    ) -> Response {
        let mut active_url = url.to_string();
        let mut active_headers = headers.to_vec();
        let mut active_ct = content_type.to_string();

        for attempt in 0..2 {
            let mut req = self.http.get(&active_url);
            for (k, v) in &active_headers {
                req = req.header(k.as_str(), v.as_str());
            }
            if let Some(r) = range_hdr {
                req = req.header(header::RANGE, r);
            }

            match req.send().await {
                Ok(resp) if resp.status().is_success() || resp.status() == reqwest::StatusCode::PARTIAL_CONTENT => {
                    let status = if resp.status() == reqwest::StatusCode::PARTIAL_CONTENT {
                        StatusCode::PARTIAL_CONTENT
                    } else {
                        StatusCode::OK
                    };
                    let mut builder = Response::builder()
                        .status(status)
                        .header(header::CONTENT_TYPE, &active_ct)
                        .header(header::ACCEPT_RANGES, "bytes")
                        .header(header::CACHE_CONTROL, "private, max-age=3600");
                    if let Some(len) = resp.headers().get(header::CONTENT_LENGTH) {
                        builder = builder.header(header::CONTENT_LENGTH, len);
                    }
                    if let Some(cr) = resp.headers().get(header::CONTENT_RANGE) {
                        builder = builder.header(header::CONTENT_RANGE, cr);
                    }
                    let stream = resp.bytes_stream().map(|res| {
                        res.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
                    });
                    return builder.body(Body::from_stream(stream)).unwrap_or_else(|_| {
                        StatusCode::INTERNAL_SERVER_ERROR.into_response()
                    });
                }
                Ok(resp)
                    if attempt == 0
                        && (resp.status() == reqwest::StatusCode::FORBIDDEN
                            || resp.status() == reqwest::StatusCode::GONE)
                        && (track.stream_url.starts_with("resolve:")
                            || track.stream_url.starts_with("ytm:")) =>
                {
                    // Cached upstream signed URL expired or IP-shifted; re-resolve cleanly.
                    debug!(track_id = %track.id, status = %resp.status(), "refreshing expired stream URL");
                    if let Ok(fresh) = self.resolve_ytm_singleflight(track, true).await {
                        active_url = fresh.url;
                        active_headers = fresh.headers;
                        active_ct = mime_for_audio_ext(&fresh.ext).to_string();
                        continue;
                    }
                    break;
                }
                Ok(resp) => {
                    warn!(status = %resp.status(), url = %active_url, "upstream stream failed");
                    break;
                }
                Err(e) => {
                    warn!(error = %e, "upstream stream request error");
                    break;
                }
            }
        }

        // Last-resort direct pipe via yt-dlp -o - if upstream direct URL failed.
        if webpage_url.contains("youtube.com") || webpage_url.contains("youtu.be") {
            return self.serve_ytdlp_pipe(webpage_url).await;
        }
        (StatusCode::BAD_GATEWAY, "Failed to proxy upstream audio").into_response()
    }

    async fn serve_ytdlp_pipe(&self, watch_url: &str) -> Response {
        let mut cmd = Command::new(self.sources.ytm.ytdlp_bin());
        cmd.args([
            "--no-playlist",
            "--no-warnings",
            "--extractor-args",
            "youtube:player_client=android",
            "-f",
            "bestaudio/best",
            "-o",
            "-",
            watch_url,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

        let Ok(mut child) = cmd.spawn() else {
            return (StatusCode::BAD_GATEWAY, "Failed to spawn yt-dlp pipe").into_response();
        };
        let Some(stdout) = child.stdout.take() else {
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        };
        tokio::spawn(async move {
            let _ = child.wait().await;
        });
        let stream = ReaderStream::new(stdout);
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "audio/mp4")
            .body(Body::from_stream(stream))
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
    }

    async fn serve_ffmpeg_transcode(
        &self,
        track: &Track,
        target: &PlayTarget,
        fmt: &str,
        bitrate_kbps: i32,
        time_offset: f64,
    ) -> Response {
        let (input_arg, custom_headers, ct) = match target {
            PlayTarget::LocalFile { path, .. } => (
                path.to_string_lossy().to_string(),
                vec![],
                mime_for_audio_ext(fmt),
            ),
            PlayTarget::RemoteHttp {
                url,
                headers,
                ext,
                cacheable,
                ..
            } => {
                if *cacheable {
                    self.spawn_background_cache(&track.id, url, headers, ext);
                }
                (url.clone(), headers.clone(), mime_for_audio_ext(fmt))
            }
        };

        let mut cmd = Command::new("ffmpeg");
        cmd.args(["-hide_banner", "-loglevel", "error"]);
        if time_offset > 0.5 {
            cmd.args(["-ss", &format!("{:.2}", time_offset)]);
        }
        if input_arg.starts_with("http://") || input_arg.starts_with("https://") {
            let mut ua = String::new();
            let mut hdr_blob = String::new();
            for (k, v) in &custom_headers {
                if k.eq_ignore_ascii_case("user-agent") {
                    ua = v.clone();
                } else {
                    hdr_blob.push_str(&format!("{}: {}\r\n", k, v));
                }
            }
            if !ua.is_empty() {
                cmd.args(["-user_agent", &ua]);
            }
            if !hdr_blob.is_empty() {
                cmd.args(["-headers", &hdr_blob]);
            }
        }
        cmd.args(["-i", &input_arg, "-vn", "-map_metadata", "-1"]);

        match fmt {
            "opus" => {
                cmd.args([
                    "-c:a",
                    "libopus",
                    "-b:a",
                    &format!("{}k", bitrate_kbps.clamp(48, 320)),
                    "-f",
                    "opus",
                    "pipe:1",
                ]);
            }
            "ogg" => {
                cmd.args([
                    "-c:a",
                    "libvorbis",
                    "-b:a",
                    &format!("{}k", bitrate_kbps.clamp(64, 320)),
                    "-f",
                    "ogg",
                    "pipe:1",
                ]);
            }
            "flac" => {
                cmd.args(["-c:a", "flac", "-f", "flac", "pipe:1"]);
            }
            "aac" | "m4a" => {
                cmd.args([
                    "-c:a",
                    "aac",
                    "-b:a",
                    &format!("{}k", bitrate_kbps.clamp(64, 320)),
                    "-f",
                    "adts",
                    "pipe:1",
                ]);
            }
            _ => {
                cmd.args([
                    "-c:a",
                    "libmp3lame",
                    "-b:a",
                    &format!("{}k", bitrate_kbps.clamp(64, 320)),
                    "-f",
                    "mp3",
                    "pipe:1",
                ]);
            }
        }

        cmd.stdout(Stdio::piped()).stderr(Stdio::null());
        let Ok(mut child) = cmd.spawn() else {
            return (StatusCode::BAD_GATEWAY, "Failed to launch ffmpeg").into_response();
        };
        let Some(stdout) = child.stdout.take() else {
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        };
        tokio::spawn(async move {
            let _ = child.wait().await;
        });

        let stream = ReaderStream::new(stdout);
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, ct)
            .body(Body::from_stream(stream))
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
    }

    pub fn spawn_background_cache(
        &self,
        track_id: &str,
        url: &str,
        headers: &[(String, String)],
        ext: &str,
    ) {
        if let Ok(Some(entry)) = self.store.get_cache_entry(track_id) {
            if Path::new(&entry.path).exists() {
                return;
            }
        }

        let this = self.clone();
        let tid = track_id.to_string();
        let u = url.to_string();
        let hdrs = headers.to_vec();
        let ext_s = if ext.is_empty() { "m4a" } else { ext }.to_string();

        tokio::spawn(async move {
            {
                let mut set = this.in_flight_downloads.lock().await;
                if !set.insert(tid.clone()) {
                    return;
                }
            }
            let _ = this.download_to_disk_cache(&tid, &u, &hdrs, &ext_s).await;
            let mut set = this.in_flight_downloads.lock().await;
            set.remove(&tid);
        });
    }

    async fn download_to_disk_cache(
        &self,
        track_id: &str,
        url: &str,
        headers: &[(String, String)],
        ext: &str,
    ) -> AppResult<()> {
        let hash = sha1_hex(track_id);
        let part_path = self.cache_dir.join(format!("{}.part", hash));
        let final_ext = if ext == "mp4" || ext == "m4a" {
            "m4a"
        } else {
            ext
        };
        let final_path = self.cache_dir.join(format!("{}.{}", hash, final_ext));
        if final_path.exists() {
            return Ok(());
        }

        let mut req = self.http.get(url);
        for (k, v) in headers {
            req = req.header(k.as_str(), v.as_str());
        }
        let resp = req.send().await?;
        if !resp.status().is_success() {
            return Err(err_msg(format!("Cache download HTTP {}", resp.status())));
        }

        let mut file = File::create(&part_path).await?;
        let mut stream = resp.bytes_stream();
        let mut total_written: i64 = 0;
        while let Some(chunk) = stream.next().await {
            let bytes = chunk?;
            total_written += bytes.len() as i64;
            if total_written > 250 * 1024 * 1024 {
                drop(file);
                let _ = tokio::fs::remove_file(&part_path).await;
                return Err(err_msg("Track exceeded 250MB cache cap"));
            }
            file.write_all(&bytes).await?;
        }
        file.flush().await?;
        drop(file);

        if total_written < 4096 {
            let _ = tokio::fs::remove_file(&part_path).await;
            return Err(err_msg("Downloaded file too small"));
        }

        // Convert mp4 progressive streams cleanly into fast-start audio-only m4a if ffmpeg is installed.
        let remuxed = if self.ffmpeg_available && (ext == "mp4" || ext == "m4a") {
            let status = Command::new("ffmpeg")
                .args([
                    "-y",
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-i",
                    &part_path.to_string_lossy(),
                    "-vn",
                    "-c:a",
                    "copy",
                    "-movflags",
                    "+faststart",
                    &final_path.to_string_lossy(),
                ])
                .status()
                .await
                .map(|s| s.success())
                .unwrap_or(false);
            if status && final_path.exists() {
                let _ = tokio::fs::remove_file(&part_path).await;
                true
            } else {
                false
            }
        } else {
            false
        };

        if !remuxed {
            tokio::fs::rename(&part_path, &final_path).await?;
        }

        let final_bytes = tokio::fs::metadata(&final_path)
            .await
            .map(|m| m.len() as i64)
            .unwrap_or(total_written);
        self.store
            .upsert_cache_entry(track_id, &final_path.to_string_lossy(), final_bytes)?;
        let _ = self.store.prune_cache(self.max_cache_bytes);
        debug!(track_id = %track_id, bytes = final_bytes, "cached track to disk");
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Cover Art & Lyrics
    // -------------------------------------------------------------------------

    pub async fn serve_cover(&self, raw_id: &str) -> Response {
        let id = raw_id
            .trim()
            .trim_start_matches("al-")
            .trim_start_matches("pl-");
        let lookup_id = if raw_id.starts_with("al:") || raw_id.starts_with("ar:") || raw_id.starts_with("pl-") {
            raw_id.trim()
        } else {
            id
        };

        let cache_key = sha1_hex(lookup_id);
        let cache_file = self.cover_dir.join(format!("{}.img", cache_key));
        let meta_file = self.cover_dir.join(format!("{}.mime", cache_key));
        let src_file = self.cover_dir.join(format!("{}.src", cache_key));

        let mut stale_cached: Option<(Vec<u8>, String)> = None;
        if cache_file.exists() {
            if let Ok(bytes) = tokio::fs::read(&cache_file).await {
                if !bytes.is_empty() {
                    let mime = tokio::fs::read_to_string(&meta_file)
                        .await
                        .unwrap_or_else(|_| "image/jpeg".to_string());
                    if !is_low_res_cover(&bytes) {
                        return image_response(bytes, &mime);
                    }
                    stale_cached = Some((bytes, mime));
                }
            }
        }

        // 1. Check playlist -> first track cover
        if lookup_id.starts_with("pl-") {
            if let Ok(tracks) = self.store.playlist_tracks(lookup_id) {
                if let Some(first) = tracks.first() {
                    return Box::pin(self.serve_cover(&first.id)).await;
                }
            }
        }

        // 2. Check album cover -> albums_meta or first track of album
        let mut label_for_svg = lookup_id.to_string();
        let mut candidate_url = String::new();
        let mut embedded_path: Option<PathBuf> = None;

        if lookup_id.starts_with("al:") {
            if let Ok(Some(meta)) = self.store.get_album_meta(lookup_id) {
                label_for_svg = meta.name.clone();
                if !meta.cover_url.is_empty() {
                    candidate_url = meta.cover_url;
                }
            }
            if candidate_url.is_empty() {
                if let Ok(tracks) = self.store.album_tracks(lookup_id) {
                    if let Some(t) = tracks.first() {
                        label_for_svg = t.album.clone();
                        if let Some(p) = t.cover_url.strip_prefix("local-art:") {
                            embedded_path = Some(PathBuf::from(p));
                        } else if !t.cover_url.is_empty() {
                            candidate_url = t.cover_url.clone();
                        }
                    }
                }
            }
        } else if lookup_id.starts_with("ar:") {
            if let Ok(Some(meta)) = self.store.get_artist_meta(lookup_id) {
                label_for_svg = meta.name.clone();
                if !meta.cover_url.is_empty() {
                    candidate_url = meta.cover_url;
                }
            }
            if candidate_url.is_empty() {
                if let Ok(tracks) = self.store.artist_tracks(lookup_id) {
                    if let Some(t) = tracks.first() {
                        label_for_svg = t.artist.clone();
                        if !t.cover_url.is_empty() && !t.cover_url.starts_with("local-art:") {
                            candidate_url = t.cover_url.clone();
                        }
                    }
                }
            }
        } else if let Ok(Some(t)) = self.store.get_track(lookup_id).or_else(|_| self.store.get_track(raw_id)) {
            label_for_svg = format!("{} - {}", primary_artist(&t.artist), t.title);
            if let Some(p) = t.cover_url.strip_prefix("local-art:") {
                embedded_path = Some(PathBuf::from(p));
            } else if !t.cover_url.is_empty() {
                candidate_url = t.cover_url;
            } else {
                // Quick fallback lookup on iTunes Search API if track has no artwork URL.
                candidate_url = self.lookup_itunes_cover(&t.artist, &t.title).await;
            }
        }

        if let Some(path) = embedded_path {
            if let Some((bytes, mime)) = extract_embedded_cover(&path) {
                let _ = tokio::fs::write(&cache_file, &bytes).await;
                let _ = tokio::fs::write(&meta_file, &mime).await;
                return image_response(bytes, &mime);
            }
        }

        let upgraded_url = upgrade_cover_url(&candidate_url);

        if let Some((stale_bytes, stale_mime)) = stale_cached {
            let prev_src = tokio::fs::read_to_string(&src_file).await.unwrap_or_default();
            if upgraded_url.starts_with("http://") || upgraded_url.starts_with("https://") {
                if prev_src.trim() != upgraded_url {
                    let fetched = self.fetch_image_bytes(&upgraded_url).await;
                    let _ = tokio::fs::write(&src_file, &upgraded_url).await;
                    if let Some((bytes, mime)) = fetched {
                        if !is_low_res_cover(&bytes) || bytes.len() > stale_bytes.len() {
                            let _ = tokio::fs::write(&cache_file, &bytes).await;
                            let _ = tokio::fs::write(&meta_file, &mime).await;
                            return image_response(bytes, &mime);
                        }
                    }
                }
            }
            return image_response(stale_bytes, &stale_mime);
        }

        if upgraded_url.starts_with("http://") || upgraded_url.starts_with("https://") {
            let fetched = match self.fetch_image_bytes(&upgraded_url).await {
                Some(res) => Some(res),
                None if upgraded_url != candidate_url => self.fetch_image_bytes(&candidate_url).await,
                None => None,
            };
            if let Some((bytes, mime)) = fetched {
                let _ = tokio::fs::write(&cache_file, &bytes).await;
                let _ = tokio::fs::write(&meta_file, &mime).await;
                let _ = tokio::fs::write(&src_file, &upgraded_url).await;
                return image_response(bytes, &mime);
            }
        }

        svg_cover_response(&label_for_svg)
    }

    async fn fetch_image_bytes(&self, url: &str) -> Option<(Vec<u8>, String)> {
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return None;
        }
        let resp = self.http.get(url).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let mime = resp
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("image/jpeg")
            .to_string();
        let bytes = resp.bytes().await.ok()?;
        if bytes.is_empty() {
            return None;
        }
        Some((bytes.to_vec(), mime))
    }

    async fn lookup_itunes_cover(&self, artist: &str, title: &str) -> String {
        let term = format!("{} {}", primary_artist(artist), title);
        let url = format!(
            "https://itunes.apple.com/search?term={}&entity=song&limit=1",
            urlencoding::encode(term.trim())
        );
        let Ok(resp) = self.http.get(&url).send().await else {
            return String::new();
        };
        let Ok(val) = resp.json::<serde_json::Value>().await else {
            return String::new();
        };
        val.pointer("/results/0/artworkUrl100")
            .and_then(|s| s.as_str())
            .map(upgrade_cover_url)
            .unwrap_or_default()
    }

    pub async fn fetch_lyrics(&self, track: &Track) -> LyricsData {
        let kv_key = format!("lyrics:{}", track.id);
        if let Some(cached) = self.store.kv_get(&kv_key) {
            if let Ok(parsed) = serde_json::from_str::<LyricsData>(&cached) {
                return parsed;
            }
        }

        let artist = primary_artist(&track.artist);
        let artist_q = if artist.is_empty() {
            track.artist.trim()
        } else {
            artist.as_str()
        };
        let clean_title = crate::models::strip_release_tags(&track.title);

        let url = format!(
            "https://lrclib.net/api/get?artist_name={}&track_name={}&album_name={}{}",
            urlencoding::encode(artist_q),
            urlencoding::encode(&clean_title),
            urlencoding::encode(track.album.trim()),
            if track.duration > 0 {
                format!("&duration={}", track.duration)
            } else {
                String::new()
            }
        );

        let mut data = self.try_lrclib_get(&url).await;
        if data.is_none() {
            let search_url = format!(
                "https://lrclib.net/api/search?q={}",
                urlencoding::encode(&format!("{} {}", artist_q, clean_title))
            );
            data = self.try_lrclib_search(&search_url).await;
        }

        let final_lyrics = data.unwrap_or_default();
        if !final_lyrics.plain.is_empty() || !final_lyrics.lines.is_empty() {
            if let Ok(json) = serde_json::to_string(&final_lyrics) {
                self.store.kv_set(&kv_key, &json);
            }
        }
        final_lyrics
    }

    async fn try_lrclib_get(&self, url: &str) -> Option<LyricsData> {
        let resp = self.http.get(url).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let item: LrclibEntry = resp.json().await.ok()?;
        lrclib_entry_to_lyrics(item)
    }

    async fn try_lrclib_search(&self, url: &str) -> Option<LyricsData> {
        let resp = self.http.get(url).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let items: Vec<LrclibEntry> = resp.json().await.ok()?;
        items.into_iter().find_map(lrclib_entry_to_lyrics)
    }
}

#[derive(Deserialize)]
struct LrclibEntry {
    #[serde(default)]
    #[serde(rename = "plainLyrics")]
    plain_lyrics: Option<String>,
    #[serde(default)]
    #[serde(rename = "syncedLyrics")]
    synced_lyrics: Option<String>,
}

fn lrclib_entry_to_lyrics(entry: LrclibEntry) -> Option<LyricsData> {
    if let Some(synced) = entry.synced_lyrics.filter(|s| !s.trim().is_empty()) {
        let lines = parse_lrc(&synced);
        let plain = entry
            .plain_lyrics
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| {
                lines
                    .iter()
                    .map(|l| l.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            });
        return Some(LyricsData {
            synced: !lines.is_empty(),
            plain,
            lines,
        });
    }
    if let Some(plain) = entry.plain_lyrics.filter(|s| !s.trim().is_empty()) {
        let lines = plain
            .lines()
            .map(|l| LyricLine {
                start_ms: None,
                text: l.to_string(),
            })
            .collect();
        return Some(LyricsData {
            synced: false,
            plain,
            lines,
        });
    }
    None
}

pub fn parse_lrc(lrc: &str) -> Vec<LyricLine> {
    let mut lines = Vec::new();
    for raw_line in lrc.lines() {
        let mut s = raw_line.trim();
        let mut timestamps = Vec::new();
        while s.starts_with('[') {
            let Some(close) = s.find(']') else {
                break;
            };
            let tag = &s[1..close];
            if let Some(ms) = parse_lrc_timestamp(tag) {
                timestamps.push(ms);
                s = &s[close + 1..];
            } else {
                break;
            }
        }
        let text = s.trim().to_string();
        if timestamps.is_empty() {
            if !text.is_empty() {
                lines.push(LyricLine {
                    start_ms: None,
                    text,
                });
            }
        } else {
            for ts in timestamps {
                lines.push(LyricLine {
                    start_ms: Some(ts),
                    text: text.clone(),
                });
            }
        }
    }
    lines.sort_by_key(|l| l.start_ms.unwrap_or(i64::MAX));
    lines
}

fn parse_lrc_timestamp(tag: &str) -> Option<i64> {
    let (m_str, rest) = tag.split_once(':')?;
    let mins: i64 = m_str.trim().parse().ok()?;
    let secs: f64 = rest.trim().parse().ok()?;
    Some(mins * 60_000 + (secs * 1000.0).round() as i64)
}

async fn serve_local_file_range(
    path: &Path,
    content_type: &str,
    range_hdr: Option<&str>,
) -> Response {
    let Ok(meta) = tokio::fs::metadata(path).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let size = meta.len();
    if size == 0 {
        return StatusCode::NO_CONTENT.into_response();
    }

    if let Some(r_str) = range_hdr {
        if let Some((start, end)) = parse_byte_range(r_str, size) {
            let len = end - start + 1;
            let Ok(mut file) = File::open(path).await else {
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            };
            if file.seek(std::io::SeekFrom::Start(start)).await.is_err() {
                return StatusCode::RANGE_NOT_SATISFIABLE.into_response();
            }
            let limited = file.take(len);
            let stream = ReaderStream::new(limited);
            return Response::builder()
                .status(StatusCode::PARTIAL_CONTENT)
                .header(header::CONTENT_TYPE, content_type)
                .header(header::ACCEPT_RANGES, "bytes")
                .header(header::CONTENT_LENGTH, len.to_string())
                .header(
                    header::CONTENT_RANGE,
                    format!("bytes {}-{}/{}", start, end, size),
                )
                .header(header::CACHE_CONTROL, "private, max-age=86400")
                .body(Body::from_stream(stream))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
        }
    }

    let Ok(file) = File::open(path).await else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let stream = ReaderStream::new(file);
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_LENGTH, size.to_string())
        .header(header::CACHE_CONTROL, "private, max-age=86400")
        .body(Body::from_stream(stream))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

pub fn parse_byte_range(header_val: &str, total: u64) -> Option<(u64, u64)> {
    let spec = header_val.trim().strip_prefix("bytes=")?;
    let first = spec.split(',').next()?.trim();
    let (s, e) = first.split_once('-')?;
    if s.is_empty() {
        let suffix: u64 = e.trim().parse().ok()?;
        if suffix == 0 {
            return None;
        }
        let start = total.saturating_sub(suffix);
        return Some((start, total - 1));
    }
    let start: u64 = s.trim().parse().ok()?;
    if start >= total {
        return None;
    }
    let end = if e.trim().is_empty() {
        total - 1
    } else {
        e.trim().parse::<u64>().ok()?.min(total - 1)
    };
    (start <= end).then_some((start, end))
}

fn choose_transcode_params(
    want_fmt: Option<&str>,
    max_bitrate: Option<i32>,
    track: &Track,
) -> (String, i32) {
    let fmt = want_fmt.unwrap_or_else(|| {
        if track.format.is_empty() {
            "mp3"
        } else if track.format == "m4a" || track.format == "mp4" {
            "aac"
        } else {
            track.format.as_str()
        }
    });
    let norm = match fmt {
        "mp3" => "mp3",
        "opus" => "opus",
        "ogg" | "vorbis" => "ogg",
        "aac" | "m4a" => "aac",
        "flac" => "flac",
        _ => "mp3",
    };
    let br = max_bitrate
        .filter(|&b| b > 0)
        .unwrap_or_else(|| if track.bitrate > 0 { track.bitrate } else { 256 });
    (norm.to_string(), br)
}

pub fn mime_for_audio_ext(ext: &str) -> &'static str {
    match ext.trim().to_lowercase().as_str() {
        "mp3" => "audio/mpeg",
        "flac" => "audio/flac",
        "m4a" | "mp4" | "alac" => "audio/mp4",
        "aac" => "audio/aac",
        "ogg" | "oga" => "audio/ogg",
        "opus" => "audio/opus",
        "wav" => "audio/wav",
        "webm" => "audio/webm",
        _ => "audio/mpeg",
    }
}

fn sha1_hex(s: &str) -> String {
    let mut h = Sha1::new();
    h.update(s.as_bytes());
    hex::encode(h.finalize())
}

/// Parses pixel `(width, height)` from PNG, JPEG, or WebP headers.
fn image_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    // PNG: 8-byte signature + IHDR chunk (width at 16..20, height at 20..24)
    if bytes.len() >= 24 && &bytes[..8] == b"\x89PNG\r\n\x1a\n" {
        let w = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
        let h = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
        return (w > 0 && h > 0).then_some((w, h));
    }

    // JPEG: scan segments until SOF0..SOF15 (excluding DHT/JPG/DAC: 0xC4, 0xC8, 0xCC)
    if bytes.len() >= 4 && bytes[0] == 0xFF && bytes[1] == 0xD8 {
        let mut i = 2usize;
        while i + 3 < bytes.len() {
            if bytes[i] != 0xFF {
                i += 1;
                continue;
            }
            while i < bytes.len() && bytes[i] == 0xFF {
                i += 1;
            }
            if i >= bytes.len() {
                break;
            }
            let marker = bytes[i];
            i += 1;
            if marker == 0xD9 || marker == 0x00 {
                break;
            }
            if marker == 0xD8 || marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
                continue;
            }
            if i + 2 > bytes.len() {
                break;
            }
            let seg_len = u16::from_be_bytes([bytes[i], bytes[i + 1]]) as usize;
            if seg_len < 2 || i + seg_len > bytes.len() {
                break;
            }
            let is_sof = matches!(
                marker,
                0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF
            );
            if is_sof && seg_len >= 7 && i + 7 <= bytes.len() {
                let h = u16::from_be_bytes([bytes[i + 3], bytes[i + 4]]) as u32;
                let w = u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]) as u32;
                return (w > 0 && h > 0).then_some((w, h));
            }
            i += seg_len;
        }
    }

    // WebP: RIFF....WEBP
    if bytes.len() >= 30 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        match &bytes[12..16] {
            b"VP8 " if bytes[23..26] == [0x9d, 0x01, 0x2a] => {
                let w = (u16::from_le_bytes([bytes[26], bytes[27]]) & 0x3FFF) as u32;
                let h = (u16::from_le_bytes([bytes[28], bytes[29]]) & 0x3FFF) as u32;
                return (w > 0 && h > 0).then_some((w, h));
            }
            b"VP8L" if bytes[20] == 0x2F => {
                let bits = u32::from_le_bytes([bytes[21], bytes[22], bytes[23], bytes[24]]);
                let w = (bits & 0x3FFF) + 1;
                let h = ((bits >> 14) & 0x3FFF) + 1;
                return Some((w, h));
            }
            b"VP8X" => {
                let w = u32::from_le_bytes([bytes[24], bytes[25], bytes[26], 0]) + 1;
                let h = u32::from_le_bytes([bytes[27], bytes[28], bytes[29], 0]) + 1;
                return Some((w, h));
            }
            _ => {}
        }
    }

    None
}

fn is_low_res_cover(bytes: &[u8]) -> bool {
    image_dimensions(bytes)
        .map(|(w, h)| w.max(h) < 240)
        .unwrap_or(false)
}

fn image_response(bytes: Vec<u8>, mime: &str) -> Response {
    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header(header::CACHE_CONTROL, "public, max-age=604800");
    if let Ok(hv) = HeaderValue::from_str(mime) {
        builder = builder.header(header::CONTENT_TYPE, hv);
    } else {
        builder = builder.header(header::CONTENT_TYPE, "image/jpeg");
    }
    builder
        .body(Body::from(bytes))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn svg_cover_response(label: &str) -> Response {
    let palettes = [
        ("#1db954", "#064e3b"),
        ("#7c3aed", "#3b0764"),
        ("#2563eb", "#1e3a8a"),
        ("#e11d48", "#881337"),
        ("#d97706", "#78350f"),
        ("#0891b2", "#164e63"),
    ];
    let hash: usize = label.bytes().fold(0usize, |acc, b| acc.wrapping_add(b as usize));
    let (c1, c2) = palettes[hash % palettes.len()];
    let initial = label
        .chars()
        .find(|c| c.is_alphanumeric())
        .unwrap_or('B')
        .to_uppercase()
        .to_string();
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 300 300">
<defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0%" stop-color="{}"/><stop offset="100%" stop-color="{}"/></linearGradient></defs>
<rect width="300" height="300" fill="url(#g)"/>
<circle cx="150" cy="150" r="84" fill="rgba(255,255,255,0.1)"/>
<text x="150" y="176" text-anchor="middle" font-family="system-ui,sans-serif" font-size="96" font-weight="700" fill="rgba(255,255,255,0.92)">{}</text>
</svg>"#,
        c1, c2, initial
    );
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "image/svg+xml")
        .header(header::CACHE_CONTROL, "public, max-age=86400")
        .body(Body::from(svg))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_range_parsing_handles_standard_cases() {
        assert_eq!(parse_byte_range("bytes=0-", 1000), Some((0, 999)));
        assert_eq!(parse_byte_range("bytes=100-499", 1000), Some((100, 499)));
        assert_eq!(parse_byte_range("bytes=-200", 1000), Some((800, 999)));
        assert_eq!(parse_byte_range("bytes=1200-", 1000), None);
    }

    #[test]
    fn lrc_parser_sorts_and_extracts_timestamps() {
        let raw = "[01:05.50]Second line\n[00:10.00]First line";
        let lines = parse_lrc(raw);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "First line");
        assert_eq!(lines[0].start_ms, Some(10_000));
        assert_eq!(lines[1].text, "Second line");
        assert_eq!(lines[1].start_ms, Some(65_500));
    }

    #[test]
    fn image_dimensions_detects_small_and_hd_covers() {
        // Minimal valid PNG header with 120x120 dimensions
        let mut small_png = vec![0u8; 24];
        small_png[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        small_png[12..16].copy_from_slice(b"IHDR");
        small_png[16..20].copy_from_slice(&120u32.to_be_bytes());
        small_png[20..24].copy_from_slice(&120u32.to_be_bytes());
        assert_eq!(image_dimensions(&small_png), Some((120, 120)));
        assert!(is_low_res_cover(&small_png));

        // Minimal valid JPEG SOI + SOF0 segment with 1080x1080 dimensions
        let hd_jpeg = vec![
            0xFF, 0xD8, // SOI
            0xFF, 0xC0, // SOF0
            0x00, 0x0B, // segment length = 11
            0x08,       // precision
            0x04, 0x38, // height = 1080
            0x04, 0x38, // width = 1080
            0x03, 0x01, 0x11, 0x00,
        ];
        assert_eq!(image_dimensions(&hd_jpeg), Some((1080, 1080)));
        assert!(!is_low_res_cover(&hd_jpeg));
    }
}
