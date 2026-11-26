use crate::auth::{
    assertion_credential_id, generate_recovery_codes, generate_totp_secret, hash_token,
    otpauth_uri, passkey_credential_id, qr_svg, random_token, verify_totp, MFA_WINDOW_SECS,
    RECOVERY_CODE_COUNT, RESTORE_WINDOW_SECS,
};
use crate::models::{
    AlbumSummary, ArtistSummary, Mood, PlayQueueState, Playlist, PublicUser, Track, User,
    UserProfile, UserTrackStats, ERR_RESTORE_REQUIRED, slug,
};
use crate::state::{client_ip, rate_limit_ip, user_agent, AppState};
use crate::store::SessionMeta;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha1::{Digest, Sha1};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;
use uuid::Uuid;
use base64::Engine as _;
use webauthn_rs::prelude::{
    AuthenticationResult, DiscoverableAuthentication, DiscoverableKey, Passkey, PasskeyAuthentication,
    PasskeyRegistration, PublicKeyCredential, RegisterPublicKeyCredential,
};

const INDEX_HTML: &str = include_str!("web/index.html");
const STYLES_CSS: &str = include_str!("web/styles.css");
const APP_JS: &str = include_str!("web/app.js");
const FAVICON_SVG: &str = include_str!("web/favicon.svg");
const MANIFEST_JSON: &str = include_str!("web/manifest.webmanifest");
const SW_JS_TEMPLATE: &str = include_str!("web/sw.js");
const ICON_192_PNG: &[u8] = include_bytes!("web/icon-192.png");
pub(crate) const ICON_512_PNG: &[u8] = include_bytes!("web/icon-512.png");
const ICON_MASKABLE_512_PNG: &[u8] = include_bytes!("web/icon-maskable-512.png");
const APPLE_TOUCH_ICON_PNG: &[u8] = include_bytes!("web/apple-touch-icon.png");

const CSS_LINK_TAG: &str = r#"<link rel="stylesheet" href="/web/styles.css" />"#;
const JS_SCRIPT_TAG: &str = r#"<script src="/web/app.js"></script>"#;
const FAVICON_HREF: &str = r#"href="/web/favicon.svg""#;
const ASSET_CACHE_CONTROL: &str = "no-cache, must-revalidate";

struct CachedAsset {
    body: &'static [u8],
    etag: String,
    short_hash: String,
}

fn asset_from_bytes(body: &'static [u8]) -> CachedAsset {
    let digest = hex::encode(Sha1::digest(body));
    let short_hash = digest[..12].to_string();
    let etag = format!("\"{short_hash}\"");
    CachedAsset {
        body,
        etag,
        short_hash,
    }
}

fn asset_from_str(body: &'static str) -> CachedAsset {
    asset_from_bytes(body.as_bytes())
}

/// Escapes closing raw-text HTML tags (case-insensitively) before inlining CSS/JS.
fn escape_inline_closing_tag(
    src: &'static str,
    needle_lower: &[u8],
    replacement: &str,
) -> Cow<'static, str> {
    let bytes = src.as_bytes();
    let n = needle_lower.len();
    let Some(first) = bytes
        .windows(n)
        .position(|w| w.eq_ignore_ascii_case(needle_lower))
    else {
        return Cow::Borrowed(src);
    };

    let mut out = String::with_capacity(src.len() + 16);
    out.push_str(&src[..first]);
    out.push_str(replacement);
    let mut cursor = first + n;
    while cursor + n <= bytes.len() {
        if bytes[cursor..cursor + n].eq_ignore_ascii_case(needle_lower) {
            out.push_str(replacement);
            cursor += n;
        } else {
            let ch = src[cursor..].chars().next().unwrap();
            out.push(ch);
            cursor += ch.len_utf8();
        }
    }
    out.push_str(&src[cursor..]);
    Cow::Owned(out)
}

/// Bundles `index.html`, `styles.css`, and `app.js` into a single self-contained HTML
/// response so CDN edge caches and multi-node deployments cannot serve mismatched versions.
fn build_bundled_index(favicon_hash: &str) -> String {
    let safe_css = escape_inline_closing_tag(STYLES_CSS, b"</style", r"\3c /style");
    let safe_js = escape_inline_closing_tag(APP_JS, b"</script", r"<\/script");

    let (before_css, after_css) = INDEX_HTML
        .split_once(CSS_LINK_TAG)
        .expect("index.html must contain /web/styles.css link tag");
    let (between_css_js, after_js) = after_css
        .split_once(JS_SCRIPT_TAG)
        .expect("index.html must contain /web/app.js script tag");

    let mut out = String::with_capacity(INDEX_HTML.len() + safe_css.len() + safe_js.len() + 64);
    if let Some((before_fav, after_fav)) = before_css.split_once(FAVICON_HREF) {
        out.push_str(before_fav);
        out.push_str(r#"href="/web/favicon.svg?v="#);
        out.push_str(favicon_hash);
        out.push('"');
        out.push_str(after_fav);
    } else {
        out.push_str(before_css);
    }
    out.push_str("<style>\n");
    out.push_str(&safe_css);
    out.push_str("\n  </style>");
    out.push_str(between_css_js);
    out.push_str("<script>\n");
    out.push_str(&safe_js);
    out.push_str("\n  </script>");
    out.push_str(after_js);
    out
}

fn css_asset() -> &'static CachedAsset {
    static ASSET: OnceLock<CachedAsset> = OnceLock::new();
    ASSET.get_or_init(|| asset_from_str(STYLES_CSS))
}

fn js_asset() -> &'static CachedAsset {
    static ASSET: OnceLock<CachedAsset> = OnceLock::new();
    ASSET.get_or_init(|| asset_from_str(APP_JS))
}

fn favicon_asset() -> &'static CachedAsset {
    static ASSET: OnceLock<CachedAsset> = OnceLock::new();
    ASSET.get_or_init(|| asset_from_str(FAVICON_SVG))
}

fn manifest_asset() -> &'static CachedAsset {
    static ASSET: OnceLock<CachedAsset> = OnceLock::new();
    ASSET.get_or_init(|| asset_from_str(MANIFEST_JSON))
}

fn icon_192_asset() -> &'static CachedAsset {
    static ASSET: OnceLock<CachedAsset> = OnceLock::new();
    ASSET.get_or_init(|| asset_from_bytes(ICON_192_PNG))
}

fn icon_512_asset() -> &'static CachedAsset {
    static ASSET: OnceLock<CachedAsset> = OnceLock::new();
    ASSET.get_or_init(|| asset_from_bytes(ICON_512_PNG))
}

fn icon_maskable_512_asset() -> &'static CachedAsset {
    static ASSET: OnceLock<CachedAsset> = OnceLock::new();
    ASSET.get_or_init(|| asset_from_bytes(ICON_MASKABLE_512_PNG))
}

fn apple_touch_icon_asset() -> &'static CachedAsset {
    static ASSET: OnceLock<CachedAsset> = OnceLock::new();
    ASSET.get_or_init(|| asset_from_bytes(APPLE_TOUCH_ICON_PNG))
}

fn index_asset() -> &'static CachedAsset {
    static ASSET: OnceLock<CachedAsset> = OnceLock::new();
    ASSET.get_or_init(|| {
        let html: &'static str =
            Box::leak(build_bundled_index(&favicon_asset().short_hash).into_boxed_str());
        asset_from_str(html)
    })
}

fn build_service_worker() -> String {
    let mut hasher = Sha1::new();
    for part in [
        index_asset().short_hash.as_bytes(),
        manifest_asset().short_hash.as_bytes(),
        favicon_asset().short_hash.as_bytes(),
        icon_192_asset().short_hash.as_bytes(),
        icon_512_asset().short_hash.as_bytes(),
        icon_maskable_512_asset().short_hash.as_bytes(),
        apple_touch_icon_asset().short_hash.as_bytes(),
        SW_JS_TEMPLATE.as_bytes(),
    ] {
        hasher.update(part);
    }
    let version = format!("beatlab-pwa-{}", &hex::encode(hasher.finalize())[..12]);
    SW_JS_TEMPLATE.replace("__BEATLAB_SW_VERSION__", &version)
}

fn sw_asset() -> &'static CachedAsset {
    static ASSET: OnceLock<CachedAsset> = OnceLock::new();
    ASSET.get_or_init(|| {
        let body: &'static str = Box::leak(build_service_worker().into_boxed_str());
        asset_from_str(body)
    })
}

fn etag_matches(headers: &HeaderMap, etag: &str) -> bool {
    let Some(val) = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
    else {
        return false;
    };
    val.split(',').any(|part| {
        let candidate = part.trim();
        let normalized = candidate.strip_prefix("W/").unwrap_or(candidate).trim();
        normalized == "*" || normalized == etag
    })
}

fn serve_asset(
    headers: &HeaderMap,
    content_type: &'static str,
    asset: &'static CachedAsset,
) -> Response {
    if etag_matches(headers, &asset.etag) {
        return (
            StatusCode::NOT_MODIFIED,
            [
                (header::CACHE_CONTROL, ASSET_CACHE_CONTROL),
                (header::ETAG, asset.etag.as_str()),
            ],
        )
            .into_response();
    }
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, ASSET_CACHE_CONTROL),
            (header::ETAG, asset.etag.as_str()),
        ],
        asset.body,
    )
        .into_response()
}

pub async fn serve_index(headers: HeaderMap) -> Response {
    serve_asset(&headers, "text/html; charset=utf-8", index_asset())
}

pub async fn serve_css(headers: HeaderMap) -> Response {
    serve_asset(&headers, "text/css; charset=utf-8", css_asset())
}

pub async fn serve_js(headers: HeaderMap) -> Response {
    serve_asset(&headers, "application/javascript; charset=utf-8", js_asset())
}

pub async fn serve_favicon(headers: HeaderMap) -> Response {
    serve_asset(&headers, "image/svg+xml; charset=utf-8", favicon_asset())
}

pub async fn serve_manifest(headers: HeaderMap) -> Response {
    serve_asset(
        &headers,
        "application/manifest+json; charset=utf-8",
        manifest_asset(),
    )
}

pub async fn serve_sw(headers: HeaderMap) -> Response {
    let mut res = serve_asset(&headers, "application/javascript; charset=utf-8", sw_asset());
    res.headers_mut().insert(
        header::HeaderName::from_static("service-worker-allowed"),
        header::HeaderValue::from_static("/"),
    );
    res
}

pub async fn serve_icon_192(headers: HeaderMap) -> Response {
    serve_asset(&headers, "image/png", icon_192_asset())
}

pub async fn serve_icon_512(headers: HeaderMap) -> Response {
    serve_asset(&headers, "image/png", icon_512_asset())
}

pub async fn serve_icon_maskable_512(headers: HeaderMap) -> Response {
    serve_asset(&headers, "image/png", icon_maskable_512_asset())
}

pub async fn serve_apple_touch_icon(headers: HeaderMap) -> Response {
    serve_asset(&headers, "image/png", apple_touch_icon_asset())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebTrack {
    pub id: String,
    pub source: String,
    pub title: String,
    pub artist: String,
    pub artist_id: String,
    pub album: String,
    pub album_id: String,
    pub genre: String,
    pub moods: String,
    pub year: i32,
    pub track_no: i32,
    pub disc_no: i32,
    pub duration: i32,
    pub format: String,
    pub bitrate: i32,
    pub is_lossless: bool,
    pub is_popular: bool,
    pub is_taste_match: bool,
    pub quality_badge: String,
    pub cover_url: String,
    pub is_starred: bool,
    pub play_count: i32,
    pub rating: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loudness: Option<crate::loudness::TrackLoudness>,
}

/// Track ids that are part of the listener's own rotation — used to badge
/// personalized picks in curated mixes.
fn personal_match_ids(state: &AppState, user_id: i64) -> HashSet<String> {
    if user_id <= 0 {
        return HashSet::new();
    }
    let mut ids: HashSet<String> = state.store.starred_ids(user_id).unwrap_or_default();
    ids.extend(
        state
            .store
            .user_frequent_tracks(user_id, 200, 0)
            .unwrap_or_default()
            .into_iter()
            .map(|(t, _)| t.id),
    );
    ids.extend(
        state
            .store
            .user_manual_playlist_tracks(user_id)
            .unwrap_or_default()
            .into_iter()
            .map(|(_, _, t)| t.id),
    );
    ids
}

fn filter_tracks_with_hidden(
    tracks: Vec<Track>,
    hidden_ids: &HashSet<String>,
    hidden_keys: &HashSet<String>,
) -> Vec<Track> {
    if tracks.is_empty() || (hidden_ids.is_empty() && hidden_keys.is_empty()) {
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

fn filter_visible_tracks(state: &AppState, user_id: i64, tracks: Vec<Track>) -> Vec<Track> {
    if user_id <= 0 || tracks.is_empty() {
        return tracks;
    }
    let (hidden_ids, hidden_keys) = state.store.hidden_track_sets(user_id).unwrap_or_default();
    filter_tracks_with_hidden(tracks, &hidden_ids, &hidden_keys)
}

fn to_web_track_with_maps(
    t: &Track,
    starred: &HashSet<String>,
    stats: &HashMap<String, UserTrackStats>,
    ratings: &HashMap<String, i32>,
    loudness: &HashMap<String, crate::loudness::TrackLoudness>,
    is_taste_match: bool,
) -> WebTrack {
    WebTrack {
        id: t.id.clone(),
        source: t.source.clone(),
        title: t.title.clone(),
        artist: t.artist.clone(),
        artist_id: t.artist_id(),
        album: t.album.clone(),
        album_id: t.album_id(),
        genre: t.genre.clone(),
        moods: t.moods.clone(),
        year: t.year,
        track_no: t.track_no,
        disc_no: t.disc_no,
        duration: t.duration,
        format: t.format.clone(),
        bitrate: t.bitrate,
        is_lossless: t.is_lossless(),
        is_popular: track_is_popular(&t.meta),
        is_taste_match,
        quality_badge: t.quality_badge(),
        cover_url: cover_api_url(&t.id),
        is_starred: starred.contains(&t.id),
        play_count: stats.get(&t.id).map(|s| s.play_count).unwrap_or(0),
        rating: ratings.get(&t.id).copied().unwrap_or(0),
        loudness: loudness.get(&t.id).copied(),
    }
}

fn to_web_tracks_from_maps(
    tracks: &[Track],
    starred: &HashSet<String>,
    stats: &HashMap<String, UserTrackStats>,
    ratings: &HashMap<String, i32>,
    loudness: &HashMap<String, crate::loudness::TrackLoudness>,
) -> Vec<WebTrack> {
    tracks
        .iter()
        .map(|t| to_web_track_with_maps(t, starred, stats, ratings, loudness, false))
        .collect()
}

fn to_web_tracks(state: &AppState, user_id: i64, tracks: &[Track]) -> Vec<WebTrack> {
    to_web_tracks_with_taste(state, user_id, tracks, &HashSet::new())
}

fn to_web_tracks_with_taste(
    state: &AppState,
    user_id: i64,
    tracks: &[Track],
    taste_match_ids: &HashSet<String>,
) -> Vec<WebTrack> {
    let starred = if user_id > 0 {
        state.store.starred_ids(user_id).unwrap_or_default()
    } else {
        HashSet::new()
    };
    let stats = if user_id > 0 {
        state.store.track_stats_map(user_id).unwrap_or_default()
    } else {
        Default::default()
    };
    let ratings = if user_id > 0 {
        state.store.user_ratings(user_id).unwrap_or_default()
    } else {
        Default::default()
    };
    let ids: Vec<String> = tracks.iter().map(|t| t.id.clone()).collect();
    let loudness = state.store.loudness_map_for_ids(&ids).unwrap_or_default();

    tracks
        .iter()
        .map(|t| {
            to_web_track_with_maps(
                t,
                &starred,
                &stats,
                &ratings,
                &loudness,
                taste_match_ids.contains(&t.id),
            )
        })
        .collect()
}

fn track_is_popular(meta: &str) -> bool {
    if meta.is_empty() {
        return false;
    }
    serde_json::from_str::<Value>(meta)
        .ok()
        .and_then(|v| v.get("rank").and_then(|r| r.as_u64()))
        .map(|r| r >= 300_000)
        .unwrap_or(false)
}

pub(crate) fn cover_api_url(id: &str) -> String {
    format!("/api/web/cover/{}?v=2", urlencoding::encode(id))
}

fn extract_web_token(headers: &HeaderMap, query_token: Option<&str>) -> Option<String> {
    if let Some(tok) = query_token.map(|s| s.trim()).filter(|s| !s.is_empty()) {
        return Some(tok.to_string());
    }
    if let Some(raw) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        let trimmed = raw.trim();
        if let Some(tok) = trimmed
            .strip_prefix("Bearer ")
            .or_else(|| trimmed.strip_prefix("bearer "))
        {
            return Some(tok.trim().to_string());
        }
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    None
}

pub(crate) fn resolve_web_user(state: &AppState, headers: &HeaderMap, query_token: Option<&str>) -> Option<User> {
    let tok = extract_web_token(headers, query_token)?;
    state.store.get_session_user(&tok).ok().flatten()
}

pub(crate) fn require_web_user(state: &AppState, headers: &HeaderMap) -> Result<User, Response> {
    resolve_web_user(state, headers, None).ok_or_else(|| {
        (
            StatusCode::UNAUTHORIZED,
            [(header::CONTENT_TYPE, "application/json")],
            json!({ "error": "Please sign in" }).to_string(),
        )
            .into_response()
    })
}

/// Small helper used by auth endpoints so the shell can paint the signed-in
/// user's custom picture immediately after load/sign-in.
fn self_avatar_url(state: &AppState, user_id: i64) -> String {
    state
        .store
        .get_public_user(user_id, user_id)
        .ok()
        .flatten()
        .map(|u| u.avatar_url)
        .unwrap_or_default()
}

pub(crate) fn json_res(val: Value) -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        val.to_string(),
    )
        .into_response()
}

pub(crate) fn err_res(status: StatusCode, msg: &str) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        json!({ "error": msg }).to_string(),
    )
        .into_response()
}

fn unauthorized() -> Response {
    err_res(StatusCode::UNAUTHORIZED, "Please sign in")
}

/// Runs CPU-heavy work (password hashing) on the blocking pool so it never
/// stalls the async workers. `None` only if the task panicked.
async fn run_blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    tokio::task::spawn_blocking(f).await.ok()
}

/// Re-checks the account password for a sensitive action. Failures count
/// toward the same per-account limit as sign-in, so a stolen session can't
/// brute-force the password through these endpoints either.
async fn check_current_password(
    user: &User,
    password: &str,
    wrong_msg: &str,
) -> Result<(), Response> {
    let rate_keys = crate::auth::rate_keys("signin", &user.username, "");
    if let Some(wait) = crate::auth::throttled_for(&rate_keys) {
        return Err(too_many_attempts(wait));
    }
    let (stored, attempt) = (user.password.clone(), password.to_string());
    let ok = run_blocking(move || crate::auth::verify_password(&stored, &attempt))
        .await
        .unwrap_or(false);
    if ok {
        Ok(())
    } else {
        crate::auth::record_failure(&rate_keys);
        Err(err_res(StatusCode::BAD_REQUEST, wrong_msg))
    }
}

fn too_many_attempts(retry_after: i64) -> Response {
    let mut res = err_res(
        StatusCode::TOO_MANY_REQUESTS,
        &crate::auth::throttle_message(retry_after),
    );
    if let Ok(v) = retry_after.max(1).to_string().parse() {
        res.headers_mut().insert(header::RETRY_AFTER, v);
    }
    res
}

/// Signed-in user together with the bearer token backing this request. Needed
/// whenever a successful MFA check must be recorded against the session.
fn require_web_user_with_token(state: &AppState, headers: &HeaderMap) -> Result<(User, String), Response> {
    let token = extract_web_token(headers, None).ok_or_else(unauthorized)?;
    let user = resolve_web_user(state, headers, None).ok_or_else(unauthorized)?;
    Ok((user, token))
}

/// Account-management gate for endpoints that need the user but not the token.
fn require_account_session(state: &AppState, headers: &HeaderMap) -> Result<User, Response> {
    require_web_user_with_token(state, headers).map(|(user, _)| user)
}

/// True when this session has proved a second factor recently.
fn session_mfa_fresh(state: &AppState, headers: &HeaderMap) -> bool {
    let Some(token) = extract_web_token(headers, None) else {
        return false;
    };
    match state.store.session_auth_state(&token) {
        Ok(Some((_, at))) => at > 0 && Utc::now().timestamp() - at < MFA_WINDOW_SECS,
        _ => false,
    }
}

/// Verify a TOTP code or an unused recovery code. Returns false when 2FA is
/// enabled and no valid factor was supplied; true when no 2FA is configured.
fn second_factor_ok(state: &AppState, user: &User, code: Option<&str>) -> Result<bool, Response> {
    if !state.store.is_2fa_enabled(user.id).unwrap_or(false) {
        return Ok(true);
    }
    let code = match code.map(str::trim).filter(|s| !s.is_empty()) {
        Some(code) => code,
        None => return Ok(false),
    };
    // A 6-digit code is guessable without a cap on attempts, so failures are
    // counted per account across sign-in and every step-up prompt.
    let rate_keys = crate::auth::rate_keys("mfa", &user.username, "");
    if let Some(wait) = crate::auth::throttled_for(&rate_keys) {
        return Err(too_many_attempts(wait));
    }
    if let Some(totp) = state.store.get_totp(user.id).ok().flatten().filter(|t| t.enabled) {
        match verify_totp(&totp.secret, code, totp.last_step) {
            Ok(Some(step)) => {
                if state.store.mark_totp_step(user.id, step).unwrap_or(false) {
                    crate::auth::record_success(&rate_keys);
                    return Ok(true);
                }
            }
            Ok(None) => {}
            Err(e) => return Err(err_res(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string())),
        }
    }
    if state.store.consume_recovery_code(user.id, code).unwrap_or(false) {
        crate::auth::record_success(&rate_keys);
        return Ok(true);
    }
    crate::auth::record_failure(&rate_keys);
    Ok(false)
}

/// Gate for sensitive operations (password change, disabling 2FA, account
/// deletion). A fresh session-level MFA check or a code in the request
/// satisfies the gate.
fn require_sensitive_auth(
    state: &AppState,
    headers: &HeaderMap,
    code: Option<&str>,
) -> Result<(User, String), Response> {
    let (user, token) = require_web_user_with_token(state, headers)?;
    if session_mfa_fresh(state, headers) {
        return Ok((user, token));
    }
    if second_factor_ok(state, &user, code)? {
        let _ = state.store.mark_session_mfa(&token);
        return Ok((user, token));
    }
    if state.store.is_2fa_enabled(user.id).unwrap_or(false) {
        return Err(err_res(
            StatusCode::FORBIDDEN,
            "A valid two-factor code is required",
        ));
    }
    Ok((user, token))
}

/// Stricter gate for the two most destructive account operations: a TOTP code
/// or an unused recovery code must be supplied every time 2FA is enabled, even
/// when the session was recently verified by another factor.
fn require_explicit_code(
    state: &AppState,
    headers: &HeaderMap,
    code: Option<&str>,
) -> Result<(User, String), Response> {
    let (user, token) = require_web_user_with_token(state, headers)?;
    if !state.store.is_2fa_enabled(user.id).unwrap_or(false) {
        return Ok((user, token));
    }
    let supplied = code.map(str::trim).filter(|s| !s.is_empty());
    if supplied.is_none() {
        return Err(err_res(
            StatusCode::FORBIDDEN,
            "A two-factor or recovery code is required",
        ));
    }
    if !second_factor_ok(state, &user, supplied)? {
        return Err(err_res(
            StatusCode::FORBIDDEN,
            "Invalid two-factor or recovery code",
        ));
    }
    let _ = state.store.mark_session_mfa(&token);
    Ok((user, token))
}

fn issue_session(state: &AppState, user: &User, headers: &HeaderMap, kind: &str) -> Result<String, Response> {
    let meta = SessionMeta {
        user_agent: user_agent(headers),
        ip: client_ip(headers),
    };
    state
        .store
        .create_session_with_meta(user.id, kind, &meta)
        .map_err(|e| err_res(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))
}

fn user_json(state: &AppState, user: &User) -> Value {
    let two_factor_enabled = state.store.is_2fa_enabled(user.id).unwrap_or(false);
    let (recovery_total, recovery_used) = state.store.recovery_code_stats(user.id).unwrap_or((0, 0));
    json!({
        "id": user.id,
        "username": user.username,
        "created": user.created,
        "avatarUrl": self_avatar_url(state, user.id),
        "twoFactorEnabled": two_factor_enabled,
        "isAdmin": crate::admin::is_admin_username(&user.username),
        "llmEnabled": state.llm.is_some(),
        "totpEnabled": state.store.get_totp(user.id).ok().flatten().map(|t| t.enabled).unwrap_or(false),
        "passkeys": state.store.passkey_count(user.id).unwrap_or(0),
        "recoveryCodesLeft": recovery_total.saturating_sub(recovery_used),
    })
}

#[derive(Deserialize, Default)]
pub struct CaptchaReq {
    #[serde(default)]
    pub action: String,
    #[serde(default)]
    pub signals: crate::captcha::BrowserSignals,
}

#[derive(Deserialize)]
pub struct AuthReq {
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub captcha: Option<crate::captcha::CaptchaSolution>,
    #[serde(default, rename = "hpToken")]
    pub hp_token: String,
}

#[derive(Deserialize)]
pub struct SignupReq {
    pub username: String,
    pub password: String,
    #[serde(rename = "confirmPassword")]
    pub confirm_password: String,
    #[serde(default)]
    pub captcha: Option<crate::captcha::CaptchaSolution>,
    #[serde(default, rename = "hpToken")]
    pub hp_token: String,
}

#[derive(Deserialize)]
pub struct StarReq {
    pub id: String,
    pub starred: bool,
    #[serde(default = "default_kind_track")]
    pub kind: String,
}

fn default_kind_track() -> String {
    "track".to_string()
}

#[derive(Deserialize)]
pub struct RateReq {
    pub id: String,
    pub rating: i32,
}

#[derive(Deserialize)]
pub struct EventReq {
    #[serde(rename = "trackId")]
    pub track_id: String,
    pub kind: String,
    #[serde(default = "default_one")]
    pub value: f64,
    #[serde(default, rename = "clientId")]
    pub client_id: String,
    #[serde(default)]
    pub seq: u64,
}

#[derive(Deserialize)]
pub struct NowPlayingReq {
    pub playing: bool,
    #[serde(default, rename = "trackId")]
    pub track_id: String,
    #[serde(default, rename = "clientId")]
    pub client_id: String,
    #[serde(default)]
    pub seq: u64,
}

#[derive(Deserialize)]
pub struct NotInterestedReq {
    #[serde(rename = "trackId")]
    pub track_id: String,
}

fn default_one() -> f64 {
    1.0
}

#[derive(Deserialize)]
pub struct PlaylistCreateReq {
    pub name: String,
    #[serde(default)]
    pub comment: String,
    #[serde(default)]
    pub public: bool,
    #[serde(default, rename = "trackIds")]
    pub track_ids: Vec<String>,
}

#[derive(Deserialize)]
pub struct PlaylistUpdateReq {
    pub name: Option<String>,
    pub comment: Option<String>,
    pub public: Option<bool>,
}

#[derive(Deserialize)]
pub struct PlaylistAddReq {
    #[serde(rename = "trackIds")]
    pub track_ids: Vec<String>,
}

#[derive(Deserialize)]
pub struct PlaylistReorderReq {
    pub from: usize,
    pub to: usize,
}

#[derive(Deserialize)]
pub struct PasswordChangeReq {
    #[serde(rename = "currentPassword")]
    pub current_password: String,
    #[serde(rename = "newPassword")]
    pub new_password: String,
    /// Required when two-factor authentication is enabled.
    #[serde(default)]
    pub code: Option<String>,
}

#[derive(Deserialize)]
pub struct SearchQuery {
    pub q: Option<String>,
    pub mode: Option<String>,
    pub phase: Option<String>,
}

#[derive(Deserialize)]
pub struct HomeQuery {
    pub tz: Option<i32>,
}

#[derive(Deserialize)]
pub struct StatsQuery {
    pub range: Option<String>,
    pub tz: Option<i32>,
}

#[derive(Deserialize)]
pub struct CreateShareReq {
    pub kind: String,
    #[serde(default, rename = "targetId")]
    pub target_id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub subtitle: String,
    #[serde(default, rename = "trackIds")]
    pub track_ids: Vec<String>,
}

#[derive(Deserialize)]
pub struct RadioQuery {
    pub seed: Option<String>,
    pub playlist: Option<String>,
    pub exclude: Option<String>,
    #[serde(default)]
    pub recent: Option<String>,
    #[serde(default)]
    pub mode: Option<String>,
}

#[derive(Deserialize)]
pub struct StreamQuery {
    pub token: Option<String>,
    #[serde(rename = "t")]
    pub offset: Option<f64>,
    pub compat: Option<String>,
}

pub async fn me(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match resolve_web_user(&state, &headers, None) {
        Some(u) => json_res(json!({ "user": user_json(&state, &u) })),
        None => json_res(json!({ "user": null })),
    }
}

fn captcha_err_res(err: crate::captcha::CaptchaError) -> Response {
    match err {
        crate::captcha::CaptchaError::RateLimited(wait) => {
            let mut res = (
                StatusCode::TOO_MANY_REQUESTS,
                [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
                json!({ "error": err.message(), "captchaRequired": true }).to_string(),
            )
                .into_response();
            if let Ok(v) = wait.max(1).to_string().parse() {
                res.headers_mut().insert(header::RETRY_AFTER, v);
            }
            res
        }
        other => (
            StatusCode::FORBIDDEN,
            [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
            json!({ "error": other.message(), "captchaRequired": true }).to_string(),
        )
            .into_response(),
    }
}

pub async fn auth_captcha(headers: HeaderMap, body: Bytes) -> Response {
    if !crate::captcha::is_enabled() {
        return json_res(json!({ "enabled": false }));
    }
    let req: CaptchaReq = if body.is_empty() {
        CaptchaReq::default()
    } else {
        serde_json::from_slice(&body).unwrap_or_default()
    };
    let ip = rate_limit_ip(&headers);
    match crate::captcha::issue_challenge(&headers, &ip, &req.action, &req.signals) {
        Ok(challenge) => json_res(serde_json::to_value(challenge).unwrap_or_else(|_| json!({}))),
        Err(err) => captcha_err_res(err),
    }
}

pub async fn signin(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<AuthReq>,
) -> Response {
    let ip = rate_limit_ip(&headers);
    let rate_keys = crate::auth::rate_keys("signin", &req.username, &ip);
    if let Some(wait) = crate::auth::throttled_for(&rate_keys) {
        return too_many_attempts(wait);
    }
    if let Err(err) = crate::captcha::verify_solution(
        &headers,
        &ip,
        "signin",
        req.captcha.as_ref(),
        &req.hp_token,
    ) {
        return captcha_err_res(err);
    }
    let store = state.store.clone();
    let (username, password) = (req.username.clone(), req.password.clone());
    // Argon2 verification is CPU-bound; keep it off the async workers.
    let outcome = tokio::task::spawn_blocking(move || store.authenticate_user(&username, &password))
        .await
        .unwrap_or_else(|_| Err(crate::models::err_msg("Sign-in failed")));
    match &outcome {
        Ok(_) => crate::auth::record_success(&rate_keys),
        Err(e) if e.0 == ERR_RESTORE_REQUIRED => crate::auth::record_success(&rate_keys),
        Err(_) => {
            crate::auth::record_failure(&rate_keys);
            crate::captcha::record_ip_failure(&ip);
        }
    }
    match outcome {
        Ok(user) => {
            if state.store.is_2fa_enabled(user.id).unwrap_or(false) {
                return match state.store.create_auth_challenge(user.id, "mfa_login", "") {
                    Ok(challenge) => json_res(json!({
                        "mfaRequired": true,
                        "challenge": challenge,
                        "totp": state.store.get_totp(user.id).ok().flatten().map(|t| t.enabled).unwrap_or(false),
                        "passkeys": state.store.passkey_count(user.id).unwrap_or(0),
                    })),
                    Err(e) => err_res(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
                };
            }
            match issue_session(&state, &user, &headers, "web") {
                Ok(token) => json_res(json!({
                    "token": token,
                    "created": false,
                    "user": user_json(&state, &user),
                })),
                Err(r) => r,
            }
        }
        Err(e) if e.0 == ERR_RESTORE_REQUIRED => {
            // The password was correct but the account is inside its recovery
            // window. Hand out a one-time challenge rather than creating a
            // duplicate account with the same username.
            let Some(user) = state.store.get_user_by_name(&req.username).ok().flatten() else {
                return err_res(StatusCode::UNAUTHORIZED, "Invalid username or password");
            };
            // Bind the challenge to the account's current restore hash so a
            // stolen challenge cannot be replayed after a re-deletion.
            let restore_hash = state
                .store
                .restore_hash(user.id)
                .ok()
                .flatten()
                .unwrap_or_default();
            match state.store.create_auth_challenge(user.id, "restore", &restore_hash) {
                Ok(challenge) => json_res(json!({
                    "restoreRequired": true,
                    "challenge": challenge,
                    "username": user.username,
                    "expiresIn": crate::auth::RESTORE_WINDOW_SECS,
                })),
                Err(e) => err_res(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
            }
        }
        Err(e) => err_res(StatusCode::UNAUTHORIZED, &e.to_string()),
    }
}

pub async fn signup(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<SignupReq>,
) -> Response {
    let ip = rate_limit_ip(&headers);
    if let Some(wait) = crate::captcha::signup_throttled_for(&ip) {
        return too_many_attempts(wait);
    }
    if let Err(err) = crate::captcha::verify_solution(
        &headers,
        &ip,
        "signup",
        req.captcha.as_ref(),
        &req.hp_token,
    ) {
        return captcha_err_res(err);
    }
    if req.password != req.confirm_password {
        return err_res(StatusCode::BAD_REQUEST, "Passwords do not match");
    }
    if crate::admin::is_admin_username(&req.username) {
        return err_res(StatusCode::CONFLICT, crate::admin::RESERVED_ADMIN_NAME);
    }
    let store = state.store.clone();
    let (username, password) = (req.username.clone(), req.password.clone());
    let created = run_blocking(move || store.create_user(&username, &password))
        .await
        .unwrap_or_else(|| Err(crate::models::err_msg("Could not create account")));
    match created {
        Ok(user) => {
            crate::captcha::record_ip_signup(&ip);
            let taste = state.taste.clone();
            let uid = user.id;
            tokio::spawn(async move {
                taste.prime_new_user(uid).await;
            });
            match issue_session(&state, &user, &headers, "web") {
                Ok(token) => json_res(json!({
                    "token": token,
                    "created": true,
                    "user": user_json(&state, &user),
                })),
                Err(r) => r,
            }
        }
        Err(e) => err_res(StatusCode::CONFLICT, &e.to_string()),
    }
}

pub async fn signout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(tok) = extract_web_token(&headers, None) {
        let _ = state.store.delete_session(&tok);
    }
    json_res(json!({ "ok": true }))
}

#[derive(Deserialize)]
pub struct RevokeSessionReq {
    pub id: String,
}

/// List the active login sessions for the signed-in user. The full session id
/// is returned so the UI can revoke individual entries; only authenticated
/// owners can reach this endpoint.
pub async fn list_sessions(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let user = match require_account_session(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let current_token = extract_web_token(&headers, None).unwrap_or_default();
    let sessions = state.store.list_sessions(user.id).unwrap_or_default();
    let items: Vec<Value> = sessions
        .into_iter()
        .map(|s| {
            json!({
                "id": s.token,
                "created": s.created,
                "lastSeen": s.last_seen,
                "userAgent": s.user_agent,
                "ip": s.ip,
                "current": s.token == current_token,
            })
        })
        .collect();
    json_res(json!({ "count": items.len(), "sessions": items }))
}

/// Revoke one session belonging to the signed-in user.
pub async fn revoke_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<RevokeSessionReq>,
) -> Response {
    let user = match require_account_session(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let removed = state
        .store
        .revoke_session_for_user(&req.id, user.id)
        .unwrap_or(false);
    if removed {
        json_res(json!({ "ok": true }))
    } else {
        err_res(StatusCode::NOT_FOUND, "Session not found")
    }
}

/// Revoke every session belonging to the signed-in user, including this one.
pub async fn revoke_all_sessions(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let user = match require_account_session(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let revoked = state.store.delete_all_sessions_for_user(user.id).unwrap_or(0);
    json_res(json!({ "ok": true, "revoked": revoked }))
}

#[derive(Deserialize, Default)]
pub struct CodeReq {
    #[serde(default)]
    pub code: Option<String>,
}

/// Parses an optional JSON body so older callers that send no body keep working.
fn optional_code(body: &Bytes) -> Option<String> {
    if body.is_empty() {
        return None;
    }
    serde_json::from_slice::<CodeReq>(body)
        .ok()
        .and_then(|r| r.code)
}

pub async fn change_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<PasswordChangeReq>,
) -> Response {
    // Check the password before the 2FA code so a typo in the password
    // doesn't burn a one-time recovery code.
    let (user, _) = match require_web_user_with_token(&state, &headers) {
        Ok(value) => value,
        Err(r) => return r,
    };
    if let Err(r) = check_current_password(&user, &req.current_password, "Current password is incorrect").await {
        return r;
    }
    let (user, token) = match require_explicit_code(&state, &headers, req.code.as_deref()) {
        Ok(value) => value,
        Err(r) => return r,
    };
    let store = state.store.clone();
    let (uid, new_password) = (user.id, req.new_password.clone());
    let changed = run_blocking(move || store.change_password(uid, &new_password))
        .await
        .unwrap_or_else(|| Err(crate::models::err_msg("Could not change password")));
    match changed {
        Ok(()) => {
            // Anyone who signed in with the old password is signed out; this
            // browser stays signed in.
            let signed_out = state.store.delete_other_sessions(user.id, &token).unwrap_or(0);
            state.store.record_security_event(
                user.id,
                "password_changed",
                &format!("Account password changed ({signed_out} other sessions signed out)"),
                &client_ip(&headers),
            );
            json_res(json!({ "ok": true, "signedOut": signed_out }))
        }
        Err(e) => err_res(StatusCode::BAD_REQUEST, &e.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Two-factor authentication, passkeys & account lifecycle
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct MfaVerifyReq {
    pub challenge: String,
    pub code: String,
}

/// Completes a password + code sign-in after `signin` reported `mfaRequired`.
pub async fn mfa_verify(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<MfaVerifyReq>,
) -> Response {
    let Some((user_id, _)) = state
        .store
        .take_auth_challenge(&req.challenge, "mfa_login")
        .ok()
        .flatten()
    else {
        return err_res(StatusCode::UNAUTHORIZED, "Sign-in challenge expired. Try again.");
    };
    let Some(user) = state.store.get_user_by_id(user_id).ok().flatten() else {
        return err_res(StatusCode::UNAUTHORIZED, "Account not found");
    };
    let verified = match second_factor_ok(&state, &user, Some(&req.code)) {
        Ok(v) => v,
        Err(r) => return r,
    };
    if !verified {
        return err_res(StatusCode::UNAUTHORIZED, "Invalid authentication code");
    }
    let token = match issue_session(&state, &user, &headers, "web") {
        Ok(t) => t,
        Err(r) => return r,
    };
    let _ = state.store.mark_session_mfa(&token);
    state.store.record_security_event(
        user.id,
        "mfa_signin",
        "Signed in with a second factor",
        &client_ip(&headers),
    );
    json_res(json!({ "token": token, "created": false, "user": user_json(&state, &user) }))
}

#[derive(Deserialize, Default)]
pub struct PasskeyLoginReq {
    #[serde(default)]
    pub username: Option<String>,
}

/// Begins a passkey assertion — either discoverable (usernameless) when no
/// username is supplied, or scoped to a named account when one is provided.
pub async fn passkey_login_options(
    State(state): State<AppState>,
    body: Option<Json<PasskeyLoginReq>>,
) -> Response {
    let Some(wa) = crate::auth::webauthn() else {
        return err_res(StatusCode::BAD_REQUEST, "Passkeys are unavailable on this server");
    };
    let username = body
        .and_then(|Json(req)| req.username)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let Some(username) = username else {
        let (challenge_response, auth_state) = match wa.start_discoverable_authentication() {
            Ok(v) => v,
            Err(e) => return err_res(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
        };
        let Ok(serialised) = serde_json::to_string(&auth_state) else {
            return err_res(StatusCode::INTERNAL_SERVER_ERROR, "Could not store challenge");
        };
        return match state
            .store
            .create_anonymous_auth_challenge("passkey_discoverable_login", &serialised)
        {
            Ok(challenge) => json_res(json!({ "challenge": challenge, "publicKey": challenge_response.public_key })),
            Err(e) => err_res(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
        };
    };

    let Some(user) = state.store.get_user_by_name(&username).ok().flatten() else {
        return err_res(StatusCode::UNAUTHORIZED, "No passkeys found for that account");
    };
    if state
        .store
        .user_deleted_at(user.id)
        .ok()
        .flatten()
        .unwrap_or(0)
        > 0
    {
        return err_res(StatusCode::UNAUTHORIZED, "This account is pending deletion");
    }
    let creds: Vec<Passkey> = state
        .store
        .get_passkeys_json(user.id)
        .unwrap_or_default()
        .iter()
        .filter_map(|json| serde_json::from_str::<Passkey>(json).ok())
        .collect();
    if creds.is_empty() {
        return err_res(StatusCode::BAD_REQUEST, "No passkeys found for that account");
    }
    let (challenge_response, auth_state) = match wa.start_passkey_authentication(&creds) {
        Ok(v) => v,
        Err(e) => return err_res(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let Ok(serialised) = serde_json::to_string(&auth_state) else {
        return err_res(StatusCode::INTERNAL_SERVER_ERROR, "Could not store challenge");
    };
    match state
        .store
        .create_auth_challenge(user.id, "passkey_login", &serialised)
    {
        Ok(challenge) => json_res(json!({ "challenge": challenge, "publicKey": challenge_response.public_key })),
        Err(e) => err_res(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

#[derive(Deserialize)]
pub struct PasskeyVerifyReq {
    pub challenge: String,
    pub credential: Value,
}

pub async fn passkey_login_verify(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<PasskeyVerifyReq>,
) -> Response {
    let Some(wa) = crate::auth::webauthn() else {
        return err_res(StatusCode::BAD_REQUEST, "Passkeys are unavailable on this server");
    };
    let Ok(credential) = serde_json::from_value::<PublicKeyCredential>(req.credential) else {
        return err_res(StatusCode::BAD_REQUEST, "Malformed passkey response");
    };

    let (user_id, credential_id, passkey_json, result) = if let Some((user_id, state_json)) = state
        .store
        .take_auth_challenge(&req.challenge, "passkey_login")
        .ok()
        .flatten()
    {
        let Ok(auth_state) = serde_json::from_str::<PasskeyAuthentication>(&state_json) else {
            return err_res(StatusCode::BAD_REQUEST, "Malformed passkey response");
        };
        let result = match wa.finish_passkey_authentication(&credential, &auth_state) {
            Ok(r) => r,
            Err(e) => return err_res(StatusCode::UNAUTHORIZED, &e.to_string()),
        };
        let credential_id = assertion_credential_id(&result);
        let Some((owner_id, passkey_json)) = state
            .store
            .get_passkey_by_id(&credential_id)
            .ok()
            .flatten()
        else {
            return err_res(StatusCode::UNAUTHORIZED, "Unknown passkey");
        };
        if owner_id != user_id {
            return err_res(StatusCode::UNAUTHORIZED, "Passkey does not belong to this account");
        }
        (user_id, credential_id, passkey_json, result)
    } else if let Some(state_json) = state
        .store
        .take_anonymous_auth_challenge(&req.challenge, "passkey_discoverable_login")
        .ok()
        .flatten()
    {
        let Ok(auth_state) = serde_json::from_str::<DiscoverableAuthentication>(&state_json) else {
            return err_res(StatusCode::BAD_REQUEST, "Malformed passkey response");
        };
        let (user_uuid, cred_id_bytes) = match wa.identify_discoverable_authentication(&credential) {
            Ok(v) => v,
            Err(_) => return err_res(StatusCode::UNAUTHORIZED, "Passkey did not include an account identifier"),
        };
        let credential_id =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(cred_id_bytes);
        let Some((owner_id, passkey_json)) = state
            .store
            .get_passkey_by_id(&credential_id)
            .ok()
            .flatten()
        else {
            return err_res(StatusCode::UNAUTHORIZED, "Unknown passkey");
        };
        if Uuid::from_u128(owner_id as u128) != user_uuid {
            return err_res(StatusCode::UNAUTHORIZED, "Passkey does not belong to this account");
        }
        if state
            .store
            .user_deleted_at(owner_id)
            .ok()
            .flatten()
            .unwrap_or(0)
            > 0
        {
            return err_res(StatusCode::UNAUTHORIZED, "This account is pending deletion");
        }
        let Ok(passkey) = serde_json::from_str::<Passkey>(&passkey_json) else {
            return err_res(StatusCode::INTERNAL_SERVER_ERROR, "Stored passkey is unreadable");
        };
        let discoverable_key = DiscoverableKey::from(&passkey);
        let result = match wa.finish_discoverable_authentication(
            &credential,
            auth_state,
            &[discoverable_key],
        ) {
            Ok(r) => r,
            Err(e) => return err_res(StatusCode::UNAUTHORIZED, &e.to_string()),
        };
        (owner_id, credential_id, passkey_json, result)
    } else {
        return err_res(StatusCode::UNAUTHORIZED, "Passkey challenge expired. Try again.");
    };

    persist_passkey_use(&state, &credential_id, &passkey_json, &result);
    let Some(user) = state.store.get_user_by_id(user_id).ok().flatten() else {
        return err_res(StatusCode::UNAUTHORIZED, "Account not found");
    };
    let token = match issue_session(&state, &user, &headers, "web") {
        Ok(t) => t,
        Err(r) => return r,
    };
    let _ = state.store.mark_session_mfa(&token);
    state.store.record_security_event(
        user.id,
        "passkey_signin",
        "Signed in with a passkey",
        &client_ip(&headers),
    );
    json_res(json!({ "token": token, "created": false, "user": user_json(&state, &user) }))
}

/// Stores the updated signature counter (when it changed) and the last-used
/// time of a passkey after a successful assertion.
fn persist_passkey_use(
    state: &AppState,
    credential_id: &str,
    passkey_json: &str,
    result: &AuthenticationResult,
) {
    let mut stored = passkey_json.to_string();
    if result.needs_update() {
        if let Ok(mut passkey) = serde_json::from_str::<Passkey>(passkey_json) {
            if passkey.update_credential(result) == Some(true) {
                if let Ok(updated) = serde_json::to_string(&passkey) {
                    stored = updated;
                }
            }
        }
    }
    let _ = state.store.touch_passkey(credential_id, &stored);
}

#[derive(Deserialize)]
pub struct RestoreReq {
    pub challenge: String,
}

/// Restores a soft-deleted account after the user signs in again.
pub async fn account_restore(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<RestoreReq>,
) -> Response {
    let Some((user_id, payload)) = state
        .store
        .take_auth_challenge(&req.challenge, "restore")
        .ok()
        .flatten()
    else {
        return err_res(StatusCode::UNAUTHORIZED, "Restore request expired. Sign in again.");
    };
    let Some(current_hash) = state.store.restore_hash(user_id).ok().flatten() else {
        return err_res(StatusCode::UNAUTHORIZED, "Account is not pending deletion");
    };
    if !crate::models::constant_time_eq(&current_hash, &payload) {
        return err_res(StatusCode::UNAUTHORIZED, "Invalid restore request");
    }
    match state.store.deletion_expires_at(user_id).ok().flatten() {
        Some(expires) if expires > Utc::now().timestamp() => {}
        _ => return err_res(StatusCode::GONE, "The recovery window for this account has ended"),
    }
    if let Err(e) = state.store.restore_user(user_id) {
        return err_res(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string());
    }
    let Some(user) = state.store.get_user_by_id(user_id).ok().flatten() else {
        return err_res(StatusCode::INTERNAL_SERVER_ERROR, "Could not load restored account");
    };
    let token = match issue_session(&state, &user, &headers, "web") {
        Ok(t) => t,
        Err(r) => return r,
    };
    state.store.record_security_event(
        user.id,
        "account_restored",
        "Account restored within the recovery window",
        &client_ip(&headers),
    );
    json_res(json!({ "token": token, "user": user_json(&state, &user) }))
}

#[derive(Deserialize)]
pub struct DeleteAccountReq {
    pub password: String,
    #[serde(default)]
    pub code: Option<String>,
}

/// Soft-deletes the account, destroys every session/API key and starts the
/// seven-day recovery window.
pub async fn account_delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<DeleteAccountReq>,
) -> Response {
    let (user, _) = match require_web_user_with_token(&state, &headers) {
        Ok(value) => value,
        Err(r) => return r,
    };
    if let Err(r) = check_current_password(&user, &req.password, "Password is incorrect").await {
        return r;
    }
    let (user, _) = match require_explicit_code(&state, &headers, req.code.as_deref()) {
        Ok(value) => value,
        Err(r) => return r,
    };
    let restore_hash = hash_token(&random_token());
    state.store.record_security_event(
        user.id,
        "account_deleted",
        "Account scheduled for deletion (7-day recovery window)",
        &client_ip(&headers),
    );
    match state.store.soft_delete_user(user.id, &restore_hash) {
        Ok(()) => json_res(json!({ "ok": true, "expiresIn": RESTORE_WINDOW_SECS })),
        Err(e) => err_res(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

/// Everything the security settings screen needs in a single request.
pub async fn twofa_status(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let user = match require_account_session(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let totp = state.store.get_totp(user.id).ok().flatten();
    let (total, used) = state.store.recovery_code_stats(user.id).unwrap_or((0, 0));
    let passkeys: Vec<Value> = state
        .store
        .list_passkeys(user.id)
        .unwrap_or_default()
        .into_iter()
        .map(|p| {
            json!({
                "id": p.credential_id,
                "label": p.label,
                "created": p.created,
                "lastUsed": p.last_used,
            })
        })
        .collect();
    let events: Vec<Value> = state
        .store
        .list_security_events(user.id, 25)
        .unwrap_or_default()
        .into_iter()
        .map(|e| {
            json!({
                "id": e.id,
                "kind": e.kind,
                "detail": e.detail,
                "ip": e.ip,
                "created": e.created,
            })
        })
        .collect();
    json_res(json!({
        "twoFactorEnabled": state.store.is_2fa_enabled(user.id).unwrap_or(false),
        "totpEnabled": totp.as_ref().map(|t| t.enabled).unwrap_or(false),
        "totpCreated": totp.as_ref().map(|t| t.created).unwrap_or(0),
        "recoveryTotal": total,
        "recoveryUsed": used,
        "passkeys": passkeys,
        "events": events,
        "passkeysAvailable": crate::auth::webauthn().is_some(),
    }))
}

/// Creates (or recreates a pending) TOTP enrolment and returns its QR code.
/// When the account already has a second factor (a passkey), adding another
/// one requires proving it first, so a hijacked session cannot plant its own.
pub async fn totp_start(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let (user, _) = match require_sensitive_auth(&state, &headers, optional_code(&body).as_deref()) {
        Ok(v) => v,
        Err(r) => return r,
    };
    if let Some(existing) = state.store.get_totp(user.id).ok().flatten() {
        if existing.enabled {
            return err_res(StatusCode::CONFLICT, "Authenticator app is already set up");
        }
    }
    let secret = generate_totp_secret();
    if let Err(e) = state.store.save_totp(user.id, &secret, false) {
        return err_res(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string());
    }
    let uri = otpauth_uri("Beatlab", &user.username, &secret);
    let svg = match qr_svg(&uri) {
        Ok(s) => s,
        Err(e) => return err_res(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    json_res(json!({ "secret": secret, "uri": uri, "qrSvg": svg }))
}

#[derive(Deserialize)]
pub struct TotpEnableReq {
    pub code: String,
}

/// Confirms a TOTP enrolment with a live code and issues recovery codes.
pub async fn totp_enable(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<TotpEnableReq>,
) -> Response {
    let (user, token) = match require_web_user_with_token(&state, &headers) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let Some(totp) = state
        .store
        .get_totp(user.id)
        .ok()
        .flatten()
        .filter(|t| !t.enabled)
    else {
        return err_res(StatusCode::CONFLICT, "Start authenticator setup first");
    };
    let step = match verify_totp(&totp.secret, &req.code, totp.last_step) {
        Ok(Some(step)) => step,
        Ok(None) => return err_res(StatusCode::BAD_REQUEST, "That code is not valid"),
        Err(e) => return err_res(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    if !state.store.mark_totp_step(user.id, step).unwrap_or(false) {
        return err_res(StatusCode::BAD_REQUEST, "That code was already used");
    }
    if let Err(e) = state.store.set_totp_enabled(user.id, true) {
        return err_res(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string());
    }
    let codes = generate_recovery_codes(RECOVERY_CODE_COUNT);
    if let Err(e) = state.store.replace_recovery_codes(user.id, &codes) {
        return err_res(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string());
    }
    let _ = state.store.mark_session_mfa(&token);
    state.store.record_security_event(
        user.id,
        "totp_enabled",
        "Authenticator app enabled",
        &client_ip(&headers),
    );
    json_res(json!({ "ok": true, "recoveryCodes": codes }))
}

/// Generates a fresh set of one-time recovery codes (invalidates the old set).
pub async fn recovery_regenerate(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CodeReq>,
) -> Response {
    let (user, _) = match require_sensitive_auth(&state, &headers, req.code.as_deref()) {
        Ok(v) => v,
        Err(r) => return r,
    };
    // Recovery codes are a backup for a second factor, not a standalone
    // account feature — refuse to hand them out until 2FA is actually enabled.
    if !state.store.is_2fa_enabled(user.id).unwrap_or(false) {
        return err_res(
            StatusCode::CONFLICT,
            "Set up two-factor authentication before generating recovery codes",
        );
    }
    let codes = generate_recovery_codes(RECOVERY_CODE_COUNT);
    if let Err(e) = state.store.replace_recovery_codes(user.id, &codes) {
        return err_res(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string());
    }
    state.store.record_security_event(
        user.id,
        "recovery_codes_generated",
        "Recovery codes regenerated",
        &client_ip(&headers),
    );
    json_res(json!({ "recoveryCodes": codes }))
}

/// Removes every second factor from the account.
pub async fn twofa_disable(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CodeReq>,
) -> Response {
    let (user, _) = match require_sensitive_auth(&state, &headers, req.code.as_deref()) {
        Ok(v) => v,
        Err(r) => return r,
    };
    if let Err(e) = state.store.set_totp_enabled(user.id, false) {
        return err_res(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string());
    }
    let _ = state.store.delete_all_passkeys(user.id);
    let _ = state.store.replace_recovery_codes(user.id, &[]);
    state.store.record_security_event(
        user.id,
        "twofa_disabled",
        "Two-factor authentication disabled",
        &client_ip(&headers),
    );
    json_res(json!({ "ok": true }))
}

/// Starts a WebAuthn credential creation ceremony. Like `totp_start`, an
/// account that already has a second factor must prove it before enrolling
/// another passkey.
pub async fn passkey_register_options(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let (user, _) = match require_sensitive_auth(&state, &headers, optional_code(&body).as_deref()) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let Some(wa) = crate::auth::webauthn() else {
        return err_res(StatusCode::BAD_REQUEST, "Passkeys are unavailable on this server");
    };
    let exclude: Vec<_> = state
        .store
        .get_passkeys_json(user.id)
        .unwrap_or_default()
        .iter()
        .filter_map(|json| serde_json::from_str::<Passkey>(json).ok())
        .map(|p| p.cred_id().clone())
        .collect();
    let user_id = Uuid::from_u128(user.id as u128);
    let (creation, registration) =
        match wa.start_passkey_registration(user_id, &user.username, &user.username, Some(exclude)) {
            Ok(v) => v,
            Err(e) => return err_res(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
        };
    // Request a discoverable (resident) credential so the user can sign in
    // with their passkey without typing a username first.
    let mut public_key = match serde_json::to_value(&creation.public_key) {
        Ok(v) => v,
        Err(e) => return err_res(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    if let Some(sel) = public_key
        .get_mut("authenticatorSelection")
        .and_then(|v| v.as_object_mut())
    {
        sel.insert("residentKey".to_string(), json!("required"));
        sel.insert("requireResidentKey".to_string(), json!(true));
    }
    let Ok(serialised) = serde_json::to_string(&registration) else {
        return err_res(StatusCode::INTERNAL_SERVER_ERROR, "Could not store challenge");
    };
    match state
        .store
        .create_auth_challenge(user.id, "passkey_register", &serialised)
    {
        Ok(challenge) => json_res(json!({ "challenge": challenge, "publicKey": public_key })),
        Err(e) => err_res(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

#[derive(Deserialize)]
pub struct PasskeyRegisterVerifyReq {
    pub challenge: String,
    pub credential: Value,
    #[serde(default)]
    pub label: Option<String>,
}

pub async fn passkey_register_verify(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<PasskeyRegisterVerifyReq>,
) -> Response {
    let (user, token) = match require_web_user_with_token(&state, &headers) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let Some(wa) = crate::auth::webauthn() else {
        return err_res(StatusCode::BAD_REQUEST, "Passkeys are unavailable on this server");
    };
    let Some((user_id, state_json)) = state
        .store
        .take_auth_challenge(&req.challenge, "passkey_register")
        .ok()
        .flatten()
    else {
        return err_res(StatusCode::UNAUTHORIZED, "Registration challenge expired. Try again.");
    };
    if user_id != user.id {
        return err_res(StatusCode::FORBIDDEN, "Challenge belongs to another account");
    }
    let (registration, credential) = match (
        serde_json::from_str::<PasskeyRegistration>(&state_json),
        serde_json::from_value::<RegisterPublicKeyCredential>(req.credential),
    ) {
        (Ok(r), Ok(c)) => (r, c),
        _ => return err_res(StatusCode::BAD_REQUEST, "Malformed passkey response"),
    };
    let passkey = match wa.finish_passkey_registration(&credential, &registration) {
        Ok(p) => p,
        Err(e) => return err_res(StatusCode::BAD_REQUEST, &e.to_string()),
    };
    let credential_id = passkey_credential_id(&passkey);
    if state
        .store
        .get_passkey_by_id(&credential_id)
        .ok()
        .flatten()
        .is_some()
    {
        return err_res(StatusCode::CONFLICT, "That passkey is already registered");
    }
    let Ok(serialised) = serde_json::to_string(&passkey) else {
        return err_res(StatusCode::INTERNAL_SERVER_ERROR, "Could not store passkey");
    };
    let label = req
        .label
        .map(|s| s.trim().chars().take(64).collect::<String>())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Passkey".to_string());
    if let Err(e) = state
        .store
        .save_passkey(user.id, &credential_id, &label, &serialised)
    {
        return err_res(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string());
    }
    let _ = state.store.mark_session_mfa(&token);
    state.store.record_security_event(
        user.id,
        "passkey_added",
        &format!("Passkey registered: {label}"),
        &client_ip(&headers),
    );
    // The first passkey may be the account's only second factor. Give that
    // user the same one-time backup codes an authenticator-app user gets.
    let mut recovery_codes: Vec<String> = Vec::new();
    let totp_enabled = state
        .store
        .get_totp(user.id)
        .ok()
        .flatten()
        .map(|t| t.enabled)
        .unwrap_or(false);
    if !totp_enabled && state.store.recovery_code_stats(user.id).unwrap_or((0, 0)).0 == 0 {
        recovery_codes = generate_recovery_codes(RECOVERY_CODE_COUNT);
        if let Err(e) = state.store.replace_recovery_codes(user.id, &recovery_codes) {
            return err_res(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string());
        }
    }
    json_res(json!({
        "ok": true,
        "credentialId": credential_id,
        "label": label,
        "recoveryCodes": recovery_codes,
    }))
}

/// Re-verifies the current session with a passkey (needed when an account has
/// no authenticator app and the 10-minute step-up window has elapsed).
pub async fn passkey_stepup_options(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    let (user, _) = match require_web_user_with_token(&state, &headers) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let Some(wa) = crate::auth::webauthn() else {
        return err_res(StatusCode::BAD_REQUEST, "Passkeys are unavailable on this server");
    };
    let creds: Vec<Passkey> = state
        .store
        .get_passkeys_json(user.id)
        .unwrap_or_default()
        .iter()
        .filter_map(|json| serde_json::from_str::<Passkey>(json).ok())
        .collect();
    if creds.is_empty() {
        return err_res(StatusCode::BAD_REQUEST, "No passkeys enrolled");
    }
    let (challenge_response, auth_state) = match wa.start_passkey_authentication(&creds) {
        Ok(v) => v,
        Err(e) => return err_res(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let Ok(serialised) = serde_json::to_string(&auth_state) else {
        return err_res(StatusCode::INTERNAL_SERVER_ERROR, "Could not store challenge");
    };
    match state
        .store
        .create_auth_challenge(user.id, "passkey_stepup", &serialised)
    {
        Ok(challenge) => json_res(json!({ "challenge": challenge, "publicKey": challenge_response.public_key })),
        Err(e) => err_res(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

pub async fn passkey_stepup_verify(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<PasskeyVerifyReq>,
) -> Response {
    let (user, token) = match require_web_user_with_token(&state, &headers) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let Some(wa) = crate::auth::webauthn() else {
        return err_res(StatusCode::BAD_REQUEST, "Passkeys are unavailable on this server");
    };
    let Some((user_id, state_json)) = state
        .store
        .take_auth_challenge(&req.challenge, "passkey_stepup")
        .ok()
        .flatten()
    else {
        return err_res(StatusCode::UNAUTHORIZED, "Passkey challenge expired. Try again.");
    };
    if user_id != user.id {
        return err_res(StatusCode::FORBIDDEN, "Challenge belongs to another account");
    }
    let (auth_state, credential) = match (
        serde_json::from_str::<PasskeyAuthentication>(&state_json),
        serde_json::from_value::<PublicKeyCredential>(req.credential),
    ) {
        (Ok(a), Ok(c)) => (a, c),
        _ => return err_res(StatusCode::BAD_REQUEST, "Malformed passkey response"),
    };
    let result = match wa.finish_passkey_authentication(&credential, &auth_state) {
        Ok(r) => r,
        Err(e) => return err_res(StatusCode::UNAUTHORIZED, &e.to_string()),
    };
    let credential_id = assertion_credential_id(&result);
    let Some((owner_id, passkey_json)) = state
        .store
        .get_passkey_by_id(&credential_id)
        .ok()
        .flatten()
    else {
        return err_res(StatusCode::UNAUTHORIZED, "Unknown passkey");
    };
    if owner_id != user.id {
        return err_res(StatusCode::UNAUTHORIZED, "Passkey does not belong to this account");
    }
    persist_passkey_use(&state, &credential_id, &passkey_json, &result);
    let _ = state.store.mark_session_mfa(&token);
    state.store.record_security_event(
        user.id,
        "passkey_stepup",
        "Sensitive action authorised with a passkey",
        &client_ip(&headers),
    );
    json_res(json!({ "ok": true }))
}

pub async fn passkey_delete(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(req): Query<CodeReq>,
    headers: HeaderMap,
) -> Response {
    let (user, _) = match require_sensitive_auth(&state, &headers, req.code.as_deref()) {
        Ok(v) => v,
        Err(r) => return r,
    };
    if !state.store.delete_passkey(user.id, &id).unwrap_or(false) {
        return err_res(StatusCode::NOT_FOUND, "Passkey not found");
    }
    // Recovery codes back up a second factor. Once the last one is gone they
    // would be inert leftovers that resurface if 2FA is set up again later.
    let still_enabled = state.store.is_2fa_enabled(user.id).unwrap_or(true);
    if !still_enabled {
        let _ = state.store.replace_recovery_codes(user.id, &[]);
    }
    state.store.record_security_event(
        user.id,
        "passkey_removed",
        if still_enabled {
            "Passkey removed"
        } else {
            "Last passkey removed — two-factor authentication is now off"
        },
        &client_ip(&headers),
    );
    json_res(json!({ "ok": true, "twoFactorEnabled": still_enabled }))
}

const NEW_RELEASES_ARTIST_POOL: usize = 40;
const NEW_RELEASES_TTL_NON_EMPTY_SECS: i64 = 3600;
const NEW_RELEASES_TTL_EMPTY_SECS: i64 = 900;

fn new_releases_window_start() -> i64 {
    let cutoff = Utc::now().date_naive()
        - chrono::Duration::days(crate::sources::deezer::NEW_RELEASE_WINDOW_DAYS);
    cutoff
        .and_hms_opt(0, 0, 0)
        .map(|dt| dt.and_utc().timestamp())
        .unwrap_or_else(|| {
            Utc::now().timestamp() - crate::sources::deezer::NEW_RELEASE_WINDOW_DAYS * 86_400
        })
}

fn new_releases_dedup_key(a: &AlbumSummary) -> String {
    format!("{}|{}", a.artist_id, slug(&a.name))
}

fn stored_new_releases(store: &crate::store::Store, uid: i64, limit: usize) -> Vec<AlbumSummary> {
    let last_key = format!("newrel:last2:{uid}");
    let window_start = new_releases_window_start();
    let stored: Vec<AlbumSummary> = store
        .kv_get(&last_key)
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default();
    let mut out = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();
    let mut seen_keys: HashSet<String> = HashSet::new();
    for album in stored {
        if album.created < window_start {
            continue;
        }
        if seen_ids.insert(album.id.clone()) && seen_keys.insert(new_releases_dedup_key(&album)) {
            out.push(album);
        }
    }
    out.sort_by(|a, b| b.created.cmp(&a.created).then_with(|| a.name.cmp(&b.name)));
    out.truncate(limit);
    out
}

fn new_releases_is_fresh(store: &crate::store::Store, uid: i64, has_items: bool) -> bool {
    let key = format!("newrel:fetched:{uid}");
    let Some(raw) = store.kv_get(&key) else {
        return false;
    };
    let Some((ts_str, win_str)) = raw.split_once('|') else {
        return false;
    };
    let Ok(ts) = ts_str.parse::<i64>() else {
        return false;
    };
    let Ok(win) = win_str.parse::<i64>() else {
        return false;
    };
    if win != crate::sources::deezer::NEW_RELEASE_WINDOW_DAYS {
        return false;
    }
    let ttl = if has_items {
        NEW_RELEASES_TTL_NON_EMPTY_SECS
    } else {
        NEW_RELEASES_TTL_EMPTY_SECS
    };
    Utc::now().timestamp() - ts < ttl
}

/// Starts (or joins) the deduplicated background sweep that refreshes a user's
/// "New Releases" shelf and persists the complete result to SQLite. Running the
/// sweep detached from the HTTP handler ensures slow Deezer bursts or startup
/// catalog warming never drop straggler artists on the floor.
fn ensure_new_releases_refresh(
    state: &AppState,
    uid: i64,
) -> tokio::sync::watch::Receiver<bool> {
    let mut inflight = state
        .new_releases_inflight
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if let Some(existing) = inflight.get(&uid) {
        if !*existing.borrow() {
            return existing.clone();
        }
    }
    let (tx, rx) = tokio::sync::watch::channel(false);
    inflight.insert(uid, rx.clone());
    drop(inflight);

    let st = state.clone();
    tokio::spawn(async move {
        struct InflightGuard {
            map: crate::state::NewReleasesInflight,
            uid: i64,
            tx: tokio::sync::watch::Sender<bool>,
        }
        impl Drop for InflightGuard {
            fn drop(&mut self) {
                let mut g = self.map.lock().unwrap_or_else(|e| e.into_inner());
                g.remove(&self.uid);
                let _ = self.tx.send(true);
            }
        }
        let _guard = InflightGuard {
            map: st.new_releases_inflight.clone(),
            uid,
            tx,
        };
        refresh_new_releases_for_user(&st, uid).await;
    });
    rx
}

/// Runs a complete personalised "New Releases" sweep for `uid` across their
/// followed, starred, saved-album, and recently listened artists, merging into
/// the persisted shelf (`newrel:last:{uid}`). Uses Deezer when reachable and
/// automatically falls back to Apple's public iTunes catalog API when Deezer
/// refuses server IPs (HTTP 403) or fails to answer.
async fn refresh_new_releases_for_user(state: &AppState, uid: i64) {
    let artists = state
        .store
        .relevant_artists_for_releases(uid, NEW_RELEASES_ARTIST_POOL)
        .unwrap_or_default();
    let now = Utc::now().timestamp();
    let fetched_key = format!("newrel:fetched:{uid}");
    let fetched_val = format!("{now}|{}", crate::sources::deezer::NEW_RELEASE_WINDOW_DAYS);
    if artists.is_empty() {
        state.store.kv_set(&fetched_key, &fetched_val);
        return;
    }

    // Song titles the user played or starred, per artist, so ambiguous names
    // ("Eve", "ASU") and localized names ("幾田りら" / "米津玄師") resolve accurately
    // without cross-contaminating homonyms with different capitalization.
    let mut hint_titles_by_id: HashMap<String, Vec<String>> = HashMap::new();
    let mut hint_titles_exact: HashMap<String, Vec<String>> = HashMap::new();
    let mut hint_titles_slug: HashMap<String, Vec<String>> = HashMap::new();
    let mut record_hint = |t: &Track| {
        let aid = t.artist_id();
        if !aid.is_empty() {
            let titles = hint_titles_by_id.entry(aid).or_default();
            if titles.len() < 15 && !titles.contains(&t.title) {
                titles.push(t.title.clone());
            }
        }
        let primary = crate::models::primary_artist(&t.artist);
        let name = if primary.is_empty() {
            t.artist.trim()
        } else {
            primary.as_str()
        };
        if !name.is_empty() {
            let titles = hint_titles_exact.entry(name.to_string()).or_default();
            if titles.len() < 15 && !titles.contains(&t.title) {
                titles.push(t.title.clone());
            }
            if crate::models::allow_case_insensitive_artist_merge(name) {
                let titles = hint_titles_slug.entry(slug(name)).or_default();
                if titles.len() < 15 && !titles.contains(&t.title) {
                    titles.push(t.title.clone());
                }
            }
        }
    };
    for (t, _) in state.store.user_recent_tracks(uid, 500, 0).unwrap_or_default() {
        record_hint(&t);
    }
    for (t, _) in state.store.starred_tracks(uid).unwrap_or_default() {
        record_hint(&t);
    }
    let hints_for_artist = |artist: &ArtistSummary| -> Vec<String> {
        let mut out = Vec::new();
        for src in [
            hint_titles_by_id.get(&artist.id),
            hint_titles_exact.get(artist.name.trim()),
            crate::models::allow_case_insensitive_artist_merge(&artist.name)
                .then(|| hint_titles_slug.get(&slug(&artist.name)))
                .flatten(),
        ]
        .into_iter()
        .flatten()
        {
            for title in src {
                if out.len() < 15 && !out.contains(title) {
                    out.push(title.clone());
                }
            }
        }
        out
    };

    let total = artists.len();
    let mut answered = 0usize;
    let mut albums: Vec<AlbumSummary> = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();
    let mut seen_keys: HashSet<String> = HashSet::new();
    let mut valid_artist_ids: HashSet<String> = artists.iter().map(|a| a.id.clone()).collect();
    let valid_artist_names: HashSet<String> = artists
        .iter()
        .map(|a| a.name.trim().to_string())
        .filter(|n| !n.is_empty())
        .collect();
    for a in &artists {
        let dz = a.deezer_id.trim();
        if !dz.is_empty() && dz != "0" {
            valid_artist_ids.insert(format!("ar:dz:{dz}"));
        }
    }
    let mut source_used = "deezer";

    if !state.sources.deezer.is_blocked() {
        let handles: Vec<_> = artists
            .iter()
            .map(|artist| {
                let dz = state.sources.deezer.clone();
                let store = state.store.clone();
                let mut id = artist.deezer_id.clone();
                let name = artist.name.clone();
                let hints = hints_for_artist(artist);
                tokio::spawn(async move {
                    if id.is_empty() {
                        match resolve_deezer_artist_by_name(&dz, &store, uid, &name, &hints).await {
                            ResolveArtist::Found(found) => id = found,
                            ResolveArtist::NotFound => return Some((None, Vec::new())),
                            ResolveArtist::TransientError => return None,
                        }
                    }
                    let resolved_id = (!id.is_empty()).then(|| format!("ar:dz:{id}"));
                    dz.fetch_artist_releases_checked(&id, &name, 4)
                        .await
                        .map(|batch| (resolved_id, batch))
                })
            })
            .collect();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(45);
        for handle in handles {
            if let Ok(Ok(Some((resolved_id, batch)))) =
                tokio::time::timeout_at(deadline, handle).await
            {
                answered += 1;
                if let Some(rid) = resolved_id {
                    valid_artist_ids.insert(rid);
                }
                for album in batch {
                    if seen_ids.insert(album.id.clone())
                        && seen_keys.insert(new_releases_dedup_key(&album))
                    {
                        albums.push(album);
                    }
                }
            }
        }
    }

    // Fallback to Apple's iTunes catalog API whenever Deezer is 403-blocked or
    // failed to answer any artists (common on cloud datacenter IPs).
    let mut itunes_incomplete = false;
    if answered == 0 && !state.sources.itunes.is_blocked() {
        source_used = "itunes";
        let hints_per_artist: Vec<Vec<String>> = artists.iter().map(&hints_for_artist).collect();
        let (it_answered, it_albums, remaining) =
            collect_itunes_new_releases(state, uid, &artists, &hints_per_artist).await;
        answered = it_answered;
        itunes_incomplete = remaining > 0;
        for album in it_albums {
            if seen_ids.insert(album.id.clone()) && seen_keys.insert(new_releases_dedup_key(&album))
            {
                albums.push(album);
            }
        }
    }
    let live = albums.len();

    // Merge in the last good shelf, retaining only releases still inside the
    // window AND belonging to artists currently in this user's pool. That way a
    // transient failure on one artist never wipes out real releases, while
    // unfollowed or previously mis-resolved artists drop off immediately.
    let last_key = format!("newrel:last2:{uid}");
    let window_start = new_releases_window_start();
    let stored: Vec<AlbumSummary> = state
        .store
        .kv_get(&last_key)
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default();
    for album in stored {
        if album.created < window_start {
            continue;
        }
        let artist_still_relevant = valid_artist_ids.contains(&album.artist_id)
            || valid_artist_names.contains(album.artist.trim());
        if !artist_still_relevant {
            continue;
        }
        if seen_ids.insert(album.id.clone()) && seen_keys.insert(new_releases_dedup_key(&album)) {
            albums.push(album);
        }
    }
    albums.retain(|a| a.created >= window_start);
    albums.sort_by(|a, b| b.created.cmp(&a.created).then_with(|| a.name.cmp(&b.name)));
    albums.truncate(24);
    for a in &albums {
        let _ = state.store.upsert_album_meta(a);
    }
    if let Ok(raw) = serde_json::to_string(&albums) {
        state.store.kv_set(&last_key, &raw);
    }
    // Mark the shelf freshly checked once the active source actually answered.
    // If iTunes only resolved a partial cold batch (rate-limit budget), keep it
    // fresh briefly if we already have releases, or retry on the next visit if empty.
    let source_ok = if source_used == "itunes" {
        !state.sources.itunes.is_blocked() && (!albums.is_empty() || (!itunes_incomplete && answered > 0))
    } else {
        !state.sources.deezer.is_blocked() && (!albums.is_empty() || answered * 2 >= total)
    };
    if source_ok {
        state.store.kv_set(&fetched_key, &fetched_val);
    }

    tracing::info!(
        uid,
        source = source_used,
        artists = total,
        answered,
        live,
        shown = albums.len().min(8),
        "home new releases"
    );
}

fn itunes_artist_cache_key(uid: i64, name: &str) -> String {
    if crate::models::allow_case_insensitive_artist_merge(name) {
        format!("itartist:byname2:{name}")
    } else {
        format!("itartist:byname2:u{uid}:{name}")
    }
}

fn deezer_artist_cache_key(uid: i64, name: &str) -> String {
    if crate::models::allow_case_insensitive_artist_merge(name) {
        format!("dzartist:byname4:{name}")
    } else {
        format!("dzartist:byname4:u{uid}:{name}")
    }
}

/// Resolves artists via Apple's iTunes catalog (caching IDs permanently in SQLite)
/// and fetches their recent releases in batched multi-ID lookups (12 artists/request).
/// Caps uncached artist searches per sweep to stay comfortably inside Apple's rate limit.
async fn collect_itunes_new_releases(
    state: &AppState,
    uid: i64,
    artists: &[ArtistSummary],
    hints_per_artist: &[Vec<String>],
) -> (usize, Vec<AlbumSummary>, usize) {
    const MAX_UNCACHED_LOOKUPS_PER_SWEEP: usize = 12;

    let mut resolved: Vec<(u64, String, String)> = Vec::new();
    let mut uncached_used = 0usize;
    let mut remaining_uncached = 0usize;
    let now = Utc::now().timestamp();

    for (idx, artist) in artists.iter().enumerate() {
        let name = artist.name.trim();
        if name.is_empty() {
            continue;
        }
        let cache_key = itunes_artist_cache_key(uid, name);
        if let Some(raw) = state.store.kv_get(&cache_key) {
            if let Some((ts, id_str)) = raw.split_once('|') {
                if let Ok(it_id) = id_str.parse::<u64>() {
                    if it_id > 0 {
                        resolved.push((it_id, artist.id.clone(), name.to_string()));
                        continue;
                    }
                }
                if id_str.is_empty() && ts.parse::<i64>().map_or(false, |t| now - t < 86_400) {
                    continue;
                }
            }
        }
        if uncached_used >= MAX_UNCACHED_LOOKUPS_PER_SWEEP || state.sources.itunes.is_blocked() {
            remaining_uncached += 1;
            continue;
        }
        uncached_used += 1;
        let hints = hints_per_artist
            .get(idx)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        if let ResolveArtist::Found(it_id_str) =
            resolve_itunes_artist_by_name(&state.sources.itunes, &state.store, uid, name, hints)
                .await
        {
            if let Ok(it_id) = it_id_str.parse::<u64>() {
                if it_id > 0 {
                    resolved.push((it_id, artist.id.clone(), name.to_string()));
                }
            }
        }
    }

    if resolved.is_empty() {
        return (0, Vec::new(), remaining_uncached);
    }
    let (answered, albums) = state
        .sources
        .itunes
        .fetch_artists_recent_releases(&resolved, 4)
        .await;
    (answered, albums, remaining_uncached)
}

async fn resolve_itunes_artist_by_name(
    it: &crate::sources::itunes::ItunesSource,
    store: &crate::store::Store,
    uid: i64,
    name: &str,
    played_titles: &[String],
) -> ResolveArtist {
    let name = name.trim();
    if slug(name).is_empty() {
        return ResolveArtist::NotFound;
    }
    let cache_key = itunes_artist_cache_key(uid, name);
    let now = Utc::now().timestamp();
    let Some(raw_results) = it.search_artists(name, 8).await else {
        return ResolveArtist::TransientError;
    };
    let found = pick_itunes_artist_from_search(it, name, raw_results, played_titles).await;
    match found {
        Some(a) => {
            store.kv_set(&cache_key, &format!("{now}|{}", a.id));
            ResolveArtist::Found(a.id.to_string())
        }
        None => {
            store.kv_set(&cache_key, &format!("{now}|"));
            ResolveArtist::NotFound
        }
    }
}

async fn pick_itunes_artist_from_search(
    it: &crate::sources::itunes::ItunesSource,
    name: &str,
    raw_results: Vec<crate::sources::itunes::ItunesArtist>,
    played_titles: &[String],
) -> Option<crate::sources::itunes::ItunesArtist> {
    let wanted = name.to_lowercase();
    let key_name = slug(name);
    let query_ascii = name.is_ascii();
    let allow_ci = crate::models::allow_case_insensitive_artist_merge(name);

    let exact: Vec<crate::sources::itunes::ItunesArtist> = raw_results
        .iter()
        .filter(|a| {
            let low = a.name.trim().to_lowercase();
            if query_ascii && !allow_ci {
                low == wanted
            } else {
                low == wanted || slug(&a.name) == key_name
            }
        })
        .cloned()
        .collect();

    // Cross-script fallback: handles BOTH "幾田りら" -> "Lilas Ikuta" AND
    // "Hina Aoki" -> "青木陽菜" where Apple stores the artist in the other script.
    let cross_script_top: Option<crate::sources::itunes::ItunesArtist> = raw_results
        .first()
        .filter(|a| a.name.is_ascii() != query_ascii && is_solo_artist_name(&a.name))
        .cloned();

    let mut candidates = exact.clone();
    if let Some(ref cs) = cross_script_top {
        if !candidates.iter().any(|c| c.id == cs.id) {
            candidates.push(cs.clone());
        }
    }
    if candidates.is_empty() {
        return None;
    }

    // Prioritize exact-case matches first so track-overlap verification inspects
    // them before any differently-cased homonyms.
    candidates.sort_by_key(|a| if a.name.trim() == name { 0u8 } else { 1u8 });

    let played: Vec<String> = played_titles
        .iter()
        .map(|t| slug(t))
        .filter(|t| t.len() >= 3)
        .collect();
    let had_played_hints = !played.is_empty();

    if candidates.len() == 1 && (!query_ascii || allow_ci || !had_played_hints) {
        if !query_ascii || allow_ci || candidates[0].name.trim() == name {
            return candidates.pop();
        }
        return None;
    }

    let exact_case_count = candidates.iter().filter(|a| a.name.trim() == name).count();
    if !had_played_hints && exact_case_count == 1 && cross_script_top.is_none() {
        if let Some(i) = candidates.iter().position(|a| a.name.trim() == name) {
            return Some(candidates.swap_remove(i));
        }
    }

    if had_played_hints {
        let mut best: Option<(usize, usize)> = None;
        for (i, cand) in candidates.iter().enumerate().take(5) {
            let songs = it.fetch_artist_top_songs(cand.id, 30).await;
            let hits = songs
                .iter()
                .map(|t| slug(t))
                .filter(|t| t.len() >= 3)
                .filter(|t| {
                    played
                        .iter()
                        .any(|p| t == p || t.starts_with(p.as_str()) || p.starts_with(t.as_str()))
                })
                .count();
            if hits > 0 && best.map_or(true, |(_, b)| hits > b) {
                best = Some((i, hits));
            }
        }
        if let Some((i, _)) = best {
            return Some(candidates.swap_remove(i));
        }
    }

    pick_itunes_artist_fallback(name, exact, cross_script_top, candidates, had_played_hints)
}

fn pick_itunes_artist_fallback(
    name: &str,
    exact: Vec<crate::sources::itunes::ItunesArtist>,
    cross_script_top: Option<crate::sources::itunes::ItunesArtist>,
    mut candidates: Vec<crate::sources::itunes::ItunesArtist>,
    had_played_hints: bool,
) -> Option<crate::sources::itunes::ItunesArtist> {
    if candidates.is_empty() {
        return None;
    }
    if exact.is_empty() {
        if let Some(cs) = cross_script_top {
            return Some(cs);
        }
    }
    // For short single-word ASCII names ("ASU", "Eve", "LiSA", "HACHI"), never
    // blindly fall back to a differently-cased homonym (e.g. "ASU" -> "Asu") or
    // guess among multiple exact-case homonyms when played-track verification
    // found zero matches.
    if name.is_ascii() && !crate::models::allow_case_insensitive_artist_merge(name) {
        let exact_case: Vec<usize> = candidates
            .iter()
            .enumerate()
            .filter(|(_, a)| a.name.trim() == name)
            .map(|(i, _)| i)
            .collect();
        if exact_case.is_empty() {
            return None;
        }
        if exact_case.len() > 1 && had_played_hints {
            return None;
        }
        return Some(candidates.swap_remove(exact_case[0]));
    }
    let exact_case_idx = candidates.iter().position(|a| a.name.trim() == name);
    let first = exact_case_idx.unwrap_or(0);
    Some(candidates.swap_remove(first))
}

#[derive(Debug, PartialEq, Eq)]
enum ResolveArtist {
    Found(String),
    NotFound,
    TransientError,
}

fn is_solo_artist_name(name: &str) -> bool {
    let low = name.trim().to_lowercase();
    if low.is_empty() {
        return false;
    }
    for marker in [
        " feat.",
        " feat ",
        " ft.",
        " ft ",
        " featuring ",
        " & ",
        " + ",
        " x ",
        " × ",
        " cv:",
        "(cv:",
        "（cv:",
        ", ",
    ] {
        if low.contains(marker) {
            return false;
        }
    }
    true
}

/// Finds an artist's Deezer ID from their name (for artists known only via
/// YouTube Music or by name).
///
/// - For ASCII names, only exact case-insensitive/slug matches are accepted so
///   unrelated artists ("ASCA" vs "Cascada") are never matched.
/// - For non-ASCII names (e.g. Japanese/Korean/Chinese like "幾田りら", "米津玄師",
///   "青木陽菜"), Deezer frequently indexes the artist's real catalog under their
///   romanized name ("Lilas Ikuta", "Kenshi Yonezu", "Hina Aoki") while native-script
///   entries are either absent or 0-album stubs, so Deezer's top solo candidate is
///   also considered and verified against `played_titles` / discography size.
/// - Transient Deezer errors are never cached as misses.
async fn resolve_deezer_artist_by_name(
    dz: &crate::sources::deezer::DeezerSource,
    store: &crate::store::Store,
    uid: i64,
    name: &str,
    played_titles: &[String],
) -> ResolveArtist {
    let name = name.trim();
    let key_name = slug(name);
    if key_name.is_empty() {
        return ResolveArtist::NotFound;
    }
    // v4: homonym-safe for short ASCII names + per-user cache isolation.
    let cache_key = deezer_artist_cache_key(uid, name);
    let now = Utc::now().timestamp();
    if let Some(raw) = store.kv_get(&cache_key) {
        if let Some((ts, id)) = raw.split_once('|') {
            if !id.is_empty() {
                return ResolveArtist::Found(id.to_string());
            }
            // Cached miss: retry after a day.
            if ts.parse::<i64>().map_or(false, |t| now - t < 86_400) {
                return ResolveArtist::NotFound;
            }
        }
    }
    let Some(raw_results) = dz.search_artists_checked(name, 8).await else {
        return ResolveArtist::TransientError;
    };
    let found = pick_artist_from_search(dz, name, raw_results, played_titles).await;
    match found {
        Some(a) => {
            let _ = store.upsert_artist_meta(&a);
            store.kv_set(&cache_key, &format!("{now}|{}", a.deezer_id));
            ResolveArtist::Found(a.deezer_id)
        }
        None => {
            store.kv_set(&cache_key, &format!("{now}|"));
            ResolveArtist::NotFound
        }
    }
}

/// Filters and ranks Deezer artist search results for `name`.
async fn pick_artist_from_search(
    dz: &crate::sources::deezer::DeezerSource,
    name: &str,
    raw_results: Vec<ArtistSummary>,
    played_titles: &[String],
) -> Option<ArtistSummary> {
    let non_ascii = !name.is_ascii();
    let wanted = name.to_lowercase();
    let key_name = slug(name);
    let allow_ci = crate::models::allow_case_insensitive_artist_merge(name);

    let mut exact: Vec<ArtistSummary> = raw_results
        .iter()
        .filter(|a| {
            let low = a.name.trim().to_lowercase();
            if !non_ascii && !allow_ci {
                low == wanted
            } else {
                low == wanted || slug(&a.name) == key_name
            }
        })
        .cloned()
        .collect();
    if exact.iter().any(|a| a.album_count > 0) {
        exact.retain(|a| a.album_count > 0);
    }

    // For non-ASCII queries, Deezer often lists the artist's real catalog under
    // their romanized name as the #1 search hit (e.g. "幾田りら" -> "Lilas Ikuta",
    // "米津玄師" -> "Kenshi Yonezu") while the native-script entry is missing or a stub.
    let top_solo: Option<ArtistSummary> = if non_ascii {
        raw_results
            .iter()
            .take(2)
            .find(|a| a.album_count > 0 && is_solo_artist_name(&a.name))
            .cloned()
    } else {
        None
    };

    let mut candidates = exact.clone();
    if non_ascii {
        for cand in raw_results
            .iter()
            .take(2)
            .filter(|a| a.album_count > 0 && is_solo_artist_name(&a.name))
        {
            if !candidates.iter().any(|c| c.deezer_id == cand.deezer_id) {
                candidates.push(cand.clone());
            }
        }
    }

    if candidates.is_empty() {
        return None;
    }

    // Prioritize exact-case matches first so track-overlap verification inspects
    // them before any differently-cased homonyms.
    candidates.sort_by_key(|a| if a.name.trim() == name { 0u8 } else { 1u8 });

    let played: Vec<String> = played_titles
        .iter()
        .map(|t| slug(t))
        .filter(|t| t.len() >= 3)
        .collect();
    let had_played_hints = !played.is_empty();

    if candidates.len() == 1 && (non_ascii || allow_ci || !had_played_hints) {
        if non_ascii || allow_ci || candidates[0].name.trim() == name {
            return candidates.pop();
        }
        return None;
    }

    // If ASCII and there is a unique exact-case match ("LiSA" vs "LISA") with no
    // played-track hints to cross-check, use it directly.
    let exact_case_count = candidates.iter().filter(|a| a.name.trim() == name).count();
    if !non_ascii && !had_played_hints && exact_case_count == 1 {
        if let Some(i) = candidates.iter().position(|a| a.name.trim() == name) {
            return Some(candidates.swap_remove(i));
        }
    }

    // Check top-track overlap with songs the user actually played/starred.
    if had_played_hints {
        let mut best: Option<(usize, usize)> = None;
        for (i, cand) in candidates.iter().enumerate().take(5) {
            let top = dz.fetch_artist_top_tracks(&cand.deezer_id, 25).await;
            let hits = top
                .iter()
                .map(|t| slug(&t.title))
                .filter(|t| t.len() >= 3)
                .filter(|t| {
                    played
                        .iter()
                        .any(|p| t == p || t.starts_with(p.as_str()) || p.starts_with(t.as_str()))
                })
                .count();
            if hits > 0 && best.map_or(true, |(_, b)| hits > b) {
                best = Some((i, hits));
            }
        }
        if let Some((i, _)) = best {
            return Some(candidates.swap_remove(i));
        }
    }

    pick_artist_fallback(name, exact, top_solo, candidates, had_played_hints)
}

fn pick_artist_fallback(
    name: &str,
    exact: Vec<ArtistSummary>,
    top_solo: Option<ArtistSummary>,
    mut candidates: Vec<ArtistSummary>,
    had_played_hints: bool,
) -> Option<ArtistSummary> {
    if candidates.is_empty() {
        return None;
    }
    if !name.is_ascii() {
        if let Some(top) = top_solo {
            let best_exact_albums = exact.iter().map(|a| a.album_count).max().unwrap_or(0);
            if exact.is_empty()
                || best_exact_albums == 0
                || top.album_count >= best_exact_albums.saturating_mul(3)
            {
                return Some(top);
            }
        }
    }
    // For short single-word ASCII names ("ASU", "Eve", "LiSA", "HACHI"), never
    // blindly fall back to a differently-cased homonym (e.g. "ASU" -> "Asu") or
    // guess among multiple exact-case homonyms when played-track verification
    // found zero matches.
    if name.is_ascii() && !crate::models::allow_case_insensitive_artist_merge(name) {
        let exact_case: Vec<usize> = candidates
            .iter()
            .enumerate()
            .filter(|(_, a)| a.name.trim() == name)
            .map(|(i, _)| i)
            .collect();
        if exact_case.is_empty() {
            return None;
        }
        if exact_case.len() > 1 && had_played_hints {
            return None;
        }
        return Some(candidates.swap_remove(exact_case[0]));
    }
    let exact_case_idx = candidates.iter().position(|a| a.name.trim() == name);
    let first = exact_case_idx.unwrap_or(0);
    Some(candidates.swap_remove(first))
}

/// Builds the new-releases shelf for `/api/web/home`.
/// Returns `(items_json, pending)` where `pending == true` means a cold
/// background sweep is still running and the client can fetch `/api/web/home/new-releases`
/// once it completes.
async fn build_new_releases(
    state: &AppState,
    uid: i64,
    saved: &HashSet<String>,
) -> (Vec<Value>, bool) {
    let stored = stored_new_releases(&state.store, uid, 8);
    if new_releases_is_fresh(&state.store, uid, !stored.is_empty()) {
        let items = stored
            .iter()
            .map(|a| album_json(a, saved.contains(&a.id)))
            .collect();
        return (items, false);
    }

    let mut rx = ensure_new_releases_refresh(state, uid);
    // If we already have valid recent releases persisted, return them right away
    // and let the background sweep refresh silently without blocking Home.
    if !stored.is_empty() {
        let items = stored
            .iter()
            .map(|a| album_json(a, saved.contains(&a.id)))
            .collect();
        return (items, false);
    }

    // Cold shelf: wait very briefly so already-cached lookups land inline,
    // then hand off immediately to `/api/web/home/new-releases` so Home never stalls.
    let _ = tokio::time::timeout(
        std::time::Duration::from_millis(250),
        rx.wait_for(|&done| done),
    )
    .await;
    let fresh = stored_new_releases(&state.store, uid, 8);
    let pending = !*rx.borrow();
    let items = fresh
        .iter()
        .map(|a| album_json(a, saved.contains(&a.id)))
        .collect();
    (items, pending)
}

/// Lightweight follow-up endpoint used by Home when a cold New Releases sweep
/// was still running past the initial Home render budget.
pub async fn home_new_releases(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let uid = user.id;
    let stored = stored_new_releases(&state.store, uid, 8);
    let existing_rx = {
        let inflight = state
            .new_releases_inflight
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        inflight.get(&uid).cloned()
    };
    let mut rx = match existing_rx {
        Some(rx) => Some(rx),
        None if !new_releases_is_fresh(&state.store, uid, !stored.is_empty()) => {
            Some(ensure_new_releases_refresh(&state, uid))
        }
        None => None,
    };
    if let Some(ref mut r) = rx {
        if !*r.borrow() {
            let _ = tokio::time::timeout(
                std::time::Duration::from_secs(8),
                r.wait_for(|&done| done),
            )
            .await;
        }
    }
    let fresh = stored_new_releases(&state.store, uid, 8);
    let saved = state.store.saved_album_ids(uid).unwrap_or_default();
    let pending = rx.as_ref().map_or(false, |r| !*r.borrow());
    let items: Vec<Value> = fresh
        .iter()
        .map(|a| album_json(a, saved.contains(&a.id)))
        .collect();
    json_res(json!({
        "newReleases": items,
        "pending": pending,
    }))
}

/// How long a page waits on upstream catalog APIs before rendering from stored data.
const UPSTREAM_PAGE_WAIT: std::time::Duration = std::time::Duration::from_millis(1500);

/// Run `fut` in its own task and wait at most `UPSTREAM_PAGE_WAIT` for it. On timeout the
/// task keeps running, so its result still lands in the caches/store for the next request.
async fn bounded_upstream<T, F>(fut: F) -> Option<T>
where
    T: Send + 'static,
    F: std::future::Future<Output = T> + Send + 'static,
{
    tokio::time::timeout(UPSTREAM_PAGE_WAIT, tokio::spawn(fut))
        .await
        .ok()
        .and_then(Result::ok)
}

async fn fetch_home_charts(
    state: &AppState,
) -> (Vec<Track>, Vec<AlbumSummary>, Vec<ArtistSummary>) {
    tokio::join!(
        state.sources.deezer.fetch_chart_tracks(20),
        state.sources.deezer.fetch_chart_albums(12),
        state.sources.deezer.fetch_chart_artists(12),
    )
}

async fn try_cached_home_charts(
    state: &AppState,
) -> Option<(Vec<Track>, Vec<AlbumSummary>, Vec<ArtistSummary>)> {
    let (t, al, ar) = tokio::join!(
        state.sources.deezer.cached_chart_tracks(20),
        state.sources.deezer.cached_chart_albums(12),
        state.sources.deezer.cached_chart_artists(12),
    );
    match (t, al, ar) {
        (Some(t), Some(al), Some(ar)) => Some((t, al, ar)),
        _ => None,
    }
}

fn spawn_home_charts_refresh(state: AppState) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static INFLIGHT: AtomicBool = AtomicBool::new(false);
    if INFLIGHT
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    tokio::spawn(async move {
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                INFLIGHT.store(false, Ordering::Release);
            }
        }
        let _reset = Reset;
        let (t, al, ar) = fetch_home_charts(&state).await;
        let st = state.clone();
        let _ = tokio::task::spawn_blocking(move || {
            record_charts(&st, &t, &al, &ar);
        })
        .await;
    });
}

async fn fetch_explore_charts(
    state: &AppState,
) -> (
    Vec<crate::sources::deezer::DeezerGenre>,
    Vec<Track>,
    Vec<AlbumSummary>,
    Vec<ArtistSummary>,
) {
    tokio::join!(
        state.sources.deezer.fetch_genres(),
        state.sources.deezer.fetch_chart_tracks(30),
        state.sources.deezer.fetch_chart_albums(18),
        state.sources.deezer.fetch_chart_artists(14),
    )
}

/// Persist chart results so pages can render them when Deezer is slow or unreachable.
fn record_charts(
    state: &AppState,
    tracks: &[Track],
    albums: &[AlbumSummary],
    artists: &[ArtistSummary],
) {
    if !tracks.is_empty() {
        let _ = state.store.upsert_tracks(tracks);
        let _ = state.store.record_chart_tracks(tracks, 1);
    }
    for a in albums {
        let _ = state.store.upsert_album_meta(a);
    }
    state.store.record_chart_albums(albums);
    for ar in artists {
        let _ = state.store.upsert_artist_meta(ar);
    }
    state.store.record_chart_artists(artists);
}

/// Warm the Deezer cache and stored charts at startup so the first visitor after a
/// machine restart doesn't pay for the upstream round trips.
pub async fn prewarm_charts(state: AppState) {
    let (t, al, ar) = fetch_home_charts(&state).await;
    record_charts(&state, &t, &al, &ar);
    let (_, t, al, ar) = fetch_explore_charts(&state).await;
    record_charts(&state, &t, &al, &ar);
}

pub async fn home(
    State(state): State<AppState>,
    Query(q): Query<HomeQuery>,
    headers: HeaderMap,
) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let uid = user.id;
    if let Some(tz) = q.tz {
        state.taste.set_user_tz_offset(uid, tz);
    }
    if let Some(ref curator) = state.llm {
        curator.maybe_spawn_refresh(&state.taste, uid);
    }

    // Fast path: serve external charts directly from Deezer's in-memory stale-while-revalidate
    // cache without writing to SQLite. If the in-memory cache is cold, prefer persisted SQLite
    // charts immediately and refresh in the background; only block on upstream for a brand-new DB.
    let (chart_tracks, chart_albums, chart_artists) = if let Some(cached) =
        try_cached_home_charts(&state).await
    {
        cached
    } else {
        let has_stored_charts = !state
            .store
            .popular_chart_tracks(1)
            .unwrap_or_default()
            .is_empty();
        if has_stored_charts {
            spawn_home_charts_refresh(state.clone());
            (Vec::new(), Vec::new(), Vec::new())
        } else {
            let st = state.clone();
            bounded_upstream(async move {
                let (t, al, ar) = fetch_home_charts(&st).await;
                record_charts(&st, &t, &al, &ar);
                (t, al, ar)
            })
            .await
            .unwrap_or_default()
        }
    };

    // Ensure auto-mixes exist; returning listeners get instant background refreshes.
    if state.store.count_tracks().unwrap_or(0) > 0 {
        // Mixed-for-you first: rebuilding generic auto mixes clears the dirty flag.
        state.taste.refresh_mixed_for_you_if_stale_async(uid).await;
        state.taste.refresh_auto_playlists_if_stale_async(uid, 900).await;
    }

    // Run all synchronous SQLite & Transformer scoring on a blocking thread with a single
    // shared ScoringContext, candidate pool, and user status lookup maps.
    let st_block = state.clone();
    let built = tokio::task::spawn_blocking(move || {
        let llm_exp = crate::llm::LlmCurator::load_experience(&st_block.store, uid);
        let is_fresh = !st_block.store.has_listening_history(uid).unwrap_or(false);
        let recs = st_block.taste.build_home_recommendations(uid);
        let playlists = st_block.store.list_playlists(uid, false).unwrap_or_default();

        let mixed_for_you = st_block.taste.mixed_for_you_cards_with_data(
            uid,
            &playlists,
            llm_exp.as_ref(),
            &recs.hidden_ids,
            &recs.hidden_keys,
        );
        let auto_mixes: Vec<Value> = playlists
            .iter()
            .filter(|p| p.is_auto && !p.kind.starts_with("mixed_for_you:"))
            .map(|p| {
                playlist_summary_with_hidden_json(
                    &st_block,
                    uid,
                    p,
                    &recs.hidden_ids,
                    &recs.hidden_keys,
                )
            })
            .collect();

        let recent: Vec<Track> = filter_tracks_with_hidden(
            st_block
                .store
                .user_recent_tracks(uid, 12, 0)
                .unwrap_or_default()
                .into_iter()
                .map(|(t, _)| t)
                .collect(),
            &recs.hidden_ids,
            &recs.hidden_keys,
        );

        let trending_tracks = filter_tracks_with_hidden(
            if !chart_tracks.is_empty() {
                chart_tracks
            } else {
                let pop = st_block.store.popular_chart_tracks(16).unwrap_or_default();
                if !pop.is_empty() || is_fresh {
                    pop
                } else {
                    st_block.store.newest_tracks(16, 0).unwrap_or_default()
                }
            },
            &recs.hidden_ids,
            &recs.hidden_keys,
        );
        let albums = if !chart_albums.is_empty() {
            chart_albums
        } else {
            let pop_albums = st_block.store.popular_chart_albums(12);
            if !pop_albums.is_empty() || is_fresh {
                pop_albums
            } else {
                st_block
                    .store
                    .build_album_summaries(uid)
                    .unwrap_or_default()
                    .into_iter()
                    .take(12)
                    .collect()
            }
        };
        let artists = resolve_featured_artists(&st_block, chart_artists, 12, is_fresh);
        let related_tracks =
            filter_tracks_with_hidden(recs.related_tracks, &recs.hidden_ids, &recs.hidden_keys);
        let moods = build_moods_json(&st_block, uid, llm_exp.as_ref(), Some(5));
        let home_categories = build_llm_categories_json(&st_block, uid, llm_exp.as_ref(), 1);

        let mut home_track_ids: Vec<String> = Vec::new();
        for list in [&recs.quick_picks, &recent, &related_tracks, &trending_tracks] {
            for t in list {
                home_track_ids.push(t.id.clone());
            }
        }
        let home_loudness = st_block
            .store
            .loudness_map_for_ids(&home_track_ids)
            .unwrap_or_default();

        let payload = json!({
            "quickPicks": to_web_tracks_from_maps(&recs.quick_picks, &recs.starred, &recs.stats, &recs.ratings, &home_loudness),
            "recent": to_web_tracks_from_maps(&recent, &recs.starred, &recs.stats, &recs.ratings, &home_loudness),
            "mixedForYou": json!(mixed_for_you),
            "madeForYou": auto_mixes,
            "homeCategories": home_categories,
            "relatedTracks": to_web_tracks_from_maps(&related_tracks, &recs.starred, &recs.stats, &recs.ratings, &home_loudness),
            "recommendedArtists": recs.recommended_artists.iter().map(|a| artist_json(a, recs.followed_artists.contains(&a.id))).collect::<Vec<_>>(),
            "trending": to_web_tracks_from_maps(&trending_tracks, &recs.starred, &recs.stats, &recs.ratings, &home_loudness),
            "albums": albums.iter().map(|a| album_json(a, recs.saved_albums.contains(&a.id))).collect::<Vec<_>>(),
            "artists": artists.iter().map(|a| artist_json(a, recs.followed_artists.contains(&a.id))).collect::<Vec<_>>(),
            "moods": moods,
            "llm": llm_meta_json(llm_exp.as_ref()),
        });
        (payload, recs.saved_albums)
    })
    .await;

    let (mut payload, saved) = match built {
        Ok(res) => res,
        Err(_) => return err_res(StatusCode::INTERNAL_SERVER_ERROR, "Failed to build home feed"),
    };

    let (new_releases_json, new_releases_pending) = build_new_releases(&state, uid, &saved).await;
    if let Some(obj) = payload.as_object_mut() {
        obj.insert("newReleases".to_string(), json!(new_releases_json));
        obj.insert(
            "newReleasesPending".to_string(),
            json!(new_releases_pending),
        );
    }
    json_res(payload)
}

fn llm_meta_json(exp: Option<&crate::llm::DailyLlmExperience>) -> Value {
    let Some(e) = exp else {
        return Value::Null;
    };
    json!({
        "editionTitle": e.edition_title,
        "dateLabel": e.date_label,
        "generatedAt": e.generated_at,
        "homeGreeting": e.home_greeting,
        "homeGreetings": e.home_greetings,
        "exploreTitle": e.explore_title,
        "homeTagline": e.home_tagline,
        "exploreTagline": e.explore_tagline,
        "sections": e.sections,
        "featuredGenres": e.featured_genres,
        "searchPrompts": e.search_prompts,
    })
}

fn build_llm_categories_json(
    state: &AppState,
    uid: i64,
    llm_exp: Option<&crate::llm::DailyLlmExperience>,
    limit: usize,
) -> Vec<Value> {
    llm_exp
        .map(|exp| {
            exp.explore_categories
                .iter()
                .take(limit)
                .map(|cat| {
                    let mixes: Vec<Value> = cat
                        .mixes
                        .iter()
                        .map(|m| {
                            let (track_count, covers) =
                                state.taste.curated_llm_explore_preview(uid, m);
                            json!({
                                "key": m.key,
                                "kind": "llm",
                                "kicker": m.kicker,
                                "title": m.title,
                                "subtitle": m.subtitle,
                                "query": m.query,
                                "c1": m.c1,
                                "c2": m.c2,
                                "trackCount": track_count,
                                "covers": covers,
                            })
                        })
                        .collect();
                    json!({
                        "slug": cat.slug,
                        "kicker": cat.kicker,
                        "title": cat.title,
                        "subtitle": cat.subtitle,
                        "mixes": mixes,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn build_moods_json(
    state: &AppState,
    uid: i64,
    llm_exp: Option<&crate::llm::DailyLlmExperience>,
    limit: Option<usize>,
) -> Vec<Value> {
    // When the LLM designed a bespoke set of custom moods for this listener, use them
    // directly so Home & Explore show fresh personalized vibes instead of static defaults.
    if let Some(exp) = llm_exp {
        if !exp.custom_moods.is_empty() {
            let max_items = limit.unwrap_or(exp.custom_moods.len());
            return exp
                .custom_moods
                .iter()
                .take(max_items)
                .map(|cm| {
                    let (track_count, covers) = state.taste.curated_llm_mood_preview(uid, cm);
                    json!({
                        "slug": cm.slug,
                        "kind": "llm",
                        "label": cm.label,
                        "subtitle": cm.subtitle,
                        "c1": cm.c1,
                        "c2": cm.c2,
                        "trackCount": track_count,
                        "covers": covers,
                    })
                })
                .collect();
        }
    }

    let mut ordered_moods: Vec<(Mood, Option<&crate::llm::LlmMoodOverride>)> = Vec::new();
    let mut seen = HashSet::new();
    if let Some(exp) = llm_exp {
        for ov in &exp.mood_overrides {
            if let Some(m) = Mood::from_slug(&ov.slug) {
                if seen.insert(m) {
                    ordered_moods.push((m, Some(ov)));
                }
            }
        }
    }
    for &m in Mood::ALL.iter() {
        if seen.insert(m) {
            ordered_moods.push((m, None));
        }
    }
    if let Some(max_items) = limit {
        ordered_moods.truncate(max_items);
    }
    ordered_moods
        .into_iter()
        .map(|(m, ov)| {
            let (def_c1, def_c2) = m.gradient();
            let c1 = ov
                .map(|o| o.c1.as_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(def_c1);
            let c2 = ov
                .map(|o| o.c2.as_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(def_c2);
            let (track_count, covers) = state.taste.curated_mood_preview_for_user(uid, m);
            let label = ov
                .map(|o| o.label.as_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| m.label());
            let subtitle = ov
                .map(|o| o.subtitle.as_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| m.subtitle());
            json!({
                "slug": m.slug(),
                "label": label,
                "subtitle": subtitle,
                "c1": c1,
                "c2": c2,
                "trackCount": track_count,
                "covers": covers,
            })
        })
        .collect()
}

pub async fn explore(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let uid = user.id;
    if let Some(ref curator) = state.llm {
        curator.maybe_spawn_refresh(&state.taste, uid);
    }
    let llm_exp = crate::llm::LlmCurator::load_experience(&state.store, uid);
    let is_fresh = !state.store.has_listening_history(uid).unwrap_or(false);
    let st = state.clone();
    let (dz_genres, chart_tracks, chart_albums, chart_artists) = bounded_upstream(async move {
        let (g, t, al, ar) = fetch_explore_charts(&st).await;
        record_charts(&st, &t, &al, &ar);
        (g, t, al, ar)
    })
    .await
    .unwrap_or_default();

    let moods = build_moods_json(&state, uid, llm_exp.as_ref(), None);

    let custom_categories = build_llm_categories_json(&state, uid, llm_exp.as_ref(), 4);

    let mut decades: Vec<Value> = Vec::new();
    if let Some(exp) = llm_exp.as_ref().filter(|e| !e.era_mixes.is_empty()) {
        for m in &exp.era_mixes {
            let (track_count, covers) = state.taste.curated_llm_explore_preview(uid, m);
            decades.push(json!({
                "key": m.key,
                "kind": "llm",
                "kicker": m.kicker,
                "title": m.title,
                "subtitle": m.subtitle,
                "query": m.query,
                "c1": m.c1,
                "c2": m.c2,
                "trackCount": track_count,
                "covers": covers,
            }));
        }
    } else {
        for spec in crate::taste::EXPLORE_MIX_SPECS {
            if spec.kind != "decade" {
                continue;
            }
            let (track_count, covers) = state.taste.curated_explore_preview_for_user(uid, spec);
            decades.push(json!({
                "key": spec.key,
                "kind": spec.kind,
                "title": spec.title,
                "subtitle": spec.subtitle,
                "query": spec.query,
                "c1": spec.c1,
                "c2": spec.c2,
                "trackCount": track_count,
                "covers": covers,
            }));
        }
    }

    let genres: Vec<Value> = dz_genres
        .into_iter()
        .filter(|g| !g.name.eq_ignore_ascii_case("all"))
        .map(|g| {
            json!({
                "id": g.id,
                "name": g.name,
                "coverUrl": g.picture_url,
            })
        })
        .collect();

    let discovery_raw = {
        let mut d = state.taste.discover_weekly(uid, 12);
        if d.len() < 6 {
            let extra = state.taste.for_you_tracks(uid, 12);
            for t in extra {
                if !d.iter().any(|x| x.id == t.id) {
                    d.push(t);
                }
            }
        }
        d.truncate(12);
        d
    };
    let discovery_picks = filter_visible_tracks(&state, uid, discovery_raw);

    let albums = if !chart_albums.is_empty() {
        chart_albums
    } else {
        let pop_albums = state.store.popular_chart_albums(18);
        if !pop_albums.is_empty() || is_fresh {
            pop_albums
        } else {
            state
                .store
                .build_album_summaries(uid)
                .unwrap_or_default()
                .into_iter()
                .take(18)
                .collect()
        }
    };
    let artists = resolve_featured_artists(&state, chart_artists, 14, is_fresh);

    let saved = state.store.saved_album_ids(uid).unwrap_or_default();
    let followed = state.store.followed_artist_ids(uid).unwrap_or_default();
    let visible_charts = filter_visible_tracks(
        &state,
        uid,
        if !chart_tracks.is_empty() {
            chart_tracks
        } else {
            let pop = state.store.popular_chart_tracks(30).unwrap_or_default();
            if !pop.is_empty() || is_fresh {
                pop
            } else {
                state.store.newest_tracks(30, 0).unwrap_or_default()
            }
        },
    );

    let related_tracks = filter_visible_tracks(&state, uid, state.taste.related_tracks_for_user(uid, 12));
    let recommended_artists = state.taste.recommended_artists_for_user_cached(uid, 14);

    let ready_mood_cards = moods
        .iter()
        .filter(|m| m["trackCount"].as_u64().unwrap_or(0) > 0)
        .count();
    let mut curator = state.taste.curator_status_snapshot();
    if let Some(obj) = curator.as_object_mut() {
        obj.insert("totalMoods".to_string(), json!(moods.len()));
        obj.insert("readyMoods".to_string(), json!(ready_mood_cards));
    }

    json_res(json!({
        "customCategories": custom_categories,
        "moods": moods,
        "decades": decades,
        "discoveryPicks": to_web_tracks(&state, uid, &discovery_picks),
        "relatedTracks": to_web_tracks(&state, uid, &related_tracks),
        "recommendedArtists": recommended_artists.iter().map(|a| artist_json(a, followed.contains(&a.id))).collect::<Vec<_>>(),
        "artists": artists.iter().map(|a| artist_json(a, followed.contains(&a.id))).collect::<Vec<_>>(),
        "genres": genres,
        "charts": to_web_tracks(&state, uid, &visible_charts),
        "newAlbums": albums.iter().map(|a| album_json(a, saved.contains(&a.id))).collect::<Vec<_>>(),
        "curator": curator,
        "llm": llm_meta_json(llm_exp.as_ref()),
    }))
}

fn resolve_featured_artists(
    state: &AppState,
    chart_artists: Vec<ArtistSummary>,
    limit: usize,
    is_fresh: bool,
) -> Vec<ArtistSummary> {
    if !chart_artists.is_empty() {
        state.store.record_chart_artists(&chart_artists);
        return chart_artists.into_iter().take(limit).collect();
    }

    let cached = state.store.popular_chart_artists(limit);
    if !cached.is_empty() || is_fresh {
        return cached;
    }

    state
        .store
        .featured_artist_summaries(limit)
        .unwrap_or_default()
}

pub async fn explore_mix(
    State(state): State<AppState>,
    Path(key): Path<String>,
    headers: HeaderMap,
) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    if let Some(spec) = crate::taste::find_explore_mix_spec(&key) {
        let raw_tracks = state
            .taste
            .get_curated_explore_mix_tracks(user.id, spec, 35)
            .await;
        let tracks = filter_visible_tracks(&state, user.id, raw_tracks);
        let total_duration: i32 = tracks.iter().map(|t| t.duration.max(0)).sum();
        let match_ids = personal_match_ids(&state, user.id);
        return json_res(json!({
            "mix": {
                "key": spec.key,
                "kind": spec.kind,
                "kicker": "ERA MIX",
                "title": spec.title,
                "subtitle": spec.subtitle,
                "query": spec.query,
                "c1": spec.c1,
                "c2": spec.c2,
                "totalDuration": total_duration,
            },
            "tracks": to_web_tracks_with_taste(&state, user.id, &tracks, &match_ids),
        }));
    }

    if let Some(exp) = crate::llm::LlmCurator::load_experience(&state.store, user.id) {
        if let Some(llm_spec) = exp.find_explore_mix(&key) {
            let raw_tracks = state
                .taste
                .get_llm_explore_mix_tracks(user.id, llm_spec, 35)
                .await;
            let tracks = filter_visible_tracks(&state, user.id, raw_tracks);
            let total_duration: i32 = tracks.iter().map(|t| t.duration.max(0)).sum();
            let match_ids = personal_match_ids(&state, user.id);
            return json_res(json!({
                "mix": {
                    "key": llm_spec.key,
                    "kind": "llm",
                    "kicker": llm_spec.kicker,
                    "title": llm_spec.title,
                    "subtitle": llm_spec.subtitle,
                    "query": llm_spec.query,
                    "c1": llm_spec.c1,
                    "c2": llm_spec.c2,
                    "totalDuration": total_duration,
                },
                "tracks": to_web_tracks_with_taste(&state, user.id, &tracks, &match_ids),
            }));
        }
    }

    err_res(StatusCode::NOT_FOUND, "Explore mix not found")
}

pub async fn mood_mix(
    State(state): State<AppState>,
    Path(slug_str): Path<String>,
    headers: HeaderMap,
) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };

    let llm_exp = crate::llm::LlmCurator::load_experience(&state.store, user.id);
    if let Some(custom) = llm_exp.as_ref().and_then(|e| e.find_custom_mood(&slug_str)) {
        let raw_tracks = state
            .taste
            .get_llm_mood_mix_tracks(user.id, custom, 35)
            .await;
        let tracks = filter_visible_tracks(&state, user.id, raw_tracks);
        let match_ids = personal_match_ids(&state, user.id);
        return json_res(json!({
            "mood": {
                "slug": custom.slug,
                "kind": "llm",
                "label": custom.label,
                "subtitle": custom.subtitle,
                "c1": custom.c1,
                "c2": custom.c2,
            },
            "tracks": to_web_tracks_with_taste(&state, user.id, &tracks, &match_ids),
        }));
    }

    let Some(mood) = Mood::from_slug(&slug_str) else {
        return err_res(StatusCode::NOT_FOUND, "Mood not found");
    };
    let tracks = state.taste.mood_mix_tracks(user.id, mood, 35).await;
    let (def_c1, def_c2) = mood.gradient();
    let llm_ov = llm_exp.and_then(|exp| {
        exp.mood_overrides
            .into_iter()
            .find(|o| o.slug == mood.slug())
    });
    let label = llm_ov
        .as_ref()
        .map(|o| o.label.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| mood.label());
    let subtitle = llm_ov
        .as_ref()
        .map(|o| o.subtitle.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| mood.subtitle());
    let c1 = llm_ov
        .as_ref()
        .map(|o| o.c1.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(def_c1);
    let c2 = llm_ov
        .as_ref()
        .map(|o| o.c2.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(def_c2);
    let match_ids = personal_match_ids(&state, user.id);
    json_res(json!({
        "mood": {
            "slug": mood.slug(),
            "label": label,
            "subtitle": subtitle,
            "c1": c1,
            "c2": c2,
        },
        "tracks": to_web_tracks_with_taste(&state, user.id, &tracks, &match_ids),
    }))
}

pub async fn genre_detail(
    State(state): State<AppState>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let decoded = urlencoding::decode(&name)
        .map(|s| s.into_owned())
        .unwrap_or(name);
    let mut tracks = state.store.tracks_by_genre(&decoded, 35, 0).unwrap_or_default();
    if tracks.len() < 12 {
        let remote = state.sources.deezer.search_tracks(&decoded, 25).await;
        if !remote.is_empty() {
            let _ = state.store.upsert_tracks(&remote);
            tracks.extend(remote);
        }
    }
    let artists = state.sources.deezer.search_artists(&decoded, 10).await;
    for ar in &artists {
        let _ = state.store.upsert_artist_meta(ar);
    }
    let followed = state.store.followed_artist_ids(user.id).unwrap_or_default();
    let visible_tracks = filter_visible_tracks(&state, user.id, tracks);
    json_res(json!({
        "genre": decoded,
        "tracks": to_web_tracks(&state, user.id, &visible_tracks),
        "artists": artists.iter().map(|a| artist_json(a, followed.contains(&a.id))).collect::<Vec<_>>(),
    }))
}

pub async fn search(
    State(state): State<AppState>,
    Query(q): Query<SearchQuery>,
    headers: HeaderMap,
) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let query = q.q.unwrap_or_default().trim().to_string();
    if query.is_empty() {
        return json_res(json!({
            "tracks": [],
            "albums": [],
            "artists": [],
            "instantMix": Value::Null,
            "refinements": Value::Null,
        }));
    }

    let phase = q
        .phase
        .as_deref()
        .map(str::trim)
        .map(str::to_lowercase);

    let intent = crate::taste::parse_search_intent(&query);
    let base_query = intent
        .entity_query
        .as_deref()
        .unwrap_or(&query)
        .to_string();

    if phase.as_deref() == Some("slow") {
        let slow_tracks = state.sources.search_slow(&base_query, 20).await;
        if !slow_tracks.is_empty() {
            let store = state.store.clone();
            let to_save = slow_tracks.clone();
            // Persist before responding so immediate playback or lossless hot-swap
            // requests for newly discovered Soulseek/Archive tracks never race 404.
            let _ = tokio::task::spawn_blocking(move || {
                let _ = store.upsert_tracks(&to_save);
            })
            .await;
        }
        return json_res(json!({
            "query": query,
            "tracks": to_web_tracks(&state, user.id, &slow_tracks),
        }));
    }

    let fast_only = phase.as_deref() == Some("fast");

    let raw_radio_fut = async {
        if intent.has_trailing_radio_ambiguity && !base_query.eq_ignore_ascii_case(&query) {
            if fast_only {
                state.sources.search_fast_with_ytm_hints(&query, 10).await
            } else {
                state.sources.search_all_with_ytm_hints(&query, 10).await
            }
        } else {
            (Vec::new(), std::collections::HashMap::new())
        }
    };

    let ((all_tracks, _ytm_hints), (dz_albums, dz_artists), (raw_tracks_extra, _raw_ytm_hints)) =
        tokio::join!(
            async {
                if fast_only {
                    state.sources.search_fast_with_ytm_hints(&base_query, 20).await
                } else {
                    state.sources.search_all_with_ytm_hints(&base_query, 20).await
                }
            },
            state.sources.search_deezer_catalog(&base_query, 12),
            raw_radio_fut,
        );

    let mut combined = [all_tracks, raw_tracks_extra].concat();
    crate::sources::disambiguate_batch_tracks(&mut combined, &dz_artists);
    let _ = state.store.hydrate_track_metadata(&mut combined);
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

    if !combined.is_empty() || !dz_albums.is_empty() || !dz_artists.is_empty() {
        let store = state.store.clone();
        let dz = state.sources.deezer.clone();
        let mut save_tracks = combined.clone();
        let save_albums = dz_albums.clone();
        let save_artists = dz_artists.clone();
        tokio::spawn(async move {
            let store_first = store.clone();
            let initial_tracks = save_tracks.clone();
            let _ = tokio::task::spawn_blocking(move || {
                if !initial_tracks.is_empty() {
                    let _ = store_first.upsert_tracks(&initial_tracks);
                }
                for a in &save_albums {
                    let _ = store_first.upsert_album_meta(a);
                }
                for ar in &save_artists {
                    let _ = store_first.upsert_artist_meta(ar);
                }
            })
            .await;
            if save_tracks.iter().any(|t| t.genre.trim().is_empty() || t.year <= 0) {
                dz.enrich_track_genres(&mut save_tracks).await;
                let _ = tokio::task::spawn_blocking(move || {
                    let _ = store.upsert_tracks(&save_tracks);
                })
                .await;
            }
        });
    }

    build_search_response_with_options(
        state,
        user.id,
        query,
        base_query,
        combined,
        Vec::new(),
        crate::taste::SearchMixContext::default(),
        dz_albums,
        dz_artists,
        false,
    )
    .await
}

pub async fn instant_mix(
    State(state): State<AppState>,
    Query(q): Query<SearchQuery>,
    headers: HeaderMap,
) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let query = q.q.unwrap_or_default().trim().to_string();
    let explicit_standard = matches!(
        q.mode.as_deref().map(str::trim).map(str::to_lowercase).as_deref(),
        Some("standard") | Some("classic") | Some("noai")
    );
    let mode_override = match q.mode.as_deref().map(str::trim).map(str::to_lowercase).as_deref() {
        Some("topic") | Some("genre") | Some("vibe") => Some("topic".to_string()),
        Some("entity") | Some("artist") | Some("song") => Some("entity".to_string()),
        _ => None,
    };
    if query.is_empty() {
        return json_res(json!({
            "query": "",
            "instantMix": Value::Null,
            "refinements": Value::Null,
        }));
    }

    if !explicit_standard {
        if let Some(ref curator) = state.llm {
            if let Some(resp) = try_build_llm_instant_mix(&state, curator, user.id, &query).await {
                return resp;
            }
        }
    }

    let intent = crate::taste::parse_search_intent(&query);
    let base_query = intent
        .entity_query
        .as_deref()
        .unwrap_or(&query)
        .to_string();
    let force_topic = mode_override.as_deref() == Some("topic");
    let force_entity = mode_override.as_deref() == Some("entity");

    let mix_future = async {
        if force_entity {
            return (Vec::new(), Vec::new());
        }
        if !intent.is_theme_mix && !force_topic {
            // Fetch curated playlists in parallel for evidence-based topic vs entity routing.
            let curated = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                state.sources.fetch_curated_mix_tracks(&base_query, 2, 24),
            )
            .await
            .unwrap_or_default();
            return (curated, Vec::new());
        }
        fetch_topic_mix_inputs(&state, &intent, &base_query).await
    };

    let raw_radio_fut = async {
        if intent.has_trailing_radio_ambiguity && !base_query.eq_ignore_ascii_case(&query) {
            state.sources.search_all_with_ytm_hints(&query, 10).await
        } else {
            (Vec::new(), std::collections::HashMap::new())
        }
    };

    let (((all_tracks, mut ytm_hints), dz_artists, (raw_tracks_extra, raw_ytm_hints)), (mut curated_batches, extra_tracks)) =
        tokio::join!(
            async {
                tokio::join!(
                    state.sources.search_all_with_ytm_hints(&base_query, 20),
                    state.sources.deezer.search_artists(&base_query, 12),
                    raw_radio_fut,
                )
            },
            mix_future
        );

    for (k, v) in raw_ytm_hints {
        ytm_hints.entry(k).or_insert(v);
    }

    let mut combined = [all_tracks, raw_tracks_extra, extra_tracks].concat();
    crate::sources::disambiguate_batch_tracks(&mut combined, &dz_artists);

    let mut to_upsert = combined.clone();
    for (_, batch) in &curated_batches {
        to_upsert.extend(batch.iter().cloned());
    }
    crate::sources::disambiguate_batch_tracks(&mut to_upsert, &dz_artists);
    if !to_upsert.is_empty() {
        let _ = state.store.upsert_tracks(&to_upsert);
    }
    for ar in &dz_artists {
        let _ = state.store.upsert_artist_meta(ar);
    }

    if intent.is_theme_mix && curated_batches.is_empty() {
        let pre_curated = state.taste.load_curated_query_tracks(&intent.normalized);
        if pre_curated.len() >= 5 {
            curated_batches.push((query.clone(), pre_curated));
        }
    }

    // Remote (Deezer) search & playlist tracks carry no genre metadata. Without it the
    // genre-clash checks cannot tell an actual J-Rock track from a rap track that happened
    // to live in a playlists whose title matched "jrock". Resolve album genres before any
    // candidate ranking so the mixer can validate tracks, not just playlist titles.
    if intent.is_theme_mix && !intent.genres.is_empty() {
        let dz = &state.sources.deezer;
        dz.enrich_track_genres(&mut combined).await;
        for (_, batch) in curated_batches.iter_mut() {
            dz.enrich_track_genres(batch).await;
        }
        // Persist the resolved genres so artist/genre indexes are useful on later requests.
        let mut enriched: Vec<Track> = combined.clone();
        for (_, batch) in &curated_batches {
            enriched.extend(batch.iter().cloned());
        }
        if !enriched.is_empty() {
            let _ = state.store.upsert_tracks(&enriched);
        }
    }

    let base_slug = crate::models::slug(&base_query);
    let name_matches_artist = !base_slug.is_empty()
        && (dz_artists
            .iter()
            .any(|a| crate::models::slug(&a.name) == base_slug)
            || combined.iter().any(|t| {
                crate::models::slug(&crate::models::primary_artist(&t.artist)) == base_slug
            }));
    // A query can collide with an obscure artist's name (e.g. "anime" is also a hardcore DJ
    // called "Anime"). Playlists titled with the query tell us which meaning people intend:
    // "100% Queen" is full of Queen tracks, "Anime Essentials" contains no tracks by "Anime".
    // When the matching artist is nearly absent from those playlists, treat it as a topic.
    let artist_absent_from_curated = {
        let curated: Vec<&Track> = curated_batches.iter().flat_map(|(_, b)| b.iter()).collect();
        let by_artist = curated
            .iter()
            .filter(|t| crate::models::slug(&crate::models::primary_artist(&t.artist)) == base_slug)
            .count();
        curated.len() >= 10 && (by_artist as f64) < (curated.len() as f64) * 0.1
    };
    let has_exact_artist = name_matches_artist && !artist_absent_from_curated;
    let has_topic_evidence = !curated_batches.is_empty()
        || state.store.has_catalog_genre(&intent.normalized);
    let use_topic_path = force_topic
        || (!force_entity && (intent.is_theme_mix || (!has_exact_artist && has_topic_evidence)));

    let mut mix_ctx = if !use_topic_path {
        let ctx =
            expand_entity_mix_context(&state, user.id, &query, &combined, &dz_artists, &ytm_hints)
                .await;
        if ctx.anchor_artist.is_none() && curated_batches.is_empty() {
            use crate::sources::MusicSource;
            let (ytm_topic, dz_music_curated) = tokio::join!(
                async {
                    tokio::time::timeout(
                        std::time::Duration::from_millis(2500),
                        state.sources.ytm.search(&format!("{} music", query), 15),
                    )
                    .await
                    .unwrap_or_default()
                },
                async {
                    tokio::time::timeout(
                        std::time::Duration::from_millis(4000),
                        state
                            .sources
                            .fetch_curated_mix_tracks(&format!("{} music", query), 3, 24),
                    )
                    .await
                    .unwrap_or_default()
                }
            );
            if !ytm_topic.is_empty() {
                let _ = state.store.upsert_tracks(&ytm_topic);
                combined.extend(ytm_topic);
            }
            if !dz_music_curated.is_empty() {
                for (_, batch) in &dz_music_curated {
                    let _ = state.store.upsert_tracks(batch);
                }
                curated_batches.extend(dz_music_curated);
            }
        }
        ctx
    } else if curated_batches.is_empty() {
        let ytm_radio = fetch_topic_ytm_radio_fallback(&state, &intent, &combined, &ytm_hints).await;
        crate::taste::SearchMixContext {
            ytm_radio,
            ..Default::default()
        }
    } else {
        crate::taste::SearchMixContext::default()
    };
    mix_ctx.mode_override = mode_override.clone();

    let fallback_inputs = if mode_override.is_none() {
        Some((combined.clone(), curated_batches.clone(), mix_ctx.clone()))
    } else {
        None
    };

    let combined_len = combined.len();
    let curated_len = curated_batches.len();
    let radio_len = mix_ctx.ytm_radio.len()
        + mix_ctx.dz_artist_top.len()
        + mix_ctx.dz_artist_radio.len()
        + mix_ctx.dz_related_tracks.len();
    let expand_inputs = (combined.clone(), curated_batches.clone(), mix_ctx.clone());
    let mut refined = run_instant_mix_refine(
        &state,
        user.id,
        &query,
        combined,
        curated_batches,
        mix_ctx,
    )
    .await;

    if refined.instant_mix.is_none() {
        if let Some((mut fb_combined, mut fb_curated, mut fb_ctx)) = fallback_inputs {
            // Auto mode failed on its initial pass; retry with the full Genre & Vibe (topic) pipeline.
            let (topic_curated, topic_extra) =
                fetch_topic_mix_inputs(&state, &intent, &base_query).await;
            let mut topic_upsert = topic_extra.clone();
            for (_, batch) in &topic_curated {
                topic_upsert.extend(batch.iter().cloned());
            }
            if !topic_upsert.is_empty() {
                crate::sources::disambiguate_batch_tracks(&mut topic_upsert, &dz_artists);
                let _ = state.store.upsert_tracks(&topic_upsert);
            }
            if !topic_extra.is_empty() {
                let mut disamb_extra = topic_extra;
                crate::sources::disambiguate_batch_tracks(&mut disamb_extra, &dz_artists);
                fb_combined.extend(disamb_extra);
            }
            fb_curated.extend(topic_curated);
            if fb_curated.is_empty() && fb_ctx.ytm_radio.is_empty() {
                fb_ctx.ytm_radio =
                    fetch_topic_ytm_radio_fallback(&state, &intent, &fb_combined, &ytm_hints).await;
            }
            fb_ctx.mode_override = Some("topic".to_string());
            let fb_combined_len = fb_combined.len();
            let fb_curated_len = fb_curated.len();
            let fb_radio_len = fb_ctx.ytm_radio.len()
                + fb_ctx.dz_artist_top.len()
                + fb_ctx.dz_artist_radio.len()
                + fb_ctx.dz_related_tracks.len();
            refined = run_instant_mix_refine(
                &state,
                user.id,
                &query,
                fb_combined.clone(),
                fb_curated.clone(),
                fb_ctx.clone(),
            )
            .await;
            refined = expand_sparse_theme_mix(
                &state,
                user.id,
                &query,
                &intent,
                refined,
                (fb_combined, fb_curated, fb_ctx),
                &ytm_hints,
            )
            .await;
            return format_instant_mix_response(
                &state,
                user.id,
                &query,
                fb_combined_len,
                fb_curated_len,
                fb_radio_len,
                refined,
            );
        }
    }

    let refined = expand_sparse_theme_mix(
        &state,
        user.id,
        &query,
        &intent,
        refined,
        expand_inputs,
        &ytm_hints,
    )
    .await;

    format_instant_mix_response(
        &state,
        user.id,
        &query,
        combined_len,
        curated_len,
        radio_len,
        refined,
    )
}

async fn try_build_llm_instant_mix(
    state: &AppState,
    curator: &crate::llm::LlmCurator,
    uid: i64,
    query: &str,
) -> Option<Response> {
    let spec = match curator
        .interpret_instant_mix_prompt(&state.store, uid, query)
        .await
    {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(user_id = uid, query = %query, error = %e, "LLM instant mix interpretation fell back to standard pipeline");
            return None;
        }
    };
    let mix = state
        .taste
        .materialize_llm_instant_mix(uid, query, &spec, 30)
        .await?;
    let taste_match_ids = personal_match_ids(state, uid);
    let mut suggestions: Vec<String> = Vec::new();
    for s in spec.seed_queries.iter().chain(spec.genres.iter()) {
        let clean = s.trim();
        if !clean.is_empty()
            && !clean.eq_ignore_ascii_case(query)
            && !suggestions.iter().any(|x| x.eq_ignore_ascii_case(clean))
        {
            suggestions.push(clean.to_string());
            if suggestions.len() >= 4 {
                break;
            }
        }
    }
    let refinements = crate::taste::SearchRefinements {
        detected_intent: "AI Curated Mix".to_string(),
        filter_chips: Vec::new(),
        query_suggestions: suggestions,
    };
    Some(json_res(json!({
        "query": query,
        "instantMix": {
            "title": mix.title,
            "subtitle": mix.subtitle,
            "query": mix.query,
            "isThemeMix": mix.is_theme_mix,
            "prominent": mix.prominent,
            "aiInterpreted": true,
            "totalDuration": mix.total_duration,
            "tracks": to_web_tracks_with_taste(state, uid, &mix.tracks, &taste_match_ids),
        },
        "refinements": refinements,
    })))
}

/// Minimum size a theme mix should reach before we try to grow it.
const SPARSE_THEME_MIX_TRACKS: usize = 18;

/// A theme mix built only from strictly verified tracks can come out thin. Grow
/// it with YTM radio seeded from those verified tracks (never from raw search
/// noise) and re-run composition, which still applies every evidence/clash rule.
async fn expand_sparse_theme_mix(
    state: &AppState,
    uid: i64,
    query: &str,
    intent: &crate::taste::SearchIntent,
    refined: crate::taste::SearchMixResult,
    inputs: (Vec<Track>, Vec<(String, Vec<Track>)>, crate::taste::SearchMixContext),
    ytm_hints: &std::collections::HashMap<String, String>,
) -> crate::taste::SearchMixResult {
    let Some(mix) = refined.instant_mix.as_ref() else {
        return refined;
    };
    if !mix.is_theme_mix || mix.tracks.len() >= SPARSE_THEME_MIX_TRACKS {
        return refined;
    }
    let mut seed_intent = intent.clone();
    seed_intent.is_theme_mix = true;
    let radio = fetch_topic_ytm_radio_fallback(state, &seed_intent, &mix.tracks, ytm_hints).await;
    if radio.is_empty() {
        return refined;
    }
    let before = mix.tracks.len();
    let (combined, curated, mut ctx) = inputs;
    ctx.ytm_radio.extend(radio);
    if ctx.mode_override.is_none() {
        ctx.mode_override = Some("topic".to_string());
    }
    let expanded = run_instant_mix_refine(state, uid, query, combined, curated, ctx).await;
    match expanded.instant_mix.as_ref() {
        Some(m) if m.tracks.len() > before => expanded,
        _ => refined,
    }
}

async fn expand_entity_mix_context(
    state: &AppState,
    uid: i64,
    query: &str,
    combined: &[Track],
    dz_artists: &[ArtistSummary],
    ytm_hints: &std::collections::HashMap<String, String>,
) -> crate::taste::SearchMixContext {
    let taste = state.taste.clone();
    let q_owned = query.to_string();
    let tracks_owned = combined.to_vec();
    let artists_owned = dz_artists.to_vec();
    let plan = tokio::task::spawn_blocking(move || {
        taste.plan_entity_mix_anchor(uid, &q_owned, &tracks_owned, &artists_owned)
    })
    .await
    .ok()
    .flatten();

    let Some(plan) = plan else {
        return crate::taste::SearchMixContext::default();
    };

    let ytm = state.sources.ytm.clone();
    let dz = state.sources.deezer.clone();
    let artist_name = plan.artist_name.clone();
    let artist_slug = crate::models::slug(&crate::models::primary_artist(&artist_name));
    let seed_track = plan.seed_track.clone();
    let ytm_hint_vid = seed_track
        .as_ref()
        .and_then(|s| ytm_hints.get(&s.song_key()).cloned());

    let ytm_branch = async {
        let vid = if let Some(ref seed) = seed_track {
            if seed.source == "ytm" || seed.id.starts_with("ytm:") {
                Some(seed.id.trim_start_matches("ytm:").to_string())
            } else if let Some(hint_vid) = ytm_hint_vid {
                Some(hint_vid)
            } else {
                ytm.find_best_video_for_track(&artist_name, &seed.title, seed.duration)
                    .await
            }
        } else {
            use crate::sources::MusicSource;
            let found = ytm.search(&artist_name, 6).await;
            found
                .into_iter()
                .find(|t| {
                    crate::models::slug(&crate::models::primary_artist(&t.artist)) == artist_slug
                })
                .map(|t| t.id.trim_start_matches("ytm:").to_string())
        };

        let Some(vid) = vid.filter(|v| !v.is_empty()) else {
            return Vec::new();
        };
        let mut radio = ytm.fetch_radio_tracks(&vid, 30).await;
        if radio.len() < 10 {
            if let Some(hop_vid) = radio
                .iter()
                .find(|t| {
                    crate::models::slug(&crate::models::primary_artist(&t.artist)) != artist_slug
                })
                .map(|t| t.id.trim_start_matches("ytm:").to_string())
            {
                let hop_tracks = tokio::time::timeout(
                    std::time::Duration::from_millis(2000),
                    ytm.fetch_radio_tracks(&hop_vid, 20),
                )
                .await
                .unwrap_or_default();
                radio.extend(hop_tracks);
            }
        }
        radio
    };

    let dz_artist_id_opt = plan.deezer_artist_id.clone();
    let dz_branch = async {
        let dz_id = match dz_artist_id_opt {
            Some(id) if !id.is_empty() => Some(id),
            _ => {
                let found = dz.search_artists(&artist_name, 4).await;
                found
                    .iter()
                    .find(|a| a.name == artist_name)
                    .or_else(|| {
                        found
                            .iter()
                            .find(|a| crate::models::slug(&a.name) == artist_slug)
                    })
                    .map(|a| a.deezer_id.clone())
                    .filter(|id| !id.is_empty())
            }
        };
        let Some(dz_id) = dz_id else {
            return (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        };
        let (artist_res, radio_res) = tokio::join!(
            dz.fetch_artist(&dz_id),
            dz.fetch_artist_radio(&dz_id),
        );
        let mut top_tracks = Vec::new();
        let mut related_artists = Vec::new();
        let mut related_tracks = Vec::new();
        if let Some((meta, top, _albums, related)) = artist_res {
            top_tracks = top;
            let hop_targets: Vec<_> = related
                .iter()
                .filter(|r| {
                    !r.deezer_id.is_empty()
                        && crate::models::slug(&r.name) != artist_slug
                })
                .take(5)
                .cloned()
                .collect();
            related_artists.push(meta);
            related_artists.extend(related);
            let rel_futs = hop_targets.into_iter().map(|rel| {
                let dz_c = dz.clone();
                async move { dz_c.fetch_artist_top_tracks(&rel.deezer_id, 6).await }
            });
            let batches = tokio::time::timeout(
                std::time::Duration::from_millis(2200),
                futures::future::join_all(rel_futs),
            )
            .await
            .unwrap_or_default();
            for batch in batches {
                related_tracks.extend(batch);
            }
        }
        (top_tracks, radio_res, related_tracks, related_artists)
    };

    let budget = std::time::Duration::from_secs(5);
    let (ytm_res, dz_res) = tokio::join!(
        tokio::time::timeout(budget, ytm_branch),
        tokio::time::timeout(budget, dz_branch),
    );

    let ytm_radio = ytm_res.unwrap_or_default();
    let (dz_artist_top, dz_artist_radio, dz_related_tracks, related_artists) =
        dz_res.unwrap_or_default();

    let mut to_upsert = Vec::with_capacity(
        ytm_radio.len() + dz_artist_top.len() + dz_artist_radio.len() + dz_related_tracks.len(),
    );
    to_upsert.extend(ytm_radio.iter().cloned());
    to_upsert.extend(dz_artist_top.iter().cloned());
    to_upsert.extend(dz_artist_radio.iter().cloned());
    to_upsert.extend(dz_related_tracks.iter().cloned());
    crate::sources::disambiguate_batch_tracks(&mut to_upsert, &related_artists);
    if !to_upsert.is_empty() {
        let _ = state.store.upsert_tracks(&to_upsert);
    }
    for ar in &related_artists {
        let _ = state.store.upsert_artist_meta(ar);
    }

    let anchor_genres = seed_track
        .as_ref()
        .map(|t| t.genre.clone())
        .filter(|g| !g.trim().is_empty())
        .into_iter()
        .collect();

    crate::taste::SearchMixContext {
        anchor_artist: Some(plan.artist_name),
        anchor_deezer_artist_id: plan.deezer_artist_id,
        anchor_track_key: seed_track.as_ref().map(Track::song_key),
        anchor_genres,
        is_artist_query: plan.is_artist_query,
        mode_override: None,
        ytm_radio,
        dz_artist_top,
        dz_artist_radio,
        dz_related_tracks,
    }
}

#[cfg(test)]
async fn build_search_response(
    state: AppState,
    uid: i64,
    query: String,
    catalog_query: String,
    combined: Vec<Track>,
    curated_batches: Vec<(String, Vec<Track>)>,
    mix_ctx: crate::taste::SearchMixContext,
    dz_albums: Vec<AlbumSummary>,
    dz_artists: Vec<ArtistSummary>,
) -> Response {
    build_search_response_with_options(
        state,
        uid,
        query,
        catalog_query,
        combined,
        curated_batches,
        mix_ctx,
        dz_albums,
        dz_artists,
        true,
    )
    .await
}

async fn build_search_response_with_options(
    state: AppState,
    uid: i64,
    query: String,
    catalog_query: String,
    mut combined: Vec<Track>,
    curated_batches: Vec<(String, Vec<Track>)>,
    mix_ctx: crate::taste::SearchMixContext,
    dz_albums: Vec<AlbumSummary>,
    dz_artists: Vec<ArtistSummary>,
    include_instant_mix: bool,
) -> Response {
    let taste = state.taste.clone();
    let q_clone = query.clone();
    let refined = tokio::task::spawn_blocking(move || {
        taste.search_and_refine_with_context(
            uid,
            &q_clone,
            std::mem::take(&mut combined),
            curated_batches,
            mix_ctx,
        )
    })
    .await
    .unwrap_or_else(|_| state.taste.search_and_refine(uid, &query, vec![], vec![]));

    let saved = state.store.saved_album_ids(uid).unwrap_or_default();
    let followed = state.store.followed_artist_ids(uid).unwrap_or_default();

    let albums = if !dz_albums.is_empty() {
        dz_albums
    } else {
        state
            .store
            .search_albums(uid, &catalog_query, 12, 0)
            .unwrap_or_default()
    };
    let artists = if !dz_artists.is_empty() {
        dz_artists
    } else {
        state
            .store
            .search_artists(&catalog_query, 12, 0)
            .unwrap_or_default()
    };

    let instant_mix_json = if include_instant_mix {
        refined.instant_mix.as_ref().map(|mix| {
            json!({
                "title": mix.title,
                "subtitle": mix.subtitle,
                "query": mix.query,
                "isThemeMix": mix.is_theme_mix,
                "prominent": mix.prominent,
                "totalDuration": mix.total_duration,
                "tracks": to_web_tracks_with_taste(&state, uid, &mix.tracks, &refined.taste_match_ids),
            })
        })
    } else {
        None
    };

    json_res(json!({
        "query": query,
        "tracks": to_web_tracks_with_taste(&state, uid, &refined.tracks, &refined.taste_match_ids),
        "albums": albums.iter().map(|a| album_json(a, saved.contains(&a.id))).collect::<Vec<_>>(),
        "artists": artists.iter().map(|a| artist_json(a, followed.contains(&a.id))).collect::<Vec<_>>(),
        "instantMix": instant_mix_json,
        "refinements": refined.refinements,
    }))
}

async fn fetch_topic_mix_inputs(
    state: &AppState,
    intent: &crate::taste::SearchIntent,
    base_query: &str,
) -> (Vec<(String, Vec<Track>)>, Vec<Track>) {
    let mut fetch_intent = intent.clone();
    fetch_intent.is_theme_mix = true;
    let mut pl_queries = fetch_intent.playlist_search_queries();
    if pl_queries.is_empty() {
        pl_queries.push(base_query.to_string());
    }
    let tr_queries = fetch_intent.expanded_track_queries();
    let genre_hints = fetch_intent.deezer_genre_hints();

    let pl_futs = pl_queries.iter().map(|pq| {
        let sources = state.sources.clone();
        let pq_owned = pq.clone();
        async move {
            tokio::time::timeout(
                std::time::Duration::from_secs(6),
                sources.fetch_curated_mix_tracks(&pq_owned, 3, 40),
            )
            .await
            .unwrap_or_default()
        }
    });
    let dz_extra_futs = tr_queries.iter().map(|tq| {
        let dz = state.sources.deezer.clone();
        let tq_owned = tq.clone();
        async move {
            tokio::time::timeout(
                std::time::Duration::from_secs(5),
                dz.search_tracks(&tq_owned, 18),
            )
            .await
            .unwrap_or_default()
        }
    });
    let ytm_extra_futs = tr_queries.iter().map(|tq| {
        let ytm = state.sources.ytm.clone();
        let tq_owned = tq.clone();
        async move {
            use crate::sources::MusicSource;
            tokio::time::timeout(std::time::Duration::from_secs(5), ytm.search(&tq_owned, 15))
                .await
                .unwrap_or_default()
        }
    });
    let genre_chart_fut = async {
        if genre_hints.is_empty() {
            return Vec::new();
        }
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let dz_genres = state.sources.deezer.fetch_genres().await;
            let matched: Vec<_> = dz_genres
                .into_iter()
                .filter(|g| {
                    let gl = g.name.to_lowercase();
                    genre_hints
                        .iter()
                        .any(|want| crate::taste::contains_term(&gl, want))
                })
                .take(2)
                .collect();
            let chart_futs = matched.into_iter().map(|g| {
                let dz = state.sources.deezer.clone();
                async move { dz.fetch_genre_chart_tracks(g.id, &g.name, 22).await }
            });
            futures::future::join_all(chart_futs)
                .await
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
        })
        .await
        .unwrap_or_default()
    };

    let (pl_batches, dz_extra, ytm_extra, genre_tracks) = tokio::join!(
        futures::future::join_all(pl_futs),
        futures::future::join_all(dz_extra_futs),
        futures::future::join_all(ytm_extra_futs),
        genre_chart_fut,
    );
    let curated: Vec<(String, Vec<Track>)> = pl_batches.into_iter().flatten().collect();
    let extra_tracks: Vec<Track> = dz_extra
        .into_iter()
        .chain(ytm_extra)
        .flatten()
        .chain(genre_tracks)
        .collect();
    (curated, extra_tracks)
}

async fn fetch_topic_ytm_radio_fallback(
    state: &AppState,
    intent: &crate::taste::SearchIntent,
    combined: &[Track],
    ytm_hints: &std::collections::HashMap<String, String>,
) -> Vec<Track> {
    let is_bad_seed = |t: &Track| -> bool {
        let art_low = t.artist.to_lowercase();
        if art_low.is_empty()
            || art_low == "unknown artist"
            || art_low.ends_with(" hits")
            || art_low.contains("hits guys")
            || art_low.contains("super hits")
            || art_low.contains("unforgettable hits")
            || art_low.contains("cover")
            || art_low.contains("karaoke")
            || art_low.contains("dj gotta")
            || art_low.contains("kiggo")
        {
            return true;
        }
        if let Some((min_y, max_y)) = intent.year_range {
            if t.year > 1920 && (t.year < min_y - 2 || t.year > max_y + 2) {
                return true;
            }
        }
        if !intent.genres.is_empty() && crate::taste::has_genre_clash(t, "", &intent.genres) {
            return true;
        }
        false
    };

    let mut ranked_seed_cands: Vec<(i32, String)> = Vec::new();
    for (idx, t) in combined.iter().enumerate() {
        if is_bad_seed(t) {
            continue;
        }
        let vid = if t.source == "ytm" || t.id.starts_with("ytm:") {
            t.id.trim_start_matches("ytm:").to_string()
        } else if let Some(v) = ytm_hints.get(&t.song_key()) {
            v.clone()
        } else {
            continue;
        };
        if vid.is_empty() {
            continue;
        }
        let mut prio = 100 - (idx as i32);
        if let Some((min_y, max_y)) = intent.year_range {
            if t.year >= min_y && t.year <= max_y {
                prio += 200;
            }
        }
        if !intent.genres.is_empty() {
            let g_low = t.genre.to_lowercase();
            let tl = t.title.to_lowercase();
            if intent
                .genres
                .iter()
                .any(|g| crate::taste::genre_matches_text(&g_low, g))
            {
                prio += 200;
            } else {
                if t.source == "ytm" {
                    prio += 40;
                }
                if crate::taste::has_conflicting_scene_genre(
                    &tl,
                    &intent.genres,
                    &intent.normalized,
                ) || intent
                    .genres
                    .iter()
                    .any(|g| crate::taste::genre_matches_text(&tl, g))
                {
                    prio -= 150;
                }
            }
        }
        ranked_seed_cands.push((prio, vid));
    }
    ranked_seed_cands.sort_by(|a, b| b.0.cmp(&a.0));

    let mut seed_vids: Vec<String> = Vec::new();
    for (_, vid) in ranked_seed_cands {
        if !seed_vids.contains(&vid) {
            seed_vids.push(vid);
            if seed_vids.len() >= 3 {
                break;
            }
        }
    }
    if seed_vids.is_empty() {
        return Vec::new();
    }
    let futs = seed_vids.into_iter().map(|vid| {
        let ytm = state.sources.ytm.clone();
        async move {
            tokio::time::timeout(
                std::time::Duration::from_millis(4000),
                ytm.fetch_radio_tracks(&vid, 22),
            )
            .await
            .unwrap_or_default()
        }
    });
    let radio_tracks: Vec<Track> = futures::future::join_all(futs)
        .await
        .into_iter()
        .flatten()
        .collect();
    if !radio_tracks.is_empty() {
        let _ = state.store.upsert_tracks(&radio_tracks);
    }
    radio_tracks
}

async fn run_instant_mix_refine(
    state: &AppState,
    uid: i64,
    query: &str,
    mut combined: Vec<Track>,
    curated_batches: Vec<(String, Vec<Track>)>,
    mix_ctx: crate::taste::SearchMixContext,
) -> crate::taste::SearchMixResult {
    let taste = state.taste.clone();
    let q_clone = query.to_string();
    let q_fallback = query.to_string();
    tokio::task::spawn_blocking(move || {
        taste.search_and_refine_with_context(
            uid,
            &q_clone,
            std::mem::take(&mut combined),
            curated_batches,
            mix_ctx,
        )
    })
    .await
    .unwrap_or_else(|_| state.taste.search_and_refine(uid, &q_fallback, vec![], vec![]))
}

fn format_instant_mix_response(
    state: &AppState,
    uid: i64,
    query: &str,
    combined_len: usize,
    curated_len: usize,
    radio_len: usize,
    refined: crate::taste::SearchMixResult,
) -> Response {
    if let Some(ref mix) = refined.instant_mix {
        if mix.is_theme_mix && curated_len > 0 && mix.tracks.len() >= 8 {
            state
                .taste
                .save_curated_query_tracks(&query.trim().to_lowercase(), &mix.tracks);
        }
    } else {
        tracing::warn!(
            query = %query,
            combined_len,
            curated_len,
            radio_len,
            ranked_len = refined.tracks.len(),
            "instant mix returned empty"
        );
    }

    let instant_mix_json = refined.instant_mix.as_ref().map(|mix| {
        json!({
            "title": mix.title,
            "subtitle": mix.subtitle,
            "query": mix.query,
            "isThemeMix": mix.is_theme_mix,
            "prominent": mix.prominent,
            "totalDuration": mix.total_duration,
            "tracks": to_web_tracks_with_taste(state, uid, &mix.tracks, &refined.taste_match_ids),
        })
    });

    json_res(json!({
        "query": query,
        "instantMix": instant_mix_json,
        "refinements": refined.refinements,
    }))
}

#[cfg(test)]
async fn build_instant_mix_response(
    state: AppState,
    uid: i64,
    query: String,
    combined: Vec<Track>,
    curated_batches: Vec<(String, Vec<Track>)>,
    mix_ctx: crate::taste::SearchMixContext,
) -> Response {
    let combined_len = combined.len();
    let curated_len = curated_batches.len();
    let radio_len = mix_ctx.ytm_radio.len()
        + mix_ctx.dz_artist_top.len()
        + mix_ctx.dz_artist_radio.len()
        + mix_ctx.dz_related_tracks.len();
    let refined =
        run_instant_mix_refine(&state, uid, &query, combined, curated_batches, mix_ctx).await;
    format_instant_mix_response(
        &state,
        uid,
        &query,
        combined_len,
        curated_len,
        radio_len,
        refined,
    )
}

#[derive(Deserialize, Default)]
pub struct EntityHintQuery {
    pub name: Option<String>,
    pub artist: Option<String>,
    /// Title of the song the album was opened from, to locate the album online.
    pub track: Option<String>,
    /// When set (`fast=1`), skip blocking on speculative Soulseek searches so
    /// direct Play clicks start playback immediately.
    #[serde(default)]
    pub fast: Option<String>,
}

pub async fn album_detail(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<EntityHintQuery>,
    headers: HeaderMap,
) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let fast = matches!(
        q.fast.as_deref().map(str::trim),
        Some("1" | "true" | "yes")
    );
    let hydrated = if fast {
        state
            .taste
            .get_or_hydrate_album_fast(
                &id,
                q.name.as_deref(),
                q.artist.as_deref(),
                q.track.as_deref(),
            )
            .await
    } else {
        state
            .taste
            .get_or_hydrate_album_with_hints(
                &id,
                q.name.as_deref(),
                q.artist.as_deref(),
                q.track.as_deref(),
            )
            .await
    };
    let Some((album, tracks)) = hydrated else {
        return err_res(StatusCode::NOT_FOUND, "Album not found");
    };
    if fast && !tracks.is_empty() && !tracks.iter().all(|t| t.is_lossless()) {
        let taste = state.taste.clone();
        let bg_album = album.clone();
        let bg_tracks = tracks.clone();
        tokio::spawn(async move {
            taste
                .upgrade_album_from_soulseek_background(&bg_album, bg_tracks)
                .await;
        });
    }
    let saved = state.store.saved_album_ids(user.id).unwrap_or_default();
    json_res(json!({
        "album": album_json(&album, saved.contains(&album.id)),
        "tracks": to_web_tracks(&state, user.id, &tracks),
    }))
}

pub async fn artist_detail(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<EntityHintQuery>,
    headers: HeaderMap,
) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let Some((artist, top_tracks, albums, related)) = state
        .taste
        .get_or_hydrate_artist_with_hint(&id, user.id, q.name.as_deref())
        .await
    else {
        return err_res(StatusCode::NOT_FOUND, "Artist not found");
    };
    let related_tracks = filter_visible_tracks(
        &state,
        user.id,
        state
            .taste
            .artist_related_tracks(&artist, &related, &top_tracks, 12)
            .await,
    );
    let followed = state.store.followed_artist_ids(user.id).unwrap_or_default();
    let saved = state.store.saved_album_ids(user.id).unwrap_or_default();
    json_res(json!({
        "artist": artist_json(&artist, followed.contains(&artist.id)),
        "topTracks": to_web_tracks(&state, user.id, &top_tracks),
        "albums": albums.iter().map(|a| album_json(a, saved.contains(&a.id))).collect::<Vec<_>>(),
        "related": related.iter().map(|a| artist_json(a, followed.contains(&a.id))).collect::<Vec<_>>(),
        "relatedTracks": to_web_tracks(&state, user.id, &related_tracks),
    }))
}

pub async fn library(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let uid = user.id;
    let playlists = state.store.list_playlists(uid, false).unwrap_or_default();
    let starred_tracks: Vec<Track> = state
        .store
        .starred_tracks(uid)
        .unwrap_or_default()
        .into_iter()
        .map(|(t, _)| t)
        .collect();
    let saved_ids = state.store.saved_album_ids(uid).unwrap_or_default();
    let saved_albums: Vec<Value> = state
        .store
        .build_album_summaries(uid)
        .unwrap_or_default()
        .into_iter()
        .filter(|a| saved_ids.contains(&a.id))
        .map(|a| album_json(&a, true))
        .collect();
    let followed_ids = state.store.followed_artist_ids(uid).unwrap_or_default();
    let followed_artists: Vec<Value> = state
        .store
        .build_artist_summaries()
        .unwrap_or_default()
        .into_iter()
        .filter(|a| followed_ids.contains(&a.id))
        .map(|a| artist_json(&a, true))
        .collect();
    let recent: Vec<Track> = state
        .store
        .user_recent_tracks(uid, 40, 0)
        .unwrap_or_default()
        .into_iter()
        .map(|(t, _)| t)
        .collect();
    let frequent: Vec<Track> = state
        .store
        .user_frequent_tracks(uid, 40, 0)
        .unwrap_or_default()
        .into_iter()
        .map(|(t, _)| t)
        .collect();

    json_res(json!({
        "playlists": playlists
            .iter()
            .map(|p| playlist_summary_for_user_json(&state, uid, p))
            .collect::<Vec<_>>(),
        "likedTracks": to_web_tracks(&state, uid, &starred_tracks),
        "savedAlbums": saved_albums,
        "followedArtists": followed_artists,
        "recentTracks": to_web_tracks(&state, uid, &recent),
        "frequentTracks": to_web_tracks(&state, uid, &frequent),
    }))
}

pub async fn stats(
    State(state): State<AppState>,
    Query(q): Query<StatsQuery>,
    headers: HeaderMap,
) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let uid = user.id;
    let range = crate::stats::StatsRange::from_str(q.range.as_deref());
    let tz = q.tz.unwrap_or(0);
    state.taste.enrich_user_listened_tracks(uid).await;
    let store = state.store.clone();
    let user_clone = user.clone();

    let Ok((computed, starred, track_stats, ratings)) =
        tokio::task::spawn_blocking(move || {
            let now_ts = Utc::now().timestamp();
            let c = crate::stats::build_user_stats(&store, &user_clone, range, tz, now_ts);
            let st = store.starred_ids(uid).unwrap_or_default();
            let ts = store.track_stats_map(uid).unwrap_or_default();
            let rt = store.user_ratings(uid).unwrap_or_default();
            (c, st, ts, rt)
        })
        .await
    else {
        return err_res(StatusCode::INTERNAL_SERVER_ERROR, "Failed to compute stats");
    };

    let llm_exp = crate::llm::LlmCurator::load_experience(&state.store, uid);
    json_res(stats_payload(
        computed,
        &starred,
        &track_stats,
        &ratings,
        true,
        llm_exp.as_ref(),
    ))
}

#[derive(Deserialize)]
pub struct CuratorPrefsReq {
    #[serde(default, rename = "tasteNote")]
    pub taste_note: String,
    #[serde(default, rename = "refreshNow")]
    pub refresh_now: bool,
}

pub async fn curator_prefs(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let enabled = state.llm.is_some();
    let taste_note = crate::llm::LlmCurator::load_taste_note(&state.store, user.id);
    let exp = crate::llm::LlmCurator::load_experience(&state.store, user.id);
    let (in_flight, cooldown_secs) = match state.llm.as_ref() {
        Some(c) => (
            c.is_in_flight(user.id),
            c.user_refresh_cooldown_remaining(&state.store, user.id),
        ),
        None => (false, 0),
    };
    json_res(json!({
        "enabled": enabled,
        "tasteNote": taste_note,
        "maxChars": crate::llm::TASTE_NOTE_MAX_CHARS,
        "hasExperience": exp.is_some(),
        "editionTitle": exp.as_ref().map(|e| e.edition_title.as_str()).unwrap_or(""),
        "dateLabel": exp.as_ref().map(|e| e.date_label.as_str()).unwrap_or(""),
        "generatedAt": exp.as_ref().map(|e| e.generated_at).unwrap_or(0),
        "inFlight": in_flight,
        "cooldownSecs": cooldown_secs,
    }))
}

pub async fn update_curator_prefs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CuratorPrefsReq>,
) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let saved_note = crate::llm::LlmCurator::save_taste_note(&state.store, user.id, &req.taste_note);
    let mut refresh_started = false;
    let mut cooldown_secs = 0i64;
    let mut in_flight = false;
    if let Some(ref curator) = state.llm {
        if req.refresh_now {
            match curator.try_spawn_user_refresh(&state.taste, user.id) {
                Ok(started) => {
                    refresh_started = started;
                    in_flight = true;
                }
                Err(wait) => {
                    cooldown_secs = wait;
                }
            }
        } else {
            in_flight = curator.is_in_flight(user.id);
            cooldown_secs = curator.user_refresh_cooldown_remaining(&state.store, user.id);
        }
    }
    json_res(json!({
        "ok": true,
        "enabled": state.llm.is_some(),
        "tasteNote": saved_note,
        "refreshStarted": refresh_started,
        "inFlight": in_flight,
        "cooldownSecs": cooldown_secs,
    }))
}

/// Full listening stats for another user's profile — the same payload as the
/// personal stats page. Track objects are decorated with the *viewer's* likes
/// and ratings, and the most personal bits (the per-track activity log and
/// the hidden-track count) are withheld unless the viewer is the target.
pub async fn social_stats(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    Query(q): Query<StatsQuery>,
    headers: HeaderMap,
) -> Response {
    let viewer = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let Some(target_id) = resolve_user_ref(&state, &reference) else {
        return err_res(StatusCode::NOT_FOUND, "User not found");
    };
    let Some(target) = state.store.get_public_user(target_id, viewer.id).ok().flatten() else {
        return err_res(StatusCode::NOT_FOUND, "User not found");
    };
    let is_self = target.id == viewer.id;
    let target_user = User {
        id: target.id,
        username: target.username.clone(),
        password: String::new(),
        created: target.created,
    };
    let range = crate::stats::StatsRange::from_str(q.range.as_deref());
    let tz = q.tz.unwrap_or(0);
    state.taste.enrich_user_listened_tracks(target.id).await;
    let store = state.store.clone();
    let vid = viewer.id;

    let Ok((computed, starred, track_stats, ratings)) =
        tokio::task::spawn_blocking(move || {
            let now_ts = Utc::now().timestamp();
            let c = crate::stats::build_user_stats(&store, &target_user, range, tz, now_ts);
            let st = store.starred_ids(vid).unwrap_or_default();
            let ts = store.track_stats_map(vid).unwrap_or_default();
            let rt = store.user_ratings(vid).unwrap_or_default();
            (c, st, ts, rt)
        })
        .await
    else {
        return err_res(StatusCode::INTERNAL_SERVER_ERROR, "Failed to compute stats");
    };

    let llm_exp = crate::llm::LlmCurator::load_experience(&state.store, target.id);
    json_res(stats_payload(
        computed,
        &starred,
        &track_stats,
        &ratings,
        is_self,
        llm_exp.as_ref(),
    ))
}

fn stats_payload(
    mut computed: crate::stats::ComputedStats,
    starred: &HashSet<String>,
    track_stats: &HashMap<String, UserTrackStats>,
    ratings: &HashMap<String, i32>,
    is_self: bool,
    llm_exp: Option<&crate::llm::DailyLlmExperience>,
) -> Value {
    if !is_self {
        computed.recent_events.clear();
        computed.library.hidden_tracks = 0;
    }
    let empty_loudness = HashMap::new();
    let wrap_track = |t: &Track| to_web_track_with_maps(t, starred, track_stats, ratings, &empty_loudness, false);

    let top_tracks: Vec<Value> = computed
        .top_tracks
        .iter()
        .map(|item| {
            json!({
                "track": wrap_track(&item.track),
                "streams": item.streams,
                "completed": item.completed,
                "skips": item.skips,
                "listenedSecs": item.listened_secs,
                "lastPlayed": item.last_played,
            })
        })
        .collect();

    let top_artists: Vec<Value> = computed
        .top_artists
        .iter()
        .map(|a| {
            json!({
                "id": a.id,
                "name": a.name,
                "coverUrl": cover_api_url(&a.id),
                "streams": a.streams,
                "listenedSecs": a.listened_secs,
                "uniqueTracks": a.unique_tracks,
                "pct": a.pct,
                "isFollowed": a.is_followed,
            })
        })
        .collect();

    let top_albums: Vec<Value> = computed
        .top_albums
        .iter()
        .map(|a| {
            json!({
                "id": a.id,
                "name": a.name,
                "artist": a.artist,
                "artistId": a.artist_id,
                "coverUrl": cover_api_url(&a.id),
                "year": a.year,
                "streams": a.streams,
                "listenedSecs": a.listened_secs,
                "uniqueTracks": a.unique_tracks,
                "isSaved": a.is_saved,
            })
        })
        .collect();

    let most_skipped: Vec<Value> = computed
        .most_skipped
        .iter()
        .map(|s| {
            json!({
                "track": wrap_track(&s.track),
                "skips": s.skips,
                "streams": s.streams,
            })
        })
        .collect();

    let highlights = json!({
        "longestTrack": computed.highlights.longest_track.as_ref().map(|t| json!({
            "track": wrap_track(t),
            "duration": t.duration,
        })),
        "shortestTrack": computed.highlights.shortest_track.as_ref().map(|t| json!({
            "track": wrap_track(t),
            "duration": t.duration,
        })),
        "firstTrackEver": computed.highlights.first_track_ever.as_ref().map(|(t, at)| json!({
            "track": wrap_track(t),
            "at": at,
        })),
        "mostCompletedTrack": computed.highlights.most_completed_track.as_ref().map(|(t, completed)| json!({
            "track": wrap_track(t),
            "completed": completed,
        })),
    });

    let recent_events: Vec<Value> = computed
        .recent_events
        .iter()
        .map(|ev| {
            json!({
                "kind": ev.kind,
                "label": ev.label,
                "at": ev.at,
                "value": ev.value,
                "track": wrap_track(&ev.track),
            })
        })
        .collect();

    let taste_portrait = if computed.summary.total_streams > 0 {
        llm_exp
            .filter(|e| !e.taste_portrait.is_empty())
            .map(|e| {
                json!({
                    "title": e.taste_portrait.title,
                    "summary": e.taste_portrait.summary,
                    "traits": e.taste_portrait.traits,
                    "dateLabel": e.date_label,
                    "editionTitle": e.edition_title,
                })
            })
    } else {
        None
    };

    json!({
        "isSelf": is_self,
        "range": computed.range.key(),
        "rangeLabel": computed.range.label(),
        "tzOffsetMin": computed.tz_offset_min,
        "generatedAt": computed.generated_at,
        "memberSince": computed.member_since,
        "firstListenAt": computed.first_listen_at,
        "lastListenAt": computed.last_listen_at,
        "summary": computed.summary,
        "streaks": computed.streaks,
        "library": computed.library,
        "persona": computed.persona,
        "tastePortrait": taste_portrait,
        "timeline": computed.timeline,
        "heatmap": computed.heatmap,
        "hourly": computed.hourly,
        "peakHour": computed.peak_hour,
        "weekdays": computed.weekdays,
        "peakWeekday": computed.peak_weekday,
        "moods": computed.moods,
        "genres": computed.genres,
        "decades": computed.decades,
        "oldestTrack": computed.oldest_track,
        "newestTrack": computed.newest_track,
        "topTracks": top_tracks,
        "topArtists": top_artists,
        "topAlbums": top_albums,
        "mostSkipped": most_skipped,
        "highlights": highlights,
        "recentEvents": recent_events,
    })
}

pub async fn get_playlist(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let Some(pl) = state.store.get_playlist(&id).ok().flatten() else {
        return err_res(StatusCode::NOT_FOUND, "Playlist not found");
    };
    if !pl.public && pl.user_id != user.id {
        return err_res(StatusCode::FORBIDDEN, "Permission denied");
    }
    let pl = if pl.is_auto && pl.user_id == user.id && state.store.count_tracks().unwrap_or(0) > 0 {
        if pl.kind.starts_with("mixed_for_you:") {
            state.taste.refresh_mixed_for_you_if_stale_async(user.id).await;
        }
        state.taste.refresh_auto_playlists_if_stale_async(user.id, 900).await;
        state.store.get_playlist(&id).ok().flatten().unwrap_or(pl)
    } else {
        pl
    };
    let raw_tracks = state.store.playlist_tracks(&id).unwrap_or_default();
    let tracks = if pl.is_auto {
        filter_visible_tracks(&state, user.id, raw_tracks)
    } else {
        raw_tracks
    };
    let mut pl_summary = pl.clone();
    if pl.is_auto {
        pl_summary.track_count = tracks.len();
    }
    json_res(json!({
        "playlist": playlist_summary_json(&pl_summary),
        "isOwner": pl.user_id == user.id,
        "tracks": to_web_tracks(&state, user.id, &tracks),
    }))
}

pub async fn create_playlist(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<PlaylistCreateReq>,
) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let name = if req.name.trim().is_empty() {
        "My Playlist"
    } else {
        req.name.trim()
    };
    match state
        .store
        .create_playlist(user.id, name, &req.comment, req.public, &req.track_ids)
    {
        Ok(pl) => {
            if !req.track_ids.is_empty() {
                state.taste.record_playlist_additions(user.id, &req.track_ids);
            }
            json_res(json!({ "playlist": playlist_summary_json(&pl) }))
        }
        Err(e) => err_res(StatusCode::BAD_REQUEST, &e.to_string()),
    }
}

pub async fn playlist_recommendations(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let Some(pl) = state.store.get_playlist(&id).ok().flatten() else {
        return err_res(StatusCode::NOT_FOUND, "Playlist not found");
    };
    if !pl.public && pl.user_id != user.id {
        return err_res(StatusCode::FORBIDDEN, "Permission denied");
    }
    let tracks = state.taste.recommend_for_playlist(user.id, &id, 15).await;
    json_res(json!({
        "playlistId": id,
        "tracks": to_web_tracks(&state, user.id, &tracks),
    }))
}

pub async fn update_playlist(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<PlaylistUpdateReq>,
) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let Some(pl) = state.store.get_playlist(&id).ok().flatten() else {
        return err_res(StatusCode::NOT_FOUND, "Playlist not found");
    };
    if pl.user_id != user.id {
        return err_res(StatusCode::FORBIDDEN, "Permission denied");
    }
    let _ = state.store.update_playlist(
        &id,
        req.name.as_deref(),
        req.comment.as_deref(),
        req.public,
    );
    let Some(updated) = state.store.get_playlist(&id).ok().flatten() else {
        return err_res(StatusCode::NOT_FOUND, "Playlist not found");
    };
    json_res(json!({ "playlist": playlist_summary_json(&updated) }))
}

pub async fn delete_playlist(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let Some(pl) = state.store.get_playlist(&id).ok().flatten() else {
        return err_res(StatusCode::NOT_FOUND, "Playlist not found");
    };
    if pl.user_id != user.id {
        return err_res(StatusCode::FORBIDDEN, "Permission denied");
    }
    let _ = state.store.delete_playlist(&id);
    if !pl.is_auto {
        state.taste.mark_taste_dirty(user.id);
    }
    json_res(json!({ "ok": true }))
}

pub async fn add_playlist_tracks(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<PlaylistAddReq>,
) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let Some(pl) = state.store.get_playlist(&id).ok().flatten() else {
        return err_res(StatusCode::NOT_FOUND, "Playlist not found");
    };
    if pl.user_id != user.id {
        return err_res(StatusCode::FORBIDDEN, "Permission denied");
    }
    let added = state
        .store
        .add_unique_tracks_to_playlist(&id, &req.track_ids)
        .unwrap_or_default();
    if !pl.is_auto && !added.is_empty() {
        state.taste.record_playlist_additions(user.id, &added);
    }
    json_res(json!({ "ok": true, "added": added.len() }))
}

pub async fn remove_playlist_track(
    State(state): State<AppState>,
    Path((id, pos)): Path<(String, usize)>,
    headers: HeaderMap,
) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let Some(pl) = state.store.get_playlist(&id).ok().flatten() else {
        return err_res(StatusCode::NOT_FOUND, "Playlist not found");
    };
    if pl.user_id != user.id {
        return err_res(StatusCode::FORBIDDEN, "Permission denied");
    }
    let current = state.store.playlist_tracks(&id).unwrap_or_default();
    let Some(removed_id) = current.get(pos).map(|t| t.id.clone()) else {
        return err_res(StatusCode::BAD_REQUEST, "Track position out of range");
    };
    if let Err(e) = state.store.remove_playlist_positions(&id, &[pos]) {
        return err_res(StatusCode::BAD_REQUEST, &e.to_string());
    }
    if !pl.is_auto {
        state.taste.record_playlist_removals(user.id, &[removed_id]);
    }
    json_res(json!({ "ok": true }))
}

pub async fn reorder_playlist_track(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<PlaylistReorderReq>,
) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let Some(pl) = state.store.get_playlist(&id).ok().flatten() else {
        return err_res(StatusCode::NOT_FOUND, "Playlist not found");
    };
    if pl.user_id != user.id {
        return err_res(StatusCode::FORBIDDEN, "Permission denied");
    }
    match state.store.move_playlist_track(&id, req.from, req.to) {
        Ok(()) => json_res(json!({ "ok": true })),
        Err(e) => err_res(StatusCode::BAD_REQUEST, &e.to_string()),
    }
}

pub async fn toggle_star(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<StarReq>,
) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    match req.kind.as_str() {
        "album" => {
            if req.starred {
                let _ = state.store.save_album(user.id, &req.id);
            } else {
                let _ = state.store.unsave_album(user.id, &req.id);
            }
            state.taste.mark_taste_dirty(user.id);
        }
        "artist" => {
            if req.starred {
                let _ = state.store.follow_artist(user.id, &req.id);
            } else {
                let _ = state.store.unfollow_artist(user.id, &req.id);
            }
            state
                .store
                .kv_set(&format!("newrel:fetched:{}", user.id), "");
            state.taste.mark_taste_dirty(user.id);
        }
        _ => {
            if req.starred {
                let _ = state.store.star_track(user.id, &req.id);
                state.taste.learn_signal(user.id, &req.id, "star", 1.0);
            } else {
                let _ = state.store.unstar_track(user.id, &req.id);
                state.taste.learn_signal(user.id, &req.id, "unstar", 0.0);
            }
        }
    }
    json_res(json!({ "ok": true, "starred": req.starred }))
}

pub async fn set_rating(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<RateReq>,
) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let rating = req.rating.clamp(0, 5);
    let _ = state.store.set_rating(user.id, &req.id, rating);
    if rating > 0 {
        state
            .taste
            .learn_signal(user.id, &req.id, "rate", rating as f64);
    } else {
        state.taste.mark_taste_dirty(user.id);
    }
    json_res(json!({ "ok": true, "rating": rating }))
}

pub async fn record_event(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<EventReq>,
) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    state
        .taste
        .record_event(user.id, &req.track_id, &req.kind, req.value);
    let mut play_patch: Option<Value> = None;
    if let Ok(Some(mut t)) = state.store.get_track(&req.track_id) {
        let missing_meta = t.genre.trim().is_empty() || t.year <= 0;
        if missing_meta {
            let _ = state
                .store
                .hydrate_track_metadata(std::slice::from_mut(&mut t));
        }
        if req.kind == "play" {
            let now = Utc::now().timestamp();
            let client_id = normalize_now_playing_client_id(&req.client_id);
            // Record now-playing immediately before awaiting any metadata enrichment so a
            // quick pause or window close cannot race ahead of a slow network call.
            upsert_now_playing(
                &state,
                user.id,
                t.clone(),
                now,
                client_id.clone(),
                req.seq,
            );
            if t.year <= 0 {
                state
                    .sources
                    .deezer
                    .enrich_track_genres_with_timeout(
                        std::slice::from_mut(&mut t),
                        std::time::Duration::from_millis(1800),
                    )
                    .await;
            }
            if missing_meta && (t.year > 0 || !t.genre.trim().is_empty()) {
                let store = state.store.clone();
                let to_save = vec![t.clone()];
                let _ = tokio::task::spawn_blocking(move || {
                    let _ = store.upsert_tracks(&to_save);
                })
                .await;
                // Refresh metadata on the live entry only if this same track is still playing.
                let mut np = state.now_playing.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(entry) = np.get_mut(&user.id) {
                    if entry.track.as_ref().map(|cur| cur.id.as_str()) == Some(t.id.as_str()) {
                        entry.track = Some(t.clone());
                    }
                }
            } else if t.genre.trim().is_empty() || t.year <= 0 {
                let dz = state.sources.deezer.clone();
                let store = state.store.clone();
                let mut batch = vec![t.clone()];
                tokio::spawn(async move {
                    dz.enrich_track_genres(&mut batch).await;
                    let _ = tokio::task::spawn_blocking(move || {
                        let _ = store.upsert_tracks(&batch);
                    })
                    .await;
                });
            }
            if t.year > 0 {
                play_patch = Some(json!({
                    "id": t.id,
                    "year": t.year,
                    "genre": t.genre,
                }));
            }
        } else if t.genre.trim().is_empty() || t.year <= 0 {
            let dz = state.sources.deezer.clone();
            let store = state.store.clone();
            let mut batch = vec![t.clone()];
            tokio::spawn(async move {
                dz.enrich_track_genres(&mut batch).await;
                let _ = tokio::task::spawn_blocking(move || {
                    let _ = store.upsert_tracks(&batch);
                })
                .await;
            });
        }
    }
    if let Some(track) = play_patch {
        json_res(json!({ "ok": true, "track": track }))
    } else {
        json_res(json!({ "ok": true }))
    }
}

pub async fn mark_not_interested(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<NotInterestedReq>,
) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let tid = req.track_id.trim();
    if tid.is_empty() {
        return err_res(StatusCode::BAD_REQUEST, "Missing trackId");
    }
    state.taste.mark_not_interested(user.id, tid);
    let count = state
        .store
        .hidden_tracks_list(user.id)
        .map(|v| v.len())
        .unwrap_or(0);
    json_res(json!({ "ok": true, "hiddenCount": count }))
}

pub async fn list_hidden(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let items = state.store.hidden_tracks_list(user.id).unwrap_or_default();
    let tracks: Vec<Value> = items
        .into_iter()
        .map(|(t, at)| {
            json!({
                "id": t.id,
                "title": t.title,
                "artist": t.artist,
                "album": t.album,
                "coverUrl": cover_api_url(&t.id),
                "hiddenAt": at,
            })
        })
        .collect();
    json_res(json!({
        "count": tracks.len(),
        "tracks": tracks,
    }))
}

pub async fn unhide_track(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let decoded = urlencoding::decode(&id)
        .map(|s| s.into_owned())
        .unwrap_or(id);
    let removed = state.taste.unhide_track(user.id, &decoded);
    let taste = state.taste.clone();
    let uid = user.id;
    let _ = tokio::task::spawn_blocking(move || taste.rebuild_auto_playlists(uid)).await;
    let count = state
        .store
        .hidden_tracks_list(user.id)
        .map(|v| v.len())
        .unwrap_or(0);
    json_res(json!({ "ok": true, "removed": removed, "count": count }))
}

pub async fn reset_hidden(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let cleared = state.taste.reset_hidden_tracks(user.id);
    let taste = state.taste.clone();
    let uid = user.id;
    let _ = tokio::task::spawn_blocking(move || taste.rebuild_auto_playlists(uid)).await;
    json_res(json!({ "ok": true, "cleared": cleared, "count": 0 }))
}

pub async fn get_lyrics(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(r) = require_web_user(&state, &headers) {
        return r;
    }
    let Some(t) = state.store.get_track(&id).ok().flatten() else {
        return err_res(StatusCode::NOT_FOUND, "Track not found");
    };
    let data = state.engine.fetch_lyrics(&t).await;
    json_res(json!(data))
}

fn parse_recent_param(raw: Option<&str>) -> Vec<(String, i64)> {
    let Some(s) = raw else {
        return Vec::new();
    };
    s.split(',')
        .filter_map(|entry| {
            let trimmed = entry.trim();
            if trimmed.is_empty() {
                return None;
            }
            if let Some((id, age_str)) = trimmed.rsplit_once('@') {
                let id = id.trim();
                if id.is_empty() {
                    return None;
                }
                let age = age_str.trim().parse::<i64>().unwrap_or(0).clamp(0, 7200);
                Some((id.to_string(), age))
            } else {
                Some((trimmed.to_string(), 0))
            }
        })
        .take(60)
        .collect()
}

pub async fn radio(
    State(state): State<AppState>,
    Query(q): Query<RadioQuery>,
    headers: HeaderMap,
) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let exclude: HashSet<String> = q
        .exclude
        .as_deref()
        .map(|s| {
            s.split(',')
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let recent = parse_recent_param(q.recent.as_deref());
    let mode = crate::taste::RadioMode::parse(q.mode.as_deref());

    let mut next = if let Some(pid) = q.playlist.as_deref().filter(|s| !s.trim().is_empty()) {
        let Some(pl) = state.store.get_playlist(pid).ok().flatten() else {
            return err_res(StatusCode::NOT_FOUND, "Playlist not found");
        };
        if !pl.public && pl.user_id != user.id {
            return err_res(StatusCode::FORBIDDEN, "Permission denied");
        }
        state
            .taste
            .playlist_radio_tracks_with_mode(user.id, pid, &exclude, &recent, 18, mode)
            .await
    } else {
        let seed = q
            .seed
            .as_deref()
            .and_then(|id| state.store.get_track(id).ok().flatten());
        state
            .taste
            .radio_next_tracks_with_mode(
                user.id,
                seed.as_ref(),
                &exclude,
                &recent,
                18,
                mode,
            )
            .await
    };
    // Hydrate from SQLite first, then enrich any fresh radio tracks (e.g. Deezer
    // artist radio returns tracks without `release_date`) within a tight time
    // budget so Next Up and skipped tracks display their release year immediately.
    let _ = state.store.hydrate_track_metadata(&mut next);
    if next.iter().any(|t| t.year <= 0 || t.genre.trim().is_empty()) {
        state
            .sources
            .deezer
            .enrich_track_genres_with_timeout(&mut next, std::time::Duration::from_millis(2500))
            .await;
        let to_save = next.clone();
        let store = state.store.clone();
        let remaining_missing: Vec<Track> = next
            .iter()
            .filter(|t| t.year <= 0 || t.genre.trim().is_empty())
            .cloned()
            .collect();
        let dz = state.sources.deezer.clone();
        tokio::spawn(async move {
            let store_first = store.clone();
            let _ = tokio::task::spawn_blocking(move || {
                let _ = store_first.upsert_tracks(&to_save);
            })
            .await;
            if !remaining_missing.is_empty() {
                let mut rest = remaining_missing;
                dz.enrich_track_genres(&mut rest).await;
                let _ = tokio::task::spawn_blocking(move || {
                    let _ = store.upsert_tracks(&rest);
                })
                .await;
            }
        });
    }
    json_res(json!({
        "tracks": to_web_tracks(&state, user.id, &next)
    }))
}

pub async fn get_queue(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let Some((q, tracks, resolved_idx, resolved_id)) =
        state.store.load_play_queue_tracks(user.id).ok().flatten()
    else {
        return json_res(json!({
            "currentId": "",
            "currentIndex": 0,
            "positionMs": 0,
            "tracks": [],
        }));
    };
    json_res(json!({
        "currentId": resolved_id,
        "currentIndex": resolved_idx,
        "positionMs": q.position_ms,
        "tracks": to_web_tracks(&state, user.id, &tracks),
    }))
}

#[derive(Deserialize)]
pub struct WebQueueSaveReq {
    #[serde(default, rename = "currentId")]
    pub current_id: String,
    #[serde(default, rename = "currentIndex")]
    pub current_index: usize,
    #[serde(default, rename = "positionMs")]
    pub position_ms: i64,
    #[serde(default, rename = "trackIds")]
    pub track_ids: Vec<String>,
}

pub async fn save_queue(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<WebQueueSaveReq>,
) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let _ = state.store.save_play_queue(
        user.id,
        &PlayQueueState {
            current_id: req.current_id,
            current_index: req.current_index,
            position_ms: req.position_ms,
            track_ids: req.track_ids,
            changed_by: "webapp".to_string(),
            updated: Utc::now().timestamp(),
        },
    );
    json_res(json!({ "ok": true }))
}

pub async fn warm_track(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if let Err(r) = require_web_user(&state, &headers) {
        return r;
    }
    // `?light=1` (hover intent) only pre-resolves the stream URL; the default
    // (next in queue) also starts caching the audio.
    let light = q.get("light").is_some_and(|v| v == "1");
    if let Some(t) = state.store.get_track(&id).ok().flatten() {
        if light {
            state.engine.spawn_light_warm(t);
        } else {
            state.engine.spawn_warm_track(t);
        }
    }
    let loudness = state.store.get_track_loudness(&id).ok().flatten();
    json_res(json!({ "ok": true, "loudness": loudness }))
}

pub async fn track_loudness(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(r) = require_web_user(&state, &headers) {
        return r;
    }
    let Some(track) = state.store.get_track(&id).ok().flatten() else {
        return err_res(StatusCode::NOT_FOUND, "Track not found");
    };
    let (loudness, pending) = state.engine.ensure_track_loudness(&track).await;
    match loudness {
        Some(m) => json_res(json!({
            "ready": m.exact,
            "pending": !m.exact && pending,
            "lufs": m.lufs,
            "truePeakDb": m.true_peak_db,
        })),
        None => json_res(json!({
            "ready": false,
            "pending": pending,
        })),
    }
}

pub async fn stream_track(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<StreamQuery>,
    headers: HeaderMap,
) -> Response {
    if resolve_web_user(&state, &headers, q.token.as_deref()).is_none() {
        return err_res(StatusCode::UNAUTHORIZED, "Unauthorized stream request");
    }
    let Some(track) = state.store.get_track(&id).ok().flatten() else {
        return err_res(StatusCode::NOT_FOUND, "Track not found");
    };
    let offset = q.offset.unwrap_or(0.0);
    let force_compat = matches!(q.compat.as_deref(), Some("1" | "true"));
    state
        .engine
        .serve_web_stream(&track, &headers, offset, force_compat)
        .await
}

pub async fn cover_art(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    state.engine.serve_cover(&id).await
}

pub async fn sources_info(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = require_web_user(&state, &headers) {
        return r;
    }
    let cache_mb = (state.store.cache_total_bytes().unwrap_or(0) as f64) / (1024.0 * 1024.0);
    json_res(json!({
        "sources": state.sources.sources_info(),
        "totalTracks": state.store.count_tracks().unwrap_or(0),
        "cacheMb": (cache_mb * 10.0).round() / 10.0,
        "remoteCacheEnabled": state.engine.remote.is_some(),
        "remoteCacheMb": (state.store.remote_cache_total_bytes().unwrap_or(0) as f64)
            / (1024.0 * 1024.0),
        "ffmpeg": state.engine.ffmpeg_available,
    }))
}

pub(crate) fn request_origin(headers: &HeaderMap) -> String {
    let host = headers
        .get("x-forwarded-host")
        .or_else(|| headers.get(header::HOST))
        .and_then(|v| v.to_str().ok())
        .and_then(|raw| raw.split(',').next())
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let proto = headers
        .get("x-forwarded-proto")
        .or_else(|| headers.get("x-forwarded-scheme"))
        .and_then(|v| v.to_str().ok())
        .and_then(|raw| raw.split(',').next())
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| s == "http" || s == "https");

    if let Some(h) = host {
        let scheme = proto.unwrap_or_else(|| {
            if h.starts_with("localhost") || h.starts_with("127.0.0.1") || h.starts_with("[::1]") {
                "http".to_string()
            } else {
                "https".to_string()
            }
        });
        return format!("{}://{}", scheme, h);
    }

    if let Ok(env_url) = std::env::var("URL") {
        let trimmed = env_url.trim().trim_end_matches('/');
        if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
            return trimmed.to_string();
        }
        if !trimmed.is_empty() {
            return format!("https://{}", trimmed);
        }
    }

    "http://localhost:4533".to_string()
}

pub(crate) fn is_link_preview_bot(headers: &HeaderMap) -> bool {
    let Some(ua) = headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    let low = ua.to_ascii_lowercase();
    [
        "discordbot",
        "twitterbot",
        "slackbot",
        "telegrambot",
        "whatsapp",
        "facebookexternalhit",
        "linkedinbot",
        "bluesky",
        "embedly",
    ]
    .iter()
    .any(|b| low.contains(b))
}

fn resolve_share_tracks_live(state: &AppState, share: &crate::models::ShareRecord) -> Vec<Track> {
    if share.kind == "playlist" {
        // Follow the playlist live only while it is public or the sharer owns
        // it. If someone else's playlist is made private after being shared,
        // the link falls back to the snapshot taken at share time instead of
        // leaking later edits.
        if let Some(pl) = state
            .store
            .get_playlist(&share.target_id)
            .ok()
            .flatten()
            .filter(|pl| pl.public || pl.user_id == share.user_id)
        {
            let raw = state.store.playlist_tracks(&pl.id).unwrap_or_default();
            let list = if pl.is_auto {
                filter_visible_tracks(state, pl.user_id, raw)
            } else {
                raw
            };
            if !list.is_empty() {
                return list;
            }
        }
    }
    let mut tracks = state.store.get_tracks_by_ids(&share.track_ids).unwrap_or_default();
    if tracks.is_empty() && share.kind == "track" && !share.target_id.is_empty() {
        if let Ok(Some(t)) = state.store.get_track(&share.target_id) {
            tracks.push(t);
        }
    }
    tracks
}

pub async fn create_share(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreateShareReq>,
) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };

    let kind = req.kind.trim().to_ascii_lowercase();
    let target_id = req.target_id.trim().to_string();

    let (resolved_target, title, subtitle, track_ids, lead_track) = match kind.as_str() {
        "track" => {
            if target_id.is_empty() {
                return err_res(StatusCode::BAD_REQUEST, "Missing track ID");
            }
            let Some(t) = state.store.get_track(&target_id).ok().flatten() else {
                return err_res(StatusCode::NOT_FOUND, "Track not found");
            };
            let sub = if t.album.trim().is_empty() {
                t.artist.clone()
            } else {
                format!("{} · {}", t.artist, t.album)
            };
            (
                t.id.clone(),
                t.title.clone(),
                sub,
                vec![t.id.clone()],
                Some(t),
            )
        }
        "playlist" => {
            if target_id.is_empty() {
                return err_res(StatusCode::BAD_REQUEST, "Missing playlist ID");
            }
            let Some(pl) = state.store.get_playlist(&target_id).ok().flatten() else {
                return err_res(StatusCode::NOT_FOUND, "Playlist not found");
            };
            if !pl.public && pl.user_id != user.id {
                return err_res(StatusCode::FORBIDDEN, "Permission denied");
            }
            let raw = state.store.playlist_tracks(&pl.id).unwrap_or_default();
            let list = if pl.is_auto {
                filter_visible_tracks(&state, user.id, raw)
            } else {
                raw
            };
            let ids: Vec<String> = list.iter().take(100).map(|t| t.id.clone()).collect();
            let sub = if pl.comment.trim().is_empty() {
                format!("Playlist by {}", pl.owner_name)
            } else {
                pl.comment.clone()
            };
            (pl.id, pl.name, sub, ids, list.into_iter().next())
        }
        "album" => {
            if target_id.is_empty() {
                return err_res(StatusCode::BAD_REQUEST, "Missing album ID");
            }
            let Some((alb, tracks)) = state.taste.get_or_hydrate_album(&target_id).await else {
                return err_res(StatusCode::NOT_FOUND, "Album not found");
            };
            let ids: Vec<String> = tracks.iter().take(100).map(|t| t.id.clone()).collect();
            let sub = if alb.year > 0 {
                format!("{} · {}", alb.artist, alb.year)
            } else {
                alb.artist.clone()
            };
            (alb.id, alb.name, sub, ids, tracks.into_iter().next())
        }
        "artist" => {
            if target_id.is_empty() {
                return err_res(StatusCode::BAD_REQUEST, "Missing artist ID");
            }
            let Some((art, top_tracks, _, _)) =
                state.taste.get_or_hydrate_artist(&target_id, user.id).await
            else {
                return err_res(StatusCode::NOT_FOUND, "Artist not found");
            };
            let ids: Vec<String> = top_tracks.iter().take(40).map(|t| t.id.clone()).collect();
            (
                art.id,
                art.name,
                "Top Tracks".to_string(),
                ids,
                top_tracks.into_iter().next(),
            )
        }
        "mix" => {
            let trimmed_ids: Vec<String> = req
                .track_ids
                .iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .take(100)
                .collect();
            let valid = state.store.get_tracks_by_ids(&trimmed_ids).unwrap_or_default();
            if valid.is_empty() {
                return err_res(StatusCode::BAD_REQUEST, "Mix contains no playable tracks");
            }
            let ids: Vec<String> = valid.iter().map(|t| t.id.clone()).collect();
            let clean_title = if req.title.trim().is_empty() {
                "Instant Mix".to_string()
            } else {
                req.title.trim().to_string()
            };
            let clean_target = if target_id.is_empty() {
                format!("mix:{}", crate::models::slug(&clean_title))
            } else {
                target_id
            };
            (
                clean_target,
                clean_title,
                req.subtitle.trim().to_string(),
                ids,
                valid.into_iter().next(),
            )
        }
        _ => return err_res(StatusCode::BAD_REQUEST, "Unsupported share kind"),
    };

    match state.store.create_or_get_share(
        user.id,
        &kind,
        &resolved_target,
        &title,
        &subtitle,
        &track_ids,
    ) {
        Ok(share) => {
            if share.kind == "track" && state.engine.ffmpeg_available {
                if let Some(lead) = lead_track {
                    let eng = state.engine.clone();
                    tokio::spawn(async move {
                        let _ = eng.ensure_share_embed_mp4(&lead, ICON_512_PNG).await;
                    });
                }
            }
            let origin = request_origin(&headers);
            let path = format!("/s/{}", share.id);
            let url = format!("{}{}", origin, path);
            json_res(json!({
                "id": share.id,
                "path": path,
                "url": url,
                "kind": share.kind,
                "title": share.title,
                "subtitle": share.subtitle,
                "trackCount": share.track_ids.len(),
            }))
        }
        Err(e) => err_res(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

pub async fn public_share_page(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let Some(share) = state.store.get_share(&id).ok().flatten() else {
        return (
            StatusCode::NOT_FOUND,
            [
                (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                (header::CACHE_CONTROL, "no-cache"),
            ],
            crate::share_page::render_share_not_found_html(),
        )
            .into_response();
    };

    let tracks = resolve_share_tracks_live(&state, &share);
    let mut has_embed_video = false;
    if share.kind == "track" {
        if let Some(lead) = tracks.first() {
            if state.engine.has_cached_embed_mp4(&lead.id) {
                has_embed_video = true;
            } else if state.engine.ffmpeg_available {
                if is_link_preview_bot(&headers) {
                    let res = tokio::time::timeout(
                        std::time::Duration::from_millis(3800),
                        state.engine.ensure_share_embed_mp4(lead, ICON_512_PNG),
                    )
                    .await;
                    has_embed_video = matches!(res, Ok(Some(_)));
                } else {
                    let eng = state.engine.clone();
                    let t_clone = lead.clone();
                    tokio::spawn(async move {
                        let _ = eng.ensure_share_embed_mp4(&t_clone, ICON_512_PNG).await;
                    });
                }
            }
        }
    }

    let primary_mime = tracks
        .first()
        .map(|t| {
            let ext = if t.format.trim().is_empty() {
                "mp3"
            } else {
                t.format.as_str()
            };
            crate::engine::mime_for_audio_ext(ext)
        })
        .unwrap_or("audio/mpeg");

    let origin = request_origin(&headers);
    let html = crate::share_page::render_share_html(
        &origin,
        &share,
        &tracks,
        has_embed_video,
        primary_mime,
    );
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        html,
    )
        .into_response()
}

pub async fn public_share_embed_page(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let Some(share) = state.store.get_share(&id).ok().flatten() else {
        return (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            crate::share_page::render_share_not_found_html(),
        )
            .into_response();
    };
    let tracks = resolve_share_tracks_live(&state, &share);
    let origin = request_origin(&headers);
    let html = crate::share_page::render_embed_html(&origin, &share, &tracks);
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        html,
    )
        .into_response()
}

pub async fn public_share_cover(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let Some(share) = state.store.get_share(&id).ok().flatten() else {
        return (StatusCode::NOT_FOUND, "Share not found").into_response();
    };
    let tracks = resolve_share_tracks_live(&state, &share);
    let primary_target = if share.kind == "mix" {
        tracks
            .first()
            .map(|t| t.id.clone())
            .unwrap_or_else(|| share.target_id.clone())
    } else {
        share.target_id.clone()
    };

    if let Some((bytes, mime)) = state.engine.cover_raster_bytes(&primary_target).await {
        return (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, mime),
                (header::CACHE_CONTROL, "public, max-age=3600".to_string()),
            ],
            bytes,
        )
            .into_response();
    }
    if let Some(first) = tracks.first() {
        if first.id != primary_target {
            if let Some((bytes, mime)) = state.engine.cover_raster_bytes(&first.id).await {
                return (
                    StatusCode::OK,
                    [
                        (header::CONTENT_TYPE, mime),
                        (header::CACHE_CONTROL, "public, max-age=3600".to_string()),
                    ],
                    bytes,
                )
                    .into_response();
            }
        }
    }
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        ICON_512_PNG,
    )
        .into_response()
}

pub async fn public_share_stream_primary(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<StreamQuery>,
    headers: HeaderMap,
) -> Response {
    let Some(share) = state.store.get_share(&id).ok().flatten() else {
        return err_res(StatusCode::NOT_FOUND, "Share not found");
    };
    let tracks = resolve_share_tracks_live(&state, &share);
    let Some(track) = tracks.into_iter().next() else {
        return err_res(StatusCode::NOT_FOUND, "No playable track in share");
    };
    let offset = q.offset.unwrap_or(0.0);
    let force_compat = matches!(q.compat.as_deref(), Some("1" | "true"));
    state
        .engine
        .serve_web_stream(&track, &headers, offset, force_compat)
        .await
}

pub async fn public_share_stream_track(
    State(state): State<AppState>,
    Path((id, track_id)): Path<(String, String)>,
    Query(q): Query<StreamQuery>,
    headers: HeaderMap,
) -> Response {
    let Some(share) = state.store.get_share(&id).ok().flatten() else {
        return err_res(StatusCode::NOT_FOUND, "Share not found");
    };
    let decoded = urlencoding::decode(&track_id)
        .map(|s| s.into_owned())
        .unwrap_or(track_id);
    let tracks = resolve_share_tracks_live(&state, &share);
    let Some(track) = tracks.into_iter().find(|t| t.id == decoded) else {
        return err_res(StatusCode::NOT_FOUND, "Track not part of this share");
    };
    let offset = q.offset.unwrap_or(0.0);
    let force_compat = matches!(q.compat.as_deref(), Some("1" | "true"));
    state
        .engine
        .serve_web_stream(&track, &headers, offset, force_compat)
        .await
}

pub async fn public_share_embed_mp4(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let Some(share) = state.store.get_share(&id).ok().flatten() else {
        return (StatusCode::NOT_FOUND, "Share not found").into_response();
    };
    let tracks = resolve_share_tracks_live(&state, &share);
    let Some(lead) = tracks.first() else {
        return (StatusCode::NOT_FOUND, "Track not found").into_response();
    };
    state
        .engine
        .serve_share_embed_mp4(lead, &headers, ICON_512_PNG)
        .await
}

pub async fn public_share_oembed(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let Some(share) = state.store.get_share(&id).ok().flatten() else {
        return err_res(StatusCode::NOT_FOUND, "Share not found");
    };
    let tracks = resolve_share_tracks_live(&state, &share);
    let (title, _) = crate::share_page::build_share_meta(&share, &tracks);
    let origin = request_origin(&headers);
    let embed_url = format!("{}/s/{}/embed", origin, share.id);
    let cover_url = format!("{}/s/{}/cover", origin, share.id);
    let iframe_html = format!(
        r#"<iframe src="{}" width="480" height="152" frameborder="0" allow="autoplay; encrypted-media"></iframe>"#,
        crate::share_page::html_escape(&embed_url)
    );
    json_res(json!({
        "version": "1.0",
        "type": "rich",
        "provider_name": "Beatlab",
        "provider_url": origin,
        "title": title,
        "thumbnail_url": cover_url,
        "thumbnail_width": 600,
        "thumbnail_height": 600,
        "width": 480,
        "height": 152,
        "html": iframe_html,
    }))
}

pub async fn public_share_lyrics(
    State(state): State<AppState>,
    Path((id, track_id)): Path<(String, String)>,
) -> Response {
    let Some(share) = state.store.get_share(&id).ok().flatten() else {
        return err_res(StatusCode::NOT_FOUND, "Share not found");
    };
    let decoded = urlencoding::decode(&track_id)
        .map(|s| s.into_owned())
        .unwrap_or(track_id);
    let tracks = resolve_share_tracks_live(&state, &share);
    let Some(track) = tracks.into_iter().find(|t| t.id == decoded) else {
        return err_res(StatusCode::NOT_FOUND, "Track not part of this share");
    };
    let lyrics = state.engine.fetch_lyrics(&track).await;
    json_res(serde_json::to_value(lyrics).unwrap_or_else(|_| json!({ "synced": false, "plain": "", "lines": [] })))
}

pub async fn public_share_loudness(
    State(state): State<AppState>,
    Path((id, track_id)): Path<(String, String)>,
) -> Response {
    let Some(share) = state.store.get_share(&id).ok().flatten() else {
        return err_res(StatusCode::NOT_FOUND, "Share not found");
    };
    let decoded = urlencoding::decode(&track_id)
        .map(|s| s.into_owned())
        .unwrap_or(track_id);
    let tracks = resolve_share_tracks_live(&state, &share);
    let Some(track) = tracks.into_iter().find(|t| t.id == decoded) else {
        return err_res(StatusCode::NOT_FOUND, "Track not part of this share");
    };
    let (loudness, pending) = state.engine.ensure_track_loudness(&track).await;
    match loudness {
        Some(m) => json_res(json!({
            "ready": m.exact,
            "pending": !m.exact && pending,
            "lufs": m.lufs,
            "truePeakDb": m.true_peak_db,
        })),
        None => json_res(json!({
            "ready": false,
            "pending": pending,
        })),
    }
}

fn album_json(a: &AlbumSummary, is_saved: bool) -> Value {
    json!({
        "id": a.id,
        "name": a.name,
        "artist": a.artist,
        "artistId": a.artist_id,
        "coverUrl": cover_api_url(&a.id),
        "genre": a.genre,
        "year": a.year,
        "label": a.label,
        "songCount": a.song_count,
        "duration": a.duration,
        "isSaved": is_saved,
    })
}

fn artist_json(a: &ArtistSummary, is_followed: bool) -> Value {
    json!({
        "id": a.id,
        "name": a.name,
        "coverUrl": cover_api_url(&a.id),
        "bio": a.bio,
        "albumCount": a.album_count,
        "songCount": a.song_count,
        "isFollowed": is_followed,
    })
}

fn playlist_summary_with_hidden_json(
    state: &AppState,
    user_id: i64,
    p: &Playlist,
    hidden_ids: &HashSet<String>,
    hidden_keys: &HashSet<String>,
) -> Value {
    if !p.is_auto || user_id <= 0 || (hidden_ids.is_empty() && hidden_keys.is_empty()) {
        return playlist_summary_json(p);
    }
    let raw = state.store.playlist_tracks(&p.id).unwrap_or_default();
    let visible_count = raw
        .into_iter()
        .filter(|t| {
            !hidden_ids.contains(&t.id)
                && (hidden_keys.is_empty() || !hidden_keys.contains(&t.song_key()))
        })
        .count();
    let mut adjusted = p.clone();
    adjusted.track_count = visible_count;
    playlist_summary_json(&adjusted)
}

fn playlist_summary_for_user_json(state: &AppState, user_id: i64, p: &Playlist) -> Value {
    if !p.is_auto || user_id <= 0 {
        return playlist_summary_json(p);
    }
    let (hidden_ids, hidden_keys) = state.store.hidden_track_sets(user_id).unwrap_or_default();
    playlist_summary_with_hidden_json(state, user_id, p, &hidden_ids, &hidden_keys)
}

fn playlist_summary_json(p: &Playlist) -> Value {
    let cover = p
        .cover_track_id
        .as_deref()
        .map(cover_api_url)
        .unwrap_or_else(|| cover_api_url(&p.id));
    json!({
        "id": p.id,
        "name": p.name,
        "comment": p.comment,
        "ownerName": p.owner_name,
        "public": p.public,
        "isAuto": p.is_auto,
        "kind": p.kind,
        "trackCount": p.track_count,
        "duration": p.duration,
        "coverUrl": cover,
        "changed": p.changed,
    })
}

// ---------------------------------------------------------------------------
// Social layer — user search, following, profiles & auto-updating user radio
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct SocialSearchQuery {
    pub q: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Deserialize)]
pub struct ProfileUpdateReq {
    #[serde(default, rename = "displayName")]
    pub display_name: Option<String>,
    #[serde(default)]
    pub bio: Option<String>,
    #[serde(default, rename = "avatarColor")]
    pub avatar_color: Option<String>,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default, rename = "publicPage")]
    pub public_page: Option<bool>,
}

fn public_user_json(u: &PublicUser) -> Value {
    let name = if u.display_name.trim().is_empty() {
        u.username.as_str()
    } else {
        u.display_name.as_str()
    };
    json!({
        "id": u.id,
        "username": u.username,
        "displayName": name,
        "bio": u.bio,
        "avatarColor": u.avatar_color,
        "avatarLetter": u.avatar_letter,
        "avatarUrl": u.avatar_url,
        "location": u.location,
        "created": u.created,
        "followersCount": u.followers_count,
        "followingCount": u.following_count,
        "isFollowing": u.is_following,
        "isSelf": u.is_self,
    })
}

fn profile_json(p: &UserProfile) -> Value {
    json!({
        "userId": p.user_id,
        "displayName": p.display_name,
        "bio": p.bio,
        "avatarColor": p.avatar_color,
        "location": p.location,
        "updated": p.updated,
        "publicPage": p.public_page,
    })
}


/// `{user}` path segment accepts either a numeric id or a username.
fn resolve_user_ref(state: &AppState, reference: &str) -> Option<i64> {
    let trimmed = reference.trim().trim_start_matches('@');
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(id) = trimmed.parse::<i64>() {
        return state
            .store
            .get_public_user(id, id)
            .ok()
            .flatten()
            .map(|_| id);
    }
    state
        .store
        .get_user_by_name(trimmed)
        .ok()
        .flatten()
        .map(|u| u.id)
}

pub async fn social_search(
    State(state): State<AppState>,
    Query(q): Query<SocialSearchQuery>,
    headers: HeaderMap,
) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let query = q.q.unwrap_or_default();
    let limit = q.limit.unwrap_or(20).clamp(1, 50);
    let users = state
        .store
        .search_public_users(&query, user.id, limit)
        .unwrap_or_default();
    json_res(json!({
        "query": query,
        "users": users.iter().map(public_user_json).collect::<Vec<_>>(),
    }))
}

pub async fn social_suggestions(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let users = state.store.suggested_users(user.id, 8).unwrap_or_default();
    json_res(json!({
        "users": users.iter().map(public_user_json).collect::<Vec<_>>(),
    }))
}

pub async fn social_me(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let profile = state.store.get_profile(user.id).unwrap_or_default();
    let public = state.store.get_public_user(user.id, user.id).ok().flatten();
    json_res(json!({
        "user": public.as_ref().map(public_user_json).unwrap_or(Value::Null),
        "profile": profile_json(&profile),
    }))
}

pub async fn social_update_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ProfileUpdateReq>,
) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let current = state.store.get_profile(user.id).unwrap_or_default();
    let display_name = req
        .display_name
        .unwrap_or(current.display_name);
    let bio = req.bio.unwrap_or(current.bio);
    let avatar_color = req.avatar_color.unwrap_or(current.avatar_color);
    let location = req.location.unwrap_or(current.location);
    if let Some(enabled) = req.public_page {
        if let Err(e) = state.store.set_public_page(user.id, enabled) {
            return err_res(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string());
        }
    }
    match state
        .store
        .update_profile(user.id, &display_name, &bio, &avatar_color, &location)
    {
        Ok(profile) => {
            let public = state.store.get_public_user(user.id, user.id).ok().flatten();
            json_res(json!({
                "ok": true,
                "profile": profile_json(&profile),
                "user": public.as_ref().map(public_user_json).unwrap_or(Value::Null),
            }))
        }
        Err(e) => err_res(StatusCode::BAD_REQUEST, &e.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Profile pictures — uploaded by the web player, stored in SQLite, served via
// a small public endpoint so profile cards and shares can display them too.
// ---------------------------------------------------------------------------

const MAX_AVATAR_BYTES: usize = 1024 * 1024;

/// Sniffs the magic bytes of an upload so we never persist arbitrary data as an
/// image. The web player crops/resizes to a small JPEG before uploading, but
/// the server validates independently.
fn detect_image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        Some("image/png")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else {
        None
    }
}

pub async fn social_set_avatar(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    if body.is_empty() || body.len() > MAX_AVATAR_BYTES {
        return err_res(StatusCode::BAD_REQUEST, "Picture must be an image under 1 MB");
    }
    let Some(mime) = detect_image_mime(&body) else {
        return err_res(StatusCode::BAD_REQUEST, "Unsupported image format");
    };
    match state.store.set_avatar(user.id, &body, mime) {
        Ok(()) => {
            // Profile pictures live in the DB, but the durable cool tier keeps
            // a copy alongside every other cached asset.
            state.engine.mirror_asset(
                "avatar",
                &user.id.to_string(),
                body.to_vec(),
                "img",
                mime,
                "",
            );
            let public = state.store.get_public_user(user.id, user.id).ok().flatten();
            json_res(json!({
                "ok": true,
                "user": public.as_ref().map(public_user_json).unwrap_or(Value::Null),
            }))
        }
        Err(e) => err_res(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

pub async fn social_clear_avatar(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    match state.store.clear_avatar(user.id) {
        Ok(()) => {
            // Drop the durable copy too, so a cleared picture stays cleared.
            if let Some(remote) = state.engine.remote.clone() {
                let id = user.id.to_string();
                let key = remote.asset_object_key("avatar", &id);
                let _ = remote.delete(&key).await;
                let _ = state.store.delete_asset_cache_entry("avatar", &id);
            }
            let public = state.store.get_public_user(user.id, user.id).ok().flatten();
            json_res(json!({
                "ok": true,
                "user": public.as_ref().map(public_user_json).unwrap_or(Value::Null),
            }))
        }
        Err(e) => err_res(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

/// Serves an uploaded picture. Public so profile cards and shared links can
/// display it; the URL carries a version query for cache busting.
pub async fn social_avatar_image(
    State(state): State<AppState>,
    Path(reference): Path<String>,
) -> Response {
    let Some(user_id) = resolve_user_ref(&state, &reference) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match state.store.get_avatar(user_id) {
        Ok(Some((bytes, mime))) => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, mime.as_str()),
                (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
            ],
            bytes,
        )
            .into_response(),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Maximum time a playing session stays live without a heartbeat before it is
/// treated as gone (e.g. crashed browser, killed process, or dropped network).
/// Active web clients ping every ~20s while audio is playing, and also send an
/// explicit `playing: false` keepalive request on pause, queue end, sign-out,
/// and `pagehide`/`beforeunload`.
const NOW_PLAYING_HEARTBEAT_TTL_SECS: i64 = 55;

fn normalize_now_playing_client_id(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.chars().take(64).collect())
    }
}

fn upsert_now_playing(
    state: &AppState,
    user_id: i64,
    track: Track,
    now: i64,
    client_id: Option<String>,
    seq: u64,
) {
    let mut np = state.now_playing.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(existing) = np.get_mut(&user_id) {
        if let (Some(ref cur_cid), Some(ref new_cid)) = (&existing.client_id, &client_id) {
            if cur_cid == new_cid && seq > 0 && seq <= existing.seq {
                return;
            }
        }
        if existing.track.as_ref().map(|t| t.id.as_str()) == Some(track.id.as_str())
            && existing.client_id == client_id
        {
            existing.last_seen_at = now;
            if seq > 0 {
                existing.seq = seq;
            }
            return;
        }
    }
    np.insert(
        user_id,
        crate::state::NowPlayingEntry {
            track: Some(track),
            started_at: now,
            last_seen_at: now,
            client_id,
            seq,
        },
    );
}

fn clear_now_playing(
    state: &AppState,
    user_id: i64,
    now: i64,
    client_id: Option<String>,
    seq: u64,
) {
    let mut np = state.now_playing.lock().unwrap_or_else(|e| e.into_inner());
    match (np.get(&user_id), client_id) {
        (Some(existing), Some(cid)) => {
            if let Some(ref cur_cid) = existing.client_id {
                if cur_cid != &cid {
                    // Another active tab/device owns now-playing; leave it untouched.
                    return;
                }
                if seq > 0 && seq <= existing.seq {
                    return;
                }
            }
            np.insert(
                user_id,
                crate::state::NowPlayingEntry {
                    track: None,
                    started_at: now,
                    last_seen_at: now,
                    client_id: Some(cid),
                    seq,
                },
            );
        }
        (None, Some(cid)) => {
            // Keep a brief tombstone so an in-flight earlier `play` from this same
            // client cannot resurrect now-playing after a quick close or pause.
            if seq > 0 {
                np.insert(
                    user_id,
                    crate::state::NowPlayingEntry {
                        track: None,
                        started_at: now,
                        last_seen_at: now,
                        client_id: Some(cid),
                        seq,
                    },
                );
            }
        }
        (_, None) => {
            np.remove(&user_id);
        }
    }
}

pub async fn update_now_playing(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<NowPlayingReq>,
) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let now = Utc::now().timestamp();
    let client_id = normalize_now_playing_client_id(&req.client_id);

    if !req.playing {
        clear_now_playing(&state, user.id, now, client_id, req.seq);
        return json_res(json!({ "ok": true }));
    }

    let tid = req.track_id.trim();
    if tid.is_empty() {
        return err_res(StatusCode::BAD_REQUEST, "Missing trackId");
    }

    // Fast path for heartbeats: refresh `last_seen_at` in-memory without touching SQLite.
    {
        let mut np = state.now_playing.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(existing) = np.get_mut(&user.id) {
            if let (Some(ref cur_cid), Some(ref new_cid)) = (&existing.client_id, &client_id) {
                if cur_cid == new_cid && req.seq > 0 && req.seq <= existing.seq {
                    return json_res(json!({ "ok": true }));
                }
            }
            if existing.track.as_ref().map(|t| t.id.as_str()) == Some(tid)
                && existing.client_id == client_id
            {
                existing.last_seen_at = now;
                if req.seq > 0 {
                    existing.seq = req.seq;
                }
                return json_res(json!({ "ok": true }));
            }
        }
    }

    if let Ok(Some(track)) = state.store.get_track(tid) {
        upsert_now_playing(&state, user.id, track, now, client_id, req.seq);
    }
    json_res(json!({ "ok": true }))
}

/// Returns the user's currently playing track if it has received a recent
/// playback start or heartbeat within `NOW_PLAYING_HEARTBEAT_TTL_SECS`.
fn active_now_playing(state: &AppState, user_id: i64, now: i64) -> Option<(Track, i64)> {
    let mut np = state.now_playing.lock().unwrap_or_else(|e| e.into_inner());
    let entry = np.get(&user_id)?;
    if now.saturating_sub(entry.last_seen_at) > NOW_PLAYING_HEARTBEAT_TTL_SECS {
        np.remove(&user_id);
        return None;
    }
    entry.track.as_ref().map(|t| (t.clone(), entry.started_at))
}

pub async fn social_profile(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    headers: HeaderMap,
) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let Some(target_id) = resolve_user_ref(&state, &reference) else {
        return err_res(StatusCode::NOT_FOUND, "User not found");
    };
    let Some(target) = state.store.get_public_user(target_id, user.id).ok().flatten() else {
        return err_res(StatusCode::NOT_FOUND, "User not found");
    };
    // Listening stats are computed from the target's all-time history and shared
    // as safe aggregates only (see stats::PublicStats).
    let target_user = User {
        id: target.id,
        username: target.username.clone(),
        password: String::new(),
        created: target.created,
    };
    let now = Utc::now().timestamp();
    let store = state.store.clone();
    let stats = tokio::task::spawn_blocking(move || {
        crate::stats::build_public_stats(&store, &target_user, now)
    })
    .await
    .unwrap_or_default();
    let profile = state.store.get_profile(target_id).unwrap_or_default();
    let radio_saved = state
        .store
        .get_user_radio(user.id, target_id)
        .ok()
        .flatten()
        .is_some();
    let playlists: Vec<Value> = state
        .store
        .list_playlists(target_id, true)
        .unwrap_or_default()
        .into_iter()
        .filter(|p| p.user_id == target_id && p.public && !p.is_auto)
        .take(12)
        .map(|p| playlist_summary_json(&p))
        .collect();
    let now_playing = active_now_playing(&state, target_id, now).and_then(|(t, started_at)| {
        to_web_tracks(&state, user.id, &[t]).into_iter().next().map(|wt| {
            json!({
                "track": wt,
                "startedAt": started_at,
            })
        })
    });
    json_res(json!({
        "user": public_user_json(&target),
        "profile": profile_json(&profile),
        "stats": serde_json::to_value(&stats).unwrap_or(Value::Null),
        "radioSaved": radio_saved,
        "playlists": playlists,
        "nowPlaying": now_playing,
    }))
}

/// Rendered public profile pages are cached briefly so anonymous traffic can't
/// force the stats computation on every hit. The public-page opt-out is
/// checked *before* the cache, so switching it off takes effect immediately;
/// other profile edits show up within the TTL.
const PUBLIC_PROFILE_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(60);
const PUBLIC_PROFILE_CACHE_MAX: usize = 512;

fn public_profile_cache() -> &'static std::sync::Mutex<HashMap<String, (std::time::Instant, String)>> {
    static CACHE: OnceLock<std::sync::Mutex<HashMap<String, (std::time::Instant, String)>>> =
        OnceLock::new();
    CACHE.get_or_init(Default::default)
}

#[derive(Deserialize)]
pub struct PublicProfileQuery {
    pub range: Option<String>,
    pub metric: Option<String>,
}

/// `/u/{username}` — public profile overview.
pub async fn public_profile_page(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    Query(q): Query<PublicProfileQuery>,
    headers: HeaderMap,
) -> Response {
    render_public_profile(&state, &headers, &reference, None, q.range.as_deref(), q.metric.as_deref()).await
}

/// `/u/{username}/{tab}` — stats, playlists, followers or following.
pub async fn public_profile_tab_page(
    State(state): State<AppState>,
    Path((reference, tab)): Path<(String, String)>,
    Query(q): Query<PublicProfileQuery>,
    headers: HeaderMap,
) -> Response {
    render_public_profile(&state, &headers, &reference, Some(&tab), q.range.as_deref(), q.metric.as_deref()).await
}

/// Public, sign-in-free profile pages. They show the same privacy-safe data
/// the in-app profile exposes to any signed-in user, plus Open Graph meta so
/// links unfurl in chat apps.
async fn render_public_profile(
    state: &AppState,
    headers: &HeaderMap,
    reference: &str,
    tab_slug: Option<&str>,
    range: Option<&str>,
    metric: Option<&str>,
) -> Response {
    use crate::profile_page::{ProfileBody, ProfileTab};
    let html_response = |status: StatusCode, html: String| {
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
    let not_found =
        || html_response(StatusCode::NOT_FOUND, crate::profile_page::render_profile_not_found_html());

    let Some(tab) = ProfileTab::from_slug(tab_slug) else {
        return not_found();
    };
    // Usernames only — numeric ids are not accepted on public URLs.
    let name = reference.trim().trim_start_matches('@');
    let Some(account) = state.store.get_user_by_name(name).ok().flatten() else {
        return not_found();
    };
    // Viewer id 0 = anonymous; accounts pending deletion are excluded.
    let Some(target) = state.store.get_public_user(account.id, 0).ok().flatten() else {
        return not_found();
    };
    // Users who switched their public page off look exactly like a missing
    // account, so the link doesn't confirm the username exists.
    if !state.store.get_profile(target.id).map(|p| p.public_page).unwrap_or(false) {
        return not_found();
    }

    let stats_range = crate::stats::StatsRange::from_str(range);
    let stats_metric = if metric.map(|m| m.eq_ignore_ascii_case("mins")).unwrap_or(false) {
        "mins"
    } else {
        "streams"
    };
    let now = Utc::now().timestamp();
    let now_playing = active_now_playing(state, target.id, now);
    let np_cache_tag = now_playing
        .as_ref()
        .map(|(t, _)| t.id.as_str())
        .unwrap_or("");
    let origin = request_origin(headers);
    let cache_key = format!(
        "{}|{}|{}|{}|{}|{}",
        origin,
        target.id,
        tab.slug(),
        stats_range.key(),
        stats_metric,
        np_cache_tag
    );
    if let Ok(cache) = public_profile_cache().lock() {
        if let Some((at, html)) = cache.get(&cache_key) {
            if at.elapsed() < PUBLIC_PROFILE_CACHE_TTL {
                return html_response(StatusCode::OK, html.clone());
            }
        }
    }

    let target_user = User {
        id: target.id,
        username: target.username.clone(),
        password: String::new(),
        created: target.created,
    };
    let public_playlists = |limit: usize| -> Vec<Playlist> {
        state
            .store
            .list_playlists(target.id, true)
            .unwrap_or_default()
            .into_iter()
            .filter(|p| p.user_id == target.id && p.public && !p.is_auto)
            .take(limit)
            .collect()
    };
    // Only list people who have a public page themselves; the rest are counted.
    let visible_people = |users: Vec<PublicUser>| -> (Vec<PublicUser>, usize) {
        let total = users.len();
        let shown: Vec<PublicUser> = users
            .into_iter()
            .filter(|u| state.store.get_profile(u.id).map(|p| p.public_page).unwrap_or(false))
            .take(200)
            .collect();
        let hidden = total.saturating_sub(shown.len());
        (shown, hidden)
    };
    let np_track = now_playing.as_ref().map(|(t, _)| t);
    let store = state.store.clone();

    let html = match tab {
        ProfileTab::Overview => {
            let stats = tokio::task::spawn_blocking(move || {
                crate::stats::build_public_stats(&store, &target_user, now)
            })
            .await
            .unwrap_or_default();
            let playlists = public_playlists(12);
            crate::profile_page::render_profile_page(
                &origin,
                &target,
                &ProfileBody::Overview { stats: &stats, playlists: &playlists },
                np_track,
            )
        }
        ProfileTab::Stats => {
            state.taste.enrich_user_listened_tracks(target.id).await;
            let Ok(mut stats) = tokio::task::spawn_blocking(move || {
                crate::stats::build_user_stats(&store, &target_user, stats_range, 0, now)
            })
            .await
            else {
                return html_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Failed to compute stats".to_string(),
                );
            };
            // Match the in-app profile stats privacy boundary: omit the
            // per-track activity log and hidden-track count.
            stats.recent_events.clear();
            stats.library.hidden_tracks = 0;
            if stats.summary.total_streams > 0 {
                if let Some(exp) = crate::llm::LlmCurator::load_experience(&state.store, target.id) {
                    if !exp.taste_portrait.is_empty() {
                        if !exp.taste_portrait.title.trim().is_empty() {
                            stats.persona.title = exp.taste_portrait.title;
                        }
                        if !exp.taste_portrait.summary.trim().is_empty() {
                            stats.persona.headline = exp.taste_portrait.summary;
                        }
                        if !exp.taste_portrait.traits.is_empty() {
                            stats.persona.badges = exp.taste_portrait.traits;
                        }
                    }
                }
            }
            crate::profile_page::render_profile_page(
                &origin,
                &target,
                &ProfileBody::Stats { stats: &stats, metric: stats_metric },
                np_track,
            )
        }
        ProfileTab::Playlists => {
            let playlists = public_playlists(100);
            crate::profile_page::render_profile_page(
                &origin,
                &target,
                &ProfileBody::Playlists(&playlists),
                np_track,
            )
        }
        ProfileTab::Followers => {
            let (users, hidden) =
                visible_people(state.store.list_followers(target.id, 0).unwrap_or_default());
            crate::profile_page::render_profile_page(
                &origin,
                &target,
                &ProfileBody::Followers(&users, hidden),
                np_track,
            )
        }
        ProfileTab::Following => {
            let (users, hidden) =
                visible_people(state.store.list_following(target.id, 0).unwrap_or_default());
            crate::profile_page::render_profile_page(
                &origin,
                &target,
                &ProfileBody::Following(&users, hidden),
                np_track,
            )
        }
    };

    if let Ok(mut cache) = public_profile_cache().lock() {
        if cache.len() >= PUBLIC_PROFILE_CACHE_MAX {
            cache.retain(|_, (at, _)| at.elapsed() < PUBLIC_PROFILE_CACHE_TTL);
            if cache.len() >= PUBLIC_PROFILE_CACHE_MAX {
                cache.clear();
            }
        }
        cache.insert(cache_key, (std::time::Instant::now(), html.clone()));
    }
    html_response(StatusCode::OK, html)
}

pub async fn social_follow(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    headers: HeaderMap,
) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let Some(target_id) = resolve_user_ref(&state, &reference) else {
        return err_res(StatusCode::NOT_FOUND, "User not found");
    };
    match state.store.follow_user(user.id, target_id) {
        Ok(()) => {
            let target = state
                .store
                .get_public_user(target_id, user.id)
                .ok()
                .flatten();
            json_res(json!({
                "ok": true,
                "following": true,
                "user": target.as_ref().map(public_user_json).unwrap_or(Value::Null),
            }))
        }
        Err(e) => err_res(StatusCode::BAD_REQUEST, &e.to_string()),
    }
}

pub async fn social_unfollow(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    headers: HeaderMap,
) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let Some(target_id) = resolve_user_ref(&state, &reference) else {
        return err_res(StatusCode::NOT_FOUND, "User not found");
    };
    match state.store.unfollow_user(user.id, target_id) {
        Ok(()) => {
            let target = state
                .store
                .get_public_user(target_id, user.id)
                .ok()
                .flatten();
            json_res(json!({
                "ok": true,
                "following": false,
                "user": target.as_ref().map(public_user_json).unwrap_or(Value::Null),
            }))
        }
        Err(e) => err_res(StatusCode::BAD_REQUEST, &e.to_string()),
    }
}

pub async fn social_followers(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    headers: HeaderMap,
) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let Some(target_id) = resolve_user_ref(&state, &reference) else {
        return err_res(StatusCode::NOT_FOUND, "User not found");
    };
    let users = state
        .store
        .list_followers(target_id, user.id)
        .unwrap_or_default();
    json_res(json!({
        "users": users.iter().map(public_user_json).collect::<Vec<_>>(),
    }))
}

pub async fn social_following(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    headers: HeaderMap,
) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let Some(target_id) = resolve_user_ref(&state, &reference) else {
        return err_res(StatusCode::NOT_FOUND, "User not found");
    };
    let users = state
        .store
        .list_following(target_id, user.id)
        .unwrap_or_default();
    json_res(json!({
        "users": users.iter().map(public_user_json).collect::<Vec<_>>(),
    }))
}

fn radio_name_for(target: &PublicUser) -> String {
    let display = if target.display_name.trim().is_empty() {
        target.username.as_str()
    } else {
        target.display_name.as_str()
    };
    format!("{} Radio", display)
}

/// Generate a listener's radio on demand from their listening taste.
pub async fn social_user_radio(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    headers: HeaderMap,
) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let Some(target_id) = resolve_user_ref(&state, &reference) else {
        return err_res(StatusCode::NOT_FOUND, "User not found");
    };
    let Some(target) = state.store.get_public_user(target_id, user.id).ok().flatten() else {
        return err_res(StatusCode::NOT_FOUND, "User not found");
    };
    let tracks = state.taste.user_radio_tracks(target_id, 40);
    let saved_pid = state
        .store
        .get_user_radio(user.id, target_id)
        .ok()
        .flatten();
    let playlist = saved_pid
        .as_deref()
        .and_then(|pid| state.store.get_playlist(pid).ok().flatten());
    json_res(json!({
        "user": public_user_json(&target),
        "tracks": to_web_tracks(&state, user.id, &tracks),
        "saved": saved_pid.is_some(),
        "playlist": playlist.as_ref().map(playlist_summary_json).unwrap_or(Value::Null),
    }))
}

/// Subscribe to a listener's radio: saves an auto-updating playlist in the
/// viewer's library that the background curator rebuilds from that listener's
/// evolving taste.
pub async fn social_save_user_radio(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    headers: HeaderMap,
) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let Some(target_id) = resolve_user_ref(&state, &reference) else {
        return err_res(StatusCode::NOT_FOUND, "User not found");
    };
    let Some(target) = state.store.get_public_user(target_id, user.id).ok().flatten() else {
        return err_res(StatusCode::NOT_FOUND, "User not found");
    };
    let pid = format!("radio-u{}-t{}", user.id, target_id);
    let ids: Vec<String> = state
        .taste
        .user_radio_tracks(target_id, 40)
        .into_iter()
        .map(|t| t.id)
        .collect();
    if state
        .store
        .upsert_auto_playlist(
            &pid,
            user.id,
            &radio_name_for(&target),
            "Auto-updating mix of what this listener plays",
            &format!("user_radio:{}", target_id),
            &ids,
        )
        .is_err()
    {
        return err_res(StatusCode::INTERNAL_SERVER_ERROR, "Could not save radio");
    }
    if state.store.save_user_radio(user.id, target_id, &pid).is_err() {
        return err_res(StatusCode::INTERNAL_SERVER_ERROR, "Could not save radio");
    }
    state.store.kv_set(
        &format!("radio:rebuilt:{}:{}", user.id, target_id),
        &Utc::now().timestamp().to_string(),
    );
    let playlist = state.store.get_playlist(&pid).ok().flatten();
    json_res(json!({
        "ok": true,
        "saved": true,
        "playlist": playlist.as_ref().map(playlist_summary_json).unwrap_or(Value::Null),
    }))
}

pub async fn social_unsave_user_radio(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    headers: HeaderMap,
) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let Some(target_id) = resolve_user_ref(&state, &reference) else {
        return err_res(StatusCode::NOT_FOUND, "User not found");
    };
    match state.store.delete_user_radio(user.id, target_id) {
        Ok(pid) => {
            if let Some(pid) = pid {
                let _ = state.store.delete_playlist(&pid);
            }
            json_res(json!({ "ok": true, "saved": false }))
        }
        Err(e) => err_res(StatusCode::BAD_REQUEST, &e.to_string()),
    }
}

/// Compact radio previews from the people the viewer follows.
pub async fn social_feed(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let following = state
        .store
        .list_following(user.id, user.id)
        .unwrap_or_default();
    let radios: Vec<Value> = following
        .into_iter()
        .take(6)
        .map(|target| {
            // Use the same generator as the profile radio. A listener can have a
            // full radio from stars/taste/catalog without any play-count stats,
            // which made the Community preview claim "0 tracks".
            let full = state.taste.user_radio_tracks(target.id, 40);
            let track_count = full.len();
            let preview: Vec<Track> = full.into_iter().take(6).collect();
            json!({
                "user": public_user_json(&target),
                "trackCount": track_count,
                "tracks": to_web_tracks(&state, user.id, &preview),
            })
        })
        .collect();
    json_res(json!({
        "radios": radios,
        "count": radios.len(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::Query;
    use axum::http::Uri;

    #[test]
    fn stream_query_parses_auto_quality_and_compat_params() {
        let uri: Uri = "/api/web/stream/tr1?token=tok123".parse().unwrap();
        let Query(q) = Query::<StreamQuery>::try_from_uri(&uri).unwrap();
        assert_eq!(q.token.as_deref(), Some("tok123"));
        assert!(q.offset.is_none());
        assert!(q.compat.is_none());

        let uri2: Uri = "/api/web/stream/tr1?token=tok123&t=42.5&compat=1".parse().unwrap();
        let Query(q2) = Query::<StreamQuery>::try_from_uri(&uri2).unwrap();
        assert_eq!(q2.offset, Some(42.5));
        assert_eq!(q2.compat.as_deref(), Some("1"));
    }

    fn account_test_state() -> AppState {
        use crate::engine::StreamEngine;
        use crate::sources::archive::ArchiveSource;
        use crate::sources::deezer::DeezerSource;
        use crate::sources::local::LocalSource;
        use crate::sources::soulseek::SoulseekSource;
        use crate::sources::ytm::YtmSource;
        use crate::sources::SourceRegistry;
        use crate::store::Store;
        use crate::taste::TasteEngine;
        use std::sync::{Arc, Mutex};

        let store = Store::open_memory().unwrap();
        let tmp = std::env::temp_dir().join(format!("beatlab-acct-{}", uuid::Uuid::new_v4()));
        let sources = SourceRegistry::new(
            DeezerSource::new(),
            YtmSource::new(None),
            ArchiveSource::new(),
            Arc::new(LocalSource::new(store.clone(), vec![])),
            SoulseekSource::new(store.clone(), tmp.join("slsk")),
        );
        let engine = Arc::new(StreamEngine::new(store.clone(), sources.clone(), &tmp, 1.0));
        let taste = Arc::new(TasteEngine::new(store.clone(), sources.clone()));
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

    fn bearer(token: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(header::AUTHORIZATION, format!("Bearer {token}").parse().unwrap());
        h
    }

    fn solved_test_captcha(headers: &HeaderMap, action: &str) -> crate::captcha::CaptchaSolution {
        let ip = rate_limit_ip(headers);
        let ch = crate::captcha::issue_challenge(
            headers,
            &ip,
            action,
            &crate::captcha::BrowserSignals {
                languages: 1,
                screen_w: 1920,
                screen_h: 1080,
                inner_w: 1280,
                inner_h: 720,
                cores: 4,
                canvas_ok: true,
                layout_ok: true,
                worker_ok: true,
                signals_present: true,
                ..Default::default()
            },
        )
        .unwrap();
        crate::captcha::solve_for_test(&ch)
    }

    #[tokio::test]
    async fn repeated_failed_signins_are_throttled_and_passwords_are_hashed() {
        let state = account_test_state();
        let (u, _) = state.store.authenticate_or_create_user("throttled-user", "right-pass").unwrap();
        assert!(crate::auth::is_password_hash(&state.store.get_user_by_id(u.id).unwrap().unwrap().password));

        let headers = HeaderMap::new();
        let attempt = |pw: &str| AuthReq {
            username: "throttled-user".into(),
            password: pw.into(),
            captcha: Some(solved_test_captcha(&headers, "signin")),
            hp_token: String::new(),
        };
        for _ in 0..crate::auth::MAX_FAILURES_PER_ACCOUNT {
            let res = signin(State(state.clone()), headers.clone(), Json(attempt("wrong"))).await;
            assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        }
        // Even the right password is refused while the account is throttled.
        let res = signin(State(state.clone()), headers.clone(), Json(attempt("right-pass"))).await;
        assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
        assert!(res.headers().contains_key(header::RETRY_AFTER));
    }

    #[tokio::test]
    async fn signin_and_signup_require_valid_pow_captcha() {
        let state = account_test_state();
        let headers = HeaderMap::new();

        // Missing captcha on signup -> rejected before account creation.
        let res = signup(
            State(state.clone()),
            headers.clone(),
            Json(SignupReq {
                username: "botuser".into(),
                password: "password123".into(),
                confirm_password: "password123".into(),
                captcha: None,
                hp_token: String::new(),
            }),
        )
        .await;
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
        assert!(state.store.get_user_by_name("botuser").unwrap().is_none());

        // Honeypot filled even with valid captcha -> rejected.
        let sol = solved_test_captcha(&headers, "signup");
        let res = signup(
            State(state.clone()),
            headers.clone(),
            Json(SignupReq {
                username: "botuser".into(),
                password: "password123".into(),
                confirm_password: "password123".into(),
                captcha: Some(sol),
                hp_token: "spam-bot".into(),
            }),
        )
        .await;
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
        assert!(state.store.get_user_by_name("botuser").unwrap().is_none());

        // Valid captcha -> signup succeeds.
        let sol = solved_test_captcha(&headers, "signup");
        let res = signup(
            State(state.clone()),
            headers.clone(),
            Json(SignupReq {
                username: "humanuser".into(),
                password: "password123".into(),
                confirm_password: "password123".into(),
                captcha: Some(sol.clone()),
                hp_token: String::new(),
            }),
        )
        .await;
        assert_eq!(res.status(), StatusCode::OK);

        // Missing captcha on signin -> rejected.
        let res = signin(
            State(state.clone()),
            headers.clone(),
            Json(AuthReq {
                username: "humanuser".into(),
                password: "password123".into(),
                captcha: None,
                hp_token: String::new(),
            }),
        )
        .await;
        assert_eq!(res.status(), StatusCode::FORBIDDEN);

        // Replaying the consumed signup captcha on signin -> rejected.
        let res = signin(
            State(state.clone()),
            headers.clone(),
            Json(AuthReq {
                username: "humanuser".into(),
                password: "password123".into(),
                captcha: Some(sol),
                hp_token: String::new(),
            }),
        )
        .await;
        assert_eq!(res.status(), StatusCode::FORBIDDEN);

        // Fresh signin captcha -> succeeds.
        let res = signin(
            State(state.clone()),
            headers.clone(),
            Json(AuthReq {
                username: "humanuser".into(),
                password: "password123".into(),
                captcha: Some(solved_test_captcha(&headers, "signin")),
                hp_token: String::new(),
            }),
        )
        .await;
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn wrong_password_does_not_burn_a_recovery_code() {
        let state = account_test_state();
        let (u, _) = state.store.authenticate_or_create_user("codeuser", "real-pass").unwrap();
        state.store.save_totp(u.id, &crate::auth::generate_totp_secret(), true).unwrap();
        let codes = crate::auth::generate_recovery_codes(2);
        state.store.replace_recovery_codes(u.id, &codes).unwrap();
        let tok = state.store.create_session(u.id, "web").unwrap();

        let res = change_password(
            State(state.clone()),
            bearer(&tok),
            Json(PasswordChangeReq {
                current_password: "wrong-pass".into(),
                new_password: "new-pass".into(),
                code: Some(codes[0].clone()),
            }),
        )
        .await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        assert_eq!(state.store.recovery_code_stats(u.id).unwrap(), (2, 0));

        let res = change_password(
            State(state.clone()),
            bearer(&tok),
            Json(PasswordChangeReq {
                current_password: "real-pass".into(),
                new_password: "new-pass".into(),
                code: Some(codes[0].clone()),
            }),
        )
        .await;
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(state.store.recovery_code_stats(u.id).unwrap(), (2, 1));
        assert!(state.store.authenticate_user("codeuser", "new-pass").is_ok());
    }

    #[tokio::test]
    async fn password_change_signs_out_other_sessions_only() {
        let state = account_test_state();
        let (u, _) = state.store.authenticate_or_create_user("pwuser", "old-pass").unwrap();
        let current = state.store.create_session(u.id, "web").unwrap();
        let other = state.store.create_session(u.id, "web").unwrap();

        let res = change_password(
            State(state.clone()),
            bearer(&current),
            Json(PasswordChangeReq {
                current_password: "old-pass".into(),
                new_password: "new-pass".into(),
                code: None,
            }),
        )
        .await;
        assert_eq!(res.status(), StatusCode::OK);
        assert!(state.store.get_session_user(&current).unwrap().is_some());
        assert!(state.store.get_session_user(&other).unwrap().is_none());
    }

    #[test]
    fn radio_query_accepts_playlist_and_exclusions() {
        let uri: Uri = "/api/web/radio?playlist=pl-123&exclude=t1,t2".parse().unwrap();
        let Query(q) = Query::<RadioQuery>::try_from_uri(&uri).unwrap();
        assert!(q.seed.is_none());
        assert_eq!(q.playlist.as_deref(), Some("pl-123"));
        assert_eq!(q.exclude.as_deref(), Some("t1,t2"));
    }

    #[tokio::test]
    async fn playlist_add_dedups_and_triggers_learning_events() {
        use crate::engine::StreamEngine;
        use crate::sources::archive::ArchiveSource;
        use crate::sources::deezer::DeezerSource;
        use crate::sources::local::LocalSource;
        use crate::sources::soulseek::SoulseekSource;
        use crate::sources::ytm::YtmSource;
        use crate::sources::SourceRegistry;
        use crate::store::Store;
        use crate::taste::TasteEngine;
        use axum::body::to_bytes;
        use std::sync::{Arc, Mutex};

        let store = Store::open_memory().unwrap();
        let tmp = std::env::temp_dir().join(format!("beatlab-web-{}", uuid::Uuid::new_v4()));
        let sources = SourceRegistry::new(
            DeezerSource::new(),
            YtmSource::new(None),
            ArchiveSource::new(),
            Arc::new(LocalSource::new(store.clone(), vec![])),
            SoulseekSource::new(store.clone(), tmp.join("slsk")),
        );
        let engine = Arc::new(StreamEngine::new(store.clone(), sources.clone(), &tmp, 1.0));
        let taste = Arc::new(TasteEngine::new(store.clone(), sources.clone()));
        let t1 = Track {
            id: "dz:1".into(),
            source: "deezer".into(),
            title: "Midnight Groove".into(),
            artist: "Parcels".into(),
            genre: "Funk, Electronic".into(),
            moods: "groove,party".into(),
            year: 2021,
            duration: 210,
            bitrate: 256,
            ..Default::default()
        };
        store.upsert_tracks(&[t1.clone()]).unwrap();
        let (u, _) = store.authenticate_or_create_user("webuser", "pw123").unwrap();
        let tok = store.create_session(u.id, "web").unwrap();
        let pl = store.create_playlist(u.id, "Funky Cuts", "", false, &[]).unwrap();

        let state = AppState {
            store: store.clone(),
            sources,
            engine: engine.clone(),
            taste,
            llm: None,
            now_playing: Arc::new(Mutex::new(Default::default())),
            new_releases_inflight: Arc::new(Mutex::new(Default::default())),
            lobbies: Default::default(),
        };
        let mut hdrs = HeaderMap::new();
        hdrs.insert(header::AUTHORIZATION, format!("Bearer {}", tok).parse().unwrap());

        let r1 = add_playlist_tracks(
            State(state.clone()),
            Path(pl.id.clone()),
            hdrs.clone(),
            Json(PlaylistAddReq {
                track_ids: vec!["dz:1".into(), "dz:1".into()],
            }),
        )
        .await;
        assert_eq!(r1.status(), StatusCode::OK);
        let b1: Value = serde_json::from_slice(&to_bytes(r1.into_body(), 1024 * 64).await.unwrap()).unwrap();
        assert_eq!(b1["added"], 1);

        let r2 = add_playlist_tracks(
            State(state.clone()),
            Path(pl.id.clone()),
            hdrs,
            Json(PlaylistAddReq {
                track_ids: vec!["dz:1".into()],
            }),
        )
        .await;
        let b2: Value = serde_json::from_slice(&to_bytes(r2.into_body(), 1024 * 64).await.unwrap()).unwrap();
        assert_eq!(b2["added"], 0);

        let events = store.user_recent_events(u.id, 10).unwrap();
        assert_eq!(events.iter().filter(|e| e.kind == "playlist_add").count(), 1);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn search_and_query_event_refine_and_never_save_playlists() {
        use crate::engine::StreamEngine;
        use crate::sources::archive::ArchiveSource;
        use crate::sources::deezer::DeezerSource;
        use crate::sources::local::LocalSource;
        use crate::sources::soulseek::SoulseekSource;
        use crate::sources::ytm::YtmSource;
        use crate::sources::SourceRegistry;
        use crate::store::Store;
        use crate::taste::TasteEngine;
        use axum::body::to_bytes;
        use std::sync::{Arc, Mutex};

        let store = Store::open_memory().unwrap();
        let tmp = std::env::temp_dir().join(format!("beatlab-web-mix-{}", uuid::Uuid::new_v4()));
        let sources = SourceRegistry::new(
            DeezerSource::new(),
            YtmSource::new(None),
            ArchiveSource::new(),
            Arc::new(LocalSource::new(store.clone(), vec![])),
            SoulseekSource::new(store.clone(), tmp.join("slsk")),
        );
        let engine = Arc::new(StreamEngine::new(store.clone(), sources.clone(), &tmp, 1.0));
        let taste = Arc::new(TasteEngine::new(store.clone(), sources.clone()));

        let t1 = Track {
            id: "loc:80s-1".into(),
            source: "local".into(),
            title: "Take On Me".into(),
            artist: "a-ha".into(),
            album: "Hunting High and Low".into(),
            genre: "Synth-Pop, 80s, Pop".into(),
            moods: "energetic,happy".into(),
            year: 1985,
            duration: 225,
            bitrate: 900,
            format: "flac".into(),
            meta: "{\"rank\":880000}".into(),
            ..Default::default()
        };
        let t2 = Track {
            id: "loc:80s-2".into(),
            source: "local".into(),
            title: "Blue Monday".into(),
            artist: "New Order".into(),
            album: "Power, Corruption & Lies".into(),
            genre: "New Wave, Electronic, 80s".into(),
            moods: "groove,night".into(),
            year: 1983,
            duration: 449,
            bitrate: 900,
            format: "flac".into(),
            meta: "{\"rank\":810000}".into(),
            ..Default::default()
        };
        store.upsert_tracks(&[t1, t2]).unwrap();
        let (u, _) = store.authenticate_or_create_user("mixuser", "pw123").unwrap();
        let tok = store.create_session(u.id, "web").unwrap();

        let state = AppState {
            store: store.clone(),
            sources,
            engine: engine.clone(),
            taste,
            llm: None,
            now_playing: Arc::new(Mutex::new(Default::default())),
            new_releases_inflight: Arc::new(Mutex::new(Default::default())),
            lobbies: Default::default(),
        };
        let mut hdrs = HeaderMap::new();
        hdrs.insert(header::AUTHORIZATION, format!("Bearer {}", tok).parse().unwrap());

        let ev_res = record_event(
            State(state.clone()),
            hdrs.clone(),
            Json(EventReq {
                track_id: "loc:80s-1".into(),
                kind: "search_pick".into(),
                value: 1.0,
                client_id: String::new(),
                seq: 0,
            }),
        )
        .await;
        assert_eq!(ev_res.status(), StatusCode::OK);
        let events = store.user_recent_events(u.id, 10).unwrap();
        assert!(events.iter().any(|e| e.kind == "search_pick" && e.track_id == "loc:80s-1"));

        let resp = build_search_response(
            state.clone(),
            u.id,
            "80s".into(),
            "80s".into(),
            vec![],
            vec![],
            crate::taste::SearchMixContext::default(),
            vec![],
            vec![],
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body: Value = serde_json::from_slice(&to_bytes(resp.into_body(), 1024 * 128).await.unwrap()).unwrap();
        assert_eq!(body["instantMix"]["title"], "80s Hits Mix");
        assert_eq!(body["instantMix"]["isThemeMix"], true);
        assert!(body["instantMix"]["tracks"].as_array().unwrap().len() >= 2);
        assert!(!body["refinements"]["filterChips"].as_array().unwrap().is_empty());
        assert!(!body["refinements"]["querySuggestions"].as_array().unwrap().is_empty());
        assert!(body["tracks"][0]["isPopular"].as_bool().unwrap());

        // Ephemeral custom mix must NOT persist any playlist to SQLite
        let playlists = store.list_playlists(u.id, false).unwrap();
        assert!(playlists.is_empty(), "expected no saved playlists, found {}", playlists.len());

        // Not interested hides the song across endpoints, and reset restores all hide data
        let hide_res = mark_not_interested(
            State(state.clone()),
            hdrs.clone(),
            Json(NotInterestedReq {
                track_id: "loc:80s-2".into(),
            }),
        )
        .await;
        assert_eq!(hide_res.status(), StatusCode::OK);
        let hide_body: Value = serde_json::from_slice(&to_bytes(hide_res.into_body(), 1024 * 64).await.unwrap()).unwrap();
        assert_eq!(hide_body["hiddenCount"], 1);

        let listed_res = list_hidden(State(state.clone()), hdrs.clone()).await;
        let listed_body: Value = serde_json::from_slice(&to_bytes(listed_res.into_body(), 1024 * 64).await.unwrap()).unwrap();
        assert_eq!(listed_body["count"], 1);
        assert_eq!(listed_body["tracks"][0]["id"], "loc:80s-2");

        let unhide_res = unhide_track(
            State(state.clone()),
            Path("loc:80s-2".into()),
            hdrs.clone(),
        )
        .await;
        assert_eq!(unhide_res.status(), StatusCode::OK);
        let unhide_body: Value = serde_json::from_slice(&to_bytes(unhide_res.into_body(), 1024 * 64).await.unwrap()).unwrap();
        assert_eq!(unhide_body["removed"], 1);
        assert_eq!(unhide_body["count"], 0);

        let _ = mark_not_interested(
            State(state.clone()),
            hdrs.clone(),
            Json(NotInterestedReq {
                track_id: "loc:80s-2".into(),
            }),
        )
        .await;

        let reset_res = reset_hidden(State(state.clone()), hdrs).await;
        assert_eq!(reset_res.status(), StatusCode::OK);
        let reset_body: Value = serde_json::from_slice(&to_bytes(reset_res.into_body(), 1024 * 64).await.unwrap()).unwrap();
        assert_eq!(reset_body["cleared"], 1);
        assert_eq!(reset_body["count"], 0);
        assert!(store.hidden_tracks_list(u.id).unwrap().is_empty());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn escape_inline_closing_tag_handles_case_and_utf8() {
        let input = "const label = '🎵</ScRiPt>—</script>';";
        let escaped = escape_inline_closing_tag(input, b"</script", r"<\/script");
        assert_eq!(escaped, r"const label = '🎵<\/script>—<\/script>';");

        let clean = "console.log('ok');";
        assert!(matches!(
            escape_inline_closing_tag(clean, b"</script", r"<\/script"),
            Cow::Borrowed(_)
        ));
    }

    #[tokio::test]
    async fn serve_index_bundles_assets_and_respects_etag() {
        use axum::body::to_bytes;

        let res = serve_index(HeaderMap::new()).await;
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get(header::CACHE_CONTROL).unwrap(),
            "no-cache, must-revalidate"
        );
        let etag = res
            .headers()
            .get(header::ETAG)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        let bytes = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let html = std::str::from_utf8(&bytes).unwrap();
        assert!(!html.contains(CSS_LINK_TAG));
        assert!(!html.contains(JS_SCRIPT_TAG));
        assert!(html.contains("<style>\n"));
        assert!(html.contains(".settings-btn {"));
        assert!(html.contains(".mobile-brand {"));
        assert!(html.contains(".stats-page {"));
        assert!(html.contains(".loading-spinner {"));
        assert!(html.contains(".mixed-for-you-grid {"));
        assert!(html.contains("<script>\n"));
        assert!(html.contains("renderMixedForYouCard"));
        assert!(html.contains("openSettingsModal"));
        assert!(html.contains("renderStatsView"));
        assert!(html.contains("shareEntity"));
        assert!(html.contains("data-route=\"stats\""));
        // @username searches render people in the main results view, not a popover.
        assert!(html.contains("people-search-grid"));
        assert!(!html.contains("mention-dropdown"));
        // Social profiles expose auto-updating user radio, not manual recommendations.
        assert!(html.contains("radio-panel"));
        assert!(!html.contains("Recommend a track"));
        // Follow buttons flip in place without a page reload.
        assert!(html.contains("data-follow-ref"));
        assert!(html.contains("syncFollowState"));
        assert!(!html.contains("Audio Quality &amp; Codecs"));
        assert!(!html.contains("Sources &amp; Signals"));
        assert!(!html.contains("favoriteSourceLabel"));
        assert!(!html.contains("renderStatsDonutSvg"));
        assert!(!html.contains(".stats-grid-3"));
        assert!(!html.contains(".stats-signal-grid"));
        assert!(html.contains(&format!(
            r#"href="/web/favicon.svg?v={}""#,
            favicon_asset().short_hash
        )));
        assert!(html.contains(r#"<link rel="manifest" href="/manifest.webmanifest" />"#));
        assert!(html.contains(r#"href="/web/apple-touch-icon.png""#));
        assert!(html.contains(r#"href="/web/icon-192.png""#));
        assert!(html.contains(r#"href="/web/icon-512.png""#));

        // Exact ETag and weak Cloudflare ETag both return 304 Not Modified
        let mut req_headers = HeaderMap::new();
        req_headers.insert(header::IF_NONE_MATCH, etag.parse().unwrap());
        assert_eq!(serve_index(req_headers).await.status(), StatusCode::NOT_MODIFIED);

        let mut weak_headers = HeaderMap::new();
        weak_headers.insert(
            header::IF_NONE_MATCH,
            format!("W/{etag}").parse().unwrap(),
        );
        assert_eq!(serve_index(weak_headers).await.status(), StatusCode::NOT_MODIFIED);

        // Mismatched ETag returns full 200 OK
        let mut stale_headers = HeaderMap::new();
        stale_headers.insert(header::IF_NONE_MATCH, "\"000000000000\"".parse().unwrap());
        assert_eq!(serve_index(stale_headers).await.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn direct_asset_endpoints_disable_edge_caching_and_support_etag() {
        for (handler_name, res) in [
            ("css", serve_css(HeaderMap::new()).await),
            ("js", serve_js(HeaderMap::new()).await),
            ("favicon", serve_favicon(HeaderMap::new()).await),
            ("manifest", serve_manifest(HeaderMap::new()).await),
            ("sw", serve_sw(HeaderMap::new()).await),
            ("icon-192", serve_icon_192(HeaderMap::new()).await),
            ("icon-512", serve_icon_512(HeaderMap::new()).await),
            ("icon-maskable-512", serve_icon_maskable_512(HeaderMap::new()).await),
            ("apple-touch-icon", serve_apple_touch_icon(HeaderMap::new()).await),
        ] {
            assert_eq!(res.status(), StatusCode::OK, "{handler_name} status");
            assert_eq!(
                res.headers().get(header::CACHE_CONTROL).unwrap(),
                "no-cache, must-revalidate",
                "{handler_name} cache-control"
            );
            assert!(
                res.headers().get(header::ETAG).is_some(),
                "{handler_name} etag"
            );
        }

        let css_etag = css_asset().etag.clone();
        let mut hdrs = HeaderMap::new();
        hdrs.insert(header::IF_NONE_MATCH, css_etag.parse().unwrap());
        assert_eq!(serve_css(hdrs).await.status(), StatusCode::NOT_MODIFIED);
    }

    #[tokio::test]
    async fn pwa_manifest_service_worker_and_icons_are_valid() {
        use axum::body::to_bytes;

        let manifest_res = serve_manifest(HeaderMap::new()).await;
        assert_eq!(
            manifest_res.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/manifest+json; charset=utf-8"
        );
        let manifest_bytes = to_bytes(manifest_res.into_body(), 1024 * 64).await.unwrap();
        let manifest: Value = serde_json::from_slice(&manifest_bytes).unwrap();
        assert_eq!(manifest["name"], "Beatlab");
        assert_eq!(manifest["start_url"], "/");
        assert_eq!(manifest["scope"], "/");
        assert_eq!(manifest["display"], "standalone");
        let icons = manifest["icons"].as_array().unwrap();
        assert!(icons.iter().any(|i| i["sizes"] == "192x192" && i["type"] == "image/png" && i["purpose"] == "any"));
        assert!(icons.iter().any(|i| i["sizes"] == "512x512" && i["type"] == "image/png" && i["purpose"] == "any"));
        assert!(icons.iter().any(|i| i["sizes"] == "512x512" && i["type"] == "image/png" && i["purpose"] == "maskable"));

        let sw_res = serve_sw(HeaderMap::new()).await;
        assert_eq!(
            sw_res.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/javascript; charset=utf-8"
        );
        assert_eq!(sw_res.headers().get("service-worker-allowed").unwrap(), "/");
        let sw_etag = sw_res.headers().get(header::ETAG).unwrap().to_str().unwrap().to_string();
        let sw_bytes = to_bytes(sw_res.into_body(), 1024 * 64).await.unwrap();
        let sw_src = std::str::from_utf8(&sw_bytes).unwrap();
        assert!(!sw_src.contains("__BEATLAB_SW_VERSION__"));
        assert!(sw_src.contains("beatlab-pwa-"));

        let mut sw_hdrs = HeaderMap::new();
        sw_hdrs.insert(header::IF_NONE_MATCH, sw_etag.parse().unwrap());
        let sw_304 = serve_sw(sw_hdrs).await;
        assert_eq!(sw_304.status(), StatusCode::NOT_MODIFIED);
        assert_eq!(sw_304.headers().get("service-worker-allowed").unwrap(), "/");

        for icon_res in [
            serve_icon_192(HeaderMap::new()).await,
            serve_icon_512(HeaderMap::new()).await,
            serve_icon_maskable_512(HeaderMap::new()).await,
            serve_apple_touch_icon(HeaderMap::new()).await,
        ] {
            assert_eq!(icon_res.headers().get(header::CONTENT_TYPE).unwrap(), "image/png");
            let bytes = to_bytes(icon_res.into_body(), 1024 * 128).await.unwrap();
            assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
        }
    }

    #[tokio::test]
    async fn stats_endpoint_returns_isolated_detailed_user_stats() {
        use crate::engine::StreamEngine;
        use crate::sources::archive::ArchiveSource;
        use crate::sources::deezer::DeezerSource;
        use crate::sources::local::LocalSource;
        use crate::sources::soulseek::SoulseekSource;
        use crate::sources::ytm::YtmSource;
        use crate::sources::SourceRegistry;
        use crate::store::Store;
        use crate::taste::TasteEngine;
        use axum::body::to_bytes;
        use std::sync::{Arc, Mutex};

        let store = Store::open_memory().unwrap();
        let tmp = std::env::temp_dir().join(format!("beatlab-web-stats-{}", uuid::Uuid::new_v4()));
        let sources = SourceRegistry::new(
            DeezerSource::new(),
            YtmSource::new(None),
            ArchiveSource::new(),
            Arc::new(LocalSource::new(store.clone(), vec![])),
            SoulseekSource::new(store.clone(), tmp.join("slsk")),
        );
        let engine = Arc::new(StreamEngine::new(store.clone(), sources.clone(), &tmp, 1.0));
        let taste = Arc::new(TasteEngine::new(store.clone(), sources.clone()));

        let t1 = Track {
            id: "loc:st1".into(),
            source: "local".into(),
            title: "Time".into(),
            artist: "Pink Floyd".into(),
            album: "The Dark Side of the Moon".into(),
            genre: "Progressive Rock, Classic Rock".into(),
            moods: "late_night,rock_indie".into(),
            year: 1973,
            duration: 413,
            bitrate: 1050,
            format: "flac".into(),
            ..Default::default()
        };
        let t2 = Track {
            id: "dz:st2".into(),
            source: "deezer".into(),
            title: "Around the World".into(),
            artist: "Daft Punk".into(),
            album: "Homework".into(),
            genre: "Electronic, House".into(),
            year: 1997,
            duration: 429,
            bitrate: 256,
            format: "m4a".into(),
            ..Default::default()
        };
        let t3 = Track {
            id: "ytm:st3".into(),
            source: "ytm".into(),
            title: "Veridis Quo".into(),
            artist: "Daft Punk".into(),
            album: "Discovery".into(),
            genre: "Electronic".into(),
            year: 2001,
            duration: 345,
            bitrate: 256,
            format: "m4a".into(),
            ..Default::default()
        };
        store.upsert_tracks(&[t1.clone(), t2.clone(), t3.clone()]).unwrap();

        let (u1, _) = store.authenticate_or_create_user("stat_alice", "pw123").unwrap();
        let (u2, _) = store.authenticate_or_create_user("stat_bob", "pw123").unwrap();
        let tok1 = store.create_session(u1.id, "web").unwrap();
        let tok2 = store.create_session(u2.id, "web").unwrap();

        store.record_event(u1.id, &t1.id, "play", 1.0).unwrap();
        store.record_event(u1.id, &t1.id, "complete", 1.0).unwrap();
        store.record_event(u1.id, &t2.id, "play", 1.0).unwrap();
        store.record_event(u1.id, &t2.id, "skip", 1.0).unwrap();
        store.star_track(u1.id, &t1.id).unwrap();
        store.set_rating(u1.id, &t1.id, 5).unwrap();
        store.record_event(u2.id, &t3.id, "complete", 1.0).unwrap();

        let state = AppState {
            store: store.clone(),
            sources,
            engine: engine.clone(),
            taste,
            llm: None,
            now_playing: Arc::new(Mutex::new(Default::default())),
            new_releases_inflight: Arc::new(Mutex::new(Default::default())),
            lobbies: Default::default(),
        };

        // Unauthenticated fails
        let unauth = stats(
            State(state.clone()),
            Query(StatsQuery { range: None, tz: None }),
            HeaderMap::new(),
        )
        .await;
        assert_eq!(unauth.status(), StatusCode::UNAUTHORIZED);

        // Alice sees her 2 streams (1 completed, 1 skipped), not Bob's
        let mut h1 = HeaderMap::new();
        h1.insert(header::AUTHORIZATION, format!("Bearer {}", tok1).parse().unwrap());
        let res1 = stats(
            State(state.clone()),
            Query(StatsQuery {
                range: Some("30d".into()),
                tz: Some(-300),
            }),
            h1,
        )
        .await;
        assert_eq!(res1.status(), StatusCode::OK);
        let b1: Value = serde_json::from_slice(&to_bytes(res1.into_body(), 1024 * 256).await.unwrap()).unwrap();
        assert_eq!(b1["range"], "30d");
        assert_eq!(b1["tzOffsetMin"], -300);
        assert_eq!(b1["summary"]["totalStreams"], 2);
        assert_eq!(b1["summary"]["completedStreams"], 1);
        assert_eq!(b1["summary"]["skippedStreams"], 1);
        assert_eq!(b1["summary"]["uniqueArtists"], 2);
        assert_eq!(b1["library"]["likedTracks"], 1);
        assert_eq!(b1["library"]["ratedTracks"], 1);
        assert_eq!(b1["topTracks"][0]["track"]["id"], "loc:st1");
        assert!(b1["hourly"].as_array().unwrap().len() == 24);
        assert!(b1["weekdays"].as_array().unwrap().len() == 7);
        assert!(b1["moods"].as_array().unwrap().len() == 9);
        assert!(b1["timeline"]["points"].as_array().unwrap().len() == 30);
        assert!(b1.get("qualityTiers").is_none());
        assert!(b1.get("formats").is_none());
        assert!(b1.get("sources").is_none());
        assert!(b1["summary"].get("favoriteSource").is_none());
        assert!(b1["summary"].get("favoriteSourceLabel").is_none());
        assert!(b1["library"].get("signalsInRange").is_none());

        // Bob sees only his 1 completed stream (and no source/codec stats)
        let mut h2 = HeaderMap::new();
        h2.insert(header::AUTHORIZATION, format!("Bearer {}", tok2).parse().unwrap());
        let res2 = stats(
            State(state),
            Query(StatsQuery { range: Some("all".into()), tz: Some(0) }),
            h2,
        )
        .await;
        let b2_bytes = to_bytes(res2.into_body(), 1024 * 256).await.unwrap();
        let b2_str = std::str::from_utf8(&b2_bytes).unwrap();
        assert!(!b2_str.contains("YouTube Music"));
        let b2: Value = serde_json::from_slice(&b2_bytes).unwrap();
        assert_eq!(b2["summary"]["totalStreams"], 1);
        assert_eq!(b2["summary"]["completedStreams"], 1);
        assert_eq!(b2["library"]["likedTracks"], 0);
        assert!(b2.get("qualityTiers").is_none());
        assert!(b2.get("formats").is_none());
        assert!(b2.get("sources").is_none());
        assert!(b2["summary"].get("favoriteSource").is_none());
        assert!(b2["summary"].get("favoriteSourceLabel").is_none());
        assert!(b2["library"].get("signalsInRange").is_none());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn share_links_work_for_public_listeners_and_discord_embeds() {
        use crate::engine::StreamEngine;
        use crate::sources::archive::ArchiveSource;
        use crate::sources::deezer::DeezerSource;
        use crate::sources::local::LocalSource;
        use crate::sources::soulseek::SoulseekSource;
        use crate::sources::ytm::YtmSource;
        use crate::sources::SourceRegistry;
        use crate::store::Store;
        use crate::taste::TasteEngine;
        use axum::body::to_bytes;
        use std::sync::{Arc, Mutex};

        let store = Store::open_memory().unwrap();
        let tmp = std::env::temp_dir().join(format!("beatlab-web-share-{}", uuid::Uuid::new_v4()));
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

        let audio_file = tmp.join("shared.mp3");
        std::fs::write(&audio_file, b"ID3\x03\x00\x00\x00\x00\x00\x00FAKE_MP3_PAYLOAD").unwrap();

        let t1 = Track {
            id: "loc:sh1".into(),
            source: "local".into(),
            title: "Resonance </script><script>alert(1)</script>".into(),
            artist: "HOME".into(),
            album: "Odyssey".into(),
            genre: "Synthwave".into(),
            year: 2014,
            duration: 212,
            format: "mp3".into(),
            bitrate: 320,
            stream_url: format!("file:{}", audio_file.display()),
            ..Default::default()
        };
        let t2 = Track {
            id: "loc:sh2".into(),
            source: "local".into(),
            title: "Secret Unshared".into(),
            artist: "Hidden".into(),
            format: "mp3".into(),
            ..Default::default()
        };
        store.upsert_tracks(&[t1.clone(), t2.clone()]).unwrap();

        let (u1, _) = store.authenticate_or_create_user("sharer", "pw123").unwrap();
        let (u2, _) = store.authenticate_or_create_user("other", "pw123").unwrap();
        let tok1 = store.create_session(u1.id, "web").unwrap();
        let tok2 = store.create_session(u2.id, "web").unwrap();

        let pl = store
            .create_playlist(u1.id, "Private Chill", "Late night synth", false, &[t1.id.clone()])
            .unwrap();

        let state = AppState {
            store: store.clone(),
            sources,
            engine: engine.clone(),
            taste,
            llm: None,
            now_playing: Arc::new(Mutex::new(Default::default())),
            new_releases_inflight: Arc::new(Mutex::new(Default::default())),
            lobbies: Default::default(),
        };

        let mut h1 = HeaderMap::new();
        h1.insert(header::AUTHORIZATION, format!("Bearer {}", tok1).parse().unwrap());
        h1.insert(header::HOST, "music.example.com".parse().unwrap());
        h1.insert("x-forwarded-proto", "https".parse().unwrap());

        // 1. Track share creation & deduplication
        let sr1 = create_share(
            State(state.clone()),
            h1.clone(),
            Json(CreateShareReq {
                kind: "track".into(),
                target_id: t1.id.clone(),
                title: String::new(),
                subtitle: String::new(),
                track_ids: vec![],
            }),
        )
        .await;
        assert_eq!(sr1.status(), StatusCode::OK);
        let b1: Value = serde_json::from_slice(&to_bytes(sr1.into_body(), 1024 * 64).await.unwrap()).unwrap();
        let share_id = b1["id"].as_str().unwrap().to_string();
        assert_eq!(b1["url"], format!("https://music.example.com/s/{}", share_id));

        // 2. Public share page loads without auth and escapes XSS while providing Discord OG tags + signup CTA
        let mut bot_hdrs = HeaderMap::new();
        bot_hdrs.insert(header::HOST, "music.example.com".parse().unwrap());
        bot_hdrs.insert("x-forwarded-proto", "https".parse().unwrap());
        let page_res = public_share_page(State(state.clone()), Path(share_id.clone()), bot_hdrs.clone()).await;
        assert_eq!(page_res.status(), StatusCode::OK);
        let html = String::from_utf8(to_bytes(page_res.into_body(), 1024 * 256).await.unwrap().to_vec()).unwrap();
        assert!(html.contains("og:title"));
        assert!(html.contains("og:audio"));
        assert!(html.contains("Sign Up Free"));
        assert!(html.contains("Sign In"));
        assert!(!html.contains("<script>alert(1)</script>"));
        assert!(html.contains("&lt;/script&gt;"));
        assert!(!html.contains("backdrop-wrap"));
        assert!(!html.contains("radial-gradient"));
        assert!(!html.contains("linear-gradient"));
        assert!(!html.contains("Shared by sharer on Beatlab"));

        // 2b. oEmbed and iframe embed omit sharer username and gradients
        let oembed_res = public_share_oembed(State(state.clone()), Path(share_id.clone()), bot_hdrs.clone()).await;
        assert_eq!(oembed_res.status(), StatusCode::OK);
        let oembed_bytes = to_bytes(oembed_res.into_body(), 1024 * 64).await.unwrap();
        let oembed_val: Value = serde_json::from_slice(&oembed_bytes).unwrap();
        assert!(oembed_val.get("author_name").is_none());
        assert!(!std::str::from_utf8(&oembed_bytes).unwrap().contains("sharer"));

        let embed_res = public_share_embed_page(State(state.clone()), Path(share_id.clone()), bot_hdrs.clone()).await;
        assert_eq!(embed_res.status(), StatusCode::OK);
        let embed_html = String::from_utf8(to_bytes(embed_res.into_body(), 1024 * 64).await.unwrap().to_vec()).unwrap();
        assert!(!embed_html.contains("linear-gradient"));
        assert!(!embed_html.contains("sharer"));

        // 3. Cover endpoint always returns raster image (PNG/JPEG), never SVG
        let cov_res = public_share_cover(State(state.clone()), Path(share_id.clone())).await;
        assert_eq!(cov_res.status(), StatusCode::OK);
        let cov_ct = cov_res.headers().get(header::CONTENT_TYPE).unwrap().to_str().unwrap().to_string();
        assert!(!cov_ct.contains("svg"));

        // 4. Shared track streams publicly, while unshared track is rejected
        let ok_stream = public_share_stream_track(
            State(state.clone()),
            Path((share_id.clone(), t1.id.clone())),
            Query(StreamQuery { token: None, offset: None, compat: None }),
            HeaderMap::new(),
        )
        .await;
        assert_eq!(ok_stream.status(), StatusCode::OK);

        let denied_stream = public_share_stream_track(
            State(state.clone()),
            Path((share_id.clone(), t2.id.clone())),
            Query(StreamQuery { token: None, offset: None, compat: None }),
            HeaderMap::new(),
        )
        .await;
        assert_eq!(denied_stream.status(), StatusCode::NOT_FOUND);

        // 5. Private playlist cannot be shared by another user, but can be shared by owner
        let mut h2 = HeaderMap::new();
        h2.insert(header::AUTHORIZATION, format!("Bearer {}", tok2).parse().unwrap());
        let forbidden = create_share(
            State(state.clone()),
            h2,
            Json(CreateShareReq {
                kind: "playlist".into(),
                target_id: pl.id.clone(),
                title: String::new(),
                subtitle: String::new(),
                track_ids: vec![],
            }),
        )
        .await;
        assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);

        let pl_share = create_share(
            State(state.clone()),
            h1.clone(),
            Json(CreateShareReq {
                kind: "playlist".into(),
                target_id: pl.id.clone(),
                title: String::new(),
                subtitle: String::new(),
                track_ids: vec![],
            }),
        )
        .await;
        assert_eq!(pl_share.status(), StatusCode::OK);

        // 6. Ephemeral Instant Mix share succeeds without persisting to playlists table
        let mix_share = create_share(
            State(state.clone()),
            h1,
            Json(CreateShareReq {
                kind: "mix".into(),
                target_id: "mix:synthwave".into(),
                title: "Synthwave Instant Mix".into(),
                subtitle: "Curated vibes".into(),
                track_ids: vec![t1.id.clone()],
            }),
        )
        .await;
        assert_eq!(mix_share.status(), StatusCode::OK);
        assert_eq!(store.list_playlists(u1.id, false).unwrap().len(), 1);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn rapid_warm_event_and_queue_bursts_succeed_without_errors() {
        let store = crate::Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("burst_u", "pw").unwrap();
        let tok = store.create_session(u.id, "web").unwrap();

        let tmp = std::env::temp_dir().join(format!("beatlab_webapp_burst_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp).unwrap();
        let audio_file = tmp.join("burst.mp3");
        std::fs::write(&audio_file, vec![0u8; 8192]).unwrap();

        let mut tracks = Vec::new();
        for i in 0..12 {
            tracks.push(Track {
                id: format!("local:burst-track-{}", i),
                source: "local".into(),
                title: format!("Track {}", i),
                artist: "Artist".into(),
                album: "Album".into(),
                duration: 180,
                format: "mp3".into(),
                bitrate: 320,
                stream_url: format!("file:{}", audio_file.display()),
                ..Default::default()
            });
        }
        store.upsert_tracks(&tracks).unwrap();

        let sources = crate::sources::SourceRegistry::new(
            crate::sources::deezer::DeezerSource::new(),
            crate::sources::ytm::YtmSource::new(None),
            crate::sources::archive::ArchiveSource::new(),
            std::sync::Arc::new(crate::sources::local::LocalSource::new(store.clone(), vec![])),
            crate::sources::soulseek::SoulseekSource::new(store.clone(), tmp.join("slsk")),
        );
        let engine = std::sync::Arc::new(crate::engine::StreamEngine::new(
            store.clone(),
            sources.clone(),
            &tmp,
            1.0,
        ));
        let state = AppState {
            store: store.clone(),
            sources: sources.clone(),
            engine,
            taste: std::sync::Arc::new(crate::taste::TasteEngine::new(store, sources)),
            llm: None,
            now_playing: std::sync::Arc::new(std::sync::Mutex::new(HashMap::new())),
            new_releases_inflight: std::sync::Arc::new(std::sync::Mutex::new(HashMap::new())),
            lobbies: Default::default(),
        };

        let mut hdr = HeaderMap::new();
        hdr.insert(header::AUTHORIZATION, format!("Bearer {}", tok).parse().unwrap());

        let mut handles = Vec::new();
        for i in 0..30 {
            let st = state.clone();
            let h = hdr.clone();
            let tid = format!("local:burst-track-{}", i % 12);
            handles.push(tokio::spawn(async move {
                let r1 = warm_track(
                    State(st.clone()),
                    Path(tid.clone()),
                    Query(HashMap::new()),
                    h.clone(),
                )
                .await;
                let r2 = record_event(
                    State(st.clone()),
                    h.clone(),
                    Json(EventReq {
                        track_id: tid.clone(),
                        kind: "skip".into(),
                        value: 1.0,
                        client_id: String::new(),
                        seq: 0,
                    }),
                )
                .await;
                let r3 = save_queue(
                    State(st),
                    h,
                    Json(WebQueueSaveReq {
                        current_id: tid.clone(),
                        current_index: i % 12,
                        position_ms: 0,
                        track_ids: vec![tid],
                    }),
                )
                .await;
                assert_eq!(r1.status(), StatusCode::OK);
                assert_eq!(r2.status(), StatusCode::OK);
                assert_eq!(r3.status(), StatusCode::OK);
            }));
        }
        for handle in handles {
            handle.await.unwrap();
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn record_event_play_hydrates_and_returns_track_year() {
        use crate::engine::StreamEngine;
        use crate::sources::{
            archive::ArchiveSource, deezer::DeezerSource, local::LocalSource,
            soulseek::SoulseekSource, ytm::YtmSource, SourceRegistry,
        };
        use crate::store::Store;
        use axum::body::to_bytes;
        use std::sync::{Arc, Mutex};

        let tmp = std::env::temp_dir().join(format!(
            "beatlab_play_year_patch_test_{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let store = Store::open_memory().unwrap();
        let sources = SourceRegistry::new(
            DeezerSource::new(),
            YtmSource::new(None),
            ArchiveSource::new(),
            Arc::new(LocalSource::new(store.clone(), vec![])),
            SoulseekSource::new(store.clone(), tmp.clone()),
        );
        let engine = Arc::new(StreamEngine::new(store.clone(), sources.clone(), &tmp, 1.0));
        let taste = Arc::new(crate::taste::TasteEngine::new(store.clone(), sources.clone()));

        let mut alb = AlbumSummary::default();
        alb.id = "al:dz-55-today-is-a-beautiful-day".into();
        alb.name = "Today Is A Beautiful Day".into();
        alb.artist = "Supercell".into();
        alb.deezer_id = "424242".into();
        alb.year = 2011;
        alb.genre = "J-Pop".into();
        store.upsert_album_meta(&alb).unwrap();

        let track = Track {
            id: "dz:9001".into(),
            source: "deezer".into(),
            title: "Perfect Day".into(),
            artist: "Supercell".into(),
            album: "Today Is A Beautiful Day".into(),
            genre: String::new(),
            moods: String::new(),
            year: 0,
            duration: 286,
            bitrate: 256,
            format: "m4a".into(),
            meta: r#"{"deezer_track_id":9001,"deezer_album_id":424242,"deezer_artist_id":55}"#
                .into(),
            ..Default::default()
        };
        store.upsert_tracks(&[track]).unwrap();

        let (u, _) = store.authenticate_or_create_user("yearuser", "pw123").unwrap();
        let tok = store.create_session(u.id, "web").unwrap();
        let state = AppState {
            store: store.clone(),
            sources,
            engine,
            taste,
            llm: None,
            now_playing: Arc::new(Mutex::new(Default::default())),
            new_releases_inflight: Arc::new(Mutex::new(Default::default())),
            lobbies: Default::default(),
        };
        let mut hdrs = HeaderMap::new();
        hdrs.insert(header::AUTHORIZATION, format!("Bearer {}", tok).parse().unwrap());

        let resp = record_event(
            State(state),
            hdrs,
            Json(EventReq {
                track_id: "dz:9001".into(),
                kind: "play".into(),
                value: 1.0,
                client_id: String::new(),
                seq: 0,
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body: Value =
            serde_json::from_slice(&to_bytes(resp.into_body(), 1024 * 64).await.unwrap()).unwrap();
        assert_eq!(body["ok"], true);
        assert_eq!(body["track"]["id"], "dz:9001");
        assert_eq!(body["track"]["year"], 2011);
        assert_eq!(store.get_track("dz:9001").unwrap().unwrap().year, 2011);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn search_omits_instant_mix_and_on_demand_endpoint_builds_it_and_radio_parses_recent() {
        use crate::engine::StreamEngine;
        use crate::sources::{
            archive::ArchiveSource, deezer::DeezerSource, local::LocalSource,
            soulseek::SoulseekSource, ytm::YtmSource, SourceRegistry,
        };
        use crate::store::Store;
        use axum::body::to_bytes;
        use std::sync::{Arc, Mutex};

        let tmp = std::env::temp_dir().join(format!(
            "beatlab_ondemand_mix_test_{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let store = Store::open_memory().unwrap();
        let sources = SourceRegistry::new(
            DeezerSource::new(),
            YtmSource::new(None),
            ArchiveSource::new(),
            Arc::new(LocalSource::new(store.clone(), vec![])),
            SoulseekSource::new(store.clone(), tmp.clone()),
        );
        let engine = Arc::new(StreamEngine::new(store.clone(), sources.clone(), &tmp, 1.0));
        let taste = Arc::new(crate::taste::TasteEngine::new(store.clone(), sources.clone()));

        let t1 = Track {
            id: "loc:80s-a".into(),
            source: "local".into(),
            title: "Take On Me".into(),
            artist: "a-ha".into(),
            album: "Hunting High and Low".into(),
            genre: "Synthpop, 80s".into(),
            moods: "upbeat".into(),
            year: 1985,
            duration: 225,
            bitrate: 900,
            format: "flac".into(),
            meta: "{\"rank\":900000}".into(),
            ..Default::default()
        };
        let t2 = Track {
            id: "loc:80s-b".into(),
            source: "local".into(),
            title: "Blue Monday".into(),
            artist: "New Order".into(),
            album: "Power, Corruption & Lies".into(),
            genre: "New Wave, Electronic, 80s".into(),
            moods: "groove,night".into(),
            year: 1983,
            duration: 449,
            bitrate: 900,
            format: "flac".into(),
            meta: "{\"rank\":810000}".into(),
            ..Default::default()
        };
        store.upsert_tracks(&[t1, t2]).unwrap();
        let (u, _) = store.authenticate_or_create_user("ondemanduser", "pw123").unwrap();
        let tok = store.create_session(u.id, "web").unwrap();

        let state = AppState {
            store: store.clone(),
            sources,
            engine: engine.clone(),
            taste,
            llm: None,
            now_playing: Arc::new(Mutex::new(Default::default())),
            new_releases_inflight: Arc::new(Mutex::new(Default::default())),
            lobbies: Default::default(),
        };
        let mut hdrs = HeaderMap::new();
        hdrs.insert(header::AUTHORIZATION, format!("Bearer {}", tok).parse().unwrap());

        // Search response no longer auto-populates instantMix
        let search_resp = build_search_response_with_options(
            state.clone(),
            u.id,
            "80s".into(),
            "80s".into(),
            vec![],
            vec![],
            crate::taste::SearchMixContext::default(),
            vec![],
            vec![],
            false,
        )
        .await;
        assert_eq!(search_resp.status(), StatusCode::OK);
        let search_body: Value =
            serde_json::from_slice(&to_bytes(search_resp.into_body(), 1024 * 128).await.unwrap())
                .unwrap();
        assert!(search_body["instantMix"].is_null());
        assert!(!search_body["tracks"].as_array().unwrap().is_empty());

        // On-demand instant mix builder returns the mix without persisting any playlist
        let mix_resp = build_instant_mix_response(
            state.clone(),
            u.id,
            "80s".into(),
            vec![],
            vec![],
            crate::taste::SearchMixContext::default(),
        )
        .await;
        assert_eq!(mix_resp.status(), StatusCode::OK);
        let mix_body: Value =
            serde_json::from_slice(&to_bytes(mix_resp.into_body(), 1024 * 128).await.unwrap())
                .unwrap();
        assert_eq!(mix_body["instantMix"]["title"], "80s Hits Mix");
        assert!(mix_body["instantMix"]["tracks"].as_array().unwrap().len() >= 2);
        assert!(store.list_playlists(u.id, false).unwrap().is_empty());

        // Radio recent param parsing handles track IDs containing colons with @age
        let parsed = parse_recent_param(Some("dz:123@15, ytm:abc@42, loc:80s-a"));
        assert_eq!(
            parsed,
            vec![
                ("dz:123".to_string(), 15),
                ("ytm:abc".to_string(), 42),
                ("loc:80s-a".to_string(), 0),
            ]
        );

        // Explore mix endpoint serves pre-curated decade mixes cleanly
        let exp_tracks: Vec<Track> = (0..8)
            .map(|i| Track {
                id: format!("loc:exp80_{}", i),
                source: "local".into(),
                title: format!("80s Anthem {}", i),
                artist: format!("80s Artist {}", i),
                year: 1980 + i,
                duration: 220,
                bitrate: 900,
                format: "flac".into(),
                ..Default::default()
            })
            .collect();
        store.upsert_tracks(&exp_tracks).unwrap();
        let exp_ids: Vec<String> = exp_tracks.iter().map(|t| t.id.clone()).collect();
        state.taste.save_curated_query_tracks("80s hits", &exp_tracks);
        if let Ok(json_s) = serde_json::to_string(&serde_json::json!({
            "updated_at": chrono::Utc::now().timestamp(),
            "track_ids": exp_ids,
        })) {
            store.kv_set("curated:explore:80s_hits", &json_s);
        }
        let exp_resp = explore_mix(
            State(state.clone()),
            Path("80s_hits".into()),
            hdrs.clone(),
        )
        .await;
        assert_eq!(exp_resp.status(), StatusCode::OK);
        let exp_body: Value =
            serde_json::from_slice(&to_bytes(exp_resp.into_body(), 1024 * 128).await.unwrap())
                .unwrap();
        assert_eq!(exp_body["mix"]["key"], "80s_hits");
        assert_eq!(exp_body["mix"]["title"], "80s Hits");
        assert!(exp_body["tracks"].as_array().unwrap().len() >= 8);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn profile_picture_endpoints_upload_serve_and_clear() {
        use crate::engine::StreamEngine;
        use crate::sources::archive::ArchiveSource;
        use crate::sources::deezer::DeezerSource;
        use crate::sources::local::LocalSource;
        use crate::sources::soulseek::SoulseekSource;
        use crate::sources::ytm::YtmSource;
        use crate::sources::SourceRegistry;
        use crate::store::Store;
        use crate::taste::TasteEngine;
        use axum::body::to_bytes;
        use std::sync::{Arc, Mutex};

        let store = Store::open_memory().unwrap();
        let tmp = std::env::temp_dir().join(format!("beatlab-avatar-{}", uuid::Uuid::new_v4()));
        let sources = SourceRegistry::new(
            DeezerSource::new(),
            YtmSource::new(None),
            ArchiveSource::new(),
            Arc::new(LocalSource::new(store.clone(), vec![])),
            SoulseekSource::new(store.clone(), tmp.join("slsk")),
        );
        let engine = Arc::new(StreamEngine::new(store.clone(), sources.clone(), &tmp, 1.0));
        let taste = Arc::new(TasteEngine::new(store.clone(), sources.clone()));
        let state = AppState {
            store: store.clone(),
            sources,
            engine: engine.clone(),
            taste,
            llm: None,
            now_playing: Arc::new(Mutex::new(Default::default())),
            new_releases_inflight: Arc::new(Mutex::new(Default::default())),
            lobbies: Default::default(),
        };
        let (u, _) = store.authenticate_or_create_user("avataruser", "pw").unwrap();
        let tok = store.create_session(u.id, "web").unwrap();
        let mut hdrs = HeaderMap::new();
        hdrs.insert(
            header::AUTHORIZATION,
            format!("Bearer {}", tok).parse().unwrap(),
        );

        // Non-image bytes are rejected before touching storage.
        let bad =
            social_set_avatar(State(state.clone()), hdrs.clone(), Bytes::from_static(b"nope")).await;
        assert_eq!(bad.status(), StatusCode::BAD_REQUEST);

        // Upload a magic-valid JPEG and check the returned user exposes a URL.
        let jpeg = b"\xFF\xD8\xFF\xE0fake-jpeg-bytes";
        let up = social_set_avatar(
            State(state.clone()),
            hdrs.clone(),
            Bytes::from_static(jpeg),
        )
        .await;
        assert_eq!(up.status(), StatusCode::OK);
        let up_body: Value =
            serde_json::from_slice(&to_bytes(up.into_body(), 1024 * 64).await.unwrap()).unwrap();
        assert!(up_body["user"]["avatarUrl"]
            .as_str()
            .unwrap()
            .contains("/api/web/social/avatar/"));

        // The public endpoint serves the exact bytes with the sniffed MIME type.
        let img = social_avatar_image(State(state.clone()), Path(u.id.to_string())).await;
        assert_eq!(img.status(), StatusCode::OK);
        assert_eq!(img.headers().get(header::CONTENT_TYPE).unwrap(), "image/jpeg");
        let served = to_bytes(img.into_body(), 1024 * 64).await.unwrap();
        assert_eq!(&served[..], &jpeg[..]);

        // Unknown users and cleared avatars 404.
        assert_eq!(
            social_avatar_image(State(state.clone()), Path("999999".into()))
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
        let clr = social_clear_avatar(State(state.clone()), hdrs.clone()).await;
        assert_eq!(clr.status(), StatusCode::OK);
        assert_eq!(
            social_avatar_image(State(state.clone()), Path(u.id.to_string()))
                .await
                .status(),
            StatusCode::NOT_FOUND
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn artist_resolution_handles_non_ascii_romanized_and_rejects_stubs() {
        let mk = |id: &str, name: &str, albums: usize| ArtistSummary {
            id: format!("ar:dz:{id}"),
            name: name.into(),
            deezer_id: id.into(),
            album_count: albums,
            ..Default::default()
        };

        // Pure non-ASCII query with romanized #1 hit on Deezer ("幾田りら" -> "Lilas Ikuta").
        let ikuta = pick_artist_fallback(
            "幾田りら",
            vec![],
            Some(mk("106606492", "Lilas Ikuta", 31)),
            vec![mk("106606492", "Lilas Ikuta", 31)],
            false,
        )
        .unwrap();
        assert_eq!(ikuta.deezer_id, "106606492");

        // Non-ASCII query where exact-script match is a 0-album stub ("米津玄師" -> "Kenshi Yonezu").
        let yonezu = pick_artist_fallback(
            "米津玄師",
            vec![mk("7232650", "米津玄師", 0)],
            Some(mk("5289039", "Kenshi Yonezu", 37)),
            vec![mk("7232650", "米津玄師", 0), mk("5289039", "Kenshi Yonezu", 37)],
            false,
        )
        .unwrap();
        assert_eq!(yonezu.deezer_id, "5289039");

        // Non-ASCII query where exact-script match is a small stub (3 albums vs 48).
        let higedan = pick_artist_fallback(
            "Official髭男dism",
            vec![mk("77044752", "Official髭男dism", 3)],
            Some(mk("7579730", "OFFICIAL HIGE DANDISM", 48)),
            vec![
                mk("77044752", "Official髭男dism", 3),
                mk("7579730", "OFFICIAL HIGE DANDISM", 48),
            ],
            false,
        )
        .unwrap();
        assert_eq!(higedan.deezer_id, "7579730");

        // ASCII exact-case disambiguation ("LiSA" vs "LISA").
        let lisa = pick_artist_fallback(
            "LiSA",
            vec![mk("14559887", "LISA", 12), mk("418196", "LiSA", 35)],
            None,
            vec![mk("14559887", "LISA", 12), mk("418196", "LiSA", 35)],
            false,
        )
        .unwrap();
        assert_eq!(lisa.deezer_id, "418196");

        // Short ASCII query with NO exact-case match ("ASU" vs Romanian "Asu") must reject.
        assert!(pick_artist_fallback(
            "ASU",
            vec![mk("1666391", "Asu", 229)],
            None,
            vec![mk("1666391", "Asu", 229)],
            true,
        )
        .is_none());

        // Short ASCII query with multiple exact-case homonyms ("Eve") and 0 track hits must reject.
        assert!(pick_artist_fallback(
            "Eve",
            vec![mk("837", "Eve", 24), mk("69128312", "Eve", 39)],
            None,
            vec![mk("837", "Eve", 24), mk("69128312", "Eve", 39)],
            true,
        )
        .is_none());

        // Collab/CV names are not treated as solo fallback candidates.
        assert!(!is_solo_artist_name("Ano x Lilas Ikuta"));
        assert!(!is_solo_artist_name("要 楽奈(CV:青木陽菜)"));
        assert!(is_solo_artist_name("Hina Aoki"));
    }

    #[test]
    fn new_releases_freshness_respects_window_and_ttl() {
        let store = crate::store::Store::open_memory().unwrap();
        let now = Utc::now().timestamp();
        let win = crate::sources::deezer::NEW_RELEASE_WINDOW_DAYS;

        assert!(!new_releases_is_fresh(&store, 1, false));

        // Old window value is treated as stale so window changes refresh immediately.
        store.kv_set("newrel:fetched:1", &format!("{now}|{}", win - 1));
        assert!(!new_releases_is_fresh(&store, 1, true));

        // Matching window within TTL is fresh.
        store.kv_set("newrel:fetched:1", &format!("{now}|{win}"));
        assert!(new_releases_is_fresh(&store, 1, true));
        assert!(new_releases_is_fresh(&store, 1, false));

        // Empty shelf expires faster (15m) than a populated shelf (1h).
        store.kv_set("newrel:fetched:1", &format!("{}|{win}", now - 1200));
        assert!(new_releases_is_fresh(&store, 1, true));
        assert!(!new_releases_is_fresh(&store, 1, false));
    }

    #[test]
    fn itunes_artist_fallback_handles_bidirectional_cross_script_and_exact_case() {
        let mk = |id: u64, name: &str| crate::sources::itunes::ItunesArtist {
            id,
            name: name.into(),
            genre: "J-Pop".into(),
        };

        // Japanese query -> romanized iTunes artist ("幾田りら" -> "Lilas Ikuta").
        let ikuta = pick_itunes_artist_fallback(
            "幾田りら",
            vec![],
            Some(mk(1489581870, "Lilas Ikuta")),
            vec![mk(1489581870, "Lilas Ikuta")],
            false,
        )
        .unwrap();
        assert_eq!(ikuta.id, 1489581870);

        // Romanized query -> Japanese iTunes artist ("Hina Aoki" -> "青木陽菜").
        let aoki = pick_itunes_artist_fallback(
            "Hina Aoki",
            vec![],
            Some(mk(1696262485, "青木陽菜")),
            vec![mk(1696262485, "青木陽菜")],
            false,
        )
        .unwrap();
        assert_eq!(aoki.id, 1696262485);

        // Exact-case wins ("HACHI" vs "Hachi").
        let hachi = pick_itunes_artist_fallback(
            "HACHI",
            vec![mk(727267782, "Hachi"), mk(1523621267, "HACHI")],
            None,
            vec![mk(727267782, "Hachi"), mk(1523621267, "HACHI")],
            false,
        )
        .unwrap();
        assert_eq!(hachi.id, 1523621267);

        // Short ASCII query with no exact-case match ("ASU" vs "Asu") must reject.
        assert!(pick_itunes_artist_fallback(
            "ASU",
            vec![mk(888433, "Asu"), mk(1291573687, "Asu")],
            None,
            vec![mk(888433, "Asu"), mk(1291573687, "Asu")],
            true,
        )
        .is_none());

        // Short ASCII query with multiple exact-case homonyms and 0 track hits must reject.
        assert!(pick_itunes_artist_fallback(
            "Eve",
            vec![mk(108709, "Eve"), mk(1080967231, "Eve")],
            None,
            vec![mk(108709, "Eve"), mk(1080967231, "Eve")],
            true,
        )
        .is_none());
    }

    #[tokio::test]
    async fn now_playing_clears_on_close_expires_without_heartbeat_and_isolates_tabs() {
        use crate::engine::StreamEngine;
        use crate::sources::{
            archive::ArchiveSource, deezer::DeezerSource, local::LocalSource,
            soulseek::SoulseekSource, ytm::YtmSource, SourceRegistry,
        };
        use crate::store::Store;
        use std::sync::{Arc, Mutex};

        let tmp = std::env::temp_dir().join(format!(
            "beatlab_now_playing_test_{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let store = Store::open_memory().unwrap();
        let sources = SourceRegistry::new(
            DeezerSource::new(),
            YtmSource::new(None),
            ArchiveSource::new(),
            Arc::new(LocalSource::new(store.clone(), vec![])),
            SoulseekSource::new(store.clone(), tmp.clone()),
        );
        let engine = Arc::new(StreamEngine::new(store.clone(), sources.clone(), &tmp, 1.0));
        let taste = Arc::new(crate::taste::TasteEngine::new(store.clone(), sources.clone()));
        let state = AppState {
            store: store.clone(),
            sources,
            engine,
            taste,
            llm: None,
            now_playing: Arc::new(Mutex::new(Default::default())),
            new_releases_inflight: Arc::new(Mutex::new(Default::default())),
            lobbies: Default::default(),
        };

        let t1 = Track {
            id: "loc:np-1".into(),
            source: "local".into(),
            title: "Midnight City".into(),
            artist: "M83".into(),
            year: 2011,
            genre: "Synthpop".into(),
            duration: 243,
            ..Default::default()
        };
        let t2 = Track {
            id: "loc:np-2".into(),
            source: "local".into(),
            title: "Resonance".into(),
            artist: "HOME".into(),
            year: 2014,
            genre: "Synthwave".into(),
            duration: 212,
            ..Default::default()
        };
        store.upsert_tracks(&[t1.clone(), t2.clone()]).unwrap();

        let (u, _) = store.authenticate_or_create_user("npuser", "pw123").unwrap();
        let tok = store.create_session(u.id, "web").unwrap();
        let mut hdrs = HeaderMap::new();
        hdrs.insert(header::AUTHORIZATION, format!("Bearer {}", tok).parse().unwrap());

        // 1. Starting playback sets now_playing immediately.
        let r = record_event(
            State(state.clone()),
            hdrs.clone(),
            Json(EventReq {
                track_id: t1.id.clone(),
                kind: "play".into(),
                value: 1.0,
                client_id: "tab-a".into(),
                seq: 1,
            }),
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK);
        let now = Utc::now().timestamp();
        let active = active_now_playing(&state, u.id, now).expect("should be active");
        assert_eq!(active.0.id, t1.id);

        // 2. Without heartbeats (e.g. abrupt browser crash), now_playing expires after TTL.
        assert!(
            active_now_playing(&state, u.id, now + NOW_PLAYING_HEARTBEAT_TTL_SECS + 1).is_none(),
            "stale session must expire without heartbeats"
        );

        // 3. Re-report via heartbeat endpoint and keep alive past initial started_at + TTL.
        let r = update_now_playing(
            State(state.clone()),
            hdrs.clone(),
            Json(NowPlayingReq {
                playing: true,
                track_id: t1.id.clone(),
                client_id: "tab-a".into(),
                seq: 2,
            }),
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK);
        let started_at = active_now_playing(&state, u.id, Utc::now().timestamp())
            .expect("should be active")
            .1;
        // Heartbeat preserves started_at while advancing last_seen_at.
        upsert_now_playing(
            &state,
            u.id,
            t1.clone(),
            started_at + 40,
            Some("tab-a".into()),
            3,
        );
        let refreshed = active_now_playing(&state, u.id, started_at + 80)
            .expect("heartbeat should keep session alive");
        assert_eq!(refreshed.1, started_at);

        // 4. Abrupt window close (playing: false) clears immediately.
        let r = update_now_playing(
            State(state.clone()),
            hdrs.clone(),
            Json(NowPlayingReq {
                playing: false,
                track_id: String::new(),
                client_id: "tab-a".into(),
                seq: 4,
            }),
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK);
        assert!(active_now_playing(&state, u.id, Utc::now().timestamp()).is_none());

        // 5. Stale out-of-order play from the closed tab (seq 3 <= 4) cannot resurrect now_playing.
        let r = record_event(
            State(state.clone()),
            hdrs.clone(),
            Json(EventReq {
                track_id: t1.id.clone(),
                kind: "play".into(),
                value: 1.0,
                client_id: "tab-a".into(),
                seq: 3,
            }),
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK);
        assert!(
            active_now_playing(&state, u.id, Utc::now().timestamp()).is_none(),
            "stale play event from closed tab must not resurrect now_playing"
        );

        // 6. Another tab (tab-b) takes over playback; closing tab-a does NOT clear tab-b.
        let r = update_now_playing(
            State(state.clone()),
            hdrs.clone(),
            Json(NowPlayingReq {
                playing: true,
                track_id: t2.id.clone(),
                client_id: "tab-b".into(),
                seq: 1,
            }),
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK);
        let r = update_now_playing(
            State(state.clone()),
            hdrs.clone(),
            Json(NowPlayingReq {
                playing: false,
                track_id: String::new(),
                client_id: "tab-a".into(),
                seq: 5,
            }),
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK);
        let still_b = active_now_playing(&state, u.id, Utc::now().timestamp())
            .expect("closing tab-a must not clear tab-b");
        assert_eq!(still_b.0.id, t2.id);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn instant_mix_always_uses_llm_picked_songs_for_short_queries_when_available() {
        use crate::engine::StreamEngine;
        use crate::llm::{LlmConfig, LlmCurator};
        use crate::sources::{
            archive::ArchiveSource, deezer::DeezerSource, local::LocalSource,
            soulseek::SoulseekSource, ytm::YtmSource, SourceRegistry,
        };
        use crate::store::Store;
        use axum::body::to_bytes;
        use axum::routing::post;
        use axum::Router;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::{Arc, Mutex};

        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = calls.clone();
        let app = Router::new().route(
            "/v1/chat/completions",
            post(move || {
                calls_clone.fetch_add(1, Ordering::SeqCst);
                async move {
                    Json(json!({
                        "choices": [{
                            "finish_reason": "stop",
                            "message": {
                                "role": "assistant",
                                "content": json!({
                                    "title": "Daft Punk French Touch Radio",
                                    "subtitle": "Robotic funk, filtered disco hooks, and Parisian electro peers.",
                                    "tracks": [
                                        { "artist": "Daft Punk", "title": "Digital Love" },
                                        { "artist": "Justice", "title": "D.A.N.C.E." },
                                        { "artist": "Daft Punk", "title": "One More Time" },
                                        { "artist": "Modjo", "title": "Lady (Hear Me Tonight)" },
                                        { "artist": "Cassius", "title": "1999" },
                                        { "artist": "Stardust", "title": "Music Sounds Better With You" },
                                        { "artist": "Daft Punk", "title": "Voyager" },
                                        { "artist": "Phoenix", "title": "Lisztomania" },
                                        { "artist": "Air", "title": "Sexy Boy" },
                                        { "artist": "Kavinsky", "title": "Nightcall" },
                                        { "artist": "Daft Punk", "title": "Instant Crush" },
                                        { "artist": "Breakbot", "title": "Baby I'm Yours" }
                                    ],
                                    "seedQueries": ["french house", "daft punk radio"],
                                    "seedArtists": ["Daft Punk", "Justice", "Cassius", "Air"],
                                    "genres": ["French House", "Electronic"],
                                    "mood": "groove",
                                    "yearRange": [1997, 2015],
                                    "familiarity": "balanced"
                                }).to_string()
                            }
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

        let tmp = std::env::temp_dir().join(format!(
            "beatlab_llm_always_imix_test_{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let store = Store::open_memory().unwrap();
        let sources = SourceRegistry::new(
            DeezerSource::new(),
            YtmSource::new(None),
            ArchiveSource::new(),
            Arc::new(LocalSource::new(store.clone(), vec![])),
            SoulseekSource::new(store.clone(), tmp.clone()),
        );
        let engine = Arc::new(StreamEngine::new(store.clone(), sources.clone(), &tmp, 1.0));
        let taste = Arc::new(crate::taste::TasteEngine::new(store.clone(), sources.clone()));

        let catalog: Vec<(&str, &str, &str)> = vec![
            ("loc:dp1", "Daft Punk", "Digital Love"),
            ("loc:ju1", "Justice", "D.A.N.C.E."),
            ("loc:dp2", "Daft Punk", "One More Time"),
            ("loc:mo1", "Modjo", "Lady (Hear Me Tonight)"),
            ("loc:ca1", "Cassius", "1999"),
            ("loc:st1", "Stardust", "Music Sounds Better With You"),
            ("loc:dp3", "Daft Punk", "Voyager"),
            ("loc:ph1", "Phoenix", "Lisztomania"),
            ("loc:ai1", "Air", "Sexy Boy"),
            ("loc:ka1", "Kavinsky", "Nightcall"),
            ("loc:dp4", "Daft Punk", "Instant Crush"),
            ("loc:br1", "Breakbot", "Baby I'm Yours"),
        ];
        let tracks: Vec<Track> = catalog
            .iter()
            .map(|(id, artist, title)| Track {
                id: (*id).into(),
                source: "local".into(),
                title: (*title).into(),
                artist: (*artist).into(),
                album: "French Touch".into(),
                genre: "Electronic".into(),
                year: 2001,
                duration: 240,
                bitrate: 900,
                format: "flac".into(),
                ..Default::default()
            })
            .collect();
        store.upsert_tracks(&tracks).unwrap();

        let (u, _) = store.authenticate_or_create_user("llmuser", "pw123").unwrap();
        let tok = store.create_session(u.id, "web").unwrap();
        let cfg = LlmConfig {
            endpoint_base: format!("http://{addr}/v1"),
            completions_url: format!("http://{addr}/v1/chat/completions"),
            api_key: String::new(),
            model: "test-model".to_string(),
            refresh_hour: 4,
            temperature: 0.7,
            max_tokens: None,
            timeout_secs: 15,
            extra_system_prompt: None,
            extra_headers: vec![],
            reasoning_effort: None,
        };
        let state = AppState {
            store: store.clone(),
            sources,
            engine,
            taste,
            llm: Some(Arc::new(LlmCurator::new_with_config(cfg))),
            now_playing: Arc::new(Mutex::new(Default::default())),
            new_releases_inflight: Arc::new(Mutex::new(Default::default())),
            lobbies: Default::default(),
        };
        let mut hdrs = HeaderMap::new();
        hdrs.insert(header::AUTHORIZATION, format!("Bearer {}", tok).parse().unwrap());

        // Short artist query (not natural language) still routes through the LLM when available.
        assert!(!crate::llm::looks_like_natural_language_mix_prompt("Daft Punk"));
        let resp = instant_mix(
            State(state),
            Query(SearchQuery {
                q: Some("Daft Punk".into()),
                mode: None,
                phase: None,
            }),
            hdrs,
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body: Value =
            serde_json::from_slice(&to_bytes(resp.into_body(), 1024 * 256).await.unwrap()).unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(body["instantMix"]["aiInterpreted"], true);
        assert_eq!(body["instantMix"]["title"], "Daft Punk French Touch Radio");
        let got_ids: Vec<&str> = body["instantMix"]["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t["id"].as_str())
            .collect();
        let expected_ids: Vec<&str> = catalog.iter().map(|(id, _, _)| *id).collect();
        assert_eq!(got_ids, expected_ids);

        server.abort();
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
