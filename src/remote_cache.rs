//! Optional S3-compatible "cool" audio cache (Cloudflare R2, AWS S3, MinIO…).
//!
//! Local disk remains the hot cache. Successfully downloaded tracks are also
//! uploaded here; when the hot copy is evicted, the next play rehydrates it
//! from the bucket. Only `reqwest`, `hmac` and `sha2` are used — no AWS SDK.

use crate::models::{err_msg, AppResult};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio_util::io::ReaderStream;

type HmacSha256 = Hmac<Sha256>;

const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
const UNSIGNED_PAYLOAD: &str = "UNSIGNED-PAYLOAD";

fn env_nonempty(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[derive(Clone, Debug)]
pub struct RemoteObject {
    pub size: i64,
    pub ext: String,
    /// Audio extension is stored in `x-amz-meta-ext`; covers use these two for
    /// their MIME type and source URL.
    pub mime: String,
    pub src: String,
}

#[derive(Clone)]
pub struct RemoteCache {
    http: reqwest::Client,
    endpoint: String,
    host_header: String,
    bucket: String,
    region: String,
    access_key: String,
    secret_key: String,
    prefix: String,
    pub max_bytes: i64,
    /// Eviction starts here, leaving headroom so the tier is never caught full.
    pub high_water_bytes: i64,
    /// Eviction trims down to here (hysteresis) so uploads don't thrash eviction.
    pub low_water_bytes: i64,
    pruning: Arc<AtomicBool>,
}

struct ResetFlagGuard<'a>(&'a AtomicBool);
impl Drop for ResetFlagGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl RemoteCache {
    /// Returns `None` unless bucket + credentials are configured.
    pub fn from_env() -> AppResult<Option<Self>> {
        let (Some(bucket), Some(access_key), Some(secret_key)) = (
            env_nonempty("BEATLAB_S3_BUCKET"),
            env_nonempty("BEATLAB_S3_ACCESS_KEY_ID"),
            env_nonempty("BEATLAB_S3_SECRET_ACCESS_KEY"),
        ) else {
            return Ok(None);
        };
        let region = env_nonempty("BEATLAB_S3_REGION").unwrap_or_else(|| "auto".to_string());
        let endpoint = env_nonempty("BEATLAB_S3_ENDPOINT").unwrap_or_else(|| {
            if region == "auto" {
                "https://s3.amazonaws.com".to_string()
            } else {
                format!("https://s3.{region}.amazonaws.com")
            }
        });
        let endpoint = endpoint.trim_end_matches('/').to_string();
        let parsed = reqwest::Url::parse(&endpoint)
            .map_err(|e| err_msg(format!("Invalid BEATLAB_S3_ENDPOINT: {e}")))?;
        let mut host_header = parsed
            .host_str()
            .ok_or_else(|| err_msg("BEATLAB_S3_ENDPOINT has no host"))?
            .to_string();
        if let Some(port) = parsed.port() {
            host_header.push_str(&format!(":{port}"));
        }
        let prefix = env_nonempty("BEATLAB_S3_PREFIX").unwrap_or_else(|| "beatlab/audio/".to_string());
        let prefix = if prefix.ends_with('/') {
            prefix
        } else {
            format!("{prefix}/")
        };
        let max_gb: f64 = env_nonempty("BEATLAB_S3_MAX_GB")
            .and_then(|s| s.parse().ok())
            .unwrap_or(100.0);
        let max_bytes = (max_gb.max(1.0) * 1024.0 * 1024.0 * 1024.0) as i64;
        // Start evicting when the bucket is this full (default 90%), and trim
        // down to the low-water mark (default 80%) so there is always headroom
        // and no eviction thrashing on every upload.
        let high_water_pct: f64 = env_nonempty("BEATLAB_S3_HIGH_WATER_PCT")
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(90.0f64)
            .clamp(1.0f64, 100.0f64);
        let default_low_pct = (high_water_pct - 10.0).max(50.0).min(high_water_pct);
        let low_water_pct: f64 = env_nonempty("BEATLAB_S3_LOW_WATER_PCT")
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(default_low_pct)
            .clamp(1.0f64, high_water_pct);
        let high_water_bytes = (max_bytes as f64 * high_water_pct / 100.0) as i64;
        let low_water_bytes = (max_bytes as f64 * low_water_pct / 100.0) as i64;
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(15))
            .read_timeout(std::time::Duration::from_secs(60))
            .pool_max_idle_per_host(8)
            .build()
            .unwrap_or_default();
        Ok(Some(Self {
            http,
            endpoint,
            host_header,
            bucket,
            region,
            access_key,
            secret_key,
            prefix,
            max_bytes,
            high_water_bytes,
            low_water_bytes,
            pruning: Arc::new(AtomicBool::new(false)),
        }))
    }

    /// Legacy audio key layout, kept so existing uploaded objects still resolve.
    pub fn object_key(&self, track_id: &str) -> String {
        format!("{}{}", self.prefix, crate::engine::sha1_hex(track_id))
    }

    /// Namespaced key for non-audio cached assets (covers, embeds, avatars).
    pub fn asset_object_key(&self, kind: &str, asset_id: &str) -> String {
        if kind == "audio" {
            return self.object_key(asset_id);
        }
        format!(
            "{}{}/{}",
            self.prefix,
            kind,
            crate::engine::sha1_hex(asset_id)
        )
    }

    pub fn describe(&self) -> String {
        format!("{} / {} ({})", self.bucket, self.prefix, self.region)
    }

    // -- AWS Signature V4 ------------------------------------------------------

    fn sign(&self, key: &[u8], data: &[u8]) -> Vec<u8> {
        let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
        mac.update(data);
        mac.finalize().into_bytes().to_vec()
    }

    fn signing_key(&self, date: &str) -> Vec<u8> {
        let k = self.sign(format!("AWS4{}", self.secret_key).as_bytes(), date.as_bytes());
        let k = self.sign(&k, self.region.as_bytes());
        let k = self.sign(&k, b"s3");
        self.sign(&k, b"aws4_request")
    }

    fn signed_request(
        &self,
        method: &str,
        key: &str,
        payload_hash: &str,
        extra_headers: &[(&str, String)],
    ) -> AppResult<(String, Vec<(String, String)>)> {
        self.signed_request_at(method, key, payload_hash, extra_headers, chrono::Utc::now())
    }

    fn signed_request_at(
        &self,
        method: &str,
        key: &str,
        payload_hash: &str,
        extra_headers: &[(&str, String)],
        now: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<(String, Vec<(String, String)>)> {
        let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
        let date = now.format("%Y%m%d").to_string();
        let path = if self.bucket.is_empty() {
            format!("/{key}")
        } else {
            format!("/{}/{}", self.bucket, key)
        };
        let canonical_uri = canonical_uri(&path);
        let mut headers: Vec<(String, String)> = vec![
            ("host".to_string(), self.host_header.clone()),
            ("x-amz-content-sha256".to_string(), payload_hash.to_string()),
            ("x-amz-date".to_string(), amz_date.clone()),
        ];
        for (name, value) in extra_headers {
            headers.push((name.to_ascii_lowercase(), value.clone()));
        }
        headers.sort_by(|a, b| a.0.cmp(&b.0));
        headers.dedup_by(|a, b| a.0 == b.0);
        let canonical_headers = headers
            .iter()
            .map(|(k, v)| format!("{}:{}\n", k, v.trim()))
            .collect::<String>();
        let signed_headers = headers
            .iter()
            .map(|(k, _)| k.as_str())
            .collect::<Vec<_>>()
            .join(";");
        let canonical_request = format!(
            "{method}\n{canonical_uri}\n\n{canonical_headers}\n{signed_headers}\n{payload_hash}"
        );
        let scope = format!("{date}/{}/s3/aws4_request", self.region);
        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
            hex::encode(Sha256::digest(canonical_request.as_bytes()))
        );
        let signature = hex::encode(self.sign(&self.signing_key(&date), string_to_sign.as_bytes()));
        let auth = format!(
            "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={signature}",
            self.access_key
        );
        headers.push(("authorization".to_string(), auth));
        Ok((format!("{}{}", self.endpoint, canonical_uri), headers))
    }

    async fn send(
        &self,
        method: reqwest::Method,
        key: &str,
        payload_hash: &str,
        extra_headers: Vec<(&str, String)>,
        body: reqwest::Body,
    ) -> AppResult<reqwest::Response> {
        let (url, headers) = self.signed_request(method.as_str(), key, payload_hash, &extra_headers)?;
        let mut req = self.http.request(method, &url);
        for (name, value) in headers {
            req = req.header(&name, &value);
        }
        req.body(body).send().await.map_err(|e| err_msg(format!("S3 request failed: {e}")))
    }

    // -- Operations ------------------------------------------------------------

    pub async fn head(&self, key: &str) -> AppResult<Option<RemoteObject>> {
        let resp = self
            .send(
                reqwest::Method::HEAD,
                key,
                EMPTY_SHA256,
                vec![],
                reqwest::Body::from(Vec::new()),
            )
            .await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !resp.status().is_success() {
            return Err(err_msg(format!("S3 HEAD returned {}", resp.status())));
        }
        let size = resp
            .headers()
            .get(reqwest::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let meta = |name: &str| {
            resp.headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .trim()
                .to_string()
        };
        let ext = {
            let raw = meta("x-amz-meta-ext");
            if raw.is_empty() {
                "m4a".to_string()
            } else {
                raw.chars().take(12).collect()
            }
        };
        let mime = meta("x-amz-meta-mime").chars().take(64).collect();
        let src = meta("x-amz-meta-src").chars().take(1024).collect();
        Ok(Some(RemoteObject {
            size,
            ext,
            mime,
            src,
        }))
    }

    /// Uploads a hot-cache file with its audio extension kept as object metadata.
    pub async fn upload_from(&self, key: &str, path: &Path, ext: &str) -> AppResult<()> {
        let file = tokio::fs::File::open(path).await?;
        let len = file.metadata().await?.len();
        let body = reqwest::Body::wrap_stream(ReaderStream::new(file));
        let extra = vec![
            ("content-length", len.to_string()),
            ("x-amz-meta-ext", ext.chars().take(12).collect()),
        ];
        let resp = self
            .send(
                reqwest::Method::PUT,
                key,
                UNSIGNED_PAYLOAD,
                extra,
                body,
            )
            .await?;
        if !resp.status().is_success() {
            return Err(err_msg(format!("S3 PUT returned {}", resp.status())));
        }
        Ok(())
    }

    /// Uploads an arbitrary cached asset (cover, avatar, …) with its metadata.
    pub async fn upload_bytes(
        &self,
        key: &str,
        bytes: Vec<u8>,
        ext: &str,
        mime: &str,
        src: &str,
    ) -> AppResult<()> {
        let extra = vec![
            ("content-length", bytes.len().to_string()),
            ("x-amz-meta-ext", ext.chars().take(12).collect()),
            ("x-amz-meta-mime", mime.chars().take(64).collect()),
            ("x-amz-meta-src", src.chars().take(1024).collect()),
        ];
        let resp = self
            .send(
                reqwest::Method::PUT,
                key,
                UNSIGNED_PAYLOAD,
                extra,
                reqwest::Body::from(bytes),
            )
            .await?;
        if !resp.status().is_success() {
            return Err(err_msg(format!("S3 PUT returned {}", resp.status())));
        }
        Ok(())
    }

    /// Builds a signed GET URL and headers so a cool object can be streamed
    /// with HTTP Range requests instead of downloaded in full before playback.
    pub fn signed_get(&self, key: &str) -> AppResult<(String, Vec<(String, String)>)> {
        self.signed_request("GET", key, EMPTY_SHA256, &[])
    }

    /// Downloads an arbitrary cached asset into memory.
    pub async fn download_bytes(&self, key: &str) -> AppResult<Vec<u8>> {
        let resp = self
            .send(
                reqwest::Method::GET,
                key,
                EMPTY_SHA256,
                vec![],
                reqwest::Body::from(Vec::new()),
            )
            .await?;
        if !resp.status().is_success() {
            return Err(err_msg(format!("S3 GET returned {}", resp.status())));
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| err_msg(format!("S3 download: {e}")))?;
        Ok(bytes.to_vec())
    }

    /// Evicts lowest-scoring assets (recency + frequency + kind + size) once the
    /// bucket reaches its high-water mark, trimming down to the low-water mark
    /// so the cool tier keeps headroom without thrashing on every upload.
    pub async fn enforce_quota(&self, store: &crate::store::Store) -> AppResult<usize> {
        let mut total = store.remote_cache_total_bytes()?;
        if total <= self.high_water_bytes {
            return Ok(0);
        }
        if self
            .pruning
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Ok(0);
        }
        let _guard = ResetFlagGuard(&self.pruning);
        let target = self.low_water_bytes.min(self.high_water_bytes).max(0);
        let mut removed = 0usize;
        const BATCH_SIZE: usize = 500;
        const MAX_BATCHES: usize = 4;
        for _ in 0..MAX_BATCHES {
            if total <= target {
                break;
            }
            let candidates = store.remote_cache_eviction_candidates(BATCH_SIZE)?;
            if candidates.is_empty() {
                break;
            }
            for (kind, asset_id, key, bytes) in candidates {
                if total <= target {
                    break;
                }
                self.delete(&key).await?;
                let _ = store.delete_asset_cache_entry(&kind, &asset_id);
                total -= bytes;
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// Triggers an asynchronous, coalesced quota enforcement pass if the bucket
    /// has crossed its high-water mark. Safe to call after every upload.
    pub fn maybe_enforce_quota_async(self: &Arc<Self>, store: crate::store::Store) {
        if self.pruning.load(Ordering::Acquire) {
            return;
        }
        if store.remote_cache_total_bytes().unwrap_or(0) <= self.high_water_bytes {
            return;
        }
        let remote = Arc::clone(self);
        tokio::spawn(async move {
            match remote.enforce_quota(&store).await {
                Ok(n) if n > 0 => {
                    tracing::info!(removed = n, "pruned S3 cool cache to low-water mark");
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(error = %e.to_string(), "S3 cool cache prune failed");
                }
            }
        });
    }

    pub async fn delete(&self, key: &str) -> AppResult<()> {
        let resp = self
            .send(
                reqwest::Method::DELETE,
                key,
                EMPTY_SHA256,
                vec![],
                reqwest::Body::from(Vec::new()),
            )
            .await?;
        if !resp.status().is_success() && resp.status() != reqwest::StatusCode::NOT_FOUND {
            return Err(err_msg(format!("S3 DELETE returned {}", resp.status())));
        }
        Ok(())
    }
}

fn encode_component(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for &byte in input.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn canonical_uri(path: &str) -> String {
    path.split('/').map(encode_component).collect::<Vec<_>>().join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_encoding_preserves_slashes_and_encodes_special_characters() {
        assert_eq!(canonical_uri("/bucket/beatlab/a b+c.flac"), "/bucket/beatlab/a%20b%2Bc.flac");
        assert_eq!(encode_component("a/b?c"), "a%2Fb%3Fc");
    }

    #[test]
    fn signatures_are_stable_and_payload_sensitive() {
        let c = RemoteCache {
            http: reqwest::Client::new(),
            endpoint: "https://example.r2.cloudflarestorage.com".into(),
            host_header: "example.r2.cloudflarestorage.com".into(),
            bucket: "music".into(),
            region: "auto".into(),
            access_key: "AKIDEXAMPLE".into(),
            secret_key: "secret".into(),
            prefix: "beatlab/audio/".into(),
            max_bytes: 1024,
            high_water_bytes: 900,
            low_water_bytes: 800,
            pruning: Arc::new(AtomicBool::new(false)),
        };
        let (_, first) = c
            .signed_request("GET", "beatlab/audio/abc", EMPTY_SHA256, &[])
            .unwrap();
        let (_, second) = c
            .signed_request("GET", "beatlab/audio/abc", EMPTY_SHA256, &[])
            .unwrap();
        // Same second -> identical signature.
        assert_eq!(first, second);
        let (_, other) = c
            .signed_request("GET", "beatlab/audio/xyz", EMPTY_SHA256, &[])
            .unwrap();
        assert_ne!(first, other);
        assert!(first.iter().any(|(k, _)| k == "authorization"));
    }

    #[test]
    fn signature_matches_reference_sigv4_calculation() {
        // Fixed GET /test.txt request, cross-checked with an independent
        // HMAC-SHA256 implementation of the SigV4 canonical request.
        let c = RemoteCache {
            http: reqwest::Client::new(),
            endpoint: "https://examplebucket.s3.amazonaws.com".into(),
            host_header: "examplebucket.s3.amazonaws.com".into(),
            bucket: "".into(),
            region: "us-east-1".into(),
            access_key: "AKIDEXAMPLE".into(),
            secret_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".into(),
            prefix: "".into(),
            max_bytes: 1,
            high_water_bytes: 1,
            low_water_bytes: 1,
            pruning: Arc::new(AtomicBool::new(false)),
        };
        let now = chrono::DateTime::parse_from_rfc3339("2013-05-24T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let (_, headers) = c
            .signed_request_at("GET", "test.txt", EMPTY_SHA256, &[], now)
            .unwrap();
        let auth = headers
            .iter()
            .find(|(k, _)| k == "authorization")
            .map(|(_, v)| v.as_str())
            .unwrap();
        assert_eq!(
            auth,
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20130524/us-east-1/s3/aws4_request, SignedHeaders=host;x-amz-content-sha256;x-amz-date, Signature=14f6a0997b2b70a86f4726658a6575b5109092ccb5fd328f51b369c44b4ac958"
        );
    }

    #[test]
    fn object_keys_are_sha1_based_and_prefixed() {
        let c = RemoteCache {
            http: reqwest::Client::new(),
            endpoint: "https://example.com".into(),
            host_header: "example.com".into(),
            bucket: "b".into(),
            region: "auto".into(),
            access_key: "a".into(),
            secret_key: "s".into(),
            prefix: "beatlab/audio/".into(),
            max_bytes: 1,
            high_water_bytes: 1,
            low_water_bytes: 1,
            pruning: Arc::new(AtomicBool::new(false)),
        };
        let key = c.object_key("track:1");
        assert!(key.starts_with("beatlab/audio/"));
        assert_eq!(key.len(), "beatlab/audio/".len() + 40);
    }
}
