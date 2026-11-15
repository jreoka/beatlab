use crate::subsonic::AppState;
use axum::Json;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::Next;
use axum::response::{Html, IntoResponse, Response};
use rand::RngCore;
use serde::Deserialize;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;

const ADMIN_HTML: &str = include_str!("admin.html");
const ADMIN_COOKIE_NAME: &str = "beatlab_admin_session";

fn constant_time_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes()
        .zip(b.bytes())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

fn extract_session_token(headers: &HeaderMap) -> Option<String> {
    let prefix = format!("{ADMIN_COOKIE_NAME}=");
    for val in headers.get_all(header::COOKIE) {
        let Ok(s) = val.to_str() else { continue };
        for part in s.split(';') {
            let part = part.trim();
            if let Some(token) = part.strip_prefix(&prefix) {
                let token = token.trim();
                if !token.is_empty() {
                    return Some(token.to_string());
                }
            }
        }
    }
    None
}

fn is_authenticated(state: &AppState, headers: &HeaderMap) -> bool {
    let Some(token) = extract_session_token(headers) else {
        return false;
    };
    state.admin_sessions.lock().unwrap().contains(&token)
}

pub async fn require_admin_auth(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Response {
    if is_authenticated(&state, req.headers()) {
        next.run(req).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "Unauthorized" })),
        )
            .into_response()
    }
}

#[derive(Deserialize)]
pub struct AdminLoginReq {
    pub password: String,
}

pub async fn api_admin_login(
    State(state): State<AppState>,
    Json(req): Json<AdminLoginReq>,
) -> Response {
    if !constant_time_eq(&req.password, &state.admin_password) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "ok": false, "error": "Invalid admin password" })),
        )
            .into_response();
    }
    let mut buf = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut buf);
    let token = hex::encode(buf);
    state.admin_sessions.lock().unwrap().insert(token.clone());
    let cookie = format!("{ADMIN_COOKIE_NAME}={token}; HttpOnly; SameSite=Lax; Path=/");
    (
        StatusCode::OK,
        [(header::SET_COOKIE, cookie)],
        Json(json!({ "ok": true })),
    )
        .into_response()
}

pub async fn api_admin_logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(token) = extract_session_token(&headers) {
        state.admin_sessions.lock().unwrap().remove(&token);
    }
    let clear_cookie = format!("{ADMIN_COOKIE_NAME}=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0");
    (
        StatusCode::OK,
        [(header::SET_COOKIE, clear_cookie)],
        Json(json!({ "ok": true })),
    )
        .into_response()
}

pub async fn admin_page() -> Html<&'static str> {
    Html(ADMIN_HTML)
}

pub async fn api_admin_status(State(state): State<AppState>) -> Response {
    let sources: Vec<_> = state
        .registry
        .all()
        .iter()
        .map(|s| {
            json!({
                "name": s.name(),
                "available": s.available(),
                "status": s.status(),
            })
        })
        .collect();

    let dirs: Vec<String> = state
        .local
        .dirs()
        .into_iter()
        .map(|p| p.display().to_string())
        .collect();

    Json(json!({
        "version": "0.2.0-rust",
        "tracks": state.store.track_count(),
        "lossless_tracks": state.store.lossless_count(),
        "cache_bytes": state.store.cache_bytes(),
        "cache_budget_bytes": state.engine.cache_budget_bytes(),
        "ffmpeg": state.engine.has_ffmpeg(),
        "local_dirs": dirs,
        "sources": sources,
        "soulseek": state.soulseek.connection_info(),
        "users": state.store.list_users().into_iter().map(|u| json!({"id": u.id, "username": u.username, "is_admin": u.is_admin})).collect::<Vec<_>>(),
    }))
    .into_response()
}

#[derive(Deserialize)]
pub struct AddDirReq {
    pub path: String,
}

pub async fn api_admin_add_dir(
    State(state): State<AppState>,
    Json(req): Json<AddDirReq>,
) -> Response {
    let p = PathBuf::from(req.path.trim());
    if !p.exists() || !p.is_dir() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Directory does not exist" })),
        )
            .into_response();
    }
    state.local.add_dir(p);
    let local = Arc::clone(&state.local);
    let scanned = tokio::task::spawn_blocking(move || local.scan())
        .await
        .unwrap_or(0);
    Json(json!({ "ok": true, "scanned": scanned })).into_response()
}

pub async fn api_admin_scan(State(state): State<AppState>) -> Response {
    let local = Arc::clone(&state.local);
    let scanned = tokio::task::spawn_blocking(move || local.scan())
        .await
        .unwrap_or(0);
    Json(json!({ "ok": true, "scanned": scanned })).into_response()
}

pub async fn api_admin_clear_cache(State(state): State<AppState>) -> Response {
    state.engine.clear_audio_cache();
    Json(json!({ "ok": true })).into_response()
}

#[derive(Deserialize)]
pub struct SoulseekConnectReq {
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub server: Option<String>,
}

pub async fn api_admin_soulseek_connect(
    State(state): State<AppState>,
    Json(req): Json<SoulseekConnectReq>,
) -> Response {
    match state
        .soulseek
        .connect(&req.username, &req.password, req.server.as_deref())
        .await
    {
        Ok(()) => {
            let taste = Arc::clone(&state.taste);
            tokio::spawn(async move {
                taste.warm_popular_on_connect().await;
            });
            Json(json!({
                "ok": true,
                "soulseek": state.soulseek.connection_info(),
            }))
            .into_response()
        }
        Err(err) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({
                "ok": false,
                "error": err,
                "soulseek": state.soulseek.connection_info(),
            })),
        )
            .into_response(),
    }
}

pub async fn api_admin_soulseek_disconnect(State(state): State<AppState>) -> Response {
    state.soulseek.disconnect().await;
    Json(json!({
        "ok": true,
        "soulseek": state.soulseek.connection_info(),
    }))
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Engine;
    use crate::sources::SourceRegistry;
    use crate::sources::local::LocalSource;
    use crate::sources::soulseek::SoulseekSource;
    use crate::store::Store;
    use crate::taste::TasteEngine;
    use axum::Router;
    use axum::routing::{get, post};
    use std::collections::{HashMap, HashSet};
    use std::sync::Mutex;

    fn test_state(password: &str) -> AppState {
        let dir = std::env::temp_dir().join(format!("beatlab-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::open(&dir).unwrap();
        let local = Arc::new(LocalSource::new(store.clone(), vec![]));
        let soulseek = SoulseekSource::new(store.clone(), dir.join("cache").join("soulseek"));
        let registry = Arc::new(SourceRegistry::new(store.clone(), vec![local.clone()]));
        let engine = Arc::new(Engine::new(store.clone(), registry.clone(), &dir, 1));
        let taste = Arc::new(TasteEngine::new(store.clone(), registry.clone()));
        AppState {
            store,
            registry,
            engine,
            taste,
            local,
            soulseek,
            now_playing: Arc::new(Mutex::new(HashMap::new())),
            admin_password: password.to_string(),
            admin_sessions: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    #[tokio::test]
    async fn admin_panel_requires_password_and_supports_logout() {
        let state = test_state("secret-admin-pw");
        let admin_api = Router::new()
            .route("/status", get(api_admin_status))
            .route("/scan", post(api_admin_scan))
            .route_layer(axum::middleware::from_fn_with_state(
                state.clone(),
                require_admin_auth,
            ));
        let app = Router::new()
            .route("/admin", get(admin_page))
            .route("/admin/api/login", post(api_admin_login))
            .route("/admin/api/logout", post(api_admin_logout))
            .nest("/admin/api", admin_api)
            .with_state(state);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let client = reqwest::Client::new();
        let base = format!("http://{addr}");

        // 1. Admin HTML page loads and includes the login/logout UI
        let page = client.get(format!("{base}/admin")).send().await.unwrap();
        assert_eq!(page.status(), reqwest::StatusCode::OK);
        let html = page.text().await.unwrap();
        assert!(html.contains("logoutAdmin()"));
        assert!(html.contains("loginAdmin(event)"));

        // 2. Unauthenticated API calls are rejected
        let unauth = client
            .get(format!("{base}/admin/api/status"))
            .send()
            .await
            .unwrap();
        assert_eq!(unauth.status(), reqwest::StatusCode::UNAUTHORIZED);

        let unauth_scan = client
            .post(format!("{base}/admin/api/scan"))
            .send()
            .await
            .unwrap();
        assert_eq!(unauth_scan.status(), reqwest::StatusCode::UNAUTHORIZED);

        // 3. Wrong password fails
        let bad_login = client
            .post(format!("{base}/admin/api/login"))
            .json(&json!({ "password": "wrong" }))
            .send()
            .await
            .unwrap();
        assert_eq!(bad_login.status(), reqwest::StatusCode::UNAUTHORIZED);

        // 4. Correct password sets session cookie
        let good_login = client
            .post(format!("{base}/admin/api/login"))
            .json(&json!({ "password": "secret-admin-pw" }))
            .send()
            .await
            .unwrap();
        assert_eq!(good_login.status(), reqwest::StatusCode::OK);
        let set_cookie = good_login
            .headers()
            .get(reqwest::header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        let cookie_pair = set_cookie.split(';').next().unwrap().to_string();
        assert!(cookie_pair.starts_with("beatlab_admin_session="));

        // 5. Authenticated request succeeds
        let authed = client
            .get(format!("{base}/admin/api/status"))
            .header(reqwest::header::COOKIE, &cookie_pair)
            .send()
            .await
            .unwrap();
        assert_eq!(authed.status(), reqwest::StatusCode::OK);

        // 6. Logout invalidates session and clears cookie
        let logout = client
            .post(format!("{base}/admin/api/logout"))
            .header(reqwest::header::COOKIE, &cookie_pair)
            .send()
            .await
            .unwrap();
        assert_eq!(logout.status(), reqwest::StatusCode::OK);
        let clear_cookie = logout
            .headers()
            .get(reqwest::header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert!(clear_cookie.contains("Max-Age=0"));

        // 7. Old session cookie is no longer accepted
        let after_logout = client
            .get(format!("{base}/admin/api/status"))
            .header(reqwest::header::COOKIE, &cookie_pair)
            .send()
            .await
            .unwrap();
        assert_eq!(after_logout.status(), reqwest::StatusCode::UNAUTHORIZED);
    }
}
