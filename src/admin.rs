//! Administrator panel: gated by the `ADMIN_USERNAME` environment variable.
//!
//! There is no separate admin password or role table — the named account(s)
//! become administrators once signed in through the normal web flow.

use crate::auth::{hash_token, random_bytes};
use crate::models::User;
use crate::state::{client_ip, AppState};
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::OnceLock;

const ADMIN_HTML: &str = include_str!("web/admin.html");

static ADMINS: OnceLock<Vec<String>> = OnceLock::new();

/// Call once at startup with the raw `ADMIN_USERNAME` value. Comma-separated
/// names are supported so a small team can share admin access.
pub fn init_admins(raw: &str) {
    let names: Vec<String> = raw
        .split(',')
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    let _ = ADMINS.set(names);
}

pub fn admin_names() -> &'static [String] {
    ADMINS.get().map(Vec::as_slice).unwrap_or(&[])
}

/// Error for open sign-up attempts that use a name listed in `ADMIN_USERNAME`.
/// Without this, anyone could register an admin name that has no account yet
/// and gain the admin panel. Admin accounts are created at startup from
/// `ADMIN_PASSWORD` instead (see `bootstrap_admin_accounts`).
pub const RESERVED_ADMIN_NAME: &str = "That username is reserved";

/// Creates any `ADMIN_USERNAME` account that doesn't exist yet, using the
/// `ADMIN_PASSWORD` environment variable. Without a password the name stays
/// reserved (sign-up refuses it) and a warning explains how to create it.
pub fn bootstrap_admin_accounts(store: &crate::store::Store, admin_password: Option<&str>) {
    for name in admin_names() {
        match store.get_user_by_name(name) {
            Ok(Some(_)) => {}
            Ok(None) => match admin_password.filter(|p| !p.is_empty()) {
                Some(pw) => match store.create_user(name, pw) {
                    Ok(_) => tracing::info!(admin = %name, "created admin account from ADMIN_PASSWORD"),
                    Err(e) => tracing::warn!(admin = %name, error = %e, "could not create admin account"),
                },
                None => tracing::warn!(
                    admin = %name,
                    "admin account does not exist and its username is reserved; set ADMIN_PASSWORD to create it at startup"
                ),
            },
            Err(e) => tracing::warn!(admin = %name, error = %e, "could not look up admin account"),
        }
    }
}

pub fn is_admin_username(username: &str) -> bool {
    let name = username.trim().to_ascii_lowercase();
    !name.is_empty() && admin_names().iter().any(|admin| admin.eq_ignore_ascii_case(&name))
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

fn token_from_headers(headers: &HeaderMap) -> Option<String> {
    let raw = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())?
        .trim();
    let token = raw
        .strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))
        .unwrap_or(raw)
        .trim();
    (!token.is_empty()).then(|| token.to_string())
}

fn signed_in_user(state: &AppState, headers: &HeaderMap) -> Option<User> {
    let token = token_from_headers(headers)?;
    state.store.get_session_user(&token).ok().flatten()
}

fn require_admin(state: &AppState, headers: &HeaderMap) -> Result<User, Response> {
    let user = signed_in_user(state, headers)
        .ok_or_else(|| err_res(StatusCode::UNAUTHORIZED, "Sign in to continue"))?;
    if !is_admin_username(&user.username) {
        return Err(err_res(
            StatusCode::FORBIDDEN,
            "Administrator access required",
        ));
    }
    Ok(user)
}

fn admin_event(state: &AppState, admin: &User, kind: &str, detail: &str, headers: &HeaderMap) {
    state.store.record_security_event(
        admin.id,
        kind,
        &format!("[admin of {admin}] {detail}", admin = admin.username),
        &client_ip(headers),
    );
}

fn user_summary_json(u: &crate::store::AdminUserSummary) -> Value {
    json!({
        "id": u.id,
        "username": u.username,
        "created": u.created,
        "deletedAt": u.deleted_at,
        "totpEnabled": u.totp_enabled,
        "passkeys": u.passkeys,
        "recoveryLeft": u.recovery_left,
        "sessions": u.sessions,
        "playlists": u.playlists,
        "stars": u.stars,
        "events": u.events,
        "listens": u.listens,
        "lastSeen": u.last_seen,
        "isAdmin": is_admin_username(&u.username),
    })
}

pub async fn serve_admin() -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        ADMIN_HTML,
    )
        .into_response()
}

/// Lets the admin page show a useful message instead of a blank screen.
pub async fn session(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match signed_in_user(&state, &headers) {
        None => err_res(StatusCode::UNAUTHORIZED, "Not signed in"),
        Some(u) => json_res(json!({
            "admin": is_admin_username(&u.username),
            "username": u.username,
            "userId": u.id,
        })),
    }
}

pub async fn overview(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = require_admin(&state, &headers) {
        return r;
    }
    match state.store.admin_overview() {
        Ok(o) => json_res(json!({
            "totalUsers": o.total_users,
            "activeUsers": o.active_users,
            "deletedUsers": o.deleted_users,
            "sessions": o.sessions,
            "tracks": o.tracks,
            "passkeys": o.passkeys,
            "totpUsers": o.totp_users,
            "playlists": o.playlists,
            "securityEvents": o.security_events,
            "admins": admin_names(),
            "localCacheBytes": state.store.cache_total_bytes().unwrap_or(0),
            "localCacheEntries": state.store.cache_entry_count().unwrap_or(0),
            "cacheMaxBytes": state.engine.max_cache_bytes,
            "remoteCacheEnabled": state.engine.remote.is_some(),
            "remoteCacheObjects": state.store.remote_cache_count().unwrap_or(0),
            "remoteCacheBytes": state.store.remote_cache_total_bytes().unwrap_or(0),
            "remoteCacheMaxBytes": state.engine.remote.as_ref().map(|r| r.max_bytes).unwrap_or(0),
            "remoteCacheTarget": state.engine.remote.as_ref().map(|r| r.describe()).unwrap_or_default(),
            "remoteCacheHighWaterBytes": state.engine.remote.as_ref().map(|r| r.high_water_bytes).unwrap_or(0),
            "remoteCacheByKind": state.store.remote_cache_kind_stats().unwrap_or_default()
                .into_iter()
                .map(|(kind, objects, bytes)| json!({ "kind": kind, "objects": objects, "bytes": bytes }))
                .collect::<Vec<_>>(),
            "maxTrackBytes": state.engine.max_track_bytes,
            "llm": state.llm.as_ref().map(|l| l.status_snapshot(&state.store, &state.taste)).unwrap_or_else(|| json!({ "enabled": false })),
        })),
        Err(e) => err_res(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

pub async fn config(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = require_admin(&state, &headers) {
        return r;
    }
    json_res(json!({
        "admins": admin_names(),
        "publicOrigin": std::env::var("BEATLAB_ORIGIN")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| std::env::var("URL").ok().filter(|s| !s.trim().is_empty()))
            .unwrap_or_else(|| "(not set)".to_string()),
        "dataDir": std::env::var("BEATLAB_DATA").unwrap_or_else(|_| "./data".to_string()),
        "cacheGb": std::env::var("BEATLAB_CACHE_GB").unwrap_or_else(|_| "5.0".to_string()),
        "listenAddr": std::env::var("BEATLAB_ADDR").unwrap_or_else(|_| "0.0.0.0:4533".to_string()),
        "webauthnEnabled": crate::auth::webauthn().is_some(),
        "llmEnabled": state.llm.is_some(),
        "llmEndpoint": state.llm.as_ref().map(|l| l.config.safe_endpoint_label()).unwrap_or_else(|| "(not configured)".to_string()),
        "llmModel": state.llm.as_ref().map(|l| l.config.model.clone()).unwrap_or_else(|| "(not configured)".to_string()),
        "llmApiKey": match state.llm.as_ref() {
            Some(l) if !l.config.api_key.is_empty() => "(configured)",
            Some(_) => "(none — custom endpoint)",
            None => "(not set)",
        },
        "llmRefreshHour": state.llm.as_ref().map(|l| format!("{:02}:00 local", l.config.refresh_hour)).unwrap_or_else(|| "—".to_string()),
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

pub async fn users(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = require_admin(&state, &headers) {
        return r;
    }
    match state.store.admin_list_users() {
        Ok(rows) => json_res(json!({ "users": rows.iter().map(user_summary_json).collect::<Vec<_>>() })),
        Err(e) => err_res(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

pub async fn user_detail(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> Response {
    if let Err(r) = require_admin(&state, &headers) {
        return r;
    }
    let rows = match state.store.admin_list_users() {
        Ok(rows) => rows,
        Err(e) => return err_res(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let Some(summary) = rows.into_iter().find(|u| u.id == id) else {
        return err_res(StatusCode::NOT_FOUND, "User not found");
    };
    let sessions: Vec<Value> = state
        .store
        .list_sessions(id)
        .unwrap_or_default()
        .into_iter()
        .map(|s| {
            json!({
                "id": s.token,
                "userAgent": s.user_agent,
                "ip": s.ip,
                "created": s.created,
                "lastSeen": s.last_seen,
            })
        })
        .collect();
    let events: Vec<Value> = state
        .store
        .list_security_events(id, 50)
        .unwrap_or_default()
        .into_iter()
        .map(|e| {
            json!({
                "kind": e.kind,
                "detail": e.detail,
                "ip": e.ip,
                "created": e.created,
            })
        })
        .collect();
    let profile = state
        .store
        .get_public_user(id, id)
        .ok()
        .flatten()
        .map(|p| {
            json!({
                "displayName": p.display_name,
                "bio": p.bio,
                "location": p.location,
                "avatarColor": p.avatar_color,
                "avatarUrl": p.avatar_url,
                "followers": p.followers_count,
                "following": p.following_count,
            })
        });
    json_res(json!({
        "user": user_summary_json(&summary),
        "profile": profile,
        "sessions": sessions,
        "events": events,
    }))
}

fn target_user(state: &AppState, id: i64) -> Result<User, Response> {
    // Look up without the deleted filter so soft-deleted accounts can be
    // restored or purged from the panel too.
    let conn_user = state
        .store
        .admin_list_users()
        .ok()
        .and_then(|rows| rows.into_iter().find(|u| u.id == id));
    let Some(row) = conn_user else {
        return Err(err_res(StatusCode::NOT_FOUND, "User not found"));
    };
    Ok(User {
        id: row.id,
        username: row.username,
        password: String::new(),
        created: row.created,
    })
}

fn prevent_self(admin: &User, target: &User) -> Result<(), Response> {
    if admin.id == target.id {
        Err(err_res(
            StatusCode::BAD_REQUEST,
            "You cannot modify your own admin account here",
        ))
    } else {
        Ok(())
    }
}

pub async fn revoke_sessions(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> Response {
    let admin = match require_admin(&state, &headers) {
        Ok(a) => a,
        Err(r) => return r,
    };
    let target = match target_user(&state, id) {
        Ok(u) => u,
        Err(r) => return r,
    };
    match state.store.delete_all_sessions_for_user(id) {
        Ok(n) => {
            admin_event(&state, &admin, "admin_sessions_revoked", &format!("Signed out {n} sessions of {}", target.username), &headers);
            json_res(json!({ "ok": true, "revoked": n }))
        }
        Err(e) => err_res(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

const TEMP_PASSWORD_ALPHABET: &[u8] =
    b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";

fn generate_temp_password() -> String {
    random_bytes(16)
        .into_iter()
        .map(|b| TEMP_PASSWORD_ALPHABET[b as usize % TEMP_PASSWORD_ALPHABET.len()] as char)
        .collect()
}

#[derive(Deserialize, Default)]
pub struct ResetPasswordReq {
    #[serde(rename = "newPassword", default)]
    pub new_password: Option<String>,
    #[serde(rename = "revokeSessions", default)]
    pub revoke_sessions: Option<bool>,
}

pub async fn reset_password(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    headers: HeaderMap,
    Json(req): Json<ResetPasswordReq>,
) -> Response {
    let admin = match require_admin(&state, &headers) {
        Ok(a) => a,
        Err(r) => return r,
    };
    let target = match target_user(&state, id) {
        Ok(u) => u,
        Err(r) => return r,
    };
    if let Err(r) = prevent_self(&admin, &target) {
        return r;
    }
    let (new_password, generated) = match req.new_password {
        Some(pw) => (pw, false),
        None => (generate_temp_password(), true),
    };
    let store = state.store.clone();
    let pw = new_password.clone();
    let changed = tokio::task::spawn_blocking(move || store.change_password(id, &pw))
        .await
        .unwrap_or_else(|_| Err(crate::models::err_msg("Could not change password")));
    if let Err(e) = changed {
        return err_res(StatusCode::BAD_REQUEST, &e.to_string());
    }
    let revoke = req.revoke_sessions.unwrap_or(true);
    let revoked = if revoke {
        state.store.delete_all_sessions_for_user(id).unwrap_or(0)
    } else {
        0
    };
    admin_event(
        &state,
        &admin,
        "admin_password_reset",
        &format!("Reset password for {}", target.username),
        &headers,
    );
    state.store.record_security_event(
        target.id,
        "password_reset_by_admin",
        "Account password reset by administrator",
        &client_ip(&headers),
    );
    json_res(json!({
        "ok": true,
        "password": new_password,
        "generated": generated,
        "revokedSessions": revoked,
    }))
}

pub async fn reset_2fa(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> Response {
    let admin = match require_admin(&state, &headers) {
        Ok(a) => a,
        Err(r) => return r,
    };
    let target = match target_user(&state, id) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let _ = state.store.set_totp_enabled(id, false);
    let _ = state.store.delete_all_passkeys(id);
    let _ = state.store.replace_recovery_codes(id, &[]);
    admin_event(&state, &admin, "admin_2fa_reset", &format!("Reset 2FA for {}", target.username), &headers);
    json_res(json!({ "ok": true }))
}

pub async fn delete_user(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> Response {
    let admin = match require_admin(&state, &headers) {
        Ok(a) => a,
        Err(r) => return r,
    };
    let target = match target_user(&state, id) {
        Ok(u) => u,
        Err(r) => return r,
    };
    if let Err(r) = prevent_self(&admin, &target) {
        return r;
    }
    let restore_hash = hash_token(&uuid::Uuid::new_v4().simple().to_string());
    match state.store.soft_delete_user(id, &restore_hash) {
        Ok(()) => {
            admin_event(&state, &admin, "admin_user_deleted", &format!("Scheduled deletion of {}", target.username), &headers);
            json_res(json!({ "ok": true }))
        }
        Err(e) => err_res(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

pub async fn restore_user(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> Response {
    let admin = match require_admin(&state, &headers) {
        Ok(a) => a,
        Err(r) => return r,
    };
    let target = match target_user(&state, id) {
        Ok(u) => u,
        Err(r) => return r,
    };
    match state.store.restore_user(id) {
        Ok(()) => {
            admin_event(&state, &admin, "admin_user_restored", &format!("Restored {}", target.username), &headers);
            json_res(json!({ "ok": true }))
        }
        Err(e) => err_res(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

/// Backfills the cool cache with audio tracks currently in the local hot cache.
/// Covers, embeds and avatars are mirrored as they are created.
pub async fn backfill_cache(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let admin = match require_admin(&state, &headers) {
        Ok(a) => a,
        Err(r) => return r,
    };
    let Some(remote) = state.engine.remote.clone() else {
        return err_res(StatusCode::BAD_REQUEST, "No S3/R2 cool cache is configured");
    };
    let store = state.store.clone();
    tokio::spawn(async move {
        let entries = store.list_cache_entries().unwrap_or_default();
        let mut uploaded = 0usize;
        for (track_id, path, bytes) in entries {
            let p = std::path::PathBuf::from(&path);
            if !p.exists() {
                continue;
            }
            let ext = p
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("m4a")
                .to_string();
            let key = remote.object_key(&track_id);
            if remote.upload_from(&key, &p, &ext).await.is_ok() {
                let _ = store.upsert_remote_cache_entry(&track_id, &key, bytes);
                uploaded += 1;
            }
        }
        store.record_security_event(
            admin.id,
            "admin_cache_backfill",
            &format!("Backfilled {uploaded} tracks to the cool cache"),
            "",
        );
    });
    json_res(json!({ "ok": true, "started": true }))
}

#[derive(Deserialize, Default)]
pub struct RefreshLlmReq {
    #[serde(rename = "userId", default)]
    pub user_id: Option<i64>,
}

/// Immediately triggers an overnight LLM customization refresh for a single user
/// or for all active users.
pub async fn refresh_llm(
    State(state): State<AppState>,
    headers: HeaderMap,
    req: Option<Json<RefreshLlmReq>>,
) -> Response {
    let admin = match require_admin(&state, &headers) {
        Ok(a) => a,
        Err(r) => return r,
    };
    let Some(curator) = state.llm.clone() else {
        return err_res(
            StatusCode::BAD_REQUEST,
            "LLM curator is not configured — set BEATLAB_LLM_MODEL (and BEATLAB_LLM_API_KEY / BEATLAB_LLM_ENDPOINT)",
        );
    };
    let target_ids: Vec<i64> = match req.and_then(|j| j.0.user_id) {
        Some(uid) if uid > 0 => vec![uid],
        _ => state.store.all_user_ids().unwrap_or_default(),
    };
    let target_count = target_ids.len();
    let taste = state.taste.clone();
    let store = state.store.clone();
    let ip = client_ip(&headers);
    let admin_id = admin.id;
    let admin_name = admin.username.clone();

    tokio::spawn(async move {
        let mut ok_count = 0usize;
        let mut err_count = 0usize;
        for uid in target_ids {
            match curator.refresh_user_experience(&taste, uid, true).await {
                Ok(true) => ok_count += 1,
                Ok(false) => {}
                Err(e) => {
                    err_count += 1;
                    tracing::warn!(user_id = uid, error = %e, "manual admin LLM refresh failed");
                }
            }
        }
        store.record_security_event(
            admin_id,
            "admin_llm_refresh",
            &format!("[admin of {admin_name}] Refreshed daily LLM experience for {ok_count}/{target_count} users ({err_count} errors)"),
            &ip,
        );
    });

    json_res(json!({
        "ok": true,
        "started": true,
        "targetUsers": target_count,
    }))
}

pub async fn purge_user(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> Response {
    let admin = match require_admin(&state, &headers) {
        Ok(a) => a,
        Err(r) => return r,
    };
    let target = match target_user(&state, id) {
        Ok(u) => u,
        Err(r) => return r,
    };
    if let Err(r) = prevent_self(&admin, &target) {
        return r;
    }
    admin_event(&state, &admin, "admin_user_purged", &format!("Permanently deleted {}", target.username), &headers);
    match state.store.hard_delete_user(id) {
        Ok(()) => json_res(json!({ "ok": true })),
        Err(e) => err_res(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}
