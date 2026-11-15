use crate::models::{Mood, Track};
use crate::subsonic::AppState;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

const PLAYER_HTML: &str = include_str!("player.html");

pub async fn index_page() -> Html<&'static str> {
    Html(PLAYER_HTML)
}

fn default_user_id(state: &AppState) -> i64 {
    state
        .store
        .list_users()
        .first()
        .map(|u| u.id)
        .unwrap_or(1)
}

fn track_dto(state: &AppState, user_id: i64, t: &Track) -> serde_json::Value {
    json!({
        "id": t.id,
        "source": t.source,
        "title": t.title,
        "artist": t.artist,
        "album": t.album,
        "genre": t.genre,
        "moods": t.moods,
        "year": t.year,
        "duration": t.duration,
        "bitrate": t.bitrate,
        "sample_rate": t.sample_rate,
        "bit_depth": t.bit_depth,
        "format": t.format,
        "quality_badge": t.quality_badge(),
        "quality_score": t.quality_score(),
        "is_lossless": t.is_lossless(),
        "starred": state.store.is_starred(user_id, &t.id),
        "rating": state.store.user_rating(user_id, &t.id),
        "play_count": t.play_count,
    })
}

pub async fn api_home(State(state): State<AppState>) -> Response {
    let uid = default_user_id(&state);
    let mixes = state.taste.generate_auto_playlists(uid);
    let recs = state.taste.recommend_for_user(uid, 24);

    let mixes_json: Vec<_> = mixes
        .into_iter()
        .map(|m| {
            json!({
                "id": m.id,
                "name": m.name,
                "subtitle": m.subtitle,
                "kind": m.kind,
                "cover_track_id": m.tracks.first().map(|t| t.id.clone()),
                "tracks": m.tracks.iter().map(|t| track_dto(&state, uid, t)).collect::<Vec<_>>(),
            })
        })
        .collect();

    let moods_json: Vec<_> = Mood::ALL
        .iter()
        .map(|m| {
            json!({
                "slug": m.slug(),
                "label": m.label(),
                "description": m.description(),
            })
        })
        .collect();

    let recs_json: Vec<_> = recs.iter().map(|t| track_dto(&state, uid, t)).collect();
    let (nn_updates, nn_loss) = state.taste.neural_stats(uid);

    Json(json!({
        "mixes": mixes_json,
        "moods": moods_json,
        "recommended": recs_json,
        "stats": {
            "total_tracks": state.store.track_count(),
            "lossless_tracks": state.store.lossless_count(),
            "nn_updates": nn_updates,
            "nn_loss": (nn_loss * 1000.0).round() / 1000.0,
        }
    }))
    .into_response()
}

pub async fn api_search(
    State(state): State<AppState>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let uid = default_user_id(&state);
    let query = q.get("q").map(|s| s.as_str()).unwrap_or("").trim();
    if query.is_empty() {
        let all = state.store.all_tracks(40);
        let items: Vec<_> = all.iter().map(|t| track_dto(&state, uid, t)).collect();
        return Json(json!({ "tracks": items })).into_response();
    }

    let mut tracks = state.store.search_tracks(query, 30);
    if tracks.len() < 15 {
        let remote = state.registry.search_all(query, 25).await;
        tracks.extend(remote);
        tracks = crate::sources::dedup_best_quality(tracks, 30);
    }
    state.engine.warm_resolve(tracks.clone(), 4);
    let items: Vec<_> = tracks.iter().map(|t| track_dto(&state, uid, t)).collect();
    Json(json!({ "tracks": items })).into_response()
}

pub async fn api_mood_radio(
    State(state): State<AppState>,
    Path(slug_str): Path<String>,
) -> Response {
    let uid = default_user_id(&state);
    let Some(mood) = Mood::from_slug(&slug_str) else {
        return (StatusCode::NOT_FOUND, "unknown mood").into_response();
    };
    let tracks = state.taste.mood_radio(mood, uid, 30).await;
    state.engine.warm_resolve(tracks.clone(), 5);
    let items: Vec<_> = tracks.iter().map(|t| track_dto(&state, uid, t)).collect();
    Json(json!({
        "mood": {
            "slug": mood.slug(),
            "label": mood.label(),
            "description": mood.description(),
        },
        "tracks": items
    }))
    .into_response()
}

pub async fn api_track_radio(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let uid = default_user_id(&state);
    let tracks = if let Some(seed) = state.store.track(&id) {
        let mut list = state.taste.similar_songs(&seed, uid, 25);
        if list.len() < 8 {
            let extra = state.taste.artist_radio(&seed.artist, uid, 15).await;
            list.extend(extra);
            list = crate::sources::dedup_best_quality(list, 25);
        }
        list
    } else {
        state.taste.recommend_for_user(uid, 25)
    };
    state.engine.warm_resolve(tracks.clone(), 5);
    let items: Vec<_> = tracks.iter().map(|t| track_dto(&state, uid, t)).collect();
    Json(json!({ "tracks": items })).into_response()
}

pub async fn api_stream(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let Some(track) = state.store.track(&id) else {
        return (StatusCode::NOT_FOUND, "track not found").into_response();
    };
    state.engine.serve_track(&track, &headers, None, None).await
}

pub async fn api_cover(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    state.engine.serve_cover(&id).await
}

#[derive(Deserialize)]
pub struct EventReq {
    pub track_id: String,
    pub kind: String, // "play", "complete", "skip", "star", "unstar"
}

pub async fn api_event(
    State(state): State<AppState>,
    Json(req): Json<EventReq>,
) -> Response {
    let uid = default_user_id(&state);
    match req.kind.as_str() {
        "star" => state.store.star_track(uid, &req.track_id),
        "unstar" => state.store.unstar_track(uid, &req.track_id),
        "complete" => {
            state.store.increment_play(&req.track_id);
            state.store.record_event(uid, &req.track_id, "complete", 1.0);
        }
        "play" | "skip" => {
            state.store.record_event(uid, &req.track_id, &req.kind, 1.0);
        }
        _ => {}
    }
    if let Some(tr) = state.store.track(&req.track_id) {
        state.taste.learn_event(uid, &tr, &req.kind, 1.0);
    }
    Json(json!({ "ok": true })).into_response()
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
