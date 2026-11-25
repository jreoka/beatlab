//! Optional OpenAI-compatible overnight LLM curator.
//!
//! Configured exclusively by the server owner via environment variables:
//! - `BEATLAB_LLM_MODEL` (required to enable)
//! - `BEATLAB_LLM_ENDPOINT` (optional, defaults to `https://api.openai.com/v1`)
//! - `BEATLAB_LLM_API_KEY` (required for default OpenAI endpoint, optional for local custom endpoints)
//! - `BEATLAB_LLM_REFRESH_HOUR` (optional local hour 0..23, default `4`)
//! - `BEATLAB_LLM_TEMPERATURE` (optional float 0.0..2.0, default `0.85`)
//! - `BEATLAB_LLM_MAX_TOKENS` (optional integer 512..16384, default `2600`)
//! - `BEATLAB_LLM_TIMEOUT_SECS` (optional integer 10..300, default `90`)
//! - `BEATLAB_LLM_SYSTEM_PROMPT` (optional extra editorial instructions from server owner)
//! - `BEATLAB_LLM_EXTRA_HEADERS` (optional JSON object of extra HTTP headers)
//! - `BEATLAB_LLM_REASONING_EFFORT` (optional `"low" | "medium" | "high"`)
//!
//! Once per listener-day (turning over at `refresh_hour` in each user's local timezone),
//! Beatlab summarizes the user's listening stats, top artists/tracks, genres, moods, and
//! playlists, asks the LLM to craft a fresh daily edition (custom Home & Explore copy,
//! custom-named Daily Mixes, custom Explore categories & mixes, personalized Mood
//! ordering/subtitles, and Search vibe prompts), and then uses `TasteEngine` + music
//! sources to materialize every mix into real playable tracks stored in SQLite.

use crate::models::{err_msg, primary_artist, slug, AppResult, Mood};
use crate::stats::{build_user_stats, StatsRange};
use crate::store::Store;
use crate::taste::TasteEngine;
use chrono::{Datelike, FixedOffset, TimeZone, Utc};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tracing::{info, warn};

const DEFAULT_OPENAI_ENDPOINT: &str = "https://api.openai.com/v1";
const EXPERIENCE_SCHEMA_VERSION: u32 = 1;
const RETRY_BACKOFF_SECS: i64 = 30 * 60;

const CURATED_PALETTES: &[(&str, &str)] = &[
    ("#38bdf8", "#0c2d48"),
    ("#f472b6", "#4a1230"),
    ("#a78bfa", "#26174a"),
    ("#fb923c", "#4c1d0d"),
    ("#34d399", "#0b3b2c"),
    ("#fbbf24", "#452908"),
    ("#f87171", "#4c1118"),
    ("#22d3ee", "#0b3640"),
    ("#c084fc", "#35144f"),
    ("#4ade80", "#113a1f"),
];

fn env_nonempty(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Normalize a user-supplied OpenAI-compatible base URL or full completions URL
/// into the exact POST target for `/chat/completions`.
pub fn normalize_completions_url(raw: &str) -> AppResult<String> {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err(err_msg("BEATLAB_LLM_ENDPOINT must not be empty"));
    }
    let parsed = reqwest::Url::parse(trimmed)
        .map_err(|e| err_msg(format!("Invalid BEATLAB_LLM_ENDPOINT '{trimmed}': {e}")))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(err_msg("BEATLAB_LLM_ENDPOINT must use http or https"));
    }
    if trimmed.ends_with("/chat/completions") {
        Ok(trimmed.to_string())
    } else {
        Ok(format!("{trimmed}/chat/completions"))
    }
}

#[derive(Clone, Debug)]
pub struct LlmConfig {
    pub endpoint_base: String,
    pub completions_url: String,
    pub api_key: String,
    pub model: String,
    pub refresh_hour: i64,
    pub temperature: f64,
    pub max_tokens: u32,
    pub timeout_secs: u64,
    pub extra_system_prompt: Option<String>,
    pub extra_headers: Vec<(String, String)>,
    pub reasoning_effort: Option<String>,
}

impl LlmConfig {
    /// Load configuration from environment variables.
    /// Returns `Ok(None)` when no model is configured (feature stays cleanly off).
    pub fn from_env() -> AppResult<Option<Self>> {
        let Some(model) = env_nonempty("BEATLAB_LLM_MODEL") else {
            return Ok(None);
        };
        let custom_endpoint = env_nonempty("BEATLAB_LLM_ENDPOINT");
        let api_key = env_nonempty("BEATLAB_LLM_API_KEY").unwrap_or_default();

        if custom_endpoint.is_none() && api_key.is_empty() {
            return Err(err_msg(
                "BEATLAB_LLM_API_KEY is required when using the default OpenAI endpoint (or set BEATLAB_LLM_ENDPOINT for a local/custom server)",
            ));
        }

        let endpoint_base = custom_endpoint
            .unwrap_or_else(|| DEFAULT_OPENAI_ENDPOINT.to_string())
            .trim_end_matches('/')
            .to_string();
        let completions_url = normalize_completions_url(&endpoint_base)?;

        let refresh_hour = env_nonempty("BEATLAB_LLM_REFRESH_HOUR")
            .and_then(|s| s.parse::<i64>().ok())
            .map(|h| h.clamp(0, 23))
            .unwrap_or(4);

        let temperature = env_nonempty("BEATLAB_LLM_TEMPERATURE")
            .and_then(|s| s.parse::<f64>().ok())
            .filter(|v| v.is_finite())
            .map(|v| v.clamp(0.0, 2.0))
            .unwrap_or(0.85);

        let max_tokens = env_nonempty("BEATLAB_LLM_MAX_TOKENS")
            .and_then(|s| s.parse::<u32>().ok())
            .map(|v| v.clamp(512, 16384))
            .unwrap_or(2600);

        let timeout_secs = env_nonempty("BEATLAB_LLM_TIMEOUT_SECS")
            .and_then(|s| s.parse::<u64>().ok())
            .map(|v| v.clamp(10, 300))
            .unwrap_or(90);

        let extra_system_prompt = env_nonempty("BEATLAB_LLM_SYSTEM_PROMPT");

        let reasoning_effort = env_nonempty("BEATLAB_LLM_REASONING_EFFORT").map(|s| {
            let low = s.to_ascii_lowercase();
            match low.as_str() {
                "low" | "medium" | "high" => low,
                _ => s,
            }
        });

        let mut extra_headers = Vec::new();
        if let Some(raw_headers) = env_nonempty("BEATLAB_LLM_EXTRA_HEADERS") {
            let val: Value = serde_json::from_str(&raw_headers)
                .map_err(|e| err_msg(format!("Invalid BEATLAB_LLM_EXTRA_HEADERS JSON: {e}")))?;
            let Some(obj) = val.as_object() else {
                return Err(err_msg("BEATLAB_LLM_EXTRA_HEADERS must be a JSON object"));
            };
            for (k, v) in obj {
                if let Some(vs) = v.as_str() {
                    let kk = k.trim();
                    let vv = vs.trim();
                    if !kk.is_empty() && !vv.is_empty() {
                        HeaderName::from_bytes(kk.as_bytes()).map_err(|e| {
                            err_msg(format!("Invalid header name '{kk}' in BEATLAB_LLM_EXTRA_HEADERS: {e}"))
                        })?;
                        HeaderValue::from_str(vv).map_err(|e| {
                            err_msg(format!("Invalid header value for '{kk}' in BEATLAB_LLM_EXTRA_HEADERS: {e}"))
                        })?;
                        extra_headers.push((kk.to_string(), vv.to_string()));
                    }
                }
            }
        }

        Ok(Some(Self {
            endpoint_base,
            completions_url,
            api_key,
            model,
            refresh_hour,
            temperature,
            max_tokens,
            timeout_secs,
            extra_system_prompt,
            extra_headers,
            reasoning_effort,
        }))
    }

    /// Safe human-readable endpoint label (scheme + host + path, never credentials).
    pub fn safe_endpoint_label(&self) -> String {
        if let Ok(mut u) = reqwest::Url::parse(&self.endpoint_base) {
            let _ = u.set_username("");
            let _ = u.set_password(None);
            u.set_query(None);
            u.set_fragment(None);
            u.to_string().trim_end_matches('/').to_string()
        } else {
            "(configured)".to_string()
        }
    }
}

/// The listener's local "edition day" for overnight LLM customization.
/// Days turn over at `refresh_hour` (e.g. 04:00) local time.
pub fn llm_day(ts: i64, tz_offset_min: i32, refresh_hour: i64) -> i64 {
    let clamped_tz = i64::from(tz_offset_min.clamp(-720, 840));
    let clamped_hr = refresh_hour.clamp(0, 23);
    (ts + clamped_tz * 60 - clamped_hr * 3600).div_euclid(86_400)
}

pub fn experience_kv_key(user_id: i64) -> String {
    format!("llm:experience:v1:{user_id}")
}

fn attempt_kv_key(user_id: i64) -> String {
    format!("llm:attempt_at:v1:{user_id}")
}

pub fn llm_explore_pool_key(user_id: i64, mix_key: &str) -> String {
    format!("curated:llm_explore:v1:{user_id}:{mix_key}")
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LlmSectionTitles {
    #[serde(default)]
    pub quick_picks: String,
    #[serde(default)]
    pub mixed_for_you: String,
    #[serde(default)]
    pub made_for_you: String,
    #[serde(default)]
    pub recent: String,
    #[serde(default)]
    pub related_tracks: String,
    #[serde(default)]
    pub recommended_artists: String,
    #[serde(default)]
    pub moods: String,
    #[serde(default)]
    pub decades: String,
    #[serde(default)]
    pub discovery: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmDailyMixSpec {
    /// `"1"`, `"2"`, `"3"`, or `"supermix"`.
    pub part: String,
    pub title: String,
    pub art_title: String,
    pub subtitle: String,
    pub c1: String,
    pub c2: String,
    #[serde(default)]
    pub seed_queries: Vec<String>,
    #[serde(default)]
    pub seed_artists: Vec<String>,
    #[serde(default)]
    pub genres: Vec<String>,
    #[serde(default)]
    pub mood: Option<String>,
    #[serde(default)]
    pub year_range: Option<(i32, i32)>,
    /// `"familiar" | "balanced" | "discovery"`
    #[serde(default)]
    pub familiarity: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LlmAutoMixCopy {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub comment: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmExploreMixSpec {
    /// Always prefixed with `llm-` so it never collides with static decade keys.
    pub key: String,
    pub kicker: String,
    pub title: String,
    pub subtitle: String,
    pub query: String,
    #[serde(default)]
    pub seed_artists: Vec<String>,
    #[serde(default)]
    pub genres: Vec<String>,
    #[serde(default)]
    pub mood: Option<String>,
    #[serde(default)]
    pub year_range: Option<(i32, i32)>,
    pub c1: String,
    pub c2: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmExploreCategory {
    pub slug: String,
    pub kicker: String,
    pub title: String,
    pub subtitle: String,
    pub mixes: Vec<LlmExploreMixSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmMoodOverride {
    pub slug: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub subtitle: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmSearchPrompt {
    pub label: String,
    pub query: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyLlmExperience {
    pub version: u32,
    pub generated_at: i64,
    pub day_key: i64,
    pub date_label: String,
    pub edition_title: String,
    pub model: String,
    pub home_tagline: String,
    pub explore_tagline: String,
    #[serde(default)]
    pub sections: LlmSectionTitles,
    #[serde(default)]
    pub daily_mixes: Vec<LlmDailyMixSpec>,
    #[serde(default)]
    pub auto_mix_overrides: HashMap<String, LlmAutoMixCopy>,
    #[serde(default)]
    pub explore_categories: Vec<LlmExploreCategory>,
    #[serde(default)]
    pub mood_overrides: Vec<LlmMoodOverride>,
    #[serde(default)]
    pub search_prompts: Vec<LlmSearchPrompt>,
}

impl DailyLlmExperience {
    pub fn find_explore_mix(&self, key: &str) -> Option<&LlmExploreMixSpec> {
        for cat in &self.explore_categories {
            if let Some(m) = cat.mixes.iter().find(|m| m.key == key) {
                return Some(m);
            }
        }
        None
    }

    pub fn find_daily_mix(&self, part: &str) -> Option<&LlmDailyMixSpec> {
        self.daily_mixes.iter().find(|m| m.part == part)
    }
}

#[derive(Clone)]
pub struct LlmCurator {
    pub config: LlmConfig,
    http: reqwest::Client,
    in_flight: Arc<Mutex<HashSet<i64>>>,
    last_error: Arc<Mutex<Option<(i64, String)>>>,
}

impl LlmCurator {
    pub fn from_env() -> AppResult<Option<Self>> {
        let Some(config) = LlmConfig::from_env()? else {
            return Ok(None);
        };
        Ok(Some(Self::new_with_config(config)))
    }

    pub fn new_with_config(config: LlmConfig) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(config.timeout_secs))
            .pool_max_idle_per_host(4)
            .build()
            .unwrap_or_default();
        Self {
            config,
            http,
            in_flight: Arc::new(Mutex::new(HashSet::new())),
            last_error: Arc::new(Mutex::new(None)),
        }
    }

    pub fn load_experience(store: &Store, user_id: i64) -> Option<DailyLlmExperience> {
        if user_id <= 0 {
            return None;
        }
        store
            .kv_get(&experience_kv_key(user_id))
            .and_then(|raw| serde_json::from_str::<DailyLlmExperience>(&raw).ok())
    }

    pub fn save_experience(store: &Store, user_id: i64, exp: &DailyLlmExperience) {
        if user_id <= 0 {
            return;
        }
        if let Ok(raw) = serde_json::to_string(exp) {
            store.kv_set(&experience_kv_key(user_id), &raw);
        }
    }

    pub fn needs_refresh(&self, store: &Store, user_id: i64, tz_offset_min: i32) -> bool {
        if user_id <= 0 {
            return false;
        }
        let now = Utc::now().timestamp();
        let last_attempt = store
            .kv_get(&attempt_kv_key(user_id))
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(0);
        if now - last_attempt < RETRY_BACKOFF_SECS {
            return false;
        }
        let today = llm_day(now, tz_offset_min, self.config.refresh_hour);
        match Self::load_experience(store, user_id) {
            None => true,
            Some(exp) => exp.version < EXPERIENCE_SCHEMA_VERSION || today > exp.day_key,
        }
    }

    /// Spawn a non-blocking refresh for a single user if their daily edition is missing or stale.
    pub fn maybe_spawn_refresh(&self, taste: &TasteEngine, user_id: i64) {
        if user_id <= 0 {
            return;
        }
        let tz = taste.user_tz_offset_min(user_id);
        if !self.needs_refresh(&taste.store, user_id, tz) {
            return;
        }
        let curator = self.clone();
        let taste_cloned = taste.clone();
        tokio::spawn(async move {
            if let Err(e) = curator.refresh_user_experience(&taste_cloned, user_id, false).await {
                warn!(user_id, error = %e, "background LLM daily customization failed");
            }
        });
    }

    /// Run a full overnight/daily LLM customization for `user_id` and materialize all
    /// custom Daily Mix playlists and Explore category pools into SQLite.
    /// Returns `Ok(true)` if a new edition was generated, `Ok(false)` if skipped/up-to-date.
    pub async fn refresh_user_experience(
        &self,
        taste: &TasteEngine,
        user_id: i64,
        force: bool,
    ) -> AppResult<bool> {
        if user_id <= 0 {
            return Ok(false);
        }
        let tz = taste.user_tz_offset_min(user_id);
        if !force && !self.needs_refresh(&taste.store, user_id, tz) {
            return Ok(false);
        }

        // Prevent overlapping runs for the same user.
        {
            let mut guard = self.in_flight.lock().unwrap_or_else(|e| e.into_inner());
            if !guard.insert(user_id) {
                return Ok(false);
            }
        }
        struct InFlightGuard {
            set: Arc<Mutex<HashSet<i64>>>,
            uid: i64,
        }
        impl Drop for InFlightGuard {
            fn drop(&mut self) {
                // Tolerate a poisoned mutex (as the insert path does), otherwise a
                // panic here would leave this user locked out of refreshes forever.
                let mut g = self.set.lock().unwrap_or_else(|e| e.into_inner());
                g.remove(&self.uid);
            }
        }
        let _guard = InFlightGuard {
            set: self.in_flight.clone(),
            uid: user_id,
        };

        // A concurrent run may have finished between the freshness check above and
        // acquiring the in-flight slot; skip generating a redundant edition.
        if !force && !self.needs_refresh(&taste.store, user_id, tz) {
            return Ok(false);
        }

        let now = Utc::now().timestamp();
        taste.store.kv_set(&attempt_kv_key(user_id), &now.to_string());

        let result = self.generate_and_materialize(taste, user_id, tz, now).await;
        match result {
            Ok(exp) => {
                taste.store.kv_set("llm:last_success_at", &now.to_string());
                info!(
                    user_id,
                    model = %exp.model,
                    edition = %exp.edition_title,
                    categories = exp.explore_categories.len(),
                    daily_mixes = exp.daily_mixes.len(),
                    "daily LLM customization complete"
                );
                Ok(true)
            }
            Err(e) => {
                if let Ok(mut le) = self.last_error.lock() {
                    *le = Some((now, e.to_string()));
                }
                taste.store.kv_set("llm:last_error", &format!("{now}: {}", e));
                Err(e)
            }
        }
    }

    async fn generate_and_materialize(
        &self,
        taste: &TasteEngine,
        user_id: i64,
        tz_offset_min: i32,
        now: i64,
    ) -> AppResult<DailyLlmExperience> {
        let prev = Self::load_experience(&taste.store, user_id);
        let store_for_digest = taste.store.clone();
        let digest = tokio::task::spawn_blocking(move || {
            build_user_taste_digest(&store_for_digest, user_id, tz_offset_min, now, prev.as_ref())
        })
        .await
        .map_err(|e| err_msg(format!("taste digest join error: {e}")))??;

        let system_prompt = build_system_prompt(self.config.extra_system_prompt.as_deref());
        let user_prompt = build_user_prompt(&digest);

        let raw_content = self.call_chat_completions(&system_prompt, &user_prompt).await?;
        let json_str = extract_json_object(&raw_content).ok_or_else(|| {
            err_msg("LLM response did not contain a valid JSON object")
        })?;
        let parsed: RawLlmResponse = serde_json::from_str(json_str)
            .map_err(|e| err_msg(format!("Failed to parse LLM JSON payload: {e}")))?;

        let day_key = llm_day(now, tz_offset_min, self.config.refresh_hour);
        let experience = sanitize_experience(
            parsed,
            &digest.date_label,
            day_key,
            now,
            &self.config.model,
        );

        // Save the experience first so even if a source search times out partway,
        // custom sections & on-demand Explore mixes are already available.
        Self::save_experience(&taste.store, user_id, &experience);

        // Record when this edition's day ends in the listener's local timezone, so the
        // generic Mixed-for-you rebuild never overwrites the custom mixes mid-day —
        // even when BEATLAB_LLM_REFRESH_HOUR differs from the mix turnover hour.
        let fresh_until = (day_key + 1) * 86_400 + self.config.refresh_hour * 3600
            - i64::from(tz_offset_min) * 60;
        taste.store.kv_set(
            &format!("mix4u:llm_fresh_until:{user_id}"),
            &fresh_until.to_string(),
        );

        // Materialize all custom Daily Mixes, Made For You copy overrides, and Explore category mixes.
        taste.materialize_llm_experience(user_id, &experience).await;

        Ok(experience)
    }

    async fn call_chat_completions(&self, system_prompt: &str, user_prompt: &str) -> AppResult<String> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if !self.config.api_key.is_empty() {
            let auth = format!("Bearer {}", self.config.api_key);
            if let Ok(hv) = HeaderValue::from_str(&auth) {
                headers.insert(AUTHORIZATION, hv);
            }
        }
        for (k, v) in &self.config.extra_headers {
            if let (Ok(hn), Ok(hv)) = (
                HeaderName::from_bytes(k.as_bytes()),
                HeaderValue::from_str(v),
            ) {
                headers.insert(hn, hv);
            }
        }

        let mut body = json!({
            "model": self.config.model,
            "messages": [
                { "role": "system", "content": system_prompt },
                { "role": "user", "content": user_prompt },
            ],
            "temperature": self.config.temperature,
            "max_tokens": self.config.max_tokens,
            "response_format": { "type": "json_object" },
        });
        if let Some(ref effort) = self.config.reasoning_effort {
            if let Some(obj) = body.as_object_mut() {
                obj.insert("reasoning_effort".to_string(), json!(effort));
            }
        }

        let resp = self
            .http
            .post(&self.config.completions_url)
            .headers(headers.clone())
            .json(&body)
            .send()
            .await
            .map_err(|e| err_msg(format!("LLM request failed: {e}")))?;

        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| err_msg(format!("Failed reading LLM response: {e}")))?;

        if status.is_success() {
            return parse_completion_content(&text);
        }

        // Compatibility retry for OpenAI-compatible servers or reasoning models that reject
        // `response_format`, `temperature`, or `max_tokens` (preferring `max_completion_tokens`).
        if status.as_u16() == 400 || status.as_u16() == 422 {
            let low = text.to_ascii_lowercase();
            let mut retry_body = body.clone();
            let mut changed = false;
            if let Some(obj) = retry_body.as_object_mut() {
                if low.contains("response_format") || low.contains("json_object") {
                    obj.remove("response_format");
                    changed = true;
                }
                if low.contains("max_completion_tokens") || low.contains("max_tokens") {
                    if let Some(mt) = obj.remove("max_tokens") {
                        obj.insert("max_completion_tokens".to_string(), mt);
                        changed = true;
                    }
                }
                if low.contains("temperature") {
                    obj.remove("temperature");
                    changed = true;
                }
                if low.contains("reasoning_effort") {
                    obj.remove("reasoning_effort");
                    changed = true;
                }
                if !changed {
                    // Fallback: strip optional params that non-standard servers sometimes dislike.
                    obj.remove("response_format");
                    changed = true;
                }
            }
            if changed {
                let retry_resp = self
                    .http
                    .post(&self.config.completions_url)
                    .headers(headers)
                    .json(&retry_body)
                    .send()
                    .await
                    .map_err(|e| err_msg(format!("LLM retry request failed: {e}")))?;
                let retry_status = retry_resp.status();
                let retry_text = retry_resp
                    .text()
                    .await
                    .map_err(|e| err_msg(format!("Failed reading LLM retry response: {e}")))?;
                if retry_status.is_success() {
                    return parse_completion_content(&retry_text);
                }
                return Err(err_msg(format!(
                    "LLM API returned {}: {}",
                    retry_status.as_u16(),
                    truncate_str(&retry_text, 280)
                )));
            }
        }

        Err(err_msg(format!(
            "LLM API returned {}: {}",
            status.as_u16(),
            truncate_str(&text, 280)
        )))
    }

    /// Safe snapshot for the Admin panel (never leaks API key or extra header values).
    pub fn status_snapshot(&self, store: &Store, taste: &TasteEngine) -> Value {
        let now = Utc::now().timestamp();
        let uids = store.all_user_ids().unwrap_or_default();
        let mut customized_total = 0usize;
        let mut customized_today = 0usize;
        let mut latest_generated_at = 0i64;
        for uid in &uids {
            if let Some(exp) = Self::load_experience(store, *uid) {
                customized_total += 1;
                latest_generated_at = latest_generated_at.max(exp.generated_at);
                let tz = taste.user_tz_offset_min(*uid);
                if exp.day_key >= llm_day(now, tz, self.config.refresh_hour) {
                    customized_today += 1;
                }
            }
        }
        let in_flight_count = self.in_flight.lock().map(|g| g.len()).unwrap_or(0);
        let last_err = self
            .last_error
            .lock()
            .ok()
            .and_then(|g| g.clone())
            .map(|(at, msg)| json!({ "at": at, "message": msg }));

        json!({
            "enabled": true,
            "endpoint": self.config.safe_endpoint_label(),
            "model": self.config.model,
            "hasApiKey": !self.config.api_key.is_empty(),
            "refreshHour": self.config.refresh_hour,
            "temperature": self.config.temperature,
            "maxTokens": self.config.max_tokens,
            "timeoutSecs": self.config.timeout_secs,
            "customizedUsers": customized_total,
            "customizedToday": customized_today,
            "totalUsers": uids.len(),
            "inFlight": in_flight_count,
            "lastGeneratedAt": latest_generated_at,
            "lastError": last_err,
        })
    }
}

fn parse_completion_content(raw_body: &str) -> AppResult<String> {
    let val: Value = serde_json::from_str(raw_body)
        .map_err(|e| err_msg(format!("Invalid JSON from LLM endpoint: {e}")))?;
    let Some(msg) = val
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
        .and_then(|c| c.get("message"))
    else {
        return Err(err_msg("LLM response missing choices[0].message"));
    };

    if let Some(s) = msg.get("content").and_then(|v| v.as_str()) {
        if !s.trim().is_empty() {
            return Ok(s.to_string());
        }
    }
    // Some OpenAI-compatible gateways return content as an array of parts: [{type:"text", text:"..."}]
    if let Some(parts) = msg.get("content").and_then(|v| v.as_array()) {
        let mut combined = String::new();
        for p in parts {
            if let Some(t) = p.get("text").and_then(|v| v.as_str()) {
                combined.push_str(t);
            }
        }
        if !combined.trim().is_empty() {
            return Ok(combined);
        }
    }
    Err(err_msg("LLM message content was empty"))
}

/// Extract the first balanced JSON object from an LLM reply, stripping `<think>...`
/// reasoning blocks and markdown code fences if present.
pub fn extract_json_object(raw: &str) -> Option<&str> {
    let mut s = raw.trim();
    // Strip a reasoning block if a thinking model emits one before the payload,
    // but keep the raw text when the JSON came first and any thinking trailed it.
    if let Some(end_think) = s.rfind("</think>") {
        let after = s[end_think + "</think>".len()..].trim();
        if after.contains('{') {
            s = after;
        }
    }
    if let Some(rest) = s.strip_prefix("```") {
        // Strip opening fence line (e.g. ```json\n)
        if let Some(nl) = rest.find('\n') {
            s = rest[nl + 1..].trim();
        } else {
            s = rest.trim();
        }
        if let Some(stripped) = s.strip_suffix("```") {
            s = stripped.trim();
        }
    }
    // Return the first balanced JSON object, scanning with string/escape awareness
    // so trailing prose or stray braces cannot corrupt the extraction.
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'{' {
            i += 1;
            continue;
        }
        let mut depth = 0usize;
        let mut in_str = false;
        let mut escaped = false;
        let mut j = i;
        while j < bytes.len() {
            let b = bytes[j];
            if escaped {
                escaped = false;
            } else if in_str {
                match b {
                    b'\\' => escaped = true,
                    b'"' => in_str = false,
                    _ => {}
                }
            } else {
                match b {
                    b'"' => in_str = true,
                    b'{' => depth += 1,
                    b'}' => {
                        depth -= 1;
                        if depth == 0 {
                            return Some(&s[i..=j]);
                        }
                    }
                    _ => {}
                }
            }
            j += 1;
        }
        // Unbalanced from this opening brace; try the next candidate.
        i += 1;
    }
    None
}

struct UserTasteDigest {
    username: String,
    display_name: String,
    date_label: String,
    weekday: String,
    month_name: String,
    is_cold_start: bool,
    chronotype: String,
    peak_hour_label: String,
    discovery_rate_pct: f64,
    total_streams_30d: usize,
    top_artists_recent: Vec<String>,
    top_artists_all: Vec<String>,
    top_tracks_recent: Vec<String>,
    top_tracks_all: Vec<String>,
    top_genres: Vec<String>,
    top_moods: Vec<String>,
    top_decades: Vec<String>,
    manual_playlists: Vec<String>,
    skipped_artists: Vec<String>,
    chart_artists: Vec<String>,
    previous_titles: Vec<String>,
}

fn build_user_taste_digest(
    store: &Store,
    user_id: i64,
    tz_offset_min: i32,
    now_ts: i64,
    previous: Option<&DailyLlmExperience>,
) -> AppResult<UserTasteDigest> {
    let user = store
        .get_user_by_id(user_id)?
        .ok_or_else(|| err_msg(format!("User {user_id} not found")))?;
    let profile = store.get_public_user(user_id, user_id).ok().flatten();
    let display_name = profile
        .as_ref()
        .map(|p| p.display_name.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| user.username.clone());

    let clamped_tz = tz_offset_min.clamp(-720, 840);
    let offset = FixedOffset::east_opt(clamped_tz * 60)
        .unwrap_or_else(|| FixedOffset::east_opt(0).unwrap());
    let local_dt = offset
        .timestamp_opt(now_ts, 0)
        .single()
        .unwrap_or_else(|| Utc::now().with_timezone(&offset));

    let weekday = local_dt.format("%A").to_string();
    let month_name = local_dt.format("%B").to_string();
    let date_label = format!("{}, {} {}", weekday, local_dt.format("%b"), local_dt.day());

    let stats_30d = build_user_stats(store, &user, StatsRange::Days30, clamped_tz, now_ts);
    let stats_all = build_user_stats(store, &user, StatsRange::All, clamped_tz, now_ts);

    let is_cold_start = stats_all.summary.total_streams == 0
        && stats_all.library.liked_tracks == 0
        && stats_all.library.manual_playlists == 0;

    let top_artists_recent: Vec<String> = stats_30d
        .top_artists
        .iter()
        .take(12)
        .map(|a| format!("{} ({} plays)", a.name, a.streams))
        .collect();
    let top_artists_all: Vec<String> = stats_all
        .top_artists
        .iter()
        .take(14)
        .map(|a| a.name.clone())
        .collect();
    let top_tracks_recent: Vec<String> = stats_30d
        .top_tracks
        .iter()
        .take(10)
        .map(|t| {
            if t.track.genre.trim().is_empty() {
                format!("{} - {}", t.track.title, t.track.artist)
            } else {
                format!("{} - {} [{}]", t.track.title, t.track.artist, t.track.genre)
            }
        })
        .collect();
    let top_tracks_all: Vec<String> = stats_all
        .top_tracks
        .iter()
        .take(10)
        .map(|t| format!("{} - {}", t.track.title, t.track.artist))
        .collect();

    let genre_src = if !stats_30d.genres.is_empty() {
        &stats_30d.genres
    } else {
        &stats_all.genres
    };
    let top_genres: Vec<String> = genre_src
        .iter()
        .take(10)
        .map(|g| format!("{} ({:.0}%)", g.name, g.pct))
        .collect();

    let mood_src = if !stats_30d.moods.is_empty() {
        &stats_30d.moods
    } else {
        &stats_all.moods
    };
    let top_moods: Vec<String> = mood_src
        .iter()
        .take(6)
        .map(|m| format!("{} ({:.0}%)", m.label, m.pct))
        .collect();

    let top_decades: Vec<String> = stats_all
        .decades
        .iter()
        .take(5)
        .map(|d| format!("{} ({:.0}%)", d.label, d.pct))
        .collect();

    let manual_playlists: Vec<String> = store
        .list_playlists(user_id, false)
        .unwrap_or_default()
        .into_iter()
        .filter(|p| !p.is_auto && !p.name.trim().is_empty())
        .take(10)
        .map(|p| p.name)
        .collect();

    let mut skipped_seen = HashSet::new();
    let skipped_artists: Vec<String> = stats_30d
        .most_skipped
        .iter()
        .filter(|s| s.skips >= 2)
        .filter_map(|s| {
            let a = primary_artist(&s.track.artist);
            if a.is_empty() || !skipped_seen.insert(a.to_lowercase()) {
                None
            } else {
                Some(a)
            }
        })
        .take(6)
        .collect();

    let chart_artists: Vec<String> = if is_cold_start {
        store
            .popular_chart_artists(12)
            .into_iter()
            .map(|a| a.name)
            .collect()
    } else {
        Vec::new()
    };

    let mut previous_titles = Vec::new();
    if let Some(prev) = previous {
        if !prev.edition_title.is_empty() {
            previous_titles.push(prev.edition_title.clone());
        }
        for m in &prev.daily_mixes {
            previous_titles.push(m.title.clone());
        }
        for c in &prev.explore_categories {
            previous_titles.push(c.title.clone());
            for m in &c.mixes {
                previous_titles.push(m.title.clone());
            }
        }
    }

    let peak_hour_label = stats_all
        .peak_hour
        .and_then(|hr| stats_all.hourly.get(hr))
        .map(|h| h.label.clone())
        .unwrap_or_else(|| "Evening".to_string());

    Ok(UserTasteDigest {
        username: user.username,
        display_name,
        date_label,
        weekday,
        month_name,
        is_cold_start,
        chronotype: stats_all.persona.chronotype,
        peak_hour_label,
        discovery_rate_pct: stats_30d.summary.discovery_rate_pct,
        total_streams_30d: stats_30d.summary.total_streams,
        top_artists_recent,
        top_artists_all,
        top_tracks_recent,
        top_tracks_all,
        top_genres,
        top_moods,
        top_decades,
        manual_playlists,
        skipped_artists,
        chart_artists,
        previous_titles,
    })
}

fn build_system_prompt(extra: Option<&str>) -> String {
    let mut prompt = String::from(
        "You are Beatlab's overnight music editorial director and personal DJ. \
Each night while the listener sleeps, you craft a fresh, deeply personalized daily edition \
of their music streaming app: custom Home & Explore headlines, 4 custom-named Daily Mixes, \
custom descriptions/titles for their signature auto-playlists, 3 custom Explore categories \
full of clickable curated mixes, personalized Mood ordering & subtitles, and daily vibe \
search prompts.\n\n\
Style guidelines:\n\
- Be genuinely creative, musical, grounded, and specific to their actual taste (artists, scenes, eras, playlists, chronotype, day of week/season). Avoid generic cliches, cringe slang, or excessive emojis (no emojis at all — keep typography clean and editorial).\n\
- Never repeat yesterday's mix or category names.\n\
- Every mix you design will be populated with real songs by Beatlab's search & taste engine using your `seedQueries`, `seedArtists`, `genres`, `mood`, and `yearRange`. Therefore:\n\
  * `seedQueries` MUST be realistic music search/playlist queries that work well on Deezer/YouTube Music (e.g., \"japanese city pop\", \"90s shoegaze\", \"melodic house\", \"indie folk acoustic\", \"uk garage classics\").\n\
  * `seedArtists` MUST be real musical artists (a mix of the listener's favorites and genuinely adjacent artists they would love).\n\
  * `mood` MUST be either null or one of: \"chill\", \"energy\", \"late_night\", \"upbeat\", \"melancholy\", \"party\", \"acoustic\", \"groove\", \"rock_indie\".\n\
  * `c1` (bright accent) and `c2` (deep dark background) MUST be valid 6-digit hex colors like \"#38bdf8\" and \"#0c2d48\" with strong contrast for white text.\n\
- Respond ONLY with a single valid JSON object matching the requested schema. No markdown fences, no commentary.",
    );
    if let Some(extra_text) = extra {
        let trimmed = extra_text.trim();
        if !trimmed.is_empty() {
            prompt.push_str("\n\nAdditional server-owner guidance:\n");
            prompt.push_str(trimmed);
        }
    }
    prompt
}

fn build_user_prompt(d: &UserTasteDigest) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "Listener: {} (@{})\nLocal date: {} ({}, {})\n",
        d.display_name, d.username, d.date_label, d.weekday, d.month_name
    ));
    if d.is_cold_start {
        out.push_str("Listening history: Brand-new listener (cold start). Craft an inviting, eclectic, high-taste daily edition using contemporary and timeless scenes.\n");
        if !d.chart_artists.is_empty() {
            out.push_str(&format!("Trending catalog artists: {}\n", d.chart_artists.join(", ")));
        }
    } else {
        out.push_str(&format!(
            "Listening profile: {} · Peak hour: {} · 30d streams: {} · Discovery rate: {:.0}%\n",
            d.chronotype, d.peak_hour_label, d.total_streams_30d, d.discovery_rate_pct
        ));
        if !d.top_artists_recent.is_empty() {
            out.push_str(&format!("Top artists (30d): {}\n", d.top_artists_recent.join(", ")));
        }
        if !d.top_artists_all.is_empty() {
            out.push_str(&format!("All-time favorite artists: {}\n", d.top_artists_all.join(", ")));
        }
        if !d.top_tracks_recent.is_empty() {
            out.push_str(&format!("Recent heavy-rotation tracks: {}\n", d.top_tracks_recent.join(" | ")));
        }
        if !d.top_tracks_all.is_empty() {
            out.push_str(&format!("All-time top tracks: {}\n", d.top_tracks_all.join(" | ")));
        }
        if !d.top_genres.is_empty() {
            out.push_str(&format!("Top genres: {}\n", d.top_genres.join(", ")));
        }
        if !d.top_moods.is_empty() {
            out.push_str(&format!("Top moods: {}\n", d.top_moods.join(", ")));
        }
        if !d.top_decades.is_empty() {
            out.push_str(&format!("Favorite eras: {}\n", d.top_decades.join(", ")));
        }
        if !d.manual_playlists.is_empty() {
            out.push_str(&format!("User's own playlist names: {}\n", d.manual_playlists.join(", ")));
        }
        if !d.skipped_artists.is_empty() {
            out.push_str(&format!("Frequently skipped artists (avoid): {}\n", d.skipped_artists.join(", ")));
        }
    }

    if !d.previous_titles.is_empty() {
        out.push_str(&format!(
            "Yesterday's titles (DO NOT repeat these): {}\n",
            d.previous_titles.join(", ")
        ));
    }

    out.push_str(
        r##"
Return a JSON object with this exact shape:
{
  "editionTitle": "Short 2-4 word name for today's edition",
  "homeTagline": "One crisp editorial sentence under the Home greeting tailored to today & their taste",
  "exploreTagline": "One crisp editorial sentence under the Explore heading",
  "sections": {
    "quickPicks": "Custom heading for Quick Picks (2-4 words)",
    "mixedForYou": "Custom heading for Daily Mixes rail (2-4 words)",
    "madeForYou": "Custom heading for Made For You rail (2-4 words)",
    "recent": "Custom heading for Recently Played (2-4 words)",
    "relatedTracks": "Custom heading for Related Tracks (2-5 words)",
    "recommendedArtists": "Custom heading for Artists You Might Like (2-5 words)",
    "moods": "Custom heading for Moods & Vibes (2-4 words)",
    "decades": "Custom heading for Decades section (2-4 words)",
    "discovery": "Custom heading for Fresh Discovery Picks (2-4 words)"
  },
  "dailyMixes": [
    {
      "part": "1",
      "title": "Evocative custom mix name (2-4 words)",
      "artTitle": "Short cover label (1-3 words)",
      "subtitle": "Short description of the vibe (under 75 chars)",
      "c1": "#38bdf8",
      "c2": "#0c2d48",
      "seedQueries": ["realistic search query 1", "realistic search query 2"],
      "seedArtists": ["Real Artist 1", "Real Artist 2", "Real Artist 3"],
      "genres": ["genre1", "genre2"],
      "mood": "late_night",
      "yearRange": null,
      "familiarity": "familiar"
    },
    { "part": "2", "...": "distinct facet #2, familiarity: balanced" },
    { "part": "3", "...": "distinct facet #3 / discovery angle, familiarity: discovery" },
    { "part": "supermix", "...": "unified signature daily supermix blending their whole taste, familiarity: balanced" }
  ],
  "autoMixOverrides": {
    "supermix": { "title": "Custom Supermix name", "comment": "Short daily note" },
    "discover": { "title": "Custom Discovery mix name", "comment": "Short daily note" },
    "repeat": { "title": "Custom On Repeat name", "comment": "Short daily note" },
    "gems": { "title": "Custom Forgotten Favorites name", "comment": "Short daily note" },
    "playlist_dna": { "title": "Custom Playlist DNA name", "comment": "Short daily note" },
    "user_radio": { "title": "Custom Personal Radio name", "comment": "Short daily note" }
  },
  "exploreCategories": [
    {
      "slug": "url-safe-category-slug",
      "kicker": "SHORT TAG",
      "title": "Custom Explore category title (2-5 words)",
      "subtitle": "One short line describing this shelf",
      "mixes": [
        {
          "key": "url-safe-mix-slug",
          "kicker": "MICRO-GENRE",
          "title": "Custom mix title (2-4 words)",
          "subtitle": "Short compelling description",
          "query": "realistic music playlist search query",
          "seedArtists": ["Real Artist A", "Real Artist B"],
          "genres": ["genre1"],
          "mood": "chill",
          "yearRange": [1990, 2005],
          "c1": "#a78bfa",
          "c2": "#26174a"
        }
      ]
    }
  ],
  "moodOverrides": [
    { "slug": "late_night", "label": "Fresh label or keep classic", "subtitle": "Personalized 1-line subtitle" }
  ],
  "searchPrompts": [
    { "label": "Short clickable vibe label (2-5 words)", "query": "instant mix query string" }
  ]
}
Requirements:
- Provide exactly 4 items in `dailyMixes` with `part` values `"1"`, `"2"`, `"3"`, `"supermix"`.
- Provide 3 distinct categories in `exploreCategories`, each with 3 to 4 mixes (9-12 total custom Explore mixes).
- Provide 5 to 9 items in `moodOverrides` ordered from most relevant to this listener first, using only valid mood slugs (`chill`, `energy`, `late_night`, `upbeat`, `melancholy`, `party`, `acoustic`, `groove`, `rock_indie`).
- Provide 4 to 6 items in `searchPrompts`.
"##,
    );
    out
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RawLlmResponse {
    #[serde(default)]
    edition_title: String,
    #[serde(default)]
    home_tagline: String,
    #[serde(default)]
    explore_tagline: String,
    #[serde(default)]
    sections: LlmSectionTitles,
    #[serde(default)]
    daily_mixes: Vec<RawDailyMix>,
    #[serde(default)]
    auto_mix_overrides: HashMap<String, LlmAutoMixCopy>,
    #[serde(default)]
    explore_categories: Vec<RawExploreCategory>,
    #[serde(default)]
    mood_overrides: Vec<LlmMoodOverride>,
    #[serde(default)]
    search_prompts: Vec<LlmSearchPrompt>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RawDailyMix {
    #[serde(default)]
    part: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    art_title: String,
    #[serde(default)]
    subtitle: String,
    #[serde(default)]
    c1: String,
    #[serde(default)]
    c2: String,
    #[serde(default)]
    seed_queries: Vec<String>,
    #[serde(default)]
    seed_artists: Vec<String>,
    #[serde(default)]
    genres: Vec<String>,
    #[serde(default)]
    mood: Option<String>,
    #[serde(default)]
    year_range: Option<Vec<i32>>,
    #[serde(default)]
    familiarity: String,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RawExploreCategory {
    #[serde(default)]
    slug: String,
    #[serde(default)]
    kicker: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    subtitle: String,
    #[serde(default)]
    mixes: Vec<RawExploreMix>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RawExploreMix {
    #[serde(default)]
    key: String,
    #[serde(default)]
    kicker: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    subtitle: String,
    #[serde(default)]
    query: String,
    #[serde(default)]
    seed_artists: Vec<String>,
    #[serde(default)]
    genres: Vec<String>,
    #[serde(default)]
    mood: Option<String>,
    #[serde(default)]
    year_range: Option<Vec<i32>>,
    #[serde(default)]
    c1: String,
    #[serde(default)]
    c2: String,
}

fn clean_text(raw: &str, max_chars: usize) -> String {
    let collapsed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_str(&collapsed, max_chars)
}

fn truncate_str(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        s.chars().take(max_chars).collect::<String>().trim_end().to_string()
    }
}

fn clean_list(items: Vec<String>, max_items: usize, max_chars: usize) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for item in items {
        let c = clean_text(&item, max_chars);
        if c.is_empty() {
            continue;
        }
        if seen.insert(c.to_lowercase()) {
            out.push(c);
            if out.len() >= max_items {
                break;
            }
        }
    }
    out
}

fn valid_hex_color(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.len() == 7 && s.starts_with('#') && s[1..].bytes().all(|b| b.is_ascii_hexdigit()) {
        Some(s.to_ascii_lowercase())
    } else {
        None
    }
}

fn sanitize_colors(c1: &str, c2: &str, idx: usize) -> (String, String) {
    let (fb1, fb2) = CURATED_PALETTES[idx % CURATED_PALETTES.len()];
    (
        valid_hex_color(c1).unwrap_or_else(|| fb1.to_string()),
        valid_hex_color(c2).unwrap_or_else(|| fb2.to_string()),
    )
}

fn sanitize_year_range(raw: Option<Vec<i32>>) -> Option<(i32, i32)> {
    let v = raw?;
    if v.len() < 2 {
        return None;
    }
    let a = v[0].clamp(1920, 2035);
    let b = v[1].clamp(1920, 2035);
    if a <= b {
        Some((a, b))
    } else {
        Some((b, a))
    }
}

fn sanitize_mood(raw: Option<String>) -> Option<String> {
    let s = raw?;
    Mood::from_slug(&s).map(|m| m.slug().to_string())
}

fn sanitize_experience(
    raw: RawLlmResponse,
    date_label: &str,
    day_key: i64,
    generated_at: i64,
    model: &str,
) -> DailyLlmExperience {
    let edition_title = {
        let t = clean_text(&raw.edition_title, 48);
        if t.is_empty() {
            format!("{date_label} Edition")
        } else {
            t
        }
    };
    let home_tagline = clean_text(&raw.home_tagline, 160);
    let explore_tagline = clean_text(&raw.explore_tagline, 160);

    let sections = LlmSectionTitles {
        quick_picks: clean_text(&raw.sections.quick_picks, 42),
        mixed_for_you: clean_text(&raw.sections.mixed_for_you, 42),
        made_for_you: clean_text(&raw.sections.made_for_you, 42),
        recent: clean_text(&raw.sections.recent, 42),
        related_tracks: clean_text(&raw.sections.related_tracks, 42),
        recommended_artists: clean_text(&raw.sections.recommended_artists, 42),
        moods: clean_text(&raw.sections.moods, 42),
        decades: clean_text(&raw.sections.decades, 42),
        discovery: clean_text(&raw.sections.discovery, 42),
    };

    // Normalize daily mixes to canonical parts ["1", "2", "3", "supermix"].
    let target_parts = ["1", "2", "3", "supermix"];
    let mut by_part: HashMap<String, RawDailyMix> = HashMap::new();
    let mut overflow: Vec<RawDailyMix> = Vec::new();
    for dm in raw.daily_mixes {
        let norm = dm.part.trim().to_ascii_lowercase();
        let mapped = match norm.as_str() {
            "1" | "mix1" | "mix_1" | "01" => Some("1"),
            "2" | "mix2" | "mix_2" | "02" => Some("2"),
            "3" | "mix3" | "mix_3" | "03" => Some("3"),
            "supermix" | "super" | "4" | "daily_supermix" => Some("supermix"),
            _ => None,
        };
        if let Some(p) = mapped {
            if !by_part.contains_key(p) {
                by_part.insert(p.to_string(), dm);
                continue;
            }
        }
        overflow.push(dm);
    }

    let mut daily_mixes = Vec::with_capacity(4);
    for (idx, part) in target_parts.iter().enumerate() {
        let item = by_part
            .remove(*part)
            .or_else(|| if !overflow.is_empty() { Some(overflow.remove(0)) } else { None });
        if let Some(dm) = item {
            let is_super = *part == "supermix";
            let title = {
                let t = clean_text(&dm.title, 44);
                if t.is_empty() {
                    if is_super {
                        "Daily Supermix".to_string()
                    } else {
                        format!("Daily Mix {}", idx + 1)
                    }
                } else {
                    t
                }
            };
            let art_title = {
                let a = clean_text(&dm.art_title, 26);
                if a.is_empty() {
                    truncate_str(&title, 22)
                } else {
                    a
                }
            };
            let subtitle = clean_text(&dm.subtitle, 100);
            let (c1, c2) = sanitize_colors(&dm.c1, &dm.c2, idx);
            let familiarity = match dm.familiarity.trim().to_ascii_lowercase().as_str() {
                "familiar" => "familiar".to_string(),
                "discovery" => "discovery".to_string(),
                _ => "balanced".to_string(),
            };
            daily_mixes.push(LlmDailyMixSpec {
                part: (*part).to_string(),
                title,
                art_title,
                subtitle,
                c1,
                c2,
                seed_queries: clean_list(dm.seed_queries, 3, 60),
                seed_artists: clean_list(dm.seed_artists, 5, 60),
                genres: clean_list(dm.genres, 3, 36),
                mood: sanitize_mood(dm.mood),
                year_range: sanitize_year_range(dm.year_range),
                familiarity,
            });
        }
    }

    let allowed_auto_kinds = [
        "supermix",
        "discover",
        "repeat",
        "gems",
        "playlist_dna",
        "user_radio",
    ];
    let mut auto_mix_overrides = HashMap::new();
    for (k, v) in raw.auto_mix_overrides {
        let norm = k.trim().to_ascii_lowercase().replace('-', "_");
        if !allowed_auto_kinds.contains(&norm.as_str()) {
            continue;
        }
        let title = clean_text(&v.title, 44);
        let comment = clean_text(&v.comment, 110);
        if !title.is_empty() || !comment.is_empty() {
            auto_mix_overrides.insert(norm, LlmAutoMixCopy { title, comment });
        }
    }

    let mut explore_categories = Vec::new();
    let mut used_mix_keys: HashSet<String> = HashSet::new();
    let mut used_cat_slugs: HashSet<String> = HashSet::new();
    let mut palette_idx = 4usize;

    for (ci, cat) in raw.explore_categories.into_iter().take(4).enumerate() {
        let title = clean_text(&cat.title, 52);
        if title.is_empty() {
            continue;
        }
        let mut cat_slug = slug(if cat.slug.trim().is_empty() {
            &title
        } else {
            &cat.slug
        });
        if !used_cat_slugs.insert(cat_slug.clone()) {
            cat_slug = format!("{cat_slug}-{}", ci + 1);
            used_cat_slugs.insert(cat_slug.clone());
        }
        let kicker = {
            let k = clean_text(&cat.kicker, 24).to_ascii_uppercase();
            if k.is_empty() {
                "CURATED".to_string()
            } else {
                k
            }
        };
        let subtitle = clean_text(&cat.subtitle, 120);

        let mut mixes = Vec::new();
        for (mi, m) in cat.mixes.into_iter().take(4).enumerate() {
            let m_title = clean_text(&m.title, 48);
            if m_title.is_empty() {
                continue;
            }
            let raw_key = if m.key.trim().is_empty() {
                slug(&m_title)
            } else {
                slug(&m.key)
            };
            let base_key = if raw_key.starts_with("llm-") {
                raw_key
            } else {
                format!("llm-{raw_key}")
            };
            let mut key = truncate_str(&base_key, 48);
            if !used_mix_keys.insert(key.clone()) {
                key = format!("{}-{}-{}", truncate_str(&base_key, 40), ci + 1, mi + 1);
                used_mix_keys.insert(key.clone());
            }
            let query = {
                let q = clean_text(&m.query, 70);
                if q.is_empty() {
                    m_title.clone()
                } else {
                    q
                }
            };
            let m_kicker = {
                let k = clean_text(&m.kicker, 22).to_ascii_uppercase();
                if k.is_empty() {
                    kicker.clone()
                } else {
                    k
                }
            };
            let m_sub = clean_text(&m.subtitle, 100);
            let (c1, c2) = sanitize_colors(&m.c1, &m.c2, palette_idx);
            palette_idx += 1;

            mixes.push(LlmExploreMixSpec {
                key,
                kicker: m_kicker,
                title: m_title,
                subtitle: m_sub,
                query,
                seed_artists: clean_list(m.seed_artists, 5, 60),
                genres: clean_list(m.genres, 3, 36),
                mood: sanitize_mood(m.mood),
                year_range: sanitize_year_range(m.year_range),
                c1,
                c2,
            });
        }
        if !mixes.is_empty() {
            explore_categories.push(LlmExploreCategory {
                slug: cat_slug,
                kicker,
                title,
                subtitle,
                mixes,
            });
        }
    }

    let mut seen_moods = HashSet::new();
    let mut mood_overrides = Vec::new();
    for mo in raw.mood_overrides {
        let Some(mood) = Mood::from_slug(&mo.slug) else {
            continue;
        };
        if !seen_moods.insert(mood) {
            continue;
        }
        mood_overrides.push(LlmMoodOverride {
            slug: mood.slug().to_string(),
            label: clean_text(&mo.label, 32),
            subtitle: clean_text(&mo.subtitle, 84),
        });
    }

    let mut seen_prompts = HashSet::new();
    let mut search_prompts = Vec::new();
    for sp in raw.search_prompts.into_iter().take(6) {
        let label = clean_text(&sp.label, 38);
        let query = clean_text(&sp.query, 64);
        if label.is_empty() || query.is_empty() {
            continue;
        }
        if seen_prompts.insert(query.to_lowercase()) {
            search_prompts.push(LlmSearchPrompt { label, query });
        }
    }

    DailyLlmExperience {
        version: EXPERIENCE_SCHEMA_VERSION,
        generated_at,
        day_key,
        date_label: date_label.to_string(),
        edition_title,
        model: model.to_string(),
        home_tagline,
        explore_tagline,
        sections,
        daily_mixes,
        auto_mix_overrides,
        explore_categories,
        mood_overrides,
        search_prompts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{routing::post, Json, Router};

    #[test]
    fn normalize_completions_url_handles_base_and_full_paths() {
        assert_eq!(
            normalize_completions_url("https://api.openai.com/v1").unwrap(),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            normalize_completions_url("https://openrouter.ai/api/v1/").unwrap(),
            "https://openrouter.ai/api/v1/chat/completions"
        );
        assert_eq!(
            normalize_completions_url("http://localhost:11434/v1/chat/completions").unwrap(),
            "http://localhost:11434/v1/chat/completions"
        );
        assert!(normalize_completions_url("ftp://example.com/v1").is_err());
        assert!(normalize_completions_url("").is_err());
    }

    #[test]
    fn extract_json_object_strips_think_and_code_fences() {
        let sample = "<think>planning the mixes...</think>\n```json\n{\"editionTitle\":\"Velvet Morning\"}\n```";
        assert_eq!(
            extract_json_object(sample),
            Some("{\"editionTitle\":\"Velvet Morning\"}")
        );
        let plain = "Here is your JSON: {\"a\": 1, \"b\": {\"c\": 2}} enjoy!";
        assert_eq!(
            extract_json_object(plain),
            Some("{\"a\": 1, \"b\": {\"c\": 2}}")
        );
        // Trailing prose with a stray brace must not extend the extracted object.
        let trailing = "{\"a\": 1} } hope this helps!";
        assert_eq!(extract_json_object(trailing), Some("{\"a\": 1}"));
        // A thinking block *after* the JSON must not hide it.
        let think_tail = "{\"b\": 2} <think>looks good</think>";
        assert_eq!(extract_json_object(think_tail), Some("{\"b\": 2}"));
        // Braces inside JSON strings must not confuse the scan.
        let braces_in_str = "{\"msg\": \"a } b { c\"}";
        assert_eq!(
            extract_json_object(braces_in_str),
            Some("{\"msg\": \"a } b { c\"}")
        );
    }

    #[test]
    fn llm_day_turns_over_at_refresh_hour_in_user_timezone() {
        // 2026-11-25 03:59:00 UTC with refresh_hour=4, tz=0 -> previous day
        let before = chrono::DateTime::parse_from_rfc3339("2026-11-25T03:59:00Z")
            .unwrap()
            .timestamp();
        let after = chrono::DateTime::parse_from_rfc3339("2026-11-25T04:01:00Z")
            .unwrap()
            .timestamp();
        assert_eq!(llm_day(after, 0, 4), llm_day(before, 0, 4) + 1);
        // UTC-5 (tz = -300): 08:59 UTC is 03:59 local, 09:01 UTC is 04:01 local.
        let before_est = chrono::DateTime::parse_from_rfc3339("2026-11-25T08:59:00Z")
            .unwrap()
            .timestamp();
        let after_est = chrono::DateTime::parse_from_rfc3339("2026-11-25T09:01:00Z")
            .unwrap()
            .timestamp();
        assert_eq!(llm_day(after_est, -300, 4), llm_day(before_est, -300, 4) + 1);
    }

    #[test]
    fn sanitize_experience_validates_colors_keys_and_moods() {
        let raw: RawLlmResponse = serde_json::from_value(json!({
            "editionTitle": "Midnight Neon",
            "homeTagline": "Warm synths and sharp guitars for your evening.",
            "dailyMixes": [
                {
                    "part": "1",
                    "title": "Neon Drive",
                    "artTitle": "Neon Drive",
                    "subtitle": "Synthpop & night grooves",
                    "c1": "not-a-color",
                    "c2": "#112233",
                    "seedQueries": ["synthwave night", "city pop"],
                    "seedArtists": ["The Midnight", "Kavinsky"],
                    "genres": ["Synthwave"],
                    "mood": "late_night",
                    "familiarity": "familiar"
                }
            ],
            "exploreCategories": [
                {
                    "slug": "Late Night Drives",
                    "kicker": "after hours",
                    "title": "After-Hours Detours",
                    "subtitle": "Moody nocturnal tracks",
                    "mixes": [
                        {
                            "key": "tokyo-rain",
                            "kicker": "vibe",
                            "title": "Tokyo Rain",
                            "subtitle": "Rain-soaked city pop",
                            "query": "japanese city pop",
                            "c1": "#ff00aa",
                            "c2": "bad",
                            "yearRange": [1989, 1979]
                        }
                    ]
                }
            ],
            "moodOverrides": [
                { "slug": "late_night", "label": "After Dark", "subtitle": "Built for 2AM" },
                { "slug": "invalid_mood", "label": "Nope", "subtitle": "Ignored" }
            ],
            "searchPrompts": [
                { "label": "Rainy Tokyo Jazz", "query": "japanese jazz chill" }
            ]
        }))
        .unwrap();

        let exp = sanitize_experience(raw, "Wed, Nov 25", 100, 1700000000, "test-model");
        assert_eq!(exp.daily_mixes.len(), 1);
        assert!(exp.daily_mixes[0].c1.starts_with('#'));
        assert_eq!(exp.daily_mixes[0].c2, "#112233");
        assert_eq!(exp.explore_categories.len(), 1);
        assert_eq!(exp.explore_categories[0].mixes[0].key, "llm-tokyo-rain");
        assert_eq!(exp.explore_categories[0].mixes[0].year_range, Some((1979, 1989)));
        assert_eq!(exp.mood_overrides.len(), 1);
        assert_eq!(exp.mood_overrides[0].slug, "late_night");
        assert_eq!(exp.search_prompts.len(), 1);
    }

    #[tokio::test]
    async fn call_chat_completions_works_against_mock_openai_server() {
        let app = Router::new().route(
            "/v1/chat/completions",
            post(|Json(payload): Json<Value>| async move {
                assert_eq!(payload["model"], "demo-model");
                Json(json!({
                    "choices": [{
                        "message": {
                            "role": "assistant",
                            "content": "{\"editionTitle\":\"Test Edition\",\"homeTagline\":\"Hello\"}"
                        }
                    }]
                }))
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let cfg = LlmConfig {
            endpoint_base: format!("http://{addr}/v1"),
            completions_url: format!("http://{addr}/v1/chat/completions"),
            api_key: "sk-test".to_string(),
            model: "demo-model".to_string(),
            refresh_hour: 4,
            temperature: 0.7,
            max_tokens: 1024,
            timeout_secs: 15,
            extra_system_prompt: None,
            extra_headers: vec![],
            reasoning_effort: None,
        };
        let curator = LlmCurator::new_with_config(cfg);
        let out = curator
            .call_chat_completions("sys", "usr")
            .await
            .unwrap();
        let extracted = extract_json_object(&out).unwrap();
        let parsed: Value = serde_json::from_str(extracted).unwrap();
        assert_eq!(parsed["editionTitle"], "Test Edition");
        server.abort();
    }

    #[tokio::test]
    async fn full_daily_llm_refresh_materializes_custom_mixes_and_explore_categories() {
        use crate::models::Track;
        use crate::sources::archive::ArchiveSource;
        use crate::sources::deezer::DeezerSource;
        use crate::sources::local::LocalSource;
        use crate::sources::soulseek::SoulseekSource;
        use crate::sources::ytm::YtmSource;
        use crate::sources::SourceRegistry;

        let llm_reply = json!({
            "editionTitle": "Midnight Neon Edition",
            "homeTagline": "Crisp synth hooks and late-night guitars tuned to your week.",
            "exploreTagline": "Fresh detours hand-picked overnight.",
            "sections": {
                "quickPicks": "Jump Right In",
                "mixedForYou": "Tonight's Custom Mixes",
                "madeForYou": "Your Signature Stacks",
                "moods": "Headspaces"
            },
            "dailyMixes": [
                {
                    "part": "1",
                    "title": "Neon Rain Drive",
                    "artTitle": "Neon Rain",
                    "subtitle": "Dreamy midnight synthwave",
                    "c1": "#38bdf8",
                    "c2": "#0c2d48",
                    "seedQueries": ["synthwave"],
                    "seedArtists": ["The Midnight"],
                    "genres": ["Synthwave"],
                    "mood": "late_night",
                    "familiarity": "familiar"
                },
                {
                    "part": "2",
                    "title": "Velvet Static",
                    "artTitle": "Velvet Static",
                    "subtitle": "Hazy shoegaze & indie guitars",
                    "c1": "#a78bfa",
                    "c2": "#26174a",
                    "seedQueries": ["shoegaze"],
                    "seedArtists": ["Slowdive"],
                    "genres": ["Indie"],
                    "mood": "chill",
                    "familiarity": "balanced"
                },
                {
                    "part": "3",
                    "title": "After-Hours Detour",
                    "artTitle": "Detour",
                    "subtitle": "Adjacent discoveries for tonight",
                    "c1": "#f472b6",
                    "c2": "#4a1230",
                    "seedQueries": ["dream pop"],
                    "seedArtists": ["Beach House"],
                    "genres": ["Dream Pop"],
                    "mood": "late_night",
                    "familiarity": "discovery"
                },
                {
                    "part": "supermix",
                    "title": "All-Night Blueprint",
                    "artTitle": "Blueprint",
                    "subtitle": "Your full sonic range in one mix",
                    "c1": "#34d399",
                    "c2": "#0b3b2c",
                    "seedQueries": ["synthwave", "dream pop"],
                    "seedArtists": ["The Midnight", "Slowdive"],
                    "genres": ["Synthwave", "Indie"],
                    "mood": "late_night",
                    "familiarity": "balanced"
                }
            ],
            "autoMixOverrides": {
                "supermix": { "title": "Midnight Core", "comment": "Your core rotation, refreshed overnight" },
                "discover": { "title": "Radar Sweep", "comment": "Fresh cuts just outside your usual orbit" }
            },
            "exploreCategories": [
                {
                    "slug": "nocturnal-detours",
                    "kicker": "LATE NIGHT",
                    "title": "Nocturnal Detours",
                    "subtitle": "Atmospheric worlds built around your taste",
                    "mixes": [
                        {
                            "key": "tokyo-midnight",
                            "kicker": "MICRO-GENRE",
                            "title": "Tokyo Rain & Neon",
                            "subtitle": "Lush synths and rain-soaked grooves",
                            "query": "synthwave",
                            "seedArtists": ["The Midnight"],
                            "genres": ["Synthwave"],
                            "mood": "late_night",
                            "c1": "#22d3ee",
                            "c2": "#0b3640"
                        }
                    ]
                }
            ],
            "moodOverrides": [
                { "slug": "late_night", "label": "After Dark", "subtitle": "Tailored for your midnight sessions" }
            ],
            "searchPrompts": [
                { "label": "Rainy Tokyo Synthwave", "query": "synthwave night drive" }
            ]
        });

        let reply_str = llm_reply.to_string();
        let app = Router::new().route(
            "/v1/chat/completions",
            post(move || {
                let body = reply_str.clone();
                async move {
                    Json(json!({
                        "choices": [{
                            "message": { "role": "assistant", "content": body }
                        }]
                    }))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let store = Store::open_memory().unwrap();
        let tmp = std::env::temp_dir().join(format!("beatlab-llm-test-{}", uuid::Uuid::new_v4()));
        let sources = SourceRegistry::new(
            DeezerSource::new(),
            YtmSource::new(None),
            ArchiveSource::new(),
            Arc::new(LocalSource::new(store.clone(), vec![])),
            SoulseekSource::new(store.clone(), tmp.join("slsk")),
        );
        let taste = TasteEngine::new(store.clone(), sources);

        let (user, _) = store.authenticate_or_create_user("alice", "password123").unwrap();
        let tracks: Vec<Track> = (1..=16)
            .map(|i| Track {
                id: format!("loc:synth-{i}"),
                source: "local".into(),
                title: format!("Night Track {i}"),
                artist: match i % 3 {
                    0 => "The Midnight".into(),
                    1 => "Slowdive".into(),
                    _ => "Beach House".into(),
                },
                album: format!("Album {i}"),
                genre: if i % 2 == 0 { "Synthwave".into() } else { "Indie, Dream Pop".into() },
                moods: "late_night,chill".into(),
                year: 2018,
                duration: 210,
                bitrate: 900,
                format: "flac".into(),
                cover_url: format!("https://example.com/c{i}.jpg"),
                ..Default::default()
            })
            .collect();
        store.upsert_tracks(&tracks).unwrap();
        store.record_event(user.id, "loc:synth-1", "play", 1.0).unwrap();
        store.record_event(user.id, "loc:synth-1", "complete", 1.0).unwrap();

        let cfg = LlmConfig {
            endpoint_base: format!("http://{addr}/v1"),
            completions_url: format!("http://{addr}/v1/chat/completions"),
            api_key: String::new(),
            model: "local-test-model".to_string(),
            refresh_hour: 4,
            temperature: 0.8,
            max_tokens: 2000,
            timeout_secs: 20,
            extra_system_prompt: None,
            extra_headers: vec![],
            reasoning_effort: None,
        };
        let curator = LlmCurator::new_with_config(cfg);

        // First refresh runs and materializes everything.
        let ran = curator.refresh_user_experience(&taste, user.id, false).await.unwrap();
        assert!(ran);

        // Second non-forced refresh on the same day is skipped.
        let ran_again = curator.refresh_user_experience(&taste, user.id, false).await.unwrap();
        assert!(!ran_again);

        let exp = LlmCurator::load_experience(&store, user.id).unwrap();
        assert_eq!(exp.edition_title, "Midnight Neon Edition");
        assert_eq!(exp.explore_categories.len(), 1);
        assert_eq!(exp.explore_categories[0].mixes[0].key, "llm-tokyo-midnight");

        // Custom Daily Mix cards reflect the LLM's titles & colors.
        let cards = taste.mixed_for_you_cards(user.id);
        assert_eq!(cards.len(), 4);
        assert_eq!(cards[0].title, "Neon Rain Drive");
        assert_eq!(cards[0].c1, "#38bdf8");
        assert_eq!(cards[3].title, "All-Night Blueprint");
        assert!(cards[3].is_super);

        // Standard auto-playlists inherit custom LLM titles while keeping stable IDs.
        let supermix_pl = store
            .get_playlist(&format!("auto-u{}-supermix", user.id))
            .unwrap()
            .unwrap();
        assert_eq!(supermix_pl.name, "Midnight Core");

        // Custom Explore category mix has real materialized tracks & preview covers.
        let llm_mix = exp.find_explore_mix("llm-tokyo-midnight").unwrap();
        let (cnt, covers) = taste.curated_llm_explore_preview(user.id, llm_mix);
        assert!(cnt > 0);
        assert!(!covers.is_empty());

        server.abort();
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
