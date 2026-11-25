use crate::models::{AlbumSummary, ArtistSummary, PlayQueueState, Playlist, RadioStation, Track, User, UserTrackStats};
use crate::store::SessionMeta;
use crate::subsonic::{
    adjust_auto_playlist_summary, client_ip, dedup_tracks_by_song_key, filter_visible_tracks,
    hydrate_album_user_stats, hydrate_genre_tracks, refresh_user_mixes, resolve_genre_name,
    unified_catalog_search, user_agent, AppState,
};
use axum::extract::{Path, RawQuery, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{DateTime, Utc};
use md5::{Digest, Md5};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

#[derive(Deserialize, Default)]
pub struct LoginReq {
    pub username: String,
    pub password: String,
    /// Preferred explicit field. Many Navidrome clients only expose a password
    /// box, so the API key may also be supplied there.
    #[serde(default, rename = "apiKey")]
    pub api_key: String,
    /// Client-supplied name (Navidrome sends `name`, some clients send
    /// `client` or Subsonic's `c`). Used to label the session in Settings.
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub client: String,
    #[serde(default)]
    pub c: String,
}

impl LoginReq {
    fn client_name(&self) -> String {
        for candidate in [&self.name, &self.client, &self.c] {
            if !candidate.trim().is_empty() {
                return candidate.trim().chars().take(64).collect();
            }
        }
        String::new()
    }
}

#[derive(Deserialize)]
pub struct CreatePlaylistReq {
    pub name: String,
    #[serde(default)]
    pub comment: String,
    #[serde(default)]
    pub public: bool,
    #[serde(default)]
    pub songs: Vec<String>,
}

#[derive(Deserialize)]
pub struct UpdatePlaylistReq {
    pub name: Option<String>,
    pub comment: Option<String>,
    pub public: Option<bool>,
}

#[derive(Deserialize)]
pub struct AddTracksReq {
    #[serde(default)]
    pub ids: Vec<String>,
    #[serde(default, rename = "albumIds")]
    pub album_ids: Vec<String>,
    #[serde(default, rename = "artistIds")]
    pub artist_ids: Vec<String>,
}

#[derive(Deserialize)]
pub struct MoveTrackReq {
    pub insert_before: usize,
}

#[derive(Deserialize)]
pub struct SaveQueueReq {
    #[serde(default)]
    pub ids: Vec<String>,
    /// Navidrome clients send either a track ID string or a 0-based numeric index here.
    #[serde(default)]
    pub current: Option<Value>,
    #[serde(default, rename = "currentId")]
    pub current_id: Option<String>,
    #[serde(default)]
    pub position: i64,
}

#[derive(Deserialize)]
pub struct RadioReq {
    pub name: String,
    #[serde(rename = "streamUrl")]
    pub stream_url: String,
    #[serde(default, rename = "homePageUrl")]
    pub home_page_url: String,
}

#[derive(Deserialize)]
pub struct UpdateUserReq {
    pub password: Option<String>,
}

fn json_ok(val: Value, total_count: Option<usize>) -> Response {
    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json; charset=utf-8");
    if let Some(tot) = total_count {
        builder = builder.header("X-Total-Count", tot.to_string());
    }
    builder
        .body(axum::body::Body::from(val.to_string()))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn json_err(status: StatusCode, msg: &str) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        json!({ "error": msg, "message": msg }).to_string(),
    )
        .into_response()
}

pub fn extract_nd_token(headers: &HeaderMap) -> Option<String> {
    let raw = headers
        .get("x-nd-authorization")
        .or_else(|| headers.get(header::AUTHORIZATION))
        .and_then(|v| v.to_str().ok())?;
    let trimmed = raw.trim();
    if let Some(tok) = trimmed.strip_prefix("Bearer ").or_else(|| trimmed.strip_prefix("bearer ")) {
        Some(tok.trim().to_string())
    } else if !trimmed.is_empty() {
        Some(trimmed.to_string())
    } else {
        None
    }
}

pub fn require_nd_user(state: &AppState, headers: &HeaderMap) -> Result<User, Response> {
    let Some(token) = extract_nd_token(headers) else {
        return Err(json_err(StatusCode::UNAUTHORIZED, "Missing authorization token"));
    };
    if let Ok(Some(u)) = state.store.get_session_user(&token) {
        state
            .store
            .touch_session(&token, &user_agent(headers), &client_ip(headers));
        return Ok(u);
    }
    if let Ok(Some(u)) = state.store.get_user_by_api_key(&token) {
        return Ok(u);
    }
    Err(json_err(StatusCode::UNAUTHORIZED, "Session expired or invalid token"))
}

fn build_login_response(state: &AppState, user: &User, token: &str) -> Value {
    let salt = Uuid::new_v4().simple().to_string()[..10].to_string();
    let api_key = state.store.get_or_create_api_key(user.id).unwrap_or_default();
    let mut h = Md5::new();
    h.update(format!("{}{}", api_key, salt).as_bytes());
    let subsonic_token = hex::encode(h.finalize());

    json!({
        "id": user.id.to_string(),
        "name": user.username,
        "username": user.username,
        "isAdmin": false,
        "token": token,
        "subsonicSalt": salt,
        "subsonicToken": subsonic_token,
        "apiKey": api_key,
    })
}

/// Navidrome-compatible clients must authenticate with the account's API key,
/// never with a username/password pair. Most clients let the key be pasted into
/// the password field; an explicit `apiKey` field is accepted as well.
pub async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<LoginReq>,
) -> Response {
    let explicit_key = req.api_key.trim();
    let key = if explicit_key.is_empty() {
        req.password.trim()
    } else {
        explicit_key
    };
    if key.is_empty() {
        return json_err(
            StatusCode::UNAUTHORIZED,
            "API key required. Find it in Beatlab → Settings → Apps & API.",
        );
    }
    let rate_keys = crate::auth::rate_keys("apikey", "", &crate::subsonic::rate_limit_ip(&headers));
    if let Some(wait) = crate::auth::throttled_for(&rate_keys) {
        return json_err(StatusCode::TOO_MANY_REQUESTS, &crate::auth::throttle_message(wait));
    }
    let Some(user) = state.store.get_user_by_api_key(key).ok().flatten() else {
        crate::auth::record_failure(&rate_keys);
        return json_err(StatusCode::UNAUTHORIZED, "Invalid API key");
    };
    let supplied_name = req.username.trim();
    if !supplied_name.is_empty() && !supplied_name.eq_ignore_ascii_case(&user.username) {
        return json_err(StatusCode::UNAUTHORIZED, "API key does not match that username");
    }
    let meta = SessionMeta {
        user_agent: user_agent(&headers),
        ip: client_ip(&headers),
        client: req.client_name(),
    };
    match state
        .store
        .create_session_with_meta(user.id, "nd", &meta)
    {
        Ok(token) => json_ok(build_login_response(&state, &user, &token), None),
        Err(e) => json_err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

/// Client-side account creation is no longer allowed: accounts are created in
/// the Beatlab web player where the user can also configure 2FA.
pub async fn signup(
    State(_state): State<AppState>,
    _headers: HeaderMap,
    _req: Json<LoginReq>,
) -> Response {
    json_err(
        StatusCode::FORBIDDEN,
        "Create an account in the Beatlab web player, then use its API key in your music client.",
    )
}

pub async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(tok) = extract_nd_token(&headers) {
        let _ = state.store.delete_session(&tok);
    }
    json_ok(json!({ "ok": true }), None)
}

fn parse_query(raw: Option<&str>) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let Some(q) = raw else { return out };
    for (k, v) in crate::subsonic::parse_multi_params(q) {
        out.insert(k, v);
    }
    out
}

fn slice_params(q: &HashMap<String, String>, default_len: usize) -> (usize, usize) {
    let start: usize = q.get("_start").and_then(|s| s.parse().ok()).unwrap_or(0);
    let end: usize = q
        .get("_end")
        .and_then(|s| s.parse().ok())
        .unwrap_or(start + default_len);
    (start, end.saturating_sub(start).clamp(1, 500))
}

fn iso_opt(ts: i64) -> Option<String> {
    (ts > 0).then(|| {
        DateTime::<Utc>::from_timestamp(ts, 0)
            .unwrap_or_default()
            .to_rfc3339()
    })
}

fn iso_opt_or_now(ts: i64) -> String {
    let effective = if ts > 0 { ts } else { Utc::now().timestamp() };
    DateTime::<Utc>::from_timestamp(effective, 0)
        .unwrap_or_default()
        .to_rfc3339()
}

fn track_to_nd(
    t: &Track,
    starred_at: Option<i64>,
    stats: Option<&UserTrackStats>,
    rating: i32,
) -> Value {
    let st = stats.copied().unwrap_or_default();
    let album_artist = {
        let pa = crate::models::primary_artist(&t.artist);
        if pa.is_empty() { t.artist.clone() } else { pa }
    };
    let starred = starred_at.is_some();
    json!({
        "id": t.id,
        "libraryId": 1,
        "title": t.title,
        "album": t.album,
        "albumId": t.album_id(),
        "artist": t.artist,
        "artistId": t.artist_id(),
        "albumArtist": album_artist,
        "albumArtistId": t.artist_id(),
        "hasCoverArt": !t.cover_url.is_empty(),
        "trackNumber": t.track_no,
        "discNumber": t.disc_no.max(1),
        "year": t.year,
        "size": t.size.max(0),
        "suffix": if t.format.is_empty() { "m4a" } else { &t.format },
        "duration": t.duration.max(0) as f64,
        "bitRate": t.bitrate.max(128),
        "sampleRate": t.sample_rate,
        "bitDepth": t.bit_depth,
        "channels": 2,
        "bpm": 0,
        "genre": t.genre,
        "genres": if t.genre.is_empty() { Value::Array(vec![]) } else { json!([{ "id": crate::models::slug(&t.genre), "name": t.genre }]) },
        "orderTitle": t.title.to_lowercase(),
        "orderAlbumName": t.album.to_lowercase(),
        "orderArtistName": t.artist.to_lowercase(),
        "orderAlbumArtistName": album_artist.to_lowercase(),
        "compilation": false,
        "starred": starred,
        "starredAt": starred_at.map(iso_opt_or_now),
        "playCount": st.play_count,
        "playDate": iso_opt(st.last_played),
        "rating": rating,
        "createdAt": iso_opt(t.created),
        "updatedAt": iso_opt(t.created),
    })
}

fn album_to_nd(a: &AlbumSummary, saved_at: Option<i64>, rating: i32) -> Value {
    let saved = saved_at.is_some();
    json!({
        "id": a.id,
        "libraryId": 1,
        "name": a.name,
        "albumArtist": a.artist,
        "albumArtistId": a.artist_id,
        "artist": a.artist,
        "artistId": a.artist_id,
        "coverArtId": a.id,
        "maxYear": a.year,
        "minYear": a.year,
        "songCount": a.song_count,
        "duration": a.duration as f64,
        "size": 0,
        "playCount": a.play_count,
        "playDate": iso_opt(a.last_played),
        "rating": rating,
        "starred": saved,
        "starredAt": saved_at.map(iso_opt_or_now),
        "genre": a.genre,
        "genres": if a.genre.is_empty() { Value::Array(vec![]) } else { json!([{ "id": crate::models::slug(&a.genre), "name": a.genre }]) },
        "orderAlbumName": a.name.to_lowercase(),
        "orderArtistName": a.artist.to_lowercase(),
        "orderAlbumArtistName": a.artist.to_lowercase(),
        "compilation": false,
        "createdAt": iso_opt(a.created),
        "updatedAt": iso_opt(a.created),
    })
}

fn artist_to_nd(a: &ArtistSummary, followed_at: Option<i64>) -> Value {
    let followed = followed_at.is_some();
    json!({
        "id": a.id,
        "libraryId": 1,
        "name": a.name,
        "orderArtistName": a.name.to_lowercase(),
        "albumCount": a.album_count,
        "songCount": a.song_count,
        "biography": a.bio,
        "starred": followed,
        "starredAt": followed_at.map(iso_opt_or_now),
        "smallImageUrl": a.cover_url,
        "mediumImageUrl": a.cover_url,
        "largeImageUrl": a.cover_url,
    })
}

fn playlist_to_nd(p: &Playlist) -> Value {
    json!({
        "id": p.id,
        "name": p.name,
        "comment": p.comment,
        "duration": p.duration as f64,
        "songCount": p.track_count,
        "ownerId": p.user_id.to_string(),
        "ownerName": p.owner_name,
        "public": p.public,
        "sync": false,
        "rules": null,
        "createdAt": iso_opt(p.created),
        "updatedAt": iso_opt(p.changed),
    })
}

fn radio_to_nd(s: &RadioStation) -> Value {
    json!({
        "id": s.id,
        "name": s.name,
        "streamUrl": s.stream_url,
        "homePageUrl": s.home_page_url,
    })
}

fn album_ratings_map(state: &AppState, ratings: &HashMap<String, i32>) -> HashMap<String, i32> {
    if ratings.is_empty() {
        return HashMap::new();
    }
    let mut accum: HashMap<String, (i64, i64)> = HashMap::new();
    for (tid, &r) in ratings {
        if r <= 0 {
            continue;
        }
        if let Ok(Some(t)) = state.store.get_track(tid) {
            let e = accum.entry(t.album_id()).or_insert((0, 0));
            e.0 += r as i64;
            e.1 += 1;
        }
    }
    accum
        .into_iter()
        .map(|(aid, (sum, cnt))| (aid, ((sum as f64 / cnt.max(1) as f64).round() as i32).clamp(0, 5)))
        .collect()
}

pub async fn list_songs(
    State(state): State<AppState>,
    RawQuery(raw): RawQuery,
    headers: HeaderMap,
) -> Response {
    let user = match require_nd_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let q = parse_query(raw.as_deref());
    let (start, count) = slice_params(&q, 50);
    let starred = state.store.starred_at_map(user.id).unwrap_or_default();
    let stats = state.store.track_stats_map(user.id).unwrap_or_default();
    let ratings = state.store.user_ratings(user.id).unwrap_or_default();
    let only_starred = q.get("starred").map(|v| v == "true").unwrap_or(false);

    let mut list: Vec<Track> = if let Some(aid) = q.get("album_id").filter(|s| !s.is_empty()) {
        if let Some((_, tracks)) = state.taste.get_or_hydrate_album(aid).await {
            tracks
        } else {
            state.store.album_tracks(aid).unwrap_or_default()
        }
    } else if let Some(arid) = q
        .get("artist_id")
        .or_else(|| q.get("album_artist_id"))
        .filter(|s| !s.is_empty())
    {
        let mut tracks = Vec::new();
        if let Some((_, top, _, _)) = state.taste.get_or_hydrate_artist(arid, user.id).await {
            tracks.extend(top);
        }
        tracks.extend(state.store.artist_tracks(arid).unwrap_or_default());
        dedup_tracks_by_song_key(tracks)
    } else if let Some(term) = q.get("title").or_else(|| q.get("q")).filter(|s| !s.is_empty()) {
        unified_catalog_search(&state, user.id, term).await.0
    } else if let Some(g) = q
        .get("genre")
        .or_else(|| q.get("genre_id"))
        .filter(|s| !s.is_empty())
    {
        hydrate_genre_tracks(&state, user.id, g, 300, 0).await
    } else if only_starred {
        state
            .store
            .starred_tracks(user.id)
            .unwrap_or_default()
            .into_iter()
            .map(|(t, _)| t)
            .collect()
    } else {
        filter_visible_tracks(
            &state,
            user.id,
            state.store.all_tracks(1000, 0).unwrap_or_default(),
        )
    };

    if only_starred {
        list.retain(|t| starred.contains_key(&t.id));
    }
    if q.get("has_rating").map(|v| v == "true").unwrap_or(false) {
        list.retain(|t| ratings.get(&t.id).copied().unwrap_or(0) > 0);
    }
    if let Some(min_r) = q.get("rating").and_then(|s| s.parse::<i32>().ok()).filter(|&r| r > 0) {
        list.retain(|t| ratings.get(&t.id).copied().unwrap_or(0) >= min_r);
    }
    if let Some(yr) = q.get("year").and_then(|s| s.parse::<i32>().ok()).filter(|&y| y > 0) {
        list.retain(|t| t.year == yr);
    }

    let sort = q.get("_sort").map(|s| s.as_str()).unwrap_or("");
    let desc = q
        .get("_order")
        .map(|o| o.eq_ignore_ascii_case("DESC"))
        .unwrap_or(false);
    match sort {
        "title" => list.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase())),
        "artist" | "album_artist" => {
            list.sort_by(|a, b| a.artist.to_lowercase().cmp(&b.artist.to_lowercase()))
        }
        "album" => list.sort_by(|a, b| {
            a.album
                .to_lowercase()
                .cmp(&b.album.to_lowercase())
                .then_with(|| a.disc_no.cmp(&b.disc_no))
                .then_with(|| a.track_no.cmp(&b.track_no))
        }),
        "track" | "disc_number" => list.sort_by(|a, b| {
            a.disc_no
                .cmp(&b.disc_no)
                .then_with(|| a.track_no.cmp(&b.track_no))
                .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
        }),
        "duration" => list.sort_by_key(|t| t.duration),
        "year" => list.sort_by_key(|t| t.year),
        "rating" => list.sort_by_key(|t| ratings.get(&t.id).copied().unwrap_or(0)),
        "play_count" => list.sort_by_key(|t| stats.get(&t.id).map(|s| s.play_count).unwrap_or(0)),
        "play_date" | "recently_played" => {
            list.sort_by_key(|t| stats.get(&t.id).map(|s| s.last_played).unwrap_or(0))
        }
        "recently_added" | "created_at" => list.sort_by_key(|t| t.created),
        "starred_at" => list.sort_by_key(|t| starred.get(&t.id).copied().unwrap_or(0)),
        "random" => {
            use rand::seq::SliceRandom;
            let mut rng = rand::thread_rng();
            list.shuffle(&mut rng);
        }
        _ => {}
    }
    if desc && !sort.is_empty() && sort != "random" {
        list.reverse();
    }

    let total = list.len();
    let items: Vec<Value> = list
        .into_iter()
        .skip(start)
        .take(count)
        .map(|t| {
            track_to_nd(
                &t,
                starred.get(&t.id).copied(),
                stats.get(&t.id),
                ratings.get(&t.id).copied().unwrap_or(0),
            )
        })
        .collect();
    json_ok(Value::Array(items), Some(total))
}

pub async fn get_song(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let user = match require_nd_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let Some(t) = state.store.get_track(&id).ok().flatten() else {
        return json_err(StatusCode::NOT_FOUND, "Song not found");
    };
    let starred = state.store.starred_at_map(user.id).unwrap_or_default();
    let stats = state.store.get_user_track_stats(user.id, &t.id).ok();
    let rating = state.store.user_ratings(user.id).ok().and_then(|m| m.get(&t.id).copied()).unwrap_or(0);
    json_ok(track_to_nd(&t, starred.get(&t.id).copied(), stats.as_ref(), rating), None)
}

pub async fn list_albums(
    State(state): State<AppState>,
    RawQuery(raw): RawQuery,
    headers: HeaderMap,
) -> Response {
    let user = match require_nd_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let q = parse_query(raw.as_deref());
    let (start, count) = slice_params(&q, 50);
    let saved = state.store.saved_album_at_map(user.id).unwrap_or_default();
    let ratings = state.store.user_ratings(user.id).unwrap_or_default();
    let album_ratings = album_ratings_map(&state, &ratings);

    let mut list: Vec<AlbumSummary> = if let Some(arid) = q
        .get("artist_id")
        .or_else(|| q.get("album_artist_id"))
        .filter(|s| !s.is_empty())
    {
        if let Some((_, _, mut albums, _)) = state.taste.get_or_hydrate_artist(arid, user.id).await {
            let user_stats: HashMap<String, (i64, i64)> = state
                .store
                .build_album_summaries(user.id)
                .unwrap_or_default()
                .into_iter()
                .map(|a| (a.id, (a.play_count, a.last_played)))
                .collect();
            for a in &mut albums {
                if let Some(&(pc, lp)) = user_stats.get(&a.id) {
                    if a.play_count == 0 {
                        a.play_count = pc;
                    }
                    if a.last_played == 0 {
                        a.last_played = lp;
                    }
                }
            }
            albums
        } else {
            state
                .store
                .build_album_summaries(user.id)
                .unwrap_or_default()
                .into_iter()
                .filter(|a| &a.artist_id == arid)
                .collect()
        }
    } else if let Some(term) = q.get("name").or_else(|| q.get("q")).filter(|s| !s.is_empty()) {
        unified_catalog_search(&state, user.id, term).await.1
    } else {
        state.store.build_album_summaries(user.id).unwrap_or_default()
    };

    if let Some(g) = q
        .get("genre")
        .or_else(|| q.get("genre_id"))
        .filter(|s| !s.is_empty())
    {
        let resolved = resolve_genre_name(&state, g).to_lowercase();
        list.retain(|a| a.genre.to_lowercase().contains(&resolved));
    }
    if q.get("starred").map(|v| v == "true").unwrap_or(false) {
        list.retain(|a| saved.contains_key(&a.id));
    }
    if q.get("recently_played").map(|v| v == "true").unwrap_or(false) {
        list.retain(|a| a.last_played > 0);
    }
    if q.get("has_rating").map(|v| v == "true").unwrap_or(false) {
        list.retain(|a| album_ratings.get(&a.id).copied().unwrap_or(0) > 0);
    }

    let sort = q.get("_sort").map(|s| s.as_str()).unwrap_or("");
    let desc = q.get("_order").map(|o| o.eq_ignore_ascii_case("DESC")).unwrap_or(false);
    match sort {
        "name" => list.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
        "artist" | "album_artist" => list.sort_by(|a, b| {
            a.artist
                .to_lowercase()
                .cmp(&b.artist.to_lowercase())
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        }),
        "max_year" | "min_year" | "year" => list.sort_by_key(|a| a.year),
        "play_count" => list.sort_by_key(|a| a.play_count),
        "play_date" | "recently_played" => list.sort_by_key(|a| a.last_played),
        "recently_added" | "created_at" => list.sort_by_key(|a| (a.song_count > 0, a.created)),
        "duration" => list.sort_by_key(|a| a.duration),
        "song_count" => list.sort_by_key(|a| a.song_count),
        "rating" => list.sort_by_key(|a| album_ratings.get(&a.id).copied().unwrap_or(0)),
        "starred_at" => list.sort_by_key(|a| saved.get(&a.id).copied().unwrap_or(0)),
        "random" => {
            use rand::seq::SliceRandom;
            let mut rng = rand::thread_rng();
            list.shuffle(&mut rng);
        }
        _ => {}
    }
    if desc && !sort.is_empty() && sort != "random" {
        list.reverse();
    }

    let total = list.len();
    let items: Vec<Value> = list
        .into_iter()
        .skip(start)
        .take(count)
        .map(|a| {
            let saved_at = saved.get(&a.id).copied();
            let r = album_ratings.get(&a.id).copied().unwrap_or(0);
            album_to_nd(&a, saved_at, r)
        })
        .collect();
    json_ok(Value::Array(items), Some(total))
}

pub async fn get_album(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let user = match require_nd_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let Some((mut alb, tracks)) = state.taste.get_or_hydrate_album(&id).await else {
        return json_err(StatusCode::NOT_FOUND, "Album not found");
    };
    let saved = state.store.saved_album_at_map(user.id).unwrap_or_default();
    let stats = state.store.track_stats_map(user.id).unwrap_or_default();
    let ratings = state.store.user_ratings(user.id).unwrap_or_default();
    hydrate_album_user_stats(&mut alb, &tracks, &stats);
    let rated_vals: Vec<i32> = tracks
        .iter()
        .filter_map(|t| ratings.get(&t.id).copied().filter(|&r| r > 0))
        .collect();
    let avg_rating = if rated_vals.is_empty() {
        0
    } else {
        let sum: i64 = rated_vals.iter().map(|&r| r as i64).sum();
        ((sum as f64 / rated_vals.len() as f64).round() as i32).clamp(0, 5)
    };
    let saved_at = saved.get(&alb.id).copied();
    json_ok(album_to_nd(&alb, saved_at, avg_rating), None)
}

pub async fn list_artists(
    State(state): State<AppState>,
    RawQuery(raw): RawQuery,
    headers: HeaderMap,
) -> Response {
    let user = match require_nd_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let q = parse_query(raw.as_deref());
    let (start, count) = slice_params(&q, 50);
    let followed = state.store.followed_artist_at_map(user.id).unwrap_or_default();

    let mut list = if let Some(term) = q.get("name").or_else(|| q.get("q")).filter(|s| !s.is_empty()) {
        unified_catalog_search(&state, user.id, term).await.2
    } else {
        state.store.build_artist_summaries().unwrap_or_default()
    };

    if q.get("starred").map(|v| v == "true").unwrap_or(false) {
        list.retain(|a| followed.contains_key(&a.id));
    }

    let sort = q.get("_sort").map(|s| s.as_str()).unwrap_or("");
    let desc = q
        .get("_order")
        .map(|o| o.eq_ignore_ascii_case("DESC"))
        .unwrap_or(false);
    match sort {
        "name" => list.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
        "album_count" => list.sort_by_key(|a| a.album_count),
        "song_count" => list.sort_by_key(|a| a.song_count),
        "starred_at" => list.sort_by_key(|a| followed.get(&a.id).copied().unwrap_or(0)),
        "random" => {
            use rand::seq::SliceRandom;
            let mut rng = rand::thread_rng();
            list.shuffle(&mut rng);
        }
        _ => {}
    }
    if desc && !sort.is_empty() && sort != "random" {
        list.reverse();
    }

    let total = list.len();
    let items: Vec<Value> = list
        .into_iter()
        .skip(start)
        .take(count)
        .map(|a| {
            let f_at = followed.get(&a.id).copied();
            artist_to_nd(&a, f_at)
        })
        .collect();
    json_ok(Value::Array(items), Some(total))
}

pub async fn get_artist(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let user = match require_nd_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let Some((art, _, _, _)) = state.taste.get_or_hydrate_artist(&id, user.id).await else {
        return json_err(StatusCode::NOT_FOUND, "Artist not found");
    };
    let followed = state.store.followed_artist_at_map(user.id).unwrap_or_default();
    let f_at = followed.get(&art.id).copied();
    json_ok(artist_to_nd(&art, f_at), None)
}

pub async fn list_genres(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    if let Err(r) = require_nd_user(&state, &headers) {
        return r;
    }
    let genres = state.store.genres().unwrap_or_default();
    let total = genres.len();
    let items: Vec<Value> = genres
        .into_iter()
        .map(|(name, songs, albums)| {
            json!({
                "id": crate::models::slug(&name),
                "name": name,
                "songCount": songs,
                "albumCount": albums,
            })
        })
        .collect();
    json_ok(Value::Array(items), Some(total))
}

pub async fn list_playlists(
    State(state): State<AppState>,
    RawQuery(raw): RawQuery,
    headers: HeaderMap,
) -> Response {
    let user = match require_nd_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    refresh_user_mixes(&state, user.id).await;
    let q = parse_query(raw.as_deref());
    let (start, count) = slice_params(&q, 100);
    let mut list = state.store.list_playlists(user.id, true).unwrap_or_default();
    for p in &mut list {
        adjust_auto_playlist_summary(&state, user.id, p);
    }
    if let Some(term) = q.get("name").or_else(|| q.get("q")).filter(|s| !s.is_empty()) {
        let low = term.to_lowercase();
        list.retain(|p| p.name.to_lowercase().contains(&low) || p.comment.to_lowercase().contains(&low));
    }
    let sort = q.get("_sort").map(|s| s.as_str()).unwrap_or("");
    let desc = q
        .get("_order")
        .map(|o| o.eq_ignore_ascii_case("DESC"))
        .unwrap_or(false);
    match sort {
        "name" => list.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
        "songCount" | "song_count" => list.sort_by_key(|p| p.track_count),
        "duration" => list.sort_by_key(|p| p.duration),
        "updatedAt" | "updated_at" => list.sort_by_key(|p| p.changed),
        "createdAt" | "created_at" => list.sort_by_key(|p| p.created),
        "ownerName" | "owner_name" => {
            list.sort_by(|a, b| a.owner_name.to_lowercase().cmp(&b.owner_name.to_lowercase()))
        }
        _ => {}
    }
    if desc && !sort.is_empty() {
        list.reverse();
    }
    let total = list.len();
    let items: Vec<Value> = list
        .into_iter()
        .skip(start)
        .take(count)
        .map(|p| playlist_to_nd(&p))
        .collect();
    json_ok(Value::Array(items), Some(total))
}

pub async fn get_playlist(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let user = match require_nd_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let Some(p) = state.store.get_playlist(&id).ok().flatten() else {
        return json_err(StatusCode::NOT_FOUND, "Playlist not found");
    };
    if !p.public && p.user_id != user.id {
        return json_err(StatusCode::FORBIDDEN, "Permission denied");
    }
    let mut p = if p.is_auto && p.user_id == user.id && state.store.count_tracks().unwrap_or(0) > 0 {
        if p.kind.starts_with("mixed_for_you:") {
            state.taste.refresh_mixed_for_you_if_stale_async(user.id).await;
        }
        state.taste.refresh_auto_playlists_if_stale_async(user.id, 900).await;
        state.store.get_playlist(&id).ok().flatten().unwrap_or(p)
    } else {
        p
    };
    adjust_auto_playlist_summary(&state, user.id, &mut p);
    json_ok(playlist_to_nd(&p), None)
}

pub async fn create_playlist(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreatePlaylistReq>,
) -> Response {
    let user = match require_nd_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let name = if req.name.trim().is_empty() {
        "New Playlist"
    } else {
        req.name.trim()
    };
    match state
        .store
        .create_playlist(user.id, name, &req.comment, req.public, &req.songs)
    {
        Ok(p) => {
            if !req.songs.is_empty() {
                state.taste.record_playlist_additions(user.id, &req.songs);
            }
            json_ok(playlist_to_nd(&p), None)
        }
        Err(e) => json_err(StatusCode::BAD_REQUEST, &e.to_string()),
    }
}

pub async fn update_playlist(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<UpdatePlaylistReq>,
) -> Response {
    let user = match require_nd_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let Some(p) = state.store.get_playlist(&id).ok().flatten() else {
        return json_err(StatusCode::NOT_FOUND, "Playlist not found");
    };
    if p.user_id != user.id {
        return json_err(StatusCode::FORBIDDEN, "Permission denied");
    }
    let _ = state.store.update_playlist(
        &id,
        req.name.as_deref(),
        req.comment.as_deref(),
        req.public,
    );
    let Some(updated) = state.store.get_playlist(&id).ok().flatten() else {
        return json_err(StatusCode::NOT_FOUND, "Playlist not found");
    };
    json_ok(playlist_to_nd(&updated), None)
}

pub async fn delete_playlist(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let user = match require_nd_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let Some(p) = state.store.get_playlist(&id).ok().flatten() else {
        return json_err(StatusCode::NOT_FOUND, "Playlist not found");
    };
    if p.user_id != user.id {
        return json_err(StatusCode::FORBIDDEN, "Permission denied");
    }
    let _ = state.store.delete_playlist(&id);
    if !p.is_auto {
        state.taste.mark_taste_dirty(user.id);
    }
    json_ok(json!({ "id": id }), None)
}

pub async fn get_playlist_tracks(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
    headers: HeaderMap,
) -> Response {
    let user = match require_nd_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let Some(p) = state.store.get_playlist(&id).ok().flatten() else {
        return json_err(StatusCode::NOT_FOUND, "Playlist not found");
    };
    if !p.public && p.user_id != user.id {
        return json_err(StatusCode::FORBIDDEN, "Permission denied");
    }
    if p.is_auto && p.user_id == user.id && state.store.count_tracks().unwrap_or(0) > 0 {
        if p.kind.starts_with("mixed_for_you:") {
            state.taste.refresh_mixed_for_you_if_stale_async(user.id).await;
        }
        state.taste.refresh_auto_playlists_if_stale_async(user.id, 900).await;
    }
    let q = parse_query(raw.as_deref());
    let (start, count) = slice_params(&q, 500);
    let raw_tracks = state.store.playlist_tracks(&id).unwrap_or_default();
    let tracks = if p.is_auto {
        filter_visible_tracks(&state, user.id, raw_tracks)
    } else {
        raw_tracks
    };
    let starred = state.store.starred_at_map(user.id).unwrap_or_default();
    let stats = state.store.track_stats_map(user.id).unwrap_or_default();
    let ratings = state.store.user_ratings(user.id).unwrap_or_default();
    let total = tracks.len();
    let items: Vec<Value> = tracks
        .into_iter()
        .enumerate()
        .skip(start)
        .take(count)
        .map(|(pos, t)| {
            let mut val = track_to_nd(
                &t,
                starred.get(&t.id).copied(),
                stats.get(&t.id),
                ratings.get(&t.id).copied().unwrap_or(0),
            );
            if let Some(obj) = val.as_object_mut() {
                obj.insert("mediaFileId".into(), Value::String(t.id.clone()));
                obj.insert("playlistId".into(), Value::String(id.clone()));
                obj.insert("id".into(), Value::String((pos + 1).to_string()));
            }
            val
        })
        .collect();
    json_ok(Value::Array(items), Some(total))
}

pub async fn add_playlist_tracks(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<AddTracksReq>,
) -> Response {
    let user = match require_nd_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let Some(p) = state.store.get_playlist(&id).ok().flatten() else {
        return json_err(StatusCode::NOT_FOUND, "Playlist not found");
    };
    if p.user_id != user.id {
        return json_err(StatusCode::FORBIDDEN, "Permission denied");
    }
    let mut to_add = req.ids;
    for aid in req.album_ids {
        for t in state.store.album_tracks(&aid).unwrap_or_default() {
            to_add.push(t.id);
        }
    }
    for arid in req.artist_ids {
        for t in state.store.artist_tracks(&arid).unwrap_or_default() {
            to_add.push(t.id);
        }
    }
    let added = state
        .store
        .add_unique_tracks_to_playlist(&id, &to_add)
        .unwrap_or_default();
    if !p.is_auto && !added.is_empty() {
        state.taste.record_playlist_additions(user.id, &added);
    }
    json_ok(json!({ "added": added.len() }), None)
}

pub async fn remove_playlist_tracks(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
    headers: HeaderMap,
) -> Response {
    let user = match require_nd_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let Some(p) = state.store.get_playlist(&id).ok().flatten() else {
        return json_err(StatusCode::NOT_FOUND, "Playlist not found");
    };
    if p.user_id != user.id {
        return json_err(StatusCode::FORBIDDEN, "Permission denied");
    }
    let pairs = raw
        .as_deref()
        .map(crate::subsonic::parse_multi_params)
        .unwrap_or_default();
    let target_ids: HashSet<String> = pairs
        .into_iter()
        .filter(|(k, _)| k == "id")
        .map(|(_, v)| v)
        .collect();
    let current = state.store.playlist_tracks(&id).unwrap_or_default();
    let mut positions = Vec::new();
    let mut removed_ids = Vec::new();
    for (idx, t) in current.iter().enumerate() {
        if target_ids.contains(&t.id) || target_ids.contains(&(idx + 1).to_string()) {
            positions.push(idx);
            removed_ids.push(t.id.clone());
        }
    }
    let _ = state.store.remove_playlist_positions(&id, &positions);
    if !p.is_auto && !removed_ids.is_empty() {
        state.taste.record_playlist_removals(user.id, &removed_ids);
    }
    json_ok(json!({ "ids": target_ids.into_iter().collect::<Vec<_>>() }), None)
}

pub async fn move_playlist_track(
    State(state): State<AppState>,
    Path((id, pos_str)): Path<(String, String)>,
    headers: HeaderMap,
    Json(req): Json<MoveTrackReq>,
) -> Response {
    let user = match require_nd_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let Some(p) = state.store.get_playlist(&id).ok().flatten() else {
        return json_err(StatusCode::NOT_FOUND, "Playlist not found");
    };
    if p.user_id != user.id {
        return json_err(StatusCode::FORBIDDEN, "Permission denied");
    }
    let current = state.store.playlist_tracks(&id).unwrap_or_default();
    if current.is_empty() {
        return json_err(StatusCode::BAD_REQUEST, "Playlist is empty");
    }
    let from_pos = match pos_str.parse::<usize>() {
        Ok(n) if n >= 1 && n <= current.len() => n - 1,
        Ok(_) => return json_err(StatusCode::BAD_REQUEST, "Track position out of range"),
        Err(_) => match current.iter().position(|t| t.id == pos_str) {
            Some(idx) => idx,
            None => return json_err(StatusCode::BAD_REQUEST, "Invalid track position"),
        },
    };
    if req.insert_before > current.len() + 1 {
        return json_err(StatusCode::BAD_REQUEST, "Target position out of range");
    }
    // `insert_before` is the 1-based position before which the track is inserted (`len + 1` = end).
    let before_idx = req.insert_before.saturating_sub(1).min(current.len());
    let to_pos = if from_pos < before_idx {
        before_idx.saturating_sub(1)
    } else {
        before_idx.min(current.len().saturating_sub(1))
    };
    match state.store.move_playlist_track(&id, from_pos, to_pos) {
        Ok(()) => json_ok(json!({ "ok": true }), None),
        Err(e) => json_err(StatusCode::BAD_REQUEST, &e.to_string()),
    }
}

pub async fn get_queue(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let user = match require_nd_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let Some((q, tracks, resolved_idx, resolved_id)) =
        state.store.load_play_queue_tracks(user.id).ok().flatten()
    else {
        return json_ok(
            json!({ "id": user.id.to_string(), "current": 0, "position": 0, "items": [] }),
            None,
        );
    };
    let starred = state.store.starred_at_map(user.id).unwrap_or_default();
    let stats = state.store.track_stats_map(user.id).unwrap_or_default();
    let ratings = state.store.user_ratings(user.id).unwrap_or_default();
    let items: Vec<Value> = tracks
        .iter()
        .map(|t| {
            track_to_nd(
                t,
                starred.get(&t.id).copied(),
                stats.get(&t.id),
                ratings.get(&t.id).copied().unwrap_or(0),
            )
        })
        .collect();
    json_ok(
        json!({
            "id": user.id.to_string(),
            "userId": user.id.to_string(),
            "current": resolved_idx,
            "currentId": resolved_id,
            "position": q.position_ms,
            "items": items,
            "changedBy": q.changed_by,
        }),
        None,
    )
}

pub async fn save_queue(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<SaveQueueReq>,
) -> Response {
    let user = match require_nd_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let (cur_id, cur_idx) = if let Some(cid) = req.current_id.filter(|s| !s.is_empty()) {
        let idx = req.ids.iter().position(|id| id == &cid).unwrap_or(0);
        (cid, idx)
    } else if let Some(cur) = req.current {
        match cur {
            Value::Number(n) => {
                let idx = n.as_u64().unwrap_or(0) as usize;
                let cid = req.ids.get(idx).cloned().unwrap_or_default();
                (cid, idx)
            }
            Value::String(cid) => {
                let idx = req.ids.iter().position(|id| id == &cid).unwrap_or(0);
                (cid, idx)
            }
            _ => (req.ids.first().cloned().unwrap_or_default(), 0),
        }
    } else {
        (req.ids.first().cloned().unwrap_or_default(), 0)
    };
    let _ = state.store.save_play_queue(
        user.id,
        &PlayQueueState {
            current_id: cur_id,
            current_index: cur_idx,
            position_ms: req.position,
            track_ids: req.ids,
            changed_by: "navidrome".to_string(),
            updated: Utc::now().timestamp(),
        },
    );
    json_ok(json!({ "ok": true }), None)
}

pub async fn list_radios(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let user = match require_nd_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let stations = state.store.list_radio_stations(user.id).unwrap_or_default();
    let total = stations.len();
    let items: Vec<Value> = stations.iter().map(radio_to_nd).collect();
    json_ok(Value::Array(items), Some(total))
}

pub async fn create_radio(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<RadioReq>,
) -> Response {
    let user = match require_nd_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    match state.store.upsert_radio_station(
        user.id,
        &RadioStation {
            id: String::new(),
            name: req.name,
            stream_url: req.stream_url,
            home_page_url: req.home_page_url,
        },
    ) {
        Ok(s) => json_ok(radio_to_nd(&s), None),
        Err(e) => json_err(StatusCode::BAD_REQUEST, &e.to_string()),
    }
}

pub async fn update_radio(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<RadioReq>,
) -> Response {
    let user = match require_nd_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    match state.store.upsert_radio_station(
        user.id,
        &RadioStation {
            id,
            name: req.name,
            stream_url: req.stream_url,
            home_page_url: req.home_page_url,
        },
    ) {
        Ok(s) => json_ok(radio_to_nd(&s), None),
        Err(e) => json_err(StatusCode::BAD_REQUEST, &e.to_string()),
    }
}

pub async fn delete_radio(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let user = match require_nd_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let _ = state.store.delete_radio_station(user.id, &id);
    json_ok(json!({ "id": id }), None)
}

pub async fn list_users(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let user = match require_nd_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    json_ok(
        json!([{
            "id": user.id.to_string(),
            "userName": user.username,
            "name": user.username,
            "isAdmin": false,
            "createdAt": iso_opt(user.created),
        }]),
        Some(1),
    )
}

pub async fn get_user(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let user = match require_nd_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    if id != user.id.to_string() {
        return json_err(StatusCode::FORBIDDEN, "Can only view your own account");
    }
    json_ok(
        json!({
            "id": user.id.to_string(),
            "userName": user.username,
            "name": user.username,
            "isAdmin": false,
            "createdAt": iso_opt(user.created),
        }),
        None,
    )
}

pub async fn update_user(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<UpdateUserReq>,
) -> Response {
    let user = match require_nd_user(&state, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    if id != user.id.to_string() {
        return json_err(StatusCode::FORBIDDEN, "Can only update your own account");
    }
    // Navidrome sessions are minted from the API key; like the Subsonic
    // `changePassword` endpoint, they must not be able to replace the account
    // password and so sidestep the web player's password + 2FA checks.
    if req.password.as_deref().is_some_and(|pw| !pw.is_empty()) {
        return json_err(
            StatusCode::FORBIDDEN,
            "Change your password in the Beatlab web player (Settings → Security)",
        );
    }
    json_ok(
        json!({
            "id": user.id.to_string(),
            "userName": user.username,
            "name": user.username,
            "isAdmin": false,
        }),
        None,
    )
}

pub async fn keepalive(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = require_nd_user(&state, &headers) {
        return r;
    }
    json_ok(json!({ "response": "ok" }), None)
}

#[cfg(test)]
mod tests {
    use super::*;
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

    fn nd_test_state() -> AppState {
        let store = Store::open_memory().unwrap();
        let tmp = std::env::temp_dir().join(format!("beatlab-nd-{}", Uuid::new_v4()));
        let sources = SourceRegistry::new(
            DeezerSource::new(),
            YtmSource::new(None),
            ArchiveSource::new(),
            Arc::new(LocalSource::new(store.clone(), vec![])),
            SoulseekSource::new(store.clone(), tmp.join("slsk")),
        );
        let engine = Arc::new(StreamEngine::new(store.clone(), sources.clone(), &tmp, 1.0));
        let taste = Arc::new(TasteEngine::new(store.clone(), sources.clone()));
        store
            .upsert_tracks(&[Track {
                id: "t1".into(),
                source: "local".into(),
                title: "Starboy".into(),
                artist: "The Weeknd".into(),
                album: "Starboy".into(),
                genre: "Pop".into(),
                year: 2016,
                duration: 230,
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

    #[tokio::test]
    async fn navidrome_login_auto_creates_user_and_playlist_crud_works() {
        let st = nd_test_state();
        // Clients authenticate with the API key, never with the account password.
        let user = st.store.create_user("maya", "password123").unwrap();
        let api_key = st.store.get_or_create_api_key(user.id).unwrap();
        let resp = login(
            State(st.clone()),
            HeaderMap::new(),
            Json(LoginReq {
                username: "maya".into(),
                password: api_key,
                ..Default::default()
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body: Value =
            serde_json::from_slice(&to_bytes(resp.into_body(), 1024 * 1024).await.unwrap())
                .unwrap();
        assert_eq!(body["username"], "maya");
        assert_eq!(body["isAdmin"], false);
        let tok = body["token"].as_str().unwrap().to_string();

        // Wrong password rejects
        let bad = login(
            State(st.clone()),
            HeaderMap::new(),
            Json(LoginReq {
                username: "maya".into(),
                password: "wrong".into(),
                ..Default::default()
            }),
        )
        .await;
        assert_eq!(bad.status(), StatusCode::UNAUTHORIZED);

        let mut hdrs = HeaderMap::new();
        hdrs.insert("x-nd-authorization", format!("Bearer {}", tok).parse().unwrap());

        let created = create_playlist(
            State(st.clone()),
            hdrs.clone(),
            Json(CreatePlaylistReq {
                name: "Night Drive".into(),
                comment: "Late vibes".into(),
                public: false,
                songs: vec!["t1".into()],
            }),
        )
        .await;
        let pl: Value =
            serde_json::from_slice(&to_bytes(created.into_body(), 1024 * 1024).await.unwrap())
                .unwrap();
        assert_eq!(pl["name"], "Night Drive");
        assert_eq!(pl["songCount"], 1);
    }

    #[tokio::test]
    async fn navidrome_move_playlist_track_handles_insert_before_and_end_of_list() {
        let st = nd_test_state();
        st.store
            .upsert_tracks(&[
                Track {
                    id: "t2".into(),
                    source: "local".into(),
                    title: "Blinding Lights".into(),
                    artist: "The Weeknd".into(),
                    duration: 200,
                    ..Default::default()
                },
                Track {
                    id: "t3".into(),
                    source: "local".into(),
                    title: "After Hours".into(),
                    artist: "The Weeknd".into(),
                    duration: 360,
                    ..Default::default()
                },
            ])
            .unwrap();
        let (u, _) = st.store.authenticate_or_create_user("ndmv", "pw123").unwrap();
        let tok = st.store.create_session(u.id, "nd").unwrap();
        let pl = st
            .store
            .create_playlist(
                u.id,
                "Order Test",
                "",
                false,
                &["t1".into(), "t2".into(), "t3".into()],
            )
            .unwrap();

        let mut hdrs = HeaderMap::new();
        hdrs.insert(
            "x-nd-authorization",
            format!("Bearer {}", tok).parse().unwrap(),
        );

        // Move item 1 ("t1") before item 3 ("t3") -> ["t2", "t1", "t3"]
        let r1 = move_playlist_track(
            State(st.clone()),
            Path((pl.id.clone(), "1".into())),
            hdrs.clone(),
            Json(MoveTrackReq { insert_before: 3 }),
        )
        .await;
        assert_eq!(r1.status(), StatusCode::OK);
        let ids1: Vec<String> = st
            .store
            .playlist_tracks(&pl.id)
            .unwrap()
            .into_iter()
            .map(|t| t.id)
            .collect();
        assert_eq!(ids1, vec!["t2", "t1", "t3"]);

        // Move item 1 ("t2") to end (insert_before = 4 = len + 1) -> ["t1", "t3", "t2"]
        let r2 = move_playlist_track(
            State(st.clone()),
            Path((pl.id.clone(), "1".into())),
            hdrs.clone(),
            Json(MoveTrackReq { insert_before: 4 }),
        )
        .await;
        assert_eq!(r2.status(), StatusCode::OK);
        let ids2: Vec<String> = st
            .store
            .playlist_tracks(&pl.id)
            .unwrap()
            .into_iter()
            .map(|t| t.id)
            .collect();
        assert_eq!(ids2, vec!["t1", "t3", "t2"]);

        // Invalid position rejects with 400 rather than mutating index 0
        let bad = move_playlist_track(
            State(st.clone()),
            Path((pl.id.clone(), "bad".into())),
            hdrs,
            Json(MoveTrackReq { insert_before: 1 }),
        )
        .await;
        assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn navidrome_login_subsonic_token_works_and_key_rotation_revokes_session() {
        let st = nd_test_state();
        let user = st.store.create_user("rotuser", "accountpass").unwrap();
        let api_key = st.store.get_or_create_api_key(user.id).unwrap();

        let resp = login(
            State(st.clone()),
            HeaderMap::new(),
            Json(LoginReq {
                username: "rotuser".into(),
                password: api_key,
                ..Default::default()
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body: Value =
            serde_json::from_slice(&to_bytes(resp.into_body(), 1024 * 1024).await.unwrap())
                .unwrap();
        let nd_token = body["token"].as_str().unwrap().to_string();
        let sub_salt = body["subsonicSalt"].as_str().unwrap().to_string();
        let sub_tok = body["subsonicToken"].as_str().unwrap().to_string();

        // The subsonicToken/subsonicSalt issued by /auth/login MUST authenticate on /rest/ping.
        let ping = crate::subsonic::dispatch(
            State(st.clone()),
            Path("ping.view".into()),
            RawQuery(Some(format!(
                "u=rotuser&t={}&s={}&f=json&v=1.16.1&c=Feishin",
                sub_tok, sub_salt
            ))),
            HeaderMap::new(),
            String::new(),
        )
        .await;
        let ping_json: Value =
            serde_json::from_slice(&to_bytes(ping.into_body(), 1024 * 1024).await.unwrap())
                .unwrap();
        assert_eq!(ping_json["subsonic-response"]["status"], "ok");

        // Rotating the API key immediately revokes the Navidrome session AND the old subsonic token.
        let _ = st.store.regenerate_api_key(user.id).unwrap();

        let mut hdrs = HeaderMap::new();
        hdrs.insert("x-nd-authorization", format!("Bearer {}", nd_token).parse().unwrap());
        let after_nd = keepalive(State(st.clone()), hdrs).await;
        assert_eq!(after_nd.status(), StatusCode::UNAUTHORIZED);

        let after_sub = crate::subsonic::dispatch(
            State(st),
            Path("ping.view".into()),
            RawQuery(Some(format!(
                "u=rotuser&t={}&s={}&f=json&v=1.16.1&c=Feishin",
                sub_tok, sub_salt
            ))),
            HeaderMap::new(),
            String::new(),
        )
        .await;
        let after_sub_json: Value =
            serde_json::from_slice(&to_bytes(after_sub.into_body(), 1024 * 1024).await.unwrap())
                .unwrap();
        assert_eq!(after_sub_json["subsonic-response"]["error"]["code"], 40);
    }

    #[tokio::test]
    async fn navidrome_radio_camel_case_queue_numeric_current_and_sorting() {
        let st = nd_test_state();
        st.store
            .upsert_tracks(&[Track {
                id: "t2".into(),
                source: "local".into(),
                title: "Die For You".into(),
                artist: "The Weeknd".into(),
                album: "Starboy".into(),
                genre: "Pop".into(),
                year: 2016,
                track_no: 2,
                duration: 260,
                ..Default::default()
            }])
            .unwrap();
        let user = st.store.create_user("nduser", "pw123").unwrap();
        let tok = st.store.create_session(user.id, "nd").unwrap();
        let mut hdrs = HeaderMap::new();
        hdrs.insert("x-nd-authorization", format!("Bearer {}", tok).parse().unwrap());

        // Radio creation returns camelCase fields matching list_radios.
        let r = create_radio(
            State(st.clone()),
            hdrs.clone(),
            Json(RadioReq {
                name: "Lo-Fi".into(),
                stream_url: "https://stream.example/lofi".into(),
                home_page_url: "https://example.com".into(),
            }),
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK);
        let rjson: Value =
            serde_json::from_slice(&to_bytes(r.into_body(), 1024 * 1024).await.unwrap()).unwrap();
        assert_eq!(rjson["streamUrl"], "https://stream.example/lofi");
        assert_eq!(rjson["homePageUrl"], "https://example.com");
        assert!(rjson.get("stream_url").is_none());

        // Queue accepts numeric `current` index and returns ratings on get_queue.
        let _ = st.store.set_rating(user.id, "t2", 4);
        let qsave = save_queue(
            State(st.clone()),
            hdrs.clone(),
            Json(SaveQueueReq {
                ids: vec!["t1".into(), "t2".into()],
                current: Some(json!(1)),
                current_id: None,
                position: 42_000,
            }),
        )
        .await;
        assert_eq!(qsave.status(), StatusCode::OK);
        let qget = get_queue(State(st.clone()), hdrs.clone()).await;
        let qjson: Value =
            serde_json::from_slice(&to_bytes(qget.into_body(), 1024 * 1024).await.unwrap())
                .unwrap();
        assert_eq!(qjson["current"], 1);
        assert_eq!(qjson["currentId"], "t2");
        assert_eq!(qjson["items"][1]["rating"], 4);

        // list_songs supports has_rating=true and _sort=rating&_order=DESC.
        let songs = list_songs(
            State(st.clone()),
            RawQuery(Some("has_rating=true&_sort=rating&_order=DESC".into())),
            hdrs.clone(),
        )
        .await;
        let sjson: Value =
            serde_json::from_slice(&to_bytes(songs.into_body(), 1024 * 1024).await.unwrap())
                .unwrap();
        let arr = sjson.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["id"], "t2");

        // list_albums supports recently_played=true.
        let empty_recent = list_albums(
            State(st.clone()),
            RawQuery(Some("recently_played=true".into())),
            hdrs.clone(),
        )
        .await;
        let erjson: Value =
            serde_json::from_slice(&to_bytes(empty_recent.into_body(), 1024 * 1024).await.unwrap())
                .unwrap();
        assert_eq!(erjson.as_array().unwrap().len(), 0);
    }
}
