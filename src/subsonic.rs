use crate::engine::{Engine, content_type_for_format};
use crate::models::{Track, User};
use crate::sources::SourceRegistry;
use crate::sources::local::LocalSource;
use crate::sources::soulseek::SoulseekSource;
use crate::store::Store;
use crate::taste::TasteEngine;
use axum::extract::{Path, RawQuery, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

const API_VERSION: &str = "1.16.1";
const SERVER_TYPE: &str = "beatlab";
const SERVER_VERSION: &str = "0.2.0";

pub type NowPlayingMap = Arc<Mutex<HashMap<i64, (Track, i64, String)>>>;

#[derive(Clone)]
pub struct AppState {
    pub store: Store,
    pub registry: Arc<SourceRegistry>,
    pub engine: Arc<Engine>,
    pub taste: Arc<TasteEngine>,
    pub local: Arc<LocalSource>,
    pub soulseek: Arc<SoulseekSource>,
    pub now_playing: NowPlayingMap,
    pub admin_password: String,
    pub admin_sessions: Arc<Mutex<HashSet<String>>>,
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
        if self.attrs.is_empty() && self.children.is_empty() && self.name != "line" {
            if let Some(ref t) = self.text {
                return Value::String(t.clone());
            }
        }
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
            if items.len() == 1 && !is_subsonic_array_child(&self.name, &name) {
                obj.insert(name, items.into_iter().next().unwrap());
            } else {
                obj.insert(name, Value::Array(items));
            }
        }
        Value::Object(obj)
    }
}

fn is_subsonic_array_child(parent: &str, child: &str) -> bool {
    if parent == "subsonic-response" {
        return child == "openSubsonicExtensions";
    }
    matches!(
        child,
        "song"
            | "album"
            | "artist"
            | "index"
            | "playlist"
            | "entry"
            | "genre"
            | "musicFolder"
            | "internetRadioStation"
            | "user"
            | "openSubsonicExtensions"
            | "child"
            | "similarArtist"
            | "structuredLyrics"
            | "line"
    )
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn respond(params: &HashMap<String, String>, ok: bool, inner: Option<XmlElem>) -> Response {
    respond_children(params, ok, inner.into_iter().collect())
}

fn respond_children(params: &HashMap<String, String>, ok: bool, children: Vec<XmlElem>) -> Response {
    let mut root = XmlElem::new("subsonic-response")
        .attr("status", if ok { "ok" } else { "failed" })
        .attr("version", API_VERSION)
        .attr("type", SERVER_TYPE)
        .attr("serverVersion", SERVER_VERSION)
        .attr("openSubsonic", true);

    for c in children {
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
            let raw_cb = params.get("callback").map(|s| s.as_str()).unwrap_or("callback");
            let cb = if !raw_cb.is_empty()
                && raw_cb
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'$'))
                && !raw_cb.as_bytes()[0].is_ascii_digit()
            {
                raw_cb
            } else {
                "callback"
            };
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
    track_elem_named("song", store, user_id, t)
}

fn track_elem_named(tag: &str, store: &Store, user_id: i64, t: &Track) -> XmlElem {
    let starred = store.is_starred(user_id, &t.id);
    let rating = store.user_rating(user_id, &t.id);
    let mut e = XmlElem::new(tag)
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

fn album_elem(album_name: &str, trs: &[Track]) -> XmlElem {
    let first = trs.first().unwrap();
    let duration: i32 = trs.iter().map(|x| x.duration).sum();
    let play_count: i64 = trs.iter().map(|x| x.play_count as i64).sum();
    let max_created = trs.iter().map(|x| x.created).max().unwrap_or(0);
    let created_iso = chrono::DateTime::from_timestamp(max_created, 0)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(iso_now);
    let best_cov = trs
        .iter()
        .find(|x| !x.cover_url.is_empty())
        .map(|x| x.id.clone())
        .unwrap_or_else(|| first.id.clone());
    let rep_artist = trs
        .iter()
        .find(|x| !crate::models::is_generic_folder_name(&x.artist))
        .unwrap_or(first);
    let best_year = trs.iter().map(|x| x.year).find(|&y| y > 0).unwrap_or(0);
    let best_genre = trs
        .iter()
        .find(|x| !x.genre.trim().is_empty())
        .map(|x| x.genre.clone())
        .unwrap_or_default();
    let mut el = XmlElem::new("album")
        .attr("id", first.album_id())
        .attr("name", album_name)
        .attr("title", album_name)
        .attr("album", album_name)
        .attr("artist", rep_artist.artist.clone())
        .attr("artistId", rep_artist.artist_id())
        .attr("coverArt", best_cov)
        .attr("songCount", trs.len())
        .attr("duration", duration)
        .attr("playCount", play_count)
        .attr("created", created_iso);
    if best_year > 0 {
        el = el.attr("year", best_year);
    }
    if !best_genre.is_empty() {
        el = el.attr("genre", best_genre);
    }
    el
}

fn iso_now() -> String {
    chrono::Utc::now().to_rfc3339()
}

pub async fn dispatch(
    State(state): State<AppState>,
    Path(action_raw): Path<String>,
    RawQuery(raw_q): RawQuery,
    headers: HeaderMap,
    body: String,
) -> Response {
    let mut params: HashMap<String, String> = HashMap::new();
    let mut multi: HashMap<String, Vec<String>> = HashMap::new();
    for (k, v) in url_form_decode(raw_q.as_deref().unwrap_or(""))
        .into_iter()
        .chain(url_form_decode(&body))
    {
        params.entry(k.clone()).or_insert_with(|| v.clone());
        multi.entry(k).or_default().push(v);
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
        "getOpenSubsonicExtensions" => respond_children(
            &params,
            true,
            vec![
                XmlElem::new("openSubsonicExtensions")
                    .attr("name", "transcodeOffset")
                    .attr("versions", serde_json::json!([1])),
                XmlElem::new("openSubsonicExtensions")
                    .attr("name", "formPost")
                    .attr("versions", serde_json::json!([1])),
                XmlElem::new("openSubsonicExtensions")
                    .attr("name", "songLyrics")
                    .attr("versions", serde_json::json!([1])),
            ],
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
            let raw_q = params.get("query").map(|s| s.as_str()).unwrap_or("");
            let q = raw_q.trim().trim_matches('"').trim();
            let song_count = params
                .get("songCount")
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(20);
            let song_offset = params
                .get("songOffset")
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(0);
            let album_count = params
                .get("albumCount")
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(20);
            let album_offset = params
                .get("albumOffset")
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(0);
            let artist_count = params
                .get("artistCount")
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(20);
            let artist_offset = params
                .get("artistOffset")
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(0);

            let max_offset = song_offset.max(album_offset).max(artist_offset);
            let fetch_limit = song_count.max(album_count).max(artist_count).max(20) + max_offset;
            let mut pool = state.store.search_tracks(q, fetch_limit);
            if !q.is_empty() && pool.len() < fetch_limit {
                let remote = state.registry.search_all(q, fetch_limit).await;
                pool.extend(remote);
                pool = crate::sources::dedup_and_rank_search(pool, q, fetch_limit);
            }

            let tag = if action == "search3" { "searchResult3" } else { "searchResult2" };
            let mut el = XmlElem::new(tag);

            if artist_count > 0 {
                let mut seen_artists: HashMap<String, (String, String, HashSet<String>)> = HashMap::new();
                let mut artist_order = Vec::new();
                for tr in &pool {
                    if crate::models::is_generic_folder_name(&tr.artist) {
                        continue;
                    }
                    let aid = tr.artist_id();
                    if !seen_artists.contains_key(&aid) {
                        artist_order.push(aid.clone());
                        seen_artists.insert(aid.clone(), (tr.artist.clone(), tr.id.clone(), HashSet::new()));
                    }
                    seen_artists.get_mut(&aid).unwrap().2.insert(tr.album_id());
                }
                for aid in artist_order.into_iter().skip(artist_offset).take(artist_count) {
                    if let Some((name, cover, albs)) = seen_artists.get(&aid) {
                        el = el.child(
                            XmlElem::new("artist")
                                .attr("id", aid)
                                .attr("name", name.clone())
                                .attr("coverArt", cover.clone())
                                .attr("albumCount", albs.len()),
                        );
                    }
                }
            }

            if album_count > 0 {
                let mut albums_map: HashMap<String, (String, Vec<Track>)> = HashMap::new();
                let mut album_order = Vec::new();
                for tr in &pool {
                    if crate::models::is_generic_folder_name(&tr.artist) {
                        continue;
                    }
                    let albid = tr.album_id();
                    if !albums_map.contains_key(&albid) {
                        album_order.push(albid.clone());
                        albums_map.insert(albid.clone(), (tr.album.clone(), Vec::new()));
                    }
                    albums_map.get_mut(&albid).unwrap().1.push(tr.clone());
                }
                for albid in album_order.into_iter().skip(album_offset).take(album_count) {
                    if let Some((alb_name, trs)) = albums_map.get(&albid) {
                        el = el.child(album_elem(alb_name, trs));
                    }
                }
            }

            for tr in pool.iter().skip(song_offset).take(song_count) {
                el = el.child(track_elem(&state.store, user.id, tr));
            }
            state.engine.warm_resolve(
                pool.iter().skip(song_offset).take(5).cloned().collect(),
                5,
            );
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
            let idx_elems = build_artist_indexes(artists);
            respond(&params, true, Some(XmlElem::new("artists").children(idx_elems)))
        }
        "getArtist" => {
            let id = params.get("id").map(|s| s.as_str()).unwrap_or("");
            let Some(name) = state.store.artist_name_from_id(id) else {
                return subsonic_err(&params, 70, "Artist not found");
            };
            let tracks = state.store.tracks_by_artist(&name, 500);
            let mut albums_map: HashMap<String, (String, Vec<Track>)> = HashMap::new();
            for tr in tracks {
                let alb_id = tr.album_id();
                let alb_name = tr.album.clone();
                albums_map
                    .entry(alb_id)
                    .or_insert_with(|| (alb_name, Vec::new()))
                    .1
                    .push(tr);
            }
            let mut albums: Vec<_> = albums_map
                .into_values()
                .map(|(alb, mut trs)| {
                    trs.sort_by_key(|t| std::cmp::Reverse(t.quality_score()));
                    let mut seen_keys = HashSet::new();
                    trs.retain(|t| seen_keys.insert(t.song_key()));
                    trs.sort_by(|a, b| a.track_no.cmp(&b.track_no).then_with(|| a.title.cmp(&b.title)));
                    let y = trs.iter().map(|t| t.year).find(|&y| y > 0).unwrap_or(0);
                    (y, album_elem(&alb, &trs))
                })
                .collect();
            albums.sort_by_key(|a| std::cmp::Reverse(a.0));
            let album_count = albums.len();
            respond(
                &params,
                true,
                Some(
                    XmlElem::new("artist")
                        .attr("id", id)
                        .attr("name", name)
                        .attr("coverArt", id)
                        .attr("albumCount", album_count)
                        .children(albums.into_iter().map(|(_, el)| el)),
                ),
            )
        }
        "getAlbum" => {
            let id = params.get("id").map(|s| s.as_str()).unwrap_or("");
            let all = state.store.all_tracks(5000);
            let mut trs: Vec<Track> = all.into_iter().filter(|t| t.album_id() == id).collect();
            if trs.is_empty() {
                return subsonic_err(&params, 70, "Album not found");
            }
            trs.sort_by(|a, b| {
                a.track_no
                    .cmp(&b.track_no)
                    .then_with(|| b.quality_score().cmp(&a.quality_score()))
                    .then_with(|| a.title.cmp(&b.title))
            });
            // Keep best quality copy per canonical song_key inside an album
            let mut seen_songs: HashSet<String> = HashSet::new();
            trs.retain(|t| seen_songs.insert(t.song_key()));
            state.engine.warm_resolve(trs.clone(), 5);
            let first = trs.first().unwrap().clone();
            let mut el = album_elem(&first.album, &trs);
            for tr in &trs {
                el = el.child(track_elem(&state.store, user.id, tr));
            }
            respond(&params, true, Some(el))
        }
        "getRandomSongs" => {
            let size = params
                .get("size")
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(20)
                .min(200);
            let genre_filter = params.get("genre").map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty());
            let from_year = params.get("fromYear").and_then(|v| v.parse::<i32>().ok());
            let to_year = params.get("toYear").and_then(|v| v.parse::<i32>().ok());

            let mut recs = state.taste.recommend_for_user(user.id, size * 3);
            if let Some(ref g) = genre_filter {
                recs.retain(|t| t.genre.to_lowercase().contains(g));
            }
            if let Some(fy) = from_year {
                recs.retain(|t| t.year >= fy);
            }
            if let Some(ty) = to_year {
                recs.retain(|t| t.year <= ty);
            }
            recs.truncate(size);
            state.engine.warm_resolve(recs.clone(), 5);
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
            state.engine.warm_resolve(trs.clone(), 5);
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
            set_starred(&state, user.id, &multi, true);
            respond(&params, true, None)
        }
        "unstar" => {
            set_starred(&state, user.id, &multi, false);
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
            state.taste.ensure_auto_playlists_fresh(user.id);
            let pls = state.store.list_playlists(user.id);
            let el = XmlElem::new("playlists").children(
                pls.into_iter()
                    .map(|p| playlist_elem(&state.store, &p, &user.username, p.track_count)),
            );
            respond(&params, true, Some(el))
        }
        "getPlaylist" => {
            let id = params.get("id").map(|s| s.as_str()).unwrap_or("");
            let Some(p) = state.store.get_playlist(id) else {
                return subsonic_err(&params, 70, "Playlist not found");
            };
            if !p.public && p.user_id != user.id && !user.is_admin {
                return subsonic_err(&params, 50, "Permission denied");
            }
            let trs = state.store.playlist_tracks(id);
            state.engine.warm_resolve(trs.clone(), 5);
            let mut el = playlist_elem(&state.store, &p, &user.username, trs.len());
            for tr in &trs {
                el = el.child(track_elem_named("entry", &state.store, user.id, tr));
            }
            respond(&params, true, Some(el))
        }
        "createPlaylist" => {
            let song_ids: Vec<String> = multi.get("songId").cloned().unwrap_or_default();
            let saved = if let Some(pid) = params.get("playlistId").filter(|s| !s.is_empty()) {
                let Some(p) = state.store.get_playlist(pid) else {
                    return subsonic_err(&params, 70, "Playlist not found");
                };
                if p.user_id != user.id && !user.is_admin {
                    return subsonic_err(&params, 50, "Permission denied");
                }
                if let Some(name) = params.get("name").map(|s| s.as_str()) {
                    state.store.update_playlist_meta(pid, Some(name), None, None);
                }
                let _ = state.store.set_playlist_tracks(pid, &song_ids);
                state.store.get_playlist(pid)
            } else {
                let name = params.get("name").map(|s| s.as_str()).unwrap_or("New Playlist");
                state
                    .store
                    .create_user_playlist(user.id, name, true, &song_ids)
                    .ok()
            };
            let body = saved.map(|p| {
                let mut el = playlist_elem(&state.store, &p, &user.username, p.track_count);
                for tr in &state.store.playlist_tracks(&p.id) {
                    el = el.child(track_elem_named("entry", &state.store, user.id, tr));
                }
                el
            });
            respond(&params, true, body)
        }
        "updatePlaylist" => {
            let id = params.get("playlistId").map(|s| s.as_str()).unwrap_or("");
            let Some(p) = state.store.get_playlist(id) else {
                return subsonic_err(&params, 70, "Playlist not found");
            };
            if p.user_id != user.id && !user.is_admin {
                return subsonic_err(&params, 50, "Permission denied");
            }
            let name = params.get("name").map(|s| s.as_str());
            let comment = params.get("comment").map(|s| s.as_str());
            let public = params.get("public").map(|s| s == "true");
            state.store.update_playlist_meta(id, name, comment, public);
            let to_add: Vec<String> = multi.get("songIdToAdd").cloned().unwrap_or_default();
            let to_remove: Vec<usize> = multi
                .get("songIndexToRemove")
                .map(|v| v.iter().filter_map(|s| s.parse::<usize>().ok()).collect())
                .unwrap_or_default();
            if !to_add.is_empty() || !to_remove.is_empty() {
                let _ = state.store.update_playlist_tracks(id, &to_add, &to_remove);
            }
            respond(&params, true, None)
        }
        "deletePlaylist" => {
            let id = params.get("id").map(|s| s.as_str()).unwrap_or("");
            let Some(p) = state.store.get_playlist(id) else {
                return subsonic_err(&params, 70, "Playlist not found");
            };
            if p.user_id != user.id && !user.is_admin {
                return subsonic_err(&params, 50, "Permission denied");
            }
            state.store.delete_playlist(id);
            respond(&params, true, None)
        }
        "getSimilarSongs" | "getSimilarSongs2" => {
            let id = params.get("id").map(|s| s.as_str()).unwrap_or("");
            let count = params.get("count").and_then(|v| v.parse().ok()).unwrap_or(25).min(100);
            let trs = if let Some(seed) = resolve_seed_track(&state.store, id) {
                state.taste.similar_songs_async(&seed, user.id, count).await
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
            state.engine.warm_resolve(trs.clone(), 5);
            let mut el = XmlElem::new("topSongs");
            for tr in &trs {
                el = el.child(track_elem(&state.store, user.id, tr));
            }
            respond(&params, true, Some(el))
        }
        "getNowPlaying" => {
            let mut np = state.now_playing.lock().unwrap();
            let now = chrono::Utc::now().timestamp();
            np.retain(|_, (_, at, _)| now - *at < 3600);
            let mut el = XmlElem::new("nowPlaying");
            for (tr, at, player) in np.values() {
                let mins_ago = ((chrono::Utc::now().timestamp() - at) / 60).max(0);
                el = el.child(
                    track_elem_named("entry", &state.store, user.id, tr)
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
            let ids: Vec<String> = multi.get("id").cloned().unwrap_or_default();
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
                        el = el.child(track_elem_named("entry", &state.store, user.id, &tr));
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
        "getAlbumList" | "getAlbumList2" => {
            let tag = if action == "getAlbumList2" { "albumList2" } else { "albumList" };
            let albums = build_album_list(&state, user.id, &params);
            let el = XmlElem::new(tag).children(albums);
            respond(&params, true, Some(el))
        }
        "getIndexes" => {
            let artists = state.store.artists();
            let idx_elems = build_artist_indexes(artists);
            respond(
                &params,
                true,
                Some(
                    XmlElem::new("indexes")
                        .attr("lastModified", chrono::Utc::now().timestamp_millis())
                        .children(idx_elems),
                ),
            )
        }
        "getArtistInfo" | "getArtistInfo2" => {
            let tag = if action == "getArtistInfo2" { "artistInfo2" } else { "artistInfo" };
            let id = params.get("id").map(|s| s.as_str()).unwrap_or("");
            let count = params.get("count").and_then(|v| v.parse().ok()).unwrap_or(12);
            let el = fetch_artist_info_elem(&state.store, tag, id, count).await;
            respond(&params, true, Some(el))
        }
        "getAlbumInfo" | "getAlbumInfo2" => {
            let id = params.get("id").map(|s| s.as_str()).unwrap_or("");
            let mut el = XmlElem::new("albumInfo");
            if let Some(tr) = resolve_seed_track(&state.store, id) {
                if tr.cover_url.starts_with("http") {
                    el = el
                        .child(XmlElem::new("smallImageUrl").text(tr.cover_url.clone()))
                        .child(XmlElem::new("mediumImageUrl").text(tr.cover_url.clone()))
                        .child(XmlElem::new("largeImageUrl").text(tr.cover_url));
                }
            }
            respond(&params, true, Some(el))
        }
        "getLyricsBySongId" => {
            let id = params.get("id").map(|s| s.as_str()).unwrap_or("");
            let el = fetch_structured_lyrics_elem(&state.store, id).await;
            respond(&params, true, Some(el))
        }
        "getLyrics" => {
            let artist = params.get("artist").map(|s| s.as_str()).unwrap_or("");
            let title = params.get("title").map(|s| s.as_str()).unwrap_or("");
            let el = fetch_plain_lyrics_elem(&state.store, artist, title).await;
            respond(&params, true, Some(el))
        }
        _ => subsonic_err(&params, 0, &format!("Unsupported endpoint: {}", action)),
    }
}

fn build_album_list(state: &AppState, user_id: i64, params: &HashMap<String, String>) -> Vec<XmlElem> {
    use rand::seq::SliceRandom;

    let list_type = params.get("type").map(|s| s.as_str()).unwrap_or("newest");
    let size = params
        .get("size")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(10)
        .min(500);
    let offset = params
        .get("offset")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);

    let tracks = if list_type == "starred" {
        state.store.starred_tracks(user_id)
    } else {
        state.store.all_tracks(5000)
    };

    let mut groups: HashMap<String, (String, Vec<Track>)> = HashMap::new();
    for t in tracks {
        let id = t.album_id();
        groups
            .entry(id)
            .or_insert_with(|| (t.album.clone(), Vec::new()))
            .1
            .push(t);
    }

    let mut albums: Vec<(String, Vec<Track>)> = groups.into_values().collect();

    if list_type == "byGenre" {
        let genre = params
            .get("genre")
            .map(|s| s.trim().to_lowercase())
            .unwrap_or_default();
        albums.retain(|(_, trs)| {
            trs.iter()
                .any(|t| t.genre.to_lowercase().contains(&genre))
        });
    } else if list_type == "byYear" {
        let from_y = params
            .get("fromYear")
            .and_then(|v| v.parse::<i32>().ok())
            .unwrap_or(0);
        let to_y = params
            .get("toYear")
            .and_then(|v| v.parse::<i32>().ok())
            .unwrap_or(9999);
        let (lo, hi) = if from_y <= to_y {
            (from_y, to_y)
        } else {
            (to_y, from_y)
        };
        albums.retain(|(_, trs)| {
            trs.first()
                .map(|t| t.year >= lo && t.year <= hi)
                .unwrap_or(false)
        });
    }

    match list_type {
        "random" => {
            let mut rng = rand::thread_rng();
            albums.shuffle(&mut rng);
        }
        "alphabeticalByName" => {
            albums.sort_by_key(|a| a.0.to_lowercase());
        }
        "alphabeticalByArtist" => {
            albums.sort_by(|a, b| {
                let aa = a.1.first().map(|t| t.artist.to_lowercase()).unwrap_or_default();
                let ba = b.1.first().map(|t| t.artist.to_lowercase()).unwrap_or_default();
                aa.cmp(&ba).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase()))
            });
        }
        "frequent" => {
            albums.sort_by(|a, b| {
                let pa: i64 = a.1.iter().map(|t| t.play_count as i64).sum();
                let pb: i64 = b.1.iter().map(|t| t.play_count as i64).sum();
                let ca = a.1.iter().map(|t| t.created).max().unwrap_or(0);
                let cb = b.1.iter().map(|t| t.created).max().unwrap_or(0);
                pb.cmp(&pa).then_with(|| cb.cmp(&ca))
            });
        }
        "recent" => {
            albums.sort_by(|a, b| {
                let la = a.1.iter().map(|t| t.last_played).max().unwrap_or(0);
                let lb = b.1.iter().map(|t| t.last_played).max().unwrap_or(0);
                let ca = a.1.iter().map(|t| t.created).max().unwrap_or(0);
                let cb = b.1.iter().map(|t| t.created).max().unwrap_or(0);
                lb.cmp(&la).then_with(|| cb.cmp(&ca))
            });
        }
        "highest" => {
            albums.sort_by(|a, b| {
                let ra = a
                    .1
                    .iter()
                    .map(|t| state.store.user_rating(user_id, &t.id))
                    .max()
                    .unwrap_or(0);
                let rb = b
                    .1
                    .iter()
                    .map(|t| state.store.user_rating(user_id, &t.id))
                    .max()
                    .unwrap_or(0);
                let qa = a.1.iter().map(|t| t.quality_score()).max().unwrap_or(0);
                let qb = b.1.iter().map(|t| t.quality_score()).max().unwrap_or(0);
                rb.cmp(&ra).then_with(|| qb.cmp(&qa))
            });
        }
        "byYear" => {
            let from_y = params.get("fromYear").and_then(|v| v.parse::<i32>().ok()).unwrap_or(0);
            let to_y = params.get("toYear").and_then(|v| v.parse::<i32>().ok()).unwrap_or(9999);
            let desc = from_y > to_y;
            albums.sort_by(|a, b| {
                let ya = a.1.first().map(|t| t.year).unwrap_or(0);
                let yb = b.1.first().map(|t| t.year).unwrap_or(0);
                if desc { yb.cmp(&ya) } else { ya.cmp(&yb) }
            });
        }
        _ => {
            // "newest" and default
            albums.sort_by(|a, b| {
                let ca = a.1.iter().map(|t| t.created).max().unwrap_or(0);
                let cb = b.1.iter().map(|t| t.created).max().unwrap_or(0);
                cb.cmp(&ca)
            });
        }
    }

    albums
        .into_iter()
        .skip(offset)
        .take(size)
        .map(|(name, trs)| album_elem(&name, &trs))
        .collect()
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

fn build_artist_indexes(artists: Vec<(String, String, usize)>) -> Vec<XmlElem> {
    let mut buckets: std::collections::BTreeMap<String, Vec<(String, String, usize)>> =
        std::collections::BTreeMap::new();
    for (id, name, count) in artists {
        let ch = name.trim().chars().next().unwrap_or('#').to_ascii_uppercase();
        let key = if ch.is_ascii_alphabetic() {
            ch.to_string()
        } else {
            "#".to_string()
        };
        buckets.entry(key).or_default().push((id, name, count));
    }
    buckets
        .into_iter()
        .map(|(letter, list)| {
            XmlElem::new("index").attr("name", letter).children(
                list.into_iter().map(|(id, name, count)| {
                    XmlElem::new("artist")
                        .attr("id", id.clone())
                        .attr("name", name)
                        .attr("coverArt", id)
                        .attr("albumCount", count)
                }),
            )
        })
        .collect()
}

fn resolve_seed_track(store: &Store, id: &str) -> Option<Track> {
    if let Some(tr) = store.track(id) {
        return Some(tr);
    }
    if id.starts_with("ar:") {
        if let Some(name) = store.artist_name_from_id(id) {
            let mut trs = store.tracks_by_artist(&name, 20);
            trs.sort_by(|a, b| b.play_count.cmp(&a.play_count).then_with(|| b.quality_score().cmp(&a.quality_score())));
            if let Some(best) = trs.into_iter().next() {
                return Some(best);
            }
            return Some(Track {
                artist: name,
                ..Default::default()
            });
        }
    }
    if id.starts_with("al:") {
        let all = store.all_tracks(3000);
        let mut trs: Vec<Track> = all.into_iter().filter(|t| t.album_id() == id).collect();
        trs.sort_by(|a, b| b.play_count.cmp(&a.play_count).then_with(|| b.quality_score().cmp(&a.quality_score())));
        return trs.into_iter().next();
    }
    None
}

fn metadata_http() -> reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(3))
                .user_agent("Beatlab/0.2")
                .build()
                .unwrap_or_default()
        })
        .clone()
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct CachedArtistInfo {
    at: i64,
    image_url: String,
    biography: String,
    similar: Vec<String>,
}

async fn fetch_artist_info_elem(store: &Store, tag: &str, id: &str, count: usize) -> XmlElem {
    let mut el = XmlElem::new(tag);
    let raw_name = if let Some(n) = store.artist_name_from_id(id) {
        n
    } else if let Some(t) = store.track(id) {
        t.artist
    } else {
        return el;
    };
    let artist_name = {
        let p = crate::models::primary_artist(&raw_name);
        if p.is_empty() { raw_name } else { p }
    };
    if crate::models::is_generic_folder_name(&artist_name) {
        return el;
    }

    let kv_key = format!("artinfo:v1:{}", crate::models::slug(&artist_name));
    let now = chrono::Utc::now().timestamp();
    let cached: Option<CachedArtistInfo> = store
        .kv_get(&kv_key)
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .filter(|c: &CachedArtistInfo| now - c.at < 7 * 86400);

    let info = if let Some(c) = cached {
        c
    } else {
        let client = metadata_http();
        let mut fetched = CachedArtistInfo {
            at: now,
            ..Default::default()
        };
        let search_url = format!(
            "https://api.deezer.com/search/artist?q={}&limit=1",
            urlencoding::encode(&artist_name)
        );
        if let Ok(resp) = client.get(&search_url).send().await {
            if let Ok(json) = resp.json::<serde_json::Value>().await {
                if let Some(first) = json.get("data").and_then(|d| d.as_array()).and_then(|a| a.first()) {
                    fetched.image_url = first
                        .get("picture_xl")
                        .or_else(|| first.get("picture_big"))
                        .or_else(|| first.get("picture_medium"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    if let Some(aid) = first.get("id").and_then(|v| v.as_u64()) {
                        let rel_url = format!("https://api.deezer.com/artist/{}/related?limit=15", aid);
                        if let Ok(r2) = client.get(&rel_url).send().await {
                            if let Ok(j2) = r2.json::<serde_json::Value>().await {
                                if let Some(arr) = j2.get("data").and_then(|d| d.as_array()) {
                                    for item in arr {
                                        if let Some(n) = item.get("name").and_then(|v| v.as_str()) {
                                            fetched.similar.push(n.to_string());
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        let wiki_url = format!(
            "https://en.wikipedia.org/api/rest_v1/page/summary/{}",
            urlencoding::encode(&artist_name.replace(' ', "_"))
        );
        if let Ok(resp) = client.get(&wiki_url).send().await {
            if resp.status().is_success() {
                if let Ok(json) = resp.json::<serde_json::Value>().await {
                    if let Some(ext) = json.get("extract").and_then(|v| v.as_str()) {
                        fetched.biography = ext.to_string();
                    }
                }
            }
        }
        if let Ok(s) = serde_json::to_string(&fetched) {
            store.kv_set(&kv_key, &s);
        }
        fetched
    };

    if !info.biography.is_empty() {
        el = el.child(XmlElem::new("biography").text(info.biography));
    }
    if !info.image_url.is_empty() {
        el = el
            .child(XmlElem::new("smallImageUrl").text(info.image_url.clone()))
            .child(XmlElem::new("mediumImageUrl").text(info.image_url.clone()))
            .child(XmlElem::new("largeImageUrl").text(info.image_url));
    }

    // Match similar artists against local store so clicking them in Feishin opens their artist page
    let local_artists: HashMap<String, (String, String, usize)> = store
        .artists()
        .into_iter()
        .map(|(aid, name, cnt)| (name.to_lowercase(), (aid, name, cnt)))
        .collect();

    for sim_name in info.similar.into_iter().take(count) {
        if let Some((aid, lname, cnt)) = local_artists.get(&sim_name.to_lowercase()) {
            el = el.child(
                XmlElem::new("similarArtist")
                    .attr("id", aid.clone())
                    .attr("name", lname.clone())
                    .attr("coverArt", aid.clone())
                    .attr("albumCount", *cnt),
            );
        }
    }
    el
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct CachedLyrics {
    at: i64,
    plain: String,
    synced: String,
}

async fn get_or_fetch_lyrics(store: &Store, artist: &str, title: &str) -> Option<CachedLyrics> {
    if artist.trim().is_empty() || title.trim().is_empty() {
        return None;
    }
    let clean_title = crate::models::core_song_title(title);
    let key = format!("lyrics:v1:{}|{}", crate::models::slug(artist), clean_title);
    let now = chrono::Utc::now().timestamp();
    if let Some(raw) = store.kv_get(&key) {
        if let Ok(c) = serde_json::from_str::<CachedLyrics>(&raw) {
            if now - c.at < 14 * 86400 {
                return Some(c);
            }
        }
    }

    let url = format!(
        "https://lrclib.net/api/get?artist_name={}&track_name={}",
        urlencoding::encode(artist.trim()),
        urlencoding::encode(title.trim())
    );
    let mut result = CachedLyrics {
        at: now,
        ..Default::default()
    };
    if let Ok(resp) = metadata_http().get(&url).send().await {
        if resp.status().is_success() {
            if let Ok(json) = resp.json::<serde_json::Value>().await {
                result.plain = json.get("plainLyrics").and_then(|v| v.as_str()).unwrap_or("").to_string();
                result.synced = json.get("syncedLyrics").and_then(|v| v.as_str()).unwrap_or("").to_string();
            }
        }
    }
    if let Ok(s) = serde_json::to_string(&result) {
        store.kv_set(&key, &s);
    }
    Some(result)
}

async fn fetch_structured_lyrics_elem(store: &Store, id: &str) -> XmlElem {
    let mut root = XmlElem::new("lyricsList");
    let Some(tr) = store.track(id) else {
        return root;
    };
    let Some(lyr) = get_or_fetch_lyrics(store, &tr.artist, &tr.title).await else {
        return root;
    };
    if !lyr.synced.is_empty() {
        let mut sl = XmlElem::new("structuredLyrics")
            .attr("displayArtist", tr.artist.clone())
            .attr("displayTitle", tr.title.clone())
            .attr("lang", "und")
            .attr("synced", true);
        for (ms, text) in parse_lrc_lines(&lyr.synced) {
            sl = sl.child(XmlElem::new("line").attr("start", ms).text(text));
        }
        root = root.child(sl);
    } else if !lyr.plain.is_empty() {
        let mut sl = XmlElem::new("structuredLyrics")
            .attr("displayArtist", tr.artist.clone())
            .attr("displayTitle", tr.title.clone())
            .attr("lang", "und")
            .attr("synced", false);
        for line in lyr.plain.lines() {
            sl = sl.child(XmlElem::new("line").text(line.to_string()));
        }
        root = root.child(sl);
    }
    root
}

async fn fetch_plain_lyrics_elem(store: &Store, artist: &str, title: &str) -> XmlElem {
    let mut el = XmlElem::new("lyrics")
        .attr("artist", artist.to_string())
        .attr("title", title.to_string());
    if let Some(lyr) = get_or_fetch_lyrics(store, artist, title).await {
        if !lyr.plain.is_empty() {
            el = el.text(lyr.plain);
        }
    }
    el
}

fn parse_lrc_lines(lrc: &str) -> Vec<(i64, String)> {
    let mut out = Vec::new();
    for raw in lrc.lines() {
        let line = raw.trim();
        if let Some(rest) = line.strip_prefix('[') {
            if let Some((ts, text)) = rest.split_once(']') {
                if let Some((min_s, sec_s)) = ts.split_once(':') {
                    if let (Ok(mins), Ok(secs)) = (min_s.parse::<i64>(), sec_s.parse::<f64>()) {
                        let ms = mins * 60_000 + (secs * 1000.0).round() as i64;
                        out.push((ms, text.trim().to_string()));
                    }
                }
            }
        }
    }
    out
}

fn iso_utc(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_else(|| "1970-01-01T00:00:00Z".to_string())
}

fn playlist_elem(
    store: &Store,
    p: &crate::models::Playlist,
    fallback_owner: &str,
    song_count: usize,
) -> XmlElem {
    let owner = store
        .get_user_by_id(p.user_id)
        .map(|u| u.username)
        .unwrap_or_else(|| fallback_owner.to_string());
    let cov = p.cover_track_id.clone().unwrap_or_else(|| p.id.clone());
    XmlElem::new("playlist")
        .attr("id", p.id.clone())
        .attr("name", p.name.clone())
        .attr("comment", p.comment.clone())
        .attr("owner", owner)
        .attr("public", p.public)
        .attr("songCount", song_count)
        .attr("duration", p.duration)
        .attr("created", iso_utc(p.created))
        .attr("changed", iso_utc(p.changed))
        .attr("coverArt", cov)
}

fn set_starred(state: &AppState, user_id: i64, multi: &HashMap<String, Vec<String>>, starred: bool) {
    let event = if starred { "star" } else { "unstar" };
    let mut seen: HashSet<String> = HashSet::new();
    let mut targets: Vec<(String, Option<Track>)> = Vec::new();

    if let Some(ids) = multi.get("id") {
        for id in ids {
            if seen.insert(id.clone()) {
                targets.push((id.clone(), state.store.track(id)));
            }
        }
    }

    if let Some(album_ids) = multi.get("albumId") {
        let wanted: HashSet<&str> = album_ids.iter().map(|s| s.as_str()).collect();
        if !wanted.is_empty() {
            for tr in state.store.all_tracks(5000) {
                if wanted.contains(tr.album_id().as_str()) && seen.insert(tr.id.clone()) {
                    targets.push((tr.id.clone(), Some(tr)));
                }
            }
        }
    }

    if let Some(artist_ids) = multi.get("artistId") {
        for aid in artist_ids {
            if let Some(name) = state.store.artist_name_from_id(aid) {
                for tr in state.store.tracks_by_artist(&name, 500) {
                    if seen.insert(tr.id.clone()) {
                        targets.push((tr.id.clone(), Some(tr)));
                    }
                }
            }
        }
    }

    for (id, track_opt) in targets {
        if starred {
            state.store.star_track(user_id, &id);
        } else {
            state.store.unstar_track(user_id, &id);
        }
        if let Some(ref tr) = track_opt {
            state.taste.learn_event(user_id, tr, event, 1.0);
        }
    }
}

fn url_form_decode(body: &str) -> Vec<(String, String)> {
    if body.is_empty() {
        return vec![];
    }
    body.split('&')
        .filter_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            let k_norm = k.replace('+', " ");
            let v_norm = v.replace('+', " ");
            let dk = urlencoding::decode(&k_norm).ok()?.into_owned();
            let dv = urlencoding::decode(&v_norm).ok()?.into_owned();
            Some((dk, dv))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::local::LocalSource;
    use axum::body::to_bytes;

    fn test_state() -> AppState {
        let dir = std::env::temp_dir().join(format!("beatlab_subsonic_test_{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&dir);
        let store = Store::open_in_memory().unwrap();
        store.create_user("admin", "pass", true).unwrap();
        store.upsert_tracks(&[Track {
            id: "t1".into(),
            source: "soulseek".into(),
            title: "Midnight City".into(),
            artist: "M83".into(),
            album: "Hurry Up, We're Dreaming".into(),
            genre: "Electronic".into(),
            year: 2011,
            duration: 243,
            bitrate: 950,
            format: "flac".into(),
            created: 1000,
            ..Default::default()
        }]);
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

    async fn call_subsonic(state: AppState, action: &str, extra: &[(&str, &str)]) -> Value {
        let mut pairs = vec![
            ("u", "admin"),
            ("p", "pass"),
            ("f", "json"),
        ];
        pairs.extend_from_slice(extra);
        let qs = pairs
            .into_iter()
            .map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v)))
            .collect::<Vec<_>>()
            .join("&");
        let resp = dispatch(
            State(state),
            Path(action.into()),
            RawQuery(Some(qs)),
            HeaderMap::new(),
            String::new(),
        )
        .await;
        let bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[test]
    fn to_json_keeps_single_collection_items_as_arrays() {
        let root = XmlElem::new("subsonic-response")
            .attr("status", "ok")
            .child(
                XmlElem::new("searchResult3")
                    .child(XmlElem::new("song").attr("id", "t1"))
                    .child(XmlElem::new("album").attr("id", "a1")),
            );
        let j = root.to_json();
        assert!(j["searchResult3"].is_object());
        assert!(j["searchResult3"]["song"].is_array());
        assert!(j["searchResult3"]["album"].is_array());
    }

    #[tokio::test]
    async fn get_album_list2_returns_albums_array() {
        let st = test_state();
        let j = call_subsonic(st, "getAlbumList2.view", &[("type", "newest"), ("size", "10")]).await;
        let sr = &j["subsonic-response"];
        assert_eq!(sr["status"], "ok");
        let albums = sr["albumList2"]["album"].as_array().expect("album must be array");
        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0]["name"], "Hurry Up, We're Dreaming");
    }

    #[tokio::test]
    async fn search3_handles_quoted_empty_and_single_hit_arrays() {
        let st = test_state();
        let j = call_subsonic(st.clone(), "search3.view", &[("query", "\"\""), ("songCount", "10")]).await;
        let sr = &j["subsonic-response"]["searchResult3"];
        assert_eq!(sr["song"].as_array().unwrap().len(), 1);
        assert_eq!(sr["album"].as_array().unwrap().len(), 1);
        assert_eq!(sr["artist"].as_array().unwrap().len(), 1);

        let j2 = call_subsonic(st, "search3.view", &[("query", "Midnight"), ("songCount", "10")]).await;
        let sr2 = &j2["subsonic-response"]["searchResult3"];
        assert_eq!(sr2["song"][0]["title"], "Midnight City");
    }

    #[tokio::test]
    async fn open_subsonic_extensions_is_top_level_array() {
        let st = test_state();
        let j = call_subsonic(st, "getOpenSubsonicExtensions.view", &[]).await;
        let exts = j["subsonic-response"]["openSubsonicExtensions"]
            .as_array()
            .expect("openSubsonicExtensions must be a JSON array directly under subsonic-response");
        assert!(exts.iter().any(|e| e["name"] == "transcodeOffset"));
        assert!(exts.iter().any(|e| e["name"] == "songLyrics"));
    }

    #[tokio::test]
    async fn save_play_queue_accepts_multiple_ids() {
        let st = test_state();
        st.store.upsert_tracks(&[Track {
            id: "t2".into(),
            source: "local".into(),
            title: "Wait".into(),
            artist: "M83".into(),
            album: "Hurry Up, We're Dreaming".into(),
            genre: "Electronic".into(),
            year: 2011,
            duration: 343,
            ..Default::default()
        }]);
        let _ = call_subsonic(
            st.clone(),
            "savePlayQueue.view",
            &[("current", "t1"), ("position", "12000"), ("id", "t1"), ("id", "t2")],
        )
        .await;
        let q = call_subsonic(st, "getPlayQueue.view", &[]).await;
        let entries = q["subsonic-response"]["playQueue"]["entry"]
            .as_array()
            .expect("playQueue entry must be array");
        assert_eq!(entries.len(), 2);
    }

    #[tokio::test]
    async fn get_artists_groups_alphabetically() {
        let st = test_state();
        let j = call_subsonic(st, "getArtists.view", &[]).await;
        let indexes = j["subsonic-response"]["artists"]["index"]
            .as_array()
            .expect("index must be array");
        assert_eq!(indexes[0]["name"], "M");
        assert_eq!(indexes[0]["artist"][0]["name"], "M83");
    }

    #[test]
    fn url_form_decode_handles_plus_as_space_and_percent() {
        let pairs = url_form_decode("artist+name=Sabrina+Carpenter&query=espresso+%26+more");
        assert_eq!(
            pairs,
            vec![
                ("artist name".to_string(), "Sabrina Carpenter".to_string()),
                ("query".to_string(), "espresso & more".to_string()),
            ]
        );
    }

    #[tokio::test]
    async fn update_playlist_adds_removes_and_enforces_owner() {
        let st = test_state();
        st.store.upsert_tracks(&[Track {
            id: "t2".into(),
            source: "local".into(),
            title: "Wait".into(),
            artist: "M83".into(),
            album: "Hurry Up, We're Dreaming".into(),
            genre: "Electronic".into(),
            year: 2011,
            duration: 343,
            ..Default::default()
        }]);
        let created = call_subsonic(
            st.clone(),
            "createPlaylist.view",
            &[("name", "Roadtrip"), ("songId", "t1")],
        )
        .await;
        let pid = created["subsonic-response"]["playlist"]["id"]
            .as_str()
            .unwrap()
            .to_string();

        let _ = call_subsonic(
            st.clone(),
            "updatePlaylist.view",
            &[
                ("playlistId", &pid),
                ("songIdToAdd", "t2"),
                ("songIndexToRemove", "0"),
            ],
        )
        .await;
        let fetched = call_subsonic(st.clone(), "getPlaylist.view", &[("id", &pid)]).await;
        let entries = fetched["subsonic-response"]["playlist"]["entry"]
            .as_array()
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["id"], "t2");

        // Non-owner non-admin receives Subsonic permission error (50)
        st.store.create_user("bob", "bobpw", false).unwrap();
        let qs = format!(
            "u=bob&p=bobpw&f=json&playlistId={}&name=Hijack",
            urlencoding::encode(&pid)
        );
        let resp = dispatch(
            State(st),
            Path("updatePlaylist.view".into()),
            RawQuery(Some(qs)),
            HeaderMap::new(),
            String::new(),
        )
        .await;
        let bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["subsonic-response"]["error"]["code"], 50);
    }

    #[test]
    fn text_only_subsonic_elements_serialize_as_json_strings() {
        let info = XmlElem::new("artistInfo2")
            .child(XmlElem::new("biography").text("Bio text"))
            .child(XmlElem::new("smallImageUrl").text("https://img.example/s.jpg"))
            .child(
                XmlElem::new("structuredLyrics")
                    .attr("synced", false)
                    .child(XmlElem::new("line").text("Lyric line")),
            );
        let j = info.to_json();
        assert_eq!(j["biography"], "Bio text");
        assert_eq!(j["smallImageUrl"], "https://img.example/s.jpg");
        assert_eq!(j["structuredLyrics"][0]["line"][0]["value"], "Lyric line");

        // Empty container with no attrs/children/text remains an empty object
        let empty = XmlElem::new("artistInfo2").to_json();
        assert!(empty.is_object());
    }
}
