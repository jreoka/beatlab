use crate::models::Track;
use crate::sources::SourceRegistry;
use crate::sources::local::extract_embedded_cover;
use crate::store::Store;
use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use futures::StreamExt;
use sha1::{Digest, Sha1};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
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
            .timeout(Duration::from_secs(8))
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

    pub async fn resolve_url(&self, track: &Track, fresh: bool) -> Option<String> {
        if !track.stream_url.starts_with("ytdlp:") {
            return Some(track.stream_url.clone());
        }
        let key = format!("ytres:{}", hash_str(&track.stream_url));
        if !fresh {
            if let Some(raw) = self.store.kv_get(&key) {
                if let Ok(ent) = serde_json::from_str::<serde_json::Value>(&raw) {
                    let u = ent["u"].as_str().unwrap_or("");
                    let at = ent["a"].as_i64().unwrap_or(0);
                    let age = chrono::Utc::now().timestamp() - at;
                    if !u.is_empty() && age < 5 * 3600 {
                        return Some(u.to_string());
                    }
                }
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
            for t in refs {
                let _ = eng.resolve_url(&t, false).await;
            }
        });
    }

    /// Serves a track with bit-perfect quality by default (full HTTP Range support),
    /// or transcodes via ffmpeg only if `target_format` / `max_bitrate` is explicitly requested.
    pub async fn serve_track(
        self: &Arc<Self>,
        track: &Track,
        req_headers: &HeaderMap,
        target_format: Option<&str>,
        max_bitrate: Option<i32>,
    ) -> Response {
        let wants_transcode = self.has_ffmpeg
            && ((max_bitrate.unwrap_or(0) > 0 && track.bitrate > max_bitrate.unwrap_or(0))
                || target_format
                    .map(|f| !f.is_empty() && f != "raw" && !f.eq_ignore_ascii_case(&track.format))
                    .unwrap_or(false));

        // 1. Local file source
        if let Some(local_path) = track.stream_url.strip_prefix("file:") {
            let path = PathBuf::from(local_path);
            if path.exists() {
                if wants_transcode {
                    return self
                        .serve_transcoded(&path, target_format.unwrap_or("mp3"), max_bitrate.unwrap_or(320))
                        .await;
                }
                return serve_local_file(&path, &track.format, req_headers).await;
            }
            return (StatusCode::NOT_FOUND, "local file missing").into_response();
        }

        // 2. Cached file on disk
        if let Some(ent) = self.store.cache_get(&track.id) {
            let p = PathBuf::from(&ent.path);
            if p.exists() {
                self.store.cache_touch(&track.id);
                if wants_transcode {
                    return self
                        .serve_transcoded(&p, target_format.unwrap_or("mp3"), max_bitrate.unwrap_or(320))
                        .await;
                }
                return serve_local_file(&p, &track.format, req_headers).await;
            }
            self.store.cache_delete(&track.id);
        }

        // 3. Stream from upstream URL + tee to disk cache
        let Some(mut url) = self.resolve_url(track, false).await else {
            return (StatusCode::BAD_GATEWAY, "stream unavailable").into_response();
        };

        let range_hdr = req_headers
            .get(header::RANGE)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let mut resp = match self.fetch_upstream(&url, range_hdr.as_deref()).await {
            Ok(r) => r,
            Err(_) => return (StatusCode::BAD_GATEWAY, "upstream error").into_response(),
        };

        if resp.status() == reqwest::StatusCode::FORBIDDEN && track.stream_url.starts_with("ytdlp:")
        {
            if let Some(fresh_url) = self.resolve_url(track, true).await {
                url = fresh_url;
                if let Ok(r2) = self.fetch_upstream(&url, range_hdr.as_deref()).await {
                    resp = r2;
                }
            }
        }

        if !resp.status().is_success() && resp.status() != reqwest::StatusCode::PARTIAL_CONTENT {
            return (StatusCode::BAD_GATEWAY, "upstream rejected stream").into_response();
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
        let ct = content_type_for_format(&track.format);
        out_headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static(ct),
        );

        let cacheable = range_hdr.is_none()
            && status == StatusCode::OK
            && content_len.unwrap_or(0) > 0;

        if !cacheable {
            let stream = resp.bytes_stream();
            let body = Body::from_stream(stream);
            return (status, out_headers, body).into_response();
        }

        // Tee stream to disk cache
        let expected_len = content_len.unwrap() as i64;
        let tmp_name = format!("{}.{}.part", hash_str(&track.id), uuid::Uuid::new_v4());
        let tmp_path = self.cache_dir.join(tmp_name);
        let ext = if track.format.is_empty() { "bin" } else { &track.format };
        let final_path = self.cache_dir.join(format!("{}.{}", hash_str(&track.id), ext));

        let Ok(mut file) = tokio::fs::File::create(&tmp_path).await else {
            let body = Body::from_stream(resp.bytes_stream());
            return (status, out_headers, body).into_response();
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
                        let _ = tx.send(Err(std::io::Error::new(std::io::ErrorKind::Other, e))).await;
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
        (status, out_headers, body).into_response()
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

    async fn serve_transcoded(&self, path: &Path, fmt: &str, bitrate_kbps: i32) -> Response {
        let br = format!("{}k", bitrate_kbps.clamp(64, 320));
        let (codec, out_fmt, ct) = match fmt.to_lowercase().as_str() {
            "opus" => ("libopus", "opus", "audio/ogg"),
            "ogg" => ("libvorbis", "ogg", "audio/ogg"),
            "aac" | "m4a" => ("aac", "adts", "audio/aac"),
            _ => ("libmp3lame", "mp3", "audio/mpeg"),
        };

        let mut child = match Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-i",
                &path.to_string_lossy(),
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
        {
            Ok(c) => c,
            Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "ffmpeg failed").into_response(),
        };

        let Some(stdout) = child.stdout.take() else {
            return (StatusCode::INTERNAL_SERVER_ERROR, "ffmpeg stdout missing").into_response();
        };
        tokio::spawn(async move {
            let _ = child.wait().await;
        });

        let stream = ReaderStream::new(stdout);
        (
            StatusCode::OK,
            [(header::CONTENT_TYPE, HeaderValue::from_static(ct))],
            Body::from_stream(stream),
        )
            .into_response()
    }

    pub fn evict(&self) {
        while self.store.cache_bytes() > self.budget_bytes {
            let entries = self.store.cache_entries_oldest();
            if entries.is_empty() {
                break;
            }
            let mut progressed = false;
            for ent in entries {
                if self.store.cache_bytes() <= self.budget_bytes {
                    break;
                }
                if std::fs::remove_file(&ent.path).is_ok() || !Path::new(&ent.path).exists() {
                    if self.store.cache_delete(&ent.track_id) {
                        progressed = true;
                    }
                }
            }
            if !progressed {
                break;
            }
        }
    }

    pub async fn serve_cover(&self, id: &str) -> Response {
        let track = self.store.track(id).or_else(|| {
            // Maybe id is an artist or album id; find any matching track with cover
            if let Some(name) = self.store.artist_name_from_id(id) {
                self.store.tracks_by_artist(&name, 1).into_iter().next()
            } else {
                None
            }
        });

        if let Some(ref t) = track {
            if let Some(local_path) = t.cover_url.strip_prefix("local-art:") {
                if let Some((bytes, mime)) = extract_embedded_cover(Path::new(local_path)) {
                    return (
                        StatusCode::OK,
                        [
                            (header::CONTENT_TYPE, mime),
                            (header::CACHE_CONTROL, "public, max-age=86400".to_string()),
                        ],
                        bytes,
                    )
                        .into_response();
                }
            } else if t.cover_url.starts_with("http") {
                let cache_path = self.cover_dir.join(format!("{}.img", hash_str(&t.cover_url)));
                if let Ok(bytes) = tokio::fs::read(&cache_path).await {
                    return (
                        StatusCode::OK,
                        [
                            (header::CONTENT_TYPE, "image/jpeg".to_string()),
                            (header::CACHE_CONTROL, "public, max-age=86400".to_string()),
                        ],
                        bytes,
                    )
                        .into_response();
                }
                if let Ok(resp) = self.cover_http.get(&t.cover_url).send().await {
                    if resp.status().is_success() {
                        let ct = resp
                            .headers()
                            .get(reqwest::header::CONTENT_TYPE)
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or("image/jpeg")
                            .to_string();
                        if let Ok(bytes) = resp.bytes().await {
                            let _ = tokio::fs::write(&cache_path, &bytes).await;
                            return (
                                StatusCode::OK,
                                [
                                    (header::CONTENT_TYPE, ct),
                                    (header::CACHE_CONTROL, "public, max-age=86400".to_string()),
                                ],
                                bytes,
                            )
                                .into_response();
                        }
                    }
                }
            }
        }

        // Crisp deterministic SVG fallback cover
        let label = track
            .as_ref()
            .map(|t| t.title.chars().take(2).collect::<String>().to_uppercase())
            .unwrap_or_else(|| "BL".to_string());
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
}

async fn serve_local_file(path: &Path, format: &str, req_headers: &HeaderMap) -> Response {
    let Ok(mut file) = tokio::fs::File::open(path).await else {
        return (StatusCode::NOT_FOUND, "file not found").into_response();
    };
    let Ok(meta) = file.metadata().await else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "stat failed").into_response();
    };
    let total = meta.len();
    let ct = content_type_for_format(format);

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
        _ => "application/octet-stream",
    }
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
    let hue1 = (b0 as u16 * 360 / 255) % 360;
    let hue2 = (hue1 + 45 + (b1 as u16 % 60)) % 360;
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
        hue1, hue2, label
    )
}
