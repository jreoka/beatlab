//! Optional OpenAI-compatible LLM integration.
//!
//! Everything in here is deliberately *additive*: when no API key / endpoint is
//! configured the client is a no-op and the existing deterministic curation,
//! instant-mix and search pipelines behave exactly as before.
//!
//! Configuration is read once per process from the environment:
//!   BEATLAB_LLM_API_KEY   (or OPENAI_API_KEY)
//!   BEATLAB_LLM_BASE_URL  (or OPENAI_BASE_URL / OPENAI_API_BASE)   default https://api.openai.com/v1
//!   BEATLAB_LLM_MODEL     (or OPENAI_MODEL)                        default gpt-4o-mini
//!   BEATLAB_LLM_ENABLED   1/true/yes to force on, 0/false/no to force off (default: on when key set)
//!   BEATLAB_LLM_TIMEOUT_MS            default 12000
//!   BEATLAB_LLM_MAX_TOKENS            default 0 (uncapped)
//!   BEATLAB_LLM_TEMPERATURE           default 0.2
//!
//! All LLM work happens in background tasks. Live search / instant mix only
//! ever read small, already-computed JSON blobs out of SQLite.

use crate::models::{primary_artist, slug, Track};
use crate::store::Store;
use chrono::Utc;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tracing::{info, warn};

const STATS_KV_KEY: &str = "llm:stats:v1";
const PENDING_Q_KV_KEY: &str = "llm:pending_queries:v1";
const MIX_CACHE_PREFIX: &str = "llm:mix:v5:";
const META_CACHE_PREFIX: &str = "llm:meta:v1:";
const POOL_CACHE_PREFIX: &str = "llm:pool:v1:";
const QUERY_EXP_PREFIX: &str = "llm:qexp:v1:";
const SEARCH_ANSWER_PREFIX: &str = "llm:answer:v1:";
const VERIFIED_MIX_PREFIX: &str = "llm:verified_mix:v1:";
const MIX_CACHE_TTL_SECS: i64 = 14 * 24 * 3600;
const PLAN_CACHE_PREFIX: &str = "llm:plan:v1:";
const PLAN_PENDING_KV_KEY: &str = "llm:pending_plans:v1";
const PLAN_TTL_SECS: i64 = 12 * 3600;

fn env_any(names: &[&str]) -> Option<String> {
    for name in names {
        if let Ok(raw) = std::env::var(name) {
            let v = raw.trim().to_string();
            if !v.is_empty() {
                return Some(v);
            }
        }
    }
    None
}

fn parse_bool(raw: Option<String>, default: bool) -> bool {
    match raw
        .map(|s| s.trim().to_ascii_lowercase())
        .unwrap_or_default()
        .as_str()
    {
        "1" | "true" | "yes" | "on" => true,
        "0" | "false" | "no" | "off" => false,
        _ => default,
    }
}

fn parse_u64(raw: Option<String>, default: u64) -> u64 {
    raw.and_then(|s| s.trim().parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(default)
}

#[derive(Debug, Clone)]
pub struct LlmConfig {
    pub enabled: bool,
    pub base_url: String,
    pub model: String,
    pub api_key: String,
    pub timeout_ms: u64,
    pub max_tokens: u64,
    pub temperature: f32,
    /// Optional background web research when the model is unsure about a fact.
    pub research_enabled: bool,
    pub search_url_template: String,
    pub research_max_results: usize,
    pub research_timeout_ms: u64,
}

impl LlmConfig {
    pub fn from_env() -> Self {
        let api_key = env_any(&["BEATLAB_LLM_API_KEY", "OPENAI_API_KEY"]).unwrap_or_default();
        let base_url = env_any(&[
            "BEATLAB_LLM_BASE_URL",
            "OPENAI_BASE_URL",
            "OPENAI_API_BASE",
        ])
        .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
        let model = env_any(&["BEATLAB_LLM_MODEL", "OPENAI_MODEL"])
            .unwrap_or_else(|| "gpt-4o-mini".to_string());
        let enabled = parse_bool(
            env_any(&["BEATLAB_LLM_ENABLED"]),
            !api_key.trim().is_empty() && !base_url.trim().is_empty(),
        );
        let temperature = env_any(&["BEATLAB_LLM_TEMPERATURE"])
            .and_then(|s| s.parse::<f32>().ok())
            .map(|v| v.clamp(0.0, 2.0))
            .unwrap_or(0.2);
        Self {
            enabled,
            base_url: base_url.trim_end_matches('/').to_string(),
            model,
            api_key,
            timeout_ms: parse_u64(env_any(&["BEATLAB_LLM_TIMEOUT_MS"]), 12_000),
            // 0 means "uncapped" — no max_tokens field is sent to the model.
            max_tokens: parse_u64(env_any(&["BEATLAB_LLM_MAX_TOKENS"]), 0),
            temperature,
            research_enabled: parse_bool(env_any(&["BEATLAB_LLM_RESEARCH"]), true),
            search_url_template: env_any(&["BEATLAB_LLM_SEARCH_URL"])
                .unwrap_or_else(|| "https://html.duckduckgo.com/html/?q={q}".to_string()),
            research_max_results: parse_u64(
                env_any(&["BEATLAB_LLM_RESEARCH_RESULTS"]),
                5,
            ) as usize,
            research_timeout_ms: parse_u64(
                env_any(&["BEATLAB_LLM_RESEARCH_TIMEOUT_MS"]),
                6000,
            ),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LlmCallLog {
    pub task: String,
    pub ok: bool,
    pub ms: u64,
    pub at: i64,
    pub error: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct LlmStats {
    calls_total: u64,
    calls_ok: u64,
    calls_failed: u64,
    prompt_tokens: u64,
    completion_tokens: u64,
    total_latency_ms: u64,
    last_latency_ms: u64,
    last_call_at: i64,
    last_ok_at: i64,
    last_error: String,
    last_error_at: i64,
    by_task: BTreeMap<String, u64>,
    cache_writes: u64,
    research_calls: u64,
    research_cache_hits: u64,
    research_failures: u64,
    recent: Vec<LlmCallLog>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MixPlan {
    pub title: String,
    pub subtitle: String,
    pub ranked_track_ids: Vec<String>,
    pub rejected_track_ids: Vec<String>,
    pub extra_queries: Vec<String>,
    #[serde(default)]
    pub generated_at: i64,
}

impl MixPlan {
    fn from_value(value: &Value, allowed: &[Track]) -> Option<Self> {
        let allowed_ids: HashSet<&str> = allowed.iter().map(|t| t.id.as_str()).collect();
        let title = value
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .chars()
            .take(80)
            .collect::<String>();
        let subtitle = value
            .get("subtitle")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .chars()
            .take(140)
            .collect::<String>();
        let ranked_track_ids = value
            .get("rankedTrackIds")
            .or_else(|| value.get("ranked_track_ids"))
            .or_else(|| value.get("trackIds"))
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .filter(|id| allowed_ids.contains(*id))
                    .take(32)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let rejected_track_ids = value
            .get("rejectedTrackIds")
            .or_else(|| value.get("rejected_track_ids"))
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .filter(|id| allowed_ids.contains(*id))
                    .take(32)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let extra_queries = value
            .get("extraQueries")
            .or_else(|| value.get("extra_queries"))
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(|q| q.trim().chars().take(80).collect::<String>())
                    .filter(|q| !q.is_empty())
                    .take(3)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if title.is_empty() && ranked_track_ids.is_empty() && rejected_track_ids.is_empty() {
            return None;
        }
        Some(Self {
            title,
            subtitle,
            ranked_track_ids,
            rejected_track_ids,
            extra_queries,
            generated_at: Utc::now().timestamp(),
        })
    }
}

/// Apply an LLM plan to a deterministic mix. The model can both reject
/// off-theme tracks and rank the keepers. A floor is enforced so a bad plan
/// can never strip a mix down to almost nothing.
pub fn apply_mix_plan(plan: &MixPlan, tracks: Vec<Track>) -> Vec<Track> {
    if tracks.is_empty() {
        return tracks;
    }
    let rejects: HashSet<&str> = plan
        .rejected_track_ids
        .iter()
        .map(String::as_str)
        .collect();
    // Quality over quantity: allow the model to cut a mix down to 8 tracks
    // rather than forcing it to fill slots with off-theme leftovers.
    let floor = 8.min(tracks.len());
    let mut filtered: Vec<Track> = tracks
        .iter()
        .filter(|t| !rejects.contains(t.id.as_str()))
        .cloned()
        .collect();
    if filtered.len() < floor {
        for t in &tracks {
            if rejects.contains(t.id.as_str()) && filtered.len() < floor {
                filtered.push(t.clone());
            }
        }
    }
    if plan.ranked_track_ids.is_empty() {
        return filtered;
    }
    let mut pool: BTreeMap<String, Track> = filtered
        .iter()
        .map(|t| (t.id.clone(), t.clone()))
        .collect();
    let mut ordered: Vec<Track> = Vec::with_capacity(filtered.len());
    let mut seen = HashSet::new();
    for id in &plan.ranked_track_ids {
        if seen.insert(id.clone()) {
            if let Some(t) = pool.remove(id) {
                ordered.push(t);
            }
        }
    }
    for t in &filtered {
        if pool.contains_key(&t.id) {
            pool.remove(&t.id);
            ordered.push(t.clone());
        }
    }
    if ordered.len() >= filtered.len().min(3) {
        ordered
    } else {
        filtered
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MetaSuggestion {
    pub track_id: String,
    pub genre: String,
    pub year: i32,
    pub moods: Vec<String>,
}

/// A background-built, AI-verified instant mix: an ordered list of real track
/// ids drawn only from editorial sources, plus the model's title/subtitle.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct VerifiedMix {
    pub track_ids: Vec<String>,
    pub title: String,
    pub subtitle: String,
    pub note: String,
    pub built_at: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MoodCatalogChange {
    pub renames: HashMap<String, String>,
    pub subtitles: HashMap<String, String>,
    pub remove: Vec<String>,
    pub add: Vec<MoodSuggestion>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MoodSuggestion {
    pub slug: String,
    pub label: String,
    pub subtitle: String,
    pub c1: String,
    pub c2: String,
}

/// LLM resolution of a descriptive/lyrical search query into concrete
/// song and artist names. Cached so live search only reads SQLite.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SearchAnswer {
    pub tracks: Vec<SearchTrackRef>,
    pub artists: Vec<String>,
    pub note: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SearchTrackRef {
    pub title: String,
    pub artist: String,
}

/// A short result from the optional background web-research tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// A cached "preference plan" for any ranking surface (radio, made-for-you,
/// Explore rails, search, …). Generated in the background and applied at
/// request time with zero model latency. `ranked_ids` may refer to tracks,
/// artists, albums or genres depending on the scope.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RankPlan {
    pub ranked_ids: Vec<String>,
    pub extra_queries: Vec<String>,
    /// Optional one-line reason per ranked id (used by curated-artist cards).
    pub notes: BTreeMap<String, String>,
    pub title: String,
    pub subtitle: String,
    pub generated_at: i64,
}

fn plan_key(scope: &str, context: &str) -> String {
    format!("{}{}:{}", PLAN_CACHE_PREFIX, slug(scope), slug(context))
}

/// Apply a cached plan to an arbitrary list. Every input item is preserved —
/// planned items come first in the model's order, everything else follows in
/// its original order. A plan that matches fewer than three items is ignored.
pub fn apply_plan<T, F>(plan: &RankPlan, items: Vec<T>, id_of: F) -> Vec<T>
where
    F: Fn(&T) -> String,
{
    let len = items.len();
    if plan.ranked_ids.is_empty() || len < 2 {
        return items;
    }
    let entries: Vec<(String, T)> = items.into_iter().map(|i| (id_of(&i), i)).collect();
    let mut by_id: HashMap<String, usize> = HashMap::new();
    for (idx, (id, _)) in entries.iter().enumerate() {
        by_id.entry(id.clone()).or_insert(idx);
    }
    let matches: usize = plan
        .ranked_ids
        .iter()
        .filter(|id| by_id.contains_key(id.as_str()))
        .count();
    let mut slots: Vec<Option<T>> = entries.into_iter().map(|(_, t)| Some(t)).collect();
    if matches < 2 {
        return slots.into_iter().flatten().collect();
    }
    let mut ordered = Vec::with_capacity(len);
    let mut used = HashSet::new();
    for id in &plan.ranked_ids {
        if let Some(&idx) = by_id.get(id) {
            if used.insert(idx) {
                if let Some(t) = slots[idx].take() {
                    ordered.push(t);
                }
            }
        }
    }
    for slot in slots.into_iter().flatten() {
        ordered.push(slot);
    }
    ordered
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Cached<T> {
    at: i64,
    value: T,
}

#[derive(Clone)]
pub struct LlmClient {
    config: Arc<LlmConfig>,
    store: Store,
    http: reqwest::Client,
    /// HTTP/1.1-only fallback with keep-alive disabled, used when the primary
    /// client fails to read a body (broken chunking / premature close by a proxy).
    http1: reqwest::Client,
    research_http: reqwest::Client,
    stats: Arc<Mutex<LlmStats>>,
}

impl LlmClient {
    pub fn new(store: Store) -> Self {
        let config = Arc::new(LlmConfig::from_env());
        // Ask for plain responses explicitly: some OpenAI-compatible proxies
        // send broken Content-Encoding headers, which makes reqwest's decoder
        // fail with "error decoding response body" before we see any JSON.
        let http = reqwest::Client::builder()
            .timeout(Duration::from_millis(config.timeout_ms))
            .default_headers({
                let mut h = reqwest::header::HeaderMap::new();
                h.insert(
                    reqwest::header::ACCEPT_ENCODING,
                    reqwest::header::HeaderValue::from_static("identity"),
                );
                h
            })
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        // HTTP/1.1-only fallback: some proxies mishandle HTTP/2 body framing
        // and keep-alive reuse, producing "error decoding response body".
        let http1 = reqwest::Client::builder()
            .timeout(Duration::from_millis(config.timeout_ms))
            .http1_only()
            .pool_max_idle_per_host(0)
            .default_headers({
                let mut h = reqwest::header::HeaderMap::new();
                h.insert(
                    reqwest::header::ACCEPT_ENCODING,
                    reqwest::header::HeaderValue::from_static("identity"),
                );
                h.insert(
                    reqwest::header::CONNECTION,
                    reqwest::header::HeaderValue::from_static("close"),
                );
                h
            })
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        let stats = store
            .kv_get(STATS_KV_KEY)
            .and_then(|raw| serde_json::from_str::<LlmStats>(&raw).ok())
            .unwrap_or_default();
        if config.enabled {
            info!(
                model = %config.model,
                endpoint = %config.base_url,
                "AI curator enabled — LLM mix & curation tasks will run in the background"
            );
        }
        let research_http = reqwest::Client::builder()
            .timeout(Duration::from_millis(config.research_timeout_ms))
            .default_headers({
                let mut h = reqwest::header::HeaderMap::new();
                h.insert(
                    reqwest::header::ACCEPT_ENCODING,
                    reqwest::header::HeaderValue::from_static("identity"),
                );
                h
            })
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            config,
            store,
            http,
            http1,
            research_http,
            stats: Arc::new(Mutex::new(stats)),
        }
    }

    pub fn enabled(&self) -> bool {
        self.config.enabled
            && !self.config.api_key.trim().is_empty()
            && !self.config.base_url.trim().is_empty()
    }

    pub fn config_snapshot(&self) -> Value {
        let c = &self.config;
        json!({
            "enabled": c.enabled,
            "configured": !c.api_key.trim().is_empty() && !c.base_url.trim().is_empty(),
            "baseUrl": c.base_url,
            "model": c.model,
            "apiKeySet": !c.api_key.trim().is_empty(),
            "apiKeyMasked": mask_key(&c.api_key),
            "timeoutMs": c.timeout_ms,
            "maxTokens": c.max_tokens,
            "temperature": c.temperature,
            "researchEnabled": c.research_enabled && c.enabled,
            "searchUrl": c.search_url_template,
            "researchResults": c.research_max_results,
        })
    }

    pub fn stats_snapshot(&self) -> Value {
        let s = self.stats.lock().unwrap_or_else(|e| e.into_inner());
        let success_rate = if s.calls_total > 0 {
            (s.calls_ok as f64 / s.calls_total as f64 * 1000.0).round() / 10.0
        } else {
            0.0
        };
        let avg_latency = if s.calls_total > 0 {
            s.total_latency_ms / s.calls_total
        } else {
            0
        };
        let mut by_task: Vec<Value> = s
            .by_task
            .iter()
            .map(|(task, calls)| json!({ "task": task, "calls": calls }))
            .collect();
        by_task.sort_by(|a, b| {
            b.get("calls")
                .and_then(Value::as_u64)
                .cmp(&a.get("calls").and_then(Value::as_u64))
        });
        json!({
            "enabled": self.enabled(),
            "model": self.config.model,
            "endpoint": self.config.base_url,
            "callsTotal": s.calls_total,
            "callsOk": s.calls_ok,
            "callsFailed": s.calls_failed,
            "successRate": success_rate,
            "promptTokens": s.prompt_tokens,
            "completionTokens": s.completion_tokens,
            "totalTokens": s.prompt_tokens + s.completion_tokens,
            "avgLatencyMs": avg_latency,
            "lastLatencyMs": s.last_latency_ms,
            "lastCallAt": s.last_call_at,
            "lastOkAt": s.last_ok_at,
            "lastError": s.last_error,
            "lastErrorAt": s.last_error_at,
            "cacheWrites": s.cache_writes,
            "researchCalls": s.research_calls,
            "researchCacheHits": s.research_cache_hits,
            "researchFailures": s.research_failures,
            "verifiedMixes": self.store.kv_count_prefix("llm:verified_mix:v1:"),
            "searchPlans": self.store.kv_count_prefix("llm:plan:v1:search:"),
            "catalogChangeCached": self.store.kv_get("llm:mood_catalog:v1").is_some(),
            "pendingQueries": self
                .store
                .kv_get(PENDING_Q_KV_KEY)
                .and_then(|raw| serde_json::from_str::<Vec<(String, String)>>(&raw).ok())
                .map(|v| v.len())
                .unwrap_or(0),
            "byTask": by_task,
            "recent": s.recent,
            "config": self.config_snapshot(),
        })
    }

    fn record_call(
        &self,
        task: &str,
        ok: bool,
        ms: u64,
        prompt_tokens: u64,
        completion_tokens: u64,
        error: &str,
    ) {
        let mut s = self.stats.lock().unwrap_or_else(|e| e.into_inner());
        let now = Utc::now().timestamp();
        s.calls_total += 1;
        if ok {
            s.calls_ok += 1;
            s.last_ok_at = now;
        } else {
            s.calls_failed += 1;
            s.last_error = error.chars().take(600).collect();
            s.last_error_at = now;
        }
        s.prompt_tokens += prompt_tokens;
        s.completion_tokens += completion_tokens;
        s.total_latency_ms += ms;
        s.last_latency_ms = ms;
        s.last_call_at = now;
        *s.by_task.entry(task.to_string()).or_insert(0) += 1;
        s.recent.insert(
            0,
            LlmCallLog {
                task: task.to_string(),
                ok,
                ms,
                at: now,
                error: error.chars().take(600).collect(),
            },
        );
        s.recent.truncate(20);
        if let Ok(raw) = serde_json::to_string(&*s) {
            self.store.kv_set(STATS_KV_KEY, &raw);
        }
    }

    fn bump_cache_writes(&self) {
        let mut s = self.stats.lock().unwrap_or_else(|e| e.into_inner());
        s.cache_writes += 1;
        if let Ok(raw) = serde_json::to_string(&*s) {
            self.store.kv_set(STATS_KV_KEY, &raw);
        }
    }

    fn cache_get<T: DeserializeOwned>(&self, key: &str, ttl_secs: i64) -> Option<T> {
        let raw = self.store.kv_get(key)?;
        let cached: Cached<T> = serde_json::from_str(&raw).ok()?;
        if ttl_secs > 0 && Utc::now().timestamp() - cached.at > ttl_secs {
            return None;
        }
        Some(cached.value)
    }

    fn cache_set<T: Serialize>(&self, key: &str, value: &T) {
        let payload = Cached {
            at: Utc::now().timestamp(),
            value,
        };
        if let Ok(raw) = serde_json::to_string(&payload) {
            self.store.kv_set(key, &raw);
        }
    }

    /// One OpenAI-compatible chat completion. Always returns parsed JSON, or
    /// `None` — failures are counted but never propagated into a user request.
    pub async fn chat_json(
        &self,
        task: &str,
        system: &str,
        user: &str,
    ) -> Option<Value> {
        self.chat_json_attempt(task, system, user, true, true).await
    }

    async fn chat_json_attempt(
        &self,
        task: &str,
        system: &str,
        user: &str,
        allow_retry: bool,
        json_mode: bool,
    ) -> Option<Value> {
        if !self.enabled() {
            return None;
        }
        // Brief global backoff after a failure. All callers are background
        // tasks, so this only prevents hammering an unavailable endpoint and
        // never affects a user-visible request.
        {
            let s = self.stats.lock().unwrap_or_else(|e| e.into_inner());
            if s.calls_failed > 0
                && Utc::now().timestamp().saturating_sub(s.last_error_at) < 30
            {
                return None;
            }
        }
        let started = std::time::Instant::now();
        let mut body = json!({
            "model": self.config.model,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": user },
            ],
            "temperature": self.config.temperature,
        });
        // Uncapped by default: a cap is only sent when explicitly configured.
        if self.config.max_tokens > 0 {
            body["max_tokens"] = json!(self.config.max_tokens);
        }
        // JSON mode is on by default, but some cheap/free endpoints reject it
        // by returning an empty 200 body — retry without it if needed.
        if json_mode {
            body["response_format"] = json!({ "type": "json_object" });
        }
        let url = format!("{}/chat/completions", self.config.base_url.trim_end_matches('/'));
        let send = |client: &reqwest::Client| {
            client
                .post(&url)
                .header(
                    "Authorization",
                    format!("Bearer {}", self.config.api_key.trim()),
                )
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
        };

        // Preferred path: our own HTTP/1.1 client with a lenient chunked parser.
        // This avoids hyper's decoder entirely, which is what has been failing.
        let request_started = std::time::Instant::now();
        match raw_http_post(&self.config, &url, &body).await {
            Ok(text) if !text.trim().is_empty() => {
                let elapsed = request_started.elapsed().as_millis() as u64;
                return self.finish_response(task, &text, elapsed, json_mode, allow_retry, system, user);
            }
            Ok(_) => {}
            Err(raw_err) => {
                warn!(task = %task, error = %raw_err, "raw HTTP attempt failed; falling back");
            }
        }

        let result = send(&self.http).await;

        let elapsed = started.elapsed().as_millis() as u64;
        let response = match result {
            Ok(r) => r,
            Err(e) => {
                warn!(task = %task, error = %e, "AI curator request failed");
                self.record_call(task, false, elapsed, 0, 0, &e.to_string());
                return None;
            }
        };
        let status = response.status();
        let describe = describe_response(&response);
        // Partial-body salvage: many proxies send a short JSON error object
        // and then close without the terminating zero-length chunk. hyper
        // reports that as "error decoding response body", but the payload is
        // already in our buffer — keep it and surface the real API message.
        let (raw, partial_err) = match read_response_body(response).await {
            Ok(text) => (text, None),
            Err(primary) if !primary.text.trim().is_empty() => (primary.text, Some(primary.error)),
            Err(primary) => {
                // Nothing salvageable — retry once over HTTP/1.1 with
                // keep-alive disabled to rule out connection reuse.
                match send(&self.http1).await {
                    Ok(retry) => match read_response_body(retry).await {
                        Ok(text) => (text, None),
                        Err(second) if !second.text.trim().is_empty() => {
                            (second.text, Some(second.error))
                        }
                        Err(second) => {
                            let detail = format!("body read failed: primary={} | http1={} | {describe}",
                primary.error, second.error);
                            warn!(task = %task, error = %detail, "AI curator call failed");
                            self.record_call(task, false, elapsed, 0, 0, &detail);
                            return None;
                        }
                    },
                    Err(e) => {
                        let detail = format!(
                            "body read failed: primary={} | http1 send failed: {e} | {describe}",
                            primary.error
                        );
                        warn!(task = %task, error = %detail, "AI curator call failed");
                        self.record_call(task, false, elapsed, 0, 0, &detail);
                        return None;
                    }
                }
            }
        };
        if let Some(err) = partial_err {
            // Put the body FIRST: the admin UI truncates details, and the body
            // is the single most useful piece of evidence.
            let preview: String = raw.trim().chars().take(200).collect();
            let detail = if preview.is_empty() {
                format!("body unreadable: {err}")
            } else {
                match extract_api_error(&raw) {
                    Some(provider_msg) => {
                        format!("body={preview} || provider error: {provider_msg}")
                    }
                    None => format!("body={preview} || {err}"),
                }
            };
            warn!(task = %task, error = %detail, "AI curator call failed");
            self.record_call(task, false, elapsed, 0, 0, &detail);
            return None;
        };
        if raw.trim().is_empty() {
            // An empty 200 usually means the endpoint rejected response_format
            // or the model/provider isn't actually serving the request.
            let err = format!(
                "empty response body (status {status}, jsonMode={json_mode}, {} bytes)",
                raw.len()
            );
            if allow_retry && json_mode {
                return Box::pin(self.chat_json_attempt(task, system, user, false, false)).await;
            }
            warn!(task = %task, error = %err, "AI curator returned empty body");
            self.record_call(task, false, elapsed, 0, 0, &err);
            return None;
        }
        if !status.is_success() {
            let err = format!("HTTP {status}: {}", raw.chars().take(200).collect::<String>());
            warn!(task = %task, error = %err, "AI curator returned an error");
            self.record_call(task, false, elapsed, 0, 0, &err);
            return None;
        }
        let parsed: Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => {
                let err = format!(
                    "invalid response (status {status}, jsonMode={json_mode}): {e}; body: {}",
                    truncate(&raw, 160)
                );
                if allow_retry {
                    return Box::pin(self.chat_json_attempt(task, system, user, false, false)).await;
                }
                self.record_call(task, false, elapsed, 0, 0, &err);
                return None;
            }
        };
        let (prompt_tokens, completion_tokens) = parsed
            .get("usage")
            .map(|u| {
                (
                    u.get("prompt_tokens").and_then(Value::as_u64).unwrap_or(0),
                    u.get("completion_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                )
            })
            .unwrap_or((0, 0));
        let content = message_text(&parsed);
        match extract_json_object(&content) {
            Some(obj) => {
                self.record_call(
                    task,
                    true,
                    elapsed,
                    prompt_tokens,
                    completion_tokens,
                    "",
                );
                Some(obj)
            }
            None => {
                // Cheap endpoints often ignore the format instruction. Give
                // them one explicit second chance with their own bad reply
                // pasted back before counting a failure.
                if allow_retry {
                    let strict_system = format!(
                        "{system}\n\nCRITICAL: Reply with EXACTLY one valid JSON object. \
                         No prose, no markdown fences, no explanation of any kind."
                    );
                    let preview: String = content.chars().take(220).collect();
                    let strict_user = format!(
                        "{user}\n\nYour previous reply was not valid JSON. This is what you said:\n\
                         {preview}\n\nReply with ONLY the corrected JSON object now."
                    );
                    return Box::pin(self.chat_json_attempt(
                        task,
                        &strict_system,
                        &strict_user,
                        false,
                        json_mode,
                    ))
                    .await;
                }
                let err = format!("not JSON: {}", truncate(&content, 180));
                warn!(task = %task, detail = %err, "AI curator response could not be parsed");
                self.record_call(
                    task,
                    false,
                    elapsed,
                    prompt_tokens,
                    completion_tokens,
                    &err,
                );
                None
            }
        }
    }

    /// Shared tail for both transports: validate the body, parse the OpenAI
    /// envelope, extract the JSON object, and record the outcome.
    fn finish_response(
        &self,
        task: &str,
        raw: &str,
        elapsed: u64,
        json_mode: bool,
        _allow_retry: bool,
        _system: &str,
        _user: &str,
    ) -> Option<Value> {
        let raw = raw.to_string();
        if raw.trim().is_empty() {
            let err = format!("empty response body (jsonMode={json_mode})");
            warn!(task = %task, error = %err, "AI curator returned empty body");
            self.record_call(task, false, elapsed, 0, 0, &err);
            return None;
        }
        let parsed: Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => {
                let err = format!(
                    "invalid response: {e}; body: {}",
                    truncate(&raw, 200)
                );
                self.record_call(task, false, elapsed, 0, 0, &err);
                return None;
            }
        };
        if let Some(api_err) = extract_api_error(&raw) {
            let err = format!("provider error: {api_err}");
            warn!(task = %task, error = %err, "AI curator request rejected");
            self.record_call(task, false, elapsed, 0, 0, &err);
            return None;
        }
        let (prompt_tokens, completion_tokens) = parsed
            .get("usage")
            .map(|u| {
                (
                    u.get("prompt_tokens").and_then(Value::as_u64).unwrap_or(0),
                    u.get("completion_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                )
            })
            .unwrap_or((0, 0));
        let content = message_text(&parsed);
        match extract_json_object(&content) {
            Some(obj) => {
                self.record_call(
                    task,
                    true,
                    elapsed,
                    prompt_tokens,
                    completion_tokens,
                    "",
                );
                Some(obj)
            }
            None => {
                let err = format!("not JSON: {}", truncate(&content, 200));
                warn!(task = %task, detail = %err, "AI curator response could not be parsed");
                self.record_call(
                    task,
                    false,
                    elapsed,
                    prompt_tokens,
                    completion_tokens,
                    &err,
                );
                None
            }
        }
    }
    /// Queue a query for the slow background path, and also fire an immediate
    /// background call so the user sees the AI engage on this very search
    /// instead of waiting for the next curator tick.
    pub fn queue_query(&self, normalized: &str) {
        if !self.enabled() || normalized.len() < 2 || normalized.len() > 80 {
            return;
        }
        let query = normalized.trim().to_string();
        let mut list: Vec<String> = self
            .store
            .kv_get(PENDING_Q_KV_KEY)
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        if !list.iter().any(|q| q.eq_ignore_ascii_case(&query)) {
            list.push(query.clone());
            if list.len() > 64 {
                list.remove(0);
            }
            if let Ok(raw) = serde_json::to_string(&list) {
                self.store.kv_set(PENDING_Q_KV_KEY, &raw);
            }
        }
        // Immediate, detached background work: resolve intent + build a mix.
        let client = self.clone();
        tokio::spawn(async move {
            let _ = client.resolve_search_intent(&query).await;
            let _ = client.discover_seed_artists(&query).await;
        });
    }

    pub fn take_pending_queries(&self, n: usize) -> Vec<String> {
        let mut list: Vec<String> = self
            .store
            .kv_get(PENDING_Q_KV_KEY)
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        if list.is_empty() {
            return Vec::new();
        }
        let take = n.min(list.len());
        let picked: Vec<String> = list.drain(..take).collect();
        if let Ok(raw) = serde_json::to_string(&list) {
            self.store.kv_set(PENDING_Q_KV_KEY, &raw);
        }
        picked
    }

    pub fn cached_verified_mix(&self, query: &str) -> Option<VerifiedMix> {
        if !self.enabled() {
            return None;
        }
        self.cache_get(
            &format!("{}{}", VERIFIED_MIX_PREFIX, slug(query)),
            7 * 24 * 3600,
        )
    }

    pub fn save_verified_mix(&self, query: &str, mix: &VerifiedMix) {
        self.cache_set(
            &format!("{}{}", VERIFIED_MIX_PREFIX, slug(query)),
            mix,
        );
        self.bump_cache_writes();
    }

    pub fn cached_mix(&self, query: &str) -> Option<MixPlan> {
        if !self.enabled() {
            return None;
        }
        self.cache_get(&format!("{}{}", MIX_CACHE_PREFIX, slug(query)), MIX_CACHE_TTL_SECS)
    }

    /// Background-only: produce an LLM mix plan for the given candidates and
    /// cache it. The live request never awaits this call.
    pub async fn enhance_mix(&self, query: &str, tracks: &[Track]) -> Option<MixPlan> {
        if !self.enabled() || tracks.is_empty() {
            return None;
        }
        let key = format!("{}{}", MIX_CACHE_PREFIX, slug(query));
        if let Some(existing) = self.cache_get::<MixPlan>(&key, MIX_CACHE_TTL_SECS) {
            return Some(existing);
        }
        // One attempt per query per 10 minutes — request-driven mix building
        // must never turn into a stream of model calls for an unresponsive API.
        let attempt_key = format!("llm:mix_attempt:v5:{}", slug(query));
        if let Some(raw) = self.store.kv_get(&attempt_key) {
            if let Ok(ts) = raw.trim().parse::<i64>() {
                if Utc::now().timestamp().saturating_sub(ts) < 600 {
                    return None;
                }
            }
        }
        self.store
            .kv_set(&attempt_key, &Utc::now().timestamp().to_string());
        let listing = track_lines(tracks, 24);
        let system = "You are a strict music curator for a self-hosted library. Decide which of the \
            supplied tracks genuinely fit the listener's request and throw out the rest. \
            Return ONLY a JSON object with keys: \
            rejectedTrackIds (array of ids that are NOT a good fit — unrelated genre, wrong era, \
            novelty/phonk/tribute/cover versions, title-word collisions, or low-quality noise), \
            rankedTrackIds (array of the remaining good ids, best first), \
            title (short mix title), subtitle (one vivid sentence), \
            extraQueries (1-3 short search phrases that would find MORE genuinely on-theme tracks). \
            Only use ids from the supplied list, never invent one. Be aggressive: a focused mix of 8 \
            correct tracks is far better than 24 loosely related ones.";
        let user = format!(
            "Listener request: {query}\n\nCandidate tracks (each id is exact):\n{listing}\n\nJSON:"
        );
        let value = self.chat_json("mix", system, &user).await?;
        let plan = MixPlan::from_value(&value, tracks)?;
        self.cache_set(&key, &plan);
        self.bump_cache_writes();
        Some(plan)
    }

    /// Background-only: suggest genre / year / mood metadata for tracks whose
    /// tags are missing. Results are cached per track so a failed persist can
    /// be retried without another paid call.
    pub async fn enrich_metadata(&self, tracks: &[Track]) -> Vec<MetaSuggestion> {
        if !self.enabled() || tracks.is_empty() {
            return Vec::new();
        }
        let listing = tracks
            .iter()
            .take(12)
            .enumerate()
            .map(|(i, t)| {
                format!(
                    "{}. id={} | title={} | artist={} | album={} | source={}",
                    i + 1,
                    t.id,
                    truncate(&t.title, 80),
                    truncate(&primary_artist(&t.artist), 60),
                    truncate(&t.album, 60),
                    t.source
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let system = "You tag music metadata. Return ONLY a JSON object with key \"tracks\": an array \
            with one object per input track in the same order, containing trackId (exact id from input), \
            genre (one or two lowercase comma-separated genres, or empty), year (4-digit integer or 0 if unknown), \
            moods (array of 1-3 lowercase mood slugs from: chill, focus, energy, late-night, upbeat, \
            melancholy, party, acoustic, groove, rock-indie). If you are unsure about a release year or \
            genre, you may instead return {\"needResearch\": [\"short search query\"], \"tracks\": []} \
            with at most two queries and the system will look it up and ask you again. Be conservative, \
            never guess a year, and only use ids from the input.";
        let user = format!("Tracks:\n{listing}\n\nJSON:");
        let Some(mut value) = self.chat_json("metadata", system, &user).await else {
            return Vec::new();
        };
        if let Some(researched) = self
            .research_followup(&value, system, &user, "metadata_research")
            .await
        {
            value = researched;
        }
        let allowed: HashSet<&str> = tracks.iter().map(|t| t.id.as_str()).collect();
        let mut out = Vec::new();
        if let Some(arr) = value.get("tracks").and_then(Value::as_array) {
            for item in arr {
                let Some(id) = item.get("trackId").and_then(Value::as_str) else {
                    continue;
                };
                if !allowed.contains(id) {
                    continue;
                }
                let cache_key = format!("{}{}", META_CACHE_PREFIX, slug(id));
                let suggestion = MetaSuggestion {
                    track_id: id.to_string(),
                    genre: truncate(item.get("genre").and_then(Value::as_str).unwrap_or(""), 80),
                    year: item
                        .get("year")
                        .and_then(Value::as_i64)
                        .filter(|y| (1900..=2099).contains(y))
                        .unwrap_or(0) as i32,
                    moods: item
                        .get("moods")
                        .and_then(Value::as_array)
                        .map(|a| {
                            a.iter()
                                .filter_map(Value::as_str)
                                .map(|m| m.trim().to_ascii_lowercase().replace(' ', "-"))
                                .filter(|m| !m.is_empty())
                                .take(3)
                                .collect()
                        })
                        .unwrap_or_default(),
                };
                self.cache_set(&cache_key, &suggestion);
                out.push(suggestion);
            }
        }
        self.bump_cache_writes();
        out
    }

    /// Background-only: choose the strongest track ids for a mood / decade /
    /// query pool. Cached by label so repeated ticks cost nothing.
    pub async fn curate_pool(&self, label: &str, query: &str, tracks: &[Track]) -> Option<Vec<String>> {
        if !self.enabled() || tracks.is_empty() {
            return None;
        }
        let key = format!("{}{}", POOL_CACHE_PREFIX, slug(label));
        if let Some(ids) = self.cache_get::<Vec<String>>(&key, MIX_CACHE_TTL_SECS) {
            return Some(ids);
        }
        let listing = track_lines(tracks, 36);
        let system = "You curate a music mix with high precision. Return ONLY a JSON object with \
            key \"trackIds\": an array of ids chosen ONLY from the supplied list, best first. \
            Prefer 8-20 genuinely on-theme ids — return fewer rather than padding with weak matches. \
            Add \"title\" and \"subtitle\" for the mix. Reject covers, karaoke, slowed/reverb, \
            phonk/novelty edits, tracks with unrelated foreign-language titles when the theme is \
            specific, and anything whose genre or scene does not match the theme. Never invent an id.";
        let user = format!("Mix theme: {query}\n\nCandidates:\n{listing}\n\nJSON:");
        let value = self.chat_json("pool", system, &user).await?;
        let allowed: HashSet<&str> = tracks.iter().map(|t| t.id.as_str()).collect();
        let ids: Vec<String> = value
            .get("trackIds")
            .or_else(|| value.get("rankedTrackIds"))
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .filter(|id| allowed.contains(*id))
                    .take(40)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        if ids.len() < 5 {
            return None;
        }
        self.cache_set(&key, &ids);
        self.bump_cache_writes();
        Some(ids)
    }

    /// Background-only: alternate search phrases for a query. Used to warm
    /// curated pools that live search can read instantly later.
    pub async fn query_expansions(&self, query: &str) -> Vec<String> {
        if !self.enabled() {
            return Vec::new();
        }
        let key = format!("{}{}", QUERY_EXP_PREFIX, slug(query));
        if let Some(v) = self.cache_get::<Vec<String>>(&key, 30 * 24 * 3600) {
            return v;
        }
        let system = "You expand music search queries. Return ONLY a JSON object with key \
            \"queries\": an array of 1-3 concise alternate search phrases (genre, era, mood or \
            playlist-style) that would find more music matching the listener's intent.";
        let user = format!("Query: {query}\n\nJSON:");
        let Some(value) = self.chat_json("query", system, &user).await else {
            return Vec::new();
        };
        let queries: Vec<String> = value
            .get("queries")
            .or_else(|| value.get("extraQueries"))
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(|q| q.trim().chars().take(80).collect::<String>())
                    .filter(|q| !q.is_empty())
                    .take(3)
                    .collect()
            })
            .unwrap_or_default();
        self.cache_set(&key, &queries);
        self.bump_cache_writes();
        queries
    }

    /// Background-only: propose renames, new moods and removals for the
    /// Moods & Vibes catalog. Results are cached for a week so the catalog
    /// doesn't churn on every curator tick.
    pub async fn curate_mood_catalog(
        &self,
        current: &[crate::models::MoodDef],
        scope: &str,
        taste_summary: &str,
    ) -> Option<MoodCatalogChange> {
        if !self.enabled() || current.is_empty() {
            return None;
        }
        let key = format!("llm:mood_catalog:v1:{scope}");
        if let Some(cached) = self.cache_get::<MoodCatalogChange>(&key, 7 * 24 * 3600) {
            return Some(cached);
        }
        let catalog_json = serde_json::to_string(current).unwrap_or_default();
        let system = "You design the Moods & Vibes sections of a music app. Review the current \
            catalog and propose improvements: clearer labels/subtitles, up to 3 genuinely new \
            moods the library could support (with hex colors), and moods that are redundant or \
            useless. Keep at least 5 moods total and at most 16. Return ONLY a JSON object with \
            keys: renames (object slug -> new label), subtitles (object slug -> new subtitle), \
            remove (array of slugs), add (array of {slug,label,subtitle,c1,c2}). Slugs must be \
            lowercase ascii letters/digits/underscore, max 24 chars. Only use slugs that exist \
            in the catalog for renames/subtitles/remove.";
        let user = format!(
            "Listener profile: {taste_summary}\n\nCurrent catalog:\n{catalog_json}\n\nJSON:"
        );
        let value = self.chat_json("mood_catalog", system, &user).await?;
        let strings = |key: &str| -> HashMap<String, String> {
            value
                .get(key)
                .and_then(Value::as_object)
                .map(|obj| {
                    obj.iter()
                        .filter_map(|(k, v)| {
                            let text = truncate(v.as_str().unwrap_or("").trim(), 80);
                            (!text.is_empty()).then(|| (k.clone(), text))
                        })
                        .collect()
                })
                .unwrap_or_default()
        };
        let valid_slug = |s: &str| {
            !s.is_empty()
                && s.len() <= 24
                && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        };
        let change = MoodCatalogChange {
            renames: strings("renames"),
            subtitles: strings("subtitles"),
            remove: value
                .get("remove")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .filter(|s| valid_slug(s))
                        .take(4)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
            add: value
                .get("add")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|item| {
                            let slug = item.get("slug").and_then(Value::as_str).unwrap_or("").trim().to_string();
                            if !valid_slug(&slug) {
                                return None;
                            }
                            let label = truncate(item.get("label").and_then(Value::as_str).unwrap_or("").trim(), 50);
                            if label.is_empty() {
                                return None;
                            }
                            let hex = |k: &str| {
                                let v = item.get(k).and_then(Value::as_str).unwrap_or("").trim();
                                if v.len() == 7 && v.starts_with('#') && v[1..].chars().all(|c| c.is_ascii_hexdigit()) {
                                    v.to_string()
                                } else {
                                    "#6366f1".to_string()
                                }
                            };
                            Some(MoodSuggestion {
                                slug,
                                label,
                                subtitle: truncate(item.get("subtitle").and_then(Value::as_str).unwrap_or("").trim(), 120),
                                c1: hex("c1"),
                                c2: hex("c2"),
                            })
                        })
                        .take(3)
                        .collect()
                })
                .unwrap_or_default(),
        };
        // Never cache an empty result: a free model often returns prose or an
        // empty object, and caching that would freeze the catalog for a week.
        if change.renames.is_empty()
            && change.subtitles.is_empty()
            && change.remove.is_empty()
            && change.add.is_empty()
        {
            return None;
        }
        self.cache_set(&key, &change);
        self.bump_cache_writes();
        Some(change)
    }

    pub fn cached_search_answer(&self, query: &str) -> Option<SearchAnswer> {
        if !self.enabled() {
            return None;
        }
        self.cache_get(
            &format!("{}{}", SEARCH_ANSWER_PREFIX, slug(query)),
            30 * 24 * 3600,
        )
    }

    /// Background-only: turn a descriptive search ("that Queen song about the
    /// poor boy from a poor family") into concrete song/artist names.
    pub async fn resolve_search_intent(&self, query: &str) -> Option<SearchAnswer> {
        if !self.enabled() || query.trim().is_empty() {
            return None;
        }
        let key = format!("{}{}", SEARCH_ANSWER_PREFIX, slug(query));
        if let Some(cached) = self.cache_get::<SearchAnswer>(&key, 30 * 24 * 3600) {
            return Some(cached);
        }
        let system = "You resolve music searches. The listener may describe a song by its lyrics, \
            story, mood or scene instead of its title. Work out the exact song(s) and artist(s) \
            they mean. Return ONLY a JSON object with keys: tracks (array of {title, artist}, at \
            most 3), artists (array of at most 3 exact artist names), note (one short sentence \
            explaining the match). Example: 'queen song poor boy from poor family' -> \
            Bohemian Rhapsody by Queen. Use research if genuinely unsure.";
        let user = format!("Search request: {query}\n\nJSON:");
        let Some(mut value) = self.chat_json("search_intent", system, &user).await else {
            return None;
        };
        if let Some(researched) = self
            .research_followup(&value, system, &user, "search_intent_research")
            .await
        {
            value = researched;
        }
        let tracks: Vec<SearchTrackRef> = value
            .get("tracks")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|item| {
                        let title = truncate(item.get("title").and_then(Value::as_str).unwrap_or("").trim(), 100);
                        let artist = truncate(item.get("artist").and_then(Value::as_str).unwrap_or("").trim(), 100);
                        (!title.is_empty() && !artist.is_empty()).then(|| SearchTrackRef { title, artist })
                    })
                    .take(3)
                    .collect()
            })
            .unwrap_or_default();
        let artists: Vec<String> = value
            .get("artists")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(|s| truncate(s.trim(), 100))
                    .filter(|s| !s.is_empty())
                    .take(3)
                    .collect()
            })
            .unwrap_or_default();
        if tracks.is_empty() && artists.is_empty() {
            return None;
        }
        let answer = SearchAnswer {
            tracks,
            artists,
            note: truncate(
                value.get("note").and_then(Value::as_str).unwrap_or("").trim(),
                160,
            ),
        };
        self.cache_set(&key, &answer);
        self.bump_cache_writes();
        Some(answer)
    }

    /// Background-only: name real artists/songs that genuinely fit a theme.
    /// Used to pull actual on-theme tracks into a mix pool instead of asking
    /// the model to reorder whatever noisy playlist search returned.
    pub async fn discover_seed_artists(&self, query: &str) -> Vec<String> {
        if !self.enabled() || query.trim().is_empty() {
            return Vec::new();
        }
        let key = format!("llm:seed:v1:{}", slug(query));
        if let Some(cached) = self.cache_get::<Vec<String>>(&key, 7 * 24 * 3600) {
            return cached;
        }
        let system = "You are a music researcher. Name REAL, well-known artists that genuinely \
            belong to the given music theme or scene. For 'anime', name actual anime opening/ending \
            artists (e.g. Yoasobi, LiSA, Kenshi Yonezu, Asian Kung-Fu Generation) — never tracks \
            that merely use anime-style artwork or put the word 'anime' in the title. \
            Return ONLY a JSON object with keys: artists (8-12 exact artist names, each at least \
            two words or a distinctive single name), songs (optional 4-6 {title,artist} objects), \
            and optionally needResearch (1-2 short queries if unsure). Use your own knowledge first.";
        let user = format!("Theme: {query}\n\nJSON:");
        let Some(mut value) = self.chat_json("seed", system, &user).await else {
            return Vec::new();
        };
        if let Some(researched) = self
            .research_followup(&value, system, &user, "seed_research")
            .await
        {
            value = researched;
        }
        let mut names = Vec::new();
        if let Some(items) = value.get("artists").and_then(Value::as_array) {
            for item in items.iter().filter_map(Value::as_str) {
                let name = truncate(item.trim(), 80);
                if !name.is_empty() && !names.contains(&name) {
                    names.push(name);
                }
            }
        }
        if let Some(items) = value.get("songs").and_then(Value::as_array) {
            for item in items {
                let title = item.get("title").and_then(Value::as_str).unwrap_or("").trim();
                let artist = item.get("artist").and_then(Value::as_str).unwrap_or("").trim();
                let name = truncate(&format!("{title} {artist}"), 100);
                if !title.is_empty() && !artist.is_empty() && !names.contains(&name) {
                    names.push(name);
                }
            }
        }
        names.truncate(14);
        if names.is_empty() {
            return Vec::new();
        }
        self.cache_set(&key, &names);
        self.bump_cache_writes();
        names
    }

    /// Build an entire verified mix from a clean editorial candidate pool.
    /// Background-only; the live endpoint just reads the cached result.
    pub async fn curate_verified_mix(
        &self,
        query: &str,
        candidates: &[Track],
    ) -> Option<VerifiedMix> {
        if !self.enabled() || candidates.len() < 8 {
            return None;
        }
        if let Some(cached) = self.cached_verified_mix(query) {
            return Some(cached);
        }
        let listing = track_lines(candidates, 40);
        let system = "You are a music curator. The candidate list comes from editorial genre \
            charts and curated playlists, not keyword search. Choose the tracks that genuinely \
            fit the listener's request and discard the rest. Return ONLY a JSON object with keys: \
            rankedTrackIds (8-20 exact ids from the list, best first), title (a short mix title), \
            subtitle (one vivid sentence), note (one short sentence on what ties the picks together). \
            Never invent an id. Prefer a tight, coherent selection over a long one.";
        let user = format!("Listener request: {query}\n\nCandidates:\n{listing}\n\nJSON:");
        let value = self.chat_json("verified_mix", system, &user).await?;
        let allowed: HashSet<&str> = candidates.iter().map(|t| t.id.as_str()).collect();
        let track_ids: Vec<String> = value
            .get("rankedTrackIds")
            .or_else(|| value.get("trackIds"))
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .filter(|id| allowed.contains(*id))
                    .take(20)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        if track_ids.len() < 8 {
            return None;
        }
        let mix = VerifiedMix {
            track_ids,
            title: truncate(value.get("title").and_then(Value::as_str).unwrap_or("").trim(), 80),
            subtitle: truncate(value.get("subtitle").and_then(Value::as_str).unwrap_or("").trim(), 160),
            note: truncate(value.get("note").and_then(Value::as_str).unwrap_or("").trim(), 160),
            built_at: Utc::now().timestamp(),
        };
        self.save_verified_mix(query, &mix);
        Some(mix)
    }

    /// Drops a cached instant-mix plan so the next request regenerates it from
    /// an improved candidate pool.
    pub fn invalidate_mix(&self, query: &str) {
        self.store
            .kv_set(&format!("{}{}", MIX_CACHE_PREFIX, slug(query)), "");
    }

    // ---- Optional background web research ---------------------------------

    /// Keyless web search used only by background LLM tasks when the model
    /// says it is unsure. Results are cached for a week and never fetched on
    /// a user request path.
    pub async fn web_search(&self, query: &str) -> Vec<WebResult> {
        let query = query.trim();
        if !self.enabled() || !self.config.research_enabled || query.is_empty() {
            return Vec::new();
        }
        let query: String = query.chars().take(120).collect();
        let cache_key = format!("llm:research:v1:{}", slug(&query));
        if let Some(cached) = self.cache_get::<Vec<WebResult>>(&cache_key, 7 * 24 * 3600) {
            let mut s = self.stats.lock().unwrap_or_else(|e| e.into_inner());
            s.research_cache_hits += 1;
            return cached;
        }
        {
            let mut s = self.stats.lock().unwrap_or_else(|e| e.into_inner());
            s.research_calls += 1;
        }
        let url = self
            .config
            .search_url_template
            .replace("{q}", &urlencoding::encode(&query));
        let html = match self.research_http.get(&url).send().await {
            Ok(r) if r.status().is_success() => r.text().await.unwrap_or_default(),
            Ok(r) => {
                let mut s = self.stats.lock().unwrap_or_else(|e| e.into_inner());
                s.research_failures += 1;
                warn!(status = %r.status(), "AI web research request failed");
                return Vec::new();
            }
            Err(e) => {
                let mut s = self.stats.lock().unwrap_or_else(|e| e.into_inner());
                s.research_failures += 1;
                warn!(error = %e, "AI web research request failed");
                return Vec::new();
            }
        };
        let results = parse_search_html(&html, self.config.research_max_results);
        self.cache_set(&cache_key, &results);
        self.bump_cache_writes();
        results
    }

    /// Ask the model once more with research notes when it explicitly asks
    /// for help. At most two queries per plan, one research round total.
    async fn research_followup(
        &self,
        value: &Value,
        system: &str,
        original_user: &str,
        task: &str,
    ) -> Option<Value> {
        if !self.config.research_enabled {
            return None;
        }
        let questions: Vec<String> = value
            .get("needResearch")
            .or_else(|| value.get("researchQueries"))
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(|q| q.trim().chars().take(120).collect::<String>())
                    .filter(|q| !q.is_empty())
                    .take(2)
                    .collect()
            })
            .unwrap_or_default();
        if questions.is_empty() {
            return None;
        }
        let mut notes = String::new();
        for q in questions {
            for result in self.web_search(&q).await {
                notes.push_str(&format!(
                    "- {}: {} ({})\n",
                    truncate(&result.title, 140),
                    truncate(&result.snippet, 260),
                    truncate(&result.url, 160),
                ));
            }
        }
        if notes.trim().is_empty() {
            return None;
        }
        let followed_up = format!(
            "{original_user}\n\nWeb research notes (verify against your own knowledge, they may be incomplete or stale):\n{notes}\n\nReturn the FINAL JSON now, using the same keys as before."
        );
        self.chat_json(task, system, &followed_up).await
    }

    // ---- Generic preference plans -----------------------------------------

    pub fn cached_plan(&self, scope: &str, context: &str) -> Option<RankPlan> {
        if !self.enabled() {
            return None;
        }
        self.cache_get(&plan_key(scope, context), PLAN_TTL_SECS)
    }

    /// Drops a cached plan so the next background pass generates a fresh one.
    /// Used when a daily mix is rebuilt.
    pub fn invalidate_plan(&self, scope: &str, context: &str) {
        self.store.kv_set(&plan_key(scope, context), "");
    }

    pub fn plan_text(&self, scope: &str, context: &str) -> Option<(String, String)> {
        let plan = self.cached_plan(scope, context)?;
        let title = plan.title.trim().to_string();
        let subtitle = plan.subtitle.trim().to_string();
        if title.is_empty() && subtitle.is_empty() {
            None
        } else {
            Some((title, subtitle))
        }
    }

    /// Enqueue a plan for background generation. Tiny SQLite write; safe to
    /// call from request handlers when no cached plan exists yet.
    pub fn queue_plan(&self, scope: &str, context: &str) {
        if !self.enabled() || scope.trim().is_empty() {
            return;
        }
        let scope = scope.trim().to_string();
        let context = context.trim().to_string();
        let mut list: Vec<(String, String)> = self
            .store
            .kv_get(PLAN_PENDING_KV_KEY)
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        if list
            .iter()
            .any(|(s, c)| s == &scope && c == &context)
        {
            return;
        }
        list.push((scope, context));
        if list.len() > 256 {
            list.remove(0);
        }
        if let Ok(raw) = serde_json::to_string(&list) {
            self.store.kv_set(PLAN_PENDING_KV_KEY, &raw);
        }
    }

    pub fn take_pending_plans(&self, n: usize) -> Vec<(String, String)> {
        let mut list: Vec<(String, String)> = self
            .store
            .kv_get(PLAN_PENDING_KV_KEY)
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        if list.is_empty() {
            return Vec::new();
        }
        let take = n.min(list.len());
        let picked = list.drain(..take).collect();
        if let Ok(raw) = serde_json::to_string(&list) {
            self.store.kv_set(PLAN_PENDING_KV_KEY, &raw);
        }
        picked
    }

    /// Background-only: generate a ranking plan for one surface from a set of
    /// candidates. The model only ever reorders ids supplied to it.
    pub async fn generate_plan(
        &self,
        scope: &str,
        context: &str,
        candidates: &[Track],
        hint: &str,
    ) -> Option<RankPlan> {
        if !self.enabled() || candidates.len() < 3 {
            return None;
        }
        let key = plan_key(scope, context);
        if let Some(existing) = self.cache_get::<RankPlan>(&key, PLAN_TTL_SECS) {
            return Some(existing);
        }
        let attempt_key = format!(
            "llm:plan_attempt:v1:{}",
            slug(&format!("{scope}:{context}"))
        );
        if let Some(raw) = self.store.kv_get(&attempt_key) {
            if let Ok(ts) = raw.trim().parse::<i64>() {
                if Utc::now().timestamp().saturating_sub(ts) < 600 {
                    return None;
                }
            }
        }
        self.store
            .kv_set(&attempt_key, &Utc::now().timestamp().to_string());

        let listing = track_lines(candidates, 32);
        let system = "You are the personal curation engine of a music app. Rank the supplied items \
            for the given surface and listener. Return ONLY a JSON object with keys: \
            rankedIds (array of exact ids from the supplied list only, best first, at most 40), \
            title (a short display title for the mix/section, or empty), \
            subtitle (one short line describing the selection, or empty), \
            extraQueries (1-3 short search phrases that would find more items like this), \
            notes (an object mapping each id you ranked to one short sentence explaining why it fits). \
            If you are unsure about a factual detail that affects the ranking (an artist's era, \
            a genre's origins, what fits a theme), you may instead return {\"needResearch\": \
            [\"short search query\"], \"rankedIds\": [], \"title\": \"\", \"subtitle\": \"\"} \
            with at most two queries; the system will look it up and ask you again. Use this \
            sparingly. Never invent or guess an id. Prefer coherence over raw popularity.";
        let user = format!(
            "Surface: {scope}\nListener/context: {context}\nGoal: {hint}\n\nItems:\n{listing}\n\nJSON:"
        );
        let mut value = self.chat_json("plan", system, &user).await?;
        if let Some(researched) = self
            .research_followup(&value, system, &user, "plan_research")
            .await
        {
            value = researched;
        }
        let allowed: HashSet<&str> = candidates.iter().map(|t| t.id.as_str()).collect();
        let ranked_ids: Vec<String> = value
            .get("rankedIds")
            .or_else(|| value.get("ranked_ids"))
            .or_else(|| value.get("trackIds"))
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .filter(|id| allowed.contains(*id))
                    .take(40)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        if ranked_ids.len() < 3 {
            return None;
        }
        let extra_queries: Vec<String> = value
            .get("extraQueries")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(|q| truncate(q.trim(), 80))
                    .filter(|q| !q.is_empty())
                    .take(3)
                    .collect()
            })
            .unwrap_or_default();
        let notes: BTreeMap<String, String> = value
            .get("notes")
            .and_then(Value::as_object)
            .map(|obj| {
                obj.iter()
                    .filter(|(id, _)| allowed.contains(id.as_str()))
                    .map(|(id, text)| {
                        (
                            id.clone(),
                            truncate(
                                text.as_str().unwrap_or("").trim(),
                                160,
                            ),
                        )
                    })
                    .filter(|(_, text)| !text.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        let plan = RankPlan {
            ranked_ids,
            extra_queries,
            notes,
            title: truncate(
                value.get("title").and_then(Value::as_str).unwrap_or(""),
                80,
            ),
            subtitle: truncate(
                value.get("subtitle").and_then(Value::as_str).unwrap_or(""),
                140,
            ),
            generated_at: Utc::now().timestamp(),
        };
        self.cache_set(&key, &plan);
        self.bump_cache_writes();
        Some(plan)
    }
}

fn truncate(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

fn html_text(raw: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for ch in raw.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out.replace("&nbsp;", " ")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn between(hay: &str, start: &str, end: &str) -> Option<String> {
    let a = hay.find(start)? + start.len();
    let b = hay[a..].find(end)? + a;
    Some(hay[a..b].to_string())
}

/// Parses DuckDuckGo-style search HTML (and reasonably similar markup such as
/// a self-hosted SearXNG page) into short, bounded snippets for the model.
fn parse_search_html(html: &str, max: usize) -> Vec<WebResult> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while out.len() < max {
        let Some(rel) = html[pos..].find("result-link") else {
            break;
        };
        let start = pos + rel;
        let end = (start + 2400).min(html.len());
        let chunk = &html[start..end];
        let Some(raw_url) = between(chunk, "href=\"", "\"")
            .or_else(|| between(chunk, "href='", "'"))
        else {
            pos = start + 11;
            continue;
        };
        let url = html_text(&raw_url);
        let anchor = chunk.find('>').map(|i| &chunk[i + 1..]).unwrap_or("");
        let title_end = anchor.find("</a>").unwrap_or(anchor.len().min(300));
        let title = html_text(&anchor[..title_end]);
        let snippet = chunk
            .find("result-snippet")
            .and_then(|s| {
                let c = &chunk[s..];
                let body = c.find('>').map(|i| &c[i + 1..])?;
                let e = body.find("</").unwrap_or(body.len().min(420));
                Some(html_text(&body[..e]))
            })
            .unwrap_or_default();
        if !url.is_empty()
            && (url.starts_with("http") || url.starts_with("//"))
            && !title.is_empty()
        {
            out.push(WebResult {
                title,
                url,
                snippet,
            });
        }
        pos = start + 11;
    }
    if out.is_empty() {
        // Generic <a href="...">text</a> fallback for custom search templates.
        let mut scan = 0usize;
        while out.len() < max {
            let Some(rel) = html[scan..].find("<a ") else {
                break;
            };
            let start = scan + rel;
            let end = (start + 1200).min(html.len());
            let chunk = &html[start..end];
            let (Some(raw_url), Some(body)) = (
                between(chunk, "href=\"", "\"").or_else(|| between(chunk, "href='", "'")),
                chunk.find('>').map(|i| &chunk[i + 1..]),
            ) else {
                scan = start + 3;
                continue;
            };
            let url = html_text(&raw_url);
            let title_end = body.find("</a>").unwrap_or(body.len().min(300));
            let title = html_text(&body[..title_end]);
            if (url.starts_with("http") || url.starts_with("//")) && !title.is_empty() {
                out.push(WebResult {
                    title,
                    url,
                    snippet: String::new(),
                });
            }
            scan = start + 3;
        }
    }
    out
}

fn track_lines(tracks: &[Track], max: usize) -> String {
    tracks
        .iter()
        .take(max)
        .enumerate()
        .map(|(i, t)| {
            format!(
                "{}. id={} | title={} | artist={} | album={} | genre={} | year={}",
                i + 1,
                t.id,
                truncate(&t.title, 70),
                truncate(&primary_artist(&t.artist), 50),
                truncate(&t.album, 50),
                truncate(&t.genre, 40),
                t.year,
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn mask_key(key: &str) -> String {
    let k = key.trim();
    if k.len() <= 8 {
        return if k.is_empty() { "—".to_string() } else { "••••".to_string() };
    }
    format!("{}…{}", &k[..4], &k[k.len() - 4..])
}

/// Split a URL into (is_tls, host, port, path).
fn parse_http_url(url: &str) -> Result<(bool, String, u16, String), String> {
    let (is_tls, rest) = if let Some(r) = url.strip_prefix("https://") {
        (true, r)
    } else if let Some(r) = url.strip_prefix("http://") {
        (false, r)
    } else {
        return Err(format!("unsupported URL: {url}"));
    };
    let (authority, path) = match rest.find('/') {
        Some(idx) => (&rest[..idx], &rest[idx..]),
        None => (rest, "/"),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) => {
            (h.to_string(), p.parse::<u16>().unwrap_or(if is_tls { 443 } else { 80 }))
        }
        _ => (
            authority.to_string(),
            if is_tls { 443 } else { 80 },
        ),
    };
    Ok((is_tls, host, port, path.to_string()))
}

/// Decode a chunked body leniently. Tolerates missing/extra whitespace around
/// chunk sizes, chunk extensions, and a missing terminating zero-length chunk —
/// all of which make strict decoders (like hyper's) fail.
fn decode_chunked(buf: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos < buf.len() {
        // Find end of the chunk-size line.
        let line_end = match find_crlf(buf, pos) {
            Some(idx) => idx,
            None => break,
        };
        let size_line = String::from_utf8_lossy(&buf[pos..line_end]);
        let size_token = size_line
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .trim_start_matches("0x")
            .trim_start_matches("0X");
        let size = match usize::from_str_radix(size_token, 16) {
            Ok(s) => s,
            Err(_) => break, // not chunked after all
        };
        pos = line_end + 2;
        if size == 0 {
            break; // terminal chunk
        }
        let end = (pos + size).min(buf.len());
        out.extend_from_slice(&buf[pos..end]);
        pos = end;
        // Skip the CRLF that follows chunk data (may be absent if truncated).
        if pos < buf.len() && buf[pos] == b'\r' {
            pos += 1;
            if pos < buf.len() && buf[pos] == b'\n' {
                pos += 1;
            }
        } else if pos < buf.len() && buf[pos] == b'\n' {
            pos += 1;
        }
        // Stop if we've consumed everything without a terminal chunk.
        if pos >= buf.len() {
            break;
        }
    }
    out
}

fn find_crlf(buf: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while i + 1 < buf.len() {
        if buf[i] == b'\r' && buf[i + 1] == b'\n' {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Raw HTTPS POST with a hand-rolled HTTP/1.1 + chunked reader.
///
/// This exists because some providers (notably behind Cloudflare) return
/// `Transfer-Encoding: chunked` bodies that hyper rejects with
/// "error decoding response body". Speaking HTTP/1.1 directly and parsing the
/// framing leniently sidesteps that entire class of failure.
async fn raw_http_post(
    config: &LlmConfig,
    url: &str,
    body: &Value,
) -> Result<String, String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let (is_tls, host, port, path) = parse_http_url(url)?;
    let payload = serde_json::to_string(body).map_err(|e| e.to_string())?;

    let mut request = String::new();
    request.push_str(&format!("POST {path} HTTP/1.1\r\n"));
    request.push_str(&format!("Host: {host}\r\n"));
    request.push_str("User-Agent: Beatlab/0.3\r\n");
    request.push_str("Content-Type: application/json\r\n");
    request.push_str("Accept: application/json\r\n");
    request.push_str("Accept-Encoding: identity\r\n");
    request.push_str("Connection: close\r\n");
    request.push_str(&format!(
        "Authorization: Bearer {}\r\n",
        config.api_key.trim()
    ));
    request.push_str(&format!("Content-Length: {}\r\n", payload.len()));
    request.push_str("\r\n");
    request.push_str(&payload);

    let mut raw: Vec<u8> = Vec::new();
    if is_tls {
        let tcp = tokio::net::TcpStream::connect((host.as_str(), port))
            .await
            .map_err(|e| format!("tcp connect failed: {e}"))?;
        let connector = tls_connector()?;
        let server_name: rustls_pki_types::ServerName<'static> =
            rustls_pki_types::ServerName::try_from(host.to_string())
                .map_err(|_| format!("invalid host: {host}"))?;
        let mut tls = connector
            .connect(server_name, tcp)
            .await
            .map_err(|e| format!("tls handshake failed: {e}"))?;
        tls.write_all(request.as_bytes())
            .await
            .map_err(|e| format!("tls write failed: {e}"))?;
        tls.flush().await.map_err(|e| format!("tls flush failed: {e}"))?;
        tls.read_to_end(&mut raw)
            .await
            .map_err(|e| format!("tls read failed after {} bytes: {e}", raw.len()))?;
    } else {
        let mut tcp = tokio::net::TcpStream::connect((host.as_str(), port))
            .await
            .map_err(|e| format!("tcp connect failed: {e}"))?;
        tcp.write_all(request.as_bytes())
            .await
            .map_err(|e| format!("write failed: {e}"))?;
        tcp.read_to_end(&mut raw)
            .await
            .map_err(|e| format!("read failed after {} bytes: {e}", raw.len()))?;
    }

    parse_http_response(&raw)
}

fn tls_connector() -> Result<tokio_rustls::TlsConnector, String> {
    use rustls::RootCertStore;
    let mut roots = RootCertStore::empty();
    // webpki-roots provides `TrustAnchor`s, which RootCertStore accepts directly.
    roots.extend(
        webpki_roots::TLS_SERVER_ROOTS
            .iter()
            .map(|ta| ta.to_owned()),
    );
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(tokio_rustls::TlsConnector::from(std::sync::Arc::new(
        config,
    )))
}

/// Parse a raw HTTP/1.1 response into its body text.
fn parse_http_response(raw: &[u8]) -> Result<String, String> {
    let header_end = find_subslice(raw, b"\r\n\r\n")
        .ok_or_else(|| "response has no header terminator".to_string())?;
    let head = String::from_utf8_lossy(&raw[..header_end]);
    let body = &raw[header_end + 4..];

    let mut chunked = false;
    for line in head.lines().skip(1) {
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("transfer-encoding")
                && value.to_ascii_lowercase().contains("chunked")
            {
                chunked = true;
            }
        }
    }
    let decoded;
    let body: &[u8] = if chunked {
        decoded = decode_chunked(body);
        &decoded
    } else {
        body
    };
    Ok(String::from_utf8_lossy(body).into_owned())
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Describes the response framing so a failed read is actually diagnosable
/// instead of showing a generic "error decoding response body".
fn describe_response(response: &reqwest::Response) -> String {
    let hdr = |name: reqwest::header::HeaderName| {
        response
            .headers()
            .get(&name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("-")
            .chars()
            .take(40)
            .collect::<String>()
    };
    format!(
        "status={} ver={:?} content-encoding={} content-length={} transfer-encoding={} content-type={} server={}",
        response.status(),
        response.version(),
        hdr(reqwest::header::CONTENT_ENCODING),
        hdr(reqwest::header::CONTENT_LENGTH),
        hdr(reqwest::header::TRANSFER_ENCODING),
        hdr(reqwest::header::CONTENT_TYPE),
        hdr(reqwest::header::SERVER),
    )
}

/// A body plus the transport error that interrupted it, if any. Keeping both
/// lets us surface a provider error message instead of a generic decode error.
struct PartialBody {
    text: String,
    error: String,
}

/// Pull a human-readable message out of a provider error payload.
fn extract_api_error(text: &str) -> Option<String> {
    let value: Value = serde_json::from_str(text).ok()?;
    let error = value.get("error")?;
    if let Some(msg) = error.as_str() {
        return Some(truncate(msg, 200));
    }
    let msg = error
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| error.get("code").and_then(Value::as_str))
        .unwrap_or("");
    if msg.is_empty() {
        None
    } else {
        Some(truncate(msg, 200))
    }
}

/// Read a response body as raw bytes and decode it ourselves.
///
/// Streams the body chunk-by-chunk instead of using `.bytes()`, so a proxy that
/// closes the connection early still yields whatever JSON arrived rather than
/// discarding the whole response with "error decoding response body".
async fn read_response_body(response: reqwest::Response) -> Result<String, PartialBody> {
    use futures::StreamExt;
    use std::io::Read;

    let describe = describe_response(&response);
    let mut buf: Vec<u8> = Vec::new();
    let mut stream_error: Option<String> = None;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(bytes) => buf.extend_from_slice(&bytes),
            Err(e) => {
                // Keep what we already have; a complete JSON object may still
                // be present before the truncation point.
                stream_error = Some(e.to_string());
                break;
            }
        }
    }
    if buf.is_empty() {
        return Err(PartialBody {
            text: String::new(),
            error: match stream_error {
                Some(err) => format!("{err} ({describe})"),
                None => format!("no bytes received ({describe})"),
            },
        });
    }
    // Defensively gunzip if a proxy compressed anyway (0x1f 0x8b magic).
    if buf.len() >= 2 && buf[0] == 0x1f && buf[1] == 0x8b {
        let mut decoder = flate2::read::GzDecoder::new(&buf[..]);
        let mut decoded = Vec::new();
        match decoder.read_to_end(&mut decoded) {
            Ok(_) => buf = decoded,
            Err(e) => {
                return Err(PartialBody {
                    text: String::new(),
                    error: format!("gzip decode failed: {e} ({describe})"),
                })
            }
        }
    }
    let text = String::from_utf8_lossy(&buf).into_owned();
    match stream_error {
        Some(err) => Err(PartialBody {
            text,
            error: format!(
                "stream failed after {} bytes: {err} ({describe})",
                buf.len()
            ),
        }),
        None => Ok(text),
    }
}

/// Pull the model's text out of the many response shapes used by compatible
/// endpoints (plain string, array of content parts, completion-style `text`).
fn text_from_content(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| {
                p.as_str()
                    .map(str::to_string)
                    .or_else(|| p.get("text").and_then(Value::as_str).map(str::to_string))
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// Reasoning models frequently prefix their visible answer with a `<think>`
/// block (or use full-width variants). Left in place, braces inside the
/// reasoning text confuse JSON extraction — so strip complete blocks first.
fn strip_thinking(text: &str) -> String {
    const PAIRS: [(&str, &str); 6] = [
        ("<think>", "</think>"),
        ("<thinking>", "</thinking>"),
        ("<reasoning>", "</reasoning>"),
        ("\u{ff1c}think\u{ff1e}", "\u{ff1c}/think\u{ff1e}"),
        ("[THINK]", "[/THINK]"),
        ("<scratchpad>", "</scratchpad>"),
    ];
    let mut out = text.to_string();
    for (open, close) in PAIRS {
        loop {
            let Some(start) = out.find(open) else { break };
            let after_open = start + open.len();
            let Some(end_rel) = out[after_open..].find(close) else { break };
            let end = after_open + end_rel + close.len();
            out = format!("{}{}", &out[..start], &out[end..]);
        }
    }
    out
}

fn message_text(parsed: &Value) -> String {
    if let Some(choice) = parsed
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
    {
        let content = text_from_content(
            choice
                .get("message")
                .and_then(|m| m.get("content")),
        );
        if !content.trim().is_empty() {
            return strip_thinking(&content);
        }
        // Some proxies put the model's text in a reasoning-specific field.
        for key in ["reasoning_content", "reasoning", "thinking"] {
            let reasoning = text_from_content(
                choice.get("message").and_then(|m| m.get(key)),
            );
            if !reasoning.trim().is_empty() {
                return strip_thinking(&reasoning);
            }
        }
        if let Some(text) = choice.get("text").and_then(Value::as_str) {
            if !text.trim().is_empty() {
                return strip_thinking(text);
            }
        }
    }
    for key in ["output_text", "content", "result", "response"] {
        let text = text_from_content(parsed.get(key));
        if !text.trim().is_empty() {
            return strip_thinking(&text);
        }
    }
    String::new()
}

/// Heuristically close a JSON object the model cut off before its final brace
/// (very common when a completion hits its token limit).
fn repair_truncated_json(text: &str) -> Option<Value> {
    let start = text.find('{')?;
    let mut candidate = text[start..].trim_end().to_string();
    let bytes = candidate.as_bytes();
    let mut braces = 0i32;
    let mut brackets = 0i32;
    let mut in_string = false;
    let mut escaped = false;
    for &b in bytes {
        if in_string {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' => braces += 1,
            b'}' => braces -= 1,
            b'[' => brackets += 1,
            b']' => brackets -= 1,
            _ => {}
        }
    }
    if braces <= 0 && brackets <= 0 {
        return None;
    }
    // A truncated value string is the most common case — close it so the
    // appended braces produce a valid object.
    if in_string {
        candidate.push('"');
    }
    candidate = candidate.trim_end_matches(',').to_string();
    if candidate.ends_with(':') {
        candidate = candidate
            .trim_end_matches(':')
            .trim_end()
            .trim_end_matches(',')
            .to_string();
    }
    for _ in 0..brackets.max(0) {
        candidate.push(']');
    }
    for _ in 0..braces.max(0) {
        candidate.push('}');
    }
    serde_json::from_str::<Value>(&candidate)
        .ok()
        .filter(|v| v.is_object())
}

/// Robustly pull the first JSON object out of a model response, tolerating
/// markdown fences, chatty wrappers and truncation from cheap / free endpoints.
pub fn extract_json_object(text: &str) -> Option<Value> {
    let cleaned = strip_thinking(text);
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
        if v.is_object() {
            return Some(v);
        }
        // A bare array is a common shortcut from small models.
        if let Value::Array(items) = v {
            if items.iter().any(|i| i.get("trackId").is_some()) {
                return Some(json!({ "tracks": items }));
            }
            if !items.is_empty() && items.iter().all(Value::is_string) {
                return Some(json!({ "rankedIds": items }));
            }
        }
    }
    let unfenced = trimmed
        .trim_start_matches("```json")
        .trim_start_matches("```JSON")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    if let Ok(v) = serde_json::from_str::<Value>(unfenced) {
        if v.is_object() {
            return Some(v);
        }
    }
    let start = unfenced.find('{')?;
    let bytes = unfenced.as_bytes();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escaped = false;
    for (idx, &b) in bytes.iter().enumerate().skip(start) {
        if in_string {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    if let Ok(v) = serde_json::from_str::<Value>(&unfenced[start..=idx]) {
                        if v.is_object() {
                            return Some(v);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    repair_truncated_json(unfenced)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_json_from_fences_and_chatty_text() {
        let v = extract_json_object("```json\n{\"title\": \"Rainy Drive\"}\n```")
            .expect("fenced json");
        assert_eq!(v["title"], "Rainy Drive");

        let v = extract_json_object("Sure! Here you go: {\"a\": 1, \"nested\": {\"b\": \"}\"}} done")
            .expect("embedded json with brace in string");
        assert_eq!(v["a"], 1);
        assert_eq!(v["nested"]["b"], "}");

        assert!(extract_json_object("not json").is_none());
    }

    #[test]
    fn config_defaults_are_safe() {
        let c = LlmConfig::from_env();
        assert!(!c.base_url.is_empty());
        assert!(!c.model.is_empty());
        assert!(c.timeout_ms > 0);
    }

    #[test]
    fn lenient_chunked_decoder_handles_missing_terminator_and_extensions() {
        // Chunked with an extension and NO terminating zero-length chunk —
        // the framing that makes strict decoders fail.
        // Chunk 1: "4;foo=bar" -> 4 bytes `{"a"` ; Chunk 2: "6" -> 6 bytes `: 1}\r\n`
        let body = b"4;foo=bar\r\n{\"a\"\r\n6\r\n: 1}\r\n";
        let decoded = decode_chunked(body);
        assert_eq!(String::from_utf8_lossy(&decoded), "{\"a\": 1}\r\n");

        // Well-formed chunked body with terminator still works.
        let ok = b"5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n";
        assert_eq!(String::from_utf8_lossy(&decode_chunked(ok)), "hello world");
    }

    #[test]
    fn parses_http_response_with_chunked_framing() {
        // Chunk sizes are exact: 8 bytes `{"choice` + 5 bytes `s":1}` = 13 bytes.
        let raw = b"HTTP/1.1 200 OK\r\n\
                    Content-Type: application/json\r\n\
                    Transfer-Encoding: chunked\r\n\
                    Server: cloudflare\r\n\r\n\
                    8\r\n{\"choice\r\n\
                    5\r\ns\":1}\r\n";
        let body = parse_http_response(raw).expect("chunked response parses");
        assert_eq!(body, "{\"choices\":1}", "body was: {body}");
    }

    #[test]
    fn parses_alternate_content_shapes_and_truncated_json() {
        let parts = json!({ "choices": [{ "message": { "content": [
            { "type": "text", "text": "Here you go: {\"rankedIds\": [\"a\", \"b\"]}" }
        ] } }] });
        assert_eq!(message_text(&parts), "Here you go: {\"rankedIds\": [\"a\", \"b\"]}");

        let completion = json!({ "choices": [{ "text": "{\"title\": \"Test\"}" }] });
        assert_eq!(message_text(&completion), "{\"title\": \"Test\"}");

        let truncated = "{\"rankedIds\": [\"a\", \"b\"], \"title\": \"Night";
        let repaired = extract_json_object(truncated).expect("truncated json repaired");
        assert_eq!(repaired["rankedIds"][0], "a");
        assert_eq!(repaired["rankedIds"][1], "b");

        let bare = extract_json_object("[\"a\", \"b\"]").expect("bare array accepted");
        assert_eq!(bare["rankedIds"][1], "b");
    }

    #[test]
    fn strips_thinking_blocks_before_json_extraction() {
        let raw = "<think>The user wants a {\"wrong\": \"object\"} with this title.</think>\n\
                   Here is the result: {\"rankedIds\": [\"a\", \"b\"], \"title\": \"Night Drive\"}";
        let value = extract_json_object(raw).expect("json after thinking block");
        assert_eq!(value["title"], "Night Drive");
        assert_eq!(value["rankedIds"][0], "a");
        assert!(value.get("wrong").is_none());

        let with_field = json!({
            "choices": [{ "message": {
                "reasoning_content": "thinking... {\"bad\": 1}",
                "content": "<think>more thinking</think>{\"title\": \"Final\"}"
            }}]
        });
        assert_eq!(message_text(&with_field), "{\"title\": \"Final\"}");
    }

    #[test]
    fn parses_search_result_html_into_bounded_snippets() {
        let html = r#"<div class="result">
          <a rel="nofollow" class="result-link" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2F80s">80s Synthpop Guide</a>
          <div class="result-snippet">A guide to 80s synthpop &amp; new wave.</div>
        </div>"#;
        let results = parse_search_html(html, 5);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "80s Synthpop Guide");
        assert!(results[0].url.contains("example.com"));
        assert_eq!(results[0].snippet, "A guide to 80s synthpop & new wave.");
        assert!(parse_search_html("<html>no results</html>", 5).is_empty());
    }

    #[test]
    fn apply_plan_prioritizes_planned_items_without_dropping_any() {
        let items = vec!["a", "b", "c", "d", "e"];
        let plan = RankPlan {
            ranked_ids: vec!["c".into(), "a".into()],
            ..Default::default()
        };
        let result = apply_plan(&plan, items, |s| (*s).to_string());
        assert_eq!(result, vec!["c", "a", "b", "d", "e"]);
    }

    #[test]
    fn apply_plan_ignores_plans_with_too_few_matches() {
        let items = vec!["a", "b", "c", "d"];
        let plan = RankPlan {
            ranked_ids: vec!["zzz".into(), "a".into()],
            ..Default::default()
        };
        let result = apply_plan(&plan, items, |s| (*s).to_string());
        assert_eq!(result, vec!["a", "b", "c", "d"]);
    }

    #[test]
    fn mix_plan_drops_off_theme_tracks_but_keeps_a_floor() {
        let tracks: Vec<Track> = (0..10)
            .map(|i| Track {
                id: format!("t{i}"),
                ..Default::default()
            })
            .collect();
        let plan = MixPlan {
            rejected_track_ids: (0..8).map(|i| format!("t{i}")).collect(),
            ranked_track_ids: vec!["t9".into(), "t8".into()],
            ..Default::default()
        };
        let result = apply_mix_plan(&plan, tracks);
        // Floor is 8 tracks, so at most 2 of the 8 rejections survive.
        assert_eq!(result.len(), 8);
        assert_eq!(result[0].id, "t9");
        assert_eq!(result[1].id, "t8");
    }

    #[test]
    fn mix_plan_ignores_unknown_track_ids() {
        let tracks = vec![
            Track {
                id: "a".into(),
                ..Default::default()
            },
            Track {
                id: "b".into(),
                ..Default::default()
            },
        ];
        let raw = json!({
            "title": "Test Mix",
            "subtitle": "A subtitle",
            "rankedTrackIds": ["b", "made-up", "a"],
            "extraQueries": ["one"],
        });
        let plan = MixPlan::from_value(&raw, &tracks).expect("plan");
        assert_eq!(plan.ranked_track_ids, vec!["b", "a"]);
        let ordered = apply_mix_plan(&plan, tracks);
        assert_eq!(ordered.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), vec!["b", "a"]);
    }
}
