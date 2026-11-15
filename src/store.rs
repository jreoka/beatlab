use crate::models::{CacheEntry, Event, Playlist, RadioStation, Track, User, slug};
use anyhow_lite::Result;
use chrono::Utc;
use md5::{Digest, Md5};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

pub mod anyhow_lite {
    pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
}

#[derive(Clone)]
pub struct Store {
    conn: Arc<Mutex<Connection>>,
}

impl Store {
    pub fn open(data_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(data_dir)?;
        let db_path = data_dir.join("beatlab.db");
        let conn = Connection::open(db_path)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             PRAGMA foreign_keys=ON;
             PRAGMA busy_timeout=5000;",
        )?;
        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        store.migrate()?;
        Ok(store)
    }

    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS users (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                username TEXT UNIQUE NOT NULL,
                password TEXT NOT NULL,
                is_admin INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS tracks (
                id TEXT PRIMARY KEY,
                source TEXT NOT NULL,
                title TEXT NOT NULL,
                artist TEXT NOT NULL,
                album TEXT NOT NULL DEFAULT '',
                genre TEXT NOT NULL DEFAULT '',
                moods TEXT NOT NULL DEFAULT '',
                year INTEGER NOT NULL DEFAULT 0,
                track_no INTEGER NOT NULL DEFAULT 0,
                duration INTEGER NOT NULL DEFAULT 0,
                bitrate INTEGER NOT NULL DEFAULT 0,
                sample_rate INTEGER NOT NULL DEFAULT 0,
                bit_depth INTEGER NOT NULL DEFAULT 0,
                format TEXT NOT NULL DEFAULT '',
                size INTEGER NOT NULL DEFAULT 0,
                stream_url TEXT NOT NULL,
                cover_url TEXT NOT NULL DEFAULT '',
                meta TEXT NOT NULL DEFAULT '',
                play_count INTEGER NOT NULL DEFAULT 0,
                last_played INTEGER NOT NULL DEFAULT 0,
                created INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_tracks_artist ON tracks(artist);
            CREATE INDEX IF NOT EXISTS idx_tracks_genre ON tracks(genre);
            CREATE INDEX IF NOT EXISTS idx_tracks_source ON tracks(source);
            CREATE INDEX IF NOT EXISTS idx_tracks_created ON tracks(created DESC);

            CREATE TABLE IF NOT EXISTS events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                user_id INTEGER NOT NULL,
                track_id TEXT NOT NULL,
                kind TEXT NOT NULL,
                value REAL NOT NULL DEFAULT 1.0,
                at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_events_user ON events(user_id, at DESC);
            CREATE INDEX IF NOT EXISTS idx_events_track ON events(track_id);

            CREATE TABLE IF NOT EXISTS stars (
                user_id INTEGER NOT NULL,
                track_id TEXT NOT NULL,
                at INTEGER NOT NULL,
                PRIMARY KEY(user_id, track_id)
            );
            CREATE TABLE IF NOT EXISTS ratings (
                user_id INTEGER NOT NULL,
                track_id TEXT NOT NULL,
                rating INTEGER NOT NULL,
                at INTEGER NOT NULL,
                PRIMARY KEY(user_id, track_id)
            );
            CREATE TABLE IF NOT EXISTS playlists (
                id TEXT PRIMARY KEY,
                user_id INTEGER NOT NULL,
                name TEXT NOT NULL,
                comment TEXT NOT NULL DEFAULT '',
                public INTEGER NOT NULL DEFAULT 1,
                is_auto INTEGER NOT NULL DEFAULT 0,
                kind TEXT NOT NULL DEFAULT 'user',
                created INTEGER NOT NULL,
                changed INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS playlist_tracks (
                playlist_id TEXT NOT NULL,
                pos INTEGER NOT NULL,
                track_id TEXT NOT NULL,
                PRIMARY KEY(playlist_id, pos)
            );
            CREATE TABLE IF NOT EXISTS radio_stations (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                stream_url TEXT NOT NULL,
                home_page_url TEXT NOT NULL DEFAULT ''
            );
            CREATE TABLE IF NOT EXISTS play_queues (
                user_id INTEGER PRIMARY KEY,
                current_id TEXT NOT NULL DEFAULT '',
                position_ms INTEGER NOT NULL DEFAULT 0,
                track_ids TEXT NOT NULL DEFAULT '[]',
                changed_by TEXT NOT NULL DEFAULT '',
                updated INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS cache_entries (
                track_id TEXT PRIMARY KEY,
                path TEXT NOT NULL,
                bytes INTEGER NOT NULL,
                last_hit INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS kv (
                k TEXT PRIMARY KEY,
                v TEXT NOT NULL
            );",
        )?;
        Ok(())
    }

    // ---------- Users ----------

    pub fn user_count(&self) -> usize {
        let conn = self.conn.lock().unwrap();
        conn.query_row("SELECT COUNT(*) FROM users", [], |r| r.get::<_, i64>(0))
            .unwrap_or(0) as usize
    }

    pub fn create_user(&self, username: &str, password: &str, is_admin: bool) -> Result<User> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO users(username, password, is_admin) VALUES(?1, ?2, ?3)",
            params![username, password, if is_admin { 1 } else { 0 }],
        )?;
        let id = conn.last_insert_rowid();
        Ok(User {
            id,
            username: username.to_string(),
            password: password.to_string(),
            is_admin,
        })
    }

    pub fn get_user_by_name(&self, username: &str) -> Option<User> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id, username, password, is_admin FROM users WHERE username = ?1",
            params![username],
            |r| {
                Ok(User {
                    id: r.get(0)?,
                    username: r.get(1)?,
                    password: r.get(2)?,
                    is_admin: r.get::<_, i64>(3)? != 0,
                })
            },
        )
        .optional()
        .ok()
        .flatten()
    }

    pub fn get_user_by_id(&self, id: i64) -> Option<User> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id, username, password, is_admin FROM users WHERE id = ?1",
            params![id],
            |r| {
                Ok(User {
                    id: r.get(0)?,
                    username: r.get(1)?,
                    password: r.get(2)?,
                    is_admin: r.get::<_, i64>(3)? != 0,
                })
            },
        )
        .optional()
        .ok()
        .flatten()
    }

    pub fn list_users(&self) -> Vec<User> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = match conn
            .prepare("SELECT id, username, password, is_admin FROM users ORDER BY id ASC")
        {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        stmt.query_map([], |r| {
            Ok(User {
                id: r.get(0)?,
                username: r.get(1)?,
                password: r.get(2)?,
                is_admin: r.get::<_, i64>(3)? != 0,
            })
        })
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|r| r.ok())
        .collect()
    }

    pub fn delete_user(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM users WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn auth_subsonic(&self, username: &str, password: &str, token: &str, salt: &str) -> Option<User> {
        let user = self.get_user_by_name(username)?;
        if !token.is_empty() && !salt.is_empty() {
            let mut hasher = Md5::new();
            hasher.update(format!("{}{}", user.password, salt).as_bytes());
            let expected = hex::encode(hasher.finalize());
            if expected.eq_ignore_ascii_case(token) {
                return Some(user);
            }
            return None;
        }
        if !password.is_empty() {
            let decoded = if let Some(hex_part) = password.strip_prefix("enc:") {
                hex::decode(hex_part)
                    .ok()
                    .and_then(|b| String::from_utf8(b).ok())
                    .unwrap_or_default()
            } else {
                password.to_string()
            };
            if decoded == user.password {
                return Some(user);
            }
        }
        None
    }

    // ---------- Tracks ----------

    fn row_to_track(r: &rusqlite::Row<'_>) -> rusqlite::Result<Track> {
        Ok(Track {
            id: r.get(0)?,
            source: r.get(1)?,
            title: r.get(2)?,
            artist: r.get(3)?,
            album: r.get(4)?,
            genre: r.get(5)?,
            moods: r.get(6)?,
            year: r.get(7)?,
            track_no: r.get(8)?,
            duration: r.get(9)?,
            bitrate: r.get(10)?,
            sample_rate: r.get(11)?,
            bit_depth: r.get(12)?,
            format: r.get(13)?,
            size: r.get(14)?,
            stream_url: r.get(15)?,
            cover_url: r.get(16)?,
            meta: r.get(17)?,
            play_count: r.get(18)?,
            last_played: r.get(19)?,
            created: r.get(20)?,
        })
    }

    const TRACK_COLS: &'static str = "id, source, title, artist, album, genre, moods, year, track_no, duration, bitrate, sample_rate, bit_depth, format, size, stream_url, cover_url, meta, play_count, last_played, created";

    pub fn upsert_track(&self, t: &Track) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let now = if t.created > 0 {
            t.created
        } else {
            Utc::now().timestamp()
        };
        conn.execute(
            "INSERT INTO tracks (id, source, title, artist, album, genre, moods, year, track_no, duration, bitrate, sample_rate, bit_depth, format, size, stream_url, cover_url, meta, play_count, last_played, created)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21)
             ON CONFLICT(id) DO UPDATE SET
                title=excluded.title,
                artist=excluded.artist,
                album=CASE WHEN excluded.album != '' THEN excluded.album ELSE tracks.album END,
                genre=CASE WHEN excluded.genre != '' THEN excluded.genre ELSE tracks.genre END,
                moods=CASE WHEN excluded.moods != '' THEN excluded.moods ELSE tracks.moods END,
                year=CASE WHEN excluded.year > 0 THEN excluded.year ELSE tracks.year END,
                track_no=CASE WHEN excluded.track_no > 0 THEN excluded.track_no ELSE tracks.track_no END,
                duration=CASE WHEN excluded.duration > 0 THEN excluded.duration ELSE tracks.duration END,
                bitrate=CASE WHEN excluded.bitrate > 0 THEN excluded.bitrate ELSE tracks.bitrate END,
                sample_rate=CASE WHEN excluded.sample_rate > 0 THEN excluded.sample_rate ELSE tracks.sample_rate END,
                bit_depth=CASE WHEN excluded.bit_depth > 0 THEN excluded.bit_depth ELSE tracks.bit_depth END,
                format=CASE WHEN excluded.format != '' THEN excluded.format ELSE tracks.format END,
                size=CASE WHEN excluded.size > 0 THEN excluded.size ELSE tracks.size END,
                stream_url=excluded.stream_url,
                cover_url=CASE WHEN excluded.cover_url != '' THEN excluded.cover_url ELSE tracks.cover_url END,
                meta=CASE WHEN excluded.meta != '' THEN excluded.meta ELSE tracks.meta END",
            params![
                t.id, t.source, t.title, t.artist, t.album, t.genre, t.moods, t.year, t.track_no,
                t.duration, t.bitrate, t.sample_rate, t.bit_depth, t.format, t.size,
                t.stream_url, t.cover_url, t.meta, t.play_count, t.last_played, now
            ],
        )?;
        Ok(())
    }

    pub fn upsert_tracks(&self, tracks: &[Track]) {
        for t in tracks {
            let _ = self.upsert_track(t);
        }
    }

    pub fn track(&self, id: &str) -> Option<Track> {
        let conn = self.conn.lock().unwrap();
        let sql = format!("SELECT {} FROM tracks WHERE id = ?1", Self::TRACK_COLS);
        conn.query_row(&sql, params![id], Self::row_to_track)
            .optional()
            .ok()
            .flatten()
    }

    pub fn all_tracks(&self, limit: usize) -> Vec<Track> {
        let conn = self.conn.lock().unwrap();
        let sql = format!(
            "SELECT {} FROM tracks ORDER BY created DESC LIMIT ?1",
            Self::TRACK_COLS
        );
        let mut stmt = match conn.prepare(&sql) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        stmt.query_map(params![limit as i64], Self::row_to_track)
            .ok()
            .into_iter()
            .flatten()
            .filter_map(|r| r.ok())
            .collect()
    }

    pub fn search_tracks(&self, query: &str, limit: usize) -> Vec<Track> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return self.all_tracks(limit);
        }
        let pattern = format!("%{}%", q);
        let conn = self.conn.lock().unwrap();
        let sql = format!(
            "SELECT {} FROM tracks WHERE lower(title) LIKE ?1 OR lower(artist) LIKE ?1 OR lower(album) LIKE ?1 OR lower(genre) LIKE ?1 LIMIT ?2",
            Self::TRACK_COLS
        );
        let mut stmt = match conn.prepare(&sql) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        let mut tracks: Vec<Track> = stmt
            .query_map(params![pattern, (limit * 4).max(100) as i64], Self::row_to_track)
            .ok()
            .into_iter()
            .flatten()
            .filter_map(|r| r.ok())
            .collect();

        // Rank by relevance + quality score (lossless/local wins ties!)
        tracks.sort_by(|a, b| {
            let sa = search_match_score(a, &q) * 10000 + a.quality_score();
            let sb = search_match_score(b, &q) * 10000 + b.quality_score();
            sb.cmp(&sa)
        });
        tracks.truncate(limit);
        tracks
    }

    pub fn tracks_by_genre(&self, genre: &str, limit: usize, offset: usize) -> Vec<Track> {
        let conn = self.conn.lock().unwrap();
        let pattern = format!("%{}%", genre.to_lowercase());
        let sql = format!(
            "SELECT {} FROM tracks WHERE lower(genre) LIKE ?1 ORDER BY play_count DESC, created DESC LIMIT ?2 OFFSET ?3",
            Self::TRACK_COLS
        );
        let mut stmt = match conn.prepare(&sql) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        stmt.query_map(params![pattern, limit as i64, offset as i64], Self::row_to_track)
            .ok()
            .into_iter()
            .flatten()
            .filter_map(|r| r.ok())
            .collect()
    }

    pub fn tracks_by_mood(&self, mood_slug: &str, limit: usize) -> Vec<Track> {
        let conn = self.conn.lock().unwrap();
        let pattern = format!("%{}%", mood_slug.to_lowercase());
        let sql = format!(
            "SELECT {} FROM tracks WHERE lower(moods) LIKE ?1 ORDER BY play_count DESC, created DESC LIMIT ?2",
            Self::TRACK_COLS
        );
        let mut stmt = match conn.prepare(&sql) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        stmt.query_map(params![pattern, limit as i64], Self::row_to_track)
            .ok()
            .into_iter()
            .flatten()
            .filter_map(|r| r.ok())
            .collect()
    }

    pub fn tracks_by_artist(&self, artist: &str, limit: usize) -> Vec<Track> {
        let conn = self.conn.lock().unwrap();
        let sql = format!(
            "SELECT {} FROM tracks WHERE lower(artist) = lower(?1) ORDER BY album ASC, track_no ASC LIMIT ?2",
            Self::TRACK_COLS
        );
        let mut stmt = match conn.prepare(&sql) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        stmt.query_map(params![artist, limit as i64], Self::row_to_track)
            .ok()
            .into_iter()
            .flatten()
            .filter_map(|r| r.ok())
            .collect()
    }

    pub fn track_count(&self) -> usize {
        let conn = self.conn.lock().unwrap();
        conn.query_row("SELECT COUNT(*) FROM tracks", [], |r| r.get::<_, i64>(0))
            .unwrap_or(0) as usize
    }

    pub fn lossless_count(&self) -> usize {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM tracks WHERE lower(format) IN ('flac','alac','wav','aiff','aif')",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0) as usize
    }

    pub fn genres(&self) -> Vec<(String, usize)> {
        let tracks = self.all_tracks(5000);
        let mut counts: HashMap<String, usize> = HashMap::new();
        for t in tracks {
            let g = t.genre.trim();
            if !g.is_empty() {
                *counts.entry(g.to_string()).or_default() += 1;
            }
        }
        let mut list: Vec<(String, usize)> = counts.into_iter().collect();
        list.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        list
    }

    pub fn artists(&self) -> Vec<(String, String, usize)> {
        let tracks = self.all_tracks(5000);
        let mut map: HashMap<String, (String, usize)> = HashMap::new();
        for t in tracks {
            if t.artist.trim().is_empty() {
                continue;
            }
            let aid = t.artist_id();
            let entry = map.entry(aid).or_insert_with(|| (t.artist.clone(), 0));
            entry.1 += 1;
        }
        let mut out: Vec<(String, String, usize)> =
            map.into_iter().map(|(id, (name, c))| (id, name, c)).collect();
        out.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()));
        out
    }

    pub fn artist_name_from_id(&self, artist_id: &str) -> Option<String> {
        let target = artist_id.strip_prefix("ar:").unwrap_or(artist_id);
        let tracks = self.all_tracks(5000);
        tracks
            .into_iter()
            .find(|t| slug(&t.artist) == target)
            .map(|t| t.artist)
    }

    pub fn increment_play(&self, track_id: &str) {
        let conn = self.conn.lock().unwrap();
        let now = Utc::now().timestamp();
        let _ = conn.execute(
            "UPDATE tracks SET play_count = play_count + 1, last_played = ?1 WHERE id = ?2",
            params![now, track_id],
        );
    }

    // ---------- Events, Stars & Ratings ----------

    pub fn record_event(&self, user_id: i64, track_id: &str, kind: &str, value: f64) {
        let conn = self.conn.lock().unwrap();
        let now = Utc::now().timestamp();
        let _ = conn.execute(
            "INSERT INTO events(user_id, track_id, kind, value, at) VALUES(?1,?2,?3,?4,?5)",
            params![user_id, track_id, kind, value, now],
        );
    }

    pub fn user_events(&self, user_id: i64, limit: usize) -> Vec<Event> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = match conn.prepare(
            "SELECT id, user_id, track_id, kind, value, at FROM events WHERE user_id = ?1 ORDER BY at DESC LIMIT ?2",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        stmt.query_map(params![user_id, limit as i64], |r| {
            Ok(Event {
                id: r.get(0)?,
                user_id: r.get(1)?,
                track_id: r.get(2)?,
                kind: r.get(3)?,
                value: r.get(4)?,
                at: r.get(5)?,
            })
        })
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|r| r.ok())
        .collect()
    }

    pub fn star_track(&self, user_id: i64, track_id: &str) {
        let now = Utc::now().timestamp();
        {
            let conn = self.conn.lock().unwrap();
            let _ = conn.execute(
                "INSERT OR REPLACE INTO stars(user_id, track_id, at) VALUES(?1,?2,?3)",
                params![user_id, track_id, now],
            );
        }
        self.record_event(user_id, track_id, "star", 1.0);
    }

    pub fn unstar_track(&self, user_id: i64, track_id: &str) {
        {
            let conn = self.conn.lock().unwrap();
            let _ = conn.execute(
                "DELETE FROM stars WHERE user_id = ?1 AND track_id = ?2",
                params![user_id, track_id],
            );
        }
        self.record_event(user_id, track_id, "unstar", 1.0);
    }

    pub fn is_starred(&self, user_id: i64, track_id: &str) -> bool {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT 1 FROM stars WHERE user_id = ?1 AND track_id = ?2",
            params![user_id, track_id],
            |_| Ok(1),
        )
        .is_ok()
    }

    pub fn starred_tracks(&self, user_id: i64) -> Vec<Track> {
        let conn = self.conn.lock().unwrap();
        let sql = format!(
            "SELECT {} FROM tracks t INNER JOIN stars s ON t.id = s.track_id WHERE s.user_id = ?1 ORDER BY s.at DESC",
            Self::TRACK_COLS
                .split(", ")
                .map(|c| format!("t.{}", c))
                .collect::<Vec<_>>()
                .join(", ")
        );
        let mut stmt = match conn.prepare(&sql) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        stmt.query_map(params![user_id], Self::row_to_track)
            .ok()
            .into_iter()
            .flatten()
            .filter_map(|r| r.ok())
            .collect()
    }

    pub fn set_rating(&self, user_id: i64, track_id: &str, rating: i32) {
        let now = Utc::now().timestamp();
        {
            let conn = self.conn.lock().unwrap();
            if rating <= 0 {
                let _ = conn.execute(
                    "DELETE FROM ratings WHERE user_id = ?1 AND track_id = ?2",
                    params![user_id, track_id],
                );
            } else {
                let _ = conn.execute(
                    "INSERT OR REPLACE INTO ratings(user_id, track_id, rating, at) VALUES(?1,?2,?3,?4)",
                    params![user_id, track_id, rating.clamp(1, 5), now],
                );
            }
        }
        self.record_event(user_id, track_id, "rate", rating as f64);
    }

    pub fn user_rating(&self, user_id: i64, track_id: &str) -> i32 {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT rating FROM ratings WHERE user_id = ?1 AND track_id = ?2",
            params![user_id, track_id],
            |r| r.get(0),
        )
        .unwrap_or(0)
    }

    // ---------- Playlists ----------

    pub fn create_playlist(
        &self,
        user_id: i64,
        name: &str,
        comment: &str,
        public: bool,
        is_auto: bool,
        kind: &str,
        track_ids: &[String],
    ) -> Result<Playlist> {
        let id = if is_auto {
            format!("auto:{}:{}", user_id, slug(kind))
        } else {
            format!("pl:{}", Uuid::new_v4())
        };
        let now = Utc::now().timestamp();
        {
            let conn = self.conn.lock().unwrap();
            conn.execute(
                "INSERT OR REPLACE INTO playlists(id, user_id, name, comment, public, is_auto, kind, created, changed)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![
                    id,
                    user_id,
                    name,
                    comment,
                    if public { 1 } else { 0 },
                    if is_auto { 1 } else { 0 },
                    kind,
                    now,
                    now
                ],
            )?;
        }
        self.set_playlist_tracks(&id, track_ids)?;
        Ok(self.get_playlist(&id).unwrap())
    }

    pub fn set_playlist_tracks(&self, playlist_id: &str, track_ids: &[String]) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM playlist_tracks WHERE playlist_id = ?1",
            params![playlist_id],
        )?;
        for (idx, tid) in track_ids.iter().enumerate() {
            conn.execute(
                "INSERT INTO playlist_tracks(playlist_id, pos, track_id) VALUES(?1,?2,?3)",
                params![playlist_id, idx as i64, tid],
            )?;
        }
        let now = Utc::now().timestamp();
        conn.execute(
            "UPDATE playlists SET changed = ?1 WHERE id = ?2",
            params![now, playlist_id],
        )?;
        Ok(())
    }

    pub fn playlist_tracks(&self, playlist_id: &str) -> Vec<Track> {
        let conn = self.conn.lock().unwrap();
        let sql = format!(
            "SELECT {} FROM tracks t INNER JOIN playlist_tracks pt ON t.id = pt.track_id WHERE pt.playlist_id = ?1 ORDER BY pt.pos ASC",
            Self::TRACK_COLS
                .split(", ")
                .map(|c| format!("t.{}", c))
                .collect::<Vec<_>>()
                .join(", ")
        );
        let mut stmt = match conn.prepare(&sql) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        stmt.query_map(params![playlist_id], Self::row_to_track)
            .ok()
            .into_iter()
            .flatten()
            .filter_map(|r| r.ok())
            .collect()
    }

    pub fn get_playlist(&self, id: &str) -> Option<Playlist> {
        let conn = self.conn.lock().unwrap();
        let mut pl = conn
            .query_row(
                "SELECT id, user_id, name, comment, public, is_auto, kind, created, changed FROM playlists WHERE id = ?1",
                params![id],
                |r| {
                    Ok(Playlist {
                        id: r.get(0)?,
                        user_id: r.get(1)?,
                        name: r.get(2)?,
                        comment: r.get(3)?,
                        public: r.get::<_, i64>(4)? != 0,
                        is_auto: r.get::<_, i64>(5)? != 0,
                        kind: r.get(6)?,
                        created: r.get(7)?,
                        changed: r.get(8)?,
                        track_count: 0,
                        duration: 0,
                        cover_track_id: None,
                    })
                },
            )
            .optional()
            .ok()
            .flatten()?;
        drop(conn);
        let tracks = self.playlist_tracks(&pl.id);
        pl.track_count = tracks.len();
        pl.duration = tracks.iter().map(|t| t.duration).sum();
        pl.cover_track_id = tracks.first().map(|t| t.id.clone());
        Some(pl)
    }

    pub fn list_playlists(&self, user_id: i64) -> Vec<Playlist> {
        let ids: Vec<String> = {
            let conn = self.conn.lock().unwrap();
            let mut stmt = match conn.prepare(
                "SELECT id FROM playlists WHERE user_id = ?1 OR public = 1 ORDER BY is_auto DESC, changed DESC",
            ) {
                Ok(s) => s,
                Err(_) => return vec![],
            };
            stmt.query_map(params![user_id], |r| r.get(0))
                .ok()
                .into_iter()
                .flatten()
                .filter_map(|r| r.ok())
                .collect()
        };
        ids.into_iter().filter_map(|id| self.get_playlist(&id)).collect()
    }

    pub fn update_playlist_meta(&self, id: &str, name: Option<&str>, comment: Option<&str>, public: Option<bool>) {
        let conn = self.conn.lock().unwrap();
        let now = Utc::now().timestamp();
        if let Some(n) = name {
            let _ = conn.execute("UPDATE playlists SET name = ?1, changed = ?2 WHERE id = ?3", params![n, now, id]);
        }
        if let Some(c) = comment {
            let _ = conn.execute("UPDATE playlists SET comment = ?1, changed = ?2 WHERE id = ?3", params![c, now, id]);
        }
        if let Some(p) = public {
            let _ = conn.execute("UPDATE playlists SET public = ?1, changed = ?2 WHERE id = ?3", params![if p {1} else {0}, now, id]);
        }
    }

    pub fn delete_playlist(&self, id: &str) {
        let conn = self.conn.lock().unwrap();
        let _ = conn.execute("DELETE FROM playlist_tracks WHERE playlist_id = ?1", params![id]);
        let _ = conn.execute("DELETE FROM playlists WHERE id = ?1", params![id]);
    }

    // ---------- Radio Stations ----------

    pub fn list_radio_stations(&self) -> Vec<RadioStation> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = match conn.prepare("SELECT id, name, stream_url, home_page_url FROM radio_stations ORDER BY name ASC") {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        stmt.query_map([], |r| {
            Ok(RadioStation {
                id: r.get(0)?,
                name: r.get(1)?,
                stream_url: r.get(2)?,
                home_page_url: r.get(3)?,
            })
        })
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|r| r.ok())
        .collect()
    }

    pub fn upsert_radio_station(&self, id: Option<&str>, name: &str, stream_url: &str, home_page_url: &str) -> RadioStation {
        let sid = id.map(|s| s.to_string()).unwrap_or_else(|| format!("rad:{}", Uuid::new_v4()));
        let conn = self.conn.lock().unwrap();
        let _ = conn.execute(
            "INSERT OR REPLACE INTO radio_stations(id, name, stream_url, home_page_url) VALUES(?1,?2,?3,?4)",
            params![sid, name, stream_url, home_page_url],
        );
        RadioStation {
            id: sid,
            name: name.to_string(),
            stream_url: stream_url.to_string(),
            home_page_url: home_page_url.to_string(),
        }
    }

    pub fn delete_radio_station(&self, id: &str) {
        let conn = self.conn.lock().unwrap();
        let _ = conn.execute("DELETE FROM radio_stations WHERE id = ?1", params![id]);
    }

    // ---------- Play Queue ----------

    pub fn save_play_queue(&self, user_id: i64, current_id: &str, position_ms: i64, track_ids: &[String], changed_by: &str) {
        let conn = self.conn.lock().unwrap();
        let now = Utc::now().timestamp();
        let json = serde_json::to_string(track_ids).unwrap_or_else(|_| "[]".into());
        let _ = conn.execute(
            "INSERT OR REPLACE INTO play_queues(user_id, current_id, position_ms, track_ids, changed_by, updated) VALUES(?1,?2,?3,?4,?5,?6)",
            params![user_id, current_id, position_ms, json, changed_by, now],
        );
    }

    pub fn get_play_queue(&self, user_id: i64) -> Option<(String, i64, Vec<String>, String, i64)> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT current_id, position_ms, track_ids, changed_by, updated FROM play_queues WHERE user_id = ?1",
            params![user_id],
            |r| {
                let ids_json: String = r.get(2)?;
                let ids: Vec<String> = serde_json::from_str(&ids_json).unwrap_or_default();
                Ok((r.get(0)?, r.get(1)?, ids, r.get(3)?, r.get(4)?))
            },
        )
        .optional()
        .ok()
        .flatten()
    }

    // ---------- Cache Index ----------

    pub fn cache_get(&self, track_id: &str) -> Option<CacheEntry> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT track_id, path, bytes, last_hit FROM cache_entries WHERE track_id = ?1",
            params![track_id],
            |r| {
                Ok(CacheEntry {
                    track_id: r.get(0)?,
                    path: r.get(1)?,
                    bytes: r.get(2)?,
                    last_hit: r.get(3)?,
                })
            },
        )
        .optional()
        .ok()
        .flatten()
    }

    pub fn cache_put(&self, track_id: &str, path: &str, bytes: i64) {
        let conn = self.conn.lock().unwrap();
        let now = Utc::now().timestamp();
        let _ = conn.execute(
            "INSERT OR REPLACE INTO cache_entries(track_id, path, bytes, last_hit) VALUES(?1,?2,?3,?4)",
            params![track_id, path, bytes, now],
        );
    }

    pub fn cache_touch(&self, track_id: &str) {
        let conn = self.conn.lock().unwrap();
        let now = Utc::now().timestamp();
        let _ = conn.execute(
            "UPDATE cache_entries SET last_hit = ?1 WHERE track_id = ?2",
            params![now, track_id],
        );
    }

    pub fn cache_delete(&self, track_id: &str) -> bool {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM cache_entries WHERE track_id = ?1", params![track_id])
            .map(|n| n > 0)
            .unwrap_or(false)
    }

    pub fn cache_bytes(&self) -> i64 {
        let conn = self.conn.lock().unwrap();
        conn.query_row("SELECT COALESCE(SUM(bytes), 0) FROM cache_entries", [], |r| r.get(0))
            .unwrap_or(0)
    }

    pub fn cache_entries_oldest(&self) -> Vec<CacheEntry> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = match conn.prepare("SELECT track_id, path, bytes, last_hit FROM cache_entries ORDER BY last_hit ASC") {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        stmt.query_map([], |r| {
            Ok(CacheEntry {
                track_id: r.get(0)?,
                path: r.get(1)?,
                bytes: r.get(2)?,
                last_hit: r.get(3)?,
            })
        })
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|r| r.ok())
        .collect()
    }

    // ---------- KV ----------

    pub fn kv_get(&self, k: &str) -> Option<String> {
        let conn = self.conn.lock().unwrap();
        conn.query_row("SELECT v FROM kv WHERE k = ?1", params![k], |r| r.get(0))
            .optional()
            .ok()
            .flatten()
    }

    pub fn kv_set(&self, k: &str, v: &str) {
        let conn = self.conn.lock().unwrap();
        let _ = conn.execute(
            "INSERT OR REPLACE INTO kv(k, v) VALUES(?1,?2)",
            params![k, v],
        );
    }
}

fn search_match_score(t: &Track, q: &str) -> i32 {
    let title = t.title.to_lowercase();
    let artist = t.artist.to_lowercase();
    let album = t.album.to_lowercase();
    let mut score = 0;
    if title == q {
        score += 100;
    } else if title.starts_with(q) {
        score += 60;
    } else if title.contains(q) {
        score += 35;
    }
    if artist == q {
        score += 90;
    } else if artist.starts_with(q) {
        score += 55;
    } else if artist.contains(q) {
        score += 30;
    }
    if album.contains(q) {
        score += 15;
    }
    score
}
