//! "Listen Together" lobbies.
//!
//! A signed-in host opens a lobby and shares its `/l/{code}` link. Anyone with
//! the link can join with just a display name (no Beatlab account) and hears
//! the host's playback in sync, with live chat and emoji reactions.
//!
//! Sync model: the host's player pushes a playback *anchor* — "track X was at
//! position P at server time T, playing/paused" — whenever something changes
//! (and on a heartbeat). Listeners project `P + (now - T)` against a
//! server-clock offset they measure themselves, then nudge their own `<audio>`
//! toward that point. Updates fan out over Server-Sent Events.
//!
//! Only the lobby row (code, host, settings) is persisted, so invite links
//! survive restarts; live state is rebuilt from the host's next push.

use crate::models::{LobbyRecord, Track};
use crate::state::AppState;
use crate::store::Store;
use crate::webapp::{
    cover_api_url, err_res, json_res, request_origin, require_web_user, resolve_web_user,
    ICON_512_PNG,
};
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet, VecDeque};
use std::convert::Infallible;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use tokio::sync::broadcast;

/// Hard cap on simultaneous listeners in one lobby.
pub const MAX_LISTENERS: usize = 50;
/// Emoji listeners can fire at the host; anything else is rejected.
pub const REACTIONS: &[&str] = &["🔥", "❤️", "🙌", "😂", "😮", "🎉"];

const CHAT_HISTORY: usize = 80;
const CHAT_MAX_CHARS: usize = 300;
const NAME_MAX_CHARS: usize = 24;
const LOBBY_NAME_MAX_CHARS: usize = 48;
const UP_NEXT_MAX: usize = 8;
const RECENT_MAX: usize = 6;
/// The host is "offline" once their player hasn't checked in for this long.
const HOST_STALE_MS: i64 = 35_000;
/// Browsers throttle timers in silent background tabs to ~1/minute, so a
/// paused host gets a longer leash before listeners are told they left.
const HOST_STALE_PAUSED_MS: i64 = 150_000;
/// A listener whose event stream dropped is kept this long so reloads and
/// brief network blips don't flicker them out of the list.
const LISTENER_GRACE_MS: i64 = 20_000;
/// Lobbies close themselves after the host has been gone this long.
const LOBBY_IDLE_END_MS: i64 = 6 * 3600 * 1000;
/// Idle lobbies with nobody connected are dropped from memory (not ended).
const LOBBY_EVICT_MS: i64 = 10 * 60 * 1000;
/// A host anchor that drifts less than this from our projection isn't rebroadcast.
const ANCHOR_TOLERANCE_MS: f64 = 400.0;
const CHAT_BURST: f64 = 5.0;
const CHAT_REFILL_MS: f64 = 2_000.0;
const REACT_MIN_GAP_MS: i64 = 350;
/// New listeners one IP address may add to a lobby per window (stops scripted
/// join floods; generous enough for a household sharing one connection).
const JOINS_PER_IP: usize = 8;
const JOIN_WINDOW_MS: i64 = 10 * 60 * 1000;
/// Cap on remembered "already announced" listener ids per lobby.
const WELCOMED_MAX: usize = 2_000;
const AVATAR_COLORS: &[&str] = &[
    "#f43f5e", "#8b5cf6", "#3b82f6", "#06b6d4", "#10b981", "#f59e0b", "#ec4899", "#6366f1",
    "#14b8a6", "#f97316",
];

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

// ---------------------------------------------------------------------------
// In-memory hub
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Outgoing {
    event: &'static str,
    data: Arc<str>,
    /// When set, only the listener with this private id receives the event.
    target: Option<Arc<str>>,
}

#[derive(Clone, Default)]
struct HostInfo {
    username: String,
    display_name: String,
    avatar_url: String,
    avatar_color: String,
    avatar_letter: String,
}

struct Listener {
    pid: String,
    name: String,
    /// Beatlab username when the listener is signed in; empty for guests.
    username: String,
    avatar_url: String,
    color: String,
    is_host: bool,
    joined_ms: i64,
    conns: u32,
    last_seen_ms: i64,
    /// Set by the page's unload beacon: drop them as soon as their stream
    /// closes instead of waiting out the reconnect grace period.
    leaving: bool,
    chat_tokens: f64,
    chat_refill_at: i64,
    last_react_ms: i64,
}

impl Listener {
    fn kind(&self) -> &'static str {
        if self.is_host {
            "host"
        } else if self.username.is_empty() {
            "guest"
        } else {
            "member"
        }
    }

    fn json(&self) -> Value {
        json!({
            "pid": self.pid,
            "name": self.name,
            "username": self.username,
            "avatarUrl": self.avatar_url,
            "color": self.color,
            "kind": self.kind(),
            "joinedAt": self.joined_ms,
        })
    }

    /// Token bucket: a short burst is fine, sustained spam is not.
    fn take_chat_token(&mut self, now: i64) -> bool {
        let elapsed = (now - self.chat_refill_at).max(0) as f64;
        self.chat_tokens = (self.chat_tokens + elapsed / CHAT_REFILL_MS).min(CHAT_BURST);
        self.chat_refill_at = now;
        if self.chat_tokens >= 1.0 {
            self.chat_tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

struct Lobby {
    code: String,
    host_id: i64,
    host: HostInfo,
    name: String,
    allow_chat: bool,
    locked: bool,
    created: i64,
    /// Which host tab/device is broadcasting; others get a "take over" prompt.
    host_client: String,
    host_seen_ms: i64,
    /// Whether the host's last report said audio was playing (drives staleness).
    host_reported_playing: bool,
    host_away: bool,
    host_offline_announced: bool,
    track: Option<Track>,
    position_ms: f64,
    playing: bool,
    buffering: bool,
    updated_ms: i64,
    up_next: Vec<Track>,
    /// Recently played ids, so listeners finishing a buffered stream aren't cut off.
    recent: VecDeque<String>,
    listeners: HashMap<String, Listener>,
    kicked: HashSet<String>,
    welcomed: HashSet<String>,
    /// Recent new-listener timestamps per client IP (join flood guard).
    joins_by_ip: HashMap<String, VecDeque<i64>>,
    chat: VecDeque<Value>,
    next_msg_id: u64,
    tx: broadcast::Sender<Outgoing>,
    last_db_touch: i64,
}

impl Lobby {
    fn new(rec: &LobbyRecord, host: HostInfo, now: i64) -> Self {
        let (tx, _) = broadcast::channel(256);
        // Freshly loaded (e.g. after a restart): the host counts as offline until
        // their player checks in, however recently the row was touched.
        let host_seen_ms = (rec.last_active * 1000).min(now - HOST_STALE_PAUSED_MS - 1);
        let mut lobby = Self {
            code: rec.code.clone(),
            host_id: rec.user_id,
            host,
            name: rec.name.clone(),
            allow_chat: rec.allow_chat,
            locked: rec.locked,
            created: rec.created,
            host_client: String::new(),
            host_seen_ms,
            host_reported_playing: false,
            host_away: false,
            host_offline_announced: false,
            track: None,
            position_ms: 0.0,
            playing: false,
            buffering: false,
            updated_ms: now,
            up_next: Vec::new(),
            recent: VecDeque::new(),
            listeners: HashMap::new(),
            kicked: HashSet::new(),
            welcomed: HashSet::new(),
            joins_by_ip: HashMap::new(),
            chat: VecDeque::new(),
            next_msg_id: 1,
            tx,
            last_db_touch: rec.last_active,
        };
        lobby.host_offline_announced = !lobby.host_online(now);
        lobby
    }

    fn host_online(&self, now: i64) -> bool {
        let limit = if self.host_reported_playing {
            HOST_STALE_MS
        } else {
            HOST_STALE_PAUSED_MS
        };
        now - self.host_seen_ms < limit
    }

    /// Whether listeners should currently be hearing audio advance.
    fn live_playing(&self, now: i64) -> bool {
        self.track.is_some()
            && self.playing
            && !self.buffering
            && !self.host_away
            && self.host_online(now)
    }

    /// Projected position of the anchor at server time `at`.
    fn position_at(&self, at: i64) -> f64 {
        let mut pos = self.position_ms;
        if self.playing && !self.buffering {
            pos += (at - self.updated_ms).max(0) as f64;
        }
        if let Some(t) = &self.track {
            if t.duration > 0 {
                pos = pos.min(t.duration as f64 * 1000.0);
            }
        }
        pos.max(0.0)
    }

    /// Freeze the anchor where the host's audio most likely stopped, so
    /// listeners pause in the right place instead of running ahead.
    fn freeze_for_offline(&mut self, now: i64) {
        self.position_ms = self.position_at(self.host_seen_ms.min(now));
        self.updated_ms = now;
        self.playing = false;
        self.buffering = false;
    }

    fn snapshot(&self, now: i64) -> Value {
        let mut listeners: Vec<&Listener> = self.listeners.values().collect();
        listeners.sort_by_key(|l| l.joined_ms);
        let live = self.live_playing(now);
        json!({
            "code": self.code,
            "name": self.name,
            "allowChat": self.allow_chat,
            "locked": self.locked,
            "createdAt": self.created * 1000,
            "host": {
                "username": self.host.username,
                "displayName": self.host.display_name,
                "avatarUrl": self.host.avatar_url,
                "avatarColor": self.host.avatar_color,
                "avatarLetter": self.host.avatar_letter,
                "online": self.host_online(now) && !self.host_away,
                "away": self.host_away,
            },
            "playback": {
                "track": self.track.as_ref().map(track_json),
                "positionMs": self.position_at(now).round(),
                "playing": live,
                "buffering": self.buffering && self.playing && self.host_online(now) && !self.host_away,
                "updatedAt": now,
            },
            "upNext": self.up_next.iter().map(track_json).collect::<Vec<_>>(),
            "listeners": listeners.iter().map(|l| l.json()).collect::<Vec<_>>(),
            "listenerCount": listeners.len(),
            "reactions": REACTIONS,
            "serverTime": now,
        })
    }

    fn emit(&self, event: &'static str, data: &Value, target: Option<&str>) {
        let _ = self.tx.send(Outgoing {
            event,
            data: Arc::from(data.to_string()),
            target: target.map(Arc::from),
        });
    }

    fn emit_state(&self, now: i64) {
        self.emit("state", &self.snapshot(now), None);
    }

    fn post_chat(&mut self, kind: &str, pid: &str, name: &str, color: &str, text: &str, now: i64) {
        let msg = json!({
            "id": self.next_msg_id,
            "kind": kind,
            "pid": pid,
            "name": name,
            "color": color,
            "text": text,
            "at": now,
        });
        self.next_msg_id += 1;
        self.chat.push_back(msg.clone());
        while self.chat.len() > CHAT_HISTORY {
            self.chat.pop_front();
        }
        self.emit("chat", &msg, None);
    }

    fn chat_json(&self) -> Vec<Value> {
        self.chat.iter().cloned().collect()
    }

    /// Records a new listener from `ip`; false once that IP hit its quota.
    fn allow_join_from(&mut self, ip: &str, now: i64) -> bool {
        self.joins_by_ip.retain(|_, times| {
            while times.front().is_some_and(|t| now - t > JOIN_WINDOW_MS) {
                times.pop_front();
            }
            !times.is_empty()
        });
        let times = self.joins_by_ip.entry(ip.to_string()).or_default();
        if times.len() >= JOINS_PER_IP {
            return false;
        }
        times.push_back(now);
        true
    }

    fn allows_stream_of(&self, track_id: &str) -> bool {
        self.track.as_ref().is_some_and(|t| t.id == track_id)
            || self.up_next.iter().take(3).any(|t| t.id == track_id)
            || self.recent.iter().any(|id| id == track_id)
    }
}

fn track_json(t: &Track) -> Value {
    json!({
        "id": t.id,
        "title": t.title,
        "artist": t.artist,
        "album": t.album,
        "year": t.year,
        "duration": t.duration,
        "format": t.format,
        "coverUrl": cover_api_url(&t.id),
        "qualityBadge": t.quality_badge(),
        "isLossless": t.is_lossless(),
    })
}

#[derive(Debug, PartialEq, Eq)]
enum Gone {
    Missing,
    Ended,
}

/// Shared registry of live lobbies, keyed by invite code.
#[derive(Clone, Default)]
pub struct LobbyHub {
    inner: Arc<Mutex<HashMap<String, Lobby>>>,
}

impl LobbyHub {
    fn lock(&self) -> MutexGuard<'_, HashMap<String, Lobby>> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Makes sure an open lobby is resident in memory, loading its row from the
    /// database after a restart. Closes lobbies whose host has been gone too long.
    fn ensure(&self, store: &Store, code: &str) -> Result<(), Gone> {
        if self.lock().contains_key(code) {
            return Ok(());
        }
        let rec = store
            .get_lobby(code)
            .ok()
            .flatten()
            .ok_or(Gone::Missing)?;
        if rec.ended {
            return Err(Gone::Ended);
        }
        let now = now_ms();
        if now - rec.last_active * 1000 > LOBBY_IDLE_END_MS {
            let _ = store.end_lobby(&rec.code);
            return Err(Gone::Ended);
        }
        let host = host_info(store, rec.user_id);
        self.lock()
            .entry(rec.code.clone())
            .or_insert_with(|| Lobby::new(&rec, host, now));
        Ok(())
    }

    fn with<R>(&self, code: &str, f: impl FnOnce(&mut Lobby) -> R) -> Option<R> {
        self.lock().get_mut(code).map(f)
    }

    fn snapshot_json(&self, code: &str) -> Option<String> {
        self.with(code, |l| l.snapshot(now_ms()).to_string())
    }

    /// Called when a listener's event stream ends; the sweeper prunes them
    /// after a grace period unless they reconnect.
    fn release(&self, code: &str, lid: &str) {
        self.with(code, |l| {
            let now = now_ms();
            let Some(listener) = l.listeners.get_mut(lid) else {
                return;
            };
            listener.conns = listener.conns.saturating_sub(1);
            listener.last_seen_ms = now;
            if listener.conns == 0 && listener.leaving {
                l.listeners.remove(lid);
                l.emit_state(now);
            }
        });
    }

    /// Periodic housekeeping: prune departed listeners, notice when the host
    /// drops off, close abandoned lobbies and evict idle ones from memory.
    fn sweep(&self, store: &Store) {
        let now = now_ms();
        let mut to_end = Vec::new();
        {
            let mut map = self.lock();
            map.retain(|code, lobby| {
                if now - lobby.host_seen_ms > LOBBY_IDLE_END_MS {
                    lobby.emit("ended", &json!({ "reason": "idle" }), None);
                    to_end.push(code.clone());
                    return false;
                }
                let before = lobby.listeners.len();
                lobby
                    .listeners
                    .retain(|_, l| {
                        l.conns > 0 || (!l.leaving && now - l.last_seen_ms < LISTENER_GRACE_MS)
                    });
                let mut changed = lobby.listeners.len() != before;
                if !lobby.host_offline_announced && !lobby.host_online(now) {
                    lobby.freeze_for_offline(now);
                    lobby.host_offline_announced = true;
                    changed = true;
                }
                if changed {
                    lobby.emit_state(now);
                }
                let idle = !lobby.host_online(now)
                    && lobby.listeners.is_empty()
                    && now - lobby.host_seen_ms > LOBBY_EVICT_MS
                    && lobby.tx.receiver_count() == 0;
                !idle
            });
        }
        for code in to_end {
            let _ = store.end_lobby(&code);
        }
    }
}

/// Runs lobby housekeeping every few seconds for the life of the server.
pub fn spawn_sweeper(hub: LobbyHub, store: Store) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        loop {
            tick.tick().await;
            hub.sweep(&store);
        }
    });
}

fn host_info(store: &Store, user_id: i64) -> HostInfo {
    match store.get_public_user(user_id, user_id).ok().flatten() {
        Some(u) => HostInfo {
            display_name: if u.display_name.trim().is_empty() {
                u.username.clone()
            } else {
                u.display_name.clone()
            },
            username: u.username,
            avatar_url: u.avatar_url,
            avatar_color: u.avatar_color,
            avatar_letter: u.avatar_letter,
        },
        None => HostInfo {
            username: "host".into(),
            display_name: "Host".into(),
            avatar_letter: "H".into(),
            ..Default::default()
        },
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Collapses whitespace, strips control/bidi-override characters and caps length.
fn clean_text(raw: &str, max_chars: usize) -> String {
    let filtered: String = raw
        .chars()
        .map(|c| {
            let invisible = matches!(c,
                '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}');
            if c.is_control() || invisible {
                ' '
            } else {
                c
            }
        })
        .collect();
    let joined = filtered.split_whitespace().collect::<Vec<_>>().join(" ");
    joined.chars().take(max_chars).collect::<String>().trim().to_string()
}

fn normalize_code(raw: &str) -> Option<String> {
    let code = raw.trim().to_ascii_lowercase();
    (!code.is_empty() && code.len() <= 16 && code.chars().all(|c| c.is_ascii_alphanumeric()))
        .then_some(code)
}

fn valid_lid(lid: &str) -> bool {
    (16..=64).contains(&lid.len())
        && lid.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Public listener id: stable per lobby, but never reveals the private `lid`
/// that authorises chat and streaming.
fn public_id(code: &str, lid: &str) -> String {
    let digest = Sha256::digest(format!("{code}:{lid}").as_bytes());
    hex::encode(digest)[..12].to_string()
}

fn color_for(pid: &str) -> String {
    let idx = pid
        .bytes()
        .fold(0usize, |acc, b| acc.wrapping_mul(31).wrapping_add(b as usize));
    AVATAR_COLORS[idx % AVATAR_COLORS.len()].to_string()
}

fn parse_body<T: DeserializeOwned>(body: &Bytes) -> Result<T, Response> {
    // Lenient about Content-Type so `navigator.sendBeacon` payloads parse too.
    serde_json::from_slice(body).map_err(|_| err_res(StatusCode::BAD_REQUEST, "Invalid request body"))
}

fn coded_err(status: StatusCode, code: &str, msg: &str) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        json!({ "error": msg, "code": code }).to_string(),
    )
        .into_response()
}

fn gone_res(g: Gone) -> Response {
    match g {
        Gone::Missing => coded_err(StatusCode::NOT_FOUND, "missing", "This lobby doesn't exist."),
        Gone::Ended => coded_err(StatusCode::GONE, "ended", "This lobby has ended."),
    }
}

fn no_store(mut resp: Response) -> Response {
    resp.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    resp
}

fn lobby_url(headers: &HeaderMap, code: &str) -> String {
    format!("{}/l/{}", request_origin(headers), code)
}

/// The signed-in user's open lobby, loaded into memory.
fn host_lobby_code(state: &AppState, user_id: i64) -> Option<String> {
    let rec = state.store.active_lobby_for_user(user_id).ok().flatten()?;
    state.lobbies.ensure(&state.store, &rec.code).ok()?;
    Some(rec.code)
}

fn host_view(state: &AppState, headers: &HeaderMap, code: &str, client_id: &str) -> Value {
    let url = lobby_url(headers, code);
    let qr_svg = crate::auth::qr_svg(&url).unwrap_or_default();
    let now = now_ms();
    state
        .lobbies
        .with(code, |l| {
            let mut v = l.snapshot(now);
            if let Some(obj) = v.as_object_mut() {
                obj.insert("url".into(), json!(url));
                obj.insert("path".into(), json!(format!("/l/{code}")));
                obj.insert("qrSvg".into(), json!(qr_svg));
                obj.insert("chat".into(), json!(l.chat_json()));
                let elsewhere = !l.host_client.is_empty()
                    && l.host_client != client_id
                    && l.host_online(now)
                    && !l.host_away;
                obj.insert("broadcastingElsewhere".into(), json!(elsewhere));
            }
            v
        })
        .unwrap_or(Value::Null)
}

// ---------------------------------------------------------------------------
// Event streams
// ---------------------------------------------------------------------------

/// Decrements the listener's connection count when their stream is dropped.
struct ConnGuard {
    hub: LobbyHub,
    code: String,
    lid: String,
}

impl Drop for ConnGuard {
    fn drop(&mut self) {
        self.hub.release(&self.code, &self.lid);
    }
}

struct StreamState {
    rx: broadcast::Receiver<Outgoing>,
    hub: LobbyHub,
    code: String,
    lid: Option<String>,
    init: Option<Event>,
    done: bool,
    _guard: Option<ConnGuard>,
}

fn event_stream(st: StreamState) -> Response {
    let stream = futures::stream::unfold(st, |mut st| async move {
        if let Some(ev) = st.init.take() {
            return Some((Ok::<Event, Infallible>(ev), st));
        }
        if st.done {
            return None;
        }
        loop {
            match st.rx.recv().await {
                Ok(out) => {
                    if let Some(target) = &out.target {
                        if st.lid.as_deref() != Some(&**target) {
                            continue;
                        }
                    }
                    if out.event == "ended" || out.event == "kicked" {
                        st.done = true;
                    }
                    let ev = Event::default().event(out.event).data(&*out.data);
                    return Some((Ok(ev), st));
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    // Too far behind to replay; a fresh snapshot is all a client needs.
                    let snap = st.hub.snapshot_json(&st.code)?;
                    return Some((Ok(Event::default().event("state").data(snap)), st));
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    });
    let mut resp = Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)).text("ping"))
        .into_response();
    let h = resp.headers_mut();
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache, no-transform"));
    h.insert("x-accel-buffering", HeaderValue::from_static("no"));
    resp
}

// ---------------------------------------------------------------------------
// Host API (signed in)
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ClientQuery {
    #[serde(default)]
    pub client_id: String,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct CreateReq {
    #[serde(default)]
    name: String,
    #[serde(default)]
    allow_chat: Option<bool>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct UpdateReq {
    name: Option<String>,
    allow_chat: Option<bool>,
    locked: Option<bool>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct HostStateReq {
    #[serde(default)]
    client_id: String,
    #[serde(default)]
    track_id: String,
    #[serde(default)]
    position_ms: f64,
    #[serde(default)]
    playing: bool,
    #[serde(default)]
    buffering: bool,
    #[serde(default)]
    at: Option<i64>,
    #[serde(default)]
    up_next: Vec<String>,
    #[serde(default)]
    away: bool,
    #[serde(default)]
    force: bool,
}

#[derive(Deserialize, Default)]
struct TextReq {
    #[serde(default)]
    text: String,
}

#[derive(Deserialize, Default)]
struct EmojiReq {
    #[serde(default)]
    emoji: String,
}

#[derive(Deserialize, Default)]
struct PidReq {
    #[serde(default)]
    pid: String,
}

#[derive(Deserialize, Default)]
pub struct TokenQuery {
    pub token: Option<String>,
}

pub async fn get_my_lobby(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<ClientQuery>,
) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let lobby = host_lobby_code(&state, user.id)
        .map(|code| host_view(&state, &headers, &code, &q.client_id))
        .unwrap_or(Value::Null);
    no_store(json_res(json!({ "lobby": lobby, "reactions": REACTIONS })))
}

pub async fn create_lobby(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let req: CreateReq = if body.is_empty() {
        CreateReq::default()
    } else {
        match parse_body(&body) {
            Ok(r) => r,
            Err(r) => return r,
        }
    };
    let host = host_info(&state.store, user.id);
    let mut name = clean_text(&req.name, LOBBY_NAME_MAX_CHARS);
    if name.is_empty() {
        name = format!("{}'s Lobby", host.display_name);
    }
    let rec = match state
        .store
        .create_lobby(user.id, &name, req.allow_chat.unwrap_or(true))
    {
        Ok(r) => r,
        Err(e) => return err_res(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    if let Err(g) = state.lobbies.ensure(&state.store, &rec.code) {
        return gone_res(g);
    }
    let lobby = host_view(&state, &headers, &rec.code, "");
    no_store(json_res(json!({ "lobby": lobby, "reactions": REACTIONS })))
}

pub async fn update_lobby(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let req: UpdateReq = match parse_body(&body) {
        Ok(r) => r,
        Err(r) => return r,
    };
    let Some(code) = host_lobby_code(&state, user.id) else {
        return err_res(StatusCode::NOT_FOUND, "You don't have an open lobby");
    };
    let now = now_ms();
    let Some((name, allow_chat, locked)) = state.lobbies.with(&code, |l| {
        if let Some(n) = req.name.as_deref() {
            let n = clean_text(n, LOBBY_NAME_MAX_CHARS);
            if !n.is_empty() {
                l.name = n;
            }
        }
        if let Some(v) = req.allow_chat {
            l.allow_chat = v;
        }
        if let Some(v) = req.locked {
            l.locked = v;
        }
        l.emit_state(now);
        (l.name.clone(), l.allow_chat, l.locked)
    }) else {
        return err_res(StatusCode::NOT_FOUND, "You don't have an open lobby");
    };
    if let Err(e) = state.store.update_lobby_settings(&code, &name, allow_chat, locked) {
        return err_res(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string());
    }
    json_res(json!({ "ok": true, "name": name, "allowChat": allow_chat, "locked": locked }))
}

pub async fn end_lobby(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let Some(rec) = state.store.active_lobby_for_user(user.id).ok().flatten() else {
        return json_res(json!({ "ok": true }));
    };
    if let Err(e) = state.store.end_lobby(&rec.code) {
        return err_res(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string());
    }
    if let Some(lobby) = state.lobbies.lock().remove(&rec.code) {
        lobby.emit("ended", &json!({ "reason": "host" }), None);
    }
    json_res(json!({ "ok": true }))
}

/// The host player's playback anchor. Sent on every play/pause/seek/track
/// change plus a heartbeat, from exactly one tab or device at a time.
pub async fn host_state(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let req: HostStateReq = match parse_body(&body) {
        Ok(r) => r,
        Err(r) => return r,
    };
    let client_id: String = req.client_id.trim().chars().take(64).collect();
    if client_id.is_empty() {
        return err_res(StatusCode::BAD_REQUEST, "Missing client id");
    }
    let Some(code) = host_lobby_code(&state, user.id) else {
        return coded_err(StatusCode::NOT_FOUND, "missing", "You don't have an open lobby");
    };
    let now = now_ms();

    // 1. Conflict check and change detection under the lock (cheap).
    let track_id = req.track_id.trim().to_string();
    let up_next_ids: Vec<String> = req
        .up_next
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .take(UP_NEXT_MAX)
        .collect();
    let Some(pre) = state.lobbies.with(&code, |l| {
        let conflict = !req.force
            && !l.host_client.is_empty()
            && l.host_client != client_id
            && l.host_online(now)
            && !l.host_away;
        let track_changed = l.track.as_ref().map(|t| t.id.as_str()).unwrap_or("") != track_id;
        let queue_changed = l.up_next.iter().map(|t| &t.id).ne(up_next_ids.iter());
        (conflict, track_changed, queue_changed)
    }) else {
        return coded_err(StatusCode::NOT_FOUND, "missing", "You don't have an open lobby");
    };
    let (conflict, track_changed, queue_changed) = pre;
    if conflict {
        return coded_err(
            StatusCode::CONFLICT,
            "conflict",
            "Your lobby is being broadcast from another tab or device.",
        );
    }

    // 2. Resolve track metadata outside the lock.
    let new_track = if track_changed && !track_id.is_empty() {
        state.store.get_track(&track_id).ok().flatten()
    } else {
        None
    };
    let new_queue = if queue_changed {
        let found = state.store.get_tracks_by_ids(&up_next_ids).unwrap_or_default();
        let by_id: HashMap<&str, &Track> = found.iter().map(|t| (t.id.as_str(), t)).collect();
        Some(
            up_next_ids
                .iter()
                .filter_map(|id| by_id.get(id.as_str()).map(|t| (*t).clone()))
                .collect::<Vec<_>>(),
        )
    } else {
        None
    };

    // 3. Apply.
    let at = req.at.filter(|a| (a - now).abs() < 5_000).unwrap_or(now);
    let position = if req.position_ms.is_finite() { req.position_ms.max(0.0) } else { 0.0 };
    let applied = state.lobbies.with(&code, |l| -> Result<bool, ()> {
        // Re-check under the lock: another push may have landed since step 1.
        if !req.force
            && !l.host_client.is_empty()
            && l.host_client != client_id
            && l.host_online(now)
            && !l.host_away
        {
            return Err(());
        }
        let mut changed = false;
        if l.host_client != client_id {
            l.host_client = client_id.clone();
            changed = true;
        }
        l.host_seen_ms = now;
        l.host_reported_playing = req.playing && !req.away;
        if l.host_offline_announced {
            l.host_offline_announced = false;
            changed = true;
        }
        if l.host_away != req.away {
            l.host_away = req.away;
            changed = true;
        }
        let current_id = l.track.as_ref().map(|t| t.id.clone()).unwrap_or_default();
        let mut reanchor = false;
        let mut stale = false;
        if current_id != track_id {
            if !track_changed {
                // A newer push changed the track after we looked; ours is stale.
                stale = true;
            } else {
                // `None` here means "nothing" or a track we can't serve to guests.
                let next = new_track.filter(|t| t.id == track_id);
                if next.is_some() || l.track.is_some() {
                    if let Some(old) = l.track.take() {
                        l.recent.retain(|id| id != &old.id);
                        l.recent.push_back(old.id);
                        while l.recent.len() > RECENT_MAX {
                            l.recent.pop_front();
                        }
                    }
                    l.track = next;
                    changed = true;
                    reanchor = true;
                }
            }
        }
        if let Some(q) = new_queue {
            if !stale {
                l.up_next = q;
                changed = true;
            }
        }
        if !stale {
            // Compare against what we'd actually store: nothing playable means not playing.
            let want_playing = req.playing && l.track.is_some();
            if l.playing != want_playing || l.buffering != req.buffering {
                reanchor = true;
            }
            if !reanchor && (l.position_at(at) - position).abs() > ANCHOR_TOLERANCE_MS {
                reanchor = true;
            }
            if reanchor {
                l.position_ms = position;
                l.updated_ms = at;
                l.playing = want_playing;
                l.buffering = req.buffering;
                changed = true;
            }
        }
        if changed {
            l.emit_state(now);
        }
        let secs = now / 1000;
        if secs - l.last_db_touch >= 60 {
            l.last_db_touch = secs;
            Ok(true)
        } else {
            Ok(false)
        }
    });
    let touch_db = match applied {
        Some(Ok(v)) => v,
        Some(Err(())) => {
            return coded_err(
                StatusCode::CONFLICT,
                "conflict",
                "Your lobby is being broadcast from another tab or device.",
            )
        }
        None => return coded_err(StatusCode::NOT_FOUND, "missing", "You don't have an open lobby"),
    };
    if touch_db {
        let _ = state.store.touch_lobby(&code, now / 1000);
    }
    let count = state.lobbies.with(&code, |l| l.listeners.len()).unwrap_or(0);
    json_res(json!({ "ok": true, "listenerCount": count }))
}

pub async fn host_chat(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let req: TextReq = match parse_body(&body) {
        Ok(r) => r,
        Err(r) => return r,
    };
    let text = clean_text(&req.text, CHAT_MAX_CHARS);
    if text.is_empty() {
        return err_res(StatusCode::BAD_REQUEST, "Message is empty");
    }
    let Some(code) = host_lobby_code(&state, user.id) else {
        return err_res(StatusCode::NOT_FOUND, "You don't have an open lobby");
    };
    let now = now_ms();
    state.lobbies.with(&code, |l| {
        let name = l.host.display_name.clone();
        let color = if l.host.avatar_color.is_empty() {
            AVATAR_COLORS[0].to_string()
        } else {
            l.host.avatar_color.clone()
        };
        l.post_chat("host", "host", &name, &color, &text, now);
    });
    json_res(json!({ "ok": true }))
}

pub async fn host_react(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let req: EmojiReq = match parse_body(&body) {
        Ok(r) => r,
        Err(r) => return r,
    };
    if !REACTIONS.contains(&req.emoji.as_str()) {
        return err_res(StatusCode::BAD_REQUEST, "Unsupported reaction");
    }
    let Some(code) = host_lobby_code(&state, user.id) else {
        return err_res(StatusCode::NOT_FOUND, "You don't have an open lobby");
    };
    state.lobbies.with(&code, |l| {
        let data = json!({ "emoji": req.emoji, "name": l.host.display_name, "pid": "host" });
        l.emit("react", &data, None);
    });
    json_res(json!({ "ok": true }))
}

pub async fn host_kick(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let req: PidReq = match parse_body(&body) {
        Ok(r) => r,
        Err(r) => return r,
    };
    let Some(code) = host_lobby_code(&state, user.id) else {
        return err_res(StatusCode::NOT_FOUND, "You don't have an open lobby");
    };
    let now = now_ms();
    let removed = state
        .lobbies
        .with(&code, |l| {
            let lid = l
                .listeners
                .iter()
                .find(|(_, v)| v.pid == req.pid)
                .map(|(k, _)| k.clone())?;
            l.listeners.remove(&lid);
            l.kicked.insert(lid.clone());
            l.emit("kicked", &json!({ "reason": "host" }), Some(&lid));
            l.emit_state(now);
            Some(())
        })
        .flatten()
        .is_some();
    if !removed {
        return err_res(StatusCode::NOT_FOUND, "That listener already left");
    }
    json_res(json!({ "ok": true }))
}

/// The host's own live feed (listeners, chat, reactions). Uses `?token=`
/// because `EventSource` can't send an Authorization header.
pub async fn host_events(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
) -> Response {
    let Some(user) = resolve_web_user(&state, &headers, q.token.as_deref()) else {
        return err_res(StatusCode::UNAUTHORIZED, "Please sign in");
    };
    let Some(code) = host_lobby_code(&state, user.id) else {
        return gone_res(Gone::Missing);
    };
    let Some((rx, init)) = state.lobbies.with(&code, |l| {
        let init = json!({ "state": l.snapshot(now_ms()), "chat": l.chat_json() });
        (l.tx.subscribe(), init)
    }) else {
        return gone_res(Gone::Missing);
    };
    event_stream(StreamState {
        rx,
        hub: state.lobbies.clone(),
        code,
        lid: None,
        init: Some(Event::default().event("init").data(init.to_string())),
        done: false,
        _guard: None,
    })
}

// ---------------------------------------------------------------------------
// Listener API (public, no account required)
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
struct JoinReq {
    #[serde(default)]
    lid: String,
    #[serde(default)]
    name: String,
}

#[derive(Deserialize, Default)]
struct LidReq {
    #[serde(default)]
    lid: String,
}

#[derive(Deserialize, Default)]
struct GuestTextReq {
    #[serde(default)]
    lid: String,
    #[serde(default)]
    text: String,
}

#[derive(Deserialize, Default)]
struct GuestEmojiReq {
    #[serde(default)]
    lid: String,
    #[serde(default)]
    emoji: String,
}

#[derive(Deserialize, Default)]
pub struct LidQuery {
    #[serde(default)]
    pub lid: String,
}

#[derive(Deserialize, Default)]
pub struct GuestStreamQuery {
    #[serde(default)]
    pub lid: String,
    #[serde(rename = "t")]
    pub offset: Option<f64>,
    pub compat: Option<String>,
}

pub async fn clock() -> Response {
    no_store(json_res(json!({ "now": now_ms() })))
}

pub async fn guest_join(
    State(state): State<AppState>,
    Path(code): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(code) = normalize_code(&code) else {
        return gone_res(Gone::Missing);
    };
    let req: JoinReq = match parse_body(&body) {
        Ok(r) => r,
        Err(r) => return r,
    };
    if !valid_lid(&req.lid) {
        return err_res(StatusCode::BAD_REQUEST, "Invalid listener id");
    }
    if let Err(g) = state.lobbies.ensure(&state.store, &code) {
        return gone_res(g);
    }
    // Signed-in Beatlab users join under their own name and picture.
    let member = resolve_web_user(&state, &headers, None).and_then(|u| {
        state
            .store
            .get_public_user(u.id, u.id)
            .ok()
            .flatten()
            .map(|p| (u.id, p))
    });
    let guest_name = clean_text(&req.name, NAME_MAX_CHARS);
    let ip = crate::state::rate_limit_ip(&headers);
    let now = now_ms();
    let lid = req.lid.clone();
    let result = state.lobbies.with(&code, |l| -> Result<Value, Response> {
        if l.kicked.contains(&lid) {
            return Err(coded_err(
                StatusCode::FORBIDDEN,
                "kicked",
                "The host removed you from this lobby.",
            ));
        }
        let pid = public_id(&l.code, &lid);
        if let Some(existing) = l.listeners.get_mut(&lid) {
            existing.last_seen_ms = now;
            existing.leaving = false;
            if existing.username.is_empty() && !guest_name.is_empty() {
                existing.name = guest_name.clone();
            }
        } else {
            let is_host = member.as_ref().is_some_and(|(id, _)| *id == l.host_id);
            if l.locked && !is_host {
                return Err(coded_err(
                    StatusCode::FORBIDDEN,
                    "locked",
                    "The host has locked this lobby. Ask them to unlock it.",
                ));
            }
            if l.listeners.len() >= MAX_LISTENERS {
                return Err(coded_err(StatusCode::CONFLICT, "full", "This lobby is full right now."));
            }
            if member.is_none() && guest_name.is_empty() {
                return Err(coded_err(StatusCode::BAD_REQUEST, "name", "Pick a name so friends know who's here."));
            }
            if !is_host && !l.allow_join_from(&ip, now) {
                return Err(coded_err(
                    StatusCode::TOO_MANY_REQUESTS,
                    "slow",
                    "Too many people joined from this connection. Try again in a few minutes.",
                ));
            }
            let (name, username, avatar_url, color) = match &member {
                Some((_, p)) => (
                    if p.display_name.trim().is_empty() { p.username.clone() } else { p.display_name.clone() },
                    p.username.clone(),
                    p.avatar_url.clone(),
                    if p.avatar_color.is_empty() { color_for(&pid) } else { p.avatar_color.clone() },
                ),
                None => (guest_name.clone(), String::new(), String::new(), color_for(&pid)),
            };
            let listener = Listener {
                pid: pid.clone(),
                name: name.clone(),
                username,
                avatar_url,
                color: color.clone(),
                is_host,
                joined_ms: now,
                conns: 0,
                last_seen_ms: now,
                leaving: false,
                chat_tokens: CHAT_BURST,
                chat_refill_at: now,
                last_react_ms: 0,
            };
            l.listeners.insert(lid.clone(), listener);
            if l.welcomed.len() >= WELCOMED_MAX {
                l.welcomed.clear();
            }
            if l.welcomed.insert(lid.clone()) {
                let text = format!("{name} joined");
                l.post_chat("system", &pid, &name, &color, &text, now);
            }
        }
        l.emit_state(now);
        Ok(json!({ "pid": pid, "state": l.snapshot(now), "chat": l.chat_json() }))
    });
    match result {
        Some(Ok(v)) => no_store(json_res(v)),
        Some(Err(r)) => r,
        None => gone_res(Gone::Missing),
    }
}

pub async fn guest_leave(State(state): State<AppState>, Path(code): Path<String>, body: Bytes) -> Response {
    let Some(code) = normalize_code(&code) else {
        return gone_res(Gone::Missing);
    };
    let req: LidReq = match parse_body(&body) {
        Ok(r) => r,
        Err(r) => return r,
    };
    let now = now_ms();
    state.lobbies.with(&code, |l| {
        let gone_now = match l.listeners.get_mut(&req.lid) {
            Some(listener) => {
                listener.leaving = true;
                listener.last_seen_ms = now;
                listener.conns == 0
            }
            None => false,
        };
        // Still connected: the stream closing (or the sweeper) finishes the job,
        // and a page that comes back from the back/forward cache reconnects.
        if gone_now {
            l.listeners.remove(&req.lid);
            l.emit_state(now);
        }
    });
    json_res(json!({ "ok": true }))
}

pub async fn guest_events(
    State(state): State<AppState>,
    Path(code): Path<String>,
    Query(q): Query<LidQuery>,
) -> Response {
    let Some(code) = normalize_code(&code) else {
        return gone_res(Gone::Missing);
    };
    if let Err(g) = state.lobbies.ensure(&state.store, &code) {
        return gone_res(g);
    }
    let now = now_ms();
    let opened = state.lobbies.with(&code, |l| {
        if l.kicked.contains(&q.lid) {
            return Err(coded_err(StatusCode::FORBIDDEN, "kicked", "Removed by the host"));
        }
        let listener = l
            .listeners
            .get_mut(&q.lid)
            .ok_or_else(|| coded_err(StatusCode::FORBIDDEN, "not_joined", "Join the lobby first"))?;
        listener.conns += 1;
        listener.last_seen_ms = now;
        listener.leaving = false;
        let pid = listener.pid.clone();
        let init = json!({ "pid": pid, "state": l.snapshot(now), "chat": l.chat_json() });
        Ok((l.tx.subscribe(), init))
    });
    let (rx, init) = match opened {
        Some(Ok(v)) => v,
        Some(Err(r)) => return r,
        None => return gone_res(Gone::Missing),
    };
    event_stream(StreamState {
        rx,
        hub: state.lobbies.clone(),
        code: code.clone(),
        lid: Some(q.lid.clone()),
        init: Some(Event::default().event("init").data(init.to_string())),
        done: false,
        _guard: Some(ConnGuard {
            hub: state.lobbies.clone(),
            code,
            lid: q.lid,
        }),
    })
}

pub async fn guest_chat(State(state): State<AppState>, Path(code): Path<String>, body: Bytes) -> Response {
    let Some(code) = normalize_code(&code) else {
        return gone_res(Gone::Missing);
    };
    let req: GuestTextReq = match parse_body(&body) {
        Ok(r) => r,
        Err(r) => return r,
    };
    let text = clean_text(&req.text, CHAT_MAX_CHARS);
    if text.is_empty() {
        return err_res(StatusCode::BAD_REQUEST, "Message is empty");
    }
    let now = now_ms();
    let result = state.lobbies.with(&code, |l| -> Result<(), Response> {
        let allow_chat = l.allow_chat;
        let listener = l
            .listeners
            .get_mut(&req.lid)
            .ok_or_else(|| coded_err(StatusCode::FORBIDDEN, "not_joined", "Join the lobby first"))?;
        if !allow_chat && !listener.is_host {
            return Err(coded_err(StatusCode::FORBIDDEN, "chat_off", "The host turned chat off."));
        }
        if !listener.take_chat_token(now) {
            return Err(coded_err(StatusCode::TOO_MANY_REQUESTS, "slow", "Slow down a little."));
        }
        let (kind, pid, name, color) = (
            listener.kind(),
            listener.pid.clone(),
            listener.name.clone(),
            listener.color.clone(),
        );
        l.post_chat(kind, &pid, &name, &color, &text, now);
        Ok(())
    });
    match result {
        Some(Ok(())) => json_res(json!({ "ok": true })),
        Some(Err(r)) => r,
        None => gone_res(Gone::Missing),
    }
}

pub async fn guest_react(State(state): State<AppState>, Path(code): Path<String>, body: Bytes) -> Response {
    let Some(code) = normalize_code(&code) else {
        return gone_res(Gone::Missing);
    };
    let req: GuestEmojiReq = match parse_body(&body) {
        Ok(r) => r,
        Err(r) => return r,
    };
    if !REACTIONS.contains(&req.emoji.as_str()) {
        return err_res(StatusCode::BAD_REQUEST, "Unsupported reaction");
    }
    let now = now_ms();
    let result = state.lobbies.with(&code, |l| -> Result<(), Response> {
        let listener = l
            .listeners
            .get_mut(&req.lid)
            .ok_or_else(|| coded_err(StatusCode::FORBIDDEN, "not_joined", "Join the lobby first"))?;
        if now - listener.last_react_ms < REACT_MIN_GAP_MS {
            return Err(coded_err(StatusCode::TOO_MANY_REQUESTS, "slow", "Slow down a little."));
        }
        listener.last_react_ms = now;
        let data = json!({ "emoji": req.emoji, "name": listener.name, "pid": listener.pid });
        l.emit("react", &data, None);
        Ok(())
    });
    match result {
        Some(Ok(())) => json_res(json!({ "ok": true })),
        Some(Err(r)) => r,
        None => gone_res(Gone::Missing),
    }
}

/// Checks the listener is in the lobby and the track is one the host is
/// playing (or just played / is about to play).
fn guest_track(state: &AppState, code: &str, lid: &str, track_id: &str) -> Result<Track, Response> {
    let code = normalize_code(code).ok_or_else(|| gone_res(Gone::Missing))?;
    state.lobbies.ensure(&state.store, &code).map_err(gone_res)?;
    let decoded = urlencoding::decode(track_id)
        .map(|s| s.into_owned())
        .unwrap_or_else(|_| track_id.to_string());
    let allowed = state
        .lobbies
        .with(&code, |l| {
            if !l.listeners.contains_key(lid) {
                return Err(coded_err(StatusCode::FORBIDDEN, "not_joined", "Join the lobby first"));
            }
            if !l.allows_stream_of(&decoded) {
                return Err(err_res(StatusCode::FORBIDDEN, "That track isn't playing in this lobby"));
            }
            Ok(())
        })
        .unwrap_or_else(|| Err(gone_res(Gone::Missing)));
    allowed?;
    state
        .store
        .get_track(&decoded)
        .ok()
        .flatten()
        .ok_or_else(|| err_res(StatusCode::NOT_FOUND, "Track not found"))
}

pub async fn guest_stream(
    State(state): State<AppState>,
    Path((code, track_id)): Path<(String, String)>,
    Query(q): Query<GuestStreamQuery>,
    headers: HeaderMap,
) -> Response {
    let track = match guest_track(&state, &code, &q.lid, &track_id) {
        Ok(t) => t,
        Err(r) => return r,
    };
    let offset = q.offset.unwrap_or(0.0);
    let force_compat = matches!(q.compat.as_deref(), Some("1" | "true"));
    state
        .engine
        .serve_web_stream(&track, &headers, offset, force_compat)
        .await
}

pub async fn guest_lyrics(
    State(state): State<AppState>,
    Path((code, track_id)): Path<(String, String)>,
    Query(q): Query<LidQuery>,
) -> Response {
    let track = match guest_track(&state, &code, &q.lid, &track_id) {
        Ok(t) => t,
        Err(r) => return r,
    };
    let lyrics = state.engine.fetch_lyrics(&track).await;
    json_res(
        serde_json::to_value(lyrics)
            .unwrap_or_else(|_| json!({ "synced": false, "plain": "", "lines": [] })),
    )
}

/// Album art of whatever the lobby is playing, for link-preview embeds.
pub async fn public_cover(State(state): State<AppState>, Path(code): Path<String>) -> Response {
    let track_id = normalize_code(&code)
        .filter(|c| state.lobbies.ensure(&state.store, c).is_ok())
        .and_then(|c| state.lobbies.with(&c, |l| l.track.as_ref().map(|t| t.id.clone())))
        .flatten();
    if let Some(id) = track_id {
        if let Some((bytes, mime)) = state.engine.cover_raster_bytes(&id).await {
            return (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, mime),
                    (header::CACHE_CONTROL, "public, max-age=60".to_string()),
                ],
                bytes,
            )
                .into_response();
        }
    }
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "public, max-age=60"),
        ],
        ICON_512_PNG,
    )
        .into_response()
}

/// The shareable listener page at `/l/{code}`.
pub async fn public_page(
    State(state): State<AppState>,
    Path(code): Path<String>,
    headers: HeaderMap,
) -> Response {
    let origin = request_origin(&headers);
    let html_res = |status: StatusCode, html: String| {
        (
            status,
            [
                (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                (header::CACHE_CONTROL, "no-cache"),
            ],
            html,
        )
            .into_response()
    };
    let Some(code) = normalize_code(&code) else {
        return html_res(
            StatusCode::NOT_FOUND,
            crate::lobby_page::render_lobby_html(&origin, "", "missing", &Value::Null),
        );
    };
    match state.lobbies.ensure(&state.store, &code) {
        Ok(()) => {
            let snap = state
                .lobbies
                .with(&code, |l| l.snapshot(now_ms()))
                .unwrap_or(Value::Null);
            html_res(
                StatusCode::OK,
                crate::lobby_page::render_lobby_html(&origin, &code, "live", &snap),
            )
        }
        Err(Gone::Ended) => html_res(
            StatusCode::GONE,
            crate::lobby_page::render_lobby_html(&origin, &code, "ended", &Value::Null),
        ),
        Err(Gone::Missing) => html_res(
            StatusCode::NOT_FOUND,
            crate::lobby_page::render_lobby_html(&origin, &code, "missing", &Value::Null),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use futures::StreamExt;

    fn test_state() -> AppState {
        use crate::engine::StreamEngine;
        use crate::sources::archive::ArchiveSource;
        use crate::sources::deezer::DeezerSource;
        use crate::sources::local::LocalSource;
        use crate::sources::soulseek::SoulseekSource;
        use crate::sources::ytm::YtmSource;
        use crate::sources::SourceRegistry;
        use crate::taste::TasteEngine;

        let store = Store::open_memory().unwrap();
        let tmp = std::env::temp_dir().join(format!("beatlab-lobby-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp).unwrap();
        let sources = SourceRegistry::new(
            DeezerSource::new(),
            YtmSource::new(None),
            ArchiveSource::new(),
            Arc::new(LocalSource::new(store.clone(), vec![])),
            SoulseekSource::new(store.clone(), tmp.join("slsk")),
        );
        let engine = Arc::new(StreamEngine::new(store.clone(), sources.clone(), &tmp, 1.0));
        let taste = Arc::new(TasteEngine::new(store.clone(), sources.clone()));

        let audio_file = tmp.join("lobby.mp3");
        std::fs::write(&audio_file, b"ID3\x03\x00\x00\x00\x00\x00\x00FAKE_MP3_PAYLOAD").unwrap();
        let mk = |id: &str, title: &str| Track {
            id: id.into(),
            source: "local".into(),
            title: title.into(),
            artist: "HOME".into(),
            album: "Odyssey".into(),
            duration: 212,
            format: "mp3".into(),
            bitrate: 320,
            stream_url: format!("file:{}", audio_file.display()),
            ..Default::default()
        };
        store
            .upsert_tracks(&[
                mk("loc:a", "Resonance"),
                mk("loc:b", "Nightcall"),
                mk("loc:c", "Secret"),
            ])
            .unwrap();

        AppState {
            store,
            sources,
            engine,
            taste,
            llm: None,
            now_playing: Arc::new(Mutex::new(Default::default())),
            new_releases_inflight: Arc::new(Mutex::new(Default::default())),
            lobbies: Default::default(),
        }
    }

    fn host_headers(state: &AppState, username: &str) -> (HeaderMap, i64) {
        let (u, _) = state.store.authenticate_or_create_user(username, "pw123456").unwrap();
        let tok = state.store.create_session(u.id, "web").unwrap();
        let mut h = HeaderMap::new();
        h.insert(header::AUTHORIZATION, format!("Bearer {tok}").parse().unwrap());
        h.insert(header::HOST, "music.example.com".parse().unwrap());
        h.insert("x-forwarded-proto", "https".parse().unwrap());
        (h, u.id)
    }

    async fn body_json(resp: Response) -> Value {
        let bytes = to_bytes(resp.into_body(), 1024 * 512).await.unwrap();
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    }

    fn body(v: Value) -> Bytes {
        Bytes::from(v.to_string())
    }

    async fn push(state: &AppState, h: &HeaderMap, v: Value) -> Response {
        host_state(State(state.clone()), h.clone(), body(v)).await
    }

    async fn join(state: &AppState, code: &str, lid: &str, name: &str) -> Response {
        guest_join(
            State(state.clone()),
            Path(code.to_string()),
            HeaderMap::new(),
            body(json!({ "lid": lid, "name": name })),
        )
        .await
    }

    const LID_A: &str = "listener-aaaaaaaaaaaaaaaa";
    const LID_B: &str = "listener-bbbbbbbbbbbbbbbb";

    #[tokio::test]
    async fn host_creates_lobby_and_guests_listen_without_accounts() {
        let state = test_state();
        let (h, _) = host_headers(&state, "dj");

        // Create: default name, invite URL on the public origin, QR code.
        let created = body_json(create_lobby(State(state.clone()), h.clone(), Bytes::new()).await).await;
        let lobby = &created["lobby"];
        let code = lobby["code"].as_str().unwrap().to_string();
        assert_eq!(code.len(), 8);
        assert_eq!(lobby["name"], "dj's Lobby");
        assert_eq!(lobby["url"], format!("https://music.example.com/l/{code}"));
        assert!(lobby["qrSvg"].as_str().unwrap().contains("<svg"));
        assert_eq!(lobby["allowChat"], true);

        // Creating again returns the same open lobby rather than a second link.
        let again = body_json(create_lobby(State(state.clone()), h.clone(), Bytes::new()).await).await;
        assert_eq!(again["lobby"]["code"], code.as_str());

        // Guests need a name and a well-formed listener id.
        let r = join(&state, &code, LID_A, "  ").await;
        assert_eq!(r.status(), StatusCode::BAD_REQUEST);
        assert_eq!(body_json(r).await["code"], "name");
        let r = join(&state, &code, "short", "Sam").await;
        assert_eq!(r.status(), StatusCode::BAD_REQUEST);

        let joined = body_json(join(&state, &code, LID_A, "Sam\u{202E} <b>").await).await;
        let pid = joined["pid"].as_str().unwrap().to_string();
        assert_eq!(pid.len(), 12);
        assert!(!joined.to_string().contains(LID_A), "private listener id must never be echoed");
        assert_eq!(joined["state"]["listenerCount"], 1);
        assert_eq!(joined["state"]["listeners"][0]["name"], "Sam <b>");
        assert_eq!(joined["state"]["listeners"][0]["kind"], "guest");
        assert_eq!(joined["chat"][0]["kind"], "system");
        assert_eq!(joined["chat"][0]["text"], "Sam <b> joined");
        assert!(joined["state"]["playback"]["track"].is_null());

        // Host starts playing: guests see the track, position and up-next.
        let r = push(&state, &h, json!({
            "clientId": "tab-1", "trackId": "loc:a", "positionMs": 30_000.0,
            "playing": true, "upNext": ["loc:b", "missing:id"],
        }))
        .await;
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(body_json(r).await["listenerCount"], 1);
        let snap: Value = serde_json::from_str(&state.lobbies.snapshot_json(&code).unwrap()).unwrap();
        assert_eq!(snap["playback"]["track"]["title"], "Resonance");
        assert_eq!(snap["playback"]["track"]["coverUrl"], "/api/web/cover/loc%3Aa?v=2");
        assert_eq!(snap["playback"]["playing"], true);
        let pos = snap["playback"]["positionMs"].as_f64().unwrap();
        assert!((30_000.0..31_000.0).contains(&pos), "position {pos}");
        assert_eq!(snap["upNext"].as_array().unwrap().len(), 1, "unknown ids are dropped");
        assert_eq!(snap["host"]["online"], true);

        // Guests may stream the current and upcoming tracks — nothing else.
        let stream = |track: &str, lid: &str| {
            guest_stream(
                State(state.clone()),
                Path((code.clone(), track.to_string())),
                Query(GuestStreamQuery { lid: lid.into(), offset: None, compat: None }),
                HeaderMap::new(),
            )
        };
        assert_eq!(stream("loc:a", LID_A).await.status(), StatusCode::OK);
        assert_eq!(stream("loc%3Ab", LID_A).await.status(), StatusCode::OK);
        assert_eq!(stream("loc:c", LID_A).await.status(), StatusCode::FORBIDDEN);
        assert_eq!(stream("loc:a", LID_B).await.status(), StatusCode::FORBIDDEN, "must join first");

        // A second tab can't silently hijack the broadcast, but can take over.
        let r = push(&state, &h, json!({ "clientId": "tab-2", "trackId": "loc:a", "playing": false })).await;
        assert_eq!(r.status(), StatusCode::CONFLICT);
        let view = body_json(
            get_my_lobby(State(state.clone()), h.clone(), Query(ClientQuery { client_id: "tab-2".into() })).await,
        )
        .await;
        assert_eq!(view["lobby"]["broadcastingElsewhere"], true);
        let r = push(&state, &h, json!({
            "clientId": "tab-2", "trackId": "loc:b", "positionMs": 0.0, "playing": true, "force": true,
        }))
        .await;
        assert_eq!(r.status(), StatusCode::OK);
        // The previous track stays streamable briefly so buffered listeners finish cleanly.
        assert_eq!(stream("loc:a", LID_A).await.status(), StatusCode::OK);

        // Chat: guests post, then get throttled after a burst.
        let chat = |text: &str| {
            guest_chat(State(state.clone()), Path(code.clone()), body(json!({ "lid": LID_A, "text": text })))
        };
        for i in 0..5 {
            assert_eq!(chat(&format!("hi {i}")).await.status(), StatusCode::OK);
        }
        assert_eq!(chat("spam").await.status(), StatusCode::TOO_MANY_REQUESTS);
        let r = host_chat(State(state.clone()), h.clone(), body(json!({ "text": "welcome!" }))).await;
        assert_eq!(r.status(), StatusCode::OK);
        let view = body_json(
            get_my_lobby(State(state.clone()), h.clone(), Query(ClientQuery { client_id: "tab-2".into() })).await,
        )
        .await;
        let chat_log = view["lobby"]["chat"].as_array().unwrap();
        assert_eq!(chat_log.last().unwrap()["kind"], "host");
        assert_eq!(chat_log.last().unwrap()["text"], "welcome!");
        assert_eq!(view["lobby"]["broadcastingElsewhere"], false);

        // Reactions are limited to the fixed palette.
        let react = |emoji: &str| {
            guest_react(State(state.clone()), Path(code.clone()), body(json!({ "lid": LID_A, "emoji": emoji })))
        };
        assert_eq!(react("🔥").await.status(), StatusCode::OK);
        assert_eq!(react("💩").await.status(), StatusCode::BAD_REQUEST);

        // Turning chat off blocks guests but not the host.
        let r = update_lobby(State(state.clone()), h.clone(), body(json!({ "allowChat": false, "name": "  Friday   Night  " }))).await;
        assert_eq!(body_json(r).await["name"], "Friday Night");
        tokio::time::sleep(Duration::from_millis(5)).await;
        let r = guest_chat(State(state.clone()), Path(code.clone()), body(json!({ "lid": LID_A, "text": "x" }))).await;
        assert_eq!(body_json(r).await["code"], "chat_off");
        assert_eq!(
            host_chat(State(state.clone()), h.clone(), body(json!({ "text": "still here" }))).await.status(),
            StatusCode::OK
        );

        // Locking stops new listeners; existing ones stay.
        update_lobby(State(state.clone()), h.clone(), body(json!({ "locked": true }))).await;
        let r = join(&state, &code, LID_B, "Late").await;
        assert_eq!(body_json(r).await["code"], "locked");
        assert_eq!(join(&state, &code, LID_A, "Sam").await.status(), StatusCode::OK);
        update_lobby(State(state.clone()), h.clone(), body(json!({ "locked": false }))).await;

        // Kicking removes the listener and keeps them out.
        assert_eq!(join(&state, &code, LID_B, "Riley").await.status(), StatusCode::OK);
        let r = host_kick(State(state.clone()), h.clone(), body(json!({ "pid": pid }))).await;
        assert_eq!(r.status(), StatusCode::OK);
        let r = join(&state, &code, LID_A, "Sam again").await;
        assert_eq!(body_json(r).await["code"], "kicked");
        assert_eq!(stream("loc:b", LID_A).await.status(), StatusCode::FORBIDDEN);
        let snap: Value = serde_json::from_str(&state.lobbies.snapshot_json(&code).unwrap()).unwrap();
        assert_eq!(snap["listenerCount"], 1);
        assert_eq!(snap["listeners"][0]["name"], "Riley");

        // The public page renders the live lobby with link-preview metadata.
        let page = public_page(State(state.clone()), Path(code.to_uppercase()), h.clone()).await;
        assert_eq!(page.status(), StatusCode::OK);
        let html = String::from_utf8(to_bytes(page.into_body(), 1 << 20).await.unwrap().to_vec()).unwrap();
        assert!(html.contains("Listen along with dj on Beatlab"));
        assert!(html.contains("Now playing: Nightcall"));

        // Ending closes the link for everyone.
        assert_eq!(end_lobby(State(state.clone()), h.clone()).await.status(), StatusCode::OK);
        let r = join(&state, &code, LID_B, "Riley").await;
        assert_eq!(r.status(), StatusCode::GONE);
        let page = public_page(State(state.clone()), Path(code.clone()), h.clone()).await;
        assert_eq!(page.status(), StatusCode::GONE);
        let view = body_json(
            get_my_lobby(State(state.clone()), h.clone(), Query(ClientQuery::default())).await,
        )
        .await;
        assert!(view["lobby"].is_null());
        let r = push(&state, &h, json!({ "clientId": "tab-2", "playing": false })).await;
        assert_eq!(r.status(), StatusCode::NOT_FOUND);

        // Unknown codes are a clean 404.
        let page = public_page(State(state.clone()), Path("nope2345".into()), h.clone()).await;
        assert_eq!(page.status(), StatusCode::NOT_FOUND);
        assert_eq!(join(&state, "../etc", LID_B, "x").await.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn anchor_projection_pauses_buffering_and_offline_hosts() {
        let state = test_state();
        let (h, _) = host_headers(&state, "anchor");
        let created = body_json(create_lobby(State(state.clone()), h.clone(), body(json!({ "name": "Test" }))).await).await;
        let code = created["lobby"]["code"].as_str().unwrap().to_string();

        // An anchor stamped 2s in the past projects forward while playing.
        let now = now_ms();
        push(&state, &h, json!({
            "clientId": "c", "trackId": "loc:a", "positionMs": 10_000.0, "playing": true, "at": now - 2_000,
        }))
        .await;
        let pos = state.lobbies.with(&code, |l| l.position_at(now_ms())).unwrap();
        assert!((11_900.0..12_600.0).contains(&pos), "projected {pos}");

        // Small heartbeat jitter doesn't re-anchor (no rebroadcast storm)...
        let before = state.lobbies.with(&code, |l| l.updated_ms).unwrap();
        push(&state, &h, json!({
            "clientId": "c", "trackId": "loc:a", "positionMs": pos + 150.0, "playing": true,
        }))
        .await;
        assert_eq!(state.lobbies.with(&code, |l| l.updated_ms).unwrap(), before);
        // ...but a real seek does.
        push(&state, &h, json!({ "clientId": "c", "trackId": "loc:a", "positionMs": 90_000.0, "playing": true })).await;
        let pos = state.lobbies.with(&code, |l| l.position_at(now_ms())).unwrap();
        assert!((90_000.0..90_500.0).contains(&pos), "after seek {pos}");

        // Buffering freezes the clock for listeners.
        push(&state, &h, json!({ "clientId": "c", "trackId": "loc:a", "positionMs": 91_000.0, "playing": true, "buffering": true })).await;
        let snap: Value = serde_json::from_str(&state.lobbies.snapshot_json(&code).unwrap()).unwrap();
        assert_eq!(snap["playback"]["playing"], false);
        assert_eq!(snap["playback"]["buffering"], true);
        assert_eq!(snap["playback"]["positionMs"], 91_000.0);

        // Position never runs past the end of the track.
        let end = state.lobbies.with(&code, |l| {
            l.buffering = false;
            l.position_at(l.updated_ms + 10 * 60 * 1000)
        });
        assert_eq!(end, Some(212_000.0));

        // When the host's player disappears, the sweeper pauses everyone in place.
        // Anchor: 50s in, 40s ago; host last checked in 36s ago.
        push(&state, &h, json!({ "clientId": "c", "trackId": "loc:a", "positionMs": 50_000.0, "playing": true })).await;
        state.lobbies.with(&code, |l| {
            l.updated_ms -= 40_000;
            l.host_seen_ms = l.updated_ms + 4_000;
        });
        state.lobbies.sweep(&state.store);
        let snap: Value = serde_json::from_str(&state.lobbies.snapshot_json(&code).unwrap()).unwrap();
        assert_eq!(snap["host"]["online"], false);
        assert_eq!(snap["playback"]["playing"], false);
        let frozen = snap["playback"]["positionMs"].as_f64().unwrap();
        assert!((53_900.0..54_200.0).contains(&frozen), "frozen where the host was last seen: {frozen}");
        // Frozen means frozen: time passing doesn't move it.
        let later = state.lobbies.with(&code, |l| l.position_at(now_ms() + 30_000)).unwrap();
        assert_eq!(later.round(), frozen);
        // The host coming back resumes for everyone.
        push(&state, &h, json!({ "clientId": "c", "trackId": "loc:a", "positionMs": 51_000.0, "playing": true })).await;
        let snap: Value = serde_json::from_str(&state.lobbies.snapshot_json(&code).unwrap()).unwrap();
        assert_eq!(snap["host"]["online"], true);
        assert_eq!(snap["playback"]["playing"], true);

        // A paused host in a throttled background tab isn't flagged offline after a
        // ~1 minute heartbeat gap; a playing one would be.
        push(&state, &h, json!({ "clientId": "c", "trackId": "loc:a", "positionMs": 51_500.0, "playing": false })).await;
        state.lobbies.with(&code, |l| l.host_seen_ms -= 65_000);
        state.lobbies.sweep(&state.store);
        let snap: Value = serde_json::from_str(&state.lobbies.snapshot_json(&code).unwrap()).unwrap();
        assert_eq!(snap["host"]["online"], true);
        assert_eq!(snap["playback"]["positionMs"], 51_500.0);

        // Stepping away (tab closed) pauses listeners without waiting for the timeout.
        push(&state, &h, json!({ "clientId": "c", "trackId": "loc:a", "positionMs": 52_000.0, "playing": false, "away": true })).await;
        let snap: Value = serde_json::from_str(&state.lobbies.snapshot_json(&code).unwrap()).unwrap();
        assert_eq!(snap["host"]["away"], true);
        assert_eq!(snap["host"]["online"], false);
        // ...and an away host never blocks their own next tab.
        let r = push(&state, &h, json!({ "clientId": "fresh-tab", "trackId": "loc:a", "playing": true })).await;
        assert_eq!(r.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn event_stream_tracks_presence_and_lobbies_survive_restarts() {
        let state = test_state();
        let (h, uid) = host_headers(&state, "presence");
        let created = body_json(create_lobby(State(state.clone()), h.clone(), Bytes::new()).await).await;
        let code = created["lobby"]["code"].as_str().unwrap().to_string();

        // Streams are only for joined listeners.
        let r = guest_events(State(state.clone()), Path(code.clone()), Query(LidQuery { lid: LID_A.into() })).await;
        assert_eq!(r.status(), StatusCode::FORBIDDEN);

        join(&state, &code, LID_A, "Sam").await;
        let resp = guest_events(State(state.clone()), Path(code.clone()), Query(LidQuery { lid: LID_A.into() })).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(resp.headers()[header::CONTENT_TYPE].to_str().unwrap().starts_with("text/event-stream"));
        let conns = |state: &AppState| state.lobbies.with(&code, |l| l.listeners.get(LID_A).map(|x| x.conns)).flatten();
        assert_eq!(conns(&state), Some(1));

        let mut frames = resp.into_body().into_data_stream();
        let first = String::from_utf8(frames.next().await.unwrap().unwrap().to_vec()).unwrap();
        assert!(first.starts_with("event: init"), "{first}");
        assert!(first.contains("\"pid\""));

        // Live updates arrive on the open stream.
        push(&state, &h, json!({ "clientId": "c", "trackId": "loc:a", "playing": true })).await;
        let next = tokio::time::timeout(Duration::from_secs(2), frames.next()).await.unwrap().unwrap().unwrap();
        let next = String::from_utf8(next.to_vec()).unwrap();
        assert!(next.starts_with("event: state") && next.contains("Resonance"), "{next}");

        // Closing the page: beacon marks them leaving; dropping the stream removes them.
        guest_leave(State(state.clone()), Path(code.clone()), body(json!({ "lid": LID_A }))).await;
        assert_eq!(conns(&state), Some(1), "still connected until the stream closes");
        drop(frames);
        assert_eq!(conns(&state), None);

        // A dropped stream without a beacon gets a grace period, then is swept.
        join(&state, &code, LID_B, "Riley").await;
        let resp = guest_events(State(state.clone()), Path(code.clone()), Query(LidQuery { lid: LID_B.into() })).await;
        drop(resp);
        state.lobbies.sweep(&state.store);
        assert!(state.lobbies.with(&code, |l| l.listeners.contains_key(LID_B)).unwrap());
        state.lobbies.with(&code, |l| l.listeners.get_mut(LID_B).unwrap().last_seen_ms -= LISTENER_GRACE_MS + 1);
        state.lobbies.sweep(&state.store);
        assert!(!state.lobbies.with(&code, |l| l.listeners.contains_key(LID_B)).unwrap());

        // A restart wipes memory, but the invite link still works.
        let rebooted = AppState { lobbies: LobbyHub::default(), ..state.clone() };
        let r = join(&rebooted, &code, LID_B, "Riley").await;
        assert_eq!(r.status(), StatusCode::OK);
        let v = body_json(r).await;
        assert_eq!(v["state"]["host"]["online"], false, "waiting for the host to reconnect");
        assert_eq!(state.store.active_lobby_for_user(uid).unwrap().unwrap().code, code);

        // Lobbies abandoned for hours close themselves for good.
        rebooted
            .lobbies
            .with(&code, |l| l.host_seen_ms = now_ms() - LOBBY_IDLE_END_MS - 60_000);
        rebooted.lobbies.sweep(&rebooted.store);
        assert_eq!(rebooted.lobbies.ensure(&rebooted.store, &code), Err(Gone::Ended));
        assert!(rebooted.store.active_lobby_for_user(uid).unwrap().is_none());
    }

    #[tokio::test]
    async fn join_floods_are_capped_and_unknown_tracks_dont_spam() {
        let state = test_state();
        let (h, _) = host_headers(&state, "guarded");
        let created = body_json(create_lobby(State(state.clone()), h.clone(), Bytes::new()).await).await;
        let code = created["lobby"]["code"].as_str().unwrap().to_string();

        // A double-submitted "Start Lobby" still yields a single lobby.
        let again = body_json(create_lobby(State(state.clone()), h.clone(), Bytes::new()).await).await;
        assert_eq!(again["lobby"]["code"], code.as_str());

        let from_ip = |ip: &str| {
            let mut hm = HeaderMap::new();
            hm.insert(crate::state::PEER_IP_HEADER, ip.parse().unwrap());
            hm
        };
        let join_from = |ip: &str, i: usize| {
            guest_join(
                State(state.clone()),
                Path(code.clone()),
                from_ip(ip),
                body(json!({ "lid": format!("flood-listener-{i:08}"), "name": format!("Bot {i}") })),
            )
        };
        for i in 0..JOINS_PER_IP {
            assert_eq!(join_from("198.51.100.7", i).await.status(), StatusCode::OK);
        }
        let r = join_from("198.51.100.7", 99).await;
        assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);
        // Someone else on another connection is unaffected...
        assert_eq!(join_from("203.0.113.5", 100).await.status(), StatusCode::OK);
        // ...and rejoining with an existing id never counts against the quota.
        assert_eq!(join_from("198.51.100.7", 0).await.status(), StatusCode::OK);

        // A track id the server can't resolve isn't streamable, and repeated
        // heartbeats about it don't rebroadcast state every time.
        push(&state, &h, json!({ "clientId": "c", "trackId": "loc:a", "playing": true })).await;
        let mut rx = state.lobbies.with(&code, |l| l.tx.subscribe()).unwrap();
        push(&state, &h, json!({ "clientId": "c", "trackId": "nope:404", "playing": true })).await;
        assert_eq!(rx.try_recv().unwrap().event, "state");
        let snap: Value = serde_json::from_str(&state.lobbies.snapshot_json(&code).unwrap()).unwrap();
        assert!(snap["playback"]["track"].is_null());
        push(&state, &h, json!({ "clientId": "c", "trackId": "nope:404", "playing": true })).await;
        assert!(rx.try_recv().is_err(), "no rebroadcast for an unchanged unknown track");
    }

    #[test]
    fn text_cleaning_and_ids() {
        assert_eq!(clean_text("  hi\n\tthere\u{0007} ", 50), "hi there");
        assert_eq!(clean_text("evil\u{202E}txt.exe\u{200B}", 50), "evil txt.exe");
        assert_eq!(clean_text("abcdef", 3), "abc");
        assert_eq!(normalize_code(" AbC234 "), Some("abc234".into()));
        assert_eq!(normalize_code("a/b"), None);
        assert_eq!(normalize_code(""), None);
        assert!(valid_lid("0123456789abcdef"));
        assert!(!valid_lid("0123456789abcde"));
        assert!(!valid_lid("0123456789abcdef!"));
        let a = public_id("code", LID_A);
        assert_eq!(a, public_id("code", LID_A));
        assert_ne!(a, public_id("other", LID_A), "ids don't correlate across lobbies");
        assert!(AVATAR_COLORS.contains(&color_for(&a).as_str()));
    }
}
