use crate::models::Track;
use crate::sources::SourceRegistry;
use crate::sources::local::extract_embedded_cover;
use crate::store::Store;
use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use futures::StreamExt;
use sha1::{Digest, Sha1};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Semaphore;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::process::Command;
use tokio_util::io::ReaderStream;
use tracing::info;

pub struct Engine {
    store: Store,
    registry: Arc<SourceRegistry>,
    cache_dir: PathBuf,
    cover_dir: PathBuf,
    budget_bytes: i64,
    http: reqwest::Client,
    cover_http: reqwest::Client,
    has_ffmpeg: bool,
    cover_sem: Arc<Semaphore>,
    cover_neg_cache: Mutex<HashMap<String, i64>>,
    inflight_resolves: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl Engine {
    pub fn new(store: Store, registry: Arc<SourceRegistry>, data_dir: &Path, cache_gb: i64) -> Self {
        let cache_dir = data_dir.join("cache").join("audio");
        let cover_dir = data_dir.join("cache").join("covers");
        let _ = std::fs::create_dir_all(&cache_dir);
        let _ = std::fs::create_dir_all(&cover_dir);
        sweep_partials(&cache_dir);

        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .user_agent("Beatlab/0.2")
            .build()
            .unwrap_or_default();

        let cover_http = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .user_agent("Beatlab/0.2")
            .build()
            .unwrap_or_default();

        let has_ffmpeg = std::process::Command::new("ffmpeg")
            .arg("-version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        Self {
            store,
            registry,
            cache_dir,
            cover_dir,
            budget_bytes: cache_gb.max(1) << 30,
            http,
            cover_http,
            has_ffmpeg,
            cover_sem: Arc::new(Semaphore::new(12)),
            cover_neg_cache: Mutex::new(HashMap::new()),
            inflight_resolves: Mutex::new(HashMap::new()),
        }
    }

    pub fn has_ffmpeg(&self) -> bool {
        self.has_ffmpeg
    }

    pub fn cache_budget_bytes(&self) -> i64 {
        self.budget_bytes
    }

    pub fn clear_audio_cache(&self) {
        for ent in self.store.cache_entries_oldest() {
            let _ = std::fs::remove_file(&ent.path);
            self.store.cache_delete(&ent.track_id);
        }
    }

    fn cached_resolved_url(&self, key: &str) -> Option<String> {
        let raw = self.store.kv_get(key)?;
        let ent = serde_json::from_str::<serde_json::Value>(&raw).ok()?;
        let u = ent["u"].as_str().unwrap_or("");
        let at = ent["a"].as_i64().unwrap_or(0);
        let age = chrono::Utc::now().timestamp() - at;
        let valid_local = u
            .strip_prefix("file:")
            .map(|p| Path::new(p).exists())
            .unwrap_or(true);
        if !u.is_empty() && age < 5 * 3600 && valid_local {
            Some(u.to_string())
        } else {
            None
        }
    }

    pub async fn resolve_url(&self, track: &Track, fresh: bool) -> Option<String> {
        if !track.stream_url.starts_with("ytdlp:") && !track.stream_url.starts_with("slsk:") {
            return Some(track.stream_url.clone());
        }
        let key = format!("ytres:{}", hash_str(&track.stream_url));
        if !fresh {
            if let Some(u) = self.cached_resolved_url(&key) {
                return Some(u);
            }
        }
        let lock = {
            let mut map = self.inflight_resolves.lock().unwrap();
            if map.len() > 256 {
                map.retain(|_, v| Arc::strong_count(v) > 1);
            }
            Arc::clone(
                map.entry(key.clone())
                    .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
            )
        };
        let _guard = lock.lock().await;
        if !fresh {
            if let Some(u) = self.cached_resolved_url(&key) {
                return Some(u);
            }
        }
        let resolved = self
            .registry
            .resolve_stream(&track.source, &track.stream_url)
            .await?;
        let payload = serde_json::json!({
            "u": resolved,
            "a": chrono::Utc::now().timestamp()
        });
        self.store.kv_set(&key, &payload.to_string());
        Some(resolved)
    }

    /// Pre-resolves upcoming `ytdlp:` tracks in the background so playback starts instantly.
    pub fn warm_resolve(self: &Arc<Self>, tracks: Vec<Track>, limit: usize) {
        let refs: Vec<Track> = tracks
            .into_iter()
            .filter(|t| t.stream_url.starts_with("ytdlp:"))
            .take(limit)
            .collect();
        if refs.is_empty() {
            return;
        }
        let eng = Arc::clone(self);
        tokio::spawn(async move {
            futures::stream::iter(refs)
                .for_each_concurrent(3, |t| {
                    let eng = Arc::clone(&eng);
                    async move {
                        let _ = eng.resolve_url(&t, false).await;
                    }
                })
                .await;
        });
    }

    /// Serves a track with bit-perfect quality by default (full HTTP Range support),
    /// transcoding only if requested or when the source format isn't browser-playable (e.g. ALAC/AIFF).
    /// Automatically falls back to an alternate source for the same song if the primary source fails.
    pub async fn serve_track(
        self: &Arc<Self>,
        track: &Track,
        req_headers: &HeaderMap,
        target_format: Option<&str>,
        max_bitrate: Option<i32>,
    ) -> Response {
        if let Some(resp) = self
            .serve_track_once(track, req_headers, target_format, max_bitrate)
            .await
        {
            return resp;
        }

        // Primary source failed (e.g. Soulseek peer queued/offline or expired URL); try fallback sources.
        let target_key = track.song_key();
        let query = format!("{} {}", track.artist, track.title);
        let mut candidates: Vec<Track> = self
            .store
            .search_tracks(&query, 12)
            .into_iter()
            .filter(|t| t.id != track.id && t.source != track.source && t.song_key() == target_key)
            .collect();

        if candidates.is_empty() && !query.trim().is_empty() {
            let remote = self.registry.search_all_raw(&query, 8).await;
            for t in remote {
                if t.id != track.id
                    && t.source != track.source
                    && (t.song_key() == target_key
                        || crate::store::search_match_score(&t, &query) >= 60)
                {
                    candidates.push(t);
                }
            }
        }

        // Prefer fast-streaming HTTP/ytdlp sources first for instant fallback playback.
        candidates.sort_by_key(|t| match t.source.as_str() {
            "local" => 0,
            "youtube" | "soundcloud" => 1,
            "bandcamp" | "archive" => 2,
            _ => 3,
        });

        for alt in candidates.into_iter().take(3) {
            if let Some(resp) = self
                .serve_track_once(&alt, req_headers, target_format, max_bitrate)
                .await
            {
                return resp;
            }
        }

        (StatusCode::BAD_GATEWAY, "stream unavailable").into_response()
    }

    async fn serve_track_once(
        self: &Arc<Self>,
        track: &Track,
        req_headers: &HeaderMap,
        target_format: Option<&str>,
        max_bitrate: Option<i32>,
    ) -> Option<Response> {
        let is_browser_incompatible = matches!(
            track.format.to_lowercase().as_str(),
            "alac" | "aiff" | "aif" | "ape" | "wv" | "dsf" | "wma"
        );
        let raw_requested = target_format
            .map(|f| f.eq_ignore_ascii_case("raw"))
            .unwrap_or(false);
        let effective_target_fmt = if target_format
            .map(|f| !f.is_empty() && !f.eq_ignore_ascii_case("raw"))
            .unwrap_or(false)
        {
            target_format
        } else if is_browser_incompatible && !raw_requested {
            Some("flac")
        } else {
            None
        };

        let wants_transcode = self.has_ffmpeg
            && ((max_bitrate.unwrap_or(0) > 0 && track.bitrate > max_bitrate.unwrap_or(0))
                || effective_target_fmt
                    .map(|f| !f.eq_ignore_ascii_case(&track.format))
                    .unwrap_or(false));

        // 1. Local file source
        if let Some(local_path) = track.stream_url.strip_prefix("file:") {
            let path = PathBuf::from(local_path);
            if path.exists() {
                if wants_transcode {
                    let s = path.to_string_lossy();
                    return self
                        .serve_transcoded(
                            &s,
                            effective_target_fmt.unwrap_or("mp3"),
                            max_bitrate.unwrap_or(320),
                        )
                        .await;
                }
                return Some(serve_local_file(&path, &track.format, req_headers).await);
            }
            return None;
        }

        // 2. Cached file on disk
        if let Some(ent) = self.store.cache_get(&track.id) {
            let p = PathBuf::from(&ent.path);
            if p.exists() {
                self.store.cache_touch(&track.id);
                if wants_transcode {
                    let s = p.to_string_lossy();
                    return self
                        .serve_transcoded(
                            &s,
                            effective_target_fmt.unwrap_or("mp3"),
                            max_bitrate.unwrap_or(320),
                        )
                        .await;
                }
                return Some(serve_local_file(&p, &track.format, req_headers).await);
            }
            self.store.cache_delete(&track.id);
        }

        // 3. Stream from upstream URL + tee to disk cache
        let mut url = self.resolve_url(track, false).await?;

        if let Some(local_path) = url.strip_prefix("file:") {
            let path = PathBuf::from(local_path);
            if path.exists() {
                if wants_transcode {
                    let s = path.to_string_lossy();
                    return self
                        .serve_transcoded(
                            &s,
                            effective_target_fmt.unwrap_or("mp3"),
                            max_bitrate.unwrap_or(320),
                        )
                        .await;
                }
                return Some(serve_local_file(&path, &track.format, req_headers).await);
            }
            return None;
        }

        if wants_transcode {
            return self
                .serve_transcoded(
                    &url,
                    effective_target_fmt.unwrap_or("mp3"),
                    max_bitrate.unwrap_or(320),
                )
                .await;
        }

        let range_hdr = req_headers
            .get(header::RANGE)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let mut resp_res = self.fetch_upstream(&url, range_hdr.as_deref()).await;
        let should_retry = match &resp_res {
            Err(_) => true,
            Ok(r) => matches!(
                r.status(),
                reqwest::StatusCode::FORBIDDEN
                    | reqwest::StatusCode::NOT_FOUND
                    | reqwest::StatusCode::GONE
            ),
        };
        if should_retry && track.stream_url.starts_with("ytdlp:") {
            if let Some(fresh_url) = self.resolve_url(track, true).await {
                url = fresh_url;
                resp_res = self.fetch_upstream(&url, range_hdr.as_deref()).await;
            }
        }
        let resp = resp_res.ok()?;

        if !resp.status().is_success() && resp.status() != reqwest::StatusCode::PARTIAL_CONTENT {
            return None;
        }

        let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::OK);
        let content_len = resp.content_length();
        let mut out_headers = HeaderMap::new();
        for h in ["content-length", "content-range", "accept-ranges", "etag"] {
            if let Some(v) = resp.headers().get(h) {
                if let Ok(name) = axum::http::HeaderName::from_bytes(h.as_bytes()) {
                    out_headers.insert(name, v.clone());
                }
            }
        }
        let ct = upstream_content_type(&resp, &url, &track.format);
        if let Ok(hv) = HeaderValue::from_str(&ct) {
            out_headers.insert(header::CONTENT_TYPE, hv);
        } else {
            out_headers.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static(content_type_for_format(&track.format)),
            );
        }

        let expected_len_u64 = content_len.unwrap_or(0);
        let cacheable = range_hdr.is_none()
            && status == StatusCode::OK
            && expected_len_u64 > 0;

        if !cacheable {
            let stream = resp.bytes_stream();
            let body = Body::from_stream(stream);
            return Some((status, out_headers, body).into_response());
        }

        // Tee stream to disk cache
        let expected_len = expected_len_u64 as i64;
        let tmp_name = format!("{}.{}.part", hash_str(&track.id), uuid::Uuid::new_v4());
        let tmp_path = self.cache_dir.join(tmp_name);
        let ext = if track.format.is_empty() { "bin" } else { &track.format };
        let final_path = self.cache_dir.join(format!("{}.{}", hash_str(&track.id), ext));

        let Ok(mut file) = tokio::fs::File::create(&tmp_path).await else {
            let body = Body::from_stream(resp.bytes_stream());
            return Some((status, out_headers, body).into_response());
        };

        let track_id = track.id.clone();
        let eng = Arc::clone(self);
        let mut upstream = resp.bytes_stream();
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<bytes::Bytes, std::io::Error>>(16);

        tokio::spawn(async move {
            let mut written: i64 = 0;
            let mut ok = true;
            while let Some(chunk) = upstream.next().await {
                match chunk {
                    Ok(bytes) => {
                        written += bytes.len() as i64;
                        if file.write_all(&bytes).await.is_err() {
                            ok = false;
                        }
                        if tx.send(Ok(bytes)).await.is_err() {
                            ok = false;
                            break;
                        }
                    }
                    Err(e) => {
                        ok = false;
                        let _ = tx.send(Err(std::io::Error::other(e))).await;
                        break;
                    }
                }
            }
            let _ = file.flush().await;
            drop(file);
            if ok && written == expected_len {
                if tokio::fs::rename(&tmp_path, &final_path).await.is_ok() {
                    eng.store
                        .cache_put(&track_id, &final_path.to_string_lossy(), written);
                    eng.evict();
                    info!(track_id = %track_id, bytes = written, "cached audio track");
                } else {
                    let _ = tokio::fs::remove_file(&tmp_path).await;
                }
            } else {
                let _ = tokio::fs::remove_file(&tmp_path).await;
            }
        });

        let stream = tokio_util::io::ReaderStream::new(tokio_util::io::StreamReader::new(
            tokio_stream_from_mpsc(rx),
        ));
        let body = Body::from_stream(stream);
        Some((status, out_headers, body).into_response())
    }

    async fn fetch_upstream(
        &self,
        url: &str,
        range: Option<&str>,
    ) -> Result<reqwest::Response, reqwest::Error> {
        let mut req = self.http.get(url);
        if let Some(r) = range {
            req = req.header(reqwest::header::RANGE, r);
        }
        req.send().await
    }

    async fn serve_transcoded(&self, input: &str, fmt: &str, bitrate_kbps: i32) -> Option<Response> {
        let br = format!("{}k", bitrate_kbps.clamp(64, 320));
        let (codec, out_fmt, ct) = match fmt.to_lowercase().as_str() {
            "flac" => ("flac", "flac", "audio/flac"),
            "opus" => ("libopus", "opus", "audio/ogg"),
            "ogg" => ("libvorbis", "ogg", "audio/ogg"),
            "aac" | "m4a" => ("aac", "adts", "audio/aac"),
            _ => ("libmp3lame", "mp3", "audio/mpeg"),
        };

        let mut child = Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-i",
                input,
                "-vn",
                "-c:a",
                codec,
                "-b:a",
                &br,
                "-f",
                out_fmt,
                "pipe:1",
            ])
            .stdout(std::process::Stdio::piped())
            .spawn()
            .ok()?;

        let mut stdout = child.stdout.take()?;
        // Verify ffmpeg produces real audio bytes before committing a 200 OK header,
        // allowing `serve_track` to fall back if transcoding the source fails.
        let mut head = vec![0u8; 4096];
        let n = match tokio::time::timeout(Duration::from_secs(12), stdout.read(&mut head)).await {
            Ok(Ok(n)) if n > 0 => n,
            _ => {
                let _ = child.kill().await;
                return None;
            }
        };
        head.truncate(n);

        tokio::spawn(async move {
            let _ = child.wait().await;
        });

        let stream = futures::stream::once(async move {
            Ok::<_, std::io::Error>(bytes::Bytes::from(head))
        })
        .chain(ReaderStream::new(stdout));
        Some(
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, HeaderValue::from_static(ct))],
                Body::from_stream(stream),
            )
                .into_response(),
        )
    }

    pub fn evict(&self) {
        let mut used = self.store.cache_bytes();
        while used > self.budget_bytes {
            let entries = self.store.cache_entries_oldest();
            if entries.is_empty() {
                break;
            }
            let mut progressed = false;
            for ent in entries {
                if used <= self.budget_bytes {
                    break;
                }
                if (std::fs::remove_file(&ent.path).is_ok() || !Path::new(&ent.path).exists())
                    && self.store.cache_delete(&ent.track_id)
                {
                    used = used.saturating_sub(ent.bytes);
                    progressed = true;
                }
            }
            if !progressed {
                break;
            }
        }
    }

    pub async fn serve_cover(&self, id: &str) -> Response {
        if let Some(raw_artist) = id
            .strip_prefix("ar:")
            .and_then(|_| self.store.artist_name_from_id(id))
        {
            let clean_artist = crate::models::primary_artist(&raw_artist);
            if !clean_artist.is_empty()
                && !clean_artist.eq_ignore_ascii_case("unknown artist")
                && !crate::models::is_generic_folder_name(&clean_artist)
            {
                let ar_slug = crate::models::slug(&clean_artist);
                let cache_path = self.cover_dir.join(format!("artist_{}.img", ar_slug));
                if let Ok(bytes) = tokio::fs::read(&cache_path).await {
                    let ct = sniff_image_mime(&bytes);
                    return cover_response(bytes, ct);
                }
                let neg_key = format!("ar:{}", ar_slug);
                let now = chrono::Utc::now().timestamp();
                if !self.is_cover_neg_cached(&neg_key, now) {
                    if let Ok(_permit) = self.cover_sem.acquire().await {
                        if let Some(art_url) = self.lookup_artist_image(&clean_artist).await {
                            if let Some(resp) = self.fetch_and_cache_image(&art_url, &cache_path).await {
                                return resp;
                            }
                        } else {
                            self.mark_cover_neg_cached(neg_key, now);
                        }
                    }
                }
            }
        }

        let track = self.resolve_cover_track(id);

        if let Some(ref t) = track {
            if let Some(local_path) = t.cover_url.strip_prefix("local-art:") {
                let cache_path = self.cover_dir.join(format!("loc_{}.img", hash_str(local_path)));
                if let Ok(bytes) = tokio::fs::read(&cache_path).await {
                    let ct = sniff_image_mime(&bytes);
                    return cover_response(bytes, ct);
                }
                let p = PathBuf::from(local_path);
                let extracted = tokio::task::spawn_blocking(move || extract_embedded_cover(&p))
                    .await
                    .ok()
                    .flatten();
                if let Some((bytes, mime)) = extracted {
                    let _ = tokio::fs::write(&cache_path, &bytes).await;
                    return cover_response(bytes, &mime);
                }
            }

            if t.cover_url.starts_with("http") {
                let cache_path = self.cover_dir.join(format!("{}.img", hash_str(&t.cover_url)));
                if let Ok(bytes) = tokio::fs::read(&cache_path).await {
                    let ct = sniff_image_mime(&bytes);
                    return cover_response(bytes, ct);
                }
                if let Ok(_permit) = self.cover_sem.acquire().await {
                    if let Some(resp) = self.fetch_and_cache_image(&t.cover_url, &cache_path).await {
                        return resp;
                    }
                }
            }

            // Missing cover_url (e.g. Soulseek or untagged local): look up real album art
            // via iTunes/Deezer and cache both the image bytes and the URL in SQLite.
            let clean_artist = crate::models::primary_artist(&t.artist);
            let clean_album = crate::models::strip_release_tags(&t.album);
            let clean_title = crate::models::strip_release_tags(&t.title);
            if !clean_artist.is_empty()
                && !clean_artist.eq_ignore_ascii_case("unknown artist")
                && !crate::models::is_generic_folder_name(&clean_artist)
            {
                let target = if !clean_album.is_empty()
                    && !clean_album.eq_ignore_ascii_case("unknown album")
                    && !crate::models::is_generic_folder_name(&clean_album)
                {
                    &clean_album
                } else {
                    &clean_title
                };
                if !target.is_empty() {
                    let art_key = format!("{}|{}", clean_artist.to_lowercase(), target.to_lowercase());
                    let cache_path = self.cover_dir.join(format!("art_{}.img", hash_str(&art_key)));
                    if let Ok(bytes) = tokio::fs::read(&cache_path).await {
                        let ct = sniff_image_mime(&bytes);
                        return cover_response(bytes, ct);
                    }
                    let now = chrono::Utc::now().timestamp();
                    if !self.is_cover_neg_cached(&art_key, now) {
                        if let Ok(_permit) = self.cover_sem.acquire().await {
                            if let Some((art_url, genre)) = self
                                .lookup_artwork(&clean_artist, &clean_album, &clean_title)
                                .await
                            {
                                self.store.update_track_cover_and_genre(
                                    &t.id,
                                    Some(&art_url),
                                    genre.as_deref(),
                                );
                                if let Some(resp) =
                                    self.fetch_and_cache_image(&art_url, &cache_path).await
                                {
                                    return resp;
                                }
                            } else {
                                self.mark_cover_neg_cached(art_key, now);
                            }
                        }
                    }
                }
            }
        }

        // Crisp deterministic SVG fallback cover
        let label = if let Some(ref t) = track {
            t.title.chars().take(2).collect::<String>().to_uppercase()
        } else if let Some(name) = id
            .strip_prefix("ar:")
            .and_then(|_| self.store.artist_name_from_id(id))
        {
            name.chars().take(2).collect::<String>().to_uppercase()
        } else {
            "BL".to_string()
        };
        let svg = generate_svg_cover(id, &label);
        (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "image/svg+xml"),
                (header::CACHE_CONTROL, "public, max-age=86400"),
            ],
            svg,
        )
            .into_response()
    }

    fn resolve_cover_track(&self, id: &str) -> Option<Track> {
        if let Some(t) = self.store.track(id) {
            return Some(t);
        }
        if id.starts_with("ar:") {
            if let Some(name) = self.store.artist_name_from_id(id) {
                let trs = self.store.tracks_by_artist(&name, 10);
                return trs
                    .iter()
                    .find(|t| !t.cover_url.is_empty())
                    .cloned()
                    .or_else(|| trs.into_iter().next());
            }
        }
        if id.starts_with("al:") {
            let all = self.store.all_tracks(3000);
            return all
                .iter()
                .find(|t| t.album_id() == id && !t.cover_url.is_empty())
                .cloned()
                .or_else(|| all.into_iter().find(|t| t.album_id() == id));
        }
        if id.starts_with("pl:") || id.starts_with("auto:") {
            let trs = self.store.playlist_tracks(id);
            return trs
                .iter()
                .find(|t| !t.cover_url.is_empty())
                .cloned()
                .or_else(|| trs.into_iter().next());
        }
        None
    }

    fn is_cover_neg_cached(&self, key: &str, now: i64) -> bool {
        self.cover_neg_cache
            .lock()
            .unwrap()
            .get(key)
            .map(|&ts| now - ts < 300)
            .unwrap_or(false)
    }

    fn mark_cover_neg_cached(&self, key: String, now: i64) {
        let mut neg = self.cover_neg_cache.lock().unwrap();
        neg.insert(key, now);
        if neg.len() > 512 {
            neg.retain(|_, ts| now - *ts < 300);
            if neg.len() > 512 {
                neg.clear();
            }
        }
    }

    async fn fetch_and_cache_image(&self, url: &str, cache_path: &Path) -> Option<Response> {
        let resp = self.cover_http.get(url).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let header_ct = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !header_ct.is_empty()
            && !header_ct.starts_with("image/")
            && !header_ct.starts_with("application/octet-stream")
        {
            return None;
        }
        let bytes = resp.bytes().await.ok()?;
        if bytes.is_empty() {
            return None;
        }
        let sniffed = sniff_image_mime(&bytes);
        let ct = if header_ct.starts_with("image/") && sniffed == "image/jpeg" {
            header_ct.split(';').next().unwrap_or(sniffed).trim()
        } else {
            sniffed
        };
        let _ = tokio::fs::write(cache_path, &bytes).await;
        Some(cover_response(bytes.to_vec(), ct))
    }

    async fn lookup_artist_image(&self, artist: &str) -> Option<String> {
        let clean = crate::models::primary_artist(artist);
        if clean.is_empty()
            || clean.eq_ignore_ascii_case("unknown artist")
            || crate::models::is_generic_folder_name(&clean)
        {
            return None;
        }
        let dz_url = format!(
            "https://api.deezer.com/search/artist?q={}&limit=5",
            urlencoding::encode(&clean)
        );
        let resp = self.cover_http.get(&dz_url).send().await.ok()?;
        let json = resp.json::<serde_json::Value>().await.ok()?;
        let arr = json.get("data").and_then(|r| r.as_array())?;
        let target_slug = crate::models::slug(&clean);
        let picked = arr
            .iter()
            .find(|a| {
                a.get("name")
                    .and_then(|v| v.as_str())
                    .map(|n| {
                        n.eq_ignore_ascii_case(&clean)
                            || (!target_slug.is_empty() && crate::models::slug(n) == target_slug)
                    })
                    .unwrap_or(false)
            })
            .or_else(|| arr.first())?;
        for field in ["picture_xl", "picture_big", "picture_medium"] {
            if let Some(url) = picked.get(field).and_then(|v| v.as_str()) {
                if !url.is_empty() && !url.contains("/artist//") {
                    return Some(url.to_string());
                }
            }
        }
        None
    }

    async fn query_track_artwork(&self, term: &str) -> Option<(String, Option<String>)> {
        let itunes_url = format!(
            "https://itunes.apple.com/search?term={}&entity=song&limit=1",
            urlencoding::encode(term)
        );
        if let Ok(resp) = self.cover_http.get(&itunes_url).send().await {
            if let Ok(json) = resp.json::<serde_json::Value>().await {
                if let Some(first) = json.get("results").and_then(|r| r.as_array()).and_then(|a| a.first()) {
                    if let Some(art100) = first.get("artworkUrl100").and_then(|v| v.as_str()) {
                        let art600 = art100.replace("100x100bb", "600x600bb");
                        let genre = first
                            .get("primaryGenreName")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        return Some((art600, genre));
                    }
                }
            }
        }
        let dz_url = format!(
            "https://api.deezer.com/search?q={}&limit=1",
            urlencoding::encode(term)
        );
        if let Ok(resp) = self.cover_http.get(&dz_url).send().await {
            if let Ok(json) = resp.json::<serde_json::Value>().await {
                if let Some(first) = json.get("data").and_then(|r| r.as_array()).and_then(|a| a.first()) {
                    if let Some(alb) = first.get("album") {
                        if let Some(cov) = alb
                            .get("cover_xl")
                            .or_else(|| alb.get("cover_big"))
                            .and_then(|v| v.as_str())
                        {
                            if !cov.is_empty() {
                                return Some((cov.to_string(), None));
                            }
                        }
                    }
                }
            }
        }
        None
    }

    async fn lookup_artwork(&self, artist: &str, album: &str, title: &str) -> Option<(String, Option<String>)> {
        let clean_artist = crate::models::primary_artist(artist);
        if clean_artist.is_empty()
            || clean_artist.eq_ignore_ascii_case("unknown artist")
            || crate::models::is_generic_folder_name(&clean_artist)
        {
            return None;
        }
        let clean_album = crate::models::strip_release_tags(album);
        let clean_title = crate::models::strip_release_tags(title);

        let has_album = !clean_album.is_empty()
            && !clean_album.eq_ignore_ascii_case("unknown album")
            && !crate::models::is_generic_folder_name(&clean_album);
        if has_album {
            let term = format!("{} {}", clean_artist, clean_album);
            if let Some(hit) = self.query_track_artwork(&term).await {
                return Some(hit);
            }
        }
        if !clean_title.is_empty() && (!has_album || !clean_title.eq_ignore_ascii_case(&clean_album)) {
            let term = format!("{} {}", clean_artist, clean_title);
            if let Some(hit) = self.query_track_artwork(&term).await {
                return Some(hit);
            }
        }
        self.lookup_artist_image(&clean_artist)
            .await
            .map(|url| (url, None))
    }
}

fn cover_response(bytes: Vec<u8>, content_type: &str) -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, content_type.to_string()),
            (header::CACHE_CONTROL, "public, max-age=86400".to_string()),
        ],
        bytes,
    )
        .into_response()
}

pub fn sniff_audio_bytes(head: &[u8], format: &str) -> &'static str {
    if head.starts_with(b"fLaC") {
        return "audio/flac";
    }
    if head.starts_with(b"OggS") {
        return "audio/ogg";
    }
    if (head.len() >= 8 && &head[4..8] == b"ftyp") || head.starts_with(b"ftyp") {
        return "audio/mp4";
    }
    if head.len() >= 12 && &head[0..4] == b"RIFF" && &head[8..12] == b"WAVE" {
        return "audio/wav";
    }
    if head.starts_with(b"ID3")
        || (head.len() >= 2 && head[0] == 0xff && (head[1] & 0xe0) == 0xe0 && (head[1] & 0x06) != 0)
    {
        return "audio/mpeg";
    }
    if head.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
        return "audio/webm";
    }
    content_type_for_format(format)
}

async fn sniff_audio_mime(file: &mut tokio::fs::File, total: u64, format: &str) -> &'static str {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};
    if total == 0 {
        return content_type_for_format(format);
    }
    let mut buf = [0u8; 16];
    let n = file.read(&mut buf).await.unwrap_or(0);
    let _ = file.seek(std::io::SeekFrom::Start(0)).await;
    sniff_audio_bytes(&buf[..n], format)
}

async fn serve_local_file(path: &Path, format: &str, req_headers: &HeaderMap) -> Response {
    let Ok(mut file) = tokio::fs::File::open(path).await else {
        return (StatusCode::NOT_FOUND, "file not found").into_response();
    };
    let Ok(meta) = file.metadata().await else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "stat failed").into_response();
    };
    let total = meta.len();
    let ct = sniff_audio_mime(&mut file, total, format).await;

    if let Some(range_val) = req_headers.get(header::RANGE).and_then(|v| v.to_str().ok()) {
        if let Ok(ranges) = http_range_header::parse_range_header(range_val) {
            if let Some(r) = ranges.ranges.first() {
                let start = match r.start {
                    http_range_header::StartPosition::Index(i) => i,
                    http_range_header::StartPosition::FromLast(n) => total.saturating_sub(n),
                };
                let end = match r.end {
                    http_range_header::EndPosition::Index(i) => i.min(total.saturating_sub(1)),
                    http_range_header::EndPosition::LastByte => total.saturating_sub(1),
                };
                if start <= end && start < total {
                    let len = end - start + 1;
                    if file.seek(std::io::SeekFrom::Start(start)).await.is_ok() {
                        let limited = file.take(len);
                        let stream = ReaderStream::new(limited);
                        return (
                            StatusCode::PARTIAL_CONTENT,
                            [
                                (header::CONTENT_TYPE, ct.to_string()),
                                (header::ACCEPT_RANGES, "bytes".to_string()),
                                (header::CONTENT_LENGTH, len.to_string()),
                                (
                                    header::CONTENT_RANGE,
                                    format!("bytes {}-{}/{}", start, end, total),
                                ),
                            ],
                            Body::from_stream(stream),
                        )
                            .into_response();
                    }
                } else {
                    return (
                        StatusCode::RANGE_NOT_SATISFIABLE,
                        [(header::CONTENT_RANGE, format!("bytes */{}", total))],
                    )
                        .into_response();
                }
            }
        }
    }

    let stream = ReaderStream::new(file);
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, ct.to_string()),
            (header::ACCEPT_RANGES, "bytes".to_string()),
            (header::CONTENT_LENGTH, total.to_string()),
        ],
        Body::from_stream(stream),
    )
        .into_response()
}

fn tokio_stream_from_mpsc(
    mut rx: tokio::sync::mpsc::Receiver<Result<bytes::Bytes, std::io::Error>>,
) -> impl futures::Stream<Item = Result<bytes::Bytes, std::io::Error>> {
    futures::stream::poll_fn(move |cx| rx.poll_recv(cx))
}

pub fn content_type_for_format(format: &str) -> &'static str {
    match format.to_lowercase().as_str() {
        "flac" => "audio/flac",
        "alac" | "m4a" | "aac" => "audio/mp4",
        "wav" => "audio/wav",
        "aiff" | "aif" => "audio/aiff",
        "mp3" => "audio/mpeg",
        "ogg" | "opus" => "audio/ogg",
        "webm" => "audio/webm",
        _ => "application/octet-stream",
    }
}

fn upstream_content_type(resp: &reqwest::Response, url: &str, track_format: &str) -> String {
    if let Some(raw) = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
    {
        let media = raw.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
        if media.starts_with("audio/") || media == "video/mp4" || media == "video/webm" {
            return media;
        }
    }
    if let Some((_, query)) = url.split_once('?') {
        for pair in query.split('&') {
            if let Some((k, v)) = pair.split_once('=') {
                if k.eq_ignore_ascii_case("mime") {
                    if let Ok(dec) = urlencoding::decode(v) {
                        let media = dec.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
                        if media.starts_with("audio/")
                            || media == "video/mp4"
                            || media == "video/webm"
                        {
                            return media;
                        }
                    }
                }
            }
        }
    }
    let path_part = url.split(['?', '#']).next().unwrap_or(url);
    if let Some(ext) = path_part.rsplit('.').next() {
        let from_ext = content_type_for_format(ext);
        if from_ext != "application/octet-stream" {
            return from_ext.to_string();
        }
    }
    content_type_for_format(track_format).to_string()
}

pub fn sniff_image_mime(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        return "image/png";
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return "image/webp";
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return "image/gif";
    }
    let trimmed = bytes
        .iter()
        .copied()
        .skip_while(|b| b.is_ascii_whitespace())
        .take(16)
        .collect::<Vec<_>>();
    if trimmed.starts_with(b"<svg") || trimmed.starts_with(b"<?xml") {
        return "image/svg+xml";
    }
    "image/jpeg"
}

fn hash_str(s: &str) -> String {
    let mut h = Sha1::new();
    h.update(s.as_bytes());
    hex::encode(h.finalize())
}

fn sweep_partials(dir: &Path) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for entry in rd.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if name.ends_with(".part") {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
    }
}

fn generate_svg_cover(seed: &str, label: &str) -> String {
    let h = hash_str(seed);
    let b0 = u8::from_str_radix(&h[0..2], 16).unwrap_or(90);
    let b1 = u8::from_str_radix(&h[2..4], 16).unwrap_or(180);
    let hue1 = (b0 as u32 * 360 / 255) % 360;
    let hue2 = (hue1 + 45 + (b1 as u32 % 60)) % 360;
    let escaped_label = label
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 300 300">
  <defs>
    <linearGradient id="g" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0%" stop-color="hsl({},72%,42%)"/>
      <stop offset="100%" stop-color="hsl({},78%,22%)"/>
    </linearGradient>
  </defs>
  <rect width="300" height="300" fill="url(#g)" rx="16"/>
  <circle cx="150" cy="150" r="88" fill="rgba(255,255,255,0.08)"/>
  <text x="150" y="168" font-family="system-ui,sans-serif" font-size="72" font-weight="700" fill="rgba(255,255,255,0.92)" text-anchor="middle">{}</text>
</svg>"#,
        hue1, hue2, escaped_label
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::SourceRegistry;
    use crate::store::Store;

    #[test]
    fn generate_svg_cover_never_overflows_and_escapes_xml() {
        for i in 0..500 {
            let svg = generate_svg_cover(&format!("track-{}", i), "A&B");
            assert!(svg.contains("<svg"));
            assert!(svg.contains("A&amp;B"));
        }
    }

    #[test]
    fn sniff_image_mime_detects_formats() {
        assert_eq!(
            sniff_image_mime(b"\x89PNG\r\n\x1a\n\x00"),
            "image/png"
        );
        assert_eq!(sniff_image_mime(b"RIFF1234WEBPabcd"), "image/webp");
        assert_eq!(sniff_image_mime(b"GIF89a..."), "image/gif");
        assert_eq!(sniff_image_mime(b"  <svg xmlns='...'>"), "image/svg+xml");
        assert_eq!(sniff_image_mime(b"\xff\xd8\xff\xe0"), "image/jpeg");
    }

    #[tokio::test]
    async fn serve_local_file_returns_416_on_unsatisfiable_range() {
        let dir = std::env::temp_dir().join(format!("beatlab_eng_test_{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&dir);
        let fpath = dir.join("tiny.mp3");
        std::fs::write(&fpath, b"1234567890").unwrap();

        let mut hdrs = HeaderMap::new();
        hdrs.insert(header::RANGE, "bytes=9999-".parse().unwrap());
        let resp = serve_local_file(&fpath, "mp3", &hdrs).await;
        assert_eq!(resp.status(), StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(resp.headers().get(header::CONTENT_RANGE).unwrap(), "bytes */10");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn resolve_url_ignores_missing_cached_local_file() {
        let dir = std::env::temp_dir().join(format!("beatlab_eng_test2_{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&dir);
        let store = Store::open_in_memory().unwrap();
        let reg = Arc::new(SourceRegistry::new(store.clone(), vec![]));
        let eng = Engine::new(store.clone(), reg, &dir, 1);
        let tr = Track {
            id: "tr1".into(),
            source: "soulseek".into(),
            stream_url: "slsk:user/file.flac".into(),
            ..Default::default()
        };
        let key = format!("ytres:{}", hash_str(&tr.stream_url));
        let missing = dir.join("does_not_exist.flac");
        let payload = serde_json::json!({
            "u": format!("file:{}", missing.display()),
            "a": chrono::Utc::now().timestamp()
        });
        store.kv_set(&key, &payload.to_string());
        assert!(eng.resolve_url(&tr, false).await.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn sniff_audio_mime_detects_m4a_under_opus_label() {
        use axum::body::to_bytes;

        assert_eq!(sniff_audio_bytes(b"fLaC\x00\x00\x00\"", "mp3"), "audio/flac");
        assert_eq!(sniff_audio_bytes(b"OggS\x00\x02", "m4a"), "audio/ogg");
        assert_eq!(sniff_audio_bytes(b"RIFF1234WAVEfmt ", "mp3"), "audio/wav");
        assert_eq!(sniff_audio_bytes(b"ID3\x03\x00", "flac"), "audio/mpeg");
        assert_eq!(sniff_audio_bytes(b"\x00\x00\x00\x00", "mp3"), "audio/mpeg");

        let dir = std::env::temp_dir().join(format!("beatlab_eng_test3_{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&dir);
        let fpath = dir.join("mislabeled.opus");
        let payload = b"\x00\x00\x00\x20ftypM4A \x00\x00\x00\x00M4A isom";
        std::fs::write(&fpath, payload).unwrap();

        let resp = serve_local_file(&fpath, "opus", &HeaderMap::new()).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers().get(header::CONTENT_TYPE).unwrap(), "audio/mp4");
        let body = to_bytes(resp.into_body(), 1024).await.unwrap();
        assert_eq!(&body[..], payload);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn serve_transcoded_returns_none_on_missing_input() {
        let dir = std::env::temp_dir().join(format!("beatlab_eng_test4_{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&dir);
        let store = Store::open_in_memory().unwrap();
        let reg = Arc::new(SourceRegistry::new(store.clone(), vec![]));
        let eng = Engine::new(store, reg, &dir, 1);
        let missing = dir.join("definitely_missing_input.flac");
        assert!(eng
            .serve_transcoded(&missing.to_string_lossy(), "mp3", 192)
            .await
            .is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
