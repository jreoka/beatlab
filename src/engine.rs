use crate::models::{err_msg, primary_artist, upgrade_cover_url, AppResult, LyricLine, LyricsData, Track};
use crate::sources::local::extract_embedded_cover;
use crate::sources::SourceRegistry;
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
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::sync::{Notify, OwnedSemaphorePermit, RwLock, Semaphore};
use tokio_util::io::ReaderStream;
use tracing::{debug, warn};

struct NotifyLeaderGuard {
    map: Arc<StdMutex<HashMap<String, Arc<Notify>>>>,
    key: String,
    notify: Arc<Notify>,
}

impl Drop for NotifyLeaderGuard {
    fn drop(&mut self) {
        let mut guard = self.map.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(cur) = guard.get(&self.key) {
            if Arc::ptr_eq(cur, &self.notify) {
                guard.remove(&self.key);
            }
        }
        self.notify.notify_waiters();
    }
}

struct InFlightSetGuard {
    set: Arc<StdMutex<HashSet<String>>>,
    key: String,
}

impl Drop for InFlightSetGuard {
    fn drop(&mut self) {
        let mut guard = self.set.lock().unwrap_or_else(|p| p.into_inner());
        guard.remove(&self.key);
    }
}

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
    pub stream_http: reqwest::Client,
    pub cache_dir: PathBuf,
    pub cover_dir: PathBuf,
    pub embed_dir: PathBuf,
    pub max_cache_bytes: i64,
    pub ffmpeg_available: bool,
    resolved_cache: Arc<RwLock<HashMap<String, CachedResolved>>>,
    in_flight_resolutions: Arc<StdMutex<HashMap<String, Arc<Notify>>>>,
    in_flight_downloads: Arc<StdMutex<HashSet<String>>>,
    in_flight_warms: Arc<StdMutex<HashSet<String>>>,
    in_flight_embeds: Arc<StdMutex<HashMap<String, Arc<Notify>>>>,
    warm_sem: Arc<Semaphore>,
    cache_sem: Arc<Semaphore>,
    stream_proc_sem: Arc<Semaphore>,
    embed_sem: Arc<Semaphore>,
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
        let embed_dir = data_dir.join("cache").join("embeds");
        let _ = std::fs::create_dir_all(&cache_dir);
        let _ = std::fs::create_dir_all(&cover_dir);
        let _ = std::fs::create_dir_all(&embed_dir);

        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(12))
            .timeout(Duration::from_secs(30))
            .tcp_nodelay(true)
            .pool_max_idle_per_host(8)
            .redirect(reqwest::redirect::Policy::limited(8))
            .build()
            .unwrap_or_default();

        // Streaming/caching client: cap connect & per-read idle stalls without aborting valid
        // multi-minute lossless FLAC downloads at a 30s total transfer ceiling.
        let stream_http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .read_timeout(Duration::from_secs(45))
            .tcp_nodelay(true)
            .pool_max_idle_per_host(8)
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
            stream_http,
            cache_dir,
            cover_dir,
            embed_dir,
            max_cache_bytes: ((max_cache_gb.max(0.5)) * 1024.0 * 1024.0 * 1024.0) as i64,
            ffmpeg_available,
            resolved_cache: Arc::new(RwLock::new(HashMap::new())),
            in_flight_resolutions: Arc::new(StdMutex::new(HashMap::new())),
            in_flight_downloads: Arc::new(StdMutex::new(HashSet::new())),
            in_flight_warms: Arc::new(StdMutex::new(HashSet::new())),
            in_flight_embeds: Arc::new(StdMutex::new(HashMap::new())),
            warm_sem: Arc::new(Semaphore::new(2)),
            cache_sem: Arc::new(Semaphore::new(3)),
            stream_proc_sem: Arc::new(Semaphore::new(8)),
            embed_sem: Arc::new(Semaphore::new(2)),
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

        // 3. Soulseek P2P source -> downloads into cache directory; falls back cleanly to
        // local/cached siblings or YouTube Music if peers are unreachable/queued/offline.
        if track.stream_url.starts_with("slsk://") {
            if let Some(resolved) = self.sources.soulseek.resolve_track(track).await {
                let local_path = PathBuf::from(resolved.strip_prefix("file:").unwrap_or(&resolved));
                if local_path.exists() {
                    let bytes = std::fs::metadata(&local_path)
                        .map(|m| m.len() as i64)
                        .unwrap_or(0);
                    if bytes > 4096 {
                        let _ = self.store.upsert_cache_entry(
                            &track.id,
                            &local_path.to_string_lossy(),
                            bytes,
                        );
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
                }
            }
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
                // Keep resolved URL in memory without overwriting a lossless track's disk cache.
                cacheable: false,
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
        let (clean_art, clean_title_str) = clean_track_query_for_fallback(track);
        let cleaned_key = format!(
            "{}|{}",
            crate::models::slug(&clean_art),
            crate::models::core_song_title(&clean_title_str)
        );
        let clean_title = crate::models::core_song_title(&clean_title_str);
        let search_title = if clean_title.trim().is_empty() {
            track.title.as_str()
        } else {
            clean_title.as_str()
        };
        let candidates = self.store.search_tracks(search_title, 40, 0).ok()?;
        for c in candidates {
            let ckey = c.song_key();
            if c.id == track.id || (ckey != key && ckey != cleaned_key) {
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
                let mut map = self
                    .in_flight_resolutions
                    .lock()
                    .unwrap_or_else(|p| p.into_inner());
                if let Some(existing) = map.get(&track.id) {
                    (Arc::clone(existing), false)
                } else {
                    let n = Arc::new(Notify::new());
                    map.insert(track.id.clone(), Arc::clone(&n));
                    (n, true)
                }
            };

            if !is_leader {
                let _ = tokio::time::timeout(Duration::from_secs(25), notify.notified()).await;
                let guard = self.resolved_cache.read().await;
                if let Some(cached) = guard.get(&track.id) {
                    if cached.expires_at > Utc::now().timestamp() {
                        return Ok(cached.clone());
                    }
                }
                continue;
            }

            let _guard = NotifyLeaderGuard {
                map: Arc::clone(&self.in_flight_resolutions),
                key: track.id.clone(),
                notify: Arc::clone(&notify),
            };
            return self.perform_ytm_resolve(track).await;
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
            let (art, title) = clean_track_query_for_fallback(track);
            self.sources
                .ytm
                .find_best_video_for_track(&art, &title, track.duration)
                .await
                .ok_or_else(|| {
                    err_msg(format!("No playable match found on YouTube Music for {} - {}", art, title))
                })?
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

    /// Deduplicates and bounds background warm hints so rapid track skipping cannot pile up tasks.
    pub fn spawn_warm_track(&self, track: Track) {
        if let Ok(Some(entry)) = self.store.get_cache_entry(&track.id) {
            let p = Path::new(&entry.path);
            if p.exists() {
                return;
            }
        }
        let Ok(permit) = self.warm_sem.clone().try_acquire_owned() else {
            return;
        };
        {
            let mut set = self
                .in_flight_warms
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            if !set.insert(track.id.clone()) {
                return;
            }
        }
        let this = self.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let _guard = InFlightSetGuard {
                set: Arc::clone(&this.in_flight_warms),
                key: track.id.clone(),
            };
            let _ = tokio::time::timeout(Duration::from_secs(30), this.warm_track(&track)).await;
        });
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

        let safe_offset = sanitize_time_offset(time_offset);
        let need_transcode = self.ffmpeg_available
            && (safe_offset > 0.5
                || want_fmt.is_some()
                || max_bitrate.map(|b| b > 0).unwrap_or(false));

        if need_transcode {
            let (fmt, br) = choose_transcode_params(
                want_fmt.as_deref(),
                max_bitrate,
                track,
            );
            return self
                .serve_ffmpeg_transcode(track, &resolved, &fmt, br, safe_offset)
                .await;
        }

        self.serve_play_target_range(track, resolved, req_headers)
            .await
    }

    /// Streams a track for the web player at the highest browser-compatible quality:
    /// passes through the original audio untouched when natively playable, or automatically
    /// transcodes unsupported formats (lossless -> FLAC, lossy -> 320k MP3, or `compat=1` retry -> 320k MP3).
    pub async fn serve_web_stream(
        &self,
        track: &Track,
        req_headers: &HeaderMap,
        time_offset: f64,
        force_compat: bool,
    ) -> Response {
        let resolved = match self.resolve_target(track).await {
            Ok(t) => t,
            Err(e) => {
                warn!(track_id = %track.id, error = %e, "stream resolution failed");
                return (StatusCode::BAD_GATEWAY, format!("Stream error: {}", e)).into_response();
            }
        };

        let resolved_ext = play_target_ext(&resolved, track);
        let is_alac = is_alac_m4a(&resolved_ext, track);
        let needs_compat = force_compat
            || !is_browser_native_ext(&resolved_ext)
            || (is_alac && !is_safari_ua(req_headers))
            || (is_safari_ua(req_headers) && is_safari_unfriendly_ext(&resolved_ext));

        let safe_offset = sanitize_time_offset(time_offset);
        if self.ffmpeg_available && (safe_offset > 0.5 || needs_compat) {
            let (fmt, br) = choose_web_compat_transcode_params(
                &resolved_ext,
                track,
                force_compat,
                is_alac,
            );
            return self
                .serve_ffmpeg_transcode(track, &resolved, &fmt, br, safe_offset)
                .await;
        }

        self.serve_play_target_range(track, resolved, req_headers)
            .await
    }

    async fn serve_play_target_range(
        &self,
        track: &Track,
        resolved: PlayTarget,
        req_headers: &HeaderMap,
    ) -> Response {
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
                let resp = self
                    .serve_remote_http_range(
                        track,
                        &url,
                        &headers,
                        &content_type,
                        &webpage_url,
                        range_hdr.as_deref(),
                    )
                    .await;
                if cacheable
                    && (resp.status().is_success() || resp.status() == StatusCode::PARTIAL_CONTENT)
                {
                    // Delay background disk caching slightly so the initial playback range request
                    // gets full upstream bandwidth and starts immediately.
                    let this = self.clone();
                    let tid = track.id.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_millis(1500)).await;
                        this.spawn_background_cache(&tid, &url, &headers, &ext);
                    });
                }
                resp
            }
        }
    }

    async fn send_stream_proxy(
        &self,
        url: &str,
        headers: &[(String, String)],
        content_type: &str,
        range_hdr: Option<&str>,
    ) -> Result<Response, reqwest::StatusCode> {
        let mut req = self.stream_http.get(url);
        for (k, v) in headers {
            req = req.header(k.as_str(), v.as_str());
        }
        if let Some(r) = range_hdr {
            req = req.header(header::RANGE, r);
        }

        match req.send().await {
            Ok(resp)
                if resp.status().is_success()
                    || resp.status() == reqwest::StatusCode::PARTIAL_CONTENT =>
            {
                let status = if resp.status() == reqwest::StatusCode::PARTIAL_CONTENT {
                    StatusCode::PARTIAL_CONTENT
                } else {
                    StatusCode::OK
                };
                let mut builder = Response::builder()
                    .status(status)
                    .header(header::CONTENT_TYPE, content_type)
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
                Ok(builder.body(Body::from_stream(stream)).unwrap_or_else(|_| {
                    StatusCode::INTERNAL_SERVER_ERROR.into_response()
                }))
            }
            Ok(resp) => {
                warn!(status = %resp.status(), url = %url, "upstream stream failed");
                Err(resp.status())
            }
            Err(e) => {
                warn!(error = %e, "upstream stream request error");
                Err(reqwest::StatusCode::BAD_GATEWAY)
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
        let is_ytm_origin = webpage_url.contains("youtube.com")
            || webpage_url.contains("youtu.be")
            || track.stream_url.starts_with("resolve:")
            || track.stream_url.starts_with("ytm:");

        for attempt in 0..2 {
            match self
                .send_stream_proxy(&active_url, &active_headers, &active_ct, range_hdr)
                .await
            {
                Ok(resp) => return resp,
                Err(status)
                    if attempt == 0
                        && is_ytm_origin
                        && (status == reqwest::StatusCode::FORBIDDEN
                            || status == reqwest::StatusCode::GONE) =>
                {
                    // Cached upstream signed URL expired or IP-shifted; re-resolve cleanly.
                    debug!(track_id = %track.id, status = %status, "refreshing expired stream URL");
                    if let Ok(fresh) = self.resolve_ytm_singleflight(track, true).await {
                        active_url = fresh.url;
                        active_headers = fresh.headers;
                        active_ct = mime_for_audio_ext(&fresh.ext).to_string();
                        continue;
                    }
                    break;
                }
                Err(_) => break,
            }
        }

        // Last-resort direct pipe via yt-dlp -o - if upstream direct URL failed.
        if webpage_url.contains("youtube.com") || webpage_url.contains("youtu.be") {
            return self.serve_ytdlp_pipe(webpage_url).await;
        }

        // Non-radio HTTP tracks (e.g. Archive.org FLAC returning 403/404/503) fall back cleanly
        // to local/cached siblings or YouTube Music.
        if track.source != "radio" {
            if let Some(PlayTarget::LocalFile { path, content_type }) =
                self.find_local_or_cached_sibling(track)
            {
                return serve_local_file_range(&path, &content_type, range_hdr).await;
            }
            if let Ok(fresh) = self.resolve_ytm_singleflight(track, false).await {
                let ct = mime_for_audio_ext(&fresh.ext);
                if let Ok(resp) = self
                    .send_stream_proxy(&fresh.url, &fresh.headers, ct, range_hdr)
                    .await
                {
                    return resp;
                }
                if fresh.webpage_url.contains("youtube.com")
                    || fresh.webpage_url.contains("youtu.be")
                {
                    return self.serve_ytdlp_pipe(&fresh.webpage_url).await;
                }
            }
        }
        (StatusCode::BAD_GATEWAY, "Failed to proxy upstream audio").into_response()
    }

    async fn acquire_stream_proc_permit(&self) -> Result<OwnedSemaphorePermit, Response> {
        match tokio::time::timeout(
            Duration::from_secs(5),
            self.stream_proc_sem.clone().acquire_owned(),
        )
        .await
        {
            Ok(Ok(permit)) => Ok(permit),
            _ => Err((StatusCode::SERVICE_UNAVAILABLE, "Stream processor busy").into_response()),
        }
    }

    async fn serve_ytdlp_pipe(&self, watch_url: &str) -> Response {
        let permit = match self.acquire_stream_proc_permit().await {
            Ok(p) => p,
            Err(r) => return r,
        };
        let mut cmd = Command::new(self.sources.ytm.ytdlp_bin());
        cmd.args([
            "--no-playlist",
            "--no-warnings",
            "--extractor-args",
            "youtube:player_client=android",
            "-f",
            "bestaudio[ext=m4a][protocol!=m3u8][protocol!=m3u8_native]/bestaudio[protocol!=m3u8][protocol!=m3u8_native]/bestaudio/best",
            "-o",
            "-",
            watch_url,
        ]);
        stream_child_stdout(cmd, "audio/mp4", "Failed to spawn yt-dlp pipe", Some(permit))
    }

    async fn serve_ffmpeg_transcode(
        &self,
        track: &Track,
        target: &PlayTarget,
        fmt: &str,
        bitrate_kbps: i32,
        time_offset: f64,
    ) -> Response {
        let permit = match self.acquire_stream_proc_permit().await {
            Ok(p) => p,
            Err(r) => return r,
        };
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
        cmd.args(["-hide_banner", "-loglevel", "error", "-fflags", "+nobuffer"]);
        let safe_offset = sanitize_time_offset(time_offset);
        if safe_offset > 0.5 {
            cmd.args(["-ss", &format!("{:.2}", safe_offset)]);
        }
        if input_arg.starts_with("http://") || input_arg.starts_with("https://") {
            cmd.args(["-probesize", "1048576", "-analyzeduration", "1500000"]);
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
        cmd.args(["-i", &input_arg, "-vn", "-map_metadata", "-1", "-flush_packets", "1"]);

        match fmt {
            "opus" => {
                cmd.args([
                    "-ac",
                    "2",
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
                    "-ac",
                    "2",
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
                cmd.args([
                    "-c:a",
                    "flac",
                    "-compression_level",
                    "0",
                    "-f",
                    "flac",
                    "pipe:1",
                ]);
            }
            "aac" | "m4a" => {
                cmd.args([
                    "-ac",
                    "2",
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
                    "-ac",
                    "2",
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

        stream_child_stdout(cmd, ct, "Failed to launch ffmpeg", Some(permit))
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
        let Ok(permit) = self.cache_sem.clone().try_acquire_owned() else {
            return;
        };
        {
            let mut set = self
                .in_flight_downloads
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            if !set.insert(track_id.to_string()) {
                return;
            }
        }

        let this = self.clone();
        let tid = track_id.to_string();
        let u = url.to_string();
        let hdrs = headers.to_vec();
        let ext_s = if ext.is_empty() { "m4a" } else { ext }.to_string();

        tokio::spawn(async move {
            let _permit = permit;
            let _guard = InFlightSetGuard {
                set: Arc::clone(&this.in_flight_downloads),
                key: tid.clone(),
            };
            let _ = this.download_to_disk_cache(&tid, &u, &hdrs, &ext_s).await;
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
        if let Ok(meta) = tokio::fs::metadata(&final_path).await {
            if meta.len() > 4096 {
                let _ = self.store.upsert_cache_entry(
                    track_id,
                    &final_path.to_string_lossy(),
                    meta.len() as i64,
                );
                return Ok(());
            }
            let _ = tokio::fs::remove_file(&final_path).await;
        }

        let mut req = self.stream_http.get(url);
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
            let mut cmd = Command::new("ffmpeg");
            cmd.kill_on_drop(true)
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
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            let status = match cmd.spawn() {
                Ok(mut child) => {
                    match tokio::time::timeout(Duration::from_secs(45), child.wait()).await {
                        Ok(Ok(s)) => s.success(),
                        _ => {
                            let _ = child.kill().await;
                            false
                        }
                    }
                }
                Err(_) => false,
            };
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
        let trimmed_raw = raw_id.trim();
        let id = trimmed_raw
            .trim_start_matches("al-")
            .trim_start_matches("pl-");
        let lookup_id = if trimmed_raw.starts_with("al:")
            || trimmed_raw.starts_with("ar:")
            || trimmed_raw.starts_with("pl-")
            || trimmed_raw.starts_with("auto-")
        {
            trimmed_raw
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

        // 1. Check playlist or auto-mix -> first track cover
        if lookup_id.starts_with("pl-") || lookup_id.starts_with("auto-") {
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

    pub async fn cover_raster_bytes(&self, raw_id: &str) -> Option<(Vec<u8>, String)> {
        let resp = self.serve_cover(raw_id).await;
        if !resp.status().is_success() {
            return None;
        }
        let mime = resp
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("image/jpeg")
            .to_string();
        if mime.contains("svg") {
            return None;
        }
        let bytes = axum::body::to_bytes(resp.into_body(), 8 * 1024 * 1024)
            .await
            .ok()?;
        if bytes.is_empty() {
            None
        } else {
            Some((bytes.to_vec(), mime))
        }
    }

    pub fn embed_mp4_path(&self, track_id: &str) -> PathBuf {
        self.embed_dir.join(format!("{}.mp4", sha1_hex(track_id)))
    }

    pub fn has_cached_embed_mp4(&self, track_id: &str) -> bool {
        let p = self.embed_mp4_path(track_id);
        std::fs::metadata(&p).map(|m| m.len() > 4096).unwrap_or(false)
    }

    pub async fn ensure_share_embed_mp4(
        &self,
        track: &Track,
        fallback_png: &[u8],
    ) -> Option<PathBuf> {
        if !self.ffmpeg_available {
            return None;
        }
        let out_path = self.embed_mp4_path(&track.id);
        if tokio::fs::metadata(&out_path)
            .await
            .map(|m| m.len() > 4096)
            .unwrap_or(false)
        {
            return Some(out_path);
        }

        let (notify, is_leader) = {
            let mut guard = self
                .in_flight_embeds
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            if let Some(existing) = guard.get(&track.id) {
                (existing.clone(), false)
            } else {
                let n = Arc::new(Notify::new());
                guard.insert(track.id.clone(), n.clone());
                (n, true)
            }
        };

        if !is_leader {
            let _ = tokio::time::timeout(Duration::from_secs(30), notify.notified()).await;
            return if tokio::fs::metadata(&out_path)
                .await
                .map(|m| m.len() > 4096)
                .unwrap_or(false)
            {
                Some(out_path)
            } else {
                None
            };
        }

        let _guard = NotifyLeaderGuard {
            map: Arc::clone(&self.in_flight_embeds),
            key: track.id.clone(),
            notify: Arc::clone(&notify),
        };
        let result = self.build_share_embed_mp4(track, &out_path, fallback_png).await;
        if result {
            Some(out_path)
        } else {
            None
        }
    }

    async fn build_share_embed_mp4(
        &self,
        track: &Track,
        out_path: &Path,
        fallback_png: &[u8],
    ) -> bool {
        let Ok(_permit) = tokio::time::timeout(Duration::from_secs(20), self.embed_sem.acquire()).await else {
            return false;
        };
        let key = sha1_hex(&track.id);
        let cover_tmp = self.embed_dir.join(format!("{}.cover", key));
        let part_path = self.embed_dir.join(format!("{}.part.mp4", key));

        let (cover_bytes, _) = match self.cover_raster_bytes(&track.id).await {
            Some(v) => v,
            None => (fallback_png.to_vec(), "image/png".to_string()),
        };
        if tokio::fs::write(&cover_tmp, &cover_bytes).await.is_err() {
            return false;
        }

        let resolved = match self.resolve_target(track).await {
            Ok(r) => r,
            Err(_) => {
                let _ = tokio::fs::remove_file(&cover_tmp).await;
                return false;
            }
        };

        let (input_arg, custom_headers) = match resolved {
            PlayTarget::LocalFile { path, .. } => (path.to_string_lossy().to_string(), vec![]),
            PlayTarget::RemoteHttp { url, headers, .. } => (url, headers),
        };

        let audio_bitrate = if track.duration > 480 { "112k" } else { "160k" };

        let mut cmd = Command::new("ffmpeg");
        cmd.kill_on_drop(true);
        cmd.args(["-hide_banner", "-loglevel", "error", "-y"]);
        cmd.args(["-loop", "1", "-framerate", "1", "-i"]);
        cmd.arg(&cover_tmp);

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
        cmd.args(["-i", &input_arg]);
        cmd.args([
            "-map",
            "0:v:0",
            "-map",
            "1:a:0",
            "-vf",
            "scale=480:480:force_original_aspect_ratio=increase,crop=480:480",
            "-c:v",
            "libx264",
            "-tune",
            "stillimage",
            "-preset",
            "ultrafast",
            "-crf",
            "28",
            "-pix_fmt",
            "yuv420p",
            "-r",
            "1",
            "-c:a",
            "aac",
            "-b:a",
            audio_bitrate,
            "-ac",
            "2",
            "-shortest",
            "-movflags",
            "+faststart",
        ]);
        cmd.arg(&part_path);
        cmd.stdout(Stdio::null());
        cmd.stderr(Stdio::null());

        let ok = match cmd.spawn() {
            Ok(mut child) => {
                match tokio::time::timeout(Duration::from_secs(45), child.wait()).await {
                    Ok(Ok(status)) => status.success(),
                    _ => {
                        let _ = child.kill().await;
                        false
                    }
                }
            }
            Err(_) => false,
        };

        let _ = tokio::fs::remove_file(&cover_tmp).await;
        if ok
            && tokio::fs::metadata(&part_path)
                .await
                .map(|m| m.len() > 4096)
                .unwrap_or(false)
        {
            let renamed = tokio::fs::rename(&part_path, out_path).await.is_ok();
            if renamed {
                return true;
            }
        }
        let _ = tokio::fs::remove_file(&part_path).await;
        false
    }

    pub async fn serve_share_embed_mp4(
        &self,
        track: &Track,
        req_headers: &HeaderMap,
        fallback_png: &[u8],
    ) -> Response {
        let Some(mp4_path) = self.ensure_share_embed_mp4(track, fallback_png).await else {
            return (StatusCode::NOT_FOUND, "Embed media unavailable").into_response();
        };
        let range_hdr = req_headers
            .get(header::RANGE)
            .and_then(|v| v.to_str().ok());
        serve_local_file_range(&mp4_path, "video/mp4", range_hdr).await
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
            let stream = ReaderStream::with_capacity(limited, 65536);
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
        } else if r_str.trim().starts_with("bytes=") {
            return Response::builder()
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header(header::CONTENT_RANGE, format!("bytes */{}", size))
                .body(Body::empty())
                .unwrap_or_else(|_| StatusCode::RANGE_NOT_SATISFIABLE.into_response());
        }
    }

    let Ok(file) = File::open(path).await else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let stream = ReaderStream::with_capacity(file, 65536);
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

fn sanitize_time_offset(t: f64) -> f64 {
    if t.is_finite() && t > 0.5 {
        t.min(86_400.0)
    } else {
        0.0
    }
}

struct ChildStdoutStream {
    reader: ReaderStream<tokio::process::ChildStdout>,
    _child: tokio::process::Child,
    _permit: Option<OwnedSemaphorePermit>,
}

impl futures::Stream for ChildStdoutStream {
    type Item = <ReaderStream<tokio::process::ChildStdout> as futures::Stream>::Item;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        std::pin::Pin::new(&mut self.reader).poll_next(cx)
    }
}

/// Spawns a subprocess with `kill_on_drop(true)` and ties its lifetime to the HTTP response stream
/// so aborted or seeked client connections never orphan `ffmpeg` or `yt-dlp` processes.
fn stream_child_stdout(
    mut cmd: Command,
    content_type: &str,
    spawn_err: &'static str,
    permit: Option<OwnedSemaphorePermit>,
) -> Response {
    cmd.kill_on_drop(true)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let Ok(mut child) = cmd.spawn() else {
        return (StatusCode::BAD_GATEWAY, spawn_err).into_response();
    };
    let Some(stdout) = child.stdout.take() else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let stream = ChildStdoutStream {
        reader: ReaderStream::with_capacity(stdout, 65536),
        _child: child,
        _permit: permit,
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .body(Body::from_stream(stream))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn play_target_ext(target: &PlayTarget, track: &Track) -> String {
    let raw = match target {
        PlayTarget::LocalFile { path, .. } => path
            .extension()
            .and_then(|e| e.to_str())
            .filter(|s| !s.is_empty())
            .unwrap_or(&track.format),
        PlayTarget::RemoteHttp { ext, .. } => {
            if ext.is_empty() {
                &track.format
            } else {
                ext
            }
        }
    };
    raw.trim().trim_start_matches('.').to_lowercase()
}

fn is_lossless_ext(ext: &str) -> bool {
    matches!(
        ext.trim().trim_start_matches('.').to_lowercase().as_str(),
        "flac" | "alac" | "wav" | "aiff" | "aif" | "ape" | "wv" | "dsf" | "dff" | "tta" | "shn"
    )
}

fn is_browser_native_ext(ext: &str) -> bool {
    let norm = ext.trim().trim_start_matches('.').to_lowercase();
    norm.is_empty()
        || matches!(
            norm.as_str(),
            "mp3" | "flac" | "wav" | "m4a" | "mp4" | "aac" | "ogg" | "oga" | "opus" | "webm"
        )
}

fn is_safari_unfriendly_ext(ext: &str) -> bool {
    matches!(
        ext.trim().trim_start_matches('.').to_lowercase().as_str(),
        "ogg" | "oga" | "opus" | "webm"
    )
}

fn is_safari_ua(headers: &HeaderMap) -> bool {
    let Some(ua) = headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    ua.contains("Safari/")
        && !ua.contains("Chrome/")
        && !ua.contains("Chromium/")
        && !ua.contains("CriOS/")
        && !ua.contains("FxiOS/")
        && !ua.contains("Edg")
        && !ua.contains("OPR/")
        && !ua.contains("Android")
}

fn clean_track_query_for_fallback(track: &Track) -> (String, String) {
    if track.source == "soulseek" || track.stream_url.starts_with("slsk://") {
        let (art, title, _) = crate::sources::soulseek::clean_slsk_title_and_artist(
            &track.artist,
            &track.album,
            &track.title,
        );
        return (art, title);
    }
    let pa = primary_artist(&track.artist);
    let art = if pa.is_empty() {
        track.artist.trim().to_string()
    } else {
        pa
    };
    let clean_title = crate::models::strip_release_tags(&track.title);
    let title = if clean_title.is_empty() {
        track.title.trim().to_string()
    } else {
        clean_title
    };
    (art, title)
}

fn is_alac_m4a(resolved_ext: &str, track: &Track) -> bool {
    let ext = resolved_ext.trim().trim_start_matches('.').to_lowercase();
    let track_fmt = track.format.trim().trim_start_matches('.').to_lowercase();
    ext == "m4a"
        && (track_fmt.is_empty() || matches!(track_fmt.as_str(), "m4a" | "mp4" | "alac"))
        && (track.bit_depth > 0 || track.bitrate > 450)
}

fn choose_web_compat_transcode_params(
    resolved_ext: &str,
    track: &Track,
    force_compat: bool,
    is_alac: bool,
) -> (String, i32) {
    // If the browser already rejected a native format (e.g. problematic/high-rate FLAC) and
    // retried with `compat=1`, transcode to 320k MP3 rather than looping back to FLAC.
    if force_compat && is_browser_native_ext(resolved_ext) && !is_alac {
        let br = if track.bitrate > 0 && !is_lossless_ext(resolved_ext) {
            track.bitrate.clamp(256, 320)
        } else {
            320
        };
        return ("mp3".to_string(), br);
    }
    choose_web_transcode_params(resolved_ext, track)
}

fn choose_web_transcode_params(resolved_ext: &str, track: &Track) -> (String, i32) {
    let ext = resolved_ext.trim().trim_start_matches('.').to_lowercase();
    let lossless = is_lossless_ext(&ext)
        || is_alac_m4a(&ext, track)
        || (ext.is_empty() && track.is_lossless());
    if lossless {
        return ("flac".to_string(), 0);
    }
    let br = if track.bitrate > 0 {
        track.bitrate.clamp(256, 320)
    } else {
        320
    };
    ("mp3".to_string(), br)
}

fn choose_transcode_params(
    want_fmt: Option<&str>,
    max_bitrate: Option<i32>,
    track: &Track,
) -> (String, i32) {
    let has_cap = max_bitrate.map(|b| b > 0).unwrap_or(false);
    let fmt = want_fmt.unwrap_or_else(|| {
        if track.format.is_empty() {
            "mp3"
        } else if is_lossless_ext(&track.format) {
            if has_cap {
                "mp3"
            } else {
                "flac"
            }
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

    #[test]
    fn web_compat_detects_native_vs_unsupported_formats() {
        for ext in ["mp3", "flac", "wav", "m4a", "mp4", "aac", "ogg", "opus", "webm", ""] {
            assert!(is_browser_native_ext(ext), "expected native: {}", ext);
        }
        for ext in ["alac", "ape", "wv", "aiff", "aif", "dsf", "wma"] {
            assert!(!is_browser_native_ext(ext), "expected unsupported: {}", ext);
        }
    }

    #[test]
    fn web_transcode_preserves_lossless_as_flac_and_lossy_as_high_mp3() {
        let alac = Track {
            format: "alac".to_string(),
            bitrate: 900,
            ..Default::default()
        };
        assert_eq!(choose_web_transcode_params("alac", &alac), ("flac".to_string(), 0));

        // ALAC inside .m4a container detected via bit depth / high bitrate
        let alac_m4a = Track {
            format: "m4a".to_string(),
            bitrate: 950,
            bit_depth: 16,
            ..Default::default()
        };
        assert_eq!(choose_web_transcode_params("m4a", &alac_m4a), ("flac".to_string(), 0));

        // Streaming track upgraded to local APE sibling
        let ytm_upgraded = Track {
            format: "m4a".to_string(),
            bitrate: 192,
            ..Default::default()
        };
        assert_eq!(choose_web_transcode_params("ape", &ytm_upgraded), ("flac".to_string(), 0));

        // Lossy incompatible format (e.g. WMA or Safari Opus fallback) -> high bitrate MP3
        let wma = Track {
            format: "wma".to_string(),
            bitrate: 192,
            ..Default::default()
        };
        assert_eq!(choose_web_transcode_params("wma", &wma), ("mp3".to_string(), 256));
        assert_eq!(choose_web_transcode_params("opus", &ytm_upgraded), ("mp3".to_string(), 256));
    }

    #[test]
    fn safari_ua_detection_excludes_chromium_and_firefox() {
        let mut safari_hdrs = HeaderMap::new();
        safari_hdrs.insert(
            header::USER_AGENT,
            HeaderValue::from_static("Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15"),
        );
        assert!(is_safari_ua(&safari_hdrs));

        let mut chrome_hdrs = HeaderMap::new();
        chrome_hdrs.insert(
            header::USER_AGENT,
            HeaderValue::from_static("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36"),
        );
        assert!(!is_safari_ua(&chrome_hdrs));

        let mut edge_hdrs = HeaderMap::new();
        edge_hdrs.insert(
            header::USER_AGENT,
            HeaderValue::from_static("Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0) AppleWebKit/537.36 Chrome/131.0.0.0 Safari/537.36 Edg/131.0.0.0"),
        );
        assert!(!is_safari_ua(&edge_hdrs));
    }

    #[test]
    fn play_target_ext_prefers_resolved_extension() {
        let track = Track {
            format: "m4a".to_string(),
            ..Default::default()
        };
        let local = PlayTarget::LocalFile {
            path: PathBuf::from("/music/01.ALAC"),
            content_type: "audio/mp4".to_string(),
        };
        assert_eq!(play_target_ext(&local, &track), "alac");

        let remote = PlayTarget::RemoteHttp {
            url: "https://example.com/audio".to_string(),
            headers: vec![],
            content_type: "audio/webm".to_string(),
            ext: "webm".to_string(),
            webpage_url: String::new(),
            cacheable: false,
        };
        assert_eq!(play_target_ext(&remote, &track), "webm");
    }

    #[test]
    fn sanitize_time_offset_rejects_nan_inf_and_negatives() {
        assert_eq!(sanitize_time_offset(f64::NAN), 0.0);
        assert_eq!(sanitize_time_offset(f64::INFINITY), 0.0);
        assert_eq!(sanitize_time_offset(-12.0), 0.0);
        assert_eq!(sanitize_time_offset(0.2), 0.0);
        assert_eq!(sanitize_time_offset(42.5), 42.5);
        assert_eq!(sanitize_time_offset(999_999.0), 86_400.0);
    }

    #[tokio::test]
    async fn serve_local_file_range_returns_416_on_unsatisfiable_bytes_range() {
        let dir = std::env::temp_dir().join(format!("beatlab_range_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let file_path = dir.join("sample.mp3");
        std::fs::write(&file_path, b"0123456789").unwrap();

        let bad_resp = serve_local_file_range(&file_path, "audio/mpeg", Some("bytes=999-")).await;
        assert_eq!(bad_resp.status(), StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(
            bad_resp
                .headers()
                .get(header::CONTENT_RANGE)
                .and_then(|v| v.to_str().ok()),
            Some("bytes */10")
        );

        let ok_resp = serve_local_file_range(&file_path, "audio/mpeg", Some("bytes=2-5")).await;
        assert_eq!(ok_resp.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            ok_resp
                .headers()
                .get(header::CONTENT_RANGE)
                .and_then(|v| v.to_str().ok()),
            Some("bytes 2-5/10")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_local_or_cached_sibling_matches_despite_remaster_title_tag() {
        use crate::sources::archive::ArchiveSource;
        use crate::sources::deezer::DeezerSource;
        use crate::sources::local::LocalSource;
        use crate::sources::soulseek::SoulseekSource;
        use crate::sources::ytm::YtmSource;
        use crate::sources::SourceRegistry;

        let dir = std::env::temp_dir().join(format!("beatlab_sibling_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let local_flac = dir.join("take_on_me.flac");
        std::fs::write(&local_flac, vec![0u8; 8192]).unwrap();

        let store = Store::open_memory().unwrap();
        let sources = SourceRegistry::new(
            DeezerSource::new(),
            YtmSource::new(None),
            ArchiveSource::new(),
            Arc::new(LocalSource::new(store.clone(), vec![])),
            SoulseekSource::new(store.clone(), dir.join("slsk")),
        );
        let engine = StreamEngine::new(store.clone(), sources, &dir, 1.0);

        let local_track = Track {
            id: "loc:aha-take-on-me".into(),
            source: "local".into(),
            title: "Take On Me".into(),
            artist: "a-ha".into(),
            duration: 225,
            format: "flac".into(),
            bitrate: 1000,
            stream_url: format!("file:{}", local_flac.display()),
            ..Default::default()
        };
        store.upsert_tracks(&[local_track]).unwrap();

        let tagged_stream_track = Track {
            id: "dz:aha-remaster".into(),
            source: "deezer".into(),
            title: "Take On Me (2015 Remaster)".into(),
            artist: "a-ha feat. Other".into(),
            duration: 226,
            format: "m4a".into(),
            bitrate: 256,
            stream_url: "resolve:ytm:a-ha|Take On Me|226".into(),
            ..Default::default()
        };

        let target = engine.find_local_or_cached_sibling(&tagged_stream_track);
        match target {
            Some(PlayTarget::LocalFile { path, .. }) => assert_eq!(path, local_flac),
            other => panic!("expected local sibling match, got {:?}", other.is_some()),
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn serve_cover_resolves_auto_playlist_first_track() {
        use crate::sources::archive::ArchiveSource;
        use crate::sources::deezer::DeezerSource;
        use crate::sources::local::LocalSource;
        use crate::sources::soulseek::SoulseekSource;
        use crate::sources::ytm::YtmSource;
        use crate::sources::SourceRegistry;
        use axum::body::to_bytes;

        let dir = std::env::temp_dir().join(format!("beatlab_cover_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::open_memory().unwrap();
        let sources = SourceRegistry::new(
            DeezerSource::new(),
            YtmSource::new(None),
            ArchiveSource::new(),
            Arc::new(LocalSource::new(store.clone(), vec![])),
            SoulseekSource::new(store.clone(), dir.join("slsk")),
        );
        let engine = StreamEngine::new(store.clone(), sources, &dir, 1.0);

        let track = Track {
            id: "loc:cover-t1".into(),
            source: "local".into(),
            title: "Horizon".into(),
            artist: "Tycho".into(),
            album: "Awake".into(),
            duration: 210,
            ..Default::default()
        };
        store.upsert_tracks(&[track.clone()]).unwrap();
        let (u, _) = store.authenticate_or_create_user("cover_u", "pw").unwrap();
        store
            .upsert_auto_playlist(
                "auto-u1-supermix",
                u.id,
                "My Supermix",
                "",
                "supermix",
                &[track.id.clone()],
            )
            .unwrap();

        let auto_resp = engine.serve_cover("auto-u1-supermix").await;
        let track_resp = engine.serve_cover(&track.id).await;
        let missing_resp = engine.serve_cover("auto-u1-nonexistent").await;

        assert_eq!(auto_resp.status(), StatusCode::OK);
        let auto_bytes = to_bytes(auto_resp.into_body(), 1024 * 64).await.unwrap();
        let track_bytes = to_bytes(track_resp.into_body(), 1024 * 64).await.unwrap();
        let missing_bytes = to_bytes(missing_resp.into_body(), 1024 * 64).await.unwrap();
        assert_eq!(auto_bytes, track_bytes);
        assert_ne!(auto_bytes, missing_bytes);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn compat_transcode_handles_flac_fallback_and_ytm_m4a_fallback_cleanly() {
        // When a Soulseek/Archive FLAC falls back to a YTM m4a stream, is_alac_m4a must be false
        // so lossy m4a streams directly instead of bogus transcode to FLAC.
        let flac_track = Track {
            format: "flac".to_string(),
            bitrate: 950,
            bit_depth: 16,
            ..Default::default()
        };
        assert!(!is_alac_m4a("m4a", &flac_track));

        // Native FLAC with force_compat=true (retry after browser error) transcodes to 320k MP3.
        assert_eq!(
            choose_web_compat_transcode_params("flac", &flac_track, true, false),
            ("mp3".to_string(), 320)
        );
        // Time-offset seek without force_compat keeps FLAC.
        assert_eq!(
            choose_web_compat_transcode_params("flac", &flac_track, false, false),
            ("flac".to_string(), 0)
        );

        // ALAC in .m4a still converts to lossless FLAC.
        let alac_m4a = Track {
            format: "m4a".to_string(),
            bitrate: 950,
            bit_depth: 16,
            ..Default::default()
        };
        assert!(is_alac_m4a("m4a", &alac_m4a));
        assert_eq!(
            choose_web_compat_transcode_params("m4a", &alac_m4a, true, true),
            ("flac".to_string(), 0)
        );
    }

    #[tokio::test]
    async fn soulseek_track_falls_back_to_local_sibling_when_offline() {
        use crate::sources::archive::ArchiveSource;
        use crate::sources::deezer::DeezerSource;
        use crate::sources::local::LocalSource;
        use crate::sources::soulseek::SoulseekSource;
        use crate::sources::ytm::YtmSource;
        use crate::sources::SourceRegistry;

        let dir = std::env::temp_dir().join(format!("beatlab_slsk_fb_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let local_flac = dir.join("look_at_the_sky.flac");
        std::fs::write(&local_flac, vec![0u8; 8192]).unwrap();

        let store = Store::open_memory().unwrap();
        let sources = SourceRegistry::new(
            DeezerSource::new(),
            YtmSource::new(None),
            ArchiveSource::new(),
            Arc::new(LocalSource::new(store.clone(), vec![])),
            SoulseekSource::new(store.clone(), dir.join("slsk")),
        );
        let engine = StreamEngine::new(store.clone(), sources, &dir, 1.0);

        let local_track = Track {
            id: "local:porter-sky".into(),
            source: "local".into(),
            title: "Look at the Sky".into(),
            artist: "Porter Robinson".into(),
            album: "Nurture".into(),
            duration: 309,
            format: "flac".into(),
            bitrate: 1000,
            stream_url: format!("file:{}", local_flac.display()),
            ..Default::default()
        };
        store.upsert_tracks(&[local_track]).unwrap();

        // Even an older uncleaned Soulseek DB entry falls back cleanly when Soulseek is offline.
        let slsk_track = Track {
            id: "soulseek:deadbeef".into(),
            source: "soulseek".into(),
            title: "Nurture - Look at the Sky".into(),
            artist: "Porter.Robinson".into(),
            album: "Nurture".into(),
            duration: 309,
            format: "flac".into(),
            bitrate: 950,
            bit_depth: 16,
            stream_url: "slsk://offline_peer/@@offline_peer/Nurture - Look at the Sky.flac".into(),
            ..Default::default()
        };

        let (clean_art, clean_title) = clean_track_query_for_fallback(&slsk_track);
        assert_eq!(clean_art, "Porter Robinson");
        assert_eq!(clean_title, "Look at the Sky");

        match engine.resolve_target(&slsk_track).await.unwrap() {
            PlayTarget::LocalFile { path, .. } => assert_eq!(path, local_flac),
            _ => panic!("expected local fallback"),
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn notify_leader_guard_cleans_up_on_cancellation_and_warm_respects_bounds() {
        let map = Arc::new(StdMutex::new(HashMap::new()));
        let notify = Arc::new(Notify::new());
        {
            let mut m = map.lock().unwrap();
            m.insert("track-x".to_string(), Arc::clone(&notify));
        }

        // Simulate a cancelled HTTP stream leader future dropping mid-await.
        let waiter = notify.notified();
        tokio::pin!(waiter);
        let fut = {
            let map_c = Arc::clone(&map);
            let n_c = Arc::clone(&notify);
            async move {
                let _guard = NotifyLeaderGuard {
                    map: map_c,
                    key: "track-x".to_string(),
                    notify: n_c,
                };
                futures::future::pending::<()>().await;
            }
        };
        let timed = tokio::time::timeout(Duration::from_millis(20), fut).await;
        assert!(timed.is_err());

        // Map entry must be removed AND waiter must wake immediately (no deadlock).
        assert!(!map.lock().unwrap().contains_key("track-x"));
        assert!(
            tokio::time::timeout(Duration::from_millis(200), waiter)
                .await
                .is_ok(),
            "follower waiter should be woken when cancelled leader drops"
        );

        // Rapid spawn_warm_track bursts must never block or panic.
        let dir = std::env::temp_dir().join(format!("beatlab_warm_burst_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let audio = dir.join("song.mp3");
        std::fs::write(&audio, vec![0u8; 8192]).unwrap();
        let store = Store::open_memory().unwrap();
        let sources = crate::sources::SourceRegistry::new(
            crate::sources::deezer::DeezerSource::new(),
            crate::sources::ytm::YtmSource::new(None),
            crate::sources::archive::ArchiveSource::new(),
            Arc::new(crate::sources::local::LocalSource::new(store.clone(), vec![])),
            crate::sources::soulseek::SoulseekSource::new(store.clone(), dir.join("slsk")),
        );
        let engine = StreamEngine::new(store, sources, &dir, 1.0);
        for i in 0..40 {
            let t = Track {
                id: format!("local:burst-{}", i),
                source: "local".into(),
                stream_url: format!("file:{}", audio.display()),
                ..Default::default()
            };
            engine.spawn_warm_track(t);
        }
        tokio::time::sleep(Duration::from_millis(40)).await;
        assert!(engine.in_flight_warms.lock().unwrap().len() <= 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
