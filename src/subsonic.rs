use crate::engine::{mime_for_audio_ext, StreamEngine};
use crate::models::{AlbumSummary, ArtistSummary, Playlist, Track, User, UserTrackStats};
use crate::sources::SourceRegistry;
use crate::store::Store;
use crate::taste::TasteEngine;
use axum::extract::{Path, RawQuery, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use md5::{Digest as Md5Digest, Md5};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};

const API_VERSION: &str = "1.16.1";
const SERVER_TYPE: &str = "beatlab";
const SERVER_VERSION: &str = "0.3.0";

/// (user_id, track, started_at_unix, username, player_name)
pub type NowPlayingMap = Arc<Mutex<HashMap<i64, (Track, i64, String)>>>;

#[derive(Clone)]
pub struct AppState {
    pub store: Store,
    pub sources: SourceRegistry,
    pub engine: Arc<StreamEngine>,
    pub taste: Arc<TasteEngine>,
    pub now_playing: NowPlayingMap,
}

/// Internal header carrying the TCP peer address, stamped by `main::stamp_peer_ip`
/// (any copy sent by a client is stripped first).
pub const PEER_IP_HEADER: &str = "x-beatlab-peer-ip";

fn peer_ip(headers: &HeaderMap) -> Option<std::net::IpAddr> {
    headers.get(PEER_IP_HEADER)?.to_str().ok()?.parse().ok()
}

/// Whether forwarded-IP headers from this peer can be believed. A peer on loopback or a
/// private network is assumed to be a reverse proxy (nginx, Caddy, Docker, Fly's 6PN);
/// a public peer is talking to us directly, so its `X-Forwarded-For` is attacker-controlled.
/// Override with `BEATLAB_TRUST_PROXY=always|never`.
fn trust_forwarded_headers(peer: Option<std::net::IpAddr>) -> bool {
    use std::net::IpAddr;
    static MODE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let mode = MODE.get_or_init(|| std::env::var("BEATLAB_TRUST_PROXY").unwrap_or_default().trim().to_ascii_lowercase());
    match mode.as_str() {
        "always" | "true" | "1" => return true,
        "never" | "false" | "0" => return false,
        _ => {}
    }
    let Some(peer) = peer else {
        // No peer recorded (tests, or a server started without connect info): legacy behaviour.
        return true;
    };
    match peer {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return v4.is_loopback() || v4.is_private() || v4.is_link_local();
            }
            let first = v6.segments()[0];
            v6.is_loopback() || (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80
        }
    }
}

fn header_ip(headers: &HeaderMap, names: &[&str]) -> Option<String> {
    for name in names {
        if let Some(v) = headers.get(*name).and_then(|v| v.to_str().ok()) {
            let first = v.split(',').next().unwrap_or("").trim();
            if !first.is_empty() {
                return Some(first.chars().take(64).collect());
            }
        }
    }
    None
}

/// Best-effort client IP. Behind a trusted reverse proxy this comes from the proxy's
/// forwarding headers; otherwise it is the socket peer itself.
pub fn client_ip(headers: &HeaderMap) -> String {
    let peer = peer_ip(headers);
    if trust_forwarded_headers(peer) {
        if let Some(ip) = header_ip(headers, &["x-forwarded-for", "x-real-ip", "cf-connecting-ip", "fly-client-ip"]) {
            return ip;
        }
    }
    peer.map(|p| p.to_string()).unwrap_or_default()
}

/// Client IP used for rate limiting. Behind a trusted proxy, prefers the header the
/// hosting proxy overwrites (Fly.io's `Fly-Client-IP`), which a client cannot forge the
/// way it can prepend entries to `X-Forwarded-For`. Directly-connected clients are keyed
/// by their socket address, so spoofed headers can't dodge the limiter.
pub fn rate_limit_ip(headers: &HeaderMap) -> String {
    if trust_forwarded_headers(peer_ip(headers)) {
        if let Some(ip) = header_ip(headers, &["fly-client-ip"]) {
            return ip;
        }
    }
    client_ip(headers)
}

pub fn user_agent(headers: &HeaderMap) -> String {
    headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .chars()
        .take(256)
        .collect()
}

/// Identify a Subsonic client from the standard `c` parameter (or common
/// alternatives), falling back to a short token from the User-Agent.
pub fn subsonic_client_name(params: &HashMap<String, String>, headers: &HeaderMap) -> String {
    for key in ["c", "name", "client", "player"] {
        if let Some(v) = params.get(key) {
            let v = v.trim();
            if !v.is_empty() {
                return v.chars().take(64).collect();
            }
        }
    }
    let ua = user_agent(headers);
    if ua.is_empty() || ua.to_ascii_lowercase().contains("mozilla/") {
        return String::new();
    }
    ua.split_whitespace()
        .next()
        .unwrap_or("")
        .chars()
        .take(64)
        .collect()
}

#[derive(Clone)]
pub struct XmlElem {
    pub name: String,
    pub attrs: Vec<(String, Value)>,
    pub children: Vec<XmlElem>,
    pub text: Option<String>,
}

pub trait IntoXmlAttr {
    fn into_xml_val(self) -> Value;
}

impl IntoXmlAttr for &str {
    fn into_xml_val(self) -> Value { Value::String(self.to_string()) }
}
impl IntoXmlAttr for String {
    fn into_xml_val(self) -> Value { Value::String(self) }
}
impl IntoXmlAttr for &String {
    fn into_xml_val(self) -> Value { Value::String(self.clone()) }
}
impl IntoXmlAttr for i32 {
    fn into_xml_val(self) -> Value { Value::from(self) }
}
impl IntoXmlAttr for i64 {
    fn into_xml_val(self) -> Value { Value::from(self) }
}
impl IntoXmlAttr for usize {
    fn into_xml_val(self) -> Value { Value::from(self) }
}
impl IntoXmlAttr for f64 {
    fn into_xml_val(self) -> Value { Value::from(self) }
}
impl IntoXmlAttr for bool {
    fn into_xml_val(self) -> Value { Value::from(self) }
}

impl XmlElem {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            attrs: Vec::new(),
            children: Vec::new(),
            text: None,
        }
    }

    pub fn attr(mut self, k: &str, v: impl IntoXmlAttr) -> Self {
        let val = v.into_xml_val();
        if let Value::String(ref s) = val {
            if s.is_empty() {
                return self;
            }
        }
        self.attrs.push((k.to_string(), val));
        self
    }

    pub fn child(mut self, c: XmlElem) -> Self {
        self.children.push(c);
        self
    }

    pub fn text(mut self, t: impl Into<String>) -> Self {
        self.text = Some(t.into());
        self
    }

    pub fn to_xml(&self) -> String {
        let mut s = format!("<{}", self.name);
        for (k, v) in &self.attrs {
            let v_str = match v {
                Value::String(st) => st.clone(),
                other => other.to_string(),
            };
            s.push_str(&format!(r#" {}="{}""#, k, xml_escape(&v_str)));
        }
        if self.children.is_empty() && self.text.is_none() {
            s.push_str("/>");
            return s;
        }
        s.push('>');
        if let Some(ref t) = self.text {
            s.push_str(&xml_escape(t));
        }
        for c in &self.children {
            s.push_str(&c.to_xml());
        }
        s.push_str(&format!("</{}>", self.name));
        s
    }

    pub fn to_json(&self) -> Value {
        if self.attrs.is_empty() && self.children.is_empty() && self.name != "line" {
            if let Some(ref t) = self.text {
                if matches!(self.name.as_str(), "versions" | "folder") {
                    if let Ok(n) = t.parse::<i64>() {
                        return Value::from(n);
                    }
                }
                return Value::String(t.clone());
            }
        }
        let mut obj = Map::new();
        for (k, v) in &self.attrs {
            obj.insert(k.clone(), v.clone());
        }
        if let Some(ref t) = self.text {
            obj.insert("value".into(), Value::String(t.clone()));
        }
        let mut groups: HashMap<String, Vec<Value>> = HashMap::new();
        let mut order = Vec::new();
        for c in &self.children {
            if !groups.contains_key(&c.name) {
                order.push(c.name.clone());
            }
            groups.entry(c.name.clone()).or_default().push(c.to_json());
        }
        for name in order {
            let Some(items) = groups.remove(&name) else {
                continue;
            };
            if items.len() == 1 && !is_subsonic_array_child(&self.name, &name) {
                if let Some(single) = items.into_iter().next() {
                    obj.insert(name, single);
                }
            } else {
                obj.insert(name, Value::Array(items));
            }
        }
        Value::Object(obj)
    }
}

fn is_subsonic_array_child(parent: &str, child: &str) -> bool {
    if parent == "subsonic-response" {
        return child == "openSubsonicExtensions";
    }
    matches!(
        child,
        "song"
            | "album"
            | "artist"
            | "index"
            | "playlist"
            | "entry"
            | "genre"
            | "musicFolder"
            | "internetRadioStation"
            | "user"
            | "openSubsonicExtensions"
            | "versions"
            | "folder"
            | "child"
            | "match"
            | "share"
            | "similarArtist"
            | "structuredLyrics"
            | "line"
            | "nowPlaying"
    )
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn iso8601(ts: i64) -> String {
    DateTime::<Utc>::from_timestamp(ts.max(0), 0)
        .unwrap_or_default()
        .to_rfc3339()
}

pub fn parse_multi_params(raw: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    for part in raw.split('&') {
        if part.is_empty() {
            continue;
        }
        let (k_raw, v_raw) = part.split_once('=').unwrap_or((part, ""));
        let k = urlencoding::decode(k_raw.replace('+', " ").as_str())
            .map(|c| c.into_owned())
            .unwrap_or_else(|_| k_raw.to_string());
        let v = urlencoding::decode(v_raw.replace('+', " ").as_str())
            .map(|c| c.into_owned())
            .unwrap_or_else(|_| v_raw.to_string());
        pairs.push((k, v));
    }
    pairs
}

fn params_map(pairs: &[(String, String)]) -> HashMap<String, String> {
    let mut m = HashMap::new();
    for (k, v) in pairs {
        m.insert(k.clone(), v.clone());
    }
    m
}

fn multi_vals(pairs: &[(String, String)], key: &str) -> Vec<String> {
    pairs
        .iter()
        .filter(|(k, _)| k == key)
        .map(|(_, v)| v.clone())
        .filter(|v| !v.is_empty())
        .collect()
}

fn respond(params: &HashMap<String, String>, ok: bool, inner: Option<XmlElem>) -> Response {
    respond_children(params, ok, inner.into_iter().collect())
}

fn respond_children(params: &HashMap<String, String>, ok: bool, children: Vec<XmlElem>) -> Response {
    let mut root = XmlElem::new("subsonic-response")
        .attr("status", if ok { "ok" } else { "failed" })
        .attr("version", API_VERSION)
        .attr("type", SERVER_TYPE)
        .attr("serverVersion", SERVER_VERSION)
        .attr("openSubsonic", true);
    for c in children {
        root = root.child(c);
    }

    let fmt = params.get("f").map(|s| s.as_str()).unwrap_or("xml");
    match fmt {
        "json" => {
            let payload = serde_json::json!({ "subsonic-response": root.to_json() });
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
                payload.to_string(),
            )
                .into_response()
        }
        "jsonp" => {
            let raw_cb = params.get("callback").map(|s| s.as_str()).unwrap_or("callback");
            let cb: String = raw_cb
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '.')
                .collect();
            let cb_name = if cb.is_empty() { "callback" } else { &cb };
            let payload = serde_json::json!({ "subsonic-response": root.to_json() });
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/javascript; charset=utf-8")],
                format!("{}({})", cb_name, payload),
            )
                .into_response()
        }
        _ => {
            let xml = format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>{}"#,
                root.attr("xmlns", "http://subsonic.org/restapi").to_xml()
            );
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/xml; charset=utf-8")],
                xml,
            )
                .into_response()
        }
    }
}

fn error_resp(params: &HashMap<String, String>, code: i32, msg: &str) -> Response {
    respond(
        params,
        false,
        Some(XmlElem::new("error").attr("code", code).attr("message", msg)),
    )
}

fn authenticate_subsonic(
    state: &AppState,
    params: &HashMap<String, String>,
    headers: &HeaderMap,
) -> Result<User, (i32, &'static str)> {
    let rate_keys = crate::auth::rate_keys("apikey", "", &rate_limit_ip(headers));
    if crate::auth::throttled_for(&rate_keys).is_some() {
        return Err((40, "Too many failed sign-in attempts from this address. Try again later."));
    }
    let user = match authenticate_subsonic_credentials(state, params) {
        Ok(u) => u,
        Err(e) => {
            // Only bad credentials count; a missing-parameter probe (code 10) doesn't.
            if e.0 == 40 {
                crate::auth::record_failure(&rate_keys);
            }
            return Err(e);
        }
    };

    // Subsonic clients re-send credentials on every request rather than using
    // a server-issued token. Track them as sessions so they appear in the
    // Settings > Sessions tab, and honour a user's revocation.
    let client = subsonic_client_name(params, headers);
    if !client.is_empty() {
        let (_, revoked) = state
            .store
            .touch_subsonic_client(
                user.id,
                &client,
                &user_agent(headers),
                &client_ip(headers),
            )
            .map_err(|_| (40, "Could not verify client session"))?;
        if revoked {
            return Err((40, "This client session was revoked. Reconnect to sign in again."));
        }
    }
    Ok(user)
}

fn authenticate_subsonic_credentials(
    state: &AppState,
    params: &HashMap<String, String>,
) -> Result<User, (i32, &'static str)> {
    // OpenSubsonic's dedicated apiKey parameter is preferred.
    if let Some(key) = params.get("apiKey").filter(|k| !k.trim().is_empty()) {
        return state
            .store
            .get_user_by_api_key(key)
            .ok()
            .flatten()
            .ok_or((40, "Invalid API key"));
    }

    // Most Subsonic clients only offer a password field. That field must now
    // contain the account's API key from Settings → Apps & API.
    if let Some(raw_key) = params.get("p").filter(|k| !k.trim().is_empty()) {
        let key = decode_subsonic_password(raw_key).map_err(|_| (40, "Malformed API key"))?;
        let user = state
            .store
            .get_user_by_api_key(&key)
            .ok()
            .flatten()
            .ok_or((40, "Invalid API key"))?;
        if let Some(name) = params.get("u").map(|s| s.trim()).filter(|s| !s.is_empty()) {
            if !name.eq_ignore_ascii_case(&user.username) {
                return Err((40, "API key does not match that username"));
            }
        }
        return Ok(user);
    }

    // Standard Subsonic 1.13+ salted token auth (`t = md5(apiKey + s)`).
    // Verified strictly against the account's API key (NEVER the account password),
    // so both standard Subsonic clients (pasting the API key into the password box)
    // and Navidrome clients (using `subsonicToken`/`subsonicSalt` from `/auth/login`)
    // work seamlessly while keeping account passwords disabled.
    if params.contains_key("t") || params.contains_key("s") {
        let username = params.get("u").map(|s| s.trim()).unwrap_or("");
        let token = params.get("t").map(|s| s.trim()).unwrap_or("");
        let salt = params.get("s").map(|s| s.as_str()).unwrap_or("");
        if username.is_empty() || token.is_empty() || salt.is_empty() {
            return Err((
                40,
                "Invalid token credentials. Use the API key from Settings → Apps & API as the password.",
            ));
        }
        let Some((user, api_key)) = state
            .store
            .get_user_and_api_key_by_name(username)
            .ok()
            .flatten()
        else {
            return Err((
                40,
                "Invalid API key token. Use the API key from Settings → Apps & API as the password.",
            ));
        };
        let mut h = Md5::new();
        h.update(format!("{}{}", api_key, salt).as_bytes());
        let expected = hex::encode(h.finalize());
        if crate::models::constant_time_eq(&expected, &token.to_ascii_lowercase()) {
            return Ok(user);
        }
        return Err((
            40,
            "Invalid API key token. Account passwords are disabled — use the API key from Settings → Apps & API as the password.",
        ));
    }

    Err((
        10,
        "Missing API key. Use the apiKey parameter, or paste the API key into the password field.",
    ))
}

fn decode_subsonic_password(p: &str) -> Result<String, (i32, &'static str)> {
    if let Some(hex_part) = p.strip_prefix("enc:") {
        let bytes = hex::decode(hex_part).map_err(|_| (40, "Malformed hex password"))?;
        let s = String::from_utf8(bytes).map_err(|_| (40, "Malformed UTF-8 password"))?;
        Ok(s)
    } else {
        Ok(p.to_string())
    }
}

pub async fn dispatch(
    State(state): State<AppState>,
    Path(action): Path<String>,
    RawQuery(raw_q): RawQuery,
    headers: HeaderMap,
    body: String,
) -> Response {
    let mut pairs = raw_q
        .as_deref()
        .map(parse_multi_params)
        .unwrap_or_default();
    if !body.trim().is_empty() && !body.trim_start().starts_with('{') {
        pairs.extend(parse_multi_params(&body));
    }
    let params = params_map(&pairs);

    let endpoint = action.trim_end_matches(".view").trim_start_matches('/');

    // Public ping only works when authenticated in Subsonic spec (so clients detect auth failure early!).
    let user = match authenticate_subsonic(&state, &params, &headers) {
        Ok(u) => u,
        Err((code, msg)) => return error_resp(&params, code, msg),
    };

    let uid = user.id;

    match endpoint {
        "ping" => respond(&params, true, None),
        "getLicense" => respond(
            &params,
            true,
            Some(
                XmlElem::new("license")
                    .attr("valid", true)
                    .attr("email", "open@beatlab.local")
                    .attr("licenseExpires", "2099-12-31T23:59:59"),
            ),
        ),
        "getOpenSubsonicExtensions" => respond_children(
            &params,
            true,
            vec![
                XmlElem::new("openSubsonicExtensions")
                    .attr("name", "apiKeyAuthentication")
                    .child(XmlElem::new("versions").text("1")),
                XmlElem::new("openSubsonicExtensions")
                    .attr("name", "transcodeOffset")
                    .child(XmlElem::new("versions").text("1")),
                XmlElem::new("openSubsonicExtensions")
                    .attr("name", "songLyrics")
                    .child(XmlElem::new("versions").text("1")),
                XmlElem::new("openSubsonicExtensions")
                    .attr("name", "formPost")
                    .child(XmlElem::new("versions").text("1")),
                XmlElem::new("openSubsonicExtensions")
                    .attr("name", "indexBasedQueue")
                    .child(XmlElem::new("versions").text("1")),
            ],
        ),
        "tokenInfo" => respond(
            &params,
            true,
            Some(XmlElem::new("tokenInfo").attr("username", &user.username)),
        ),

        "getUser" => {
            let target = params
                .get("username")
                .map(|s| s.as_str())
                .unwrap_or(&user.username);
            if !target.eq_ignore_ascii_case(&user.username) {
                return error_resp(&params, 50, "You can only view your own user profile");
            }
            respond(&params, true, Some(user_to_xml(&user)))
        }
        "getUsers" => respond(
            &params,
            true,
            Some(XmlElem::new("users").child(user_to_xml(&user))),
        ),
        // Client-side account creation is disabled (matching Navidrome signup):
        // accounts are created in the Beatlab web player where 2FA can be configured.
        "createUser" => error_resp(
            &params,
            50,
            "Create an account in the Beatlab web player, then use its API key in your music client.",
        ),
        // Clients authenticate with the API key, a credential that lives in
        // third-party apps. It must not be able to replace the account
        // password (which would bypass the web player's current-password and
        // two-factor checks), so password changes stay in the web player.
        "changePassword" => error_resp(
            &params,
            50,
            "Change your password in the Beatlab web player (Settings → Security)",
        ),

        "getMusicFolders" => respond(
            &params,
            true,
            Some(
                XmlElem::new("musicFolders").child(
                    XmlElem::new("musicFolder")
                        .attr("id", "1")
                        .attr("name", "Beatlab Library"),
                ),
            ),
        ),
        "getScanStatus" => {
            let count = state.store.count_tracks().unwrap_or(0);
            respond(
                &params,
                true,
                Some(
                    XmlElem::new("scanStatus")
                        .attr("scanning", false)
                        .attr("count", count),
                ),
            )
        }
        "startScan" => {
            let local = state.sources.local.clone();
            tokio::task::spawn_blocking(move || local.scan());
            let count = state.store.count_tracks().unwrap_or(0);
            respond(
                &params,
                true,
                Some(
                    XmlElem::new("scanStatus")
                        .attr("scanning", true)
                        .attr("count", count),
                ),
            )
        }

        "getIndexes" => handle_get_indexes(&state, &params, uid, false),
        "getArtists" => handle_get_indexes(&state, &params, uid, true),

        "getMusicDirectory" => {
            let id = params.get("id").map(|s| s.as_str()).unwrap_or("1");
            handle_music_directory(&state, &params, uid, id).await
        }
        "getArtist" => {
            let Some(id) = params.get("id") else {
                return error_resp(&params, 10, "Missing id");
            };
            let Some((artist, _top, mut albums, _related)) =
                state.taste.get_or_hydrate_artist(id, uid).await
            else {
                return error_resp(&params, 70, "Artist not found");
            };
            let followed = state.store.followed_artist_at_map(uid).unwrap_or_default();
            let saved_albums = state.store.saved_album_at_map(uid).unwrap_or_default();
            let user_albums: HashMap<String, (i64, i64)> = state
                .store
                .build_album_summaries(uid)
                .unwrap_or_default()
                .into_iter()
                .map(|a| (a.id, (a.play_count, a.last_played)))
                .collect();
            let mut elem = XmlElem::new("artist")
                .attr("id", &artist.id)
                .attr("name", &artist.name)
                .attr("coverArt", &artist.id)
                .attr("albumCount", albums.len())
                .attr("artistImageUrl", &artist.cover_url);
            if let Some(&at) = followed.get(&artist.id) {
                elem = elem.attr("starred", iso8601_or_now(at));
            }
            for alb in &mut albums {
                if let Some(&(pc, lp)) = user_albums.get(&alb.id) {
                    if alb.play_count == 0 {
                        alb.play_count = pc;
                    }
                    if alb.last_played == 0 {
                        alb.last_played = lp;
                    }
                }
                elem = elem.child(album_to_xml(alb, &saved_albums, "album"));
            }
            respond(&params, true, Some(elem))
        }
        "getArtistInfo" | "getArtistInfo2" => {
            let id = params.get("id").map(|s| s.as_str()).unwrap_or("");
            let count = parse_usize(&params, "count", 20).min(50);
            let is_v2 = endpoint == "getArtistInfo2";
            let root_name = if is_v2 { "artistInfo2" } else { "artistInfo" };
            if let Some((artist, _, _, related)) =
                state.taste.get_or_hydrate_artist(id, uid).await
            {
                let followed = state.store.followed_artist_at_map(uid).unwrap_or_default();
                let mut elem = XmlElem::new(root_name);
                if !artist.bio.is_empty() {
                    elem = elem.child(XmlElem::new("biography").text(&artist.bio));
                }
                if !artist.cover_url.is_empty() {
                    elem = elem
                        .child(XmlElem::new("smallImageUrl").text(&artist.cover_url))
                        .child(XmlElem::new("mediumImageUrl").text(&artist.cover_url))
                        .child(XmlElem::new("largeImageUrl").text(&artist.cover_url));
                }
                for rel in related.into_iter().take(count) {
                    let mut s = XmlElem::new("similarArtist")
                        .attr("id", &rel.id)
                        .attr("name", &rel.name)
                        .attr("coverArt", &rel.id)
                        .attr("albumCount", rel.album_count)
                        .attr("artistImageUrl", &rel.cover_url);
                    if let Some(&at) = followed.get(&rel.id) {
                        s = s.attr("starred", iso8601_or_now(at));
                    }
                    elem = elem.child(s);
                }
                respond(&params, true, Some(elem))
            } else {
                respond(&params, true, Some(XmlElem::new(root_name)))
            }
        }
        "getAlbum" => {
            let Some(id) = params.get("id") else {
                return error_resp(&params, 10, "Missing id");
            };
            let Some((mut alb, tracks)) = state.taste.get_or_hydrate_album(id).await else {
                return error_resp(&params, 70, "Album not found");
            };
            let starred = state.store.starred_at_map(uid).unwrap_or_default();
            let saved_albums = state.store.saved_album_at_map(uid).unwrap_or_default();
            let stats = state.store.track_stats_map(uid).unwrap_or_default();
            let ratings = state.store.user_ratings(uid).unwrap_or_default();
            hydrate_album_user_stats(&mut alb, &tracks, &stats);
            let mut elem = album_to_xml(&alb, &saved_albums, "album");
            for t in &tracks {
                elem = elem.child(track_to_xml(
                    t,
                    starred.get(&t.id).copied(),
                    stats.get(&t.id),
                    ratings.get(&t.id).copied(),
                    "song",
                ));
            }
            respond(&params, true, Some(elem))
        }
        "getAlbumInfo" | "getAlbumInfo2" => {
            let root_name = "albumInfo";
            let id = params.get("id").map(|s| s.as_str()).unwrap_or("");
            if let Some((alb, _)) = state.taste.get_or_hydrate_album(id).await {
                let mut elem = XmlElem::new(root_name);
                if !alb.cover_url.is_empty() {
                    elem = elem
                        .child(XmlElem::new("smallImageUrl").text(&alb.cover_url))
                        .child(XmlElem::new("mediumImageUrl").text(&alb.cover_url))
                        .child(XmlElem::new("largeImageUrl").text(&alb.cover_url));
                }
                if !alb.label.is_empty() {
                    elem = elem.child(XmlElem::new("notes").text(&format!("Label: {}", alb.label)));
                }
                respond(&params, true, Some(elem))
            } else {
                respond(&params, true, Some(XmlElem::new(root_name)))
            }
        }
        "getSong" => {
            let Some(id) = params.get("id") else {
                return error_resp(&params, 10, "Missing id");
            };
            let Some(t) = state.store.get_track(id).ok().flatten() else {
                return error_resp(&params, 70, "Song not found");
            };
            let starred = state.store.starred_at_map(uid).unwrap_or_default();
            let st = state.store.get_user_track_stats(uid, &t.id).ok();
            let rating = state.store.user_ratings(uid).ok().and_then(|m| m.get(&t.id).copied());
            respond(
                &params,
                true,
                Some(track_to_xml(&t, starred.get(&t.id).copied(), st.as_ref(), rating, "song")),
            )
        }
        "getGenres" => {
            let genres = state.store.genres().unwrap_or_default();
            let mut root = XmlElem::new("genres");
            for (name, sc, ac) in genres {
                root = root.child(
                    XmlElem::new("genre")
                        .attr("songCount", sc)
                        .attr("albumCount", ac)
                        .text(name),
                );
            }
            respond(&params, true, Some(root))
        }
        "getSongsByGenre" => {
            let genre = params.get("genre").map(|s| s.as_str()).unwrap_or("");
            let count = parse_usize(&params, "count", 50).min(200);
            let offset = parse_usize(&params, "offset", 0);
            let tracks = hydrate_genre_tracks(&state, uid, genre, count, offset).await;
            let elem = songs_container(&state, uid, "songsByGenre", "song", &tracks);
            respond(&params, true, Some(elem))
        }
        "getRandomSongs" => {
            let size = parse_usize(&params, "size", 20).min(100);
            let genre = params
                .get("genre")
                .map(|s| s.trim())
                .filter(|s| !s.is_empty());
            let from_year = params.get("fromYear").and_then(|s| s.parse::<i32>().ok());
            let to_year = params.get("toYear").and_then(|s| s.parse::<i32>().ok());
            let mut pool = if let Some(g) = genre {
                let mut items = hydrate_genre_tracks(&state, uid, g, (size * 3).max(60), 0).await;
                use rand::seq::SliceRandom;
                let mut rng = rand::thread_rng();
                items.shuffle(&mut rng);
                items
            } else {
                let raw = state
                    .store
                    .random_tracks((size * 3).max(60), None)
                    .unwrap_or_default();
                dedup_tracks_by_song_key(filter_visible_tracks(&state, uid, raw))
            };
            if from_year.is_some() || to_year.is_some() {
                let lo = from_year.unwrap_or(0);
                let hi = to_year.unwrap_or(9999);
                pool.retain(|t| t.year >= lo && t.year <= hi);
            }
            pool.truncate(size);
            let elem = songs_container(&state, uid, "randomSongs", "song", &pool);
            respond(&params, true, Some(elem))
        }
        "getTopSongs" => {
            let artist_name = params.get("artist").map(|s| s.trim()).unwrap_or("");
            let count = parse_usize(&params, "count", 25).min(100);
            let canonical_id = state
                .store
                .build_artist_summaries()
                .unwrap_or_default()
                .into_iter()
                .find(|a| a.name.eq_ignore_ascii_case(artist_name))
                .map(|a| a.id)
                .unwrap_or_else(|| format!("ar:{}", crate::models::slug(artist_name)));
            let tracks: Vec<Track> = if let Some((_, top, _, _)) = state
                .taste
                .get_or_hydrate_artist_with_hint(&canonical_id, uid, Some(artist_name))
                .await
            {
                filter_visible_tracks(&state, uid, top)
                    .into_iter()
                    .take(count)
                    .collect()
            } else {
                let fallback = state
                    .store
                    .search_tracks(artist_name, count * 2, 0)
                    .unwrap_or_default();
                dedup_tracks_by_song_key(filter_visible_tracks(&state, uid, fallback))
                    .into_iter()
                    .take(count)
                    .collect()
            };
            let elem = songs_container(&state, uid, "topSongs", "song", &tracks);
            respond(&params, true, Some(elem))
        }
        "getSimilarSongs" | "getSimilarSongs2" => {
            let root_name = if endpoint == "getSimilarSongs2" {
                "similarSongs2"
            } else {
                "similarSongs"
            };
            let id = params.get("id").map(|s| s.trim()).unwrap_or("");
            let count = parse_usize(&params, "count", 25).min(100);
            let tracks = similar_songs_for_entity(&state, uid, id, count).await;
            let elem = songs_container(&state, uid, root_name, "song", &tracks);
            respond(&params, true, Some(elem))
        }
        "getAlbumList" | "getAlbumList2" => {
            let root_name = if endpoint == "getAlbumList2" {
                "albumList2"
            } else {
                "albumList"
            };
            let list_type = params.get("type").map(|s| s.as_str()).unwrap_or("newest");
            let size = parse_usize(&params, "size", 20).min(100);
            let offset = parse_usize(&params, "offset", 0);
            let albums = get_sorted_albums(&state, uid, list_type, &params, size, offset);
            let saved_albums = state.store.saved_album_at_map(uid).unwrap_or_default();
            let mut root = XmlElem::new(root_name);
            for a in albums {
                root = root.child(album_to_xml(&a, &saved_albums, "album"));
            }
            respond(&params, true, Some(root))
        }
        "search" | "search2" | "search3" => {
            handle_search(&state, &params, uid, endpoint).await
        }

        "getStarred" | "getStarred2" => {
            let root_name = if endpoint == "getStarred2" {
                "starred2"
            } else {
                "starred"
            };
            let starred_tracks = state.store.starred_tracks(uid).unwrap_or_default();
            let stats = state.store.track_stats_map(uid).unwrap_or_default();
            let ratings = state.store.user_ratings(uid).unwrap_or_default();
            let saved_albums = state.store.saved_album_at_map(uid).unwrap_or_default();
            let followed_artists = state.store.followed_artist_at_map(uid).unwrap_or_default();

            let mut root = XmlElem::new(root_name);
            for a in state.store.build_artist_summaries().unwrap_or_default() {
                if followed_artists.contains_key(&a.id) {
                    root = root.child(artist_to_xml(&a, &followed_artists));
                }
            }
            for alb in state.store.build_album_summaries(uid).unwrap_or_default() {
                if saved_albums.contains_key(&alb.id) {
                    root = root.child(album_to_xml(&alb, &saved_albums, "album"));
                }
            }
            for (t, at) in starred_tracks {
                root = root.child(track_to_xml(
                    &t,
                    Some(at),
                    stats.get(&t.id),
                    ratings.get(&t.id).copied(),
                    "song",
                ));
            }
            respond(&params, true, Some(root))
        }
        "star" => {
            for id in multi_vals(&pairs, "id") {
                if id.starts_with("al:") {
                    let _ = state.store.save_album(uid, &id);
                    state.taste.mark_taste_dirty(uid);
                } else if id.starts_with("ar:") {
                    let _ = state.store.follow_artist(uid, &id);
                    state.taste.mark_taste_dirty(uid);
                } else {
                    let _ = state.store.star_track(uid, &id);
                    state.taste.learn_signal(uid, &id, "star", 1.0);
                }
            }
            for aid in multi_vals(&pairs, "albumId") {
                let _ = state.store.save_album(uid, &aid);
                state.taste.mark_taste_dirty(uid);
            }
            for arid in multi_vals(&pairs, "artistId") {
                let _ = state.store.follow_artist(uid, &arid);
                state.taste.mark_taste_dirty(uid);
            }
            respond(&params, true, None)
        }
        "unstar" => {
            for id in multi_vals(&pairs, "id") {
                if id.starts_with("al:") {
                    let _ = state.store.unsave_album(uid, &id);
                    state.taste.mark_taste_dirty(uid);
                } else if id.starts_with("ar:") {
                    let _ = state.store.unfollow_artist(uid, &id);
                    state.taste.mark_taste_dirty(uid);
                } else {
                    let _ = state.store.unstar_track(uid, &id);
                    state.taste.learn_signal(uid, &id, "unstar", 0.0);
                }
            }
            for aid in multi_vals(&pairs, "albumId") {
                let _ = state.store.unsave_album(uid, &aid);
                state.taste.mark_taste_dirty(uid);
            }
            for arid in multi_vals(&pairs, "artistId") {
                let _ = state.store.unfollow_artist(uid, &arid);
                state.taste.mark_taste_dirty(uid);
            }
            respond(&params, true, None)
        }
        "setRating" => {
            let Some(id) = params.get("id") else {
                return error_resp(&params, 10, "Missing id");
            };
            let rating: i32 = params
                .get("rating")
                .and_then(|s| s.parse::<i32>().ok())
                .unwrap_or(0)
                .clamp(0, 5);
            let _ = state.store.set_rating(uid, id, rating);
            if rating > 0 {
                state.taste.learn_signal(uid, id, "rate", rating as f64);
            } else {
                state.taste.mark_taste_dirty(uid);
            }
            respond(&params, true, None)
        }

        "getPlaylists" => {
            refresh_user_mixes(&state, uid).await;
            let mut playlists = state.store.list_playlists(uid, true).unwrap_or_default();
            let mut root = XmlElem::new("playlists");
            for p in &mut playlists {
                adjust_auto_playlist_summary(&state, uid, p);
                root = root.child(playlist_to_xml(p));
            }
            respond(&params, true, Some(root))
        }
        "getPlaylist" => {
            let Some(id) = params.get("id") else {
                return error_resp(&params, 10, "Missing id");
            };
            let Some(p) = state.store.get_playlist(id).ok().flatten() else {
                return error_resp(&params, 70, "Playlist not found");
            };
            if !p.public && p.user_id != uid {
                return error_resp(&params, 50, "Permission denied");
            }
            let mut p = if p.is_auto && p.user_id == uid && state.store.count_tracks().unwrap_or(0) > 0 {
                if p.kind.starts_with("mixed_for_you:") {
                    state.taste.refresh_mixed_for_you_if_stale_async(uid).await;
                }
                state.taste.refresh_auto_playlists_if_stale_async(uid, 900).await;
                state.store.get_playlist(id).ok().flatten().unwrap_or(p)
            } else {
                p
            };
            let raw_tracks = state.store.playlist_tracks(id).unwrap_or_default();
            let tracks = if p.is_auto {
                filter_visible_tracks(&state, uid, raw_tracks)
            } else {
                raw_tracks
            };
            if p.is_auto {
                p.track_count = tracks.len();
                p.duration = tracks.iter().map(|t| t.duration.max(0)).sum();
                if p.cover_track_id
                    .as_ref()
                    .is_some_and(|cid| !tracks.iter().any(|t| &t.id == cid))
                {
                    p.cover_track_id = tracks.first().map(|t| t.id.clone());
                }
            }
            let starred = state.store.starred_at_map(uid).unwrap_or_default();
            let stats = state.store.track_stats_map(uid).unwrap_or_default();
            let ratings = state.store.user_ratings(uid).unwrap_or_default();
            let mut elem = playlist_to_xml(&p);
            for t in tracks {
                elem = elem.child(track_to_xml(
                    &t,
                    starred.get(&t.id).copied(),
                    stats.get(&t.id),
                    ratings.get(&t.id).copied(),
                    "entry",
                ));
            }
            respond(&params, true, Some(elem))
        }
        "createPlaylist" => {
            let song_ids = multi_vals(&pairs, "songId");
            if let Some(pid) = params.get("playlistId") {
                let Some(existing) = state.store.get_playlist(pid).ok().flatten() else {
                    return error_resp(&params, 70, "Playlist not found");
                };
                if existing.user_id != uid {
                    return error_resp(&params, 50, "Permission denied");
                }
                let _ = state.store.update_playlist(
                    pid,
                    params.get("name").map(|s| s.as_str()),
                    None,
                    None,
                );
                let _ = state.store.replace_playlist_tracks(pid, &song_ids);
                if !existing.is_auto && !song_ids.is_empty() {
                    state.taste.record_playlist_additions(uid, &song_ids);
                }
                let Some(updated) = state.store.get_playlist(pid).ok().flatten() else {
                    return error_resp(&params, 70, "Playlist not found");
                };
                respond(&params, true, Some(playlist_to_xml(&updated)))
            } else {
                let name = params
                    .get("name")
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .unwrap_or("New Playlist");
                match state.store.create_playlist(uid, name, "", false, &song_ids) {
                    Ok(p) => {
                        if !song_ids.is_empty() {
                            state.taste.record_playlist_additions(uid, &song_ids);
                        }
                        respond(&params, true, Some(playlist_to_xml(&p)))
                    }
                    Err(e) => error_resp(&params, 0, &e.to_string()),
                }
            }
        }
        "updatePlaylist" => {
            let Some(pid) = params.get("playlistId") else {
                return error_resp(&params, 10, "Missing playlistId");
            };
            let Some(existing) = state.store.get_playlist(pid).ok().flatten() else {
                return error_resp(&params, 70, "Playlist not found");
            };
            if existing.user_id != uid {
                return error_resp(&params, 50, "Permission denied");
            }
            let public = params.get("public").map(|v| v == "true" || v == "1");
            let _ = state.store.update_playlist(
                pid,
                params.get("name").map(|s| s.as_str()),
                params.get("comment").map(|s| s.as_str()),
                public,
            );
            let remove_idxs: Vec<usize> = multi_vals(&pairs, "songIndexToRemove")
                .into_iter()
                .filter_map(|s| s.parse().ok())
                .collect();
            if !remove_idxs.is_empty() {
                let current = state.store.playlist_tracks(pid).unwrap_or_default();
                let removed_ids: Vec<String> = remove_idxs
                    .iter()
                    .filter_map(|&idx| current.get(idx).map(|t| t.id.clone()))
                    .collect();
                let _ = state.store.remove_playlist_positions(pid, &remove_idxs);
                if !existing.is_auto && !removed_ids.is_empty() {
                    state.taste.record_playlist_removals(uid, &removed_ids);
                }
            }
            let add_ids = multi_vals(&pairs, "songIdToAdd");
            if !add_ids.is_empty() {
                let _ = state.store.add_tracks_to_playlist(pid, &add_ids);
                if !existing.is_auto {
                    state.taste.record_playlist_additions(uid, &add_ids);
                }
            }
            respond(&params, true, None)
        }
        "deletePlaylist" => {
            let Some(pid) = params.get("id") else {
                return error_resp(&params, 10, "Missing id");
            };
            let Some(existing) = state.store.get_playlist(pid).ok().flatten() else {
                return error_resp(&params, 70, "Playlist not found");
            };
            if existing.user_id != uid {
                return error_resp(&params, 50, "Permission denied");
            }
            let _ = state.store.delete_playlist(pid);
            if !existing.is_auto {
                state.taste.mark_taste_dirty(uid);
            }
            respond(&params, true, None)
        }

        "scrobble" => {
            let ids = multi_vals(&pairs, "id");
            let submission = params
                .get("submission")
                .map(|v| v != "false" && v != "0")
                .unwrap_or(true);
            for id in &ids {
                let kind = if submission { "complete" } else { "play" };
                state.taste.record_event(uid, id, kind, 1.0);
                if let Ok(Some(t)) = state.store.get_track(id) {
                    spawn_genre_enrichment_if_needed(&state, &t);
                    // Only `submission=false` is a live "now playing" notification;
                    // `submission=true` can be a completed track or an offline scrobble batch.
                    if !submission {
                        let mut np = state.now_playing.lock().unwrap_or_else(|e| e.into_inner());
                        np.insert(uid, (t, Utc::now().timestamp(), user.username.clone()));
                    }
                }
            }
            respond(&params, true, None)
        }
        "getNowPlaying" => {
            let cutoff = Utc::now().timestamp() - 600;
            // Scope to the signed-in user so other listeners' activity is never leaked.
            let own: Option<(Track, i64, String)> = {
                let np = state.now_playing.lock().unwrap_or_else(|e| e.into_inner());
                np.get(&uid)
                    .filter(|(_, ts, _)| *ts >= cutoff)
                    .cloned()
            };
            let mut root = XmlElem::new("nowPlaying");
            if let Some((t, ts, uname)) = own {
                let starred = state.store.starred_at_map(uid).unwrap_or_default();
                let stats = state.store.track_stats_map(uid).unwrap_or_default();
                let ratings = state.store.user_ratings(uid).unwrap_or_default();
                let now = Utc::now().timestamp();
                let elem = track_to_xml(
                    &t,
                    starred.get(&t.id).copied(),
                    stats.get(&t.id),
                    ratings.get(&t.id).copied(),
                    "entry",
                )
                .attr("username", uname)
                .attr("minutesAgo", ((now - ts).max(0)) / 60);
                root = root.child(elem);
            }
            respond(&params, true, Some(root))
        }

        "getPlayQueue" | "getPlayQueueByIndex" => {
            let root_name = if endpoint == "getPlayQueueByIndex" {
                "playQueueByIndex"
            } else {
                "playQueue"
            };
            let Some((q, tracks, resolved_idx, resolved_id)) =
                state.store.load_play_queue_tracks(uid).ok().flatten()
            else {
                return respond(&params, true, None);
            };
            let starred = state.store.starred_at_map(uid).unwrap_or_default();
            let stats = state.store.track_stats_map(uid).unwrap_or_default();
            let ratings = state.store.user_ratings(uid).unwrap_or_default();
            let mut root = XmlElem::new(root_name)
                .attr("position", q.position_ms)
                .attr("username", &user.username)
                .attr("changed", iso8601(q.updated))
                .attr("changedBy", &q.changed_by);
            if endpoint == "getPlayQueueByIndex" {
                root = root.attr("currentIndex", resolved_idx);
            } else {
                root = root.attr("current", &resolved_id);
            }
            for t in tracks {
                root = root.child(track_to_xml(
                    &t,
                    starred.get(&t.id).copied(),
                    stats.get(&t.id),
                    ratings.get(&t.id).copied(),
                    "entry",
                ));
            }
            respond(&params, true, Some(root))
        }
        "savePlayQueue" | "savePlayQueueByIndex" => {
            let ids = multi_vals(&pairs, "id");
            let cur_idx: usize = params
                .get("currentIndex")
                .and_then(|s| s.parse().ok())
                .unwrap_or_else(|| {
                    params
                        .get("current")
                        .and_then(|cid| ids.iter().position(|id| id == cid))
                        .unwrap_or(0)
                });
            let cur_id = params
                .get("current")
                .cloned()
                .or_else(|| ids.get(cur_idx).cloned())
                .unwrap_or_default();
            let pos_ms: i64 = params
                .get("position")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            let client = params
                .get("c")
                .cloned()
                .unwrap_or_else(|| "subsonic".to_string());
            let _ = state.store.save_play_queue(
                uid,
                &crate::models::PlayQueueState {
                    current_id: cur_id,
                    current_index: cur_idx,
                    position_ms: pos_ms,
                    track_ids: ids,
                    changed_by: client,
                    updated: Utc::now().timestamp(),
                },
            );
            respond(&params, true, None)
        }

        "getLyrics" => {
            let artist = params.get("artist").map(|s| s.as_str()).unwrap_or("");
            let title = params.get("title").map(|s| s.as_str()).unwrap_or("");
            let dummy = Track {
                id: format!("adhoc:{}:{}", artist, title),
                artist: artist.to_string(),
                title: title.to_string(),
                ..Default::default()
            };
            let data = state.engine.fetch_lyrics(&dummy).await;
            respond(
                &params,
                true,
                Some(
                    XmlElem::new("lyrics")
                        .attr("artist", artist)
                        .attr("title", title)
                        .text(data.plain),
                ),
            )
        }
        "getLyricsBySongId" => {
            let id = params.get("id").map(|s| s.as_str()).unwrap_or("");
            let Some(t) = state.store.get_track(id).ok().flatten() else {
                return respond(&params, true, Some(XmlElem::new("lyricsList")));
            };
            let data = state.engine.fetch_lyrics(&t).await;
            let mut list = XmlElem::new("lyricsList");
            if !data.lines.is_empty() {
                let mut sl = XmlElem::new("structuredLyrics")
                    .attr("displayArtist", &t.artist)
                    .attr("displayTitle", &t.title)
                    .attr("lang", "und")
                    .attr("synced", data.synced);
                for line in data.lines {
                    let mut le = XmlElem::new("line").text(line.text);
                    if let Some(ms) = line.start_ms {
                        le = le.attr("start", ms);
                    }
                    sl = sl.child(le);
                }
                list = list.child(sl);
            }
            respond(&params, true, Some(list))
        }

        "getInternetRadioStations" => {
            let stations = state.store.list_radio_stations(uid).unwrap_or_default();
            let mut root = XmlElem::new("internetRadioStations");
            for s in stations {
                root = root.child(
                    XmlElem::new("internetRadioStation")
                        .attr("id", &s.id)
                        .attr("name", &s.name)
                        .attr("streamUrl", &s.stream_url)
                        .attr("homePageUrl", &s.home_page_url),
                );
            }
            respond(&params, true, Some(root))
        }
        "createInternetRadioStation" => {
            let station = crate::models::RadioStation {
                id: String::new(),
                name: params.get("name").cloned().unwrap_or_default(),
                stream_url: params.get("streamUrl").cloned().unwrap_or_default(),
                home_page_url: params.get("homepageUrl").cloned().unwrap_or_default(),
            };
            let _ = state.store.upsert_radio_station(uid, &station);
            respond(&params, true, None)
        }
        "updateInternetRadioStation" => {
            let station = crate::models::RadioStation {
                id: params.get("id").cloned().unwrap_or_default(),
                name: params.get("name").cloned().unwrap_or_default(),
                stream_url: params.get("streamUrl").cloned().unwrap_or_default(),
                home_page_url: params.get("homepageUrl").cloned().unwrap_or_default(),
            };
            match state.store.upsert_radio_station(uid, &station) {
                Ok(_) => respond(&params, true, None),
                Err(e) => error_resp(&params, 50, &e.to_string()),
            }
        }
        "deleteInternetRadioStation" => {
            if let Some(id) = params.get("id") {
                if let Err(e) = state.store.delete_radio_station(uid, id) {
                    return error_resp(&params, 50, &e.to_string());
                }
            }
            respond(&params, true, None)
        }

        "getBookmarks" => respond(&params, true, Some(XmlElem::new("bookmarks"))),
        "getShares" => respond(&params, true, Some(XmlElem::new("shares"))),
        "getPodcasts" => respond(&params, true, Some(XmlElem::new("podcasts"))),
        "getNewestPodcasts" => respond(&params, true, Some(XmlElem::new("newestPodcasts"))),

        "stream" | "download" => {
            let Some(id) = params.get("id") else {
                return error_resp(&params, 10, "Missing id");
            };
            let Some(t) = state.store.get_track(id).ok().flatten() else {
                return error_resp(&params, 70, "Song not found");
            };
            let max_br = params
                .get("maxBitRate")
                .and_then(|s| s.parse::<i32>().ok())
                .filter(|&b| b > 0);
            let fmt = if endpoint == "download" {
                None
            } else {
                params.get("format").map(|s| s.as_str())
            };
            let offset = params
                .get("timeOffset")
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(0.0);
            if endpoint == "stream" {
                let mut np = state.now_playing.lock().unwrap_or_else(|e| e.into_inner());
                np.insert(uid, (t.clone(), Utc::now().timestamp(), user.username.clone()));
            }
            state
                .engine
                .serve_track_stream(&t, &headers, max_br, fmt, offset)
                .await
        }
        "getCoverArt" => {
            let id = params.get("id").map(|s| s.as_str()).unwrap_or("");
            state.engine.serve_cover(id).await
        }
        "getAvatar" => {
            if let Ok(Some((bytes, mime))) = state.store.get_avatar(uid) {
                (StatusCode::OK, [(header::CONTENT_TYPE, mime.as_str())], bytes).into_response()
            } else {
                state.engine.serve_cover(&format!("ar:{}", user.username)).await
            }
        }

        _ => error_resp(&params, 0, &format!("Unsupported endpoint '{}'", endpoint)),
    }
}

fn user_to_xml(u: &User) -> XmlElem {
    XmlElem::new("user")
        .attr("username", &u.username)
        .attr("email", format!("{}@beatlab.local", u.username))
        .attr("scrobblingEnabled", true)
        .attr("adminRole", false)
        .attr("settingsRole", true)
        .attr("downloadRole", true)
        .attr("uploadRole", false)
        .attr("playlistRole", true)
        .attr("coverArtRole", true)
        .attr("commentRole", false)
        .attr("podcastRole", false)
        .attr("streamRole", true)
        .attr("jukeboxRole", false)
        .attr("shareRole", true)
        .attr("videoConversionRole", false)
        .child(XmlElem::new("folder").text("1"))
}

fn iso8601_or_now(ts: i64) -> String {
    if ts > 0 {
        iso8601(ts)
    } else {
        iso8601(Utc::now().timestamp())
    }
}

pub fn track_to_xml(
    t: &Track,
    starred_at: Option<i64>,
    stats: Option<&UserTrackStats>,
    rating: Option<i32>,
    elem_name: &str,
) -> XmlElem {
    let suffix = if t.format.is_empty() { "m4a" } else { &t.format };
    let ct = mime_for_audio_ext(suffix);
    let primary_art = crate::models::primary_artist(&t.artist);
    let display_album_artist = if primary_art.is_empty() {
        t.artist.as_str()
    } else {
        primary_art.as_str()
    };
    let mut e = XmlElem::new(elem_name)
        .attr("id", &t.id)
        .attr("parent", t.album_id())
        .attr("isDir", false)
        .attr("title", &t.title)
        .attr("album", &t.album)
        .attr("artist", &t.artist)
        .attr("displayArtist", &t.artist)
        .attr("displayAlbumArtist", display_album_artist)
        .attr("discNumber", t.disc_no.max(1))
        .attr("genre", &t.genre)
        .attr("coverArt", &t.id)
        .attr("size", t.size.max(0))
        .attr("contentType", ct)
        .attr("suffix", suffix)
        .attr("duration", t.duration.max(0))
        .attr("bitRate", t.bitrate.max(128))
        .attr("channelCount", 2)
        .attr("type", "music")
        .attr("mediaType", "song")
        .attr("isVideo", false)
        .attr("albumId", t.album_id())
        .attr("artistId", t.artist_id())
        .attr("created", iso8601_or_now(t.created));

    if t.track_no > 0 {
        e = e.attr("track", t.track_no);
    }
    if t.year > 0 {
        e = e.attr("year", t.year);
    }
    if t.sample_rate > 0 {
        e = e.attr("samplingRate", t.sample_rate);
    }
    if t.bit_depth > 0 {
        e = e.attr("bitDepth", t.bit_depth);
    }
    if let Some(at) = starred_at {
        e = e.attr("starred", iso8601_or_now(at));
    }
    if let Some(st) = stats {
        e = e.attr("playCount", st.play_count);
        if st.last_played > 0 {
            e = e.attr("played", iso8601(st.last_played));
        }
    }
    if let Some(r) = rating.filter(|&r| r > 0) {
        e = e.attr("userRating", r);
    }
    e
}

fn album_to_xml(a: &AlbumSummary, saved: &HashMap<String, i64>, elem_name: &str) -> XmlElem {
    let mut e = XmlElem::new(elem_name)
        .attr("id", &a.id)
        .attr("name", &a.name)
        .attr("title", &a.name)
        .attr("artist", &a.artist)
        .attr("displayArtist", &a.artist)
        .attr("artistId", &a.artist_id)
        .attr("coverArt", &a.id)
        .attr("songCount", a.song_count)
        .attr("duration", a.duration)
        .attr("playCount", a.play_count)
        .attr("genre", &a.genre)
        .attr("isDir", true)
        .attr("mediaType", "album")
        .attr("created", iso8601_or_now(a.created));
    if a.year > 0 {
        e = e.attr("year", a.year);
    }
    if a.last_played > 0 {
        e = e.attr("played", iso8601(a.last_played));
    }
    if let Some(&at) = saved.get(&a.id) {
        e = e.attr("starred", iso8601_or_now(at));
    }
    e
}

fn artist_to_xml(a: &ArtistSummary, followed: &HashMap<String, i64>) -> XmlElem {
    let mut e = XmlElem::new("artist")
        .attr("id", &a.id)
        .attr("name", &a.name)
        .attr("coverArt", &a.id)
        .attr("albumCount", a.album_count)
        .attr("artistImageUrl", &a.cover_url);
    if let Some(&at) = followed.get(&a.id) {
        e = e.attr("starred", iso8601_or_now(at));
    }
    e
}

fn playlist_to_xml(p: &Playlist) -> XmlElem {
    XmlElem::new("playlist")
        .attr("id", &p.id)
        .attr("name", &p.name)
        .attr("comment", &p.comment)
        .attr("owner", &p.owner_name)
        .attr("public", p.public)
        .attr("songCount", p.track_count)
        .attr("duration", p.duration)
        .attr("created", iso8601_or_now(p.created))
        .attr("changed", iso8601_or_now(p.changed))
        .attr("coverArt", p.cover_track_id.as_deref().unwrap_or(&p.id))
}

fn songs_container(
    state: &AppState,
    uid: i64,
    container: &str,
    child_name: &str,
    tracks: &[Track],
) -> XmlElem {
    let starred = state.store.starred_at_map(uid).unwrap_or_default();
    let stats = state.store.track_stats_map(uid).unwrap_or_default();
    let ratings = state.store.user_ratings(uid).unwrap_or_default();
    let mut root = XmlElem::new(container);
    for t in tracks {
        root = root.child(track_to_xml(
            t,
            starred.get(&t.id).copied(),
            stats.get(&t.id),
            ratings.get(&t.id).copied(),
            child_name,
        ));
    }
    root
}

// ---------------------------------------------------------------------------
// Shared web-parity helpers (reused by both Subsonic and Navidrome endpoints)
// ---------------------------------------------------------------------------

pub(crate) fn filter_visible_tracks(state: &AppState, user_id: i64, tracks: Vec<Track>) -> Vec<Track> {
    if user_id <= 0 || tracks.is_empty() {
        return tracks;
    }
    let (hidden_ids, hidden_keys) = state.store.hidden_track_sets(user_id).unwrap_or_default();
    if hidden_ids.is_empty() && hidden_keys.is_empty() {
        return tracks;
    }
    tracks
        .into_iter()
        .filter(|t| {
            !hidden_ids.contains(&t.id)
                && (hidden_keys.is_empty() || !hidden_keys.contains(&t.song_key()))
        })
        .collect()
}

pub(crate) fn dedup_tracks_by_song_key(tracks: Vec<Track>) -> Vec<Track> {
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(tracks.len());
    for t in tracks {
        if seen.insert(t.song_key()) {
            out.push(t);
        }
    }
    out
}

pub(crate) async fn refresh_user_mixes(state: &AppState, user_id: i64) {
    if state.store.count_tracks().unwrap_or(0) > 0 {
        // Mixed-for-you first: rebuilding generic auto mixes clears the dirty flag.
        state.taste.refresh_mixed_for_you_if_stale_async(user_id).await;
        state.taste.refresh_auto_playlists_if_stale_async(user_id, 900).await;
    }
}

pub(crate) fn adjust_auto_playlist_summary(state: &AppState, user_id: i64, p: &mut Playlist) {
    if !p.is_auto || user_id <= 0 {
        return;
    }
    let (hidden_ids, hidden_keys) = state.store.hidden_track_sets(user_id).unwrap_or_default();
    if hidden_ids.is_empty() && hidden_keys.is_empty() {
        return;
    }
    let raw = state.store.playlist_tracks(&p.id).unwrap_or_default();
    let visible: Vec<Track> = raw
        .into_iter()
        .filter(|t| {
            !hidden_ids.contains(&t.id)
                && (hidden_keys.is_empty() || !hidden_keys.contains(&t.song_key()))
        })
        .collect();
    p.track_count = visible.len();
    p.duration = visible.iter().map(|t| t.duration.max(0)).sum();
    if p.cover_track_id
        .as_ref()
        .is_some_and(|cid| !visible.iter().any(|t| &t.id == cid))
    {
        p.cover_track_id = visible.first().map(|t| t.id.clone());
    }
}

pub(crate) fn hydrate_album_user_stats(
    album: &mut AlbumSummary,
    tracks: &[Track],
    stats: &HashMap<String, UserTrackStats>,
) {
    let mut play_count: i64 = 0;
    let mut last_played: i64 = 0;
    for t in tracks {
        if let Some(st) = stats.get(&t.id) {
            play_count += st.play_count as i64;
            if st.last_played > last_played {
                last_played = st.last_played;
            }
        }
    }
    if play_count > album.play_count {
        album.play_count = play_count;
    }
    if last_played > album.last_played {
        album.last_played = last_played;
    }
}

pub(crate) fn spawn_genre_enrichment_if_needed(state: &AppState, track: &Track) {
    if !track.genre.trim().is_empty() && track.year > 0 {
        return;
    }
    let dz = state.sources.deezer.clone();
    let store = state.store.clone();
    let mut batch = vec![track.clone()];
    tokio::spawn(async move {
        dz.enrich_track_genres(&mut batch).await;
        let _ = tokio::task::spawn_blocking(move || {
            let _ = store.upsert_tracks(&batch);
        })
        .await;
    });
}

pub(crate) fn resolve_genre_name(state: &AppState, raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let want_slug = crate::models::slug(trimmed);
    if let Ok(genres) = state.store.genres() {
        if let Some((name, _, _)) = genres
            .iter()
            .find(|(n, _, _)| n.eq_ignore_ascii_case(trimmed) || crate::models::slug(n) == want_slug)
        {
            return name.clone();
        }
    }
    trimmed.to_string()
}

pub(crate) async fn hydrate_genre_tracks(
    state: &AppState,
    uid: i64,
    genre: &str,
    limit: usize,
    offset: usize,
) -> Vec<Track> {
    let resolved = resolve_genre_name(state, genre);
    if resolved.is_empty() {
        return Vec::new();
    }
    let want_pool = (limit + offset).max(35);
    let mut tracks = state
        .store
        .tracks_by_genre(&resolved, want_pool, 0)
        .unwrap_or_default();
    if tracks.len() < 12 {
        let remote = state.sources.deezer.search_tracks(&resolved, 25).await;
        if !remote.is_empty() {
            let _ = state.store.upsert_tracks(&remote);
            tracks.extend(remote);
        }
    }
    let visible = dedup_tracks_by_song_key(filter_visible_tracks(state, uid, tracks));
    visible.into_iter().skip(offset).take(limit).collect()
}

/// Unified search matching the web player's search pipeline: intent parsing,
/// cross-source search + cached Deezer catalog, artist disambiguation, album
/// genre/year backfill, persistence, and `TasteEngine::search_and_refine_with_context`.
pub(crate) async fn unified_catalog_search(
    state: &AppState,
    uid: i64,
    query: &str,
) -> (Vec<Track>, Vec<AlbumSummary>, Vec<ArtistSummary>) {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return (Vec::new(), Vec::new(), Vec::new());
    }
    let intent = crate::taste::parse_search_intent(trimmed);
    let base_query = intent
        .entity_query
        .as_deref()
        .unwrap_or(trimmed)
        .to_string();

    let ((all_tracks, _ytm_hints), (dz_albums, dz_artists)) = tokio::join!(
        state.sources.search_all_with_ytm_hints(&base_query, 20),
        state.sources.search_deezer_catalog(&base_query, 15),
    );

    let mut combined = all_tracks;
    crate::sources::disambiguate_batch_tracks(&mut combined, &dz_artists);
    if !dz_albums.is_empty() {
        for t in &mut combined {
            if !t.genre.trim().is_empty() && t.year > 0 {
                continue;
            }
            let dz_aid = t.deezer_album_id();
            let want_slug = crate::models::slug(&t.album);
            if let Some(alb) = dz_albums.iter().find(|a| {
                (dz_aid.is_some() && dz_aid.as_deref() == Some(a.deezer_id.as_str()))
                    || (!want_slug.is_empty() && crate::models::slug(&a.name) == want_slug)
            }) {
                if t.genre.trim().is_empty() && !alb.genre.trim().is_empty() {
                    t.genre = alb.genre.clone();
                }
                if t.year <= 0 && alb.year > 0 {
                    t.year = alb.year;
                }
                if !t.genre.trim().is_empty() && t.moods.trim().is_empty() {
                    t.moods = crate::taste::classify_moods_str(&t.title, &t.album, &t.genre, "");
                }
            }
        }
    }

    // Persist before returning so follow-up stream/getAlbum/getArtist/getCoverArt
    // requests from the client immediately find every returned ID.
    if !combined.is_empty() || !dz_albums.is_empty() || !dz_artists.is_empty() {
        let store = state.store.clone();
        let initial_tracks = combined.clone();
        let save_albums = dz_albums.clone();
        let save_artists = dz_artists.clone();
        let _ = tokio::task::spawn_blocking(move || {
            if !initial_tracks.is_empty() {
                let _ = store.upsert_tracks(&initial_tracks);
            }
            for a in &save_albums {
                let _ = store.upsert_album_meta(a);
            }
            for ar in &save_artists {
                let _ = store.upsert_artist_meta(ar);
            }
        })
        .await;

        if combined.iter().any(|t| t.genre.trim().is_empty() || t.year <= 0) {
            let dz = state.sources.deezer.clone();
            let store = state.store.clone();
            let mut enrich_tracks = combined.clone();
            tokio::spawn(async move {
                dz.enrich_track_genres(&mut enrich_tracks).await;
                let _ = tokio::task::spawn_blocking(move || {
                    let _ = store.upsert_tracks(&enrich_tracks);
                })
                .await;
            });
        }
    }

    let taste = state.taste.clone();
    let q_owned = trimmed.to_string();
    let refined = tokio::task::spawn_blocking(move || {
        taste.search_and_refine_with_context(
            uid,
            &q_owned,
            combined,
            Vec::new(),
            crate::taste::SearchMixContext::default(),
        )
    })
    .await
    .unwrap_or_else(|_| state.taste.search_and_refine(uid, trimmed, vec![], vec![]));

    // Merge ranked Deezer catalog results with local matches so both online and
    // local-only albums/artists appear cleanly without duplicates.
    let local_albums = state
        .store
        .search_albums(uid, &base_query, 50, 0)
        .unwrap_or_default();
    let local_album_map: HashMap<String, AlbumSummary> = local_albums
        .iter()
        .cloned()
        .map(|a| (a.id.clone(), a))
        .collect();
    let mut seen_albums = HashSet::new();
    let mut merged_albums = Vec::with_capacity(dz_albums.len() + local_albums.len());
    for mut a in dz_albums {
        if seen_albums.insert(a.id.clone()) {
            if let Some(loc) = local_album_map.get(&a.id) {
                if a.song_count == 0 {
                    a.song_count = loc.song_count;
                }
                if a.duration == 0 {
                    a.duration = loc.duration;
                }
                if a.play_count == 0 {
                    a.play_count = loc.play_count;
                }
                if a.last_played == 0 {
                    a.last_played = loc.last_played;
                }
                if a.created == 0 {
                    a.created = loc.created;
                }
            }
            merged_albums.push(a);
        }
    }
    for a in local_albums {
        if seen_albums.insert(a.id.clone()) {
            merged_albums.push(a);
        }
    }

    let local_artists = state
        .store
        .search_artists(&base_query, 50, 0)
        .unwrap_or_default();
    let local_artist_map: HashMap<String, ArtistSummary> = local_artists
        .iter()
        .cloned()
        .map(|a| (a.id.clone(), a))
        .collect();
    let mut seen_artists = HashSet::new();
    let mut merged_artists = Vec::with_capacity(dz_artists.len() + local_artists.len());
    for mut ar in dz_artists {
        if seen_artists.insert(ar.id.clone()) {
            if let Some(loc) = local_artist_map.get(&ar.id) {
                if ar.album_count == 0 {
                    ar.album_count = loc.album_count;
                }
                if ar.song_count == 0 {
                    ar.song_count = loc.song_count;
                }
                if ar.bio.is_empty() && !loc.bio.is_empty() {
                    ar.bio = loc.bio.clone();
                }
            }
            merged_artists.push(ar);
        }
    }
    for ar in local_artists {
        if seen_artists.insert(ar.id.clone()) {
            merged_artists.push(ar);
        }
    }

    (refined.tracks, merged_albums, merged_artists)
}

async fn similar_songs_for_entity(
    state: &AppState,
    uid: i64,
    id: &str,
    count: usize,
) -> Vec<Track> {
    // 1. Artist radio (`ar:...`) — Subsonic clients invoke getSimilarSongs2 on artist pages.
    if id.starts_with("ar:") {
        if let Some((artist, top, _, related)) = state.taste.get_or_hydrate_artist(id, uid).await {
            let mut out = state
                .taste
                .artist_related_tracks(&artist, &related, &top, count)
                .await;
            if out.len() < count {
                for t in top {
                    if !out.iter().any(|x| x.song_key() == t.song_key()) {
                        out.push(t);
                    }
                    if out.len() >= count {
                        break;
                    }
                }
            }
            let visible = filter_visible_tracks(state, uid, out);
            return dedup_tracks_by_song_key(visible)
                .into_iter()
                .take(count)
                .collect();
        }
    }

    // 2. Album radio (`al:...`) — seed personalized radio from the album's lead track.
    if id.starts_with("al:") {
        if let Some((_, tracks)) = state.taste.get_or_hydrate_album(id).await {
            if let Some(seed) = tracks.first() {
                let exclude: HashSet<String> = tracks.iter().map(|t| t.id.clone()).collect();
                let next = state
                    .taste
                    .radio_next_tracks_with_mode(
                        uid,
                        Some(seed),
                        &exclude,
                        &[],
                        count,
                        crate::taste::RadioMode::Default,
                    )
                    .await;
                if !next.is_empty() {
                    return filter_visible_tracks(state, uid, next)
                        .into_iter()
                        .take(count)
                        .collect();
                }
            }
        }
    }

    // 3. Track radio — use the web client's personalized neural + YTM/Deezer radio engine.
    if let Some(seed) = state.store.get_track(id).ok().flatten() {
        let mut exclude = HashSet::new();
        exclude.insert(seed.id.clone());
        let radio = state
            .taste
            .radio_next_tracks_with_mode(
                uid,
                Some(&seed),
                &exclude,
                &[],
                count,
                crate::taste::RadioMode::Default,
            )
            .await;
        let visible = filter_visible_tracks(state, uid, radio);
        if !visible.is_empty() {
            return visible.into_iter().take(count).collect();
        }
        let fallback = state.taste.similar_songs_async(&seed, count).await;
        return filter_visible_tracks(state, uid, fallback)
            .into_iter()
            .take(count)
            .collect();
    }

    // 4. Fallback: personalized For You tracks, respecting hidden tracks.
    filter_visible_tracks(state, uid, state.taste.for_you_tracks(uid, count * 2))
        .into_iter()
        .take(count)
        .collect()
}

fn strip_ignored_article(name: &str) -> &str {
    for prefix in [
        "The ", "the ", "El ", "el ", "La ", "la ", "Los ", "los ", "Las ", "las ", "Le ", "le ",
        "Les ", "les ",
    ] {
        if let Some(rest) = name.strip_prefix(prefix) {
            if !rest.is_empty() {
                return rest;
            }
        }
    }
    name
}

fn handle_get_indexes(
    state: &AppState,
    params: &HashMap<String, String>,
    uid: i64,
    id3: bool,
) -> Response {
    let artists = state.store.build_artist_summaries().unwrap_or_default();
    let followed = state.store.followed_artist_at_map(uid).unwrap_or_default();
    let mut buckets: BTreeMap<String, Vec<ArtistSummary>> = BTreeMap::new();
    for a in artists {
        let stripped = strip_ignored_article(&a.name);
        let ch = stripped
            .chars()
            .next()
            .unwrap_or('#')
            .to_uppercase()
            .next()
            .unwrap_or('#');
        let key = if ch.is_ascii_alphabetic() {
            ch.to_string()
        } else {
            "#".to_string()
        };
        buckets.entry(key).or_default().push(a);
    }

    let mut root = XmlElem::new(if id3 { "artists" } else { "indexes" })
        .attr("ignoredArticles", "The El La Los Las Le Les")
        .attr("lastModified", Utc::now().timestamp_millis());
    for (letter, items) in buckets {
        let mut idx = XmlElem::new("index").attr("name", letter);
        for a in items {
            idx = idx.child(artist_to_xml(&a, &followed));
        }
        root = root.child(idx);
    }
    respond(params, true, Some(root))
}

async fn handle_music_directory(
    state: &AppState,
    params: &HashMap<String, String>,
    uid: i64,
    id: &str,
) -> Response {
    if id == "1" || id.is_empty() {
        let artists = state.store.build_artist_summaries().unwrap_or_default();
        let followed = state.store.followed_artist_at_map(uid).unwrap_or_default();
        let mut dir = XmlElem::new("directory").attr("id", "1").attr("name", "Beatlab Library");
        for a in artists {
            let mut ch = XmlElem::new("child")
                .attr("id", &a.id)
                .attr("parent", "1")
                .attr("title", &a.name)
                .attr("isDir", true)
                .attr("coverArt", &a.id);
            if let Some(&at) = followed.get(&a.id) {
                ch = ch.attr("starred", iso8601_or_now(at));
            }
            dir = dir.child(ch);
        }
        return respond(params, true, Some(dir));
    }
    if id.starts_with("ar:") {
        if let Some((art, _, albums, _)) = state.taste.get_or_hydrate_artist(id, uid).await {
            let saved = state.store.saved_album_at_map(uid).unwrap_or_default();
            let mut dir = XmlElem::new("directory")
                .attr("id", &art.id)
                .attr("parent", "1")
                .attr("name", &art.name);
            for a in albums {
                dir = dir.child(album_to_xml(&a, &saved, "child").attr("parent", &art.id));
            }
            return respond(params, true, Some(dir));
        }
    }
    if id.starts_with("al:") {
        if let Some((alb, tracks)) = state.taste.get_or_hydrate_album(id).await {
            let starred = state.store.starred_at_map(uid).unwrap_or_default();
            let stats = state.store.track_stats_map(uid).unwrap_or_default();
            let ratings = state.store.user_ratings(uid).unwrap_or_default();
            let mut dir = XmlElem::new("directory")
                .attr("id", &alb.id)
                .attr("parent", &alb.artist_id)
                .attr("name", &alb.name);
            for t in &tracks {
                dir = dir.child(track_to_xml(
                    t,
                    starred.get(&t.id).copied(),
                    stats.get(&t.id),
                    ratings.get(&t.id).copied(),
                    "child",
                ));
            }
            return respond(params, true, Some(dir));
        }
    }
    error_resp(params, 70, "Directory not found")
}

fn get_sorted_albums(
    state: &AppState,
    uid: i64,
    list_type: &str,
    params: &HashMap<String, String>,
    size: usize,
    offset: usize,
) -> Vec<AlbumSummary> {
    let mut all = state.store.build_album_summaries(uid).unwrap_or_default();
    let saved = state.store.saved_album_ids(uid).unwrap_or_default();
    match list_type {
        "random" => {
            use rand::seq::SliceRandom;
            let mut rng = rand::thread_rng();
            all.shuffle(&mut rng);
        }
        "newest" => {
            all.sort_by(|a, b| {
                (b.song_count > 0)
                    .cmp(&(a.song_count > 0))
                    .then_with(|| b.created.cmp(&a.created))
                    .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            });
        }
        "frequent" => {
            all.retain(|a| a.play_count > 0);
            all.sort_by(|a, b| {
                b.play_count
                    .cmp(&a.play_count)
                    .then_with(|| b.last_played.cmp(&a.last_played))
            });
        }
        "recent" => {
            all.retain(|a| a.last_played > 0);
            all.sort_by(|a, b| b.last_played.cmp(&a.last_played));
        }
        "highest" => {
            let ratings = state.store.user_ratings(uid).unwrap_or_default();
            if ratings.is_empty() {
                all.clear();
            } else {
                let mut album_scores: HashMap<String, (i64, i64)> = HashMap::new();
                for (tid, r) in &ratings {
                    if *r <= 0 {
                        continue;
                    }
                    if let Ok(Some(t)) = state.store.get_track(tid) {
                        let e = album_scores.entry(t.album_id()).or_insert((0, 0));
                        e.0 += *r as i64;
                        e.1 += 1;
                    }
                }
                all.retain(|a| album_scores.contains_key(&a.id));
                all.sort_by(|a, b| {
                    let avg = |id: &str| -> f64 {
                        album_scores
                            .get(id)
                            .map(|(sum, cnt)| *sum as f64 / (*cnt).max(1) as f64)
                            .unwrap_or(0.0)
                    };
                    avg(&b.id)
                        .partial_cmp(&avg(&a.id))
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then_with(|| b.play_count.cmp(&a.play_count))
                });
            }
        }
        "starred" => {
            all.retain(|a| saved.contains(&a.id));
        }
        "alphabeticalByName" => {
            all.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        }
        "alphabeticalByArtist" => {
            all.sort_by(|a, b| {
                a.artist
                    .to_lowercase()
                    .cmp(&b.artist.to_lowercase())
                    .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            });
        }
        "byGenre" => {
            let g = resolve_genre_name(
                state,
                params.get("genre").map(|s| s.as_str()).unwrap_or(""),
            )
            .to_lowercase();
            all.retain(|a| a.genre.to_lowercase().contains(&g));
        }
        "byYear" => {
            let from_y: i32 = params
                .get("fromYear")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            let to_y: i32 = params
                .get("toYear")
                .and_then(|s| s.parse().ok())
                .unwrap_or(9999);
            let (lo, hi) = if from_y <= to_y {
                (from_y, to_y)
            } else {
                (to_y, from_y)
            };
            all.retain(|a| a.year >= lo && a.year <= hi);
            if from_y > to_y {
                all.sort_by(|a, b| b.year.cmp(&a.year));
            } else {
                all.sort_by(|a, b| a.year.cmp(&b.year));
            }
        }
        _ => {}
    }
    all.into_iter().skip(offset).take(size).collect()
}

async fn handle_search(
    state: &AppState,
    params: &HashMap<String, String>,
    uid: i64,
    endpoint: &str,
) -> Response {
    let q = params.get("query").map(|s| s.trim()).unwrap_or("");
    let song_count = parse_usize(params, "songCount", 25).min(100);
    let song_offset = parse_usize(params, "songOffset", 0);
    let album_count = parse_usize(params, "albumCount", 15).min(50);
    let album_offset = parse_usize(params, "albumOffset", 0);
    let artist_count = parse_usize(params, "artistCount", 15).min(50);
    let artist_offset = parse_usize(params, "artistOffset", 0);

    let (tracks, albums, artists): (Vec<Track>, Vec<AlbumSummary>, Vec<ArtistSummary>) = if !q.is_empty() && q != "\"\"" {
        let (all_tracks, all_albums, all_artists) = unified_catalog_search(state, uid, q).await;
        (
            all_tracks
                .into_iter()
                .skip(song_offset)
                .take(song_count)
                .collect(),
            all_albums
                .into_iter()
                .skip(album_offset)
                .take(album_count)
                .collect(),
            all_artists
                .into_iter()
                .skip(artist_offset)
                .take(artist_count)
                .collect(),
        )
    } else {
        (
            state.store.all_tracks(song_count, song_offset).unwrap_or_default(),
            state
                .store
                .build_album_summaries(uid)
                .unwrap_or_default()
                .into_iter()
                .skip(album_offset)
                .take(album_count)
                .collect(),
            state
                .store
                .build_artist_summaries()
                .unwrap_or_default()
                .into_iter()
                .skip(artist_offset)
                .take(artist_count)
                .collect(),
        )
    };

    let root_name = match endpoint {
        "search3" => "searchResult3",
        "search2" => "searchResult2",
        _ => "searchResult",
    };

    let starred = state.store.starred_at_map(uid).unwrap_or_default();
    let stats = state.store.track_stats_map(uid).unwrap_or_default();
    let ratings = state.store.user_ratings(uid).unwrap_or_default();
    let saved = state.store.saved_album_at_map(uid).unwrap_or_default();
    let followed = state.store.followed_artist_at_map(uid).unwrap_or_default();

    let mut root = XmlElem::new(root_name);
    for a in artists {
        root = root.child(artist_to_xml(&a, &followed));
    }
    for alb in albums {
        root = root.child(album_to_xml(&alb, &saved, "album"));
    }
    let child_song_name = if endpoint == "search" { "match" } else { "song" };
    for t in tracks {
        root = root.child(track_to_xml(
            &t,
            starred.get(&t.id).copied(),
            stats.get(&t.id),
            ratings.get(&t.id).copied(),
            child_song_name,
        ));
    }
    respond(params, true, Some(root))
}

fn parse_usize(params: &HashMap<String, String>, k: &str, default: usize) -> usize {
    params
        .get(k)
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::archive::ArchiveSource;
    use crate::sources::deezer::DeezerSource;
    use crate::sources::local::LocalSource;
    use crate::sources::soulseek::SoulseekSource;
    use crate::sources::ytm::YtmSource;
    use axum::body::to_bytes;

    fn test_state() -> AppState {
        let store = Store::open_memory().unwrap();
        let tmp = std::env::temp_dir().join(format!("beatlab-test-{}", uuid::Uuid::new_v4()));
        let deezer = DeezerSource::new();
        let ytm = YtmSource::new(None);
        let archive = ArchiveSource::new();
        let local = Arc::new(LocalSource::new(store.clone(), vec![]));
        let soulseek = SoulseekSource::new(store.clone(), tmp.join("slsk"));
        let sources = SourceRegistry::new(deezer, ytm, archive, local, soulseek);
        let engine = Arc::new(StreamEngine::new(store.clone(), sources.clone(), &tmp, 1.0));
        let taste = Arc::new(TasteEngine::new(store.clone(), sources.clone()));
        store
            .upsert_tracks(&[Track {
                id: "t1".into(),
                source: "local".into(),
                title: "Midnight City".into(),
                artist: "M83".into(),
                album: "Hurry Up, We're Dreaming".into(),
                genre: "Electronic".into(),
                year: 2011,
                track_no: 1,
                disc_no: 1,
                duration: 243,
                format: "flac".into(),
                bitrate: 900,
                ..Default::default()
            }])
            .unwrap();
        AppState {
            store,
            sources,
            engine,
            taste,
            now_playing: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    async fn call_subsonic(
        st: AppState,
        user: &str,
        pass: &str,
        action: &str,
        extra: &[(&str, &str)],
    ) -> Value {
        let mut qs = format!(
            "u={}&p={}&f=json&v=1.16.1&c=test",
            urlencoding::encode(user),
            urlencoding::encode(pass)
        );
        for (k, v) in extra {
            qs.push('&');
            qs.push_str(k);
            qs.push('=');
            qs.push_str(&urlencoding::encode(v));
        }
        let resp = dispatch(
            State(st),
            Path(action.into()),
            RawQuery(Some(qs)),
            HeaderMap::new(),
            String::new(),
        )
        .await;
        let bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn api_key_for(st: &AppState, username: &str, password: &str) -> String {
        let user = st
            .store
            .get_user_by_name(username)
            .unwrap()
            .unwrap_or_else(|| st.store.create_user(username, password).unwrap());
        st.store.get_or_create_api_key(user.id).unwrap()
    }

    #[tokio::test]
    async fn subsonic_requires_api_key_and_rejects_password_auth() {
        let st = test_state();

        // Unknown user salted token is rejected.
        let rejected = dispatch(
            State(st.clone()),
            Path("ping.view".into()),
            RawQuery(Some("u=newlistener&t=deadbeef&s=1234&f=json".into())),
            HeaderMap::new(),
            String::new(),
        )
        .await;
        let bytes = to_bytes(rejected.into_body(), 1024 * 1024).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["subsonic-response"]["error"]["code"], 40);

        // Account passwords (plain or enc:) never authenticate or auto-create.
        let bad = call_subsonic(st.clone(), "newlistener", "enc:736563726574", "ping.view", &[]).await;
        assert_eq!(bad["subsonic-response"]["error"]["code"], 40);
        assert!(st.store.get_user_by_name("newlistener").unwrap().is_none());

        // Create the user and mint an API key.
        let user = st.store.create_user("newlistener", "secret").unwrap();
        let key = st.store.get_or_create_api_key(user.id).unwrap();

        // Salted token computed from the account PASSWORD must still be rejected.
        let mut pw_md5 = Md5::new();
        pw_md5.update(b"secretmysalt");
        let pw_tok = hex::encode(pw_md5.finalize());
        let bad_tok = dispatch(
            State(st.clone()),
            Path("ping.view".into()),
            RawQuery(Some(format!("u=newlistener&t={}&s=mysalt&f=json", pw_tok))),
            HeaderMap::new(),
            String::new(),
        )
        .await;
        let body: Value = serde_json::from_slice(&to_bytes(bad_tok.into_body(), 1024 * 1024).await.unwrap()).unwrap();
        assert_eq!(body["subsonic-response"]["error"]["code"], 40);

        // The API key from Settings works in the password field.
        let ok = call_subsonic(st.clone(), "newlistener", &key, "ping.view", &[]).await;
        assert_eq!(ok["subsonic-response"]["status"], "ok");

        // Standard Subsonic 1.13+ salted token computed from the API key ALSO works.
        let mut key_md5 = Md5::new();
        key_md5.update(format!("{}mysalt", key).as_bytes());
        let key_tok = hex::encode(key_md5.finalize());
        let ok_tok = dispatch(
            State(st.clone()),
            Path("ping.view".into()),
            RawQuery(Some(format!("u=newlistener&t={}&s=mysalt&f=json", key_tok))),
            HeaderMap::new(),
            String::new(),
        )
        .await;
        let body: Value = serde_json::from_slice(&to_bytes(ok_tok.into_body(), 1024 * 1024).await.unwrap()).unwrap();
        assert_eq!(body["subsonic-response"]["status"], "ok");

        // And via OpenSubsonic's dedicated apiKey parameter.
        let explicit = dispatch(
            State(st.clone()),
            Path("ping.view".into()),
            RawQuery(Some(format!("u=newlistener&apiKey={}&f=json", key))),
            HeaderMap::new(),
            String::new(),
        )
        .await;
        let bytes = to_bytes(explicit.into_body(), 1024 * 1024).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["subsonic-response"]["status"], "ok");

        // createUser is disabled for API-key clients (Subsonic error 50).
        let cu = call_subsonic(
            st.clone(),
            "newlistener",
            &key,
            "createUser.view",
            &[("username", "victim"), ("password", "pw123")],
        )
        .await;
        assert_eq!(cu["subsonic-response"]["error"]["code"], 50);
        assert!(st.store.get_user_by_name("victim").unwrap().is_none());

        let uinfo = call_subsonic(st, "newlistener", &key, "getUser.view", &[]).await;
        assert_eq!(uinfo["subsonic-response"]["user"]["adminRole"], false);
        assert_eq!(uinfo["subsonic-response"]["user"]["folder"], serde_json::json!([1]));
    }

    #[tokio::test]
    async fn opensubsonic_extensions_serialize_integer_version_arrays() {
        let st = test_state();
        let key = api_key_for(&st, "extuser", "pw123");
        let resp = call_subsonic(st, "extuser", &key, "getOpenSubsonicExtensions.view", &[]).await;
        let exts = resp["subsonic-response"]["openSubsonicExtensions"]
            .as_array()
            .unwrap();
        assert!(!exts.is_empty());
        for ext in exts {
            assert_eq!(ext["versions"], serde_json::json!([1]), "ext: {ext}");
        }
    }

    #[tokio::test]
    async fn starred_albums_artists_and_album_lists_match_user_state() {
        let st = test_state();
        let key = api_key_for(&st, "staruser", "pw123");
        let t1 = st.store.get_track("t1").unwrap().unwrap();
        let album_id = t1.album_id();
        let artist_id = t1.artist_id();

        // Before playing anything, recent and frequent album lists are empty.
        let recent_empty = call_subsonic(
            st.clone(),
            "staruser",
            &key,
            "getAlbumList2.view",
            &[("type", "recent")],
        )
        .await;
        assert!(recent_empty["subsonic-response"]["albumList2"]["album"].is_null());

        let freq_empty = call_subsonic(
            st.clone(),
            "staruser",
            &key,
            "getAlbumList2.view",
            &[("type", "frequent")],
        )
        .await;
        assert!(freq_empty["subsonic-response"]["albumList2"]["album"].is_null());

        // Star the track, save the album, follow the artist, rate and scrobble the track.
        let _ = call_subsonic(
            st.clone(),
            "staruser",
            &key,
            "star.view",
            &[("id", "t1"), ("albumId", &album_id), ("artistId", &artist_id)],
        )
        .await;
        let _ = call_subsonic(
            st.clone(),
            "staruser",
            &key,
            "setRating.view",
            &[("id", "t1"), ("rating", "5")],
        )
        .await;
        let _ = call_subsonic(
            st.clone(),
            "staruser",
            &key,
            "scrobble.view",
            &[("id", "t1"), ("submission", "true")],
        )
        .await;

        // getAlbumList2?type=starred must include the starred attribute on the album!
        let starred_albs = call_subsonic(
            st.clone(),
            "staruser",
            &key,
            "getAlbumList2.view",
            &[("type", "starred")],
        )
        .await;
        let alb = &starred_albs["subsonic-response"]["albumList2"]["album"][0];
        assert_eq!(alb["id"], album_id);
        let alb_starred = alb["starred"].as_str().unwrap();
        assert!(!alb_starred.starts_with("1970-"));

        // getArtists must include starred on followed artists!
        let artists_resp = call_subsonic(st.clone(), "staruser", &key, "getArtists.view", &[]).await;
        let ar = &artists_resp["subsonic-response"]["artists"]["index"][0]["artist"][0];
        assert_eq!(ar["id"], artist_id);
        assert!(ar["starred"].as_str().is_some());

        // getAlbumList2?type=recent, frequent, highest now return the album.
        for ty in ["recent", "frequent", "highest"] {
            let res = call_subsonic(
                st.clone(),
                "staruser",
                &key,
                "getAlbumList2.view",
                &[("type", ty)],
            )
            .await;
            let arr = res["subsonic-response"]["albumList2"]["album"]
                .as_array()
                .unwrap_or_else(|| panic!("expected album array for type={ty}: {res}"));
            assert_eq!(arr.len(), 1, "type={ty}");
            assert_eq!(arr[0]["id"], album_id);
        }
    }

    #[tokio::test]
    async fn now_playing_is_scoped_to_user_and_ignores_submission_scrobbles() {
        let st = test_state();
        st.store
            .upsert_tracks(&[Track {
                id: "t2".into(),
                source: "local".into(),
                title: "Wait".into(),
                artist: "M83".into(),
                album: "Hurry Up, We're Dreaming".into(),
                duration: 343,
                ..Default::default()
            }])
            .unwrap();
        let alice_key = api_key_for(&st, "np_alice", "pw1");
        let bob_key = api_key_for(&st, "np_bob", "pw2");

        let _ = call_subsonic(
            st.clone(),
            "np_alice",
            &alice_key,
            "scrobble.view",
            &[("id", "t1"), ("submission", "false")],
        )
        .await;

        // Alice sees t1; Bob must NOT see Alice's listening activity.
        let alice_np = call_subsonic(st.clone(), "np_alice", &alice_key, "getNowPlaying.view", &[]).await;
        assert_eq!(alice_np["subsonic-response"]["nowPlaying"]["entry"][0]["id"], "t1");

        let bob_np = call_subsonic(st.clone(), "np_bob", &bob_key, "getNowPlaying.view", &[]).await;
        assert!(bob_np["subsonic-response"]["nowPlaying"]["entry"].is_null());

        // A historical/completion submission scrobble must not clobber nowPlaying.
        let _ = call_subsonic(
            st.clone(),
            "np_alice",
            &alice_key,
            "scrobble.view",
            &[("id", "t2"), ("submission", "true")],
        )
        .await;
        let alice_np2 = call_subsonic(st, "np_alice", &alice_key, "getNowPlaying.view", &[]).await;
        assert_eq!(alice_np2["subsonic-response"]["nowPlaying"]["entry"][0]["id"], "t1");
    }

    #[tokio::test]
    async fn hidden_tracks_are_filtered_from_random_songs_and_auto_playlists() {
        let st = test_state();
        st.store
            .upsert_tracks(&[Track {
                id: "t2".into(),
                source: "local".into(),
                title: "Outro".into(),
                artist: "M83".into(),
                album: "Hurry Up, We're Dreaming".into(),
                genre: "Electronic".into(),
                year: 2011,
                duration: 247,
                ..Default::default()
            }])
            .unwrap();
        let key = api_key_for(&st, "hideuser", "pw1");
        let uid = st.store.get_user_by_name("hideuser").unwrap().unwrap().id;

        // Mark t1 as Not Interested.
        st.taste.mark_not_interested(uid, "t1");

        // getRandomSongs must exclude t1.
        let rand_resp = call_subsonic(st.clone(), "hideuser", &key, "getRandomSongs.view", &[("size", "10")]).await;
        let songs = rand_resp["subsonic-response"]["randomSongs"]["song"]
            .as_array()
            .unwrap();
        assert_eq!(songs.len(), 1);
        assert_eq!(songs[0]["id"], "t2");

        // Create an auto playlist containing both t1 and t2 and verify getPlaylist filters t1.
        let auto_id = format!("auto-{uid}-test");
        st.store
            .upsert_auto_playlist(&auto_id, uid, "test_auto", "Auto Mix", "Test", &["t1".into(), "t2".into()])
            .unwrap();
        let pl_resp = call_subsonic(st, "hideuser", &key, "getPlaylist.view", &[("id", &auto_id)]).await;
        let pl = &pl_resp["subsonic-response"]["playlist"];
        assert_eq!(pl["songCount"], 1);
        assert_eq!(pl["duration"], 247);
        let entries = pl["entry"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["id"], "t2");
    }

    #[tokio::test]
    async fn update_playlist_enforces_owner() {
        let st = test_state();
        st.store
            .upsert_tracks(&[Track {
                id: "t2".into(),
                source: "local".into(),
                title: "Wait".into(),
                artist: "M83".into(),
                album: "Hurry Up, We're Dreaming".into(),
                genre: "Electronic".into(),
                year: 2011,
                duration: 343,
                ..Default::default()
            }])
            .unwrap();

        let alice_key = api_key_for(&st, "alice", "pass1");
        let bob_key = api_key_for(&st, "bob", "pass2");
        let created = call_subsonic(
            st.clone(),
            "alice",
            &alice_key,
            "createPlaylist.view",
            &[("name", "Roadtrip"), ("songId", "t1")],
        )
        .await;
        let pid = created["subsonic-response"]["playlist"]["id"]
            .as_str()
            .unwrap()
            .to_string();

        let _ = call_subsonic(
            st.clone(),
            "alice",
            &alice_key,
            "updatePlaylist.view",
            &[
                ("playlistId", &pid),
                ("songIdToAdd", "t2"),
                ("songIndexToRemove", "0"),
            ],
        )
        .await;
        let fetched = call_subsonic(st.clone(), "alice", &alice_key, "getPlaylist.view", &[("id", &pid)]).await;
        let entries = fetched["subsonic-response"]["playlist"]["entry"]
            .as_array()
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["id"], "t2");

        // Bob cannot modify Alice's playlist (Subsonic error 50)
        let hijack = call_subsonic(
            st,
            "bob",
            &bob_key,
            "updatePlaylist.view",
            &[("playlistId", &pid), ("name", "Hijack")],
        )
        .await;
        assert_eq!(hijack["subsonic-response"]["error"]["code"], 50);
    }

    #[test]
    fn text_only_subsonic_elements_serialize_as_json_strings() {
        let info = XmlElem::new("artistInfo2")
            .child(XmlElem::new("biography").text("Bio text"))
            .child(XmlElem::new("smallImageUrl").text("https://img.example/s.jpg"))
            .child(
                XmlElem::new("structuredLyrics")
                    .attr("synced", false)
                    .child(XmlElem::new("line").text("Lyric line")),
            );
        let j = info.to_json();
        assert_eq!(j["biography"], "Bio text");
        assert_eq!(j["smallImageUrl"], "https://img.example/s.jpg");
        assert_eq!(j["structuredLyrics"][0]["line"][0]["value"], "Lyric line");
    }

    #[test]
    fn forwarded_headers_only_trusted_from_proxy_peers() {
        let mk = |peer: Option<&str>, xff: &str| {
            let mut h = HeaderMap::new();
            if let Some(p) = peer {
                h.insert(PEER_IP_HEADER, p.parse().unwrap());
            }
            h.insert("x-forwarded-for", xff.parse().unwrap());
            h.insert("fly-client-ip", "203.0.113.9".parse().unwrap());
            h
        };
        // Public peer talking directly: spoofed headers are ignored.
        let direct = mk(Some("198.51.100.7"), "1.2.3.4");
        assert_eq!(client_ip(&direct), "198.51.100.7");
        assert_eq!(rate_limit_ip(&direct), "198.51.100.7");
        // Reverse proxy on loopback / private / IPv6 ULA: headers are believed.
        for proxy in ["127.0.0.1", "10.0.0.2", "172.17.0.1", "fdaa:0:1::3", "::ffff:192.168.1.5"] {
            let h = mk(Some(proxy), "1.2.3.4, 10.0.0.1");
            assert_eq!(client_ip(&h), "1.2.3.4", "proxy {proxy}");
            assert_eq!(rate_limit_ip(&h), "203.0.113.9", "proxy {proxy}");
        }
        // No proxy headers from a LAN client: fall back to the peer address instead of "".
        let mut lan = HeaderMap::new();
        lan.insert(PEER_IP_HEADER, "192.168.1.20".parse().unwrap());
        assert_eq!(rate_limit_ip(&lan), "192.168.1.20");
    }
}
