use crate::auth::{
    assertion_credential_id, generate_recovery_codes, generate_totp_secret, hash_token,
    otpauth_uri, passkey_credential_id, qr_svg, random_token, verify_totp, MFA_WINDOW_SECS,
    RECOVERY_CODE_COUNT, RESTORE_WINDOW_SECS,
};
use crate::models::{
    AlbumSummary, ArtistSummary, Mood, PlayQueueState, Playlist, PublicUser, Track, User,
    UserProfile, UserTrackStats, ERR_RESTORE_REQUIRED, slug,
};
use crate::store::SessionMeta;
use crate::subsonic::{client_ip, user_agent, AppState};
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
const ICON_512_PNG: &[u8] = include_bytes!("web/icon-512.png");
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

fn filter_visible_tracks(state: &AppState, user_id: i64, tracks: Vec<Track>) -> Vec<Track> {
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

fn to_web_track_with_maps(
    t: &Track,
    starred: &HashSet<String>,
    stats: &HashMap<String, UserTrackStats>,
    ratings: &HashMap<String, i32>,
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
    }
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

    tracks
        .iter()
        .map(|t| {
            to_web_track_with_maps(
                t,
                &starred,
                &stats,
                &ratings,
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

fn cover_api_url(id: &str) -> String {
    format!("/api/web/cover/{}?v=2", urlencoding::encode(id))
}

fn extract_web_token(headers: &HeaderMap, query_token: Option<&str>) -> Option<String> {
    if let Some(tok) = query_token.map(|s| s.trim()).filter(|s| !s.is_empty()) {
        return Some(tok.to_string());
    }
    if let Some(raw) = headers
        .get(header::AUTHORIZATION)
        .or_else(|| headers.get("x-nd-authorization"))
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

fn resolve_web_user(state: &AppState, headers: &HeaderMap, query_token: Option<&str>) -> Option<User> {
    let tok = extract_web_token(headers, query_token)?;
    if let Ok(Some(u)) = state.store.get_session_user(&tok) {
        return Some(u);
    }
    state.store.get_user_by_api_key(&tok).ok().flatten()
}

fn require_web_user(state: &AppState, headers: &HeaderMap) -> Result<User, Response> {
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

fn json_res(val: Value) -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        val.to_string(),
    )
        .into_response()
}

fn err_res(status: StatusCode, msg: &str) -> Response {
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
///
/// Only interactive web-player sessions qualify. The API key (and Navidrome
/// sessions minted from it) is a credential users paste into third-party
/// music apps, so it must never be able to manage the account itself —
/// otherwise anyone holding it could e.g. enrol their own passkey and take
/// over the account.
fn require_web_user_with_token(state: &AppState, headers: &HeaderMap) -> Result<(User, String), Response> {
    let token = extract_web_token(headers, None).ok_or_else(unauthorized)?;
    let user = resolve_web_user(state, headers, None).ok_or_else(unauthorized)?;
    if state.store.session_kind(&token).ok().flatten().as_deref() != Some("web") {
        return Err(err_res(
            StatusCode::FORBIDDEN,
            "Sign in to the Beatlab web player to manage your account",
        ));
    }
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
/// deletion, API-key rotation). A fresh session-level MFA check or a code in
/// the request satisfies the gate.
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
        client: if kind == "nd" { "Navidrome".to_string() } else { String::new() },
    };
    state
        .store
        .create_session_with_meta(user.id, kind, &meta)
        .map_err(|e| err_res(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))
}

fn user_json(state: &AppState, user: &User) -> Value {
    let api_key = state.store.get_or_create_api_key(user.id).unwrap_or_default();
    let two_factor_enabled = state.store.is_2fa_enabled(user.id).unwrap_or(false);
    let (recovery_total, recovery_used) = state.store.recovery_code_stats(user.id).unwrap_or((0, 0));
    json!({
        "id": user.id,
        "username": user.username,
        "apiKey": api_key,
        "created": user.created,
        "avatarUrl": self_avatar_url(state, user.id),
        "twoFactorEnabled": two_factor_enabled,
        "isAdmin": crate::admin::is_admin_username(&user.username),
        "totpEnabled": state.store.get_totp(user.id).ok().flatten().map(|t| t.enabled).unwrap_or(false),
        "passkeys": state.store.passkey_count(user.id).unwrap_or(0),
        "recoveryCodesLeft": recovery_total.saturating_sub(recovery_used),
    })
}

#[derive(Deserialize)]
pub struct AuthReq {
    pub username: String,
    pub password: String,
}

#[derive(Deserialize)]
pub struct SignupReq {
    pub username: String,
    pub password: String,
    #[serde(rename = "confirmPassword")]
    pub confirm_password: String,
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

pub async fn signin(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<AuthReq>,
) -> Response {
    let rate_keys = crate::auth::rate_keys(
        "signin",
        &req.username,
        &crate::subsonic::rate_limit_ip(&headers),
    );
    if let Some(wait) = crate::auth::throttled_for(&rate_keys) {
        return too_many_attempts(wait);
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
        Err(_) => crate::auth::record_failure(&rate_keys),
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
                "kind": s.kind,
                "created": s.created,
                "lastSeen": s.last_seen,
                "userAgent": s.user_agent,
                "ip": s.ip,
                "client": s.client,
                "revoked": s.revoked,
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

/// Allow a previously revoked Subsonic client session again.
pub async fn allow_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<RevokeSessionReq>,
) -> Response {
    let user = match require_account_session(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let allowed = state
        .store
        .allow_session_for_user(&req.id, user.id)
        .unwrap_or(false);
    if allowed {
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

pub async fn regenerate_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let (user, _) = match require_sensitive_auth(&state, &headers, optional_code(&body).as_deref()) {
        Ok(value) => value,
        Err(r) => return r,
    };
    match state.store.regenerate_api_key(user.id) {
        Ok(api_key) => {
            state.store.record_security_event(
                user.id,
                "api_key_rotated",
                "API key regenerated",
                &client_ip(&headers),
            );
            json_res(json!({ "apiKey": api_key }))
        }
        Err(e) => err_res(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
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

/// Builds the personalised "New Releases" feed: recent albums from artists the
/// user follows or listens to. Fresh releases are pulled from Deezer in
/// parallel (only releases from the last ~month); a short timeout keeps the
/// home tab responsive.
async fn collect_new_releases(state: &AppState, uid: i64, limit: usize) -> Vec<AlbumSummary> {
    let artists = state
        .store
        .relevant_artists_for_releases(uid, 16)
        .unwrap_or_default();
    if artists.is_empty() {
        return Vec::new();
    }

    // Each artist is fetched in its own task. We keep whatever finishes before
    // the deadline; stragglers keep running detached so their responses land in
    // the Deezer JSON cache and the next Home visit gets them instantly. (A
    // single all-or-nothing timeout used to discard every result — and cancel
    // the requests before they could be cached — whenever any one was slow.)
    let handles: Vec<_> = artists
        .iter()
        .map(|artist| {
            let dz = state.sources.deezer.clone();
            let store = state.store.clone();
            let mut id = artist.deezer_id.clone();
            let name = artist.name.clone();
            tokio::spawn(async move {
                if id.is_empty() {
                    match resolve_deezer_artist_by_name(&dz, &store, &name).await {
                        Some(found) => id = found,
                        None => return Vec::new(),
                    }
                }
                dz.fetch_artist_releases(&id, &name, 4).await
            })
        })
        .collect();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
    let mut batches: Vec<Vec<AlbumSummary>> = Vec::with_capacity(handles.len());
    for handle in handles {
        if let Ok(Ok(batch)) = tokio::time::timeout_at(deadline, handle).await {
            batches.push(batch);
        }
    }

    let mut albums: Vec<AlbumSummary> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for batch in batches {
        for album in batch {
            if seen.insert(album.id.clone()) {
                albums.push(album);
            }
        }
    }

    albums.sort_by(|a, b| b.created.cmp(&a.created).then_with(|| a.name.cmp(&b.name)));
    albums.truncate(limit);
    albums
}

/// Finds an artist's Deezer ID from their name (for artists known only via
/// YouTube Music or by name). Only an exact, case-insensitive name match is
/// accepted so we never show releases from a different artist. Hits and misses
/// are remembered so each name is searched at most once a day.
async fn resolve_deezer_artist_by_name(
    dz: &crate::sources::deezer::DeezerSource,
    store: &crate::store::Store,
    name: &str,
) -> Option<String> {
    let name = name.trim();
    let key_name = slug(name);
    if key_name.is_empty() {
        return None;
    }
    let cache_key = format!("dzartist:byname:{key_name}");
    let now = Utc::now().timestamp();
    if let Some(raw) = store.kv_get(&cache_key) {
        if let Some((ts, id)) = raw.split_once('|') {
            if !id.is_empty() {
                return Some(id.to_string());
            }
            // Cached miss: retry after a day.
            if ts.parse::<i64>().map_or(false, |t| now - t < 86_400) {
                return None;
            }
        }
    }
    let wanted = name.to_lowercase();
    let found = dz
        .search_artists(name, 5)
        .await
        .into_iter()
        .find(|a| a.name.trim().to_lowercase() == wanted || slug(&a.name) == key_name);
    match found {
        Some(a) => {
            let _ = store.upsert_artist_meta(&a);
            store.kv_set(&cache_key, &format!("{now}|{}", a.deezer_id));
            Some(a.deezer_id)
        }
        None => {
            store.kv_set(&cache_key, &format!("{now}|"));
            None
        }
    }
}

/// Builds the new-releases shelf and advances the per-account "seen" marker so
/// the NEW badge only highlights arrivals since the last Home visit.
async fn build_new_releases(
    state: &AppState,
    uid: i64,
    saved: &HashSet<String>,
) -> (Vec<Value>, usize, i64) {
    let new_releases = collect_new_releases(state, uid, 8).await;
    for a in &new_releases {
        let _ = state.store.upsert_album_meta(a);
    }

    let seen_key = format!("newrel:seen:{uid}");
    let previous_ids: Option<Vec<String>> = state
        .store
        .kv_get(&seen_key)
        .and_then(|raw| serde_json::from_str::<Vec<String>>(&raw).ok());
    let previous_set: Option<HashSet<String>> = previous_ids
        .as_ref()
        .map(|ids| ids.iter().cloned().collect());
    let unseen: HashSet<String> = match &previous_set {
        Some(prev) => new_releases
            .iter()
            .filter(|a| !prev.contains(&a.id))
            .map(|a| a.id.clone())
            .collect(),
        // First visit: everything is "already seen" so we don't claim 12 new drops.
        None => HashSet::new(),
    };
    let items: Vec<Value> = new_releases
        .iter()
        .map(|a| {
            let mut value = album_json(a, saved.contains(&a.id));
            if let Value::Object(map) = &mut value {
                map.insert("isNew".to_string(), json!(unseen.contains(&a.id)));
            }
            value
        })
        .collect();

    let mut known = previous_ids.unwrap_or_default();
    let mut known_set: HashSet<String> = known.iter().cloned().collect();
    for a in &new_releases {
        if known_set.insert(a.id.clone()) {
            known.push(a.id.clone());
        }
    }
    if known.len() > 1000 {
        known = known.split_off(known.len() - 1000);
    }
    state.store.kv_set(
        &seen_key,
        &serde_json::to_string(&known).unwrap_or_else(|_| "[]".to_string()),
    );

    (items, unseen.len(), Utc::now().timestamp())
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
    let is_fresh = !state.store.has_listening_history(uid).unwrap_or(false);

    // Fetch external charts first so fresh accounts immediately have popular
    // tracks, albums, and artists available before auto-mixes & picks are built.
    // If Deezer is slow (cold cache), fall back to the charts stored from earlier fetches.
    let st = state.clone();
    let (chart_tracks, chart_albums, chart_artists) = bounded_upstream(async move {
        let (t, al, ar) = fetch_home_charts(&st).await;
        record_charts(&st, &t, &al, &ar);
        (t, al, ar)
    })
    .await
    .unwrap_or_default();

    // Ensure auto-mixes exist and refresh cleanly whenever user feedback/playlists changed them.
    if state.store.count_tracks().unwrap_or(0) > 0 {
        // Mixed-for-you first: rebuilding generic auto mixes clears the dirty flag.
        state.taste.refresh_mixed_for_you_if_stale_async(uid).await;
        state.taste.refresh_auto_playlists_if_stale_async(uid, 900).await;
    }
    let playlists = state.store.list_playlists(uid, false).unwrap_or_default();

    let auto_mixes: Vec<Value> = playlists
        .into_iter()
        .filter(|p| p.is_auto && !p.kind.starts_with("mixed_for_you:"))
        .map(|p| playlist_summary_for_user_json(&state, uid, &p))
        .collect();

    let quick_picks = state.taste.quick_picks(uid, 36);
    let recent: Vec<Track> = filter_visible_tracks(
        &state,
        uid,
        state
            .store
            .user_recent_tracks(uid, 12, 0)
            .unwrap_or_default()
            .into_iter()
            .map(|(t, _)| t)
            .collect(),
    );

    let trending_tracks = filter_visible_tracks(
        &state,
        uid,
        if !chart_tracks.is_empty() {
            chart_tracks
        } else {
            let pop = state.store.popular_chart_tracks(16).unwrap_or_default();
            if !pop.is_empty() || is_fresh {
                pop
            } else {
                state.store.newest_tracks(16, 0).unwrap_or_default()
            }
        },
    );
    let albums = if !chart_albums.is_empty() {
        chart_albums
    } else {
        let pop_albums = state.store.popular_chart_albums(12);
        if !pop_albums.is_empty() || is_fresh {
            pop_albums
        } else {
            state
                .store
                .build_album_summaries(uid)
                .unwrap_or_default()
                .into_iter()
                .take(12)
                .collect()
        }
    };
    let artists = resolve_featured_artists(&state, chart_artists, 12, is_fresh);

    let saved = state.store.saved_album_ids(uid).unwrap_or_default();
    let followed = state.store.followed_artist_ids(uid).unwrap_or_default();

    let (new_releases_json, new_release_unseen, new_release_checked_at) =
        build_new_releases(&state, uid, &saved).await;

    let moods: Vec<Value> = Mood::ALL
        .iter()
        .map(|m| {
            let (c1, c2) = m.gradient();
            let (track_count, covers) = state.taste.curated_mood_preview_for_user(uid, *m);
            json!({
                "slug": m.slug(),
                "label": m.label(),
                "subtitle": m.subtitle(),
                "c1": c1,
                "c2": c2,
                "trackCount": track_count,
                "covers": covers,
            })
        })
        .collect();

    let related_tracks = filter_visible_tracks(&state, uid, state.taste.related_tracks_for_user(uid, 12));
    let recommended_artists = state.taste.recommended_artists_for_user(uid, 12);

    json_res(json!({
        "quickPicks": to_web_tracks(&state, uid, &quick_picks),
        "newReleases": new_releases_json,
        "newReleaseUnseen": new_release_unseen,
        "newReleaseCheckedAt": new_release_checked_at,
        "recent": to_web_tracks(&state, uid, &recent),
        "mixedForYou": json!(state.taste.mixed_for_you_cards(uid)),
        "madeForYou": auto_mixes,
        "relatedTracks": to_web_tracks(&state, uid, &related_tracks),
        "recommendedArtists": recommended_artists.iter().map(|a| artist_json(a, followed.contains(&a.id))).collect::<Vec<_>>(),
        "trending": to_web_tracks(&state, uid, &trending_tracks),
        "albums": albums.iter().map(|a| album_json(a, saved.contains(&a.id))).collect::<Vec<_>>(),
        "artists": artists.iter().map(|a| artist_json(a, followed.contains(&a.id))).collect::<Vec<_>>(),
        "moods": moods,
    }))
}

pub async fn explore(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let uid = user.id;
    let is_fresh = !state.store.has_listening_history(uid).unwrap_or(false);
    let st = state.clone();
    let (dz_genres, chart_tracks, chart_albums, chart_artists) = bounded_upstream(async move {
        let (g, t, al, ar) = fetch_explore_charts(&st).await;
        record_charts(&st, &t, &al, &ar);
        (g, t, al, ar)
    })
    .await
    .unwrap_or_default();

    let moods: Vec<Value> = Mood::ALL
        .iter()
        .map(|m| {
            let (c1, c2) = m.gradient();
            let (track_count, covers) = state.taste.curated_mood_preview_for_user(uid, *m);
            json!({
                "slug": m.slug(),
                "label": m.label(),
                "subtitle": m.subtitle(),
                "c1": c1,
                "c2": c2,
                "trackCount": track_count,
                "covers": covers,
            })
        })
        .collect();

    let mut decades: Vec<Value> = Vec::new();
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
    let recommended_artists = state.taste.recommended_artists_for_user(uid, 14);

    let ready_mood_cards = moods
        .iter()
        .filter(|m| m["trackCount"].as_u64().unwrap_or(0) > 0)
        .count();
    let mut curator = state.taste.curator_status_snapshot();
    if let Some(obj) = curator.as_object_mut() {
        let cur_ready = obj.get("readyMoods").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        if ready_mood_cards > cur_ready {
            obj.insert("readyMoods".to_string(), json!(ready_mood_cards));
        }
    }

    json_res(json!({
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
    let Some(spec) = crate::taste::find_explore_mix_spec(&key) else {
        return err_res(StatusCode::NOT_FOUND, "Explore mix not found");
    };
    let raw_tracks = state
        .taste
        .get_curated_explore_mix_tracks(user.id, spec, 35)
        .await;
    let tracks = filter_visible_tracks(&state, user.id, raw_tracks);
    let total_duration: i32 = tracks.iter().map(|t| t.duration.max(0)).sum();
    let match_ids = personal_match_ids(&state, user.id);
    json_res(json!({
        "mix": {
            "key": spec.key,
            "kind": spec.kind,
            "title": spec.title,
            "subtitle": spec.subtitle,
            "query": spec.query,
            "c1": spec.c1,
            "c2": spec.c2,
            "totalDuration": total_duration,
        },
        "tracks": to_web_tracks_with_taste(&state, user.id, &tracks, &match_ids),
    }))
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
    let Some(mood) = Mood::from_slug(&slug_str) else {
        return err_res(StatusCode::NOT_FOUND, "Mood not found");
    };
    let tracks = state.taste.mood_mix_tracks(user.id, mood, 35).await;
    let (c1, c2) = mood.gradient();
    let match_ids = personal_match_ids(&state, user.id);
    json_res(json!({
        "mood": {
            "slug": mood.slug(),
            "label": mood.label(),
            "subtitle": mood.subtitle(),
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
            tokio::spawn(async move {
                let _ = tokio::task::spawn_blocking(move || {
                    let _ = store.upsert_tracks(&to_save);
                })
                .await;
            });
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
    let Some((album, tracks)) = state
        .taste
        .get_or_hydrate_album_with_hints(
            &id,
            q.name.as_deref(),
            q.artist.as_deref(),
            q.track.as_deref(),
        )
        .await
    else {
        return err_res(StatusCode::NOT_FOUND, "Album not found");
    };
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

    json_res(stats_payload(computed, &starred, &track_stats, &ratings, true))
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

    json_res(stats_payload(computed, &starred, &track_stats, &ratings, is_self))
}

fn stats_payload(
    mut computed: crate::stats::ComputedStats,
    starred: &HashSet<String>,
    track_stats: &HashMap<String, UserTrackStats>,
    ratings: &HashMap<String, i32>,
    is_self: bool,
) -> Value {
    if !is_self {
        computed.recent_events.clear();
        computed.library.hidden_tracks = 0;
    }
    let wrap_track = |t: &Track| to_web_track_with_maps(t, starred, track_stats, ratings, false);

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
    if let Ok(Some(t)) = state.store.get_track(&req.track_id) {
        if t.genre.trim().is_empty() || t.year <= 0 {
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
        if req.kind == "play" {
            let mut np = state.now_playing.lock().unwrap_or_else(|e| e.into_inner());
            np.insert(user.id, (t, Utc::now().timestamp(), user.username));
        }
    }
    json_res(json!({ "ok": true }))
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

    let next = if let Some(pid) = q.playlist.as_deref().filter(|s| !s.trim().is_empty()) {
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
    json_res(json!({ "ok": true }))
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

fn request_origin(headers: &HeaderMap) -> String {
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

fn is_link_preview_bot(headers: &HeaderMap) -> bool {
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

fn playlist_summary_for_user_json(state: &AppState, user_id: i64, p: &Playlist) -> Value {
    if !p.is_auto || user_id <= 0 {
        return playlist_summary_json(p);
    }
    let (hidden_ids, hidden_keys) = state.store.hidden_track_sets(user_id).unwrap_or_default();
    if hidden_ids.is_empty() && hidden_keys.is_empty() {
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
    let store = state.store.clone();
    let stats = tokio::task::spawn_blocking(move || {
        crate::stats::build_public_stats(&store, &target_user, Utc::now().timestamp())
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
    json_res(json!({
        "user": public_user_json(&target),
        "profile": profile_json(&profile),
        "stats": serde_json::to_value(&stats).unwrap_or(Value::Null),
        "radioSaved": radio_saved,
        "playlists": playlists,
    }))
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
            now_playing: Arc::new(Mutex::new(Default::default())),
        }
    }

    fn bearer(token: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(header::AUTHORIZATION, format!("Bearer {token}").parse().unwrap());
        h
    }

    #[tokio::test]
    async fn api_keys_and_client_sessions_cannot_manage_the_account() {
        let state = account_test_state();
        let (u, _) = state.store.authenticate_or_create_user("owner", "pw123").unwrap();
        let api_key = state.store.get_or_create_api_key(u.id).unwrap();
        let nd_token = state.store.create_session(u.id, "nd").unwrap();
        let web_token = state.store.create_session(u.id, "web").unwrap();

        for tok in [&api_key, &nd_token] {
            let res = regenerate_api_key(State(state.clone()), bearer(tok), Bytes::new()).await;
            assert_eq!(res.status(), StatusCode::FORBIDDEN);
            let res = passkey_register_options(State(state.clone()), bearer(tok), Bytes::new()).await;
            assert_eq!(res.status(), StatusCode::FORBIDDEN);
            let res = list_sessions(State(state.clone()), bearer(tok)).await;
            assert_eq!(res.status(), StatusCode::FORBIDDEN);
        }
        // The API key still works for ordinary player endpoints.
        assert_eq!(
            sources_info(State(state.clone()), bearer(&api_key)).await.status(),
            StatusCode::OK
        );
        // A real web session can manage the account.
        let res = regenerate_api_key(State(state.clone()), bearer(&web_token), Bytes::new()).await;
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn repeated_failed_signins_are_throttled_and_passwords_are_hashed() {
        let state = account_test_state();
        let (u, _) = state.store.authenticate_or_create_user("throttled-user", "right-pass").unwrap();
        assert!(crate::auth::is_password_hash(&state.store.get_user_by_id(u.id).unwrap().unwrap().password));

        let attempt = |pw: &str| AuthReq { username: "throttled-user".into(), password: pw.into() };
        for _ in 0..crate::auth::MAX_FAILURES_PER_ACCOUNT {
            let res = signin(State(state.clone()), HeaderMap::new(), Json(attempt("wrong"))).await;
            assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        }
        // Even the right password is refused while the account is throttled.
        let res = signin(State(state.clone()), HeaderMap::new(), Json(attempt("right-pass"))).await;
        assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
        assert!(res.headers().contains_key(header::RETRY_AFTER));
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
        let nd = state.store.create_session(u.id, "nd").unwrap();

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
        assert!(state.store.get_session_user(&nd).unwrap().is_none());
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
            now_playing: Arc::new(Mutex::new(Default::default())),
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
            now_playing: Arc::new(Mutex::new(Default::default())),
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
            now_playing: Arc::new(Mutex::new(Default::default())),
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
            now_playing: Arc::new(Mutex::new(Default::default())),
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
            now_playing: std::sync::Arc::new(std::sync::Mutex::new(HashMap::new())),
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
            now_playing: Arc::new(Mutex::new(Default::default())),
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
            now_playing: Arc::new(Mutex::new(Default::default())),
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
}
