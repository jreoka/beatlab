use crate::models::Track;
use crate::store::Store;
use crate::subsonic::AppState;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

static TOKENS: std::sync::OnceLock<Arc<Mutex<HashMap<String, i64>>>> = std::sync::OnceLock::new();

fn token_map() -> &'static Arc<Mutex<HashMap<String, i64>>> {
    TOKENS.get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
}

#[derive(Deserialize)]
pub struct LoginReq {
    pub username: String,
    pub password: String,
}

pub async fn nd_login(State(state): State<AppState>, Json(req): Json<LoginReq>) -> Response {
    let Some(user) = state
        .store
        .auth_subsonic(&req.username, &req.password, "", "")
    else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "Invalid credentials"})),
        )
            .into_response();
    };
    let tok = uuid::Uuid::new_v4().to_string();
    token_map().lock().unwrap().insert(tok.clone(), user.id);

    Json(json!({
        "id": user.id.to_string(),
        "username": user.username,
        "name": user.username,
        "isAdmin": user.is_admin,
        "token": tok,
        "subsonicSalt": "beatlab",
        "subsonicToken": md5_hex(&format!("{}beatlab", user.password))
    }))
    .into_response()
}

fn auth_nd(state: &AppState, headers: &HeaderMap) -> Option<crate::models::User> {
    let hdr = headers
        .get("x-nd-authorization")
        .or_else(|| headers.get(axum::http::header::AUTHORIZATION))
        .and_then(|v| v.to_str().ok())?;
    let tok = hdr.strip_prefix("Bearer ").unwrap_or(hdr).trim();
    let uid = token_map().lock().unwrap().get(tok).copied()?;
    state.store.get_user_by_id(uid)
}

fn nd_song_json(store: &Store, user_id: i64, t: &Track) -> serde_json::Value {
    json!({
        "id": t.id,
        "title": t.title,
        "album": t.album,
        "albumId": t.album_id(),
        "artist": t.artist,
        "artistId": t.artist_id(),
        "genre": t.genre,
        "trackNumber": t.track_no,
        "year": t.year,
        "duration": t.duration,
        "bitRate": t.bitrate,
        "sampleRate": t.sample_rate,
        "bitDepth": t.bit_depth,
        "suffix": t.format,
        "size": t.size,
        "playCount": t.play_count,
        "starred": store.is_starred(user_id, &t.id),
        "rating": store.user_rating(user_id, &t.id),
    })
}

pub async fn nd_song_list(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let Some(user) = auth_nd(&state, &headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let limit = q
        .get("_end")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(50)
        .min(200);
    let tracks = if let Some(search) = q.get("title").or_else(|| q.get("q")) {
        state.store.search_tracks(search, limit)
    } else {
        state.store.all_tracks(limit)
    };
    let total = tracks.len();
    let items: Vec<_> = tracks
        .iter()
        .map(|t| nd_song_json(&state.store, user.id, t))
        .collect();
    (
        StatusCode::OK,
        [("x-total-count", total.to_string())],
        Json(items),
    )
        .into_response()
}

pub async fn nd_playlist_list(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(user) = auth_nd(&state, &headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let _ = state.taste.generate_auto_playlists(user.id);
    let pls = state.store.list_playlists(user.id);
    let total = pls.len();
    let items: Vec<_> = pls
        .into_iter()
        .map(|p| {
            json!({
                "id": p.id,
                "name": p.name,
                "comment": p.comment,
                "public": p.public,
                "songCount": p.track_count,
                "duration": p.duration,
                "ownerName": user.username,
            })
        })
        .collect();
    (
        StatusCode::OK,
        [("x-total-count", total.to_string())],
        Json(items),
    )
        .into_response()
}

pub async fn nd_playlist_tracks(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let Some(user) = auth_nd(&state, &headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let tracks = state.store.playlist_tracks(&id);
    let total = tracks.len();
    let items: Vec<_> = tracks
        .iter()
        .map(|t| nd_song_json(&state.store, user.id, t))
        .collect();
    (
        StatusCode::OK,
        [("x-total-count", total.to_string())],
        Json(items),
    )
        .into_response()
}

pub async fn nd_radio_list(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if auth_nd(&state, &headers).is_none() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let stations = state.store.list_radio_stations();
    let total = stations.len();
    (
        StatusCode::OK,
        [("x-total-count", total.to_string())],
        Json(stations),
    )
        .into_response()
}

fn md5_hex(s: &str) -> String {
    use md5::{Digest, Md5};
    let mut h = Md5::new();
    h.update(s.as_bytes());
    hex::encode(h.finalize())
}
