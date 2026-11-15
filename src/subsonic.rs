use crate::engine::{Engine, content_type_for_format};
use crate::models::{Track, User};
use crate::sources::SourceRegistry;
use crate::sources::local::LocalSource;
use crate::store::Store;
use crate::taste::TasteEngine;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

const API_VERSION: &str = "1.16.1";
const SERVER_TYPE: &str = "beatlab";
const SERVER_VERSION: &str = "0.2.0";

#[derive(Clone)]
pub struct AppState {
    pub store: Store,
    pub registry: Arc<SourceRegistry>,
    pub engine: Arc<Engine>,
    pub taste: Arc<TasteEngine>,
    pub local: Arc<LocalSource>,
    pub admin_password: String,
    pub now_playing: Arc<Mutex<HashMap<i64, (Track, i64, String)>>>,
}

#[derive(Clone)]
pub struct XmlElem {
    pub name: String,
    pub attrs: Vec<(String, Value)>,
    pub children: Vec<XmlElem>,
    pub text: Option<String>,
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

    pub fn attr(mut self, k: &str, v: impl Into<Value>) -> Self {
        let val = v.into();
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

    pub fn children(mut self, cs: impl IntoIterator<Item = XmlElem>) -> Self {
        self.children.extend(cs);
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
            let items = groups.remove(&name).unwrap();
            if items.len() == 1 {
                obj.insert(name, items.into_iter().next().unwrap());
            } else {
                obj.insert(name, Value::Array(items));
            }
        }
        Value::Object(obj)
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn respond(params: &HashMap<String, String>, ok: bool, inner: Option<XmlElem>) -> Response {
    let mut root = XmlElem::new("subsonic-response")
        .attr("status", if ok { "ok" } else { "failed" })
        .attr("version", API_VERSION)
        .attr("type", SERVER_TYPE)
        .attr("serverVersion", SERVER_VERSION)
        .attr("openSubsonic", true);

    if let Some(c) = inner {
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
            let cb = params.get("callback").map(|s| s.as_str()).unwrap_or("callback");
            let payload = serde_json::json!({ "subsonic-response": root.to_json() });
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/javascript; charset=utf-8")],
                format!("{}({});", cb, payload),
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

fn subsonic_err(params: &HashMap<String, String>, code: i32, msg: &str) -> Response {
    respond(
        params,
        false,
        Some(XmlElem::new("error").attr("code", code).attr("message", msg)),
    )
}

fn track_elem(store: &Store, user_id: i64, t: &Track) -> XmlElem {
    let starred = store.is_starred(user_id, &t.id);
    let rating = store.user_rating(user_id, &t.id);
    let mut e = XmlElem::new("song")
        .attr("id", t.id.clone())
        .attr("parent", t.album_id())
        .attr("isDir", false)
        .attr("title", t.title.clone())
        .attr("album", t.album.clone())
        .attr("artist", t.artist.clone())
        .attr("track", t.track_no)
        .attr("year", t.year)
        .attr("genre", t.genre.clone())
        .attr("coverArt", t.id.clone())
        .attr("size", t.size)
        .attr("contentType", content_type_for_format(&t.format))
        .attr("suffix", t.format.clone())
        .attr("duration", t.duration)
        .attr("bitRate", t.bitrate)
        .attr("samplingRate", t.sample_rate)
        .attr("bitDepth", t.bit_depth)
        .attr("albumId", t.album_id())
        .attr("artistId", t.artist_id())
        .attr("type", "music")
        .attr("playCount", t.play_count);

    if starred {
        e = e.attr("starred", iso_now());
    }
    if rating > 0 {
        e = e.attr("userRating", rating);
    }
    e
}

fn iso_now() -> String {
    chrono::Utc::now().to_rfc3339()
}

pub async fn dispatch(
    State(state): State<AppState>,
    Path(action_raw): Path<String>,
    Query(q_params): Query<HashMap<String, String>>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let mut params = q_params;
    for (k, v) in url_form_decode(&body) {
        params.entry(k).or_insert(v);
    }

    let action = action_raw.trim_end_matches(".view").trim_start_matches('/');
    let u = params.get("u").map(|s| s.as_str()).unwrap_or("");
    let p = params.get("p").map(|s| s.as_str()).unwrap_or("");
    let t = params.get("t").map(|s| s.as_str()).unwrap_or("");
    let s = params.get("s").map(|s| s.as_str()).unwrap_or("");

    let Some(user) = state.store.auth_subsonic(u, p, t, s) else {
        return subsonic_err(&params, 40, "Wrong username or password.");
    };

    match action {
        "ping" => respond(&params, true, None),
        "getLicense" => respond(
            &params,
            true,
            Some(XmlElem::new("license").attr("valid", true).attr("email", "admin@beatlab.local")),
        ),
        "getOpenSubsonicExtensions" => respond(
            &params,
            true,
            Some(
                XmlElem::new("openSubsonicExtensions")
                    .child(XmlElem::new("openSubsonicExtensions").attr("name", "transcodeOffset").attr("versions", "1"))
                    .child(XmlElem::new("openSubsonicExtensions").attr("name", "formPost").attr("versions", "1")),
            ),
        ),
        "getMusicFolders" => respond(
            &params,
            true,
            Some(
                XmlElem::new("musicFolders")
                    .child(XmlElem::new("musicFolder").attr("id", "1").attr("name", "Beatlab")),
            ),
        ),
        "getUser" => respond(&params, true, Some(user_elem(&user))),
        "getUsers" => {
            let users = state.store.list_users();
            respond(
                &params,
                true,
                Some(XmlElem::new("users").children(users.iter().map(user_elem))),
            )
        }
        "stream" | "download" => {
            let id = params.get("id").map(|s| s.as_str()).unwrap_or("");
            let Some(track) = state.store.track(id) else {
                return (StatusCode::NOT_FOUND, "track not found").into_response();
            };
            let target_fmt = params.get("format").map(|s| s.as_str());
            let max_br = params.get("maxBitRate").and_then(|v| v.parse::<i32>().ok());
            state
                .engine
                .serve_track(&track, &headers, target_fmt, max_br)
                .await
        }
        "getCoverArt" => {
            let id = params.get("id").map(|s| s.as_str()).unwrap_or("");
            state.engine.serve_cover(id).await
        }
        "search3" | "search2" => {
            let q = params.get("query").map(|s| s.as_str()).unwrap_or("");
            let song_count = params
                .get("songCount")
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(20);
            let mut songs = state.store.search_tracks(q, song_count);
            if !q.trim().is_empty() && songs.len() < song_count {
                let remote = state.registry.search_all(q, song_count).await;
                songs.extend(remote);
                songs = crate::sources::dedup_best_quality(songs, song_count);
            }
            let tag = if action == "search3" { "searchResult3" } else { "searchResult2" };
            let mut el = XmlElem::new(tag);
            for tr in &songs {
                el = el.child(track_elem(&state.store, user.id, tr));
            }
            respond(&params, true, Some(el))
        }
        "getSong" => {
            let id = params.get("id").map(|s| s.as_str()).unwrap_or("");
            let Some(tr) = state.store.track(id) else {
                return subsonic_err(&params, 70, "Song not found");
            };
            respond(&params, true, Some(track_elem(&state.store, user.id, &tr)))
        }
        "getArtists" => {
            let artists = state.store.artists();
            let idx = XmlElem::new("index").attr("name", "#").children(
                artists.into_iter().map(|(id, name, count)| {
                    XmlElem::new("artist")
                        .attr("id", id)
                        .attr("name", name)
                        .attr("albumCount", count)
                }),
            );
            respond(&params, true, Some(XmlElem::new("artists").child(idx)))
        }
        "getArtist" => {
            let id = params.get("id").map(|s| s.as_str()).unwrap_or("");
            let Some(name) = state.store.artist_name_from_id(id) else {
                return subsonic_err(&params, 70, "Artist not found");
            };
            let tracks = state.store.tracks_by_artist(&name, 200);
            let mut albums_map: HashMap<String, Vec<Track>> = HashMap::new();
            for tr in tracks {
                albums_map.entry(tr.album.clone()).or_default().push(tr);
            }
            let albums: Vec<_> = albums_map
                .into_iter()
                .map(|(alb, trs)| {
                    let first = trs.first().unwrap();
                    XmlElem::new("album")
                        .attr("id", first.album_id())
                        .attr("name", alb)
                        .attr("artist", name.clone())
                        .attr("artistId", first.artist_id())
                        .attr("coverArt", first.id.clone())
                        .attr("songCount", trs.len())
                        .attr("duration", trs.iter().map(|x| x.duration).sum::<i32>())
                })
                .collect();
            respond(
                &params,
                true,
                Some(
                    XmlElem::new("artist")
                        .attr("id", id)
                        .attr("name", name)
                        .children(albums),
                ),
            )
        }
        "getAlbum" => {
            let id = params.get("id").map(|s| s.as_str()).unwrap_or("");
            let all = state.store.all_tracks(3000);
            let trs: Vec<Track> = all.into_iter().filter(|t| t.album_id() == id).collect();
            let Some(first) = trs.first() else {
                return subsonic_err(&params, 70, "Album not found");
            };
            let mut el = XmlElem::new("album")
                .attr("id", id)
                .attr("name", first.album.clone())
                .attr("artist", first.artist.clone())
                .attr("artistId", first.artist_id())
                .attr("coverArt", first.id.clone())
                .attr("songCount", trs.len())
                .attr("duration", trs.iter().map(|x| x.duration).sum::<i32>());
            for tr in &trs {
                el = el.child(track_elem(&state.store, user.id, tr));
            }
            respond(&params, true, Some(el))
        }
        "getRandomSongs" => {
            let size = params
                .get("size")
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(20);
            let recs = state.taste.recommend_for_user(user.id, size);
            let mut el = XmlElem::new("randomSongs");
            for tr in &recs {
                el = el.child(track_elem(&state.store, user.id, tr));
            }
            respond(&params, true, Some(el))
        }
        "getGenres" => {
            let genres = state.store.genres();
            let el = XmlElem::new("genres").children(genres.into_iter().map(|(g, c)| {
                XmlElem::new("genre")
                    .attr("songCount", c)
                    .attr("albumCount", 1)
                    .text(g)
            }));
            respond(&params, true, Some(el))
        }
        "getSongsByGenre" => {
            let genre = params.get("genre").map(|s| s.as_str()).unwrap_or("");
            let count = params.get("count").and_then(|v| v.parse().ok()).unwrap_or(20);
            let offset = params.get("offset").and_then(|v| v.parse().ok()).unwrap_or(0);
            let trs = state.store.tracks_by_genre(genre, count, offset);
            let mut el = XmlElem::new("songsByGenre");
            for tr in &trs {
                el = el.child(track_elem(&state.store, user.id, tr));
            }
            respond(&params, true, Some(el))
        }
        "getStarred" | "getStarred2" => {
            let starred = state.store.starred_tracks(user.id);
            let tag = if action == "getStarred2" { "starred2" } else { "starred" };
            let mut el = XmlElem::new(tag);
            for tr in &starred {
                el = el.child(track_elem(&state.store, user.id, tr));
            }
            respond(&params, true, Some(el))
        }
        "star" => {
            if let Some(id) = params.get("id") {
                state.store.star_track(user.id, id);
                if let Some(tr) = state.store.track(id) {
                    state.taste.learn_event(user.id, &tr, "star", 1.0);
                }
            }
            respond(&params, true, None)
        }
        "unstar" => {
            if let Some(id) = params.get("id") {
                state.store.unstar_track(user.id, id);
                if let Some(tr) = state.store.track(id) {
                    state.taste.learn_event(user.id, &tr, "unstar", 1.0);
                }
            }
            respond(&params, true, None)
        }
        "setRating" => {
            let id = params.get("id").map(|s| s.as_str()).unwrap_or("");
            let rating = params.get("rating").and_then(|v| v.parse::<i32>().ok()).unwrap_or(0);
            state.store.set_rating(user.id, id, rating);
            if let Some(tr) = state.store.track(id) {
                state.taste.learn_event(user.id, &tr, "rate", rating as f64);
            }
            respond(&params, true, None)
        }
        "scrobble" => {
            let id = params.get("id").map(|s| s.as_str()).unwrap_or("");
            let submission = params.get("submission").map(|s| s != "false").unwrap_or(true);
            if let Some(tr) = state.store.track(id) {
                if submission {
                    state.store.increment_play(id);
                    state.store.record_event(user.id, id, "complete", 1.0);
                    state.taste.learn_event(user.id, &tr, "complete", 1.0);
                } else {
                    state.store.record_event(user.id, id, "play", 1.0);
                    state.taste.learn_event(user.id, &tr, "play", 1.0);
                    let mut np = state.now_playing.lock().unwrap();
                    np.insert(user.id, (tr, chrono::Utc::now().timestamp(), "Subsonic".into()));
                }
            }
            respond(&params, true, None)
        }
        "getPlaylists" => {
            let _ = state.taste.generate_auto_playlists(user.id);
            let pls = state.store.list_playlists(user.id);
            let el = XmlElem::new("playlists").children(pls.into_iter().map(|p| {
                XmlElem::new("playlist")
                    .attr("id", p.id)
                    .attr("name", p.name)
                    .attr("comment", p.comment)
                    .attr("owner", user.username.clone())
                    .attr("public", p.public)
                    .attr("songCount", p.track_count)
                    .attr("duration", p.duration)
                    .attr("coverArt", p.cover_track_id.unwrap_or_default())
            }));
            respond(&params, true, Some(el))
        }
        "getPlaylist" => {
            let id = params.get("id").map(|s| s.as_str()).unwrap_or("");
            let Some(p) = state.store.get_playlist(id) else {
                return subsonic_err(&params, 70, "Playlist not found");
            };
            let trs = state.store.playlist_tracks(id);
            let mut el = XmlElem::new("playlist")
                .attr("id", p.id)
                .attr("name", p.name)
                .attr("comment", p.comment)
                .attr("owner", user.username.clone())
                .attr("public", p.public)
                .attr("songCount", trs.len())
                .attr("duration", p.duration);
            for tr in &trs {
                el = el.child(track_elem(&state.store, user.id, tr));
            }
            respond(&params, true, Some(el))
        }
        "createPlaylist" => {
            let name = params.get("name").map(|s| s.as_str()).unwrap_or("New Playlist");
            let song_ids: Vec<String> = params
                .get("songId")
                .map(|s| vec![s.clone()])
                .unwrap_or_default();
            let _ = state
                .store
                .create_playlist(user.id, name, "", true, false, "user", &song_ids);
            respond(&params, true, None)
        }
        "updatePlaylist" => {
            let id = params.get("playlistId").map(|s| s.as_str()).unwrap_or("");
            let name = params.get("name").map(|s| s.as_str());
            let comment = params.get("comment").map(|s| s.as_str());
            let public = params.get("public").map(|s| s == "true");
            state.store.update_playlist_meta(id, name, comment, public);
            respond(&params, true, None)
        }
        "deletePlaylist" => {
            if let Some(id) = params.get("id") {
                state.store.delete_playlist(id);
            }
            respond(&params, true, None)
        }
        "getSimilarSongs" | "getSimilarSongs2" => {
            let id = params.get("id").map(|s| s.as_str()).unwrap_or("");
            let count = params.get("count").and_then(|v| v.parse().ok()).unwrap_or(25);
            let trs = if let Some(seed) = state.store.track(id) {
                state.taste.similar_songs(&seed, user.id, count)
            } else {
                state.taste.recommend_for_user(user.id, count)
            };
            state.engine.warm_resolve(trs.clone(), 5);
            let tag = if action == "getSimilarSongs2" { "similarSongs2" } else { "similarSongs" };
            let mut el = XmlElem::new(tag);
            for tr in &trs {
                el = el.child(track_elem(&state.store, user.id, tr));
            }
            respond(&params, true, Some(el))
        }
        "getTopSongs" => {
            let artist = params.get("artist").map(|s| s.as_str()).unwrap_or("");
            let count = params.get("count").and_then(|v| v.parse().ok()).unwrap_or(20);
            let trs = state.store.tracks_by_artist(artist, count);
            let mut el = XmlElem::new("topSongs");
            for tr in &trs {
                el = el.child(track_elem(&state.store, user.id, tr));
            }
            respond(&params, true, Some(el))
        }
        "getNowPlaying" => {
            let np = state.now_playing.lock().unwrap();
            let mut el = XmlElem::new("nowPlaying");
            for (_, (tr, at, player)) in np.iter() {
                let mins_ago = ((chrono::Utc::now().timestamp() - at) / 60).max(0);
                el = el.child(
                    track_elem(&state.store, user.id, tr)
                        .attr("username", user.username.clone())
                        .attr("minutesAgo", mins_ago)
                        .attr("playerName", player.clone()),
                );
            }
            respond(&params, true, Some(el))
        }
        "getScanStatus" => respond(
            &params,
            true,
            Some(
                XmlElem::new("scanStatus")
                    .attr("scanning", false)
                    .attr("count", state.store.track_count()),
            ),
        ),
        "startScan" => {
            let local = Arc::clone(&state.local);
            tokio::task::spawn_blocking(move || local.scan());
            respond(
                &params,
                true,
                Some(
                    XmlElem::new("scanStatus")
                        .attr("scanning", true)
                        .attr("count", state.store.track_count()),
                ),
            )
        }
        "savePlayQueue" => {
            let cur = params.get("current").map(|s| s.as_str()).unwrap_or("");
            let pos = params.get("position").and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
            let ids: Vec<String> = params
                .get("id")
                .map(|s| vec![s.clone()])
                .unwrap_or_default();
            state
                .store
                .save_play_queue(user.id, cur, pos, &ids, params.get("c").map(|s| s.as_str()).unwrap_or("client"));
            respond(&params, true, None)
        }
        "getPlayQueue" => {
            if let Some((cur, pos, ids, by, _)) = state.store.get_play_queue(user.id) {
                let mut el = XmlElem::new("playQueue")
                    .attr("current", cur)
                    .attr("position", pos)
                    .attr("username", user.username.clone())
                    .attr("changedBy", by);
                for id in ids {
                    if let Some(tr) = state.store.track(&id) {
                        el = el.child(track_elem(&state.store, user.id, &tr));
                    }
                }
                respond(&params, true, Some(el))
            } else {
                respond(&params, true, Some(XmlElem::new("playQueue")))
            }
        }
        "getInternetRadioStations" => {
            let stations = state.store.list_radio_stations();
            let el = XmlElem::new("internetRadioStations").children(stations.into_iter().map(|r| {
                XmlElem::new("internetRadioStation")
                    .attr("id", r.id)
                    .attr("name", r.name)
                    .attr("streamUrl", r.stream_url)
                    .attr("homePageUrl", r.home_page_url)
            }));
            respond(&params, true, Some(el))
        }
        "createInternetRadioStation" | "updateInternetRadioStation" => {
            let id = params.get("id").map(|s| s.as_str());
            let name = params.get("name").map(|s| s.as_str()).unwrap_or("Radio");
            let stream_url = params.get("streamUrl").map(|s| s.as_str()).unwrap_or("");
            let home = params.get("homepageUrl").map(|s| s.as_str()).unwrap_or("");
            state.store.upsert_radio_station(id, name, stream_url, home);
            respond(&params, true, None)
        }
        "deleteInternetRadioStation" => {
            if let Some(id) = params.get("id") {
                state.store.delete_radio_station(id);
            }
            respond(&params, true, None)
        }
        _ => subsonic_err(&params, 0, &format!("Unsupported endpoint: {}", action)),
    }
}

fn user_elem(u: &User) -> XmlElem {
    XmlElem::new("user")
        .attr("username", u.username.clone())
        .attr("adminRole", u.is_admin)
        .attr("settingsRole", true)
        .attr("streamRole", true)
        .attr("jukeboxRole", false)
        .attr("downloadRole", true)
        .attr("uploadRole", false)
        .attr("playlistRole", true)
        .attr("coverArtRole", true)
        .attr("commentRole", false)
        .attr("podcastRole", false)
        .attr("shareRole", false)
}

fn url_form_decode(body: &str) -> Vec<(String, String)> {
    if body.is_empty() {
        return vec![];
    }
    body.split('&')
        .filter_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            let dk = urlencoding::decode(k).ok()?.into_owned();
            let dv = urlencoding::decode(v).ok()?.into_owned();
            Some((dk, dv))
        })
        .collect()
}
