use crate::models::{AlbumSummary, ArtistSummary, Mood, PlayQueueState, Playlist, Track, User};
use crate::subsonic::AppState;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;

const INDEX_HTML: &str = include_str!("web/index.html");
const STYLES_CSS: &str = include_str!("web/styles.css");
const APP_JS: &str = include_str!("web/app.js");
const FAVICON_SVG: &str = include_str!("web/favicon.svg");

pub async fn serve_index() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        INDEX_HTML,
    )
}

pub async fn serve_css() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        STYLES_CSS,
    )
}

pub async fn serve_js() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/javascript; charset=utf-8")],
        APP_JS,
    )
}

pub async fn serve_favicon() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "image/svg+xml; charset=utf-8")],
        FAVICON_SVG,
    )
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
        .map(|t| WebTrack {
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
            is_taste_match: taste_match_ids.contains(&t.id),
            quality_badge: t.quality_badge(),
            cover_url: cover_api_url(&t.id),
            is_starred: starred.contains(&t.id),
            play_count: stats.get(&t.id).map(|s| s.play_count).unwrap_or(0),
            rating: ratings.get(&t.id).copied().unwrap_or(0),
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

#[derive(Deserialize)]
pub struct AuthReq {
    pub username: String,
    pub password: String,
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
    #[serde(default)]
    pub query: Option<String>,
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
}

#[derive(Deserialize)]
pub struct SearchQuery {
    pub q: Option<String>,
}

#[derive(Deserialize)]
pub struct RadioQuery {
    pub seed: Option<String>,
    pub playlist: Option<String>,
    pub exclude: Option<String>,
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
        Some(u) => {
            let api_key = state.store.get_or_create_api_key(u.id).unwrap_or_default();
            json_res(json!({
                "user": {
                    "id": u.id,
                    "username": u.username,
                    "apiKey": api_key,
                    "created": u.created,
                }
            }))
        }
        None => json_res(json!({ "user": null })),
    }
}

pub async fn signin(State(state): State<AppState>, Json(req): Json<AuthReq>) -> Response {
    match state
        .store
        .authenticate_or_create_user(&req.username, &req.password)
    {
        Ok((user, created)) => {
            if created {
                let taste = state.taste.clone();
                let uid = user.id;
                tokio::spawn(async move {
                    taste.rebuild_auto_playlists(uid);
                });
            }
            match state.store.create_session(user.id, "web", 30 * 86400) {
                Ok(token) => {
                    let api_key = state.store.get_or_create_api_key(user.id).unwrap_or_default();
                    json_res(json!({
                        "token": token,
                        "created": created,
                        "user": {
                            "id": user.id,
                            "username": user.username,
                            "apiKey": api_key,
                            "created": user.created,
                        }
                    }))
                }
                Err(e) => err_res(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
            }
        }
        Err(e) => err_res(StatusCode::UNAUTHORIZED, &e.to_string()),
    }
}

pub async fn signup(State(state): State<AppState>, Json(req): Json<AuthReq>) -> Response {
    match state.store.create_user(&req.username, &req.password) {
        Ok(user) => {
            let taste = state.taste.clone();
            let uid = user.id;
            tokio::spawn(async move {
                taste.rebuild_auto_playlists(uid);
            });
            match state.store.create_session(user.id, "web", 30 * 86400) {
                Ok(token) => {
                    let api_key = state.store.get_or_create_api_key(user.id).unwrap_or_default();
                    json_res(json!({
                        "token": token,
                        "created": true,
                        "user": {
                            "id": user.id,
                            "username": user.username,
                            "apiKey": api_key,
                            "created": user.created,
                        }
                    }))
                }
                Err(e) => err_res(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
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

pub async fn regenerate_api_key(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    match state.store.regenerate_api_key(user.id) {
        Ok(api_key) => json_res(json!({ "apiKey": api_key })),
        Err(e) => err_res(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

pub async fn change_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<PasswordChangeReq>,
) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    if !crate::models::constant_time_eq(&user.password, &req.current_password) {
        return err_res(StatusCode::UNAUTHORIZED, "Current password is incorrect");
    }
    match state.store.change_password(user.id, &req.new_password) {
        Ok(()) => json_res(json!({ "ok": true })),
        Err(e) => err_res(StatusCode::BAD_REQUEST, &e.to_string()),
    }
}

pub async fn home(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let uid = user.id;

    // Ensure auto-mixes exist and refresh cleanly whenever user feedback/playlists changed them.
    if state.store.count_tracks().unwrap_or(0) > 0 {
        state.taste.refresh_auto_playlists_if_stale_async(uid, 900).await;
    }
    let playlists = state.store.list_playlists(uid, false).unwrap_or_default();

    let auto_mixes: Vec<Value> = playlists
        .into_iter()
        .filter(|p| p.is_auto)
        .map(|p| playlist_summary_json(&p))
        .collect();

    let quick_picks = state.taste.quick_picks(uid, 12);
    let recent: Vec<Track> = state
        .store
        .user_recent_tracks(uid, 12, 0)
        .unwrap_or_default()
        .into_iter()
        .map(|(t, _)| t)
        .collect();

    let (chart_tracks, chart_albums, chart_artists) = tokio::join!(
        state.sources.deezer.fetch_chart_tracks(20),
        state.sources.deezer.fetch_chart_albums(12),
        state.sources.deezer.fetch_chart_artists(12),
    );
    if !chart_tracks.is_empty() {
        let _ = state.store.upsert_tracks(&chart_tracks);
    }
    for a in &chart_albums {
        let _ = state.store.upsert_album_meta(a);
    }
    for ar in &chart_artists {
        let _ = state.store.upsert_artist_meta(ar);
    }

    let trending_tracks = if !chart_tracks.is_empty() {
        chart_tracks
    } else {
        state.store.newest_tracks(16, 0).unwrap_or_default()
    };
    let albums = if !chart_albums.is_empty() {
        chart_albums
    } else {
        state
            .store
            .build_album_summaries(uid)
            .unwrap_or_default()
            .into_iter()
            .take(12)
            .collect()
    };
    let artists = if !chart_artists.is_empty() {
        chart_artists
    } else {
        state
            .store
            .build_artist_summaries()
            .unwrap_or_default()
            .into_iter()
            .take(12)
            .collect()
    };

    let saved = state.store.saved_album_ids(uid).unwrap_or_default();
    let followed = state.store.followed_artist_ids(uid).unwrap_or_default();

    let moods: Vec<Value> = Mood::ALL
        .iter()
        .map(|m| {
            let (c1, c2) = m.gradient();
            json!({
                "slug": m.slug(),
                "label": m.label(),
                "subtitle": m.subtitle(),
                "c1": c1,
                "c2": c2,
            })
        })
        .collect();

    json_res(json!({
        "quickPicks": to_web_tracks(&state, uid, &quick_picks),
        "recent": to_web_tracks(&state, uid, &recent),
        "madeForYou": auto_mixes,
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
    let (dz_genres, chart_tracks, chart_albums) = tokio::join!(
        state.sources.deezer.fetch_genres(),
        state.sources.deezer.fetch_chart_tracks(30),
        state.sources.deezer.fetch_chart_albums(18),
    );
    if !chart_tracks.is_empty() {
        let _ = state.store.upsert_tracks(&chart_tracks);
    }
    for a in &chart_albums {
        let _ = state.store.upsert_album_meta(a);
    }

    let moods: Vec<Value> = Mood::ALL
        .iter()
        .map(|m| {
            let (c1, c2) = m.gradient();
            json!({
                "slug": m.slug(),
                "label": m.label(),
                "subtitle": m.subtitle(),
                "c1": c1,
                "c2": c2,
            })
        })
        .collect();

    let genres: Vec<Value> = dz_genres
        .into_iter()
        .map(|g| {
            json!({
                "id": g.id,
                "name": g.name,
                "coverUrl": g.picture_url,
            })
        })
        .collect();

    let saved = state.store.saved_album_ids(user.id).unwrap_or_default();
    json_res(json!({
        "moods": moods,
        "genres": genres,
        "charts": to_web_tracks(&state, user.id, &chart_tracks),
        "newAlbums": chart_albums.iter().map(|a| album_json(a, saved.contains(&a.id))).collect::<Vec<_>>(),
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
    json_res(json!({
        "mood": {
            "slug": mood.slug(),
            "label": mood.label(),
            "subtitle": mood.subtitle(),
            "c1": c1,
            "c2": c2,
        },
        "tracks": to_web_tracks(&state, user.id, &tracks),
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
    json_res(json!({
        "genre": decoded,
        "tracks": to_web_tracks(&state, user.id, &tracks),
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

    let intent = crate::taste::parse_search_intent(&query);
    let base_query = intent
        .entity_query
        .as_deref()
        .unwrap_or(&query)
        .to_string();

    let mix_future = async {
        if !intent.is_theme_mix {
            return (Vec::new(), Vec::new());
        }
        let pl_queries = intent.playlist_search_queries();
        let tr_queries = intent.expanded_track_queries();
        let genre_hints = intent.deezer_genre_hints();

        let pl_futs = pl_queries
            .iter()
            .map(|pq| state.sources.deezer.fetch_curated_mix_tracks(pq, 2, 24));
        let dz_extra_futs = tr_queries
            .iter()
            .map(|tq| state.sources.deezer.search_tracks(tq, 18));
        let ytm_extra_futs = tr_queries.iter().map(|tq| {
            use crate::sources::MusicSource;
            state.sources.ytm.search(tq, 15)
        });
        let genre_chart_fut = async {
            if genre_hints.is_empty() {
                return Vec::new();
            }
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
    };

    let raw_radio_fut = async {
        if intent.has_trailing_radio_ambiguity && !base_query.eq_ignore_ascii_case(&query) {
            state.sources.search_all_with_ytm_hints(&query, 10).await
        } else {
            (Vec::new(), std::collections::HashMap::new())
        }
    };

    let (((all_tracks, mut ytm_hints), (dz_albums, dz_artists), (raw_tracks_extra, raw_ytm_hints)), mix_res) =
        tokio::join!(
            async {
                tokio::join!(
                    state.sources.search_all_with_ytm_hints(&base_query, 20),
                    async {
                        tokio::join!(
                            state.sources.deezer.search_albums(&base_query, 12),
                            state.sources.deezer.search_artists(&base_query, 12),
                        )
                    },
                    raw_radio_fut,
                )
            },
            tokio::time::timeout(std::time::Duration::from_secs(4), mix_future)
        );

    for (k, v) in raw_ytm_hints {
        ytm_hints.entry(k).or_insert(v);
    }

    let (curated_batches, extra_tracks) = mix_res.unwrap_or_default();
    let combined = [all_tracks, raw_tracks_extra, extra_tracks].concat();

    // Persist initial search hits before entity-radio expansion.
    let mut to_upsert = combined.clone();
    for (_, batch) in &curated_batches {
        to_upsert.extend(batch.iter().cloned());
    }
    if !to_upsert.is_empty() {
        let _ = state.store.upsert_tracks(&to_upsert);
    }
    for a in &dz_albums {
        let _ = state.store.upsert_album_meta(a);
    }
    for ar in &dz_artists {
        let _ = state.store.upsert_artist_meta(ar);
    }

    let mix_ctx = if !intent.is_theme_mix {
        expand_entity_mix_context(&state, user.id, &query, &combined, &dz_artists, &ytm_hints).await
    } else {
        crate::taste::SearchMixContext::default()
    };

    build_search_response(
        state,
        user.id,
        query,
        base_query,
        combined,
        curated_batches,
        mix_ctx,
        dz_albums,
        dz_artists,
    )
    .await
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
                let hop_tracks = ytm.fetch_radio_tracks(&hop_vid, 20).await;
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
                    .into_iter()
                    .find(|a| crate::models::slug(&a.name) == artist_slug)
                    .map(|a| a.deezer_id)
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
                .take(3)
                .cloned()
                .collect();
            related_artists.push(meta);
            related_artists.extend(related);
            let rel_futs = hop_targets.into_iter().map(|rel| {
                let dz_c = dz.clone();
                async move { dz_c.fetch_artist_top_tracks(&rel.deezer_id, 5).await }
            });
            for batch in futures::future::join_all(rel_futs).await {
                related_tracks.extend(batch);
            }
        }
        (top_tracks, radio_res, related_tracks, related_artists)
    };

    let budget = std::time::Duration::from_millis(2600);
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
        anchor_track_key: seed_track.as_ref().map(Track::song_key),
        anchor_genres,
        is_artist_query: plan.is_artist_query,
        ytm_radio,
        dz_artist_top,
        dz_artist_radio,
        dz_related_tracks,
    }
}

async fn build_search_response(
    state: AppState,
    uid: i64,
    query: String,
    catalog_query: String,
    mut combined: Vec<Track>,
    curated_batches: Vec<(String, Vec<Track>)>,
    mix_ctx: crate::taste::SearchMixContext,
    dz_albums: Vec<AlbumSummary>,
    dz_artists: Vec<ArtistSummary>,
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

    let instant_mix_json = refined.instant_mix.as_ref().map(|mix| {
        json!({
            "title": mix.title,
            "subtitle": mix.subtitle,
            "query": mix.query,
            "isThemeMix": mix.is_theme_mix,
            "prominent": mix.prominent,
            "totalDuration": mix.total_duration,
            "tracks": to_web_tracks_with_taste(&state, uid, &mix.tracks, &refined.taste_match_ids),
        })
    });

    json_res(json!({
        "query": query,
        "tracks": to_web_tracks_with_taste(&state, uid, &refined.tracks, &refined.taste_match_ids),
        "albums": albums.iter().map(|a| album_json(a, saved.contains(&a.id))).collect::<Vec<_>>(),
        "artists": artists.iter().map(|a| artist_json(a, followed.contains(&a.id))).collect::<Vec<_>>(),
        "instantMix": instant_mix_json,
        "refinements": refined.refinements,
    }))
}

pub async fn album_detail(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let Some((album, tracks)) = state.taste.get_or_hydrate_album(&id).await else {
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
    headers: HeaderMap,
) -> Response {
    let user = match require_web_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let Some((artist, top_tracks, albums, related)) =
        state.taste.get_or_hydrate_artist(&id, user.id).await
    else {
        return err_res(StatusCode::NOT_FOUND, "Artist not found");
    };
    let followed = state.store.followed_artist_ids(user.id).unwrap_or_default();
    let saved = state.store.saved_album_ids(user.id).unwrap_or_default();
    json_res(json!({
        "artist": artist_json(&artist, followed.contains(&artist.id)),
        "topTracks": to_web_tracks(&state, user.id, &top_tracks),
        "albums": albums.iter().map(|a| album_json(a, saved.contains(&a.id))).collect::<Vec<_>>(),
        "related": related.iter().map(|a| artist_json(a, followed.contains(&a.id))).collect::<Vec<_>>(),
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
        "playlists": playlists.iter().map(playlist_summary_json).collect::<Vec<_>>(),
        "likedTracks": to_web_tracks(&state, uid, &starred_tracks),
        "savedAlbums": saved_albums,
        "followedArtists": followed_artists,
        "recentTracks": to_web_tracks(&state, uid, &recent),
        "frequentTracks": to_web_tracks(&state, uid, &frequent),
    }))
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
        state.taste.refresh_auto_playlists_if_stale_async(user.id, 900).await;
        state.store.get_playlist(&id).ok().flatten().unwrap_or(pl)
    } else {
        pl
    };
    let tracks = state.store.playlist_tracks(&id).unwrap_or_default();
    json_res(json!({
        "playlist": playlist_summary_json(&pl),
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
            state.taste.record_album_save(user.id, &req.id, req.starred);
        }
        "artist" => {
            if req.starred {
                let _ = state.store.follow_artist(user.id, &req.id);
            } else {
                let _ = state.store.unfollow_artist(user.id, &req.id);
            }
            state.taste.record_artist_follow(user.id, &req.id, req.starred);
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
    state.taste.record_event_with_query(
        user.id,
        &req.track_id,
        &req.kind,
        req.value,
        req.query.as_deref(),
    );
    if let Ok(Some(t)) = state.store.get_track(&req.track_id) {
        if req.kind == "play" {
            let mut np = state.now_playing.lock().unwrap_or_else(|e| e.into_inner());
            np.insert(user.id, (t, Utc::now().timestamp(), user.username));
        }
    }
    json_res(json!({ "ok": true }))
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

    let next = if let Some(pid) = q.playlist.as_deref().filter(|s| !s.trim().is_empty()) {
        let Some(pl) = state.store.get_playlist(pid).ok().flatten() else {
            return err_res(StatusCode::NOT_FOUND, "Playlist not found");
        };
        if !pl.public && pl.user_id != user.id {
            return err_res(StatusCode::FORBIDDEN, "Permission denied");
        }
        state
            .taste
            .playlist_radio_tracks(user.id, pid, &exclude, 18)
            .await
    } else {
        let seed = q
            .seed
            .as_deref()
            .and_then(|id| state.store.get_track(id).ok().flatten());
        state
            .taste
            .radio_next_tracks_with_exclusions(user.id, seed.as_ref(), &exclude, 18)
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
    headers: HeaderMap,
) -> Response {
    if let Err(r) = require_web_user(&state, &headers) {
        return r;
    }
    if let Some(t) = state.store.get_track(&id).ok().flatten() {
        let engine = state.engine.clone();
        tokio::spawn(async move {
            let _ = engine.warm_track(&t).await;
        });
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
        "ffmpeg": state.engine.ffmpeg_available,
    }))
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
        let tok = store.create_session(u.id, "web", 3600).unwrap();
        let pl = store.create_playlist(u.id, "Funky Cuts", "", false, &[]).unwrap();

        let state = AppState {
            store: store.clone(),
            sources,
            engine,
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
        let tok = store.create_session(u.id, "web", 3600).unwrap();

        let state = AppState {
            store: store.clone(),
            sources,
            engine,
            taste,
            now_playing: Arc::new(Mutex::new(Default::default())),
        };
        let mut hdrs = HeaderMap::new();
        hdrs.insert(header::AUTHORIZATION, format!("Bearer {}", tok).parse().unwrap());

        let ev_res = record_event(
            State(state.clone()),
            hdrs,
            Json(EventReq {
                track_id: "loc:80s-1".into(),
                kind: "search_pick".into(),
                value: 1.0,
                query: Some("80s".into()),
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

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
