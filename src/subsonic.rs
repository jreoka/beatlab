use crate::engine::{mime_for_audio_ext, StreamEngine};
use crate::models::{AlbumSummary, ArtistSummary, Playlist, Track, User, UserTrackStats};
use crate::sources::SourceRegistry;
use crate::store::Store;
use crate::taste::TasteEngine;
use axum::extract::{Path, RawQuery, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
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
    pub quality_hunter: crate::quality_hunter::QualityHunter,
}

/// Best-effort client IP from common reverse-proxy headers. Behind a proxy the
/// socket peer is the proxy itself, so these are the only meaningful source.
pub fn client_ip(headers: &HeaderMap) -> String {
    for name in ["x-forwarded-for", "x-real-ip", "cf-connecting-ip", "fly-client-ip"] {
        if let Some(v) = headers.get(name).and_then(|v| v.to_str().ok()) {
            let first = v.split(',').next().unwrap_or("").trim();
            if !first.is_empty() {
                return first.chars().take(64).collect();
            }
        }
    }
    String::new()
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
            | "child"
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
    let user = authenticate_subsonic_credentials(state, params)?;

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

    if params.contains_key("t") || params.contains_key("s") {
        return Err((
            40,
            "Password/token sign-in is disabled. Use the API key from Settings → Apps & API as the password.",
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
        "createUser" => {
            let u = params.get("username").map(|s| s.trim()).unwrap_or("");
            let p = params.get("password").map(|s| s.as_str()).unwrap_or("");
            match state.store.create_user(u, p) {
                Ok(new_user) => {
                    state.taste.prime_new_user(new_user.id).await;
                    respond(&params, true, None)
                }
                Err(e) => error_resp(&params, 0, &e.to_string()),
            }
        }
        "changePassword" => {
            let target = params
                .get("username")
                .map(|s| s.as_str())
                .unwrap_or(&user.username);
            if !target.eq_ignore_ascii_case(&user.username) {
                return error_resp(&params, 50, "You can only change your own password");
            }
            let Some(new_pw) = params.get("password") else {
                return error_resp(&params, 10, "Missing password");
            };
            match state.store.change_password(uid, new_pw) {
                Ok(()) => respond(&params, true, None),
                Err(e) => error_resp(&params, 0, &e.to_string()),
            }
        }

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

        "getIndexes" => handle_get_indexes(&state, &params, false),
        "getArtists" => handle_get_indexes(&state, &params, true),

        "getMusicDirectory" => {
            let id = params.get("id").map(|s| s.as_str()).unwrap_or("1");
            handle_music_directory(&state, &params, uid, id).await
        }
        "getArtist" => {
            let Some(id) = params.get("id") else {
                return error_resp(&params, 10, "Missing id");
            };
            let Some((artist, _top, albums, _related)) =
                state.taste.get_or_hydrate_artist(id, uid).await
            else {
                return error_resp(&params, 70, "Artist not found");
            };
            let starred = state.store.starred_ids(uid).unwrap_or_default();
            let mut elem = XmlElem::new("artist")
                .attr("id", &artist.id)
                .attr("name", &artist.name)
                .attr("coverArt", &artist.id)
                .attr("albumCount", albums.len())
                .attr("artistImageUrl", &artist.cover_url);
            if starred.contains(&artist.id) {
                elem = elem.attr("starred", iso8601(Utc::now().timestamp()));
            }
            for alb in albums {
                elem = elem.child(album_to_xml(&alb, &starred, "album"));
            }
            respond(&params, true, Some(elem))
        }
        "getArtistInfo" | "getArtistInfo2" => {
            let id = params.get("id").map(|s| s.as_str()).unwrap_or("");
            let is_v2 = endpoint == "getArtistInfo2";
            let root_name = if is_v2 { "artistInfo2" } else { "artistInfo" };
            if let Some((artist, _, _, related)) =
                state.taste.get_or_hydrate_artist(id, uid).await
            {
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
                for rel in related {
                    elem = elem.child(
                        XmlElem::new("similarArtist")
                            .attr("id", &rel.id)
                            .attr("name", &rel.name)
                            .attr("coverArt", &rel.id)
                            .attr("artistImageUrl", &rel.cover_url),
                    );
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
            let Some((alb, tracks)) = state.taste.get_or_hydrate_album(id).await else {
                return error_resp(&params, 70, "Album not found");
            };
            let starred = state.store.starred_ids(uid).unwrap_or_default();
            let stats = state.store.track_stats_map(uid).unwrap_or_default();
            let ratings = state.store.user_ratings(uid).unwrap_or_default();
            let mut elem = album_to_xml(&alb, &starred, "album");
            for t in &tracks {
                elem = elem.child(track_to_xml(
                    t,
                    starred.contains(&t.id),
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
            let starred = state.store.starred_ids(uid).unwrap_or_default();
            let st = state.store.get_user_track_stats(uid, &t.id).ok();
            let rating = state.store.user_ratings(uid).ok().and_then(|m| m.get(&t.id).copied());
            respond(
                &params,
                true,
                Some(track_to_xml(&t, starred.contains(&t.id), st.as_ref(), rating, "song")),
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
            let tracks = state
                .store
                .tracks_by_genre(genre, count, offset)
                .unwrap_or_default();
            let elem = songs_container(&state, uid, "songsByGenre", "song", &tracks);
            respond(&params, true, Some(elem))
        }
        "getRandomSongs" => {
            let size = parse_usize(&params, "size", 20).min(100);
            let genre = params.get("genre").map(|s| s.as_str());
            let tracks = state.store.random_tracks(size, genre).unwrap_or_default();
            let elem = songs_container(&state, uid, "randomSongs", "song", &tracks);
            respond(&params, true, Some(elem))
        }
        "getTopSongs" => {
            let artist_name = params.get("artist").map(|s| s.as_str()).unwrap_or("");
            let count = parse_usize(&params, "count", 25).min(100);
            let aid = format!("ar:{}", crate::models::slug(artist_name));
            let tracks = if let Some((_, top, _, _)) =
                state.taste.get_or_hydrate_artist(&aid, uid).await
            {
                top.into_iter().take(count).collect()
            } else {
                state
                    .store
                    .search_tracks(artist_name, count, 0)
                    .unwrap_or_default()
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
            let id = params.get("id").map(|s| s.as_str()).unwrap_or("");
            let count = parse_usize(&params, "count", 25).min(100);
            let tracks = if let Some(seed) = state.store.get_track(id).ok().flatten() {
                state.taste.similar_songs_async(&seed, count).await
            } else {
                state.taste.for_you_tracks(uid, count)
            };
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
            let starred = state.store.starred_ids(uid).unwrap_or_default();
            let mut root = XmlElem::new(root_name);
            for a in albums {
                root = root.child(album_to_xml(&a, &starred, "album"));
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
            let starred_ids = state.store.starred_ids(uid).unwrap_or_default();
            let stats = state.store.track_stats_map(uid).unwrap_or_default();
            let ratings = state.store.user_ratings(uid).unwrap_or_default();
            let saved_albums = state.store.saved_album_ids(uid).unwrap_or_default();
            let followed_artists = state.store.followed_artist_ids(uid).unwrap_or_default();

            let mut root = XmlElem::new(root_name);
            for a in state.store.build_artist_summaries().unwrap_or_default() {
                if followed_artists.contains(&a.id) {
                    root = root.child(artist_to_xml(&a));
                }
            }
            for alb in state.store.build_album_summaries(uid).unwrap_or_default() {
                if saved_albums.contains(&alb.id) {
                    root = root.child(album_to_xml(&alb, &saved_albums, "album"));
                }
            }
            for (t, _) in starred_tracks {
                root = root.child(track_to_xml(
                    &t,
                    starred_ids.contains(&t.id),
                    stats.get(&t.id),
                    ratings.get(&t.id).copied(),
                    "song",
                ));
            }
            respond(&params, true, Some(root))
        }
        "star" => {
            for id in multi_vals(&pairs, "id") {
                let _ = state.store.star_track(uid, &id);
                state.taste.learn_signal(uid, &id, "star", 1.0);
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
                let _ = state.store.unstar_track(uid, &id);
                state.taste.learn_signal(uid, &id, "unstar", 0.0);
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
            if state.store.count_tracks().unwrap_or(0) > 0 {
                state.taste.refresh_auto_playlists_if_stale_async(uid, 900).await;
            }
            let playlists = state.store.list_playlists(uid, true).unwrap_or_default();
            let mut root = XmlElem::new("playlists");
            for p in playlists {
                root = root.child(playlist_to_xml(&p));
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
            let p = if p.is_auto && p.user_id == uid && state.store.count_tracks().unwrap_or(0) > 0 {
                state.taste.refresh_auto_playlists_if_stale_async(uid, 900).await;
                state.store.get_playlist(id).ok().flatten().unwrap_or(p)
            } else {
                p
            };
            let tracks = state.store.playlist_tracks(id).unwrap_or_default();
            let starred = state.store.starred_ids(uid).unwrap_or_default();
            let stats = state.store.track_stats_map(uid).unwrap_or_default();
            let ratings = state.store.user_ratings(uid).unwrap_or_default();
            let mut elem = playlist_to_xml(&p);
            for t in tracks {
                elem = elem.child(track_to_xml(
                    &t,
                    starred.contains(&t.id),
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
                    let mut np = state.now_playing.lock().unwrap_or_else(|e| e.into_inner());
                    np.insert(uid, (t, Utc::now().timestamp(), user.username.clone()));
                }
            }
            respond(&params, true, None)
        }
        "getNowPlaying" => {
            let cutoff = Utc::now().timestamp() - 600;
            let active: Vec<(Track, i64, String)> = {
                let np = state.now_playing.lock().unwrap_or_else(|e| e.into_inner());
                np.values()
                    .filter(|(_, ts, _)| *ts >= cutoff)
                    .cloned()
                    .collect()
            };
            let starred = state.store.starred_ids(uid).unwrap_or_default();
            let stats = state.store.track_stats_map(uid).unwrap_or_default();
            let mut root = XmlElem::new("nowPlaying");
            let now = Utc::now().timestamp();
            for (t, ts, uname) in active {
                let mut elem = track_to_xml(&t, starred.contains(&t.id), stats.get(&t.id), None, "entry");
                elem = elem
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
            let starred = state.store.starred_ids(uid).unwrap_or_default();
            let stats = state.store.track_stats_map(uid).unwrap_or_default();
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
                    starred.contains(&t.id),
                    stats.get(&t.id),
                    None,
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
            {
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

pub fn track_to_xml(
    t: &Track,
    is_starred: bool,
    stats: Option<&UserTrackStats>,
    rating: Option<i32>,
    elem_name: &str,
) -> XmlElem {
    let ct = mime_for_audio_ext(&t.format);
    let mut e = XmlElem::new(elem_name)
        .attr("id", &t.id)
        .attr("parent", t.album_id())
        .attr("isDir", false)
        .attr("title", &t.title)
        .attr("album", &t.album)
        .attr("artist", &t.artist)
        .attr("track", t.track_no)
        .attr("discNumber", t.disc_no.max(1))
        .attr("year", t.year)
        .attr("genre", &t.genre)
        .attr("coverArt", &t.id)
        .attr("size", t.size.max(0))
        .attr("contentType", ct)
        .attr("suffix", if t.format.is_empty() { "m4a" } else { &t.format })
        .attr("duration", t.duration.max(0))
        .attr("bitRate", t.bitrate.max(128))
        .attr("samplingRate", t.sample_rate)
        .attr("bitDepth", t.bit_depth)
        .attr("type", "music")
        .attr("isVideo", false)
        .attr("albumId", t.album_id())
        .attr("artistId", t.artist_id())
        .attr("created", iso8601(t.created));

    if is_starred {
        e = e.attr("starred", iso8601(t.created));
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

fn album_to_xml(a: &AlbumSummary, saved: &HashSet<String>, elem_name: &str) -> XmlElem {
    let mut e = XmlElem::new(elem_name)
        .attr("id", &a.id)
        .attr("name", &a.name)
        .attr("title", &a.name)
        .attr("artist", &a.artist)
        .attr("artistId", &a.artist_id)
        .attr("coverArt", &a.id)
        .attr("songCount", a.song_count)
        .attr("duration", a.duration)
        .attr("playCount", a.play_count)
        .attr("year", a.year)
        .attr("genre", &a.genre)
        .attr("isDir", true)
        .attr("created", iso8601(a.created));
    if saved.contains(&a.id) {
        e = e.attr("starred", iso8601(a.created));
    }
    e
}

fn artist_to_xml(a: &ArtistSummary) -> XmlElem {
    XmlElem::new("artist")
        .attr("id", &a.id)
        .attr("name", &a.name)
        .attr("coverArt", &a.id)
        .attr("albumCount", a.album_count)
        .attr("artistImageUrl", &a.cover_url)
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
        .attr("created", iso8601(p.created))
        .attr("changed", iso8601(p.changed))
        .attr("coverArt", p.cover_track_id.as_deref().unwrap_or(&p.id))
}

fn songs_container(
    state: &AppState,
    uid: i64,
    container: &str,
    child_name: &str,
    tracks: &[Track],
) -> XmlElem {
    let starred = state.store.starred_ids(uid).unwrap_or_default();
    let stats = state.store.track_stats_map(uid).unwrap_or_default();
    let ratings = state.store.user_ratings(uid).unwrap_or_default();
    let mut root = XmlElem::new(container);
    for t in tracks {
        root = root.child(track_to_xml(
            t,
            starred.contains(&t.id),
            stats.get(&t.id),
            ratings.get(&t.id).copied(),
            child_name,
        ));
    }
    root
}

fn handle_get_indexes(state: &AppState, params: &HashMap<String, String>, id3: bool) -> Response {
    let artists = state.store.build_artist_summaries().unwrap_or_default();
    let mut buckets: BTreeMap<String, Vec<ArtistSummary>> = BTreeMap::new();
    for a in artists {
        let stripped = a
            .name
            .strip_prefix("The ")
            .or_else(|| a.name.strip_prefix("the "))
            .unwrap_or(&a.name);
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
            idx = idx.child(artist_to_xml(&a));
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
        let mut dir = XmlElem::new("directory").attr("id", "1").attr("name", "Beatlab Library");
        for a in artists {
            dir = dir.child(
                XmlElem::new("child")
                    .attr("id", &a.id)
                    .attr("parent", "1")
                    .attr("title", &a.name)
                    .attr("isDir", true)
                    .attr("coverArt", &a.id),
            );
        }
        return respond(params, true, Some(dir));
    }
    if id.starts_with("ar:") {
        if let Some((art, _, albums, _)) = state.taste.get_or_hydrate_artist(id, uid).await {
            let saved = state.store.saved_album_ids(uid).unwrap_or_default();
            let mut dir = XmlElem::new("directory").attr("id", &art.id).attr("name", &art.name);
            for a in albums {
                dir = dir.child(album_to_xml(&a, &saved, "child"));
            }
            return respond(params, true, Some(dir));
        }
    }
    if id.starts_with("al:") {
        if let Some((alb, tracks)) = state.taste.get_or_hydrate_album(id).await {
            let starred = state.store.starred_ids(uid).unwrap_or_default();
            let stats = state.store.track_stats_map(uid).unwrap_or_default();
            let mut dir = XmlElem::new("directory").attr("id", &alb.id).attr("name", &alb.name);
            for t in &tracks {
                dir = dir.child(track_to_xml(
                    t,
                    starred.contains(&t.id),
                    stats.get(&t.id),
                    None,
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
            all.sort_by(|a, b| b.created.cmp(&a.created));
        }
        "frequent" => {
            all.sort_by(|a, b| b.play_count.cmp(&a.play_count));
        }
        "recent" => {
            all.sort_by(|a, b| b.last_played.cmp(&a.last_played));
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
            let g = params
                .get("genre")
                .map(|s| s.to_lowercase())
                .unwrap_or_default();
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

    let (tracks, albums, artists) = if !q.is_empty() && q != "\"\"" {
        let (remote_tracks, (dz_tracks, dz_albums, dz_artists)) = tokio::join!(
            state.sources.search_all(q, 20),
            state.sources.deezer.search_all(q, 12),
        );
        let combined = [remote_tracks, dz_tracks].concat();
        let _ = state.store.upsert_tracks(&combined);
        for a in &dz_albums {
            let _ = state.store.upsert_album_meta(a);
        }
        for ar in &dz_artists {
            let _ = state.store.upsert_artist_meta(ar);
        }
        (
            state
                .store
                .search_tracks(q, song_count, song_offset)
                .unwrap_or_default(),
            state
                .store
                .search_albums(uid, q, album_count, album_offset)
                .unwrap_or_default(),
            state
                .store
                .search_artists(q, artist_count, artist_offset)
                .unwrap_or_default(),
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

    let starred = state.store.starred_ids(uid).unwrap_or_default();
    let stats = state.store.track_stats_map(uid).unwrap_or_default();
    let ratings = state.store.user_ratings(uid).unwrap_or_default();
    let saved = state.store.saved_album_ids(uid).unwrap_or_default();

    let mut root = XmlElem::new(root_name);
    for a in artists {
        root = root.child(artist_to_xml(&a));
    }
    for alb in albums {
        root = root.child(album_to_xml(&alb, &saved, "album"));
    }
    let child_song_name = if endpoint == "search" { "match" } else { "song" };
    for t in tracks {
        root = root.child(track_to_xml(
            &t,
            starred.contains(&t.id),
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
        let quality_hunter = crate::quality_hunter::QualityHunter::new(store.clone(), (*engine).clone());
        AppState {
            store,
            sources,
            engine,
            taste,
            now_playing: Arc::new(Mutex::new(HashMap::new())),
            quality_hunter,
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

        // Salted token auth is no longer accepted at all.
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

        // The API key from Settings works in the password field.
        let user = st.store.create_user("newlistener", "secret").unwrap();
        let key = st.store.get_or_create_api_key(user.id).unwrap();
        let ok = call_subsonic(st.clone(), "newlistener", &key, "ping.view", &[]).await;
        assert_eq!(ok["subsonic-response"]["status"], "ok");

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

        let uinfo = call_subsonic(st, "newlistener", &key, "getUser.view", &[]).await;
        assert_eq!(uinfo["subsonic-response"]["user"]["adminRole"], false);
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
}
