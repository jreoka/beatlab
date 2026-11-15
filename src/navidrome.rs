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

const TOKEN_TTL_SECS: i64 = 30 * 86400;

type TokenMap = Arc<Mutex<HashMap<String, (i64, i64)>>>;

static TOKENS: std::sync::OnceLock<TokenMap> = std::sync::OnceLock::new();

fn token_map() -> &'static TokenMap {
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
    let now = chrono::Utc::now().timestamp();
    {
        let mut map = token_map().lock().unwrap();
        if map.len() >= 1000 {
            map.retain(|_, (_, ts)| now - *ts < TOKEN_TTL_SECS);
            if map.len() >= 1000 {
                let mut by_age: Vec<(String, i64)> =
                    map.iter().map(|(k, (_, ts))| (k.clone(), *ts)).collect();
                by_age.sort_by_key(|(_, ts)| *ts);
                let drop_n = map.len().saturating_sub(500);
                for (k, _) in by_age.into_iter().take(drop_n) {
                    map.remove(&k);
                }
            }
        }
        map.insert(tok.clone(), (user.id, now));
    }

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
    let now = chrono::Utc::now().timestamp();
    let uid = {
        let mut map = token_map().lock().unwrap();
        match map.get(tok).copied() {
            Some((uid, ts)) if now - ts < TOKEN_TTL_SECS => Some(uid),
            Some(_) => {
                map.remove(tok);
                None
            }
            None => None,
        }
    }?;
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
    let start = q.get("_start").and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
    let end = q.get("_end").and_then(|v| v.parse::<usize>().ok()).unwrap_or(start + 50);
    let limit = end.saturating_sub(start).clamp(1, 200);
    let fetch_limit = start + limit;
    let search = q
        .get("title")
        .or_else(|| q.get("q"))
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());
    let album_id = q.get("album_id").map(|s| s.trim()).filter(|s| !s.is_empty());
    let artist_id = q
        .get("artist_id")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());
    let starred_only = q.get("starred").map(|v| v == "true").unwrap_or(false);
    let genre_filter = q
        .get("genre_id")
        .or_else(|| q.get("genre"))
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());

    let mut tracks = if let Some(search) = search {
        let mut pool = state.store.search_tracks(search, fetch_limit.max(200));
        if pool.len() < fetch_limit {
            let remote = state.registry.search_all(search, fetch_limit).await;
            pool.extend(remote);
            pool = crate::sources::dedup_and_rank_search(pool, search, fetch_limit.max(200));
        }
        pool
    } else if let Some(aid) = artist_id {
        state
            .store
            .artist_name_from_id(aid)
            .map(|name| state.store.tracks_by_artist(&name, 5000))
            .unwrap_or_default()
    } else {
        state.store.all_tracks(5000)
    };

    if let Some(alb) = album_id {
        tracks.retain(|t| t.album_id() == alb);
    }
    if let Some(aid) = artist_id {
        tracks.retain(|t| t.artist_id() == aid);
    }
    if starred_only {
        tracks.retain(|t| state.store.is_starred(user.id, &t.id));
    }
    if let Some(gf) = genre_filter {
        let gf_lc = gf.to_lowercase();
        let gf_slug = crate::models::slug(gf);
        tracks.retain(|t| {
            !t.genre.is_empty()
                && (t.genre.to_lowercase() == gf_lc
                    || crate::models::slug(&t.genre) == gf_slug
                    || t.genre.to_lowercase().contains(&gf_lc))
        });
    }

    if let Some(sort) = q.get("_sort").map(|s| s.trim().to_ascii_lowercase()) {
        let desc = q
            .get("_order")
            .map(|s| s.eq_ignore_ascii_case("DESC"))
            .unwrap_or(false);
        if sort == "random" {
            use rand::seq::SliceRandom;
            tracks.shuffle(&mut rand::thread_rng());
        } else {
            tracks.sort_by(|a, b| {
                let ord = match sort.as_str() {
                    "title" => a.title.to_lowercase().cmp(&b.title.to_lowercase()),
                    "album" => a
                        .album
                        .to_lowercase()
                        .cmp(&b.album.to_lowercase())
                        .then_with(|| a.track_no.cmp(&b.track_no)),
                    "artist" => a.artist.to_lowercase().cmp(&b.artist.to_lowercase()),
                    "year" => a.year.cmp(&b.year),
                    "track" | "tracknumber" => a.track_no.cmp(&b.track_no),
                    "duration" => a.duration.cmp(&b.duration),
                    "size" => a.size.cmp(&b.size),
                    "bitrate" => a.bitrate.cmp(&b.bitrate),
                    "play_count" | "playcount" => a.play_count.cmp(&b.play_count),
                    "created" | "createdat" => a.created.cmp(&b.created),
                    "starred" => state
                        .store
                        .is_starred(user.id, &a.id)
                        .cmp(&state.store.is_starred(user.id, &b.id)),
                    _ => a.id.cmp(&b.id),
                };
                if desc { ord.reverse() } else { ord }
            });
        }
    } else if album_id.is_some() {
        tracks.sort_by(|a, b| a.track_no.cmp(&b.track_no).then_with(|| a.title.cmp(&b.title)));
    }

    let total = tracks.len();
    state.engine.warm_resolve(
        tracks.iter().skip(start).take(5).cloned().collect(),
        5,
    );
    let items: Vec<_> = tracks
        .iter()
        .skip(start)
        .take(limit)
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
    state.taste.ensure_auto_playlists_fresh(user.id);
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
    state.engine.warm_resolve(
        tracks.iter().take(5).cloned().collect(),
        5,
    );
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

pub async fn nd_keepalive(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if auth_nd(&state, &headers).is_none() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(json!({"response": "ok"})).into_response()
}

pub async fn nd_playlist_get(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let Some(user) = auth_nd(&state, &headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let Some(p) = state.store.get_playlist(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    Json(json!({
        "id": p.id,
        "name": p.name,
        "comment": p.comment,
        "public": p.public,
        "songCount": p.track_count,
        "duration": p.duration,
        "ownerName": user.username,
    }))
    .into_response()
}

pub async fn nd_album_list(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let Some(user) = auth_nd(&state, &headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let start = q.get("_start").and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
    let end = q.get("_end").and_then(|v| v.parse::<usize>().ok()).unwrap_or(start + 20);
    let limit = end.saturating_sub(start).clamp(1, 200);
    let sort = q.get("_sort").map(|s| s.as_str()).unwrap_or("recently_added");
    let order = q.get("_order").map(|s| s.as_str()).unwrap_or("DESC");

    let tracks = state.store.all_tracks(5000);
    let mut groups: HashMap<String, Vec<Track>> = HashMap::new();
    for t in tracks {
        groups.entry(t.album_id()).or_default().push(t);
    }

    let mut albums: Vec<Vec<Track>> = groups.into_values().collect();
    if q.get("starred").map(|s| s == "true").unwrap_or(false) {
        albums.retain(|trs| trs.iter().any(|t| state.store.is_starred(user.id, &t.id)));
    }
    if let Some(search) = q.get("name").or_else(|| q.get("q")) {
        let low = search.trim().to_lowercase();
        if !low.is_empty() {
            albums.retain(|trs| {
                trs.first()
                    .map(|t| t.album.to_lowercase().contains(&low) || t.artist.to_lowercase().contains(&low))
                    .unwrap_or(false)
            });
            if albums.is_empty() {
                let remote = state.registry.search_all(search.trim(), 30).await;
                let mut remote_groups: HashMap<String, Vec<Track>> = HashMap::new();
                for t in remote {
                    if t.album.to_lowercase().contains(&low) || t.artist.to_lowercase().contains(&low) {
                        remote_groups.entry(t.album_id()).or_default().push(t);
                    }
                }
                albums = remote_groups.into_values().collect();
            }
        }
    }

    match sort {
        "random" => {
            use rand::seq::SliceRandom;
            let mut rng = rand::thread_rng();
            albums.shuffle(&mut rng);
        }
        "play_count" => {
            albums.sort_by(|a, b| {
                let pa: i64 = a.iter().map(|t| t.play_count as i64).sum();
                let pb: i64 = b.iter().map(|t| t.play_count as i64).sum();
                pb.cmp(&pa)
            });
        }
        "play_date" => {
            albums.sort_by(|a, b| {
                let la = a.iter().map(|t| t.last_played).max().unwrap_or(0);
                let lb = b.iter().map(|t| t.last_played).max().unwrap_or(0);
                lb.cmp(&la)
            });
        }
        "name" => {
            albums.sort_by(|a, b| {
                let na = a.first().map(|t| t.album.to_lowercase()).unwrap_or_default();
                let nb = b.first().map(|t| t.album.to_lowercase()).unwrap_or_default();
                na.cmp(&nb)
            });
        }
        _ => {
            albums.sort_by(|a, b| {
                let ca = a.iter().map(|t| t.created).max().unwrap_or(0);
                let cb = b.iter().map(|t| t.created).max().unwrap_or(0);
                cb.cmp(&ca)
            });
        }
    }
    if order.eq_ignore_ascii_case("ASC") && sort != "name" && sort != "random" {
        albums.reverse();
    }

    let total = albums.len();
    let items: Vec<_> = albums
        .into_iter()
        .skip(start)
        .take(limit)
        .map(|trs| nd_album_json(&state.store, user.id, &trs))
        .collect();

    (
        StatusCode::OK,
        [("x-total-count", total.to_string())],
        Json(items),
    )
        .into_response()
}

pub async fn nd_album_get(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let Some(user) = auth_nd(&state, &headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let trs: Vec<Track> = state
        .store
        .all_tracks(5000)
        .into_iter()
        .filter(|t| t.album_id() == id)
        .collect();
    if trs.is_empty() {
        return StatusCode::NOT_FOUND.into_response();
    }
    Json(nd_album_json(&state.store, user.id, &trs)).into_response()
}

pub async fn nd_artist_list(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    if auth_nd(&state, &headers).is_none() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let start = q.get("_start").and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
    let end = q.get("_end").and_then(|v| v.parse::<usize>().ok()).unwrap_or(start + 50);
    let limit = end.saturating_sub(start).clamp(1, 200);
    let mut artists = state.store.artists();
    if let Some(search) = q.get("name").or_else(|| q.get("q")) {
        let low = search.trim().to_lowercase();
        if !low.is_empty() {
            artists.retain(|(_, n, _)| n.to_lowercase().contains(&low));
            if artists.is_empty() {
                let _ = state.registry.search_all(search.trim(), 30).await;
                artists = state.store.artists();
                artists.retain(|(_, n, _)| n.to_lowercase().contains(&low));
            }
        }
    }
    let total = artists.len();
    let items: Vec<_> = artists
        .into_iter()
        .skip(start)
        .take(limit)
        .map(|(id, name, album_count)| {
            json!({
                "id": id,
                "name": name,
                "albumCount": album_count,
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

pub async fn nd_artist_get(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if auth_nd(&state, &headers).is_none() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Some(name) = state.store.artist_name_from_id(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let tracks = state.store.tracks_by_artist(&name, 500);
    let album_count = tracks
        .iter()
        .map(|t| t.album_id())
        .collect::<std::collections::HashSet<_>>()
        .len();
    Json(json!({
        "id": id,
        "name": name,
        "albumCount": album_count,
        "songCount": tracks.len(),
    }))
    .into_response()
}

pub async fn nd_genre_list(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if auth_nd(&state, &headers).is_none() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let genres = state.store.genres();
    let total = genres.len();
    let items: Vec<_> = genres
        .into_iter()
        .map(|(name, count)| json!({ "id": crate::models::slug(&name), "name": name, "songCount": count }))
        .collect();
    (
        StatusCode::OK,
        [("x-total-count", total.to_string())],
        Json(items),
    )
        .into_response()
}

fn nd_album_json(store: &Store, user_id: i64, trs: &[Track]) -> serde_json::Value {
    let first = &trs[0];
    let duration: i32 = trs.iter().map(|t| t.duration).sum();
    let play_count: i64 = trs.iter().map(|t| t.play_count as i64).sum();
    let starred = trs.iter().any(|t| store.is_starred(user_id, &t.id));
    json!({
        "id": first.album_id(),
        "name": first.album,
        "albumArtist": first.artist,
        "albumArtistId": first.artist_id(),
        "artist": first.artist,
        "artistId": first.artist_id(),
        "coverArtId": first.id,
        "songCount": trs.len(),
        "duration": duration,
        "playCount": play_count,
        "minYear": first.year,
        "maxYear": first.year,
        "genre": first.genre,
        "starred": starred,
    })
}

fn md5_hex(s: &str) -> String {
    use md5::{Digest, Md5};
    let mut h = Md5::new();
    h.update(s.as_bytes());
    hex::encode(h.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Engine;
    use crate::models::Track;
    use crate::sources::local::LocalSource;
    use crate::sources::soulseek::SoulseekSource;
    use crate::sources::SourceRegistry;
    use crate::taste::TasteEngine;
    use axum::body::to_bytes;
    use std::collections::HashSet;

    fn test_state() -> AppState {
        let dir = std::env::temp_dir().join(format!("beatlab_nd_test_{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&dir);
        let store = Store::open_in_memory().unwrap();
        let u = store.create_user("alice", "pw", false).unwrap();
        let t1 = Track {
            id: "s1".into(),
            source: "local".into(),
            title: "Alpha".into(),
            artist: "Daft Punk".into(),
            album: "Discovery".into(),
            genre: "French House".into(),
            track_no: 2,
            year: 2001,
            duration: 200,
            ..Default::default()
        };
        let t2 = Track {
            id: "s2".into(),
            source: "local".into(),
            title: "Beta".into(),
            artist: "Daft Punk".into(),
            album: "Discovery".into(),
            genre: "French House".into(),
            track_no: 1,
            year: 2001,
            duration: 240,
            ..Default::default()
        };
        let t3 = Track {
            id: "s3".into(),
            source: "local".into(),
            title: "Gamma".into(),
            artist: "M83".into(),
            album: "Saturdays".into(),
            genre: "Synthpop".into(),
            track_no: 1,
            year: 2008,
            duration: 180,
            ..Default::default()
        };
        store.upsert_tracks(&[t1, t2, t3]);
        store.star_track(u.id, "s3");
        let local = Arc::new(LocalSource::new(store.clone(), vec![]));
        let soulseek = SoulseekSource::new(store.clone(), dir.join("slsk"));
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
            admin_password: "pass".into(),
            admin_sessions: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    #[tokio::test]
    async fn nd_song_list_filters_sorts_and_paginates() {
        let st = test_state();
        let login_resp = nd_login(
            State(st.clone()),
            Json(LoginReq {
                username: "alice".into(),
                password: "pw".into(),
            }),
        )
        .await;
        let login_bytes = to_bytes(login_resp.into_body(), 1024 * 1024).await.unwrap();
        let login_json: serde_json::Value = serde_json::from_slice(&login_bytes).unwrap();
        let tok = login_json["token"].as_str().unwrap().to_string();

        let mut hdrs = HeaderMap::new();
        hdrs.insert("x-nd-authorization", format!("Bearer {}", tok).parse().unwrap());

        // Filter by album_id (sorted by trackNumber by default)
        let discovery_id = st.store.track("s1").unwrap().album_id();
        let mut q = HashMap::new();
        q.insert("album_id".into(), discovery_id);
        let resp = nd_song_list(State(st.clone()), hdrs.clone(), Query(q)).await;
        assert_eq!(resp.headers().get("x-total-count").unwrap(), "2");
        let arr: Vec<serde_json::Value> =
            serde_json::from_slice(&to_bytes(resp.into_body(), 1024 * 1024).await.unwrap())
                .unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["id"], "s2");
        assert_eq!(arr[1]["id"], "s1");

        // Filter by starred=true and genre_id slug
        let mut q2 = HashMap::new();
        q2.insert("starred".into(), "true".into());
        q2.insert("genre_id".into(), crate::models::slug("Synthpop"));
        let resp2 = nd_song_list(State(st.clone()), hdrs.clone(), Query(q2)).await;
        assert_eq!(resp2.headers().get("x-total-count").unwrap(), "1");
        let arr2: Vec<serde_json::Value> =
            serde_json::from_slice(&to_bytes(resp2.into_body(), 1024 * 1024).await.unwrap())
                .unwrap();
        assert_eq!(arr2[0]["id"], "s3");

        // Sort + pagination keeps total count accurate
        let mut q3 = HashMap::new();
        q3.insert("_sort".into(), "year".into());
        q3.insert("_order".into(), "DESC".into());
        q3.insert("_start".into(), "0".into());
        q3.insert("_end".into(), "1".into());
        let resp3 = nd_song_list(State(st), hdrs, Query(q3)).await;
        assert_eq!(resp3.headers().get("x-total-count").unwrap(), "3");
        let arr3: Vec<serde_json::Value> =
            serde_json::from_slice(&to_bytes(resp3.into_body(), 1024 * 1024).await.unwrap())
                .unwrap();
        assert_eq!(arr3.len(), 1);
        assert_eq!(arr3[0]["id"], "s3");
    }
}
