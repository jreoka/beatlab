use crate::auth::{hash_recovery_code, RESTORE_WINDOW_SECS};
use crate::models::{
    constant_time_eq, err_msg, primary_artist, slug, validate_credentials, AlbumSummary, AppResult,
    ArtistSummary, CacheEntry, Event, Mood, PlayQueueState, Playlist, PublicUser, RadioStation,
    ShareRecord, Track, User, UserEventRow, UserProfile, UserTrackStats,
};
use chrono::Utc;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension, Row, ToSql};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// Sessions stay signed in until the user revokes or signs them out. The
/// `expires` column is kept for schema compatibility and set to a far-future
/// sentinel; it is never consulted.
const SESSION_NO_EXPIRY: i64 = 4_102_444_800; // 2100-01-01 UTC

const TRACK_COLS: &str = "id, source, title, artist, album, genre, moods, year, track_no, disc_no, \
    duration, bitrate, sample_rate, bit_depth, format, size, stream_url, cover_url, meta, deezer_id, created";

fn prefixed_track_cols(alias: &str) -> String {
    TRACK_COLS
        .split(',')
        .map(|c| format!("{}.{}", alias, c.trim()))
        .collect::<Vec<_>>()
        .join(", ")
}

#[derive(Debug, Default, Clone)]
pub struct NeuralTrainingCorpus {
    pub user_ids: Vec<i64>,
    pub user_events: HashMap<i64, Vec<(String, String, f64, i64)>>,
    pub user_stars: HashMap<i64, Vec<String>>,
    pub user_ratings: HashMap<i64, Vec<(String, i32)>>,
    pub user_disliked: HashMap<i64, Vec<String>>,
    pub playlists: Vec<(i64, String, Vec<String>)>,
    pub catalog_ids: Vec<String>,
    pub tracks: HashMap<String, Track>,
}

#[derive(Clone)]
pub struct Store {
    conn: Arc<Mutex<Connection>>,
}

/// A lightweight view of a persisted login session, safe to return to the
/// browser. `token` doubles as the session id used by the revoke endpoints.
#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub token: String,
    pub kind: String,
    pub created: i64,
    pub last_seen: i64,
    pub user_agent: String,
    pub ip: String,
    pub client: String,
    pub revoked: bool,
}

/// Context captured when a session is created, used to describe it later in
/// the settings "Sessions" tab.
#[derive(Debug, Clone, Default)]
pub struct SessionMeta {
    pub user_agent: String,
    pub ip: String,
    pub client: String,
}

/// Server-side state of a user's TOTP enrolment.
#[derive(Debug, Clone)]
pub struct TotpState {
    pub secret: String,
    pub enabled: bool,
    pub created: i64,
    pub last_step: i64,
}

/// A registered passkey, safe to show in settings (never exposes key material).
#[derive(Debug, Clone)]
pub struct PasskeyInfo {
    pub credential_id: String,
    pub label: String,
    pub created: i64,
    pub last_used: i64,
}

/// Admin-facing summary of one account (no credentials exposed).
#[derive(Debug, Clone)]
pub struct AdminUserSummary {
    pub id: i64,
    pub username: String,
    pub created: i64,
    pub deleted_at: i64,
    pub totp_enabled: bool,
    pub passkeys: usize,
    pub recovery_left: usize,
    pub sessions: usize,
    pub playlists: usize,
    pub stars: usize,
    pub events: usize,
    pub listens: usize,
    pub last_seen: i64,
}

#[derive(Debug, Clone, Default)]
pub struct AdminOverview {
    pub total_users: usize,
    pub active_users: usize,
    pub deleted_users: usize,
    pub sessions: usize,
    pub client_sessions: usize,
    pub tracks: usize,
    pub passkeys: usize,
    pub totp_users: usize,
    pub recovery_codes: usize,
    pub playlists: usize,
    pub security_events: usize,
}

/// A security event with the account it belongs to (global admin feed).
#[derive(Debug, Clone)]
pub struct AdminEvent {
    pub user_id: i64,
    pub username: String,
    pub kind: String,
    pub detail: String,
    pub ip: String,
    pub created: i64,
}

/// One line in the account's security audit trail.
#[derive(Debug, Clone)]
pub struct SecurityEvent {
    pub id: i64,
    pub kind: String,
    pub detail: String,
    pub ip: String,
    pub created: i64,
}

impl Store {
    pub fn open(path: &Path) -> AppResult<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path).map_err(|e| err_msg(format!("open sqlite database: {}", e)))?;
        configure_conn(&conn)?;
        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        store.init_schema()?;
        Ok(store)
    }

    #[cfg(test)]
    pub fn open_memory() -> AppResult<Self> {
        let conn = Connection::open_in_memory()?;
        configure_conn(&conn)?;
        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        store.init_schema()?;
        Ok(store)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn init_schema(&self) -> AppResult<()> {
        let mut conn = self.lock();
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS users (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                username TEXT NOT NULL,
                password TEXT NOT NULL,
                created INTEGER NOT NULL
            );
            CREATE UNIQUE INDEX IF NOT EXISTS idx_users_username_lower
                ON users(lower(username));

            CREATE TABLE IF NOT EXISTS sessions (
                token TEXT PRIMARY KEY,
                user_id INTEGER NOT NULL,
                kind TEXT NOT NULL DEFAULT 'web',
                created INTEGER NOT NULL,
                expires INTEGER NOT NULL,
                FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_sessions_user ON sessions(user_id);

            CREATE TABLE IF NOT EXISTS api_keys (
                key TEXT PRIMARY KEY,
                user_id INTEGER NOT NULL,
                label TEXT NOT NULL DEFAULT 'default',
                created INTEGER NOT NULL,
                FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_api_keys_user ON api_keys(user_id);

            -- Two-factor authentication (TOTP secret + single-use recovery codes)
            CREATE TABLE IF NOT EXISTS totp_secrets (
                user_id INTEGER PRIMARY KEY,
                secret TEXT NOT NULL,
                enabled INTEGER NOT NULL DEFAULT 0,
                created INTEGER NOT NULL,
                last_step INTEGER NOT NULL DEFAULT 0,
                FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE
            );
            CREATE TABLE IF NOT EXISTS recovery_codes (
                code_hash TEXT PRIMARY KEY,
                user_id INTEGER NOT NULL,
                used INTEGER NOT NULL DEFAULT 0,
                created INTEGER NOT NULL,
                FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_recovery_codes_user ON recovery_codes(user_id);

            -- WebAuthn passkeys and the in-flight challenges that pair each
            -- browser ceremony with its server-side state.
            CREATE TABLE IF NOT EXISTS passkeys (
                credential_id TEXT PRIMARY KEY,
                user_id INTEGER NOT NULL,
                label TEXT NOT NULL DEFAULT 'Passkey',
                passkey_json TEXT NOT NULL,
                created INTEGER NOT NULL,
                last_used INTEGER NOT NULL DEFAULT 0,
                FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_passkeys_user ON passkeys(user_id);
            CREATE TABLE IF NOT EXISTS auth_challenges (
                id TEXT PRIMARY KEY,
                user_id INTEGER NOT NULL DEFAULT 0,
                kind TEXT NOT NULL,
                payload TEXT NOT NULL,
                created INTEGER NOT NULL,
                FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_auth_challenges_created ON auth_challenges(created);

            -- Security audit trail and soft-deletion / 7-day account recovery.
            CREATE TABLE IF NOT EXISTS security_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                user_id INTEGER NOT NULL,
                kind TEXT NOT NULL,
                detail TEXT NOT NULL DEFAULT '',
                ip TEXT NOT NULL DEFAULT '',
                created INTEGER NOT NULL,
                FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_security_events_user ON security_events(user_id, created DESC);

            CREATE TABLE IF NOT EXISTS tracks (
                id TEXT PRIMARY KEY,
                source TEXT NOT NULL,
                title TEXT NOT NULL,
                artist TEXT NOT NULL,
                album TEXT NOT NULL,
                genre TEXT NOT NULL DEFAULT '',
                moods TEXT NOT NULL DEFAULT '',
                year INTEGER NOT NULL DEFAULT 0,
                track_no INTEGER NOT NULL DEFAULT 0,
                disc_no INTEGER NOT NULL DEFAULT 0,
                duration INTEGER NOT NULL DEFAULT 0,
                bitrate INTEGER NOT NULL DEFAULT 0,
                sample_rate INTEGER NOT NULL DEFAULT 0,
                bit_depth INTEGER NOT NULL DEFAULT 0,
                format TEXT NOT NULL DEFAULT '',
                size INTEGER NOT NULL DEFAULT 0,
                stream_url TEXT NOT NULL,
                cover_url TEXT NOT NULL DEFAULT '',
                meta TEXT NOT NULL DEFAULT '',
                deezer_id TEXT NOT NULL DEFAULT '',
                created INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_tracks_source ON tracks(source);
            CREATE INDEX IF NOT EXISTS idx_tracks_artist ON tracks(artist);
            CREATE INDEX IF NOT EXISTS idx_tracks_album ON tracks(album);
            CREATE INDEX IF NOT EXISTS idx_tracks_genre ON tracks(genre);
            CREATE INDEX IF NOT EXISTS idx_tracks_created ON tracks(created DESC);

            CREATE TABLE IF NOT EXISTS albums_meta (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                artist TEXT NOT NULL,
                artist_id TEXT NOT NULL DEFAULT '',
                cover_url TEXT NOT NULL DEFAULT '',
                genre TEXT NOT NULL DEFAULT '',
                year INTEGER NOT NULL DEFAULT 0,
                label TEXT NOT NULL DEFAULT '',
                deezer_id TEXT NOT NULL DEFAULT '',
                updated INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS artists_meta (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                cover_url TEXT NOT NULL DEFAULT '',
                bio TEXT NOT NULL DEFAULT '',
                deezer_id TEXT NOT NULL DEFAULT '',
                updated INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS user_track_stats (
                user_id INTEGER NOT NULL,
                track_id TEXT NOT NULL,
                play_count INTEGER NOT NULL DEFAULT 0,
                skip_count INTEGER NOT NULL DEFAULT 0,
                last_played INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY(user_id, track_id)
            );
            CREATE INDEX IF NOT EXISTS idx_uts_recent ON user_track_stats(user_id, last_played DESC);
            CREATE INDEX IF NOT EXISTS idx_uts_freq ON user_track_stats(user_id, play_count DESC);

            CREATE TABLE IF NOT EXISTS events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                user_id INTEGER NOT NULL,
                track_id TEXT NOT NULL,
                kind TEXT NOT NULL,
                value REAL NOT NULL DEFAULT 1,
                at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_events_user_at ON events(user_id, at DESC);

            CREATE TABLE IF NOT EXISTS stars (
                user_id INTEGER NOT NULL,
                track_id TEXT NOT NULL,
                at INTEGER NOT NULL,
                PRIMARY KEY(user_id, track_id)
            );

            CREATE TABLE IF NOT EXISTS saved_albums (
                user_id INTEGER NOT NULL,
                album_id TEXT NOT NULL,
                at INTEGER NOT NULL,
                PRIMARY KEY(user_id, album_id)
            );

            CREATE TABLE IF NOT EXISTS followed_artists (
                user_id INTEGER NOT NULL,
                artist_id TEXT NOT NULL,
                at INTEGER NOT NULL,
                PRIMARY KEY(user_id, artist_id)
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
                public INTEGER NOT NULL DEFAULT 0,
                is_auto INTEGER NOT NULL DEFAULT 0,
                kind TEXT NOT NULL DEFAULT 'user',
                created INTEGER NOT NULL,
                changed INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_playlists_user ON playlists(user_id, changed DESC);

            CREATE TABLE IF NOT EXISTS playlist_tracks (
                playlist_id TEXT NOT NULL,
                pos INTEGER NOT NULL,
                track_id TEXT NOT NULL,
                added INTEGER NOT NULL,
                PRIMARY KEY(playlist_id, pos)
            );

            CREATE TABLE IF NOT EXISTS play_queues (
                user_id INTEGER PRIMARY KEY,
                current_id TEXT NOT NULL DEFAULT '',
                current_index INTEGER NOT NULL DEFAULT 0,
                position_ms INTEGER NOT NULL DEFAULT 0,
                track_ids TEXT NOT NULL DEFAULT '[]',
                changed_by TEXT NOT NULL DEFAULT '',
                updated INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS radio_stations (
                id TEXT PRIMARY KEY,
                user_id INTEGER NOT NULL DEFAULT 0,
                name TEXT NOT NULL,
                stream_url TEXT NOT NULL,
                home_page_url TEXT NOT NULL DEFAULT ''
            );

            CREATE TABLE IF NOT EXISTS cache_entries (
                track_id TEXT PRIMARY KEY,
                path TEXT NOT NULL,
                bytes INTEGER NOT NULL,
                last_accessed INTEGER NOT NULL
            );

            -- Registry of objects uploaded to the optional S3/R2 cool cache.
            -- `kind` namespaces audio / covers / embeds / avatars so every cached
            -- asset can live in one durable tier. last_accessed is the recency
            -- signal for eviction: the cool tier keeps everything until it is
            -- nearly full, then drops the least recently used asset rather than
            -- the least recently uploaded one.
            CREATE TABLE IF NOT EXISTS remote_cache_entries (
                track_id TEXT PRIMARY KEY,
                kind TEXT NOT NULL DEFAULT 'audio',
                object_key TEXT NOT NULL,
                bytes INTEGER NOT NULL,
                created INTEGER NOT NULL,
                last_accessed INTEGER NOT NULL DEFAULT 0
            );
            -- The recency index is created after migrations add its columns.
            CREATE INDEX IF NOT EXISTS idx_remote_cache_created
                ON remote_cache_entries(created ASC);

            CREATE TABLE IF NOT EXISTS kv (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS user_taste_links (
                user_id INTEGER NOT NULL,
                kind TEXT NOT NULL,
                key TEXT NOT NULL,
                weight REAL NOT NULL DEFAULT 1.0,
                updated INTEGER NOT NULL,
                PRIMARY KEY(user_id, kind, key)
            );
            CREATE INDEX IF NOT EXISTS idx_taste_links_user
                ON user_taste_links(user_id, kind, updated DESC);

            CREATE TABLE IF NOT EXISTS hidden_tracks (
                user_id INTEGER NOT NULL,
                track_id TEXT NOT NULL,
                song_key TEXT NOT NULL DEFAULT '',
                at INTEGER NOT NULL,
                PRIMARY KEY(user_id, track_id)
            );
            CREATE INDEX IF NOT EXISTS idx_hidden_tracks_user
                ON hidden_tracks(user_id, at DESC);

            CREATE TABLE IF NOT EXISTS shares (
                id TEXT PRIMARY KEY,
                user_id INTEGER NOT NULL,
                kind TEXT NOT NULL,
                target_id TEXT NOT NULL,
                title TEXT NOT NULL DEFAULT '',
                subtitle TEXT NOT NULL DEFAULT '',
                track_ids TEXT NOT NULL DEFAULT '[]',
                created INTEGER NOT NULL,
                FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_shares_user_kind_target
                ON shares(user_id, kind, target_id);

            CREATE TABLE IF NOT EXISTS chart_catalog (
                track_id TEXT PRIMARY KEY,
                rank_pos INTEGER NOT NULL DEFAULT 0,
                updated INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_chart_catalog_rank
                ON chart_catalog(rank_pos ASC, updated DESC);

            -- Social layer: public profiles, following graph and auto-updating user radio.
            CREATE TABLE IF NOT EXISTS user_profiles (
                user_id INTEGER PRIMARY KEY,
                display_name TEXT NOT NULL DEFAULT '',
                bio TEXT NOT NULL DEFAULT '',
                avatar_color TEXT NOT NULL DEFAULT '',
                location TEXT NOT NULL DEFAULT '',
                updated INTEGER NOT NULL,
                avatar BLOB,
                avatar_mime TEXT NOT NULL DEFAULT '',
                avatar_version INTEGER NOT NULL DEFAULT 0,
                FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS follows (
                follower_id INTEGER NOT NULL,
                following_id INTEGER NOT NULL,
                created INTEGER NOT NULL,
                PRIMARY KEY(follower_id, following_id),
                FOREIGN KEY(follower_id) REFERENCES users(id) ON DELETE CASCADE,
                FOREIGN KEY(following_id) REFERENCES users(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_follows_following
                ON follows(following_id, created DESC);
            CREATE INDEX IF NOT EXISTS idx_follows_follower
                ON follows(follower_id, created DESC);

            -- A listener's saved radios: a playlist in the owner's library that
            -- auto-refreshes from the target user's listening taste.
            CREATE TABLE IF NOT EXISTS user_radios (
                owner_id INTEGER NOT NULL,
                target_id INTEGER NOT NULL,
                playlist_id TEXT NOT NULL,
                created INTEGER NOT NULL,
                PRIMARY KEY(owner_id, target_id),
                FOREIGN KEY(owner_id) REFERENCES users(id) ON DELETE CASCADE,
                FOREIGN KEY(target_id) REFERENCES users(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_user_radios_target
                ON user_radios(target_id);
            ",
        )?;

        // Forward-compatible column additions when upgrading an existing database.
        ensure_column(&conn, "tracks", "disc_no", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_column(&conn, "tracks", "deezer_id", "TEXT NOT NULL DEFAULT ''")?;
        ensure_column(&conn, "tracks", "moods", "TEXT NOT NULL DEFAULT ''")?;
        ensure_column(&conn, "tracks", "sample_rate", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_column(&conn, "tracks", "bit_depth", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_column(&conn, "playlists", "is_auto", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_column(&conn, "playlists", "kind", "TEXT NOT NULL DEFAULT 'user'")?;
        ensure_column(
            &conn,
            "play_queues",
            "current_index",
            "INTEGER NOT NULL DEFAULT 0",
        )?;
        ensure_column(&conn, "radio_stations", "user_id", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_column(&conn, "albums_meta", "artist_id", "TEXT NOT NULL DEFAULT ''")?;
        ensure_column(&conn, "user_profiles", "avatar", "BLOB")?;
        ensure_column(&conn, "user_profiles", "avatar_mime", "TEXT NOT NULL DEFAULT ''")?;
        ensure_column(&conn, "user_profiles", "avatar_version", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_column(&conn, "sessions", "user_agent", "TEXT NOT NULL DEFAULT ''")?;
        ensure_column(&conn, "sessions", "ip", "TEXT NOT NULL DEFAULT ''")?;
        ensure_column(&conn, "sessions", "client", "TEXT NOT NULL DEFAULT ''")?;
        ensure_column(&conn, "sessions", "last_seen", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_column(&conn, "sessions", "revoked", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_column(&conn, "sessions", "mfa_at", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_column(&conn, "users", "deleted_at", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_column(&conn, "users", "restore_hash", "TEXT NOT NULL DEFAULT ''")?;
        // Cool cache: namespace entries by asset type and track real usage so
        // eviction only happens once the bucket is nearly full.
        ensure_column(
            &conn,
            "remote_cache_entries",
            "kind",
            "TEXT NOT NULL DEFAULT 'audio'",
        )?;
        ensure_column(
            &conn,
            "remote_cache_entries",
            "last_accessed",
            "INTEGER NOT NULL DEFAULT 0",
        )?;
        conn.execute(
            "UPDATE remote_cache_entries SET last_accessed = created WHERE last_accessed = 0",
            [],
        )?;
        let _ = conn.execute("DROP INDEX IF EXISTS idx_remote_cache_created", []);
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_remote_cache_recency
             ON remote_cache_entries(kind ASC, last_accessed ASC)",
            [],
        )?;
        // Sessions used to expire after 30 days; normalise old rows so the
        // stored value matches the now-persistent behaviour.
        conn.execute(
            "UPDATE sessions SET expires = ?1 WHERE expires < ?1",
            params![SESSION_NO_EXPIRY],
        )?;
        // Older builds indexed client sessions by (client, user-agent); drop
        // that so a client name maps to a single session even when its app
        // version (and therefore User-Agent) changes.
        conn.execute("DROP INDEX IF EXISTS idx_sessions_client", [])?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_tracks_deezer ON tracks(deezer_id)",
            [],
        )?;
        let _ = conn.execute("DELETE FROM kv WHERE key LIKE 'nn:user:%'", []);

        migrate_legacy_moods(&mut conn)?;
        migrate_artist_disambiguation(&mut conn)?;
        migrate_artist_disambiguation_v2(&mut conn)?;

        Ok(())
    }

    // -------------------------------------------------------------------------
    // Users, Open Auto-Signup, Sessions & API Keys (zero admin concepts)
    // -------------------------------------------------------------------------

    pub fn create_user(&self, username: &str, password: &str) -> AppResult<User> {
        let uname = validate_credentials(username, password).map_err(|e| err_msg(e))?;
        let conn = self.lock();
        let existing: Option<i64> = conn
            .query_row(
                "SELECT id FROM users WHERE lower(username) = lower(?1)",
                params![uname],
                |r| r.get(0),
            )
            .optional()?;
        if existing.is_some() {
            return Err(err_msg(format!("Username '{}' is already taken", uname)));
        }
        let now = Utc::now().timestamp();
        conn.execute(
            "INSERT INTO users(username, password, created) VALUES (?1, ?2, ?3)",
            params![uname, password, now],
        )?;
        let id = conn.last_insert_rowid();
        Ok(User {
            id,
            username: uname,
            password: password.to_string(),
            created: now,
        })
    }

    /// Strict credential check used by the web sign-in form: never creates an
    /// account. Unknown usernames must go through the sign-up flow instead.
    pub fn authenticate_user(&self, username: &str, password: &str) -> AppResult<User> {
        let uname = validate_credentials(username, password).map_err(err_msg)?;
        let conn = self.lock();
        let existing: Option<(User, i64)> = conn
            .query_row(
                "SELECT id, username, password, created, deleted_at FROM users WHERE lower(username) = lower(?1)",
                params![uname],
                |r| Ok((row_to_user(r)?, r.get::<_, i64>(4)?)),
            )
            .optional()?;
        match existing {
            Some((user, deleted_at)) if constant_time_eq(&user.password, password) => {
                if deleted_at > 0 {
                    Err(err_msg(crate::models::ERR_RESTORE_REQUIRED))
                } else {
                    Ok(user)
                }
            }
            _ => Err(err_msg("Invalid username or password")),
        }
    }

    /// Test convenience helper: authenticate or create a user in one call.
    /// Production sign-in must use the strict `authenticate_user` instead.
    #[cfg(test)]
    pub fn authenticate_or_create_user(
        &self,
        username: &str,
        password: &str,
    ) -> AppResult<(User, bool)> {
        let uname = validate_credentials(username, password).map_err(|e| err_msg(e))?;
        let conn = self.lock();
        let existing: Option<(User, i64)> = conn
            .query_row(
                "SELECT id, username, password, created, deleted_at FROM users WHERE lower(username) = lower(?1)",
                params![uname],
                |r| Ok((row_to_user(r)?, r.get::<_, i64>(4)?)),
            )
            .optional()?;
        if let Some((u, deleted_at)) = existing {
            if !constant_time_eq(&u.password, password) {
                return Err(err_msg("Invalid username or password"));
            }
            if deleted_at > 0 {
                return Err(err_msg(crate::models::ERR_RESTORE_REQUIRED));
            }
            return Ok((u, false));
        }

        let now = Utc::now().timestamp();
        match conn.execute(
            "INSERT INTO users(username, password, created) VALUES (?1, ?2, ?3)",
            params![uname, password, now],
        ) {
            Ok(_) => {
                let id = conn.last_insert_rowid();
                Ok((
                    User {
                        id,
                        username: uname,
                        password: password.to_string(),
                        created: now,
                    },
                    true,
                ))
            }
            Err(_) => {
                // Concurrent signup race on same username: verify against inserted row.
                let u = conn
                    .query_row(
                        "SELECT id, username, password, created FROM users WHERE lower(username) = lower(?1)",
                        params![uname],
                        row_to_user,
                    )
                    .optional()?
                    .ok_or_else(|| err_msg("Failed to create user"))?;
                if constant_time_eq(&u.password, password) {
                    Ok((u, false))
                } else {
                    Err(err_msg("Invalid username or password"))
                }
            }
        }
    }

    pub fn get_user_by_name(&self, name: &str) -> AppResult<Option<User>> {
        let conn = self.lock();
        Ok(conn
            .query_row(
                "SELECT id, username, password, created FROM users WHERE lower(username) = lower(?1)",
                params![name.trim()],
                row_to_user,
            )
            .optional()?)
    }

    pub fn get_user_by_id(&self, id: i64) -> AppResult<Option<User>> {
        let conn = self.lock();
        Ok(conn
            .query_row(
                "SELECT id, username, password, created FROM users WHERE id = ?1 AND deleted_at = 0",
                params![id],
                row_to_user,
            )
            .optional()?)
    }

    pub fn change_password(&self, user_id: i64, new_password: &str) -> AppResult<()> {
        if new_password.is_empty() || new_password.len() > 256 {
            return Err(err_msg("Password must be 1 to 256 characters"));
        }
        let conn = self.lock();
        conn.execute(
            "UPDATE users SET password = ?1 WHERE id = ?2",
            params![new_password, user_id],
        )?;
        Ok(())
    }

    pub fn all_user_ids(&self) -> AppResult<Vec<i64>> {
        let conn = self.lock();
        let mut stmt = conn.prepare("SELECT id FROM users ORDER BY id ASC")?;
        let rows = stmt
            .query_map([], |r| r.get::<_, i64>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Convenience wrapper used by tests; production callers pass full metadata.
    #[cfg(test)]
    pub fn create_session(&self, user_id: i64, kind: &str) -> AppResult<String> {
        self.create_session_with_meta(user_id, kind, &SessionMeta::default())
    }

    /// Create a session while recording where/how it was opened so the
    /// settings "Sessions" tab can describe it to the user. Sessions never
    /// expire on their own — sign-out or revocation is the only way to end one.
    pub fn create_session_with_meta(
        &self,
        user_id: i64,
        kind: &str,
        meta: &SessionMeta,
    ) -> AppResult<String> {
        let token = format!(
            "{}{}",
            Uuid::new_v4().simple(),
            Uuid::new_v4().simple()
        );
        let now = Utc::now().timestamp();
        let conn = self.lock();
        conn.execute(
            "INSERT INTO sessions(token, user_id, kind, created, expires, last_seen, user_agent, ip, client, revoked)
             VALUES (?1, ?2, ?3, ?4, ?5, ?4, ?6, ?7, ?8, 0)",
            params![token, user_id, kind, now, SESSION_NO_EXPIRY, meta.user_agent, meta.ip, meta.client],
        )?;
        Ok(token)
    }

    pub fn get_session_user(&self, token: &str) -> AppResult<Option<User>> {
        if token.trim().is_empty() {
            return Ok(None);
        }
        let conn = self.lock();
        let user = conn
            .query_row(
                "SELECT u.id, u.username, u.password, u.created
                 FROM sessions s JOIN users u ON u.id = s.user_id
                 WHERE s.token = ?1 AND s.revoked = 0 AND u.deleted_at = 0",
                params![token.trim()],
                row_to_user,
            )
            .optional()?;
        Ok(user)
    }

    pub fn delete_session(&self, token: &str) -> AppResult<()> {
        let conn = self.lock();
        conn.execute("DELETE FROM sessions WHERE token = ?1", params![token])?;
        Ok(())
    }

    /// Find and refresh a Subsonic client's session row, creating one on first
    /// sight. Returns its token and whether it has been revoked by the user.
    /// These clients re-send credentials on every request instead of using a
    /// server-issued token, so the row is keyed by the client identity.
    pub fn touch_subsonic_client(
        &self,
        user_id: i64,
        client: &str,
        user_agent: &str,
        ip: &str,
    ) -> AppResult<(String, bool)> {
        let now = Utc::now().timestamp();
        let conn = self.lock();
        // Keyed on client name only: a client's User-Agent changes with every
        // app update, and revocation must survive that.
        let existing: Option<(String, i64, i64, String, String)> = conn
            .query_row(
                "SELECT token, revoked, last_seen, ip, user_agent FROM sessions
                 WHERE user_id = ?1 AND client = ?2 AND kind = 'subsonic'
                 ORDER BY created DESC LIMIT 1",
                params![user_id, client],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?;
        if let Some((token, revoked, last_seen, saved_ip, saved_ua)) = existing {
            // Subsonic clients re-authenticate on every request; throttle the
            // write to once a minute (or when the connection details change)
            // to avoid a DB write per media chunk.
            if now - last_seen >= 60 || saved_ip != ip || saved_ua != user_agent {
                if revoked == 0 {
                    conn.execute(
                        "UPDATE sessions SET last_seen = ?1, ip = ?2, user_agent = ?3
                         WHERE token = ?4",
                        params![now, ip, user_agent, token],
                    )?;
                } else {
                    conn.execute(
                        "UPDATE sessions SET last_seen = ?1, ip = ?2, user_agent = ?3 WHERE token = ?4",
                        params![now, ip, user_agent, token],
                    )?;
                }
            }
            return Ok((token, revoked != 0));
        }
        let token = format!(
            "{}{}",
            Uuid::new_v4().simple(),
            Uuid::new_v4().simple()
        );
        conn.execute(
            "INSERT INTO sessions(token, user_id, kind, created, expires, last_seen, user_agent, ip, client, revoked)
             VALUES (?1, ?2, 'subsonic', ?3, ?4, ?3, ?5, ?6, ?7, 0)",
            params![token, user_id, now, SESSION_NO_EXPIRY, user_agent, ip, client],
        )?;
        Ok((token, false))
    }

    /// List the sessions belonging to a user, newest first. Sessions persist
    /// until revoked or signed out, so nothing is pruned here.
    pub fn list_sessions(&self, user_id: i64) -> AppResult<Vec<SessionInfo>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT token, kind, created, last_seen, user_agent, ip, client, revoked
             FROM sessions WHERE user_id = ?1
             ORDER BY revoked ASC, created DESC",
        )?;
        let rows = stmt
            .query_map(params![user_id], |r| {
                Ok(SessionInfo {
                    token: r.get(0)?,
                    kind: r.get(1)?,
                    created: r.get(2)?,
                    last_seen: r.get(3)?,
                    user_agent: r.get(4)?,
                    ip: r.get(5)?,
                    client: r.get(6)?,
                    revoked: r.get::<_, i64>(7)? != 0,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Revoke a single session, but only if it belongs to the given user.
    /// Token sessions are deleted; Subsonic client sessions are flagged so the
    /// client cannot simply authenticate again.
    pub fn revoke_session_for_user(&self, token: &str, user_id: i64) -> AppResult<bool> {
        let conn = self.lock();
        let kind: Option<String> = conn
            .query_row(
                "SELECT kind FROM sessions WHERE token = ?1 AND user_id = ?2",
                params![token, user_id],
                |r| r.get(0),
            )
            .optional()?;
        match kind.as_deref() {
            None => Ok(false),
            Some("subsonic") => Ok(conn.execute(
                "UPDATE sessions SET revoked = 1 WHERE token = ?1 AND user_id = ?2",
                params![token, user_id],
            )? > 0),
            Some(_) => Ok(conn.execute(
                "DELETE FROM sessions WHERE token = ?1 AND user_id = ?2",
                params![token, user_id],
            )? > 0),
        }
    }

    /// Undo a revocation for a Subsonic client session.
    pub fn allow_session_for_user(&self, token: &str, user_id: i64) -> AppResult<bool> {
        let conn = self.lock();
        Ok(conn.execute(
            "UPDATE sessions SET revoked = 0
             WHERE token = ?1 AND user_id = ?2 AND kind = 'subsonic'",
            params![token, user_id],
        )? > 0)
    }

    /// Revoke every session belonging to a user. Returns the number affected.
    pub fn delete_all_sessions_for_user(&self, user_id: i64) -> AppResult<usize> {
        let conn = self.lock();
        let deleted = conn.execute(
            "DELETE FROM sessions WHERE user_id = ?1 AND kind <> 'subsonic'",
            params![user_id],
        )?;
        let flagged = conn.execute(
            "UPDATE sessions SET revoked = 1 WHERE user_id = ?1 AND kind = 'subsonic'",
            params![user_id],
        )?;
        Ok(deleted + flagged)
    }

    pub fn get_or_create_api_key(&self, user_id: i64) -> AppResult<String> {
        let conn = self.lock();
        if let Some(key) = conn
            .query_row(
                "SELECT key FROM api_keys WHERE user_id = ?1 ORDER BY created ASC LIMIT 1",
                params![user_id],
                |r| r.get::<_, String>(0),
            )
            .optional()?
        {
            return Ok(key);
        }
        let key = format!("bl_{}", Uuid::new_v4().simple());
        let now = Utc::now().timestamp();
        conn.execute(
            "INSERT INTO api_keys(key, user_id, label, created) VALUES (?1, ?2, 'default', ?3)",
            params![key, user_id, now],
        )?;
        Ok(key)
    }

    pub fn regenerate_api_key(&self, user_id: i64) -> AppResult<String> {
        let key = format!("bl_{}", Uuid::new_v4().simple());
        let now = Utc::now().timestamp();
        let conn = self.lock();
        conn.execute("DELETE FROM api_keys WHERE user_id = ?1", params![user_id])?;
        conn.execute(
            "INSERT INTO api_keys(key, user_id, label, created) VALUES (?1, ?2, 'default', ?3)",
            params![key, user_id, now],
        )?;
        Ok(key)
    }

    pub fn get_user_by_api_key(&self, key: &str) -> AppResult<Option<User>> {
        if key.trim().is_empty() {
            return Ok(None);
        }
        let conn = self.lock();
        Ok(conn
            .query_row(
                "SELECT u.id, u.username, u.password, u.created
                 FROM api_keys k JOIN users u ON u.id = k.user_id
                 WHERE k.key = ?1 AND u.deleted_at = 0",
                params![key.trim()],
                row_to_user,
            )
            .optional()?)
    }

    // -------------------------------------------------------------------------
    // Two-factor authentication, passkeys & seven-day account recovery
    // -------------------------------------------------------------------------

    pub fn get_totp(&self, user_id: i64) -> AppResult<Option<TotpState>> {
        let conn = self.lock();
        Ok(conn
            .query_row(
                "SELECT secret, enabled, created, last_step FROM totp_secrets WHERE user_id = ?1",
                params![user_id],
                |r| {
                    Ok(TotpState {
                        secret: r.get(0)?,
                        enabled: r.get::<_, i64>(1)? != 0,
                        created: r.get(2)?,
                        last_step: r.get(3)?,
                    })
                },
            )
            .optional()?)
    }

    /// Creates or replaces a pending/enabled TOTP enrolment. Re-enrolling always
    /// resets the replay counter and invalidates previously issued codes.
    pub fn save_totp(&self, user_id: i64, secret: &str, enabled: bool) -> AppResult<()> {
        let conn = self.lock();
        let now = Utc::now().timestamp();
        conn.execute(
            "INSERT INTO totp_secrets(user_id, secret, enabled, created, last_step)
             VALUES (?1, ?2, ?3, ?4, 0)
             ON CONFLICT(user_id) DO UPDATE SET
                 secret = excluded.secret, enabled = excluded.enabled,
                 created = excluded.created, last_step = 0",
            params![user_id, secret, if enabled { 1 } else { 0 }, now],
        )?;
        Ok(())
    }

    pub fn set_totp_enabled(&self, user_id: i64, enabled: bool) -> AppResult<()> {
        let conn = self.lock();
        conn.execute(
            "UPDATE totp_secrets SET enabled = ?1 WHERE user_id = ?2",
            params![if enabled { 1 } else { 0 }, user_id],
        )?;
        Ok(())
    }

    pub fn mark_totp_step(&self, user_id: i64, step: i64) -> AppResult<()> {
        let conn = self.lock();
        conn.execute(
            "UPDATE totp_secrets SET last_step = ?1 WHERE user_id = ?2",
            params![step, user_id],
        )?;
        Ok(())
    }

    pub fn passkey_count(&self, user_id: i64) -> AppResult<usize> {
        let conn = self.lock();
        Ok(conn.query_row(
            "SELECT COUNT(*) FROM passkeys WHERE user_id = ?1",
            params![user_id],
            |r| r.get::<_, i64>(0).map(|n| n as usize),
        )?)
    }

    /// A user has 2FA when TOTP is enabled or at least one passkey is enrolled.
    pub fn is_2fa_enabled(&self, user_id: i64) -> AppResult<bool> {
        if let Some(state) = self.get_totp(user_id)? {
            if state.enabled {
                return Ok(true);
            }
        }
        Ok(self.passkey_count(user_id)? > 0)
    }

    /// List the credentials registered for an account (public data only).
    pub fn list_passkeys(&self, user_id: i64) -> AppResult<Vec<PasskeyInfo>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT credential_id, label, created, last_used FROM passkeys
             WHERE user_id = ?1 ORDER BY created DESC",
        )?;
        let rows = stmt
            .query_map(params![user_id], |r| {
                Ok(PasskeyInfo {
                    credential_id: r.get(0)?,
                    label: r.get(1)?,
                    created: r.get(2)?,
                    last_used: r.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Serialised passkeys used to build a WebAuthn assertion challenge.
    pub fn get_passkeys_json(&self, user_id: i64) -> AppResult<Vec<String>> {
        let conn = self.lock();
        let mut stmt = conn.prepare("SELECT passkey_json FROM passkeys WHERE user_id = ?1")?;
        let rows = stmt
            .query_map(params![user_id], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn get_passkey_by_id(&self, credential_id: &str) -> AppResult<Option<(i64, String)>> {
        let conn = self.lock();
        Ok(conn
            .query_row(
                "SELECT user_id, passkey_json FROM passkeys WHERE credential_id = ?1",
                params![credential_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }

    pub fn save_passkey(
        &self,
        user_id: i64,
        credential_id: &str,
        label: &str,
        passkey_json: &str,
    ) -> AppResult<()> {
        let conn = self.lock();
        let now = Utc::now().timestamp();
        conn.execute(
            "INSERT INTO passkeys(credential_id, user_id, label, passkey_json, created)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(credential_id) DO UPDATE SET
                 user_id = excluded.user_id, label = excluded.label,
                 passkey_json = excluded.passkey_json, created = excluded.created",
            params![credential_id, user_id, label, passkey_json, now],
        )?;
        Ok(())
    }

    pub fn delete_passkey(&self, user_id: i64, credential_id: &str) -> AppResult<bool> {
        let conn = self.lock();
        Ok(conn.execute(
            "DELETE FROM passkeys WHERE user_id = ?1 AND credential_id = ?2",
            params![user_id, credential_id],
        )? > 0)
    }

    pub fn delete_all_passkeys(&self, user_id: i64) -> AppResult<usize> {
        let conn = self.lock();
        Ok(conn.execute("DELETE FROM passkeys WHERE user_id = ?1", params![user_id])?)
    }

    pub fn touch_passkey(&self, credential_id: &str, passkey_json: &str) -> AppResult<()> {
        let conn = self.lock();
        conn.execute(
            "UPDATE passkeys SET passkey_json = ?1, last_used = ?2 WHERE credential_id = ?3",
            params![passkey_json, Utc::now().timestamp(), credential_id],
        )?;
        Ok(())
    }

    /// Stores a short-lived challenge (login, restore, WebAuthn ceremony) and
    /// returns the opaque id handed to the browser.
    pub fn create_auth_challenge(&self, user_id: i64, kind: &str, payload: &str) -> AppResult<String> {
        let id = crate::auth::random_token();
        let conn = self.lock();
        conn.execute(
            "INSERT INTO auth_challenges(id, user_id, kind, payload, created) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, user_id, kind, payload, Utc::now().timestamp()],
        )?;
        Ok(id)
    }

    /// Atomically reads and removes a challenge. Expired challenges are dropped.
    pub fn take_auth_challenge(&self, id: &str, kind: &str) -> AppResult<Option<(i64, String)>> {
        let conn = self.lock();
        let row: Option<(i64, String, i64)> = conn
            .query_row(
                "SELECT user_id, payload, created FROM auth_challenges WHERE id = ?1 AND kind = ?2",
                params![id, kind],
                |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)?)),
            )
            .optional()?;
        conn.execute("DELETE FROM auth_challenges WHERE id = ?1", params![id])?;
        let Some((user_id, payload, created)) = row else {
            return Ok(None);
        };
        let ttl = if kind.starts_with("passkey") {
            crate::auth::WEBAUTHN_CHALLENGE_TTL
        } else {
            crate::auth::LOGIN_CHALLENGE_TTL
        };
        if Utc::now().timestamp() - created > ttl {
            return Ok(None);
        }
        Ok(Some((user_id, payload)))
    }

    pub fn purge_expired_auth_challenges(&self) -> AppResult<()> {
        let conn = self.lock();
        conn.execute(
            "DELETE FROM auth_challenges WHERE created < ?1",
            params![Utc::now().timestamp() - 3600],
        )?;
        Ok(())
    }

    /// Replace all recovery codes with freshly generated one-time codes (stored
    /// only as SHA-256 hashes).
    pub fn replace_recovery_codes(&self, user_id: i64, codes: &[String]) -> AppResult<()> {
        let mut conn = self.lock();
        let now = Utc::now().timestamp();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM recovery_codes WHERE user_id = ?1", params![user_id])?;
        for code in codes {
            tx.execute(
                "INSERT INTO recovery_codes(code_hash, user_id, created) VALUES (?1, ?2, ?3)",
                params![hash_recovery_code(code), user_id, now],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Atomically consume a recovery code. Returns true only on first use.
    pub fn consume_recovery_code(&self, user_id: i64, code: &str) -> AppResult<bool> {
        let hash = hash_recovery_code(code);
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        let found: Option<String> = tx
            .query_row(
                "SELECT code_hash FROM recovery_codes
                 WHERE user_id = ?1 AND code_hash = ?2 AND used = 0",
                params![user_id, hash],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(code_hash) = found {
            tx.execute(
                "UPDATE recovery_codes SET used = 1 WHERE code_hash = ?1",
                params![code_hash],
            )?;
            tx.commit()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Returns (total, used) recovery codes, so the UI can show what is left.
    pub fn recovery_code_stats(&self, user_id: i64) -> AppResult<(usize, usize)> {
        let conn = self.lock();
        Ok(conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(used), 0) FROM recovery_codes WHERE user_id = ?1",
            params![user_id],
            |r| Ok((r.get::<_, i64>(0)? as usize, r.get::<_, i64>(1)? as usize)),
        )?)
    }

    // -- Session step-up state -------------------------------------------------

    /// (user_id, mfa_at) for a session token.
    pub fn session_auth_state(&self, token: &str) -> AppResult<Option<(i64, i64)>> {
        let conn = self.lock();
        Ok(conn
            .query_row(
                "SELECT user_id, mfa_at FROM sessions WHERE token = ?1 AND revoked = 0",
                params![token.trim()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }

    pub fn mark_session_mfa(&self, token: &str) -> AppResult<()> {
        let conn = self.lock();
        conn.execute(
            "UPDATE sessions SET mfa_at = ?1 WHERE token = ?2",
            params![Utc::now().timestamp(), token.trim()],
        )?;
        Ok(())
    }

    // -- Soft deletion & recovery ---------------------------------------------

    pub fn user_deleted_at(&self, user_id: i64) -> AppResult<Option<i64>> {
        let conn = self.lock();
        Ok(conn
            .query_row(
                "SELECT deleted_at FROM users WHERE id = ?1",
                params![user_id],
                |r| r.get::<_, i64>(0),
            )
            .optional()?)
    }

    /// Soft-delete an account: all sessions and API keys are destroyed so no
    /// client stays signed in, while library data and 2FA settings are kept for
    /// the seven-day recovery window.
    pub fn soft_delete_user(&self, user_id: i64, restore_hash: &str) -> AppResult<()> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        tx.execute(
            "UPDATE users SET deleted_at = ?1, restore_hash = ?2 WHERE id = ?3",
            params![Utc::now().timestamp(), restore_hash, user_id],
        )?;
        tx.execute("DELETE FROM sessions WHERE user_id = ?1", params![user_id])?;
        tx.execute("DELETE FROM api_keys WHERE user_id = ?1", params![user_id])?;
        tx.commit()?;
        Ok(())
    }

    pub fn restore_hash(&self, user_id: i64) -> AppResult<Option<String>> {
        let conn = self.lock();
        Ok(conn
            .query_row(
                "SELECT restore_hash FROM users WHERE id = ?1 AND deleted_at > 0",
                params![user_id],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// Ways a user may still be inside the recovery window.
    pub fn deletion_expires_at(&self, user_id: i64) -> AppResult<Option<i64>> {
        Ok(self
            .user_deleted_at(user_id)?
            .map(|at| at + RESTORE_WINDOW_SECS))
    }

    pub fn restore_user(&self, user_id: i64) -> AppResult<()> {
        let conn = self.lock();
        conn.execute(
            "UPDATE users SET deleted_at = 0, restore_hash = '' WHERE id = ?1",
            params![user_id],
        )?;
        Ok(())
    }

    /// Hard-deletes accounts whose recovery window has expired. Cascading
    /// foreign keys remove their library, playlists, social graph and 2FA data.
    pub fn purge_expired_deleted_users(&self) -> AppResult<usize> {
        let conn = self.lock();
        Ok(conn.execute(
            "DELETE FROM users WHERE deleted_at > 0 AND deleted_at < ?1",
            params![Utc::now().timestamp() - RESTORE_WINDOW_SECS],
        )?)
    }

    // -- Security audit trail --------------------------------------------------

    pub fn record_security_event(&self, user_id: i64, kind: &str, detail: &str, ip: &str) {
        let conn = self.lock();
        let _ = conn.execute(
            "INSERT INTO security_events(user_id, kind, detail, ip, created) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![user_id, kind, detail, ip, Utc::now().timestamp()],
        );
    }

    pub fn list_security_events(&self, user_id: i64, limit: usize) -> AppResult<Vec<SecurityEvent>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT id, kind, detail, ip, created FROM security_events
             WHERE user_id = ?1 ORDER BY created DESC, id DESC LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![user_id, limit as i64], |r| {
                Ok(SecurityEvent {
                    id: r.get(0)?,
                    kind: r.get(1)?,
                    detail: r.get(2)?,
                    ip: r.get(3)?,
                    created: r.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    // -------------------------------------------------------------------------
    // Administration (gated by ADMIN_USERNAME at the HTTP layer)
    // -------------------------------------------------------------------------

    pub fn admin_overview(&self) -> AppResult<AdminOverview> {
        let conn = self.lock();
        let count = |sql: &str| -> AppResult<usize> {
            Ok(conn.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap_or(0) as usize)
        };
        Ok(AdminOverview {
            total_users: count("SELECT COUNT(*) FROM users")?,
            active_users: count("SELECT COUNT(*) FROM users WHERE deleted_at = 0")?,
            deleted_users: count("SELECT COUNT(*) FROM users WHERE deleted_at > 0")?,
            sessions: count("SELECT COUNT(*) FROM sessions WHERE revoked = 0")?,
            client_sessions: count("SELECT COUNT(*) FROM sessions WHERE kind = 'subsonic' AND revoked = 0")?,
            tracks: count("SELECT COUNT(*) FROM tracks")?,
            passkeys: count("SELECT COUNT(*) FROM passkeys")?,
            totp_users: count("SELECT COUNT(*) FROM totp_secrets WHERE enabled = 1")?,
            recovery_codes: count("SELECT COUNT(*) FROM recovery_codes WHERE used = 0")?,
            playlists: count("SELECT COUNT(*) FROM playlists")?,
            security_events: count("SELECT COUNT(*) FROM security_events")?,
        })
    }

    pub fn admin_list_users(&self) -> AppResult<Vec<AdminUserSummary>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT u.id, u.username, u.created, u.deleted_at,
                    COALESCE(t.enabled, 0),
                    (SELECT COUNT(*) FROM passkeys p WHERE p.user_id = u.id),
                    (SELECT COUNT(*) FROM recovery_codes r WHERE r.user_id = u.id AND r.used = 0),
                    (SELECT COUNT(*) FROM sessions s WHERE s.user_id = u.id AND s.revoked = 0),
                    (SELECT COUNT(*) FROM playlists p WHERE p.user_id = u.id),
                    (SELECT COUNT(*) FROM stars st WHERE st.user_id = u.id),
                    (SELECT COUNT(*) FROM security_events e WHERE e.user_id = u.id),
                    (SELECT COUNT(*) FROM user_track_stats uts WHERE uts.user_id = u.id),
                    COALESCE((SELECT MAX(last_seen) FROM sessions s WHERE s.user_id = u.id), 0)
             FROM users u LEFT JOIN totp_secrets t ON t.user_id = u.id
             ORDER BY u.id ASC",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(AdminUserSummary {
                    id: r.get(0)?,
                    username: r.get(1)?,
                    created: r.get(2)?,
                    deleted_at: r.get(3)?,
                    totp_enabled: r.get::<_, i64>(4)? != 0,
                    passkeys: r.get::<_, i64>(5)? as usize,
                    recovery_left: r.get::<_, i64>(6)? as usize,
                    sessions: r.get::<_, i64>(7)? as usize,
                    playlists: r.get::<_, i64>(8)? as usize,
                    stars: r.get::<_, i64>(9)? as usize,
                    events: r.get::<_, i64>(10)? as usize,
                    listens: r.get::<_, i64>(11)? as usize,
                    last_seen: r.get(12)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn admin_global_events(&self, limit: usize) -> AppResult<Vec<AdminEvent>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT e.user_id, u.username, e.kind, e.detail, e.ip, e.created
             FROM security_events e JOIN users u ON u.id = e.user_id
             ORDER BY e.created DESC, e.id DESC LIMIT ?1",
        )?;
        let rows = stmt
            .query_map(params![limit as i64], |r| {
                Ok(AdminEvent {
                    user_id: r.get(0)?,
                    username: r.get(1)?,
                    kind: r.get(2)?,
                    detail: r.get(3)?,
                    ip: r.get(4)?,
                    created: r.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Permanently removes an account. Only for admin use after the recovery
    /// window has been skipped explicitly.
    pub fn hard_delete_user(&self, user_id: i64) -> AppResult<()> {
        let conn = self.lock();
        conn.execute("DELETE FROM users WHERE id = ?1", params![user_id])?;
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Social layer: profiles, following graph & auto-updating user radio
    // -------------------------------------------------------------------------

    /// Base SELECT for a `PublicUser`. Viewer id is always bound as `?1` so the
    /// `is_following` flag can be computed in SQL for any caller.
    const PUBLIC_USER_SELECT: &'static str = "
        SELECT u.id, u.username, u.created,
               COALESCE(p.display_name, '') AS display_name,
               COALESCE(p.bio, '') AS bio,
               COALESCE(p.avatar_color, '') AS avatar_color,
               COALESCE(p.location, '') AS location,
               (SELECT COUNT(*) FROM follows f WHERE f.following_id = u.id) AS followers_count,
               (SELECT COUNT(*) FROM follows f WHERE f.follower_id = u.id) AS following_count,
               EXISTS(SELECT 1 FROM follows f WHERE f.follower_id = ?1 AND f.following_id = u.id) AS is_following,
               COALESCE(p.avatar_version, 0) AS avatar_version,
               COALESCE(p.avatar_mime, '') AS avatar_mime
        FROM users u
        LEFT JOIN user_profiles p ON p.user_id = u.id
    ";

    pub fn get_profile(&self, user_id: i64) -> AppResult<UserProfile> {
        let conn = self.lock();
        let profile = conn
            .query_row(
                "SELECT user_id, display_name, bio, avatar_color, location, updated
                 FROM user_profiles WHERE user_id = ?1",
                params![user_id],
                |r| {
                    Ok(UserProfile {
                        user_id: r.get(0)?,
                        display_name: r.get(1)?,
                        bio: r.get(2)?,
                        avatar_color: r.get(3)?,
                        location: r.get(4)?,
                        updated: r.get(5)?,
                    })
                },
            )
            .optional()?;
        Ok(profile.unwrap_or_else(|| UserProfile {
            user_id,
            ..Default::default()
        }))
    }

    pub fn update_profile(
        &self,
        user_id: i64,
        display_name: &str,
        bio: &str,
        avatar_color: &str,
        location: &str,
    ) -> AppResult<UserProfile> {
        let display_name = clip_str(display_name, 40);
        let bio = clip_str(bio, 280);
        let avatar_color = clip_str(avatar_color, 24);
        // Only accept real hex colors — this value is rendered into a CSS
        // style attribute on the client.
        let avatar_color = if is_hex_color(&avatar_color) {
            avatar_color
        } else {
            String::new()
        };
        let location = clip_str(location, 80);
        let now = Utc::now().timestamp();
        let conn = self.lock();
        conn.execute(
            "INSERT INTO user_profiles(user_id, display_name, bio, avatar_color, location, updated)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(user_id) DO UPDATE SET
                 display_name = excluded.display_name,
                 bio = excluded.bio,
                 avatar_color = excluded.avatar_color,
                 location = excluded.location,
                 updated = excluded.updated",
            params![user_id, display_name, bio, avatar_color, location, now],
        )?;
        Ok(UserProfile {
            user_id,
            display_name,
            bio,
            avatar_color,
            location,
            updated: now,
        })
    }

    /// Stores (or replaces) the user's uploaded profile picture. The row is
    /// created with defaults if the user has never edited their profile.
    pub fn set_avatar(&self, user_id: i64, bytes: &[u8], mime: &str) -> AppResult<()> {
        let now = Utc::now().timestamp();
        let conn = self.lock();
        conn.execute(
            "INSERT INTO user_profiles(user_id, updated, avatar, avatar_mime, avatar_version)
             VALUES (?1, ?2, ?3, ?4, 1)
             ON CONFLICT(user_id) DO UPDATE SET
                 updated = excluded.updated,
                 avatar = excluded.avatar,
                 avatar_mime = excluded.avatar_mime,
                 avatar_version = user_profiles.avatar_version + 1",
            params![user_id, now, bytes, mime],
        )?;
        Ok(())
    }

    /// Removes the uploaded picture so the generated initial/color avatar shows.
    pub fn clear_avatar(&self, user_id: i64) -> AppResult<()> {
        let now = Utc::now().timestamp();
        let conn = self.lock();
        conn.execute(
            "INSERT INTO user_profiles(user_id, updated, avatar_mime, avatar_version)
             VALUES (?1, ?2, '', 1)
             ON CONFLICT(user_id) DO UPDATE SET
                 updated = excluded.updated,
                 avatar = NULL,
                 avatar_mime = '',
                 avatar_version = user_profiles.avatar_version + 1",
            params![user_id, now],
        )?;
        Ok(())
    }

    /// Reads the stored picture and its MIME type, if the user has one.
    pub fn get_avatar(&self, user_id: i64) -> AppResult<Option<(Vec<u8>, String)>> {
        let conn = self.lock();
        let result = conn
            .query_row(
                "SELECT avatar, avatar_mime FROM user_profiles
                 WHERE user_id = ?1 AND avatar IS NOT NULL AND avatar_mime != ''",
                params![user_id],
                |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()?;
        Ok(result)
    }

    pub fn get_public_user(&self, target_id: i64, viewer_id: i64) -> AppResult<Option<PublicUser>> {
        let conn = self.lock();
        let sql = format!("{} WHERE u.id = ?2", Self::PUBLIC_USER_SELECT);
        let user = conn
            .query_row(&sql, params![viewer_id, target_id], row_to_public_user)
            .optional()?;
        Ok(user.map(|mut u| {
            u.is_self = u.id == viewer_id;
            u
        }))
    }

    /// Search users by username, display name or bio. A leading `@` is ignored
    /// so `@emily` and `emily` behave identically.
    pub fn search_public_users(
        &self,
        query: &str,
        viewer_id: i64,
        limit: usize,
    ) -> AppResult<Vec<PublicUser>> {
        let q = query.trim().trim_start_matches('@').trim().to_lowercase();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.lock();
        let sql = format!(
            "{base}
             WHERE instr(lower(u.username), ?2) > 0
                OR instr(lower(COALESCE(p.display_name, '')), ?2) > 0
                OR instr(lower(COALESCE(p.bio, '')), ?2) > 0
             ORDER BY
                CASE WHEN lower(u.username) = ?2 THEN 0
                     WHEN lower(COALESCE(p.display_name, '')) = ?2 THEN 1
                     ELSE 2 END ASC,
                is_following DESC,
                followers_count DESC,
                u.username ASC
             LIMIT ?3",
            base = Self::PUBLIC_USER_SELECT,
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![viewer_id, q, limit as i64], row_to_public_user)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows
            .into_iter()
            .map(|mut u| {
                u.is_self = u.id == viewer_id;
                u
            })
            .collect())
    }

    /// A handful of suggested people for the "find friends" area.
    pub fn suggested_users(&self, viewer_id: i64, limit: usize) -> AppResult<Vec<PublicUser>> {
        let conn = self.lock();
        let sql = format!(
            "{base}
             WHERE u.id != ?1
             ORDER BY followers_count DESC, u.username ASC
             LIMIT ?2",
            base = Self::PUBLIC_USER_SELECT,
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![viewer_id, limit as i64], row_to_public_user)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows
            .into_iter()
            .map(|mut u| {
                u.is_self = u.id == viewer_id;
                u
            })
            .collect())
    }

    pub fn follow_user(&self, follower_id: i64, following_id: i64) -> AppResult<()> {
        if follower_id == following_id {
            return Err(err_msg("You can't follow yourself"));
        }
        let conn = self.lock();
        let exists: Option<i64> = conn
            .query_row("SELECT id FROM users WHERE id = ?1", params![following_id], |r| {
                r.get(0)
            })
            .optional()?;
        if exists.is_none() {
            return Err(err_msg("User not found"));
        }
        conn.execute(
            "INSERT INTO follows(follower_id, following_id, created) VALUES (?1, ?2, ?3)
             ON CONFLICT(follower_id, following_id) DO NOTHING",
            params![follower_id, following_id, Utc::now().timestamp()],
        )?;
        Ok(())
    }

    pub fn unfollow_user(&self, follower_id: i64, following_id: i64) -> AppResult<()> {
        let conn = self.lock();
        conn.execute(
            "DELETE FROM follows WHERE follower_id = ?1 AND following_id = ?2",
            params![follower_id, following_id],
        )?;
        Ok(())
    }

    pub fn list_followers(&self, user_id: i64, viewer_id: i64) -> AppResult<Vec<PublicUser>> {
        let conn = self.lock();
        let sql = format!(
            "{base}
             WHERE u.id IN (SELECT follower_id FROM follows WHERE following_id = ?2)
             ORDER BY (SELECT created FROM follows WHERE follower_id = u.id AND following_id = ?2) DESC,
                      u.username ASC",
            base = Self::PUBLIC_USER_SELECT,
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![viewer_id, user_id], row_to_public_user)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows
            .into_iter()
            .map(|mut u| {
                u.is_self = u.id == viewer_id;
                u
            })
            .collect())
    }

    pub fn list_following(&self, user_id: i64, viewer_id: i64) -> AppResult<Vec<PublicUser>> {
        let conn = self.lock();
        let sql = format!(
            "{base}
             WHERE u.id IN (SELECT following_id FROM follows WHERE follower_id = ?2)
             ORDER BY (SELECT created FROM follows WHERE follower_id = ?2 AND following_id = u.id) DESC,
                      u.username ASC",
            base = Self::PUBLIC_USER_SELECT,
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![viewer_id, user_id], row_to_public_user)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows
            .into_iter()
            .map(|mut u| {
                u.is_self = u.id == viewer_id;
                u
            })
            .collect())
    }

    /// Playlist backing a saved radio for this (owner, target) pair.
    pub fn get_user_radio(&self, owner_id: i64, target_id: i64) -> AppResult<Option<String>> {
        let conn = self.lock();
        Ok(conn
            .query_row(
                "SELECT playlist_id FROM user_radios WHERE owner_id = ?1 AND target_id = ?2",
                params![owner_id, target_id],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn save_user_radio(
        &self,
        owner_id: i64,
        target_id: i64,
        playlist_id: &str,
    ) -> AppResult<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO user_radios(owner_id, target_id, playlist_id, created)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(owner_id, target_id) DO UPDATE SET playlist_id = excluded.playlist_id",
            params![owner_id, target_id, playlist_id, Utc::now().timestamp()],
        )?;
        Ok(())
    }

    /// Removes a saved radio and returns its playlist id so the caller can also
    /// delete the playlist from the owner's library.
    pub fn delete_user_radio(&self, owner_id: i64, target_id: i64) -> AppResult<Option<String>> {
        let conn = self.lock();
        let pid: Option<String> = conn
            .query_row(
                "SELECT playlist_id FROM user_radios WHERE owner_id = ?1 AND target_id = ?2",
                params![owner_id, target_id],
                |r| r.get(0),
            )
            .optional()?;
        conn.execute(
            "DELETE FROM user_radios WHERE owner_id = ?1 AND target_id = ?2",
            params![owner_id, target_id],
        )?;
        Ok(pid)
    }

    /// All saved radios — the background curator walks this to refresh each
    /// playlist from its target user's evolving listening taste.
    pub fn all_user_radios(&self) -> AppResult<Vec<(i64, i64, String)>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT owner_id, target_id, playlist_id FROM user_radios ORDER BY created ASC",
        )?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    // -------------------------------------------------------------------------
    // Tracks, Albums Metadata & Artists Metadata
    // -------------------------------------------------------------------------

    pub fn upsert_tracks(&self, tracks: &[Track]) -> AppResult<()> {
        if tracks.is_empty() {
            return Ok(());
        }
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO tracks (
                    id, source, title, artist, album, genre, moods, year, track_no, disc_no,
                    duration, bitrate, sample_rate, bit_depth, format, size, stream_url,
                    cover_url, meta, deezer_id, created
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)
                ON CONFLICT(id) DO UPDATE SET
                    title = CASE WHEN excluded.title != '' THEN excluded.title ELSE tracks.title END,
                    artist = CASE WHEN excluded.artist != '' THEN excluded.artist ELSE tracks.artist END,
                    album = CASE WHEN excluded.album != '' THEN excluded.album ELSE tracks.album END,
                    genre = CASE WHEN excluded.genre != '' THEN excluded.genre ELSE tracks.genre END,
                    moods = CASE WHEN excluded.moods != '' THEN excluded.moods ELSE tracks.moods END,
                    year = CASE WHEN excluded.year > 0 THEN excluded.year ELSE tracks.year END,
                    track_no = CASE WHEN excluded.track_no > 0 THEN excluded.track_no ELSE tracks.track_no END,
                    disc_no = CASE WHEN excluded.disc_no > 0 THEN excluded.disc_no ELSE tracks.disc_no END,
                    duration = CASE WHEN excluded.duration > 0 THEN excluded.duration ELSE tracks.duration END,
                    bitrate = CASE WHEN excluded.bitrate > 0 THEN excluded.bitrate ELSE tracks.bitrate END,
                    sample_rate = CASE WHEN excluded.sample_rate > 0 THEN excluded.sample_rate ELSE tracks.sample_rate END,
                    bit_depth = CASE WHEN excluded.bit_depth > 0 THEN excluded.bit_depth ELSE tracks.bit_depth END,
                    format = CASE WHEN excluded.format != '' THEN excluded.format ELSE tracks.format END,
                    size = CASE WHEN excluded.size > 0 THEN excluded.size ELSE tracks.size END,
                    stream_url = CASE WHEN excluded.stream_url != '' THEN excluded.stream_url ELSE tracks.stream_url END,
                    cover_url = CASE WHEN excluded.cover_url != '' THEN excluded.cover_url ELSE tracks.cover_url END,
                    meta = CASE WHEN excluded.meta != '' THEN excluded.meta ELSE tracks.meta END,
                    deezer_id = CASE WHEN excluded.deezer_id != '' THEN excluded.deezer_id ELSE tracks.deezer_id END",
            )?;
            let now = Utc::now().timestamp();
            for t in tracks {
                let created = if t.created > 0 { t.created } else { now };
                stmt.execute(params![
                    t.id,
                    t.source,
                    t.title,
                    t.artist,
                    t.album,
                    t.genre,
                    t.moods,
                    t.year,
                    t.track_no,
                    t.disc_no,
                    t.duration,
                    t.bitrate,
                    t.sample_rate,
                    t.bit_depth,
                    t.format,
                    t.size,
                    t.stream_url,
                    t.cover_url,
                    t.meta,
                    t.deezer_id,
                    created,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn get_track(&self, id: &str) -> AppResult<Option<Track>> {
        let conn = self.lock();
        let sql = format!("SELECT {} FROM tracks WHERE id = ?1", TRACK_COLS);
        Ok(conn.query_row(&sql, params![id], row_to_track).optional()?)
    }

    pub fn get_tracks_by_ids(&self, ids: &[String]) -> AppResult<Vec<Track>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.lock();
        let sql = format!("SELECT {} FROM tracks WHERE id = ?1", TRACK_COLS);
        let mut stmt = conn.prepare(&sql)?;
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(t) = stmt.query_row(params![id], row_to_track).optional()? {
                out.push(t);
            }
        }
        Ok(out)
    }

    pub fn count_tracks(&self) -> AppResult<i64> {
        let conn = self.lock();
        Ok(conn.query_row("SELECT COUNT(*) FROM tracks", [], |r| r.get(0))?)
    }

    pub fn all_tracks(&self, limit: usize, offset: usize) -> AppResult<Vec<Track>> {
        let conn = self.lock();
        let sql = format!(
            "SELECT {} FROM tracks ORDER BY created DESC LIMIT ?1 OFFSET ?2",
            TRACK_COLS
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![limit as i64, offset as i64], row_to_track)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Returns user-affinity tracks (starred, manual playlists, played, rated, expanded radio links,
    /// same-artist/local/same-genre catalog matches) unioned with recent catalog tracks so personalized
    /// mixes never lose taste candidates past recent-insert offset limits.
    /// True once the account has produced any personal listening/taste signal.
    /// Brand-new accounts are kept on a clean, chart-only starter slate so they
    /// never inherit other listeners' activity or cached/local songs.
    pub fn has_listening_history(&self, user_id: i64) -> AppResult<bool> {
        if user_id <= 0 {
            return Ok(false);
        }
        let conn = self.lock();
        let signals: i64 = conn.query_row(
            "SELECT
                (SELECT COUNT(*) FROM user_track_stats WHERE user_id = ?1) +
                (SELECT COUNT(*) FROM stars WHERE user_id = ?1) +
                (SELECT COUNT(*) FROM ratings WHERE user_id = ?1) +
                (SELECT COUNT(*) FROM playlists WHERE user_id = ?1 AND is_auto = 0) +
                (SELECT COUNT(*) FROM user_taste_links WHERE user_id = ?1) +
                (SELECT COUNT(*) FROM followed_artists WHERE user_id = ?1)",
            params![user_id],
            |r| r.get(0),
        )?;
        Ok(signals > 0)
    }

    /// External, currently-popular chart tracks only. This is the entire
    /// starter pool for fresh accounts — no instance listening, no local files,
    /// no cached search/listen results. Returns empty until charts are loaded,
    /// rather than falling back to arbitrary rows from the shared catalog.
    pub fn popular_chart_tracks_for_new_user(&self, limit: usize) -> AppResult<Vec<Track>> {
        if limit == 0 {
            return Ok(vec![]);
        }
        let conn = self.lock();
        let tcols = prefixed_track_cols("t");
        let sql = format!(
            "SELECT {} FROM chart_catalog c
             JOIN tracks t ON t.id = c.track_id
             WHERE t.source = 'deezer'
             ORDER BY c.rank_pos ASC, c.updated DESC
             LIMIT ?1",
            tcols
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![limit as i64], row_to_track)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn candidate_tracks_for_user(&self, user_id: i64, limit: usize) -> AppResult<Vec<Track>> {
        // Cold users get a dedicated clean-slate pool; normal recommendation
        // paths (including the recent-catalog fallback) are skipped entirely.
        if user_id > 0 && !self.has_listening_history(user_id)? {
            return self.popular_chart_tracks_for_new_user(limit);
        }
        let conn = self.lock();
        let tcols = prefixed_track_cols("t");
        let mut out = Vec::new();
        let mut seen = HashSet::new();

        if user_id > 0 {
            let user_sql = format!(
                "SELECT {} FROM tracks t
                 WHERE t.id IN (
                     SELECT track_id FROM stars WHERE user_id = ?1
                     UNION
                     SELECT pt.track_id FROM playlist_tracks pt
                       JOIN playlists p ON p.id = pt.playlist_id
                       WHERE p.user_id = ?1 AND p.is_auto = 0
                     UNION
                     SELECT track_id FROM user_track_stats WHERE user_id = ?1
                     UNION
                     SELECT track_id FROM ratings WHERE user_id = ?1
                 )",
                tcols
            );
            let mut ustmt = conn.prepare(&user_sql)?;
            for t in ustmt
                .query_map(params![user_id], row_to_track)?
                .collect::<Result<Vec<_>, _>>()?
            {
                if seen.insert(t.id.clone()) {
                    out.push(t);
                }
            }

            // Include related tracks discovered for this user's taste seeds before generic charts.
            if out.len() < limit {
                let rel_sql = format!(
                    "SELECT {} FROM user_taste_links l
                     JOIN tracks t ON t.id = l.key
                     WHERE l.user_id = ?1 AND l.kind IN ('related_track', 'neural_track')
                     ORDER BY l.updated DESC, l.weight DESC
                     LIMIT ?2",
                    tcols
                );
                let mut rstmt = conn.prepare(&rel_sql)?;
                let rel_lim = (limit - out.len()).min(500) as i64;
                for t in rstmt
                    .query_map(params![user_id, rel_lim], row_to_track)?
                    .collect::<Result<Vec<_>, _>>()?
                {
                    if out.len() >= limit {
                        break;
                    }
                    if seen.insert(t.id.clone()) {
                        out.push(t);
                    }
                }
            }

            // Collect positive artists & genres (excluding disliked/skipped tracks) so catalog tracks
            // by known/related/followed artists, local files, and matching genres beat recent charts.
            if out.len() < limit {
                let mut disliked_ids: HashSet<String> = HashSet::new();
                {
                    let mut dstmt = conn.prepare(
                        "SELECT track_id FROM ratings WHERE user_id = ?1 AND rating <= 2
                         UNION
                         SELECT track_id FROM user_track_stats
                           WHERE user_id = ?1 AND skip_count >= play_count + 2
                         UNION
                         SELECT track_id FROM hidden_tracks WHERE user_id = ?1",
                    )?;
                    for id in dstmt.query_map(params![user_id], |r| r.get::<_, String>(0))?.flatten() {
                        disliked_ids.insert(id);
                    }
                }

                let mut artist_pats: Vec<String> = Vec::new();
                let mut seen_art = HashSet::new();
                let mut push_artist_pat = |raw: &str| {
                    let clean = raw.trim().to_lowercase();
                    if clean.len() >= 2 && clean != "unknown artist" && seen_art.insert(clean.clone()) {
                        let escaped = clean
                            .replace('\\', "\\\\")
                            .replace('%', "\\%")
                            .replace('_', "\\_");
                        artist_pats.push(format!("%{}%", escaped));
                    }
                };

                let mut genre_pats: Vec<String> = Vec::new();
                let mut seen_gen = HashSet::new();
                for t in &out {
                    if disliked_ids.contains(&t.id) {
                        continue;
                    }
                    let pa = primary_artist(&t.artist);
                    if !pa.is_empty() {
                        push_artist_pat(&pa);
                    }
                    for g in t.genre.split([',', ';', '/']) {
                        let gl = g.trim().to_lowercase();
                        if gl.len() >= 2 && seen_gen.insert(gl.clone()) {
                            let escaped = gl
                                .replace('\\', "\\\\")
                                .replace('%', "\\%")
                                .replace('_', "\\_");
                            genre_pats.push(format!("%{}%", escaped));
                        }
                    }
                }

                {
                    let mut fstmt = conn.prepare(
                        "SELECT COALESCE(NULLIF(a.name, ''), f.artist_id)
                         FROM followed_artists f
                         LEFT JOIN artists_meta a ON a.id = f.artist_id
                         WHERE f.user_id = ?1
                         ORDER BY f.at DESC LIMIT 12",
                    )?;
                    for raw in fstmt.query_map(params![user_id], |r| r.get::<_, String>(0))?.flatten() {
                        if raw.starts_with("ar:dz:") || raw.starts_with("ar:ytm:") {
                            continue;
                        }
                        let clean = raw.trim_start_matches("ar:").replace('-', " ");
                        push_artist_pat(&clean);
                    }
                }
                {
                    let mut rastmt = conn.prepare(
                        "SELECT key FROM user_taste_links
                         WHERE user_id = ?1 AND kind = 'related_artist'
                         ORDER BY weight DESC, updated DESC LIMIT 16",
                    )?;
                    for k in rastmt.query_map(params![user_id], |r| r.get::<_, String>(0))?.flatten() {
                        let words = k.trim_start_matches("ar:").replace('-', " ");
                        push_artist_pat(&words);
                    }
                }

                artist_pats.truncate(24);
                genre_pats.truncate(12);

                // Query 2a: same/related artists + user's local library (only when user has taste signals)
                if !out.is_empty() || !artist_pats.is_empty() || !genre_pats.is_empty() {
                    let rem = (limit - out.len()).min(limit / 2 + 200);
                    let mut clauses = Vec::with_capacity(artist_pats.len() + 1);
                    for idx in 0..artist_pats.len() {
                        clauses.push(format!("lower(artist) LIKE ?{} ESCAPE '\\'", idx + 1));
                    }
                    clauses.push("source = 'local'".to_string());
                    let lim_param_idx = artist_pats.len() + 1;
                    let aff_sql = format!(
                        "SELECT {} FROM tracks WHERE ({}) ORDER BY created DESC LIMIT ?{}",
                        TRACK_COLS,
                        clauses.join(" OR "),
                        lim_param_idx
                    );
                    let mut values: Vec<Box<dyn ToSql>> = artist_pats
                        .into_iter()
                        .map(|p| Box::new(p) as Box<dyn ToSql>)
                        .collect();
                    values.push(Box::new(rem as i64));
                    let mut astmt = conn.prepare(&aff_sql)?;
                    for t in astmt
                        .query_map(params_from_iter(values.iter().map(|v| v.as_ref())), row_to_track)?
                        .collect::<Result<Vec<_>, _>>()?
                    {
                        if out.len() >= limit {
                            break;
                        }
                        if seen.insert(t.id.clone()) {
                            out.push(t);
                        }
                    }
                }

                // Query 2b: catalog tracks matching positive user genres
                if out.len() < limit && !genre_pats.is_empty() {
                    let rem_g = (limit - out.len()).min(limit / 3 + 150);
                    let gclauses: Vec<String> = (0..genre_pats.len())
                        .map(|idx| format!("lower(genre) LIKE ?{} ESCAPE '\\'", idx + 1))
                        .collect();
                    let gsql = format!(
                        "SELECT {} FROM tracks WHERE ({}) ORDER BY created DESC LIMIT ?{}",
                        TRACK_COLS,
                        gclauses.join(" OR "),
                        genre_pats.len() + 1
                    );
                    let mut gvals: Vec<Box<dyn ToSql>> = genre_pats
                        .into_iter()
                        .map(|p| Box::new(p) as Box<dyn ToSql>)
                        .collect();
                    gvals.push(Box::new(rem_g as i64));
                    let mut gstmt = conn.prepare(&gsql)?;
                    for t in gstmt
                        .query_map(params_from_iter(gvals.iter().map(|v| v.as_ref())), row_to_track)?
                        .collect::<Result<Vec<_>, _>>()?
                    {
                        if out.len() >= limit {
                            break;
                        }
                        if seen.insert(t.id.clone()) {
                            out.push(t);
                        }
                    }
                }
            }
        }

        if out.len() < limit {
            let remaining = limit - out.len();
            let chart_sql = format!(
                "SELECT {} FROM chart_catalog c
                 JOIN tracks t ON t.id = c.track_id
                 ORDER BY c.rank_pos ASC, c.updated DESC
                 LIMIT ?1",
                tcols
            );
            let mut chart_stmt = conn.prepare(&chart_sql)?;
            let mut chart_added = 0usize;
            for t in chart_stmt
                .query_map(params![remaining as i64], row_to_track)?
                .collect::<Result<Vec<_>, _>>()?
            {
                chart_added += 1;
                if out.len() >= limit {
                    break;
                }
                if seen.insert(t.id.clone()) {
                    out.push(t);
                }
            }

            // Only fall back to arbitrary recent catalog tracks if chart_catalog is not yet populated
            // (e.g., offline first boot or unit tests), so user-cached searches/listens never leak across users.
            let chart_total: i64 = conn
                .query_row("SELECT COUNT(*) FROM chart_catalog", [], |r| r.get(0))
                .unwrap_or(0);
            if chart_total < 20 && chart_added == 0 && out.len() < limit {
                let rem2 = limit - out.len();
                let cat_sql = format!(
                    "SELECT {} FROM tracks ORDER BY created DESC LIMIT ?1",
                    TRACK_COLS
                );
                let mut cstmt = conn.prepare(&cat_sql)?;
                for t in cstmt
                    .query_map(params![rem2 as i64], row_to_track)?
                    .collect::<Result<Vec<_>, _>>()?
                {
                    if out.len() >= limit {
                        break;
                    }
                    if seen.insert(t.id.clone()) {
                        out.push(t);
                    }
                }
            }
        }

        Ok(out)
    }

    pub fn record_chart_tracks(&self, tracks: &[Track], base_pos: usize) -> AppResult<()> {
        if tracks.is_empty() {
            return Ok(());
        }
        let now = Utc::now().timestamp();
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO chart_catalog(track_id, rank_pos, updated) VALUES (?1, ?2, ?3)
                 ON CONFLICT(track_id) DO UPDATE SET
                   rank_pos = CASE
                     WHEN (?3 - chart_catalog.updated) > 21600 THEN excluded.rank_pos
                     ELSE MIN(chart_catalog.rank_pos, excluded.rank_pos)
                   END,
                   updated = excluded.updated",
            )?;
            for (idx, t) in tracks.iter().enumerate() {
                if t.id.trim().is_empty() {
                    continue;
                }
                let pos = (base_pos + idx) as i64;
                stmt.execute(params![t.id, pos, now])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn popular_chart_tracks(&self, limit: usize) -> AppResult<Vec<Track>> {
        let conn = self.lock();
        let tcols = prefixed_track_cols("t");
        let sql = format!(
            "SELECT {} FROM chart_catalog c
             JOIN tracks t ON t.id = c.track_id
             ORDER BY c.rank_pos ASC, c.updated DESC
             LIMIT ?1",
            tcols
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![limit as i64], row_to_track)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn newest_tracks(&self, limit: usize, offset: usize) -> AppResult<Vec<Track>> {
        self.all_tracks(limit, offset)
    }

    pub fn random_tracks(&self, limit: usize, genre: Option<&str>) -> AppResult<Vec<Track>> {
        let conn = self.lock();
        let g = genre.map(|s| s.trim()).filter(|s| !s.is_empty());
        if let Some(gen) = g {
            let pat = format!("%{}%", gen.to_lowercase());
            let sql = format!(
                "SELECT {} FROM tracks WHERE lower(genre) LIKE ?1 ORDER BY RANDOM() LIMIT ?2",
                TRACK_COLS
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt
                .query_map(params![pat, limit as i64], row_to_track)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        } else {
            let sql = format!(
                "SELECT {} FROM tracks ORDER BY RANDOM() LIMIT ?1",
                TRACK_COLS
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt
                .query_map(params![limit as i64], row_to_track)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        }
    }

    pub fn upsert_album_meta(&self, meta: &AlbumSummary) -> AppResult<()> {
        let conn = self.lock();
        let now = Utc::now().timestamp();
        conn.execute(
            "INSERT INTO albums_meta(id, name, artist, artist_id, cover_url, genre, year, label, deezer_id, updated)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(id) DO UPDATE SET
               name = CASE WHEN excluded.name != '' THEN excluded.name ELSE albums_meta.name END,
               artist = CASE WHEN excluded.artist != '' THEN excluded.artist ELSE albums_meta.artist END,
               artist_id = CASE WHEN excluded.artist_id != '' THEN excluded.artist_id ELSE albums_meta.artist_id END,
               cover_url = CASE WHEN excluded.cover_url != '' THEN excluded.cover_url ELSE albums_meta.cover_url END,
               genre = CASE WHEN excluded.genre != '' THEN excluded.genre ELSE albums_meta.genre END,
               year = CASE WHEN excluded.year > 0 THEN excluded.year ELSE albums_meta.year END,
               label = CASE WHEN excluded.label != '' THEN excluded.label ELSE albums_meta.label END,
               deezer_id = CASE WHEN excluded.deezer_id != '' THEN excluded.deezer_id ELSE albums_meta.deezer_id END,
               updated = excluded.updated",
            params![
                meta.id,
                meta.name,
                meta.artist,
                meta.artist_id,
                meta.cover_url,
                meta.genre,
                meta.year,
                meta.label,
                meta.deezer_id,
                now
            ],
        )?;
        Ok(())
    }

    pub fn get_album_meta(&self, id: &str) -> AppResult<Option<AlbumSummary>> {
        let conn = self.lock();
        let row = conn
            .query_row(
                "SELECT id, name, artist, artist_id, cover_url, genre, year, label, deezer_id FROM albums_meta WHERE id = ?1",
                params![id],
                |r| {
                    let name: String = r.get(1)?;
                    let artist: String = r.get(2)?;
                    let stored_artist_id: String = r.get(3)?;
                    let artist_id = if stored_artist_id.trim().is_empty() {
                        format!("ar:{}", slug(&primary_artist(&artist)))
                    } else {
                        stored_artist_id
                    };
                    Ok(AlbumSummary {
                        id: r.get(0)?,
                        name,
                        artist_id,
                        artist,
                        cover_url: r.get(4)?,
                        genre: r.get(5)?,
                        year: r.get(6)?,
                        label: r.get(7)?,
                        deezer_id: r.get(8)?,
                        ..Default::default()
                    })
                },
            )
            .optional()?;
        Ok(row)
    }

    pub fn upsert_artist_meta(&self, meta: &ArtistSummary) -> AppResult<()> {
        let conn = self.lock();
        let now = Utc::now().timestamp();
        let dz_clean = meta.deezer_id.trim();
        let canonical_id = if !dz_clean.is_empty() && !meta.id.starts_with("ar:dz:") {
            format!("ar:dz:{}", dz_clean)
        } else {
            meta.id.clone()
        };
        conn.execute(
            "INSERT INTO artists_meta(id, name, cover_url, bio, deezer_id, updated)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET
               name = CASE WHEN excluded.name != '' THEN excluded.name ELSE artists_meta.name END,
               cover_url = CASE WHEN excluded.cover_url != '' THEN excluded.cover_url ELSE artists_meta.cover_url END,
               bio = CASE WHEN excluded.bio != '' THEN excluded.bio ELSE artists_meta.bio END,
               deezer_id = CASE WHEN excluded.deezer_id != '' THEN excluded.deezer_id ELSE artists_meta.deezer_id END,
               updated = excluded.updated",
            params![
                canonical_id,
                meta.name,
                meta.cover_url,
                meta.bio,
                meta.deezer_id,
                now
            ],
        )?;
        Ok(())
    }

    pub fn get_artist_meta(&self, id: &str) -> AppResult<Option<ArtistSummary>> {
        let conn = self.lock();
        let row = conn
            .query_row(
                "SELECT id, name, cover_url, bio, deezer_id FROM artists_meta WHERE id = ?1",
                params![id],
                |r| {
                    Ok(ArtistSummary {
                        id: r.get(0)?,
                        name: r.get(1)?,
                        cover_url: r.get(2)?,
                        bio: r.get(3)?,
                        deezer_id: r.get(4)?,
                        album_count: 0,
                        song_count: 0,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }

    fn all_album_metas_map(&self) -> AppResult<HashMap<String, AlbumSummary>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT id, name, artist, artist_id, cover_url, genre, year, label, deezer_id FROM albums_meta",
        )?;
        let mut map = HashMap::new();
        let rows = stmt.query_map([], |r| {
            let name: String = r.get(1)?;
            let artist: String = r.get(2)?;
            let stored_artist_id: String = r.get(3)?;
            let artist_id = if stored_artist_id.trim().is_empty() {
                format!("ar:{}", slug(&primary_artist(&artist)))
            } else {
                stored_artist_id
            };
            Ok(AlbumSummary {
                id: r.get(0)?,
                name,
                artist_id,
                artist,
                cover_url: r.get(4)?,
                genre: r.get(5)?,
                year: r.get(6)?,
                label: r.get(7)?,
                deezer_id: r.get(8)?,
                ..Default::default()
            })
        })?;
        for r in rows.flatten() {
            map.insert(r.id.clone(), r);
        }
        Ok(map)
    }

    fn all_artist_metas_map(&self) -> AppResult<HashMap<String, ArtistSummary>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT id, name, cover_url, bio, deezer_id FROM artists_meta",
        )?;
        let mut map = HashMap::new();
        let rows = stmt.query_map([], |r| {
            Ok(ArtistSummary {
                id: r.get(0)?,
                name: r.get(1)?,
                cover_url: r.get(2)?,
                bio: r.get(3)?,
                deezer_id: r.get(4)?,
                album_count: 0,
                song_count: 0,
            })
        })?;
        for r in rows.flatten() {
            map.insert(r.id.clone(), r);
        }
        Ok(map)
    }

    pub fn build_album_summaries(&self, user_id: i64) -> AppResult<Vec<AlbumSummary>> {
        let tracks = self.all_tracks(8000, 0)?;
        let stats = if user_id > 0 {
            self.track_stats_map(user_id)?
        } else {
            HashMap::new()
        };
        let metas = self.all_album_metas_map()?;
        let artist_metas = self.all_artist_metas_map()?;
        let artist_resolver = ArtistResolver::build(&tracks, &artist_metas);
        let mut map: HashMap<String, AlbumSummary> = HashMap::new();

        for t in tracks {
            let aid = t.album_id();
            let resolved_artist_id = artist_resolver.resolve_track(&t);
            let entry = map.entry(aid.clone()).or_insert_with(|| {
                let name = if t.album.trim().is_empty() {
                    t.title.clone()
                } else {
                    t.album.clone()
                };
                AlbumSummary {
                    id: aid.clone(),
                    name,
                    artist: t.artist.clone(),
                    artist_id: resolved_artist_id,
                    cover_id: t.id.clone(),
                    cover_url: t.cover_url.clone(),
                    genre: t.genre.clone(),
                    year: t.year,
                    label: String::new(),
                    deezer_id: String::new(),
                    song_count: 0,
                    duration: 0,
                    play_count: 0,
                    last_played: 0,
                    created: t.created,
                }
            });
            entry.song_count += 1;
            entry.duration += t.duration.max(0);
            if entry.cover_url.is_empty() && !t.cover_url.is_empty() {
                entry.cover_url = t.cover_url.clone();
                entry.cover_id = t.id.clone();
            }
            if entry.genre.is_empty() && !t.genre.is_empty() {
                entry.genre = t.genre.clone();
            }
            if entry.year == 0 && t.year > 0 {
                entry.year = t.year;
            }
            if t.created > entry.created {
                entry.created = t.created;
            }
            if let Some(st) = stats.get(&t.id) {
                entry.play_count += st.play_count as i64;
                if st.last_played > entry.last_played {
                    entry.last_played = st.last_played;
                }
            }
        }

        for (id, meta) in metas {
            let raw_artist_id = if meta.artist_id.trim().is_empty() {
                format!("ar:{}", slug(&primary_artist(&meta.artist)))
            } else {
                meta.artist_id.clone()
            };
            let resolved_artist_id = artist_resolver.resolve(&raw_artist_id, &meta.artist);
            let entry = map.entry(id.clone()).or_insert_with(|| AlbumSummary {
                id: id.clone(),
                name: meta.name.clone(),
                artist: meta.artist.clone(),
                artist_id: resolved_artist_id.clone(),
                cover_id: id,
                cover_url: meta.cover_url.clone(),
                genre: meta.genre.clone(),
                year: meta.year,
                label: meta.label.clone(),
                deezer_id: meta.deezer_id.clone(),
                ..Default::default()
            });
            if resolved_artist_id.starts_with("ar:dz:")
                && !entry.artist_id.starts_with("ar:dz:")
            {
                entry.artist_id = resolved_artist_id;
            }
            if !meta.cover_url.is_empty() {
                entry.cover_url = meta.cover_url;
            }
            if !meta.label.is_empty() {
                entry.label = meta.label;
            }
            if !meta.deezer_id.is_empty() {
                entry.deezer_id = meta.deezer_id;
            }
            if entry.genre.is_empty() && !meta.genre.is_empty() {
                entry.genre = meta.genre;
            }
            if entry.year == 0 && meta.year > 0 {
                entry.year = meta.year;
            }
        }

        let mut list: Vec<AlbumSummary> = map.into_values().collect();
        list.sort_by(|a, b| b.created.cmp(&a.created).then_with(|| a.name.cmp(&b.name)));
        Ok(list)
    }

    pub fn album_summary_by_id(
        &self,
        user_id: i64,
        album_id: &str,
    ) -> AppResult<Option<AlbumSummary>> {
        let summaries = self.build_album_summaries(user_id)?;
        if let Some(exact) = summaries
            .iter()
            .find(|a| a.id == album_id || (!a.deezer_id.is_empty() && a.deezer_id == album_id))
        {
            return Ok(Some(exact.clone()));
        }
        let matched_tracks = self.album_tracks(album_id).unwrap_or_default();
        if let Some(first) = matched_tracks.first() {
            let canon_alb_id = first.album_id();
            let alb_name = if first.album.trim().is_empty() {
                &first.title
            } else {
                &first.album
            };
            let alb_slug = slug(alb_name);
            let pa_slug = slug(&primary_artist(&first.artist));
            if let Some(found) = summaries.into_iter().find(|a| {
                a.id == canon_alb_id
                    || (slug(&a.name) == alb_slug
                        && !pa_slug.is_empty()
                        && slug(&primary_artist(&a.artist)) == pa_slug)
            }) {
                return Ok(Some(found));
            }
        }
        Ok(None)
    }

    pub fn album_tracks(&self, album_id: &str) -> AppResult<Vec<Track>> {
        let dz_album_id = self
            .get_album_meta(album_id)
            .ok()
            .flatten()
            .map(|m| m.deezer_id)
            .filter(|s| !s.is_empty());
        let all = self.all_tracks(8000, 0)?;
        let artist_metas = self.all_artist_metas_map()?;
        let resolver = ArtistResolver::build(&all, &artist_metas);

        let mut matched_artist_album: Option<(String, String)> = None;
        for t in &all {
            if t.matches_album_id(album_id)
                || (dz_album_id.is_some() && t.deezer_album_id() == dz_album_id)
            {
                let alb_name = if t.album.trim().is_empty() {
                    &t.title
                } else {
                    &t.album
                };
                matched_artist_album = Some((resolver.resolve_track(t), slug(alb_name)));
                break;
            }
        }

        let mut tracks: Vec<Track> = all
            .into_iter()
            .filter(|t| {
                if t.matches_album_id(album_id)
                    || (dz_album_id.is_some() && t.deezer_album_id() == dz_album_id)
                {
                    return true;
                }
                let alb_name = if t.album.trim().is_empty() {
                    &t.title
                } else {
                    &t.album
                };
                let alb_slug = slug(alb_name);
                let res_art = resolver.resolve_track(t);
                if let Some(dz_aid) = res_art.strip_prefix("ar:dz:") {
                    if format!("al:dz-{}-{}", dz_aid, alb_slug) == album_id {
                        return true;
                    }
                }
                if let Some((ref want_art, ref want_alb)) = matched_artist_album {
                    if !want_alb.is_empty() && &alb_slug == want_alb && &res_art == want_art {
                        return true;
                    }
                }
                false
            })
            .collect();
        tracks.sort_by(|a, b| {
            a.disc_no
                .cmp(&b.disc_no)
                .then_with(|| a.track_no.cmp(&b.track_no))
                .then_with(|| b.quality_score().cmp(&a.quality_score()))
                .then_with(|| a.title.cmp(&b.title))
        });
        // Collapse exact duplicate song_keys within the same album keeping highest quality.
        let mut seen = HashSet::new();
        let mut out = Vec::with_capacity(tracks.len());
        for t in tracks {
            let key = format!("{}:{}", t.disc_no, t.song_key());
            if seen.insert(key) {
                out.push(t);
            }
        }
        Ok(out)
    }

    pub fn build_artist_summaries(&self) -> AppResult<Vec<ArtistSummary>> {
        let tracks = self.all_tracks(8000, 0)?;
        let metas = self.all_artist_metas_map()?;
        let resolver = ArtistResolver::build(&tracks, &metas);
        let mut albums_per_artist: HashMap<String, HashSet<String>> = HashMap::new();
        let mut map: HashMap<String, ArtistSummary> = HashMap::new();

        for t in tracks {
            let arid = resolver.resolve_track(&t);
            albums_per_artist
                .entry(arid.clone())
                .or_default()
                .insert(t.album_id());
            let entry = map.entry(arid.clone()).or_insert_with(|| ArtistSummary {
                id: arid.clone(),
                name: crate::models::primary_artist(&t.artist),
                cover_url: t.cover_url.clone(),
                bio: String::new(),
                deezer_id: t.deezer_artist_id().unwrap_or_default(),
                album_count: 0,
                song_count: 0,
            });
            if entry.name.is_empty() {
                entry.name = t.artist.clone();
            }
            if entry.deezer_id.is_empty() {
                if let Some(dz_id) = t.deezer_artist_id() {
                    entry.deezer_id = dz_id;
                }
            }
            entry.song_count += 1;
            if entry.cover_url.is_empty() && !t.cover_url.is_empty() {
                entry.cover_url = t.cover_url.clone();
            }
        }

        for (arid, set) in albums_per_artist {
            if let Some(a) = map.get_mut(&arid) {
                a.album_count = set.len();
            }
        }

        for (id, meta) in metas {
            let canonical_id = resolver.resolve_meta(&id, &meta);
            let is_canonical = canonical_id == id;
            let entry = map.entry(canonical_id.clone()).or_insert_with(|| ArtistSummary {
                id: canonical_id,
                name: meta.name.clone(),
                cover_url: meta.cover_url.clone(),
                bio: meta.bio.clone(),
                deezer_id: meta.deezer_id.clone(),
                album_count: 0,
                song_count: 0,
            });
            if is_canonical {
                if !meta.name.is_empty() {
                    entry.name = meta.name;
                }
                if !meta.cover_url.is_empty() {
                    entry.cover_url = meta.cover_url;
                }
                if !meta.bio.is_empty() {
                    entry.bio = meta.bio;
                }
                if !meta.deezer_id.is_empty() {
                    entry.deezer_id = meta.deezer_id;
                }
            } else {
                if entry.name.is_empty() && !meta.name.is_empty() {
                    entry.name = meta.name;
                }
                if entry.cover_url.is_empty() && !meta.cover_url.is_empty() {
                    entry.cover_url = meta.cover_url;
                }
                if entry.bio.is_empty() && !meta.bio.is_empty() {
                    entry.bio = meta.bio;
                }
                if entry.deezer_id.is_empty() && !meta.deezer_id.is_empty() {
                    entry.deezer_id = meta.deezer_id;
                }
            }
        }

        let mut list: Vec<ArtistSummary> = map.into_values().collect();
        list.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(list)
    }

    pub fn featured_artist_summaries(&self, limit: usize) -> AppResult<Vec<ArtistSummary>> {
        if limit == 0 {
            return Ok(vec![]);
        }
        let mut ranked: Vec<(f64, ArtistSummary)> = self
            .build_artist_summaries()?
            .into_iter()
            .filter(|a| !is_junk_featured_artist_name(&a.name))
            .map(|a| {
                let mut score = 0.0f64;
                if !a.cover_url.trim().is_empty() {
                    score += 100.0;
                }
                if !a.deezer_id.trim().is_empty() {
                    score += 40.0;
                }
                if let Some(fans_str) = a.bio.split_whitespace().next() {
                    if let Ok(fans) = fans_str.parse::<u64>() {
                        if fans > 0 {
                            score += (fans as f64).log10() * 12.0;
                        }
                    }
                }
                score += (a.song_count.min(25) as f64) * 2.0;
                score += (a.album_count.min(10) as f64) * 3.0;
                (score, a)
            })
            .collect();
        ranked.sort_by(|a, b| {
            b.0.partial_cmp(&a.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.1.song_count.cmp(&a.1.song_count))
                .then_with(|| a.1.name.to_lowercase().cmp(&b.1.name.to_lowercase()))
        });
        Ok(ranked.into_iter().take(limit).map(|(_, a)| a).collect())
    }

    pub fn artist_summary_by_id(&self, artist_id: &str) -> AppResult<Option<ArtistSummary>> {
        let tracks = self.all_tracks(8000, 0)?;
        let metas = self.all_artist_metas_map()?;
        let resolver = ArtistResolver::build(&tracks, &metas);
        let mut canonical_id = resolver.resolve(artist_id, "");
        if canonical_id == artist_id {
            if let Some(t) = tracks.iter().find(|t| t.matches_artist_id(artist_id)) {
                canonical_id = resolver.resolve_track(t);
            }
        }
        let summaries = self.build_artist_summaries()?;
        if let Some(exact) = summaries.iter().find(|a| {
            a.id == artist_id
                || a.id == canonical_id
                || (!a.deezer_id.is_empty() && a.deezer_id == artist_id)
        }) {
            return Ok(Some(exact.clone()));
        }
        if !artist_id.starts_with("ar:dz:") && !artist_id.starts_with("ar:ytm:") {
            let target_slug = artist_id.strip_prefix("ar:").unwrap_or(artist_id);
            let matches: Vec<&ArtistSummary> = summaries
                .iter()
                .filter(|a| slug(&a.name) == target_slug)
                .collect();
            if matches.len() == 1 {
                return Ok(Some(matches[0].clone()));
            }
        }
        Ok(None)
    }

    pub fn artist_tracks(&self, artist_id: &str) -> AppResult<Vec<Track>> {
        let all = self.all_tracks(8000, 0)?;
        let metas = self.all_artist_metas_map()?;
        let resolver = ArtistResolver::build(&all, &metas);
        let mut canonical_id = resolver.resolve(artist_id, "");
        if canonical_id == artist_id {
            if let Some(t) = all.iter().find(|t| t.matches_artist_id(artist_id)) {
                canonical_id = resolver.resolve_track(t);
            }
        }
        let mut tracks: Vec<Track> = all
            .into_iter()
            .filter(|t| {
                t.matches_artist_id(artist_id)
                    || t.matches_artist_id(&canonical_id)
                    || resolver.resolve_track(t) == artist_id
                    || resolver.resolve_track(t) == canonical_id
            })
            .collect();
        if tracks.is_empty() {
            if let Some(ext_id) = artist_id
                .strip_prefix("ar:ytm:")
                .or_else(|| artist_id.strip_prefix("ar:dz:"))
                .filter(|s| !s.trim().is_empty())
            {
                let conn = self.lock();
                let pat = format!("%{}%", ext_id.trim());
                let sql = format!(
                    "SELECT {} FROM tracks WHERE meta LIKE ?1 ORDER BY created DESC LIMIT 100",
                    TRACK_COLS
                );
                if let Ok(mut stmt) = conn.prepare(&sql) {
                    if let Ok(rows) = stmt.query_map(params![pat], row_to_track) {
                        for t in rows.flatten() {
                            if t.matches_artist_id(artist_id) {
                                tracks.push(t);
                            }
                        }
                    }
                };
            }
        }
        tracks.sort_by(|a, b| {
            b.quality_score()
                .cmp(&a.quality_score())
                .then_with(|| b.year.cmp(&a.year))
                .then_with(|| a.title.cmp(&b.title))
        });
        Ok(tracks)
    }

    pub fn artist_albums(
        &self,
        user_id: i64,
        artist_id: &str,
    ) -> AppResult<Vec<AlbumSummary>> {
        let mut albums: Vec<AlbumSummary> = self
            .build_album_summaries(user_id)?
            .into_iter()
            .filter(|a| a.artist_id == artist_id)
            .collect();
        albums.sort_by(|a, b| b.year.cmp(&a.year).then_with(|| a.name.cmp(&b.name)));
        Ok(albums)
    }

    pub fn genres(&self) -> AppResult<Vec<(String, usize, usize)>> {
        let tracks = self.all_tracks(8000, 0)?;
        let mut song_counts: HashMap<String, usize> = HashMap::new();
        let mut album_sets: HashMap<String, HashSet<String>> = HashMap::new();
        for t in tracks {
            for raw in t.genre.split(&[',', ';', '/'][..]) {
                let g = raw.trim();
                if g.is_empty() {
                    continue;
                }
                let norm = title_case(g);
                *song_counts.entry(norm.clone()).or_insert(0) += 1;
                album_sets.entry(norm).or_default().insert(t.album_id());
            }
        }
        let mut out: Vec<(String, usize, usize)> = song_counts
            .into_iter()
            .map(|(g, sc)| {
                let ac = album_sets.get(&g).map(|s| s.len()).unwrap_or(0);
                (g, sc, ac)
            })
            .collect();
        out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        Ok(out)
    }

    pub fn tracks_by_genre(
        &self,
        genre: &str,
        limit: usize,
        offset: usize,
    ) -> AppResult<Vec<Track>> {
        let conn = self.lock();
        let pat = format!("%{}%", genre.trim().to_lowercase());
        let sql = format!(
            "SELECT {} FROM tracks WHERE lower(genre) LIKE ?1 ORDER BY created DESC LIMIT ?2 OFFSET ?3",
            TRACK_COLS
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![pat, limit as i64, offset as i64], row_to_track)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn tracks_by_mood(
        &self,
        mood: Mood,
        limit: usize,
        offset: usize,
    ) -> AppResult<Vec<Track>> {
        let conn = self.lock();
        let pat = format!("%,{},%", mood.slug());
        let sql = format!(
            "SELECT {} FROM tracks
             WHERE (',' || lower(moods) || ',') LIKE ?1
             ORDER BY
               (CASE WHEN trim(genre) != '' THEN 2 ELSE 0 END)
               + (CASE WHEN source = 'local' THEN 2 WHEN source = 'deezer' THEN 1 ELSE 0 END) DESC,
               created DESC
             LIMIT ?2 OFFSET ?3",
            TRACK_COLS
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![pat, limit as i64, offset as i64], row_to_track)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Returns a comma-joined map of known non-empty genres keyed by lowercase `primary_artist`
    /// for the requested artist names so mood validation can catch off-genre search tracks.
    pub fn known_artist_genres_for(&self, artists: &[String]) -> AppResult<HashMap<String, String>> {
        let wanted: HashSet<String> = artists
            .iter()
            .map(|a| primary_artist(a).to_lowercase())
            .filter(|k| !k.is_empty())
            .collect();
        if wanted.is_empty() {
            return Ok(HashMap::new());
        }
        let conn = self.lock();
        artist_genre_index_locked(&conn, Some(&wanted))
    }

    pub fn search_tracks(
        &self,
        query: &str,
        limit: usize,
        offset: usize,
    ) -> AppResult<Vec<Track>> {
        self.search_tracks_filtered(query, None, limit, offset)
    }

    pub fn search_local_tracks(
        &self,
        query: &str,
        limit: usize,
        offset: usize,
    ) -> AppResult<Vec<Track>> {
        self.search_tracks_filtered(query, Some("local"), limit, offset)
    }

    fn search_tracks_filtered(
        &self,
        query: &str,
        source_filter: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> AppResult<Vec<Track>> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return match source_filter {
                Some(src) => {
                    let conn = self.lock();
                    let sql = format!(
                        "SELECT {} FROM tracks WHERE source = ?1 ORDER BY created DESC LIMIT ?2 OFFSET ?3",
                        TRACK_COLS
                    );
                    let mut stmt = conn.prepare(&sql)?;
                    let rows = stmt
                        .query_map(params![src, limit as i64, offset as i64], row_to_track)?
                        .collect::<Result<Vec<_>, _>>()?;
                    Ok(rows)
                }
                None => self.all_tracks(limit, offset),
            };
        }

        let tokens: Vec<String> = q
            .split_whitespace()
            .filter(|s| !s.is_empty())
            .take(8)
            .map(|tok| {
                let escaped = tok
                    .replace('\\', "\\\\")
                    .replace('%', "\\%")
                    .replace('_', "\\_");
                format!("%{}%", escaped)
            })
            .collect();

        let candidates = {
            let conn = self.lock();
            let mut clauses = Vec::new();
            let mut param_vals: Vec<rusqlite::types::Value> = Vec::new();
            if let Some(src) = source_filter {
                clauses.push(format!("source = ?{}", param_vals.len() + 1));
                param_vals.push(rusqlite::types::Value::Text(src.to_string()));
            }
            for pat in &tokens {
                let idx = param_vals.len() + 1;
                clauses.push(format!(
                    "(LOWER(title) LIKE ?{idx} ESCAPE '\\' OR LOWER(artist) LIKE ?{idx} ESCAPE '\\' OR LOWER(album) LIKE ?{idx} ESCAPE '\\' OR LOWER(genre) LIKE ?{idx} ESCAPE '\\')"
                ));
                param_vals.push(rusqlite::types::Value::Text(pat.clone()));
            }
            let where_sql = if clauses.is_empty() {
                String::new()
            } else {
                format!("WHERE {}", clauses.join(" AND "))
            };
            let sql = format!(
                "SELECT {} FROM tracks {} ORDER BY created DESC LIMIT 1500",
                TRACK_COLS, where_sql
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt
                .query_map(rusqlite::params_from_iter(param_vals), row_to_track)?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };

        let mut scored: Vec<(i32, Track)> = candidates
            .into_iter()
            .filter_map(|t| {
                let s = search_match_score(&t, &q);
                (s > 0).then_some((s, t))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0));
        Ok(scored
            .into_iter()
            .skip(offset)
            .take(limit)
            .map(|(_, t)| t)
            .collect())
    }

    pub fn search_albums(
        &self,
        user_id: i64,
        query: &str,
        limit: usize,
        offset: usize,
    ) -> AppResult<Vec<AlbumSummary>> {
        let q = query.trim().to_lowercase();
        let all = self.build_album_summaries(user_id)?;
        let filtered: Vec<AlbumSummary> = if q.is_empty() {
            all
        } else {
            all.into_iter()
                .filter(|a| {
                    a.name.to_lowercase().contains(&q) || a.artist.to_lowercase().contains(&q)
                })
                .collect()
        };
        Ok(filtered.into_iter().skip(offset).take(limit).collect())
    }

    pub fn search_artists(
        &self,
        query: &str,
        limit: usize,
        offset: usize,
    ) -> AppResult<Vec<ArtistSummary>> {
        let q = query.trim().to_lowercase();
        let all = self.build_artist_summaries()?;
        let filtered: Vec<ArtistSummary> = if q.is_empty() {
            all
        } else {
            all.into_iter()
                .filter(|a| a.name.to_lowercase().contains(&q))
                .collect()
        };
        Ok(filtered.into_iter().skip(offset).take(limit).collect())
    }

    // -------------------------------------------------------------------------
    // Per-User Listening Stats, Events, Stars, Saved Albums, Followed Artists
    // -------------------------------------------------------------------------

    pub fn record_event(
        &self,
        user_id: i64,
        track_id: &str,
        kind: &str,
        value: f64,
    ) -> AppResult<()> {
        let now = Utc::now().timestamp();
        let safe_val = if value.is_finite() { value } else { 0.0 };
        let conn = self.lock();
        conn.execute(
            "INSERT INTO events(user_id, track_id, kind, value, at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![user_id, track_id, kind, safe_val, now],
        )?;
        match kind {
            "play" | "complete" => {
                conn.execute(
                    "INSERT INTO user_track_stats(user_id, track_id, play_count, skip_count, last_played)
                     VALUES (?1, ?2, 1, 0, ?3)
                     ON CONFLICT(user_id, track_id) DO UPDATE SET
                       play_count = play_count + 1,
                       last_played = excluded.last_played",
                    params![user_id, track_id, now],
                )?;
            }
            "skip" => {
                conn.execute(
                    "INSERT INTO user_track_stats(user_id, track_id, play_count, skip_count, last_played)
                     VALUES (?1, ?2, 0, 1, ?3)
                     ON CONFLICT(user_id, track_id) DO UPDATE SET
                       skip_count = skip_count + 1,
                       last_played = CASE WHEN last_played < excluded.last_played THEN excluded.last_played ELSE last_played END",
                    params![user_id, track_id, now],
                )?;
            }
            _ => {}
        }
        Ok(())
    }

    pub fn track_stats_map(&self, user_id: i64) -> AppResult<HashMap<String, UserTrackStats>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT track_id, play_count, skip_count, last_played FROM user_track_stats WHERE user_id = ?1",
        )?;
        let rows = stmt.query_map(params![user_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                UserTrackStats {
                    play_count: r.get(1)?,
                    skip_count: r.get(2)?,
                    last_played: r.get(3)?,
                },
            ))
        })?;
        let mut map = HashMap::new();
        for item in rows.flatten() {
            map.insert(item.0, item.1);
        }
        Ok(map)
    }

    pub fn get_user_track_stats(
        &self,
        user_id: i64,
        track_id: &str,
    ) -> AppResult<UserTrackStats> {
        let conn = self.lock();
        let st = conn
            .query_row(
                "SELECT play_count, skip_count, last_played FROM user_track_stats WHERE user_id = ?1 AND track_id = ?2",
                params![user_id, track_id],
                |r| {
                    Ok(UserTrackStats {
                        play_count: r.get(0)?,
                        skip_count: r.get(1)?,
                        last_played: r.get(2)?,
                    })
                },
            )
            .optional()?
            .unwrap_or_default();
        Ok(st)
    }

    pub fn user_recent_tracks(
        &self,
        user_id: i64,
        limit: usize,
        offset: usize,
    ) -> AppResult<Vec<(Track, UserTrackStats)>> {
        let conn = self.lock();
        let sql = format!(
            "SELECT {}, uts.play_count, uts.skip_count, uts.last_played
             FROM user_track_stats uts
             JOIN tracks t ON t.id = uts.track_id
             WHERE uts.user_id = ?1 AND uts.play_count > 0
             ORDER BY uts.last_played DESC
             LIMIT ?2 OFFSET ?3",
            prefixed_track_cols("t")
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![user_id, limit as i64, offset as i64], |r| {
                let t = row_to_track(r)?;
                let st = UserTrackStats {
                    play_count: r.get(21)?,
                    skip_count: r.get(22)?,
                    last_played: r.get(23)?,
                };
                Ok((t, st))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn user_frequent_tracks(
        &self,
        user_id: i64,
        limit: usize,
        offset: usize,
    ) -> AppResult<Vec<(Track, UserTrackStats)>> {
        let conn = self.lock();
        let sql = format!(
            "SELECT {}, uts.play_count, uts.skip_count, uts.last_played
             FROM user_track_stats uts
             JOIN tracks t ON t.id = uts.track_id
             WHERE uts.user_id = ?1 AND uts.play_count > 0
             ORDER BY uts.play_count DESC, uts.last_played DESC
             LIMIT ?2 OFFSET ?3",
            prefixed_track_cols("t")
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![user_id, limit as i64, offset as i64], |r| {
                let t = row_to_track(r)?;
                let st = UserTrackStats {
                    play_count: r.get(21)?,
                    skip_count: r.get(22)?,
                    last_played: r.get(23)?,
                };
                Ok((t, st))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn user_recent_events(&self, user_id: i64, limit: usize) -> AppResult<Vec<Event>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT track_id, kind, value, at FROM events WHERE user_id = ?1 ORDER BY at DESC LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![user_id, limit as i64], |r| {
                Ok(Event {
                    track_id: r.get(0)?,
                    kind: r.get(1)?,
                    value: r.get(2)?,
                    at: r.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn user_events_with_tracks(&self, user_id: i64) -> AppResult<Vec<UserEventRow>> {
        if user_id <= 0 {
            return Ok(Vec::new());
        }
        let conn = self.lock();
        let sql = format!(
            "SELECT {}, e.track_id, e.kind, e.value, e.at
             FROM events e
             LEFT JOIN tracks t ON t.id = e.track_id
             WHERE e.user_id = ?1
             ORDER BY e.at ASC, e.id ASC",
            prefixed_track_cols("t")
        );
        let mut stmt = conn.prepare(&sql)?;
        let mut rows = stmt
            .query_map(params![user_id], |r| {
                let has_track: Option<String> = r.get(0)?;
                let track = if has_track.is_some() {
                    Some(row_to_track(r)?)
                } else {
                    None
                };
                Ok(UserEventRow {
                    track_id: r.get(21)?,
                    kind: r.get(22)?,
                    value: r.get(23)?,
                    at: r.get(24)?,
                    track,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        if rows.is_empty() {
            return Ok(rows);
        }

        // Cross-reference catalog metadata (sibling tracks, albums_meta, artists_meta, artist genres)
        // so streamed tracks from YTM / Deezer / Soulseek inherit known genres, release years,
        // moods, and canonical artist IDs.
        let artist_genres = artist_genre_index_locked(&conn, None).unwrap_or_default();

        let mut album_by_id: HashMap<String, (String, i32)> = HashMap::new();
        let mut album_by_dz_id: HashMap<String, (String, i32)> = HashMap::new();
        let mut album_by_slug: HashMap<String, (String, i32)> = HashMap::new();
        if let Ok(mut astmt) = conn.prepare(
            "SELECT id, name, artist, genre, year, deezer_id FROM albums_meta WHERE trim(genre) != '' OR year > 0",
        ) {
            if let Ok(mut arows) = astmt.query([]) {
                while let Ok(Some(r)) = arows.next() {
                    let id: String = r.get(0).unwrap_or_default();
                    let name: String = r.get(1).unwrap_or_default();
                    let artist: String = r.get(2).unwrap_or_default();
                    let genre: String = r.get(3).unwrap_or_default();
                    let year: i32 = r.get(4).unwrap_or(0);
                    let dz_id: String = r.get(5).unwrap_or_default();
                    if !id.is_empty() {
                        album_by_id.insert(id, (genre.clone(), year));
                    }
                    if !dz_id.trim().is_empty() {
                        album_by_dz_id.insert(dz_id.trim().to_string(), (genre.clone(), year));
                    }
                    let pa = primary_artist(&artist);
                    let art_key = if pa.is_empty() { artist.trim() } else { pa.as_str() };
                    let alb_slug = slug(&format!("{}|{}", art_key, name));
                    if !alb_slug.is_empty() {
                        album_by_slug
                            .entry(alb_slug)
                            .and_modify(|(g, y)| {
                                if g.is_empty() && !genre.is_empty() {
                                    *g = genre.clone();
                                }
                                if *y <= 0 && year > 0 {
                                    *y = year;
                                }
                            })
                            .or_insert((genre, year));
                    }
                }
            }
        }

        let mut song_meta: HashMap<String, (String, i32, String, Option<String>)> = HashMap::new();
        let mut artist_dz_by_slug: HashMap<String, Option<String>> = HashMap::new();
        if let Ok(mut tstmt) = conn.prepare(
            "SELECT title, artist, album, genre, year, moods, meta FROM tracks WHERE trim(genre) != '' OR year > 0 OR trim(moods) != '' OR meta LIKE '%deezer_artist_id%'",
        ) {
            if let Ok(mut trows) = tstmt.query([]) {
                while let Ok(Some(r)) = trows.next() {
                    let title: String = r.get(0).unwrap_or_default();
                    let artist: String = r.get(1).unwrap_or_default();
                    let album: String = r.get(2).unwrap_or_default();
                    let genre: String = r.get(3).unwrap_or_default();
                    let year: i32 = r.get(4).unwrap_or(0);
                    let moods: String = r.get(5).unwrap_or_default();
                    let meta: String = r.get(6).unwrap_or_default();
                    let tmp = Track {
                        title,
                        artist: artist.clone(),
                        album: album.clone(),
                        genre: genre.clone(),
                        year,
                        moods: moods.clone(),
                        meta,
                        ..Default::default()
                    };
                    let dz_aid = tmp.deezer_artist_id();
                    let pa = primary_artist(&artist);
                    let art_key = if pa.is_empty() { artist.trim() } else { pa.as_str() };
                    if let Some(ref dz) = dz_aid {
                        let sl = slug(art_key);
                        if !sl.is_empty() {
                            artist_dz_by_slug
                                .entry(sl)
                                .and_modify(|cur| {
                                    if cur.as_deref() != Some(dz.as_str()) {
                                        *cur = None;
                                    }
                                })
                                .or_insert_with(|| Some(dz.clone()));
                        }
                    }
                    if !genre.trim().is_empty() || year > 0 {
                        let alb_slug = slug(&format!("{}|{}", art_key, album));
                        if !alb_slug.is_empty() {
                            album_by_slug
                                .entry(alb_slug)
                                .and_modify(|(g, y)| {
                                    if g.is_empty() && !genre.trim().is_empty() {
                                        *g = genre.clone();
                                    }
                                    if *y <= 0 && year > 0 {
                                        *y = year;
                                    }
                                })
                                .or_insert_with(|| (genre.clone(), year));
                        }
                    }
                    let sk = tmp.song_key();
                    song_meta
                        .entry(sk)
                        .and_modify(|(g, y, m, d)| {
                            if g.is_empty() && !genre.trim().is_empty() {
                                *g = genre.clone();
                            }
                            if *y <= 0 && year > 0 {
                                *y = year;
                            }
                            if m.is_empty() && !moods.trim().is_empty() {
                                *m = moods.clone();
                            }
                            if d.is_none() && dz_aid.is_some() {
                                *d = dz_aid.clone();
                            }
                        })
                        .or_insert((genre, year, moods, dz_aid));
                }
            }
        }

        for row in &mut rows {
            let Some(ref mut t) = row.track else {
                continue;
            };
            let sk = t.song_key();
            if let Some((sg, sy, sm, sdz)) = song_meta.get(&sk) {
                if t.genre.trim().is_empty() && !sg.trim().is_empty() {
                    t.genre = sg.clone();
                }
                if t.year <= 0 && *sy > 0 {
                    t.year = *sy;
                }
                if t.moods.trim().is_empty() && !sm.trim().is_empty() {
                    t.moods = sm.clone();
                }
                if t.deezer_artist_id().is_none() {
                    if let Some(ref dz) = sdz {
                        t.set_deezer_artist_id(dz);
                    }
                }
            }
            if t.deezer_artist_id().is_none() {
                let pa = primary_artist(&t.artist);
                let sl = slug(if pa.is_empty() { t.artist.trim() } else { pa.as_str() });
                if let Some(Some(dz)) = artist_dz_by_slug.get(&sl) {
                    t.set_deezer_artist_id(dz);
                }
            }
            if t.genre.trim().is_empty() || t.year <= 0 {
                let by_id = album_by_id.get(&t.album_id()).cloned();
                let by_dz = t
                    .deezer_album_id()
                    .and_then(|dz| album_by_dz_id.get(&dz).cloned());
                let pa = primary_artist(&t.artist);
                let art_key = if pa.is_empty() { t.artist.trim() } else { pa.as_str() };
                let by_slug = album_by_slug
                    .get(&slug(&format!("{}|{}", art_key, t.album)))
                    .cloned();
                for cand in [by_id, by_dz, by_slug].into_iter().flatten() {
                    if t.genre.trim().is_empty() && !cand.0.trim().is_empty() {
                        t.genre = cand.0;
                    }
                    if t.year <= 0 && cand.1 > 0 {
                        t.year = cand.1;
                    }
                }
            }
            let pa_low = primary_artist(&t.artist).to_lowercase();
            let artist_g = artist_genres.get(&pa_low).map(|s| s.as_str()).unwrap_or("");
            if t.genre.trim().is_empty() && !artist_g.trim().is_empty() {
                // Inherit the primary genre tag of the artist when the track itself was untagged.
                if let Some(first_g) = artist_g.split(',').map(str::trim).find(|s| !s.is_empty()) {
                    t.genre = first_g.to_string();
                }
            }
            if !t.genre.trim().is_empty() || !artist_g.trim().is_empty() {
                let classified =
                    crate::taste::classify_moods_str(&t.title, &t.album, &t.genre, artist_g);
                if !classified.is_empty() || t.moods.trim().is_empty() {
                    t.moods = classified;
                }
            }
        }

        Ok(rows)
    }

    /// Returns up to `limit` distinct tracks from a user's listening history that are missing
    /// `genre` or `year`, ordered by most recently played so stats enrichment can hydrate them.
    pub fn user_tracks_missing_metadata(&self, user_id: i64, limit: usize) -> AppResult<Vec<Track>> {
        if user_id <= 0 || limit == 0 {
            return Ok(Vec::new());
        }
        let conn = self.lock();
        let sql = format!(
            "SELECT {}
             FROM tracks t
             WHERE t.id IN (
                 SELECT track_id FROM events WHERE user_id = ?1 AND kind IN ('play', 'complete', 'skip')
                 UNION
                 SELECT track_id FROM user_track_stats WHERE user_id = ?1
             )
             AND (trim(t.genre) = '' OR t.year <= 0)
             ORDER BY t.created DESC
             LIMIT ?2",
            prefixed_track_cols("t")
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![user_id, limit as i64], row_to_track)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn star_track(&self, user_id: i64, track_id: &str) -> AppResult<()> {
        let now = Utc::now().timestamp();
        let conn = self.lock();
        conn.execute(
            "INSERT OR IGNORE INTO stars(user_id, track_id, at) VALUES (?1, ?2, ?3)",
            params![user_id, track_id, now],
        )?;
        conn.execute(
            "INSERT INTO events(user_id, track_id, kind, value, at) VALUES (?1, ?2, 'star', 1.0, ?3)",
            params![user_id, track_id, now],
        )?;
        Ok(())
    }

    pub fn unstar_track(&self, user_id: i64, track_id: &str) -> AppResult<()> {
        let now = Utc::now().timestamp();
        let conn = self.lock();
        conn.execute(
            "DELETE FROM stars WHERE user_id = ?1 AND track_id = ?2",
            params![user_id, track_id],
        )?;
        conn.execute(
            "INSERT INTO events(user_id, track_id, kind, value, at) VALUES (?1, ?2, 'unstar', 0.0, ?3)",
            params![user_id, track_id, now],
        )?;
        Ok(())
    }

    pub fn starred_ids(&self, user_id: i64) -> AppResult<HashSet<String>> {
        let conn = self.lock();
        let mut stmt = conn.prepare("SELECT track_id FROM stars WHERE user_id = ?1")?;
        let rows = stmt
            .query_map(params![user_id], |r| r.get::<_, String>(0))?
            .collect::<Result<HashSet<_>, _>>()?;
        Ok(rows)
    }

    pub fn starred_tracks(&self, user_id: i64) -> AppResult<Vec<(Track, i64)>> {
        let conn = self.lock();
        let sql = format!(
            "SELECT {}, s.at FROM stars s JOIN tracks t ON t.id = s.track_id WHERE s.user_id = ?1 ORDER BY s.at DESC",
            prefixed_track_cols("t")
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![user_id], |r| Ok((row_to_track(r)?, r.get(21)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn save_album(&self, user_id: i64, album_id: &str) -> AppResult<()> {
        let now = Utc::now().timestamp();
        let conn = self.lock();
        conn.execute(
            "INSERT OR IGNORE INTO saved_albums(user_id, album_id, at) VALUES (?1, ?2, ?3)",
            params![user_id, album_id, now],
        )?;
        Ok(())
    }

    pub fn unsave_album(&self, user_id: i64, album_id: &str) -> AppResult<()> {
        let conn = self.lock();
        conn.execute(
            "DELETE FROM saved_albums WHERE user_id = ?1 AND album_id = ?2",
            params![user_id, album_id],
        )?;
        Ok(())
    }

    pub fn saved_album_ids(&self, user_id: i64) -> AppResult<HashSet<String>> {
        let conn = self.lock();
        let mut stmt = conn.prepare("SELECT album_id FROM saved_albums WHERE user_id = ?1")?;
        let rows = stmt
            .query_map(params![user_id], |r| r.get::<_, String>(0))?
            .collect::<Result<HashSet<_>, _>>()?;
        Ok(rows)
    }

    pub fn follow_artist(&self, user_id: i64, artist_id: &str) -> AppResult<()> {
        let now = Utc::now().timestamp();
        let conn = self.lock();
        conn.execute(
            "INSERT OR IGNORE INTO followed_artists(user_id, artist_id, at) VALUES (?1, ?2, ?3)",
            params![user_id, artist_id, now],
        )?;
        Ok(())
    }

    pub fn unfollow_artist(&self, user_id: i64, artist_id: &str) -> AppResult<()> {
        let conn = self.lock();
        conn.execute(
            "DELETE FROM followed_artists WHERE user_id = ?1 AND artist_id = ?2",
            params![user_id, artist_id],
        )?;
        Ok(())
    }

    pub fn followed_artist_ids(&self, user_id: i64) -> AppResult<HashSet<String>> {
        let conn = self.lock();
        let mut stmt = conn.prepare("SELECT artist_id FROM followed_artists WHERE user_id = ?1")?;
        let rows = stmt
            .query_map(params![user_id], |r| r.get::<_, String>(0))?
            .collect::<Result<HashSet<_>, _>>()?;
        Ok(rows)
    }

    /// Artists the user cares about most, used to personalise the "New Releases"
    /// feed on the home tab. Explicitly followed artists always rank first; the
    /// rest come from listening history (play count + recency). Only artists we
    /// can look up on Deezer are returned so fresh releases can be fetched for
    /// them.
    pub fn relevant_artists_for_releases(
        &self,
        user_id: i64,
        limit: usize,
    ) -> AppResult<Vec<ArtistSummary>> {
        if user_id <= 0 {
            return Ok(Vec::new());
        }
        let followed = self.followed_artist_ids(user_id).unwrap_or_default();
        let mut score: HashMap<String, f64> = HashMap::new();
        // Followed artists get a strong, stable boost so they lead the list.
        for id in &followed {
            *score.entry(id.clone()).or_insert(0.0) += 1000.0;
        }

        // Artists from listening history, ranked by play count and recency.
        let listened = self.user_recent_tracks(user_id, 500, 0).unwrap_or_default();
        let mut names: HashMap<String, String> = HashMap::new();
        for (t, st) in listened {
            let aid = t.artist_id();
            let name = primary_artist(&t.artist);
            let name = if name.is_empty() { t.artist.clone() } else { name };
            names.entry(aid.clone()).or_insert(name);
            let plays = (st.play_count.max(0) as f64).ln_1p().min(8.0);
            let recency = ((st.last_played.max(0) as f64) / 86_400.0).ln_1p().min(6.0);
            *score.entry(aid).or_insert(0.0) += plays + recency;
        }

        let metas = self.all_artist_metas_map()?;
        let mut ranked: Vec<(f64, ArtistSummary)> = score
            .into_iter()
            .filter_map(|(id, s)| {
                let meta = metas.get(&id);
                let deezer_id = meta
                    .and_then(|m| {
                        let d = m.deezer_id.trim();
                        (!d.is_empty() && d != "0").then(|| d.to_string())
                    })
                    .or_else(|| {
                        id.strip_prefix("ar:dz:")
                            .and_then(|d| (!d.is_empty() && d != "0").then(|| d.to_string()))
                    })
                    .unwrap_or_default();
                if deezer_id.is_empty() {
                    return None;
                }
                let name = meta
                    .map(|m| m.name.clone())
                    .filter(|n| !n.is_empty())
                    .or_else(|| names.get(&id).cloned())
                    .filter(|n| !n.is_empty())
                    .unwrap_or_else(|| deezer_id.clone());
                let cover_url = meta.map(|m| m.cover_url.clone()).unwrap_or_default();
                Some((
                    s,
                    ArtistSummary {
                        id,
                        name,
                        cover_url,
                        bio: String::new(),
                        deezer_id,
                        album_count: 0,
                        song_count: 0,
                    },
                ))
            })
            .collect();
        ranked.sort_by(|a, b| {
            b.0.total_cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name))
        });
        ranked.truncate(limit);
        Ok(ranked.into_iter().map(|(_, a)| a).collect())
    }

    pub fn set_rating(&self, user_id: i64, track_id: &str, rating: i32) -> AppResult<()> {
        let now = Utc::now().timestamp();
        let clamped = rating.clamp(0, 5);
        let conn = self.lock();
        if clamped == 0 {
            conn.execute(
                "DELETE FROM ratings WHERE user_id = ?1 AND track_id = ?2",
                params![user_id, track_id],
            )?;
        } else {
            conn.execute(
                "INSERT INTO ratings(user_id, track_id, rating, at) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(user_id, track_id) DO UPDATE SET rating = excluded.rating, at = excluded.at",
                params![user_id, track_id, clamped, now],
            )?;
            conn.execute(
                "INSERT INTO events(user_id, track_id, kind, value, at) VALUES (?1, ?2, 'rate', ?3, ?4)",
                params![user_id, track_id, clamped as f64, now],
            )?;
        }
        Ok(())
    }

    pub fn user_ratings(&self, user_id: i64) -> AppResult<HashMap<String, i32>> {
        let conn = self.lock();
        let mut stmt = conn.prepare("SELECT track_id, rating FROM ratings WHERE user_id = ?1")?;
        let rows = stmt.query_map(params![user_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i32>(1)?))
        })?;
        let mut map = HashMap::new();
        for item in rows.flatten() {
            map.insert(item.0, item.1);
        }
        Ok(map)
    }

    pub fn hide_track(&self, user_id: i64, track_id: &str, song_key: &str) -> AppResult<()> {
        if user_id <= 0 || track_id.trim().is_empty() {
            return Ok(());
        }
        let now = Utc::now().timestamp();
        let conn = self.lock();
        conn.execute(
            "INSERT INTO hidden_tracks(user_id, track_id, song_key, at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(user_id, track_id) DO UPDATE SET
               song_key = CASE WHEN excluded.song_key != '' THEN excluded.song_key ELSE hidden_tracks.song_key END,
               at = excluded.at",
            params![user_id, track_id, song_key, now],
        )?;
        Ok(())
    }

    pub fn unhide_track(&self, user_id: i64, track_id: &str) -> AppResult<usize> {
        if user_id <= 0 || track_id.trim().is_empty() {
            return Ok(0);
        }
        let conn = self.lock();
        let key: Option<String> = conn
            .query_row(
                "SELECT song_key FROM hidden_tracks WHERE user_id = ?1 AND track_id = ?2",
                params![user_id, track_id],
                |r| r.get(0),
            )
            .optional()?
            .filter(|k: &String| !k.is_empty());

        let removed = if let Some(ref sk) = key {
            conn.execute(
                "DELETE FROM events WHERE user_id = ?1 AND kind = 'not_interested'
                 AND (track_id = ?2 OR track_id IN (SELECT track_id FROM hidden_tracks WHERE user_id = ?1 AND song_key = ?3))",
                params![user_id, track_id, sk],
            )?;
            conn.execute(
                "DELETE FROM hidden_tracks WHERE user_id = ?1 AND (track_id = ?2 OR song_key = ?3)",
                params![user_id, track_id, sk],
            )?
        } else {
            conn.execute(
                "DELETE FROM events WHERE user_id = ?1 AND kind = 'not_interested' AND track_id = ?2",
                params![user_id, track_id],
            )?;
            conn.execute(
                "DELETE FROM hidden_tracks WHERE user_id = ?1 AND track_id = ?2",
                params![user_id, track_id],
            )?
        };
        Ok(removed)
    }

    pub fn reset_hidden_tracks(&self, user_id: i64) -> AppResult<usize> {
        if user_id <= 0 {
            return Ok(0);
        }
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        let removed = tx.execute(
            "DELETE FROM hidden_tracks WHERE user_id = ?1",
            params![user_id],
        )?;
        tx.execute(
            "DELETE FROM events WHERE user_id = ?1 AND kind = 'not_interested'",
            params![user_id],
        )?;
        tx.commit()?;
        Ok(removed)
    }

    pub fn hidden_track_sets(&self, user_id: i64) -> AppResult<(HashSet<String>, HashSet<String>)> {
        let mut ids = HashSet::new();
        let mut keys = HashSet::new();
        if user_id <= 0 {
            return Ok((ids, keys));
        }
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT track_id, song_key FROM hidden_tracks WHERE user_id = ?1",
        )?;
        let rows = stmt.query_map(params![user_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        for row in rows.flatten() {
            ids.insert(row.0);
            if !row.1.is_empty() {
                keys.insert(row.1);
            }
        }
        Ok((ids, keys))
    }

    pub fn hidden_tracks_list(&self, user_id: i64) -> AppResult<Vec<(Track, i64)>> {
        if user_id <= 0 {
            return Ok(Vec::new());
        }
        let conn = self.lock();
        let sql = format!(
            "SELECT {}, h.at, h.track_id
             FROM hidden_tracks h
             LEFT JOIN tracks t ON t.id = h.track_id
             WHERE h.user_id = ?1
             ORDER BY h.at DESC",
            prefixed_track_cols("t")
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![user_id], |r| {
                let at: i64 = r.get(21)?;
                let raw_id: String = r.get(22)?;
                let has_track: Option<String> = r.get(0)?;
                let track = if has_track.is_some() {
                    row_to_track(r)?
                } else {
                    Track {
                        id: raw_id.clone(),
                        title: raw_id,
                        artist: "Unknown Artist".to_string(),
                        ..Default::default()
                    }
                };
                Ok((track, at))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn upsert_taste_links(&self, user_id: i64, links: &[(&str, &str, f64)]) -> AppResult<()> {
        if user_id <= 0 || links.is_empty() {
            return Ok(());
        }
        let now = Utc::now().timestamp();
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO user_taste_links(user_id, kind, key, weight, updated)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(user_id, kind, key) DO UPDATE SET
                   weight = CASE WHEN excluded.weight > user_taste_links.weight THEN excluded.weight ELSE user_taste_links.weight END,
                   updated = excluded.updated",
            )?;
            for &(kind, key, weight) in links {
                let k = key.trim();
                if k.is_empty() || !weight.is_finite() || weight <= 0.0 {
                    continue;
                }
                stmt.execute(params![user_id, kind, k, weight, now])?;
            }
            let mut prune = tx.prepare(
                "DELETE FROM user_taste_links
                 WHERE user_id = ?1 AND kind = ?2 AND key NOT IN (
                     SELECT key FROM user_taste_links
                     WHERE user_id = ?1 AND kind = ?2
                     ORDER BY updated DESC, weight DESC
                     LIMIT ?3
                 )",
            )?;
            prune.execute(params![user_id, "related_track", 450i64])?;
            prune.execute(params![user_id, "related_artist", 250i64])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn user_taste_links(
        &self,
        user_id: i64,
    ) -> AppResult<(HashMap<String, f64>, HashMap<String, f64>)> {
        let mut rel_artists = HashMap::new();
        let mut rel_tracks = HashMap::new();
        if user_id <= 0 {
            return Ok((rel_artists, rel_tracks));
        }
        let now = Utc::now().timestamp();
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT kind, key, weight, updated FROM user_taste_links WHERE user_id = ?1",
        )?;
        let rows = stmt.query_map(params![user_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, f64>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })?;
        for row in rows.flatten() {
            let (kind, key, weight, updated) = row;
            let age_days = ((now - updated).max(0) as f64) / 86400.0;
            let decayed = weight * (-age_days / 45.0).exp().max(0.35);
            match kind.as_str() {
                "related_artist" => {
                    rel_artists.insert(key, decayed);
                }
                "related_track" => {
                    rel_tracks.insert(key, decayed);
                }
                "neural_artist" => {
                    rel_artists
                        .entry(key)
                        .and_modify(|w| *w = (*w).max(decayed * 0.85))
                        .or_insert(decayed * 0.85);
                }
                "neural_track" => {
                    rel_tracks
                        .entry(key)
                        .and_modify(|w| *w = (*w).max(decayed * 0.85))
                        .or_insert(decayed * 0.85);
                }
                _ => {}
            }
        }
        Ok((rel_artists, rel_tracks))
    }

    /// Returns the top Transformer-distilled `neural_artist` slugs and weights for `user_id`.
    pub fn load_neural_artist_links(
        &self,
        user_id: i64,
        limit: usize,
    ) -> AppResult<Vec<(String, f64)>> {
        if user_id <= 0 || limit == 0 {
            return Ok(Vec::new());
        }
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT key, weight FROM user_taste_links
             WHERE user_id = ?1 AND kind = 'neural_artist'
             ORDER BY weight DESC, updated DESC
             LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![user_id, limit as i64], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?))
            })?
            .flatten()
            .collect();
        Ok(rows)
    }

    /// Returns true if any track in the catalog has a genre matching `term` (whole-word/tag).
    pub fn has_catalog_genre(&self, term: &str) -> bool {
        let low = term.trim().to_lowercase();
        if low.len() < 2 {
            return false;
        }
        let conn = self.lock();
        let like = format!("%{}%", low);
        let Ok(mut stmt) =
            conn.prepare("SELECT genre FROM tracks WHERE genre != '' AND lower(genre) LIKE ?1 LIMIT 25")
        else {
            return false;
        };
        let Ok(rows) = stmt.query_map(params![like], |r| r.get::<_, String>(0)) else {
            return false;
        };
        for g_str in rows.flatten() {
            for part in g_str.split([',', ';', '/']) {
                let p = part.trim().to_lowercase();
                if p == low
                    || p.split_whitespace().any(|w| w == low)
                    || (low.contains(' ') && p.contains(&low))
                {
                    return true;
                }
            }
        }
        false
    }

    /// Replaces the Transformer-distilled `neural_track` and `neural_artist` links for `user_id`
    /// without disturbing external `related_track`/`related_artist` graph links.
    pub fn replace_neural_taste_links(
        &self,
        user_id: i64,
        track_links: &[(String, f64)],
        artist_links: &[(String, f64)],
    ) -> AppResult<()> {
        if user_id <= 0 {
            return Ok(());
        }
        let now = Utc::now().timestamp();
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        {
            tx.execute(
                "DELETE FROM user_taste_links WHERE user_id = ?1 AND kind IN ('neural_track', 'neural_artist')",
                params![user_id],
            )?;
            let mut stmt = tx.prepare(
                "INSERT INTO user_taste_links(user_id, kind, key, weight, updated)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(user_id, kind, key) DO UPDATE SET
                   weight = excluded.weight,
                   updated = excluded.updated",
            )?;
            for (key, weight) in track_links {
                let k = key.trim();
                if !k.is_empty() && weight.is_finite() && *weight > 0.0 {
                    stmt.execute(params![user_id, "neural_track", k, *weight, now])?;
                }
            }
            for (key, weight) in artist_links {
                let k = key.trim();
                if !k.is_empty() && weight.is_finite() && *weight > 0.0 {
                    stmt.execute(params![user_id, "neural_artist", k, *weight, now])?;
                }
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Loads a compact chronological interaction sequence (track + event kind) for one user.
    pub fn load_user_neural_history(
        &self,
        user_id: i64,
        limit: usize,
    ) -> AppResult<Vec<(Track, String, i64)>> {
        if user_id <= 0 || limit == 0 {
            return Ok(Vec::new());
        }
        let conn = self.lock();
        let sql = format!(
            "SELECT {}, e.kind, e.at
             FROM events e
             JOIN tracks t ON t.id = e.track_id
             WHERE e.user_id = ?1
             ORDER BY e.at DESC, e.id DESC
             LIMIT ?2",
            prefixed_track_cols("t")
        );
        let mut stmt = conn.prepare(&sql)?;
        let mut rows = stmt
            .query_map(params![user_id, limit as i64], |r| {
                let t = row_to_track(r)?;
                let kind: String = r.get(21)?;
                let at: i64 = r.get(22)?;
                Ok((t, kind, at))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows.reverse();
        Ok(rows)
    }

    /// Loads multi-user training sequences, manual playlists, positive/negative signals,
    /// and a catalog track pool for the background Transformer trainer & distiller.
    pub fn load_transformer_corpus(
        &self,
        max_events_per_user: usize,
        catalog_limit: usize,
    ) -> AppResult<NeuralTrainingCorpus> {
        let conn = self.lock();
        let mut corpus = NeuralTrainingCorpus::default();

        let mut ustmt = conn.prepare("SELECT id FROM users ORDER BY id ASC")?;
        corpus.user_ids = ustmt
            .query_map([], |r| r.get::<_, i64>(0))?
            .collect::<Result<Vec<_>, _>>()?;

        let mut needed_ids: HashSet<String> = HashSet::new();

        for &uid in &corpus.user_ids {
            let mut estmt = conn.prepare(
                "SELECT track_id, kind, value, at FROM events
                 WHERE user_id = ?1
                 ORDER BY at DESC, id DESC
                 LIMIT ?2",
            )?;
            let mut evs: Vec<(String, String, f64, i64)> = estmt
                .query_map(params![uid, max_events_per_user as i64], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                })?
                .flatten()
                .collect();
            evs.reverse();
            for (tid, _, _, _) in &evs {
                needed_ids.insert(tid.clone());
            }
            if !evs.is_empty() {
                corpus.user_events.insert(uid, evs);
            }

            let mut sstmt = conn.prepare("SELECT track_id FROM stars WHERE user_id = ?1")?;
            let stars: Vec<String> = sstmt
                .query_map(params![uid], |r| r.get::<_, String>(0))?
                .flatten()
                .collect();
            for tid in &stars {
                needed_ids.insert(tid.clone());
            }
            if !stars.is_empty() {
                corpus.user_stars.insert(uid, stars);
            }

            let mut rstmt = conn.prepare("SELECT track_id, rating FROM ratings WHERE user_id = ?1")?;
            let ratings: Vec<(String, i32)> = rstmt
                .query_map(params![uid], |r| Ok((r.get(0)?, r.get(1)?)))?
                .flatten()
                .collect();
            for (tid, _) in &ratings {
                needed_ids.insert(tid.clone());
            }
            if !ratings.is_empty() {
                corpus.user_ratings.insert(uid, ratings);
            }

            let mut dstmt = conn.prepare(
                "SELECT track_id FROM hidden_tracks WHERE user_id = ?1
                 UNION
                 SELECT track_id FROM ratings WHERE user_id = ?1 AND rating <= 2
                 UNION
                 SELECT track_id FROM user_track_stats WHERE user_id = ?1 AND skip_count >= play_count + 2",
            )?;
            let dislikes: Vec<String> = dstmt
                .query_map(params![uid], |r| r.get::<_, String>(0))?
                .flatten()
                .collect();
            for tid in &dislikes {
                needed_ids.insert(tid.clone());
            }
            if !dislikes.is_empty() {
                corpus.user_disliked.insert(uid, dislikes);
            }
        }

        // Load all users' manual playlists for cross-user collaborative sequence/set learning.
        let mut pstmt = conn.prepare(
            "SELECT p.id, p.user_id, p.name, pt.track_id
             FROM playlists p
             JOIN playlist_tracks pt ON pt.playlist_id = p.id
             WHERE p.is_auto = 0
             ORDER BY p.id, pt.pos ASC",
        )?;
        let mut pl_map: HashMap<String, (i64, String, Vec<String>)> = HashMap::new();
        for row in pstmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })?
            .flatten()
        {
            let (pid, uid, name, tid) = row;
            needed_ids.insert(tid.clone());
            let entry = pl_map
                .entry(pid)
                .or_insert_with(|| (uid, name, Vec::new()));
            entry.2.push(tid);
        }
        corpus.playlists = pl_map
            .into_values()
            .filter(|(_, _, tids)| tids.len() >= 2)
            .collect();

        // Load recent catalog tracks for negative sampling and background user distillation.
        let cat_sql = format!(
            "SELECT {} FROM tracks ORDER BY created DESC LIMIT ?1",
            TRACK_COLS
        );
        let mut cstmt = conn.prepare(&cat_sql)?;
        for t in cstmt
            .query_map(params![catalog_limit as i64], row_to_track)?
            .flatten()
        {
            corpus.catalog_ids.push(t.id.clone());
            corpus.tracks.insert(t.id.clone(), t);
        }

        let missing: Vec<String> = needed_ids
            .into_iter()
            .filter(|id| !corpus.tracks.contains_key(id))
            .collect();
        for chunk in missing.chunks(250) {
            let placeholders: String = (1..=chunk.len())
                .map(|i| format!("?{}", i))
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!(
                "SELECT {} FROM tracks WHERE id IN ({})",
                TRACK_COLS, placeholders
            );
            let mut stmt = conn.prepare(&sql)?;
            let params_refs: Vec<&dyn ToSql> = chunk.iter().map(|s| s as &dyn ToSql).collect();
            for t in stmt
                .query_map(params_from_iter(params_refs), row_to_track)?
                .flatten()
            {
                corpus.tracks.insert(t.id.clone(), t);
            }
        }

        Ok(corpus)
    }

    // -------------------------------------------------------------------------
    // Playlists CRUD, Auto-Mixes & Play Queues
    // -------------------------------------------------------------------------

    pub fn create_playlist(
        &self,
        user_id: i64,
        name: &str,
        comment: &str,
        public: bool,
        track_ids: &[String],
    ) -> AppResult<Playlist> {
        let id = format!("pl-{}", Uuid::new_v4().simple());
        let now = Utc::now().timestamp();
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO playlists(id, user_id, name, comment, public, is_auto, kind, created, changed)
             VALUES (?1, ?2, ?3, ?4, ?5, 0, 'user', ?6, ?6)",
            params![id, user_id, name.trim(), comment, if public { 1 } else { 0 }, now],
        )?;
        for (idx, tid) in track_ids.iter().filter(|s| !s.trim().is_empty()).enumerate() {
            tx.execute(
                "INSERT INTO playlist_tracks(playlist_id, pos, track_id, added) VALUES (?1, ?2, ?3, ?4)",
                params![id, idx as i64, tid.trim(), now],
            )?;
        }
        tx.commit()?;
        drop(conn);
        self.get_playlist(&id)?
            .ok_or_else(|| err_msg("Failed to load created playlist"))
    }

    pub fn upsert_auto_playlist(
        &self,
        id: &str,
        user_id: i64,
        name: &str,
        comment: &str,
        kind: &str,
        track_ids: &[String],
    ) -> AppResult<()> {
        let now = Utc::now().timestamp();
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO playlists(id, user_id, name, comment, public, is_auto, kind, created, changed)
             VALUES (?1, ?2, ?3, ?4, 0, 1, ?5, ?6, ?6)
             ON CONFLICT(id) DO UPDATE SET
               name = excluded.name,
               comment = excluded.comment,
               kind = excluded.kind,
               changed = excluded.changed",
            params![id, user_id, name, comment, kind, now],
        )?;
        tx.execute(
            "DELETE FROM playlist_tracks WHERE playlist_id = ?1",
            params![id],
        )?;
        for (idx, tid) in track_ids.iter().filter(|s| !s.trim().is_empty()).enumerate() {
            tx.execute(
                "INSERT INTO playlist_tracks(playlist_id, pos, track_id, added) VALUES (?1, ?2, ?3, ?4)",
                params![id, idx as i64, tid.trim(), now],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn get_playlist(&self, id: &str) -> AppResult<Option<Playlist>> {
        let conn = self.lock();
        Ok(conn
            .query_row(
                "SELECT p.id, p.user_id, COALESCE(u.username, 'beatlab'), p.name, p.comment,
                        p.public, p.is_auto, p.kind, p.created, p.changed,
                        (SELECT COUNT(*) FROM playlist_tracks pt WHERE pt.playlist_id = p.id),
                        COALESCE((SELECT SUM(t.duration) FROM playlist_tracks pt JOIN tracks t ON t.id = pt.track_id WHERE pt.playlist_id = p.id), 0),
                        (SELECT pt.track_id FROM playlist_tracks pt WHERE pt.playlist_id = p.id ORDER BY pt.pos ASC LIMIT 1)
                 FROM playlists p
                 LEFT JOIN users u ON u.id = p.user_id
                 WHERE p.id = ?1",
                params![id],
                row_to_playlist,
            )
            .optional()?)
    }

    pub fn list_playlists(&self, user_id: i64, include_public: bool) -> AppResult<Vec<Playlist>> {
        let conn = self.lock();
        let sql = if include_public {
            "SELECT p.id, p.user_id, COALESCE(u.username, 'beatlab'), p.name, p.comment,
                    p.public, p.is_auto, p.kind, p.created, p.changed,
                    (SELECT COUNT(*) FROM playlist_tracks pt WHERE pt.playlist_id = p.id),
                    COALESCE((SELECT SUM(t.duration) FROM playlist_tracks pt JOIN tracks t ON t.id = pt.track_id WHERE pt.playlist_id = p.id), 0),
                    (SELECT pt.track_id FROM playlist_tracks pt WHERE pt.playlist_id = p.id ORDER BY pt.pos ASC LIMIT 1)
             FROM playlists p
             LEFT JOIN users u ON u.id = p.user_id
             WHERE p.user_id = ?1 OR p.public = 1
             ORDER BY p.is_auto ASC, p.changed DESC"
        } else {
            "SELECT p.id, p.user_id, COALESCE(u.username, 'beatlab'), p.name, p.comment,
                    p.public, p.is_auto, p.kind, p.created, p.changed,
                    (SELECT COUNT(*) FROM playlist_tracks pt WHERE pt.playlist_id = p.id),
                    COALESCE((SELECT SUM(t.duration) FROM playlist_tracks pt JOIN tracks t ON t.id = pt.track_id WHERE pt.playlist_id = p.id), 0),
                    (SELECT pt.track_id FROM playlist_tracks pt WHERE pt.playlist_id = p.id ORDER BY pt.pos ASC LIMIT 1)
             FROM playlists p
             LEFT JOIN users u ON u.id = p.user_id
             WHERE p.user_id = ?1
             ORDER BY p.is_auto ASC, p.changed DESC"
        };
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt
            .query_map(params![user_id], row_to_playlist)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn playlist_tracks(&self, playlist_id: &str) -> AppResult<Vec<Track>> {
        let conn = self.lock();
        let sql = format!(
            "SELECT {} FROM playlist_tracks pt JOIN tracks t ON t.id = pt.track_id WHERE pt.playlist_id = ?1 ORDER BY pt.pos ASC",
            prefixed_track_cols("t")
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![playlist_id], row_to_track)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Returns `(playlist_id, playlist_name, track)` across all non-auto playlists owned by `user_id`.
    pub fn user_manual_playlist_tracks(&self, user_id: i64) -> AppResult<Vec<(String, String, Track)>> {
        let conn = self.lock();
        let sql = format!(
            "SELECT {}, p.id, p.name
             FROM playlist_tracks pt
             JOIN playlists p ON p.id = pt.playlist_id
             JOIN tracks t ON t.id = pt.track_id
             WHERE p.user_id = ?1 AND p.is_auto = 0
             ORDER BY p.changed DESC, pt.pos ASC",
            prefixed_track_cols("t")
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![user_id], |r| {
                let t = row_to_track(r)?;
                let pid: String = r.get(21)?;
                let pname: String = r.get(22)?;
                Ok((pid, pname, t))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn update_playlist(
        &self,
        id: &str,
        name: Option<&str>,
        comment: Option<&str>,
        public: Option<bool>,
    ) -> AppResult<()> {
        let now = Utc::now().timestamp();
        let conn = self.lock();
        if let Some(n) = name.map(str::trim).filter(|s| !s.is_empty()) {
            conn.execute(
                "UPDATE playlists SET name = ?1, changed = ?2 WHERE id = ?3",
                params![n, now, id],
            )?;
        }
        if let Some(c) = comment {
            conn.execute(
                "UPDATE playlists SET comment = ?1, changed = ?2 WHERE id = ?3",
                params![c, now, id],
            )?;
        }
        if let Some(p) = public {
            conn.execute(
                "UPDATE playlists SET public = ?1, changed = ?2 WHERE id = ?3",
                params![if p { 1 } else { 0 }, now, id],
            )?;
        }
        Ok(())
    }

    pub fn add_tracks_to_playlist(&self, id: &str, track_ids: &[String]) -> AppResult<()> {
        if track_ids.is_empty() {
            return Ok(());
        }
        let now = Utc::now().timestamp();
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        let max_pos: i64 = tx.query_row(
            "SELECT COALESCE(MAX(pos), -1) FROM playlist_tracks WHERE playlist_id = ?1",
            params![id],
            |r| r.get(0),
        )?;
        let mut pos = max_pos + 1;
        for tid in track_ids.iter().map(|s| s.trim()).filter(|s| !s.is_empty()) {
            tx.execute(
                "INSERT INTO playlist_tracks(playlist_id, pos, track_id, added) VALUES (?1, ?2, ?3, ?4)",
                params![id, pos, tid, now],
            )?;
            pos += 1;
        }
        tx.execute(
            "UPDATE playlists SET changed = ?1 WHERE id = ?2",
            params![now, id],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Appends `track_ids` not already present in playlist `id` and returns the newly added IDs.
    pub fn add_unique_tracks_to_playlist(&self, id: &str, track_ids: &[String]) -> AppResult<Vec<String>> {
        if track_ids.is_empty() {
            return Ok(vec![]);
        }
        let now = Utc::now().timestamp();
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        let mut existing = HashSet::new();
        {
            let mut stmt = tx.prepare("SELECT track_id FROM playlist_tracks WHERE playlist_id = ?1")?;
            for tid in stmt.query_map(params![id], |r| r.get::<_, String>(0))?.flatten() {
                existing.insert(tid);
            }
        }
        let max_pos: i64 = tx.query_row(
            "SELECT COALESCE(MAX(pos), -1) FROM playlist_tracks WHERE playlist_id = ?1",
            params![id],
            |r| r.get(0),
        )?;
        let mut pos = max_pos + 1;
        let mut added = Vec::new();
        for tid in track_ids.iter().map(|s| s.trim()).filter(|s| !s.is_empty()) {
            if !existing.insert(tid.to_string()) {
                continue;
            }
            tx.execute(
                "INSERT INTO playlist_tracks(playlist_id, pos, track_id, added) VALUES (?1, ?2, ?3, ?4)",
                params![id, pos, tid, now],
            )?;
            pos += 1;
            added.push(tid.to_string());
        }
        if !added.is_empty() {
            tx.execute(
                "UPDATE playlists SET changed = ?1 WHERE id = ?2",
                params![now, id],
            )?;
        }
        tx.commit()?;
        Ok(added)
    }

    pub fn replace_playlist_tracks(&self, id: &str, track_ids: &[String]) -> AppResult<()> {
        let now = Utc::now().timestamp();
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM playlist_tracks WHERE playlist_id = ?1",
            params![id],
        )?;
        for (idx, tid) in track_ids.iter().filter(|s| !s.trim().is_empty()).enumerate() {
            tx.execute(
                "INSERT INTO playlist_tracks(playlist_id, pos, track_id, added) VALUES (?1, ?2, ?3, ?4)",
                params![id, idx as i64, tid.trim(), now],
            )?;
        }
        tx.execute(
            "UPDATE playlists SET changed = ?1 WHERE id = ?2",
            params![now, id],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn remove_playlist_positions(&self, id: &str, positions: &[usize]) -> AppResult<()> {
        if positions.is_empty() {
            return Ok(());
        }
        let current = self.playlist_tracks(id)?;
        let drop_set: HashSet<usize> = positions.iter().copied().collect();
        let kept: Vec<String> = current
            .into_iter()
            .enumerate()
            .filter(|(idx, _)| !drop_set.contains(idx))
            .map(|(_, t)| t.id)
            .collect();
        self.replace_playlist_tracks(id, &kept)
    }

    pub fn move_playlist_track(
        &self,
        id: &str,
        from_pos: usize,
        to_pos: usize,
    ) -> AppResult<()> {
        let mut tracks: Vec<String> = self
            .playlist_tracks(id)?
            .into_iter()
            .map(|t| t.id)
            .collect();
        if from_pos >= tracks.len() || to_pos >= tracks.len() {
            return Err(err_msg("Track position out of range"));
        }
        let item = tracks.remove(from_pos);
        tracks.insert(to_pos, item);
        self.replace_playlist_tracks(id, &tracks)
    }

    pub fn delete_playlist(&self, id: &str) -> AppResult<()> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM playlist_tracks WHERE playlist_id = ?1",
            params![id],
        )?;
        tx.execute("DELETE FROM playlists WHERE id = ?1", params![id])?;
        tx.commit()?;
        Ok(())
    }

    pub fn save_play_queue(&self, user_id: i64, state: &PlayQueueState) -> AppResult<()> {
        let now = Utc::now().timestamp();
        let json = serde_json::to_string(&state.track_ids)?;
        let conn = self.lock();
        conn.execute(
            "INSERT INTO play_queues(user_id, current_id, current_index, position_ms, track_ids, changed_by, updated)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(user_id) DO UPDATE SET
               current_id = excluded.current_id,
               current_index = excluded.current_index,
               position_ms = excluded.position_ms,
               track_ids = excluded.track_ids,
               changed_by = excluded.changed_by,
               updated = excluded.updated",
            params![
                user_id,
                state.current_id,
                state.current_index as i64,
                state.position_ms,
                json,
                state.changed_by,
                now
            ],
        )?;
        Ok(())
    }

    pub fn get_play_queue(&self, user_id: i64) -> AppResult<Option<PlayQueueState>> {
        let conn = self.lock();
        let row = conn
            .query_row(
                "SELECT current_id, current_index, position_ms, track_ids, changed_by, updated
                 FROM play_queues WHERE user_id = ?1",
                params![user_id],
                |r| {
                    let ids_json: String = r.get(3)?;
                    let track_ids: Vec<String> = serde_json::from_str(&ids_json).unwrap_or_default();
                    let cur_idx: i64 = r.get(1)?;
                    Ok(PlayQueueState {
                        current_id: r.get(0)?,
                        current_index: cur_idx.max(0) as usize,
                        position_ms: r.get(2)?,
                        track_ids,
                        changed_by: r.get(4)?,
                        updated: r.get(5)?,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }

    /// Loads the saved play queue along with surviving tracks and reconciles `current_index` and
    /// `current_id` so deletions from the `tracks` table never desynchronize the active track.
    pub fn load_play_queue_tracks(
        &self,
        user_id: i64,
    ) -> AppResult<Option<(PlayQueueState, Vec<Track>, usize, String)>> {
        let Some(q) = self.get_play_queue(user_id)? else {
            return Ok(None);
        };
        let tracks = self.get_tracks_by_ids(&q.track_ids)?;
        if tracks.is_empty() {
            return Ok(Some((q, tracks, 0, String::new())));
        }
        let resolved_idx = if !q.current_id.is_empty() {
            tracks
                .iter()
                .position(|t| t.id == q.current_id)
                .unwrap_or_else(|| q.current_index.min(tracks.len() - 1))
        } else {
            q.current_index.min(tracks.len() - 1)
        };
        let resolved_id = tracks[resolved_idx].id.clone();
        Ok(Some((q, tracks, resolved_idx, resolved_id)))
    }

    // -------------------------------------------------------------------------
    // Internet Radio Stations, Cache & KV
    // -------------------------------------------------------------------------

    pub fn list_radio_stations(&self, user_id: i64) -> AppResult<Vec<RadioStation>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT id, name, stream_url, home_page_url FROM radio_stations
             WHERE user_id = ?1 OR user_id = 0 ORDER BY name ASC",
        )?;
        let rows = stmt
            .query_map(params![user_id], |r| {
                Ok(RadioStation {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    stream_url: r.get(2)?,
                    home_page_url: r.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn upsert_radio_station(
        &self,
        user_id: i64,
        station: &RadioStation,
    ) -> AppResult<RadioStation> {
        let trimmed_id = station.id.trim();
        let id = if trimmed_id.is_empty() {
            format!("rad-{}", Uuid::new_v4().simple())
        } else {
            trimmed_id.to_string()
        };
        let conn = self.lock();
        if !trimmed_id.is_empty() {
            let existing_owner: Option<i64> = conn
                .query_row(
                    "SELECT user_id FROM radio_stations WHERE id = ?1",
                    params![&id],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(owner) = existing_owner {
                if owner != user_id {
                    return Err(err_msg("Permission denied"));
                }
            }
        }
        conn.execute(
            "INSERT INTO radio_stations(id, user_id, name, stream_url, home_page_url)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET
               name = excluded.name,
               stream_url = excluded.stream_url,
               home_page_url = excluded.home_page_url
             WHERE radio_stations.user_id = excluded.user_id",
            params![
                id,
                user_id,
                station.name.trim(),
                station.stream_url.trim(),
                station.home_page_url.trim()
            ],
        )?;
        Ok(RadioStation {
            id,
            name: station.name.trim().to_string(),
            stream_url: station.stream_url.trim().to_string(),
            home_page_url: station.home_page_url.trim().to_string(),
        })
    }

    pub fn delete_radio_station(&self, user_id: i64, id: &str) -> AppResult<()> {
        let conn = self.lock();
        let existing_owner: Option<i64> = conn
            .query_row(
                "SELECT user_id FROM radio_stations WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(owner) = existing_owner {
            if owner != user_id {
                return Err(err_msg("Permission denied"));
            }
            conn.execute(
                "DELETE FROM radio_stations WHERE id = ?1 AND user_id = ?2",
                params![id, user_id],
            )?;
        }
        Ok(())
    }

    // -- Optional S3/R2 cool cache registry ----------------------------------

    /// Audio rows keep their legacy bare track id; other asset kinds are
    /// prefixed so an embed/avatar sharing a track's id cannot overwrite it.
    fn cool_registry_id(kind: &str, asset_id: &str) -> String {
        if kind == "audio" {
            asset_id.to_string()
        } else {
            format!("{}:{}", kind, asset_id)
        }
    }

    /// Registers (or refreshes) any asset in the cool tier. `kind` namespaces
    /// audio / cover / embed / avatar objects so one durable tier can hold
    /// everything the app has cached.
    pub fn upsert_asset_cache_entry(
        &self,
        kind: &str,
        asset_id: &str,
        object_key: &str,
        bytes: i64,
    ) -> AppResult<()> {
        let conn = self.lock();
        let now = Utc::now().timestamp();
        conn.execute(
            "INSERT INTO remote_cache_entries(track_id, kind, object_key, bytes, created, last_accessed)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)
             ON CONFLICT(track_id) DO UPDATE SET
                 kind = excluded.kind, object_key = excluded.object_key,
                 bytes = excluded.bytes, created = excluded.created,
                 last_accessed = excluded.last_accessed",
            params![Self::cool_registry_id(kind, asset_id), kind, object_key, bytes, now],
        )?;
        Ok(())
    }

    /// Backwards-compatible audio wrapper.
    pub fn upsert_remote_cache_entry(
        &self,
        track_id: &str,
        object_key: &str,
        bytes: i64,
    ) -> AppResult<()> {
        self.upsert_asset_cache_entry("audio", track_id, object_key, bytes)
    }

    pub fn get_asset_cache_entry(
        &self,
        kind: &str,
        asset_id: &str,
    ) -> AppResult<Option<(String, i64)>> {
        let conn = self.lock();
        Ok(conn
            .query_row(
                "SELECT object_key, bytes FROM remote_cache_entries WHERE kind = ?1 AND track_id = ?2",
                params![kind, Self::cool_registry_id(kind, asset_id)],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }

    /// Backwards-compatible audio wrapper.
    pub fn get_remote_cache_entry(&self, track_id: &str) -> AppResult<Option<(String, i64)>> {
        self.get_asset_cache_entry("audio", track_id)
    }

    /// Refreshes the recency signal so frequently used assets survive eviction.
    pub fn touch_asset_cache_entry(&self, kind: &str, asset_id: &str) -> AppResult<()> {
        let conn = self.lock();
        conn.execute(
            "UPDATE remote_cache_entries SET last_accessed = ?1 WHERE kind = ?2 AND track_id = ?3",
            params![
                Utc::now().timestamp(),
                kind,
                Self::cool_registry_id(kind, asset_id)
            ],
        )?;
        Ok(())
    }

    /// Backwards-compatible audio wrapper.
    pub fn touch_remote_cache_entry(&self, track_id: &str) -> AppResult<()> {
        self.touch_asset_cache_entry("audio", track_id)
    }

    pub fn remote_cache_total_bytes(&self) -> AppResult<i64> {
        let conn = self.lock();
        Ok(conn.query_row(
            "SELECT COALESCE(SUM(bytes), 0) FROM remote_cache_entries",
            [],
            |r| r.get(0),
        )?)
    }

    pub fn remote_cache_count(&self) -> AppResult<usize> {
        let conn = self.lock();
        Ok(conn.query_row(
            "SELECT COUNT(*) FROM remote_cache_entries",
            [],
            |r| r.get::<_, i64>(0).map(|n| n as usize),
        )?)
    }

    /// Per-kind (audio / cover / embed / avatar) object counts and byte totals.
    pub fn remote_cache_kind_stats(&self) -> AppResult<Vec<(String, i64, i64)>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT kind, COUNT(*), COALESCE(SUM(bytes), 0)
             FROM remote_cache_entries GROUP BY kind ORDER BY SUM(bytes) DESC",
        )?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Least-recently-used first, for cool-cache quota eviction. Returns
    /// (kind, asset_id, object_key, bytes) so any asset kind can be evicted.
    pub fn remote_cache_eviction_candidates(
        &self,
        limit: usize,
    ) -> AppResult<Vec<(String, String, String, i64)>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT kind,
                    CASE WHEN kind = 'audio' THEN track_id
                         ELSE substr(track_id, length(kind) + 2) END AS asset_id,
                    object_key, bytes
             FROM remote_cache_entries ORDER BY last_accessed ASC LIMIT ?1",
        )?;
        let rows = stmt
            .query_map(params![limit as i64], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn delete_asset_cache_entry(&self, kind: &str, asset_id: &str) -> AppResult<()> {
        let conn = self.lock();
        conn.execute(
            "DELETE FROM remote_cache_entries WHERE kind = ?1 AND track_id = ?2",
            params![kind, Self::cool_registry_id(kind, asset_id)],
        )?;
        Ok(())
    }

    pub fn upsert_cache_entry(&self, track_id: &str, path: &str, bytes: i64) -> AppResult<()> {
        let now = Utc::now().timestamp();
        let conn = self.lock();
        conn.execute(
            "INSERT INTO cache_entries(track_id, path, bytes, last_accessed) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(track_id) DO UPDATE SET path = excluded.path, bytes = excluded.bytes, last_accessed = excluded.last_accessed",
            params![track_id, path, bytes, now],
        )?;
        Ok(())
    }

    pub fn list_cache_entries(&self) -> AppResult<Vec<(String, String, i64)>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT track_id, path, bytes FROM cache_entries ORDER BY last_accessed DESC",
        )?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn get_cache_entry(&self, track_id: &str) -> AppResult<Option<CacheEntry>> {
        let conn = self.lock();
        let row = conn
            .query_row(
                "SELECT path FROM cache_entries WHERE track_id = ?1",
                params![track_id],
                |r| Ok(CacheEntry { path: r.get(0)? }),
            )
            .optional()?;
        Ok(row)
    }

    pub fn touch_cache_entry(&self, track_id: &str) -> AppResult<()> {
        let now = Utc::now().timestamp();
        let conn = self.lock();
        conn.execute(
            "UPDATE cache_entries SET last_accessed = ?1 WHERE track_id = ?2",
            params![now, track_id],
        )?;
        Ok(())
    }

    pub fn delete_cache_entry(&self, track_id: &str) -> AppResult<()> {
        let conn = self.lock();
        conn.execute(
            "DELETE FROM cache_entries WHERE track_id = ?1",
            params![track_id],
        )?;
        Ok(())
    }

    pub fn cache_entry_count(&self) -> AppResult<usize> {
        let conn = self.lock();
        Ok(conn.query_row(
            "SELECT COUNT(*) FROM cache_entries",
            [],
            |r| r.get::<_, i64>(0).map(|n| n as usize),
        )?)
    }

    pub fn cache_total_bytes(&self) -> AppResult<i64> {
        let conn = self.lock();
        Ok(conn.query_row(
            "SELECT COALESCE(SUM(bytes), 0) FROM cache_entries",
            [],
            |r| r.get(0),
        )?)
    }

    pub fn prune_cache(&self, max_bytes: i64) -> AppResult<()> {
        let mut total = self.cache_total_bytes()?;
        if total <= max_bytes {
            return Ok(());
        }
        let entries: Vec<(String, String, i64)> = {
            let conn = self.lock();
            let mut stmt = conn.prepare(
                "SELECT track_id, path, bytes FROM cache_entries ORDER BY last_accessed ASC",
            )?;
            let rows = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };
        let mut removed = Vec::new();
        for (tid, path, bytes) in entries {
            if total <= max_bytes {
                break;
            }
            let _ = std::fs::remove_file(&path);
            removed.push(tid);
            total -= bytes;
        }
        if !removed.is_empty() {
            let mut conn = self.lock();
            let tx = conn.transaction()?;
            for tid in removed {
                tx.execute(
                    "DELETE FROM cache_entries WHERE track_id = ?1",
                    params![tid],
                )?;
            }
            tx.commit()?;
        }
        Ok(())
    }

    pub fn kv_get(&self, key: &str) -> Option<String> {
        let conn = self.lock();
        conn.query_row("SELECT value FROM kv WHERE key = ?1", params![key], |r| r.get(0))
            .optional()
            .ok()
            .flatten()
    }

    pub fn kv_set(&self, key: &str, value: &str) {
        let conn = self.lock();
        let _ = conn.execute(
            "INSERT INTO kv(key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        );
    }

    // -------------------------------------------------------------------------
    // Public Share Links
    // -------------------------------------------------------------------------

    pub fn create_or_get_share(
        &self,
        user_id: i64,
        kind: &str,
        target_id: &str,
        title: &str,
        subtitle: &str,
        track_ids: &[String],
    ) -> AppResult<ShareRecord> {
        let ids_json = serde_json::to_string(track_ids).unwrap_or_else(|_| "[]".to_string());
        let now = Utc::now().timestamp();
        let conn = self.lock();

        let existing_id: Option<String> = if kind == "mix" {
            conn.query_row(
                "SELECT id FROM shares
                 WHERE user_id = ?1 AND kind = 'mix' AND title = ?2 AND track_ids = ?3
                 LIMIT 1",
                params![user_id, title, ids_json],
                |r| r.get(0),
            )
            .optional()?
        } else {
            conn.query_row(
                "SELECT id FROM shares
                 WHERE user_id = ?1 AND kind = ?2 AND target_id = ?3
                 LIMIT 1",
                params![user_id, kind, target_id],
                |r| r.get(0),
            )
            .optional()?
        };

        let share_id = if let Some(id) = existing_id {
            conn.execute(
                "UPDATE shares SET title = ?1, subtitle = ?2, track_ids = ?3 WHERE id = ?4",
                params![title, subtitle, ids_json, id],
            )?;
            id
        } else {
            let mut id = Uuid::new_v4().simple().to_string()[..10].to_string();
            while conn
                .query_row("SELECT 1 FROM shares WHERE id = ?1", params![id], |r| {
                    r.get::<_, i64>(0)
                })
                .optional()?
                .is_some()
            {
                id = Uuid::new_v4().simple().to_string()[..10].to_string();
            }
            conn.execute(
                "INSERT INTO shares(id, user_id, kind, target_id, title, subtitle, track_ids, created)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![id, user_id, kind, target_id, title, subtitle, ids_json, now],
            )?;
            id
        };

        let creator_name: String = conn
            .query_row(
                "SELECT username FROM users WHERE id = ?1",
                params![user_id],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or_else(|| "Beatlab User".to_string());

        Ok(ShareRecord {
            id: share_id,
            user_id,
            creator_name,
            kind: kind.to_string(),
            target_id: target_id.to_string(),
            title: title.to_string(),
            subtitle: subtitle.to_string(),
            track_ids: track_ids.to_vec(),
            created: now,
        })
    }

    pub fn get_share(&self, id: &str) -> AppResult<Option<ShareRecord>> {
        let conn = self.lock();
        let row = conn
            .query_row(
                "SELECT s.id, s.user_id, COALESCE(u.username, 'Beatlab User'), s.kind, s.target_id, s.title, s.subtitle, s.track_ids, s.created
                 FROM shares s
                 LEFT JOIN users u ON u.id = s.user_id
                 WHERE s.id = ?1",
                params![id.trim()],
                |r| {
                    let ids_json: String = r.get(7)?;
                    let track_ids: Vec<String> =
                        serde_json::from_str(&ids_json).unwrap_or_default();
                    Ok(ShareRecord {
                        id: r.get(0)?,
                        user_id: r.get(1)?,
                        creator_name: r.get(2)?,
                        kind: r.get(3)?,
                        target_id: r.get(4)?,
                        title: r.get(5)?,
                        subtitle: r.get(6)?,
                        track_ids,
                        created: r.get(8)?,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }
}

fn configure_conn(conn: &Connection) -> AppResult<()> {
    conn.execute_batch(
        "
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;
        PRAGMA foreign_keys = ON;
        PRAGMA busy_timeout = 5000;
        ",
    )?;
    Ok(())
}

fn ensure_column(conn: &Connection, table: &str, col: &str, def: &str) -> AppResult<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({})", table))?;
    let cols: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    if !cols.iter().any(|c| c == col) {
        conn.execute(&format!("ALTER TABLE {} ADD COLUMN {} {}", table, col, def), [])?;
    }
    Ok(())
}

fn row_to_user(row: &Row) -> rusqlite::Result<User> {
    Ok(User {
        id: row.get(0)?,
        username: row.get(1)?,
        password: row.get(2)?,
        created: row.get(3)?,
    })
}

fn clip_str(s: &str, max_chars: usize) -> String {
    s.trim().chars().take(max_chars).collect()
}

fn is_hex_color(s: &str) -> bool {
    let s = s.trim();
    (s.len() == 4 || s.len() == 7)
        && s.starts_with('#')
        && s[1..].chars().all(|c| c.is_ascii_hexdigit())
}

fn avatar_color_for(id: i64) -> String {
    const PALETTE: [&str; 10] = [
        "#f43f5e", "#8b5cf6", "#3b82f6", "#06b6d4", "#10b981", "#f59e0b", "#ec4899", "#6366f1",
        "#14b8a6", "#f97316",
    ];
    PALETTE[(id.unsigned_abs() as usize) % PALETTE.len()].to_string()
}

/// Maps the PUBLIC_USER_SELECT projection (viewer id bound as ?1).
fn row_to_public_user(row: &Row) -> rusqlite::Result<PublicUser> {
    let id: i64 = row.get(0)?;
    let username: String = row.get(1)?;
    let display_name: String = row.get(3)?;
    let mut avatar_color: String = row.get(5)?;
    if avatar_color.trim().is_empty() {
        avatar_color = avatar_color_for(id);
    }
    let letter_source = if display_name.trim().is_empty() {
        &username
    } else {
        &display_name
    };
    let avatar_letter = letter_source
        .trim()
        .chars()
        .next()
        .unwrap_or('?')
        .to_uppercase()
        .to_string();
    let is_following: i64 = row.get(9)?;
    let avatar_version: i64 = row.get(10)?;
    let avatar_mime: String = row.get(11)?;
    // The picture itself is served from a dedicated endpoint. A monotonic
    // version acts as a cache-buster so replaced pictures refresh everywhere.
    let avatar_url = if avatar_mime.is_empty() {
        String::new()
    } else {
        format!("/api/web/social/avatar/{}?v={}", id, avatar_version)
    };
    Ok(PublicUser {
        id,
        username,
        display_name,
        bio: row.get(4)?,
        avatar_color,
        avatar_letter,
        avatar_url,
        location: row.get(6)?,
        created: row.get(2)?,
        followers_count: row.get(7)?,
        following_count: row.get(8)?,
        is_following: is_following != 0,
        is_self: false,
    })
}

fn row_to_track(row: &Row) -> rusqlite::Result<Track> {
    row_to_track_at(row, 0)
}

/// Read a Track from a row where the TRACK_COLS begin at `offset` (used when
/// joining recommendations, shares, etc. where other columns come first).
fn row_to_track_at(row: &Row, offset: usize) -> rusqlite::Result<Track> {
    Ok(Track {
        id: row.get(offset)?,
        source: row.get(offset + 1)?,
        title: row.get(offset + 2)?,
        artist: row.get(offset + 3)?,
        album: row.get(offset + 4)?,
        genre: row.get(offset + 5)?,
        moods: row.get(offset + 6)?,
        year: row.get(offset + 7)?,
        track_no: row.get(offset + 8)?,
        disc_no: row.get(offset + 9)?,
        duration: row.get(offset + 10)?,
        bitrate: row.get(offset + 11)?,
        sample_rate: row.get(offset + 12)?,
        bit_depth: row.get(offset + 13)?,
        format: row.get(offset + 14)?,
        size: row.get(offset + 15)?,
        stream_url: row.get(offset + 16)?,
        cover_url: row.get(offset + 17)?,
        meta: row.get(offset + 18)?,
        deezer_id: row.get(offset + 19)?,
        created: row.get(offset + 20)?,
    })
}

fn row_to_playlist(row: &Row) -> rusqlite::Result<Playlist> {
    let pub_int: i64 = row.get(5)?;
    let auto_int: i64 = row.get(6)?;
    let cnt: i64 = row.get(10)?;
    let dur: i64 = row.get(11)?;
    Ok(Playlist {
        id: row.get(0)?,
        user_id: row.get(1)?,
        owner_name: row.get(2)?,
        name: row.get(3)?,
        comment: row.get(4)?,
        public: pub_int != 0,
        is_auto: auto_int != 0,
        kind: row.get(7)?,
        created: row.get(8)?,
        changed: row.get(9)?,
        track_count: cnt.max(0) as usize,
        duration: dur.max(0) as i32,
        cover_track_id: row.get(12)?,
    })
}

pub fn search_match_score(t: &Track, query: &str) -> i32 {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return 1;
    }
    let tokens: Vec<&str> = q.split_whitespace().filter(|s| !s.is_empty()).collect();
    if tokens.is_empty() {
        return 1;
    }
    let tl = t.title.to_lowercase();
    let al = t.artist.to_lowercase();
    let abl = t.album.to_lowercase();
    let gl = t.genre.to_lowercase();
    let hay = format!("{} {} {} {}", tl, al, abl, gl);
    let matched = tokens.iter().filter(|tok| hay.contains(**tok)).count();
    if matched < tokens.len() {
        return 0;
    }

    let mut score = 50 + (t.quality_score() / 10);

    let artist_title = format!("{} {}", al, tl);
    let title_artist = format!("{} {}", tl, al);
    if tl == q || artist_title == q || title_artist == q {
        score += 550;
    } else if tl.starts_with(&q) {
        score += 320;
    } else if tl.contains(&q) {
        score += 180;
    }

    if al == q {
        score += 320;
    } else if al.contains(&q) {
        score += 150;
    } else if !al.is_empty() {
        // Reward artist+title combined queries (e.g. "daft punk one more time").
        let artist_words: Vec<&str> = al.split_whitespace().collect();
        if !artist_words.is_empty() && artist_words.iter().all(|w| tokens.contains(w)) {
            score += 240;
            let non_artist_tokens: Vec<&str> = tokens
                .iter()
                .copied()
                .filter(|tok| !artist_words.contains(tok))
                .collect();
            if !non_artist_tokens.is_empty() {
                let rem = non_artist_tokens.join(" ");
                let core_tl = crate::models::core_song_title(&t.title).to_lowercase();
                if tl == rem || core_tl == rem {
                    score += 360;
                } else if tl.starts_with(&rem) || core_tl.starts_with(&rem) {
                    score += 260;
                } else if non_artist_tokens.iter().all(|tok| tl.contains(*tok)) {
                    score += 160;
                }
            }
        }
    }

    if abl.contains(&q) {
        score += 80;
    }

    // Demote parody/karaoke/tribute/ringtone clutter unless explicitly searched.
    for junk in ["parody", "karaoke", "tribute", "made famous by", "ringtone", "8-bit"] {
        if (tl.contains(junk) || al.contains(junk)) && !q.contains(junk) {
            score -= 260;
        }
    }

    score.max(1)
}

fn artist_genre_index_locked(
    conn: &Connection,
    filter: Option<&HashSet<String>>,
) -> AppResult<HashMap<String, String>> {
    let mut by_artist: HashMap<String, Vec<String>> = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT artist, genre FROM tracks WHERE trim(genre) != ''
         UNION ALL
         SELECT artist, genre FROM albums_meta WHERE trim(genre) != ''",
    )?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let artist: String = row.get(0)?;
        let genre: String = row.get(1)?;
        let key = primary_artist(&artist).to_lowercase();
        if key.is_empty() {
            continue;
        }
        if let Some(wanted) = filter {
            if !wanted.contains(&key) {
                continue;
            }
        }
        let g = genre.trim();
        if !g.is_empty() {
            let entry = by_artist.entry(key).or_default();
            if !entry.iter().any(|existing| existing.eq_ignore_ascii_case(g)) {
                entry.push(g.to_string());
            }
        }
    }
    Ok(by_artist
        .into_iter()
        .map(|(k, v)| (k, v.join(", ")))
        .collect())
}

pub(crate) fn is_junk_featured_artist_name(name: &str) -> bool {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return true;
    }
    let low = trimmed.to_lowercase();
    if matches!(
        low.as_str(),
        "unknown" | "unknown artist" | "various" | "various artists" | "va" | "artist" | "none" | "null" | "untitled"
    ) {
        return true;
    }
    let stripped_bang = low.trim_start_matches('!').trim();
    if stripped_bang.is_empty() || stripped_bang == "artist" {
        return true;
    }
    if !trimmed.chars().any(|c| c.is_alphabetic()) {
        return true;
    }
    for bad in [
        "to sort",
        "unsorted",
        "new folder",
        "greatest hits",
        "super hits",
        "pop hits",
        "karaoke",
        "tribute",
        "cover band",
        "covers",
    ] {
        if low.contains(bad) {
            return true;
        }
    }
    if low.ends_with(" hits") {
        return true;
    }
    false
}

fn migrate_legacy_moods(conn: &mut Connection) -> AppResult<()> {
    let done: Option<String> = conn
        .query_row(
            "SELECT value FROM kv WHERE key = 'moods_classifier_v3'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    if done.is_some() {
        return Ok(());
    }

    let count: i64 = conn.query_row("SELECT COUNT(*) FROM tracks", [], |r| r.get(0))?;
    if count > 0 {
        let artist_genres = artist_genre_index_locked(conn, None)?;
        let mut updates: Vec<(String, String)> = Vec::new();
        {
            let mut stmt = conn.prepare("SELECT id, title, artist, album, genre, moods FROM tracks")?;
            let mut rows = stmt.query([])?;
            while let Some(row) = rows.next()? {
                let id: String = row.get(0)?;
                let title: String = row.get(1)?;
                let artist: String = row.get(2)?;
                let album: String = row.get(3)?;
                let genre: String = row.get(4)?;
                let old_moods: String = row.get(5)?;
                let extra_genres = if genre.trim().is_empty() {
                    artist_genres
                        .get(&primary_artist(&artist).to_lowercase())
                        .map(|s| s.as_str())
                        .unwrap_or("")
                } else {
                    ""
                };
                let new_moods = crate::taste::classify_moods_str(&title, &album, &genre, extra_genres);
                if new_moods != old_moods {
                    updates.push((new_moods, id));
                }
            }
        }
        if !updates.is_empty() {
            let tx = conn.transaction()?;
            {
                let mut upd = tx.prepare("UPDATE tracks SET moods = ?1 WHERE id = ?2")?;
                for (new_moods, id) in &updates {
                    upd.execute(params![new_moods, id])?;
                }
            }
            tx.commit()?;
        }
    }

    conn.execute("DELETE FROM kv WHERE key LIKE 'curated:%'", [])?;
    conn.execute(
        "INSERT OR REPLACE INTO kv(key, value) VALUES ('moods_classifier_v3', '1')",
        [],
    )?;
    Ok(())
}

struct ArtistResolver {
    ytm_to_dz: HashMap<String, Option<String>>,
    dz_exact: HashMap<String, Option<String>>,
    dz_slug: HashMap<String, Option<String>>,
    ytm_exact: HashMap<String, Option<String>>,
    ytm_slug: HashMap<String, Option<String>>,
}

impl ArtistResolver {
    fn record_unique(map: &mut HashMap<String, Option<String>>, key: String, val: &str) {
        if key.is_empty() || val.is_empty() {
            return;
        }
        map.entry(key)
            .and_modify(|cur| {
                if cur.as_deref() != Some(val) {
                    *cur = None;
                }
            })
            .or_insert_with(|| Some(val.to_string()));
    }

    fn record_artist(&mut self, id: &str, name: &str) {
        let pa = primary_artist(name);
        let key_name = if pa.is_empty() { name.trim() } else { pa.as_str() };
        if key_name.is_empty() {
            return;
        }
        let sl = slug(key_name);
        if id.starts_with("ar:dz:") {
            Self::record_unique(&mut self.dz_exact, key_name.to_string(), id);
            Self::record_unique(&mut self.dz_slug, sl, id);
        } else if id.starts_with("ar:ytm:") {
            Self::record_unique(&mut self.ytm_exact, key_name.to_string(), id);
            Self::record_unique(&mut self.ytm_slug, sl, id);
        }
    }

    fn build(tracks: &[Track], metas: &HashMap<String, ArtistSummary>) -> Self {
        let mut res = Self {
            ytm_to_dz: HashMap::new(),
            dz_exact: HashMap::new(),
            dz_slug: HashMap::new(),
            ytm_exact: HashMap::new(),
            ytm_slug: HashMap::new(),
        };

        for m in metas.values() {
            res.record_artist(&m.id, &m.name);
            let dz_clean = m.deezer_id.trim();
            if !dz_clean.is_empty() && !m.id.starts_with("ar:dz:") {
                let dz_ar_id = format!("ar:dz:{}", dz_clean);
                res.record_artist(&dz_ar_id, &m.name);
                if m.id.starts_with("ar:ytm:") {
                    Self::record_unique(&mut res.ytm_to_dz, m.id.clone(), &dz_ar_id);
                }
            }
        }

        for t in tracks {
            let pa = primary_artist(&t.artist);
            let name = if pa.is_empty() { t.artist.as_str() } else { pa.as_str() };
            if let Some(dz_id) = t.deezer_artist_id() {
                let dz_ar_id = format!("ar:dz:{}", dz_id);
                res.record_artist(&dz_ar_id, name);
                if let Some(ytm_id) = t.ytm_artist_id() {
                    let ytm_ar_id = format!("ar:ytm:{}", ytm_id);
                    Self::record_unique(&mut res.ytm_to_dz, ytm_ar_id, &dz_ar_id);
                }
            } else if let Some(ytm_id) = t.ytm_artist_id() {
                let ytm_ar_id = format!("ar:ytm:{}", ytm_id);
                res.record_artist(&ytm_ar_id, name);
            }
        }

        for (name_key, ytm_opt) in &res.ytm_exact {
            if let (Some(ytm_ar_id), Some(Some(dz_ar_id))) =
                (ytm_opt.as_deref(), res.dz_exact.get(name_key))
            {
                Self::record_unique(&mut res.ytm_to_dz, ytm_ar_id.to_string(), dz_ar_id);
            }
        }
        for (sl_key, ytm_opt) in &res.ytm_slug {
            if let (Some(ytm_ar_id), Some(Some(dz_ar_id))) =
                (ytm_opt.as_deref(), res.dz_slug.get(sl_key))
            {
                Self::record_unique(&mut res.ytm_to_dz, ytm_ar_id.to_string(), dz_ar_id);
            }
        }

        res
    }

    fn resolve(&self, raw_id: &str, name: &str) -> String {
        if raw_id.starts_with("ar:dz:") {
            return raw_id.to_string();
        }
        if raw_id.starts_with("ar:ytm:") {
            if let Some(Some(dz_ar_id)) = self.ytm_to_dz.get(raw_id) {
                return dz_ar_id.clone();
            }
        }
        let pa = primary_artist(name);
        let key_name = if pa.is_empty() { name.trim() } else { pa.as_str() };
        if key_name.is_empty() {
            return raw_id.to_string();
        }
        match self.dz_exact.get(key_name) {
            Some(Some(dz_ar_id)) => return dz_ar_id.clone(),
            Some(None) => return raw_id.to_string(),
            None => {
                let sl = slug(key_name);
                if !sl.is_empty() {
                    match self.dz_slug.get(&sl) {
                        Some(Some(dz_ar_id)) => return dz_ar_id.clone(),
                        Some(None) => return raw_id.to_string(),
                        None => {}
                    }
                }
            }
        }
        if !raw_id.starts_with("ar:ytm:") {
            match self.ytm_exact.get(key_name) {
                Some(Some(ytm_ar_id)) => return ytm_ar_id.clone(),
                Some(None) => return raw_id.to_string(),
                None => {
                    let sl = slug(key_name);
                    if !sl.is_empty() {
                        if let Some(Some(ytm_ar_id)) = self.ytm_slug.get(&sl) {
                            return ytm_ar_id.clone();
                        }
                    }
                }
            }
        }
        raw_id.to_string()
    }

    fn resolve_track(&self, t: &Track) -> String {
        self.resolve(&t.artist_id(), &t.artist)
    }

    fn resolve_meta(&self, id: &str, meta: &ArtistSummary) -> String {
        let dz_clean = meta.deezer_id.trim();
        if !dz_clean.is_empty() && !id.starts_with("ar:dz:") {
            return format!("ar:dz:{}", dz_clean);
        }
        self.resolve(id, &meta.name)
    }
}

fn migrate_artist_disambiguation(conn: &mut Connection) -> AppResult<()> {
    let done: Option<String> = conn
        .query_row(
            "SELECT value FROM kv WHERE key = 'artist_disambig_v1'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    if done.is_some() {
        return Ok(());
    }

    let legacy_artists: Vec<(String, String)> = {
        let mut stmt = conn.prepare(
            "SELECT id, deezer_id FROM artists_meta
             WHERE deezer_id != '' AND id NOT LIKE 'ar:dz:%'",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        rows.flatten().collect()
    };

    let tx = conn.transaction()?;
    for (old_id, dz_id) in legacy_artists {
        let new_id = format!("ar:dz:{}", dz_id.trim());
        tx.execute(
            "UPDATE OR IGNORE artists_meta SET id = ?1 WHERE id = ?2",
            params![new_id, old_id],
        )?;
        tx.execute("DELETE FROM artists_meta WHERE id = ?1", params![old_id])?;
        tx.execute(
            "UPDATE OR IGNORE followed_artists SET artist_id = ?1 WHERE artist_id = ?2",
            params![new_id, old_id],
        )?;
        tx.execute(
            "DELETE FROM followed_artists WHERE artist_id = ?1",
            params![old_id],
        )?;
    }

    // Purge unreferenced legacy Deezer catalog rows that lack `deezer_artist_id` in `meta`,
    // so subsequent searches and artist pages re-hydrate with disambiguated artist IDs.
    tx.execute(
        "DELETE FROM tracks
         WHERE source = 'deezer'
           AND meta NOT LIKE '%deezer_artist_id%'
           AND id NOT IN (SELECT track_id FROM playlist_tracks)
           AND id NOT IN (SELECT track_id FROM stars)
           AND id NOT IN (SELECT track_id FROM ratings)
           AND id NOT IN (SELECT track_id FROM user_track_stats)",
        [],
    )?;

    tx.execute(
        "INSERT OR REPLACE INTO kv(key, value) VALUES ('artist_disambig_v1', '1')",
        [],
    )?;
    tx.commit()?;
    Ok(())
}

/// Consolidates duplicate non-Deezer `artists_meta` and `followed_artists` rows
/// (e.g. `ar:yorushika` or `ar:ytm:...`) into their unambiguous canonical `ar:dz:<id>`
/// profile, without ever merging distinct homonym artists (e.g. Korean `LISA` vs Japanese `LiSA`).
fn migrate_artist_disambiguation_v2(conn: &mut Connection) -> AppResult<()> {
    let done: Option<String> = conn
        .query_row(
            "SELECT value FROM kv WHERE key = 'artist_disambig_v2'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    if done.is_some() {
        return Ok(());
    }

    let metas: HashMap<String, ArtistSummary> = {
        let mut stmt = conn.prepare("SELECT id, name, cover_url, bio, deezer_id FROM artists_meta")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                ArtistSummary {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    cover_url: r.get(2)?,
                    bio: r.get(3)?,
                    deezer_id: r.get(4)?,
                    album_count: 0,
                    song_count: 0,
                },
            ))
        })?;
        rows.flatten().collect()
    };

    let tracks: Vec<Track> = {
        let sql = format!("SELECT {} FROM tracks LIMIT 8000", TRACK_COLS);
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], row_to_track)?;
        rows.flatten().collect()
    };

    let resolver = ArtistResolver::build(&tracks, &metas);
    let mut remaps: Vec<(String, String)> = Vec::new();
    for (id, meta) in &metas {
        let canonical = resolver.resolve_meta(id, meta);
        if canonical != *id {
            remaps.push((id.clone(), canonical));
        }
    }

    let tx = conn.transaction()?;
    for (old_id, new_id) in remaps {
        tx.execute(
            "UPDATE OR IGNORE artists_meta SET id = ?1 WHERE id = ?2",
            params![new_id, old_id],
        )?;
        tx.execute("DELETE FROM artists_meta WHERE id = ?1", params![old_id])?;
        tx.execute(
            "UPDATE OR IGNORE followed_artists SET artist_id = ?1 WHERE artist_id = ?2",
            params![new_id, old_id],
        )?;
        tx.execute(
            "DELETE FROM followed_artists WHERE artist_id = ?1",
            params![old_id],
        )?;
    }

    tx.execute(
        "INSERT OR REPLACE INTO kv(key, value) VALUES ('artist_disambig_v2', '1')",
        [],
    )?;
    tx.commit()?;
    Ok(())
}

fn title_case(s: &str) -> String {
    s.split_whitespace()
        .map(|w| {
            let mut ch = w.chars();
            match ch.next() {
                Some(first) => first.to_uppercase().to_string() + &ch.as_str().to_lowercase(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_track(id: &str, title: &str, artist: &str, album: &str) -> Track {
        Track {
            id: id.into(),
            source: "deezer".into(),
            title: title.into(),
            artist: artist.into(),
            album: album.into(),
            genre: "Electronic".into(),
            moods: "energy,party".into(),
            year: 2020,
            track_no: 1,
            disc_no: 1,
            duration: 210,
            bitrate: 256,
            format: "m4a".into(),
            stream_url: format!("resolve:ytm:{}|{}|210", artist, title),
            deezer_id: id.trim_start_matches("dz:").into(),
            ..Default::default()
        }
    }

    #[test]
    fn open_signup_and_sessions_persist() {
        let store = Store::open_memory().unwrap();
        let (alice, created) = store.authenticate_or_create_user("Alice", "secret123").unwrap();
        assert!(created);
        assert_eq!(alice.username, "Alice");

        // Second login (case-insensitive username) with right password succeeds without creating duplicate.
        let (alice2, created2) = store.authenticate_or_create_user("alice", "secret123").unwrap();
        assert!(!created2);
        assert_eq!(alice2.id, alice.id);

        // Wrong password rejects.
        assert!(store.authenticate_or_create_user("alice", "wrongpass").is_err());

        // Session roundtrip.
        let tok = store.create_session(alice.id, "nd").unwrap();
        let loaded = store.get_session_user(&tok).unwrap().unwrap();
        assert_eq!(loaded.id, alice.id);
        store.delete_session(&tok).unwrap();
        assert!(store.get_session_user(&tok).unwrap().is_none());

        // API key roundtrip.
        let key1 = store.get_or_create_api_key(alice.id).unwrap();
        assert_eq!(store.get_user_by_api_key(&key1).unwrap().unwrap().id, alice.id);
        let key2 = store.regenerate_api_key(alice.id).unwrap();
        assert_ne!(key1, key2);
        assert!(store.get_user_by_api_key(&key1).unwrap().is_none());
    }

    #[test]
    fn totp_recovery_codes_and_session_mfa_roundtrip() {
        let store = Store::open_memory().unwrap();
        let (alice, _) = store.authenticate_or_create_user("alice", "secret123").unwrap();
        assert!(!store.is_2fa_enabled(alice.id).unwrap());

        let secret = crate::auth::generate_totp_secret();
        store.save_totp(alice.id, &secret, false).unwrap();
        assert!(!store.is_2fa_enabled(alice.id).unwrap());
        let step = Utc::now().timestamp() / crate::auth::TOTP_PERIOD;
        let code = crate::auth::totp_code(&secret, step).unwrap();
        assert_eq!(crate::auth::verify_totp(&secret, &code, step - 1).unwrap(), Some(step));
        assert_eq!(crate::auth::verify_totp(&secret, &code, step).unwrap(), None);
        store.mark_totp_step(alice.id, step).unwrap();
        store.set_totp_enabled(alice.id, true).unwrap();
        assert!(store.is_2fa_enabled(alice.id).unwrap());

        let codes = crate::auth::generate_recovery_codes(crate::auth::RECOVERY_CODE_COUNT);
        store.replace_recovery_codes(alice.id, &codes).unwrap();
        assert_eq!(store.recovery_code_stats(alice.id).unwrap(), (10, 0));
        assert!(store.consume_recovery_code(alice.id, &codes[0]).unwrap());
        assert!(!store.consume_recovery_code(alice.id, &codes[0]).unwrap());
        assert_eq!(store.recovery_code_stats(alice.id).unwrap(), (10, 1));

        let token = store.create_session(alice.id, "web").unwrap();
        assert_eq!(store.session_auth_state(&token).unwrap().unwrap().1, 0);
        store.mark_session_mfa(&token).unwrap();
        assert!(store.session_auth_state(&token).unwrap().unwrap().1 > 0);

        store.record_security_event(alice.id, "test", "detail", "1.2.3.4");
        assert_eq!(store.list_security_events(alice.id, 10).unwrap().len(), 1);
    }

    #[test]
    fn soft_delete_signs_out_everywhere_and_allows_restore() {
        let store = Store::open_memory().unwrap();
        let (alice, _) = store.authenticate_or_create_user("alice", "secret123").unwrap();
        let key = store.get_or_create_api_key(alice.id).unwrap();
        let token = store.create_session(alice.id, "web").unwrap();

        store
            .soft_delete_user(alice.id, &crate::auth::hash_token("restore-secret"))
            .unwrap();
        assert!(store.get_session_user(&token).unwrap().is_none());
        assert!(store.get_user_by_api_key(&key).unwrap().is_none());
        assert!(store.get_user_by_id(alice.id).unwrap().is_none());
        assert_eq!(
            store.authenticate_or_create_user("alice", "secret123").unwrap_err().0,
            crate::models::ERR_RESTORE_REQUIRED
        );
        assert!(store.restore_hash(alice.id).unwrap().is_some());
        assert!(store.deletion_expires_at(alice.id).unwrap().unwrap() > Utc::now().timestamp());

        store.restore_user(alice.id).unwrap();
        assert!(store.authenticate_or_create_user("alice", "secret123").is_ok());
        assert_ne!(store.get_or_create_api_key(alice.id).unwrap(), key);

        // Accounts past the seven-day window are removed permanently.
        let (bob, _) = store.authenticate_or_create_user("bob", "secret123").unwrap();
        store.soft_delete_user(bob.id, "hash").unwrap();
        {
            let conn = store.lock();
            conn.execute(
                "UPDATE users SET deleted_at = 1 WHERE id = ?1",
                params![bob.id],
            )
            .unwrap();
        }
        assert_eq!(store.purge_expired_deleted_users().unwrap(), 1);
        assert!(store.get_user_by_id(bob.id).unwrap().is_none());
    }

    #[test]
    fn session_listing_and_revocation_roundtrip() {
        let store = Store::open_memory().unwrap();
        let (alice, _) = store.authenticate_or_create_user("alice", "secret123").unwrap();
        let (bob, _) = store.authenticate_or_create_user("bob", "secret123").unwrap();

        let t1 = store.create_session(alice.id, "web").unwrap();
        let t2 = store.create_session(alice.id, "nd").unwrap();
        let bob_tok = store.create_session(bob.id, "web").unwrap();

        let sessions = store.list_sessions(alice.id).unwrap();
        assert_eq!(sessions.len(), 2);
        assert!(sessions.iter().any(|s| s.token == t1 && s.kind == "web"));
        assert!(sessions.iter().any(|s| s.token == t2 && s.kind == "nd"));
        assert_eq!(store.list_sessions(bob.id).unwrap().len(), 1);

        // A user cannot revoke somebody else's session.
        assert!(!store.revoke_session_for_user(&bob_tok, alice.id).unwrap());
        assert_eq!(store.list_sessions(bob.id).unwrap().len(), 1);

        // Revoke one of our own sessions.
        assert!(store.revoke_session_for_user(&t1, alice.id).unwrap());
        let sessions = store.list_sessions(alice.id).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].token, t2);

        // Revoke every remaining session.
        assert_eq!(store.delete_all_sessions_for_user(alice.id).unwrap(), 1);
        assert!(store.list_sessions(alice.id).unwrap().is_empty());
        // Other users are untouched.
        assert_eq!(store.list_sessions(bob.id).unwrap().len(), 1);
    }

    #[test]
    fn session_metadata_and_subsonic_client_revocation() {
        let store = Store::open_memory().unwrap();
        let (alice, _) = store.authenticate_or_create_user("alice", "secret123").unwrap();

        let meta = SessionMeta {
            user_agent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/131.0".to_string(),
            ip: "203.0.113.7".to_string(),
            client: String::new(),
        };
        let tok = store
            .create_session_with_meta(alice.id, "web", &meta)
            .unwrap();
        let sessions = store.list_sessions(alice.id).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].token, tok);
        assert_eq!(sessions[0].user_agent, meta.user_agent);
        assert_eq!(sessions[0].ip, meta.ip);
        assert!(sessions[0].last_seen > 0);

        // Subsonic client sessions are tracked by client identity.
        let (client_tok, revoked) = store
            .touch_subsonic_client(alice.id, "Symfonium", "Symfonium/12", "203.0.113.7")
            .unwrap();
        assert!(!revoked);
        let (client_tok2, revoked2) = store
            .touch_subsonic_client(alice.id, "Symfonium", "Symfonium/12", "203.0.113.9")
            .unwrap();
        assert_eq!(client_tok, client_tok2);
        assert!(!revoked2);

        let sessions = store.list_sessions(alice.id).unwrap();
        assert_eq!(sessions.len(), 2);
        let client = sessions.iter().find(|s| s.kind == "subsonic").unwrap();
        assert_eq!(client.client, "Symfonium");
        assert_eq!(client.ip, "203.0.113.9");
        assert!(!client.revoked);

        // Revoking a Subsonic session flags it instead of deleting it, so the
        // client can't simply authenticate again.
        assert!(store.revoke_session_for_user(&client_tok, alice.id).unwrap());
        let (_, revoked) = store
            .touch_subsonic_client(alice.id, "Symfonium", "Symfonium/12", "203.0.113.7")
            .unwrap();
        assert!(revoked);
        assert!(store
            .list_sessions(alice.id)
            .unwrap()
            .iter()
            .find(|s| s.kind == "subsonic")
            .unwrap()
            .revoked);

        // Re-allowing clears the flag.
        assert!(store.allow_session_for_user(&client_tok, alice.id).unwrap());
        let (_, revoked) = store
            .touch_subsonic_client(alice.id, "Symfonium", "Symfonium/12", "203.0.113.7")
            .unwrap();
        assert!(!revoked);

        // Revoking all deletes the web session and flags the client session.
        assert_eq!(store.delete_all_sessions_for_user(alice.id).unwrap(), 2);
        let sessions = store.list_sessions(alice.id).unwrap();
        assert_eq!(sessions.len(), 1);
        assert!(sessions[0].revoked);
    }

    #[test]
    fn sessions_do_not_expire() {
        let store = Store::open_memory().unwrap();
        let (alice, _) = store.authenticate_or_create_user("alice", "secret123").unwrap();
        let tok = store.create_session(alice.id, "web").unwrap();

        // Even a row left over from the old 30-day scheme with a past expiry
        // stays valid: expiry is no longer consulted or pruned.
        {
            let conn = store.lock();
            conn.execute(
                "UPDATE sessions SET expires = 1 WHERE token = ?1",
                params![tok],
            )
            .unwrap();
        }
        assert_eq!(store.get_session_user(&tok).unwrap().unwrap().id, alice.id);
        let sessions = store.list_sessions(alice.id).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].token, tok);
    }

    #[test]
    fn social_profiles_follows_and_user_radios_roundtrip() {
        let store = Store::open_memory().unwrap();
        let (emily, _) = store.authenticate_or_create_user("emily", "pw1").unwrap();
        let (alex, _) = store.authenticate_or_create_user("alex", "pw2").unwrap();
        let (sam, _) = store.authenticate_or_create_user("sam", "pw3").unwrap();

        // Profile editing
        let profile = store
            .update_profile(emily.id, "Emily C", "Synth-pop obsessive", "#8b5cf6", "Brooklyn")
            .unwrap();
        assert_eq!(profile.display_name, "Emily C");
        assert_eq!(store.get_profile(emily.id).unwrap().bio, "Synth-pop obsessive");

        // @mention-style search matches username, display name and bio
        let by_handle = store.search_public_users("@em", alex.id, 10).unwrap();
        assert!(by_handle.iter().any(|u| u.id == emily.id));
        let by_name = store.search_public_users("emily c", alex.id, 10).unwrap();
        assert_eq!(by_name[0].id, emily.id);
        assert_eq!(by_name[0].avatar_letter, "E");
        assert_eq!(by_name[0].avatar_color, "#8b5cf6");
        assert!(store.search_public_users("@nobody", alex.id, 10).unwrap().is_empty());

        // Self-follow is rejected
        assert!(store.follow_user(emily.id, emily.id).is_err());

        // Follow graph
        store.follow_user(alex.id, emily.id).unwrap();
        store.follow_user(alex.id, emily.id).unwrap(); // idempotent
        store.follow_user(sam.id, emily.id).unwrap();
        let emily_pub = store.get_public_user(emily.id, alex.id).unwrap().unwrap();
        assert_eq!(emily_pub.followers_count, 2);
        assert!(emily_pub.is_following);
        assert!(!emily_pub.is_self);
        let self_pub = store.get_public_user(alex.id, alex.id).unwrap().unwrap();
        assert!(self_pub.is_self);
        assert_eq!(store.list_followers(emily.id, alex.id).unwrap().len(), 2);
        assert_eq!(store.list_following(alex.id, alex.id).unwrap().len(), 1);

        // Saved user radios: a listener subscribes to another listener's
        // auto-updating mix, which lives in their own library.
        let track = sample_track("dz:rec", "Digital Love", "Daft Punk", "Discovery");
        store.upsert_tracks(&[track.clone()]).unwrap();
        let pid = "radio-u2-t1";
        store
            .upsert_auto_playlist(
                pid,
                alex.id,
                "Emily Radio",
                "Auto mix",
                "user_radio:1",
                &[track.id.clone()],
            )
            .unwrap();
        store.save_user_radio(alex.id, emily.id, pid).unwrap();
        assert_eq!(
            store.get_user_radio(alex.id, emily.id).unwrap().as_deref(),
            Some(pid)
        );
        let all = store.all_user_radios().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0], (alex.id, emily.id, pid.to_string()));
        assert_eq!(store.playlist_tracks(pid).unwrap().len(), 1);

        // Refreshing swaps the playlist contents in place.
        store
            .upsert_auto_playlist(pid, alex.id, "Emily Radio", "Auto mix", "user_radio:1", &[])
            .unwrap();
        assert!(store.playlist_tracks(pid).unwrap().is_empty());

        let removed = store.delete_user_radio(alex.id, emily.id).unwrap();
        assert_eq!(removed.as_deref(), Some(pid));
        assert!(store.get_user_radio(alex.id, emily.id).unwrap().is_none());
        assert!(store.all_user_radios().unwrap().is_empty());
        store.delete_playlist(pid).unwrap();

        // Non-hex colors are rejected before they can reach a CSS style attribute.
        let bad = store
            .update_profile(emily.id, "Emily C", "", "red;position:fixed", "")
            .unwrap();
        assert!(bad.avatar_color.is_empty());
    }

    #[test]
    fn profile_picture_set_replace_and_clear_roundtrip() {
        let store = Store::open_memory().unwrap();
        let (user, _) = store.authenticate_or_create_user("picuser", "pw").unwrap();

        // No picture yet: generated avatar only.
        assert!(store.get_avatar(user.id).unwrap().is_none());
        assert!(store.get_public_user(user.id, user.id).unwrap().unwrap().avatar_url.is_empty());

        store.set_avatar(user.id, b"first-image-bytes", "image/jpeg").unwrap();
        let (bytes, mime) = store.get_avatar(user.id).unwrap().unwrap();
        assert_eq!(bytes, b"first-image-bytes");
        assert_eq!(mime, "image/jpeg");
        let with_pic = store.get_public_user(user.id, user.id).unwrap().unwrap();
        assert!(with_pic.avatar_url.starts_with("/api/web/social/avatar/"));

        // Replacing keeps the text profile fields intact and bumps the version.
        store.update_profile(user.id, "Pic User", "hello", "", "").unwrap();
        let v1 = store.get_public_user(user.id, user.id).unwrap().unwrap().avatar_url;
        store.set_avatar(user.id, b"second-image-bytes", "image/png").unwrap();
        let v2 = store.get_public_user(user.id, user.id).unwrap().unwrap().avatar_url;
        assert_ne!(v1, v2);
        assert_eq!(store.get_profile(user.id).unwrap().display_name, "Pic User");
        let (bytes, mime) = store.get_avatar(user.id).unwrap().unwrap();
        assert_eq!(bytes, b"second-image-bytes");
        assert_eq!(mime, "image/png");

        // Clearing removes the bytes but preserves the profile text.
        store.clear_avatar(user.id).unwrap();
        assert!(store.get_avatar(user.id).unwrap().is_none());
        assert!(store.get_public_user(user.id, user.id).unwrap().unwrap().avatar_url.is_empty());
        assert_eq!(store.get_profile(user.id).unwrap().display_name, "Pic User");
    }

    #[test]
    fn per_user_stats_and_playlists_isolated() {
        let store = Store::open_memory().unwrap();
        let (u1, _) = store.authenticate_or_create_user("bob", "pw1").unwrap();
        let (u2, _) = store.authenticate_or_create_user("carol", "pw2").unwrap();
        let t1 = sample_track("dz:1", "One More Time", "Daft Punk", "Discovery");
        let t2 = sample_track("dz:2", "Aerodynamic", "Daft Punk", "Discovery");
        store.upsert_tracks(&[t1.clone(), t2.clone()]).unwrap();

        store.record_event(u1.id, &t1.id, "play", 1.0).unwrap();
        store.record_event(u1.id, &t1.id, "play", 1.0).unwrap();
        store.record_event(u2.id, &t2.id, "play", 1.0).unwrap();

        assert_eq!(store.get_user_track_stats(u1.id, &t1.id).unwrap().play_count, 2);
        assert_eq!(store.get_user_track_stats(u2.id, &t1.id).unwrap().play_count, 0);

        let pl = store
            .create_playlist(u1.id, "Mix", "My mix", false, &[t1.id.clone(), t2.id.clone()])
            .unwrap();
        assert_eq!(pl.track_count, 2);
        store.move_playlist_track(&pl.id, 1, 0).unwrap();
        let tracks = store.playlist_tracks(&pl.id).unwrap();
        assert_eq!(tracks[0].id, t2.id);
        store.remove_playlist_positions(&pl.id, &[0]).unwrap();
        assert_eq!(store.playlist_tracks(&pl.id).unwrap().len(), 1);

        // u2 shouldn't see u1's private playlist
        assert!(store.list_playlists(u2.id, true).unwrap().is_empty());
    }

    #[test]
    fn play_queue_roundtrip() {
        let store = Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("dave", "pw").unwrap();
        let state = PlayQueueState {
            current_id: "dz:10".into(),
            current_index: 1,
            position_ms: 42000,
            track_ids: vec!["dz:9".into(), "dz:10".into(), "dz:11".into()],
            changed_by: "webapp".into(),
            updated: 0,
        };
        store.save_play_queue(u.id, &state).unwrap();
        let got = store.get_play_queue(u.id).unwrap().unwrap();
        assert_eq!(got.current_id, "dz:10");
        assert_eq!(got.current_index, 1);
        assert_eq!(got.position_ms, 42000);
        assert_eq!(got.track_ids.len(), 3);
    }

    #[test]
    fn search_match_prefers_original_artist_and_title_over_parody() {
        let orig = sample_track("dz:orig", "One More Time", "Daft Punk", "Discovery");
        let parody = sample_track(
            "dz:par",
            "Sample a Song (Parody of Daft Punk One More Time)",
            "DJ Timbo",
            "Parodies",
        );
        let q = "daft punk one more time";
        assert!(
            search_match_score(&orig, q) > search_match_score(&parody, q),
            "orig={}, parody={}",
            search_match_score(&orig, q),
            search_match_score(&parody, q)
        );
    }

    #[test]
    fn candidate_tracks_for_user_prioritizes_taste_links_and_local_before_recent_charts() {
        let store = Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("niche", "pw").unwrap();

        let mut seed = sample_track("ytm:seed1", "Dawn", "Niche Band", "Album One");
        seed.source = "ytm".into();
        seed.genre = String::new();
        seed.moods = String::new();
        seed.created = 100;

        let mut same_artist = sample_track("ytm:same1", "Dusk", "Niche Band", "Album Two");
        same_artist.source = "ytm".into();
        same_artist.genre = String::new();
        same_artist.moods = String::new();
        same_artist.created = 101;

        let mut related = sample_track("ytm:rel1", "Echoes", "Kindred Act", "Echo Album");
        related.source = "ytm".into();
        related.genre = String::new();
        related.moods = String::new();
        related.created = 102;

        let mut local = sample_track("loc:1", "Basement Tape", "Local Friend", "Demos");
        local.source = "local".into();
        local.created = 103;

        let mut old_batch = vec![seed.clone(), same_artist.clone(), related.clone(), local.clone()];
        let mut chart_batch = Vec::new();
        for i in 0..25 {
            let mut pop = sample_track(
                &format!("dz:chart{}", i),
                &format!("Chart Hit {}", i),
                &format!("Pop Star {}", i),
                "Top 50",
            );
            pop.genre = "Pop".into();
            pop.moods = "upbeat".into();
            pop.created = 5000 + i;
            chart_batch.push(pop);
        }
        old_batch.extend(chart_batch);
        store.upsert_tracks(&old_batch).unwrap();

        store.record_event(u.id, &seed.id, "play", 1.0).unwrap();
        store
            .upsert_taste_links(
                u.id,
                &[
                    ("related_track", &related.id, 1.5),
                    ("related_artist", "kindred-act", 1.4),
                ],
            )
            .unwrap();

        let pool = store.candidate_tracks_for_user(u.id, 12).unwrap();
        let ids: HashSet<&str> = pool.iter().map(|t| t.id.as_str()).collect();
        assert!(ids.contains("ytm:seed1"), "ids={:?}", ids);
        assert!(ids.contains("ytm:rel1"), "ids={:?}", ids);
        assert!(ids.contains("ytm:same1"), "ids={:?}", ids);
        assert!(ids.contains("loc:1"), "ids={:?}", ids);
    }

    #[test]
    fn legacy_dirty_moods_migrate_cleanly_on_store_open() {
        let dir = std::env::temp_dir().join(format!("beatlab_mood_mig_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("beatlab.db");

        {
            let store = Store::open(&db_path).unwrap();
            let mut waltz = sample_track("ytm:waltz", "Waltz for Zizi", "Seatbelts", "OST");
            waltz.genre = String::new();
            waltz.moods = "rock_indie,chill".into();

            let mut untagged = sample_track("ytm:untagged", "Kurogane", "Nocturne", "Demo");
            untagged.genre = String::new();
            untagged.moods = "upbeat,chill".into();

            let mut cover = sample_track("ytm:cover", "Hit Song (Instrumental)", "Covers", "Pop");
            cover.genre = String::new();
            cover.moods = "focus".into();

            let mut eno_known = sample_track("dz:eno1", "An Ending (Ascent)", "Brian Eno", "Apollo");
            eno_known.genre = "Ambient".into();
            eno_known.moods = "".into();

            let mut eno_ytm = sample_track("ytm:eno2", "1/1", "Brian Eno", "Music for Airports");
            eno_ytm.genre = String::new();
            eno_ytm.moods = "upbeat,chill".into();

            store
                .upsert_tracks(&[waltz, untagged, cover, eno_known, eno_ytm])
                .unwrap();

            // Simulate upgrading a legacy pre-v3 database where kv flag does not yet exist.
            let conn = store.lock();
            conn.execute("DELETE FROM kv WHERE key = 'moods_classifier_v3'", [])
                .unwrap();
        }

        let reopened = Store::open(&db_path).unwrap();
        assert_eq!(reopened.get_track("ytm:waltz").unwrap().unwrap().moods, "");
        assert_eq!(reopened.get_track("ytm:untagged").unwrap().unwrap().moods, "");
        assert_eq!(reopened.get_track("ytm:cover").unwrap().unwrap().moods, "");
        assert!(
            reopened
                .get_track("dz:eno1")
                .unwrap()
                .unwrap()
                .moods
                .split(',')
                .any(|m| m == "focus")
        );
        assert!(
            reopened
                .get_track("ytm:eno2")
                .unwrap()
                .unwrap()
                .moods
                .split(',')
                .any(|m| m == "focus")
        );

        let by_focus = reopened.tracks_by_mood(Mood::Focus, 20, 0).unwrap();
        let focus_ids: Vec<&str> = by_focus.iter().map(|t| t.id.as_str()).collect();
        assert!(focus_ids.contains(&"dz:eno1"), "{:?}", focus_ids);
        assert!(focus_ids.contains(&"ytm:eno2"), "{:?}", focus_ids);
        assert!(!focus_ids.contains(&"ytm:cover"), "{:?}", focus_ids);

        drop(reopened);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn radio_station_ownership_prevents_cross_user_overwrite_and_delete() {
        let store = Store::open_memory().unwrap();
        let (alice, _) = store.authenticate_or_create_user("alice", "pw1").unwrap();
        let (bob, _) = store.authenticate_or_create_user("bob", "pw2").unwrap();

        let created = store
            .upsert_radio_station(
                alice.id,
                &RadioStation {
                    id: String::new(),
                    name: "Alice FM".into(),
                    stream_url: "https://example.com/alice.mp3".into(),
                    home_page_url: String::new(),
                },
            )
            .unwrap();

        // Bob cannot overwrite or delete Alice's station.
        let overwrite = store.upsert_radio_station(
            bob.id,
            &RadioStation {
                id: created.id.clone(),
                name: "Hijacked FM".into(),
                stream_url: "https://evil.example/stream.mp3".into(),
                home_page_url: String::new(),
            },
        );
        assert!(overwrite.is_err());
        assert!(store.delete_radio_station(bob.id, &created.id).is_err());

        // Global (user_id = 0) stations are also protected against non-owner deletes.
        {
            let conn = store.lock();
            conn.execute(
                "INSERT INTO radio_stations(id, user_id, name, stream_url, home_page_url)
                 VALUES ('rad-global', 0, 'Global FM', 'https://example.com/global.mp3', '')",
                [],
            )
            .unwrap();
        }
        assert!(store.delete_radio_station(bob.id, "rad-global").is_err());

        // Owner can still update and delete her own station.
        let updated = store
            .upsert_radio_station(
                alice.id,
                &RadioStation {
                    id: created.id.clone(),
                    name: "Alice HD".into(),
                    stream_url: "https://example.com/alice-hd.mp3".into(),
                    home_page_url: String::new(),
                },
            )
            .unwrap();
        assert_eq!(updated.name, "Alice HD");
        store.delete_radio_station(alice.id, &created.id).unwrap();
    }

    #[test]
    fn load_play_queue_tracks_reconciles_index_after_missing_track() {
        let store = Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("q_user", "pw").unwrap();
        let t2 = sample_track("dz:2", "Two", "Artist", "Album");
        let t3 = sample_track("dz:3", "Three", "Artist", "Album");
        store.upsert_tracks(&[t2, t3]).unwrap();

        // Queue originally held ["missing:1", "dz:2", "dz:3"] with currentIndex = 2 pointing at "dz:3".
        store
            .save_play_queue(
                u.id,
                &PlayQueueState {
                    current_id: "dz:3".into(),
                    current_index: 2,
                    position_ms: 42_000,
                    track_ids: vec!["missing:1".into(), "dz:2".into(), "dz:3".into()],
                    changed_by: "webapp".into(),
                    updated: 1,
                },
            )
            .unwrap();

        let (q, tracks, resolved_idx, resolved_id) =
            store.load_play_queue_tracks(u.id).unwrap().unwrap();
        assert_eq!(q.position_ms, 42_000);
        assert_eq!(tracks.len(), 2);
        assert_eq!(resolved_idx, 1);
        assert_eq!(resolved_id, "dz:3");
    }

    #[test]
    fn update_playlist_ignores_blank_name() {
        let store = Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("pl_user", "pw").unwrap();
        let pl = store.create_playlist(u.id, "Original Name", "", false, &[]).unwrap();
        store.update_playlist(&pl.id, Some("   "), None, None).unwrap();
        let after = store.get_playlist(&pl.id).unwrap().unwrap();
        assert_eq!(after.name, "Original Name");
    }

    #[test]
    fn playlist_visibility_set_at_creation_and_editable_anytime() {
        let store = Store::open_memory().unwrap();
        let (owner, _) = store.authenticate_or_create_user("visowner", "pw").unwrap();
        let (other, _) = store.authenticate_or_create_user("visother", "pw").unwrap();

        let secret = store.create_playlist(owner.id, "Secret", "", false, &[]).unwrap();
        let shared = store.create_playlist(owner.id, "Shared", "", true, &[]).unwrap();
        assert!(!secret.public);
        assert!(shared.public);

        // Another listener only sees the public one.
        let visible: HashSet<String> = store
            .list_playlists(other.id, true)
            .unwrap()
            .into_iter()
            .map(|p| p.id)
            .collect();
        assert!(visible.contains(&shared.id));
        assert!(!visible.contains(&secret.id));

        // Visibility can be flipped freely after creation, in both directions.
        store.update_playlist(&secret.id, None, None, Some(true)).unwrap();
        assert!(store.get_playlist(&secret.id).unwrap().unwrap().public);
        store.update_playlist(&shared.id, None, None, Some(false)).unwrap();
        assert!(!store.get_playlist(&shared.id).unwrap().unwrap().public);

        let visible_after: HashSet<String> = store
            .list_playlists(other.id, true)
            .unwrap()
            .into_iter()
            .map(|p| p.id)
            .collect();
        assert!(visible_after.contains(&secret.id));
        assert!(!visible_after.contains(&shared.id));
    }

    #[test]
    fn homonym_artists_lisa_kr_and_lisa_jp_stay_separated_in_summaries_tracks_and_albums() {
        let store = Store::open_memory().unwrap();

        let lisa_kr_meta = ArtistSummary {
            id: "ar:dz:14559887".into(),
            name: "LISA".into(),
            cover_url: "https://cdn.deezer.com/lisa_kr.jpg".into(),
            deezer_id: "14559887".into(),
            ..Default::default()
        };
        let lisa_jp_meta = ArtistSummary {
            id: "ar:dz:418196".into(),
            name: "LiSA".into(),
            cover_url: "https://cdn.deezer.com/lisa_jp.jpg".into(),
            deezer_id: "418196".into(),
            ..Default::default()
        };
        store.upsert_artist_meta(&lisa_kr_meta).unwrap();
        store.upsert_artist_meta(&lisa_jp_meta).unwrap();

        let alb_kr = AlbumSummary {
            id: "al:dz-14559887-lalisa".into(),
            name: "LALISA".into(),
            artist: "LISA".into(),
            artist_id: "ar:dz:14559887".into(),
            deezer_id: "257654".into(),
            year: 2021,
            ..Default::default()
        };
        let alb_jp = AlbumSummary {
            id: "al:dz-418196-leo-nine".into(),
            name: "LEO-NiNE".into(),
            artist: "LiSA".into(),
            artist_id: "ar:dz:418196".into(),
            deezer_id: "179000".into(),
            year: 2020,
            ..Default::default()
        };
        store.upsert_album_meta(&alb_kr).unwrap();
        store.upsert_album_meta(&alb_jp).unwrap();

        let mut money = sample_track("dz:money", "MONEY", "LISA", "LALISA");
        money.meta = r#"{"rank":920000,"deezer_artist_id":14559887,"deezer_album_id":257654}"#.into();
        let mut rockstar = sample_track("dz:rockstar_kr", "Rockstar", "LISA", "Alter Ego");
        rockstar.meta = r#"{"rank":890000,"deezer_artist_id":14559887,"deezer_album_id":555111}"#.into();

        let mut gurenge = sample_track("dz:gurenge", "Gurenge", "LiSA", "LEO-NiNE");
        gurenge.meta = r#"{"rank":910000,"deezer_artist_id":418196,"deezer_album_id":179000}"#.into();
        let mut crossing = sample_track("ytm:crossing", "Crossing Field", "LiSA", "LANDSPACE");
        crossing.source = "ytm".into();
        crossing.meta = r#"{"ytm_artist_id":"UC_LISA_JP"}"#.into();

        let mut batch = vec![money.clone(), rockstar.clone(), gurenge.clone(), crossing.clone()];
        crate::sources::disambiguate_batch_tracks(
            &mut batch,
            &[lisa_kr_meta.clone(), lisa_jp_meta.clone()],
        );
        store.upsert_tracks(&batch).unwrap();

        let summaries = store.build_artist_summaries().unwrap();
        let kr_summary = summaries
            .iter()
            .find(|a| a.id == "ar:dz:14559887")
            .expect("Korean LISA summary should exist");
        let jp_summary = summaries
            .iter()
            .find(|a| a.id == "ar:dz:418196")
            .expect("Japanese LiSA summary should exist");
        assert_eq!(kr_summary.name, "LISA");
        assert_eq!(jp_summary.name, "LiSA");
        assert_eq!(kr_summary.cover_url, "https://cdn.deezer.com/lisa_kr.jpg");
        assert_eq!(jp_summary.cover_url, "https://cdn.deezer.com/lisa_jp.jpg");
        assert_eq!(kr_summary.song_count, 2);
        assert_eq!(jp_summary.song_count, 2);

        let kr_tracks = store.artist_tracks("ar:dz:14559887").unwrap();
        let kr_titles: Vec<&str> = kr_tracks.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(kr_titles.len(), 2);
        assert!(kr_titles.contains(&"MONEY"));
        assert!(kr_titles.contains(&"Rockstar"));
        assert!(!kr_titles.contains(&"Gurenge"));
        assert!(!kr_titles.contains(&"Crossing Field"));

        let jp_tracks = store.artist_tracks("ar:dz:418196").unwrap();
        let jp_titles: Vec<&str> = jp_tracks.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(jp_titles.len(), 2);
        assert!(jp_titles.contains(&"Gurenge"));
        assert!(jp_titles.contains(&"Crossing Field"));
        assert!(!jp_titles.contains(&"MONEY"));
        assert!(!jp_titles.contains(&"Rockstar"));

        let kr_albums = store.artist_albums(0, "ar:dz:14559887").unwrap();
        assert!(kr_albums.iter().any(|a| a.name == "LALISA"));
        assert!(!kr_albums.iter().any(|a| a.name == "LEO-NiNE"));

        let jp_albums = store.artist_albums(0, "ar:dz:418196").unwrap();
        assert!(jp_albums.iter().any(|a| a.name == "LEO-NiNE"));
        assert!(!jp_albums.iter().any(|a| a.name == "LALISA"));
    }

    #[test]
    fn featured_artist_summaries_filters_junk_and_ranks_by_quality_not_alphabet() {
        let store = Store::open_memory().unwrap();
        let tracks = vec![
            sample_track("t1", "Song", "!ARTIST", "Alb"),
            sample_track("t2", "Song", "03 2025", "Alb"),
            sample_track("t3", "Song", "07", "Alb"),
            sample_track("t4", "Song", "08 To Sort", "Alb"),
            sample_track("t5", "Song", "'80s Pop Hits", "Alb"),
            sample_track("t6", "Song", "Unknown Artist", "Alb"),
            sample_track("t7", "Song", "! 멜 벅", "Alb"),
            sample_track("t8", "Idol", "YOASOBI", "THE BOOK"),
            sample_track("t9", "Tracing A Dream", "YOASOBI", "THE BOOK"),
        ];
        store.upsert_tracks(&tracks).unwrap();
        store
            .upsert_artist_meta(&ArtistSummary {
                id: "ar:dz:999".into(),
                name: "YOASOBI".into(),
                cover_url: "https://cdn.deezer.com/yoasobi.jpg".into(),
                bio: "500000 fans on Deezer".into(),
                deezer_id: "999".into(),
                ..Default::default()
            })
            .unwrap();

        let featured = store.featured_artist_summaries(5).unwrap();
        let names: Vec<&str> = featured.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names.first().copied(), Some("YOASOBI"), "featured={:?}", names);
        for junk in ["!ARTIST", "03 2025", "07", "08 To Sort", "'80s Pop Hits", "Unknown Artist"] {
            assert!(!names.contains(&junk), "junk leaked into featured: {:?}", names);
        }
    }

    #[test]
    fn consolidates_same_artist_across_sources_without_merging_homonyms() {
        let store = Store::open_memory().unwrap();

        // Yorushika appears across Deezer, YTM (without deezer_artist_id), and legacy/local slug.
        store
            .upsert_artist_meta(&ArtistSummary {
                id: "ar:dz:14873123".into(),
                name: "Yorushika".into(),
                cover_url: "https://cdn.deezer.com/yorushika_official.jpg".into(),
                bio: "250000 fans on Deezer".into(),
                deezer_id: "14873123".into(),
                ..Default::default()
            })
            .unwrap();
        store
            .upsert_artist_meta(&ArtistSummary {
                id: "ar:yorushika".into(),
                name: "Yorushika".into(),
                cover_url: "https://local.example/yorushika_alt.jpg".into(),
                ..Default::default()
            })
            .unwrap();
        store
            .upsert_artist_meta(&ArtistSummary {
                id: "ar:ytm:UC_YORU".into(),
                name: "Yorushika".into(),
                cover_url: "https://ytm.example/yorushika_ytm.jpg".into(),
                ..Default::default()
            })
            .unwrap();

        let mut yoru_dz = sample_track("dz:yoru1", "Say It.", "Yorushika", "Natsukusa ga Jama wo Suru");
        yoru_dz.cover_url = "https://cdn.deezer.com/album1.jpg".into();
        yoru_dz.meta = r#"{"deezer_artist_id":14873123}"#.into();

        let mut yoru_ytm = sample_track("ytm:yoru2", "Spring Thief", "Yorushika", "Creation");
        yoru_ytm.source = "ytm".into();
        yoru_ytm.cover_url = "https://ytm.example/spring_thief.jpg".into();
        yoru_ytm.meta = r#"{"ytm_artist_id":"UC_YORU"}"#.into();

        let mut yoru_local = sample_track("loc:yoru3", "Hitchcock", "Yorushika", "Makeinu ni Encore wa Iranai");
        yoru_local.source = "local".into();
        yoru_local.cover_url = "https://local.example/hitchcock.jpg".into();

        // Homonym artists Korean LISA and Japanese LiSA in the same store must stay separate.
        store
            .upsert_artist_meta(&ArtistSummary {
                id: "ar:dz:14559887".into(),
                name: "LISA".into(),
                cover_url: "https://cdn.deezer.com/lisa_kr.jpg".into(),
                deezer_id: "14559887".into(),
                ..Default::default()
            })
            .unwrap();
        store
            .upsert_artist_meta(&ArtistSummary {
                id: "ar:dz:418196".into(),
                name: "LiSA".into(),
                cover_url: "https://cdn.deezer.com/lisa_jp.jpg".into(),
                deezer_id: "418196".into(),
                ..Default::default()
            })
            .unwrap();
        let mut lisa_ambig = sample_track("loc:lisa_ambig", "Some Song", "Lisa", "Single");
        lisa_ambig.source = "local".into();

        store
            .upsert_tracks(&[yoru_dz, yoru_ytm, yoru_local, lisa_ambig])
            .unwrap();

        let summaries = store.build_artist_summaries().unwrap();
        let yoru_profiles: Vec<&ArtistSummary> = summaries
            .iter()
            .filter(|a| a.name.eq_ignore_ascii_case("Yorushika"))
            .collect();
        assert_eq!(
            yoru_profiles.len(),
            1,
            "Expected exactly 1 Yorushika profile, got: {:?}",
            yoru_profiles
        );
        assert_eq!(yoru_profiles[0].id, "ar:dz:14873123");
        assert_eq!(
            yoru_profiles[0].cover_url,
            "https://cdn.deezer.com/yorushika_official.jpg"
        );
        assert_eq!(yoru_profiles[0].song_count, 3);
        assert_eq!(yoru_profiles[0].album_count, 3);

        let yoru_tracks = store.artist_tracks("ar:dz:14873123").unwrap();
        assert_eq!(yoru_tracks.len(), 3);
        let yoru_albums = store.artist_albums(0, "ar:dz:14873123").unwrap();
        assert_eq!(yoru_albums.len(), 3);

        assert!(summaries.iter().any(|a| a.id == "ar:dz:14559887" && a.name == "LISA"));
        assert!(summaries.iter().any(|a| a.id == "ar:dz:418196" && a.name == "LiSA"));
    }

    #[test]
    fn migrate_artist_disambiguation_v2_merges_legacy_followed_and_meta_safely() {
        let store = Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("tester", "secret123").unwrap();

        // Simulate legacy DB state before v2 migration.
        store
            .upsert_artist_meta(&ArtistSummary {
                id: "ar:dz:14873123".into(),
                name: "Yorushika".into(),
                cover_url: "https://cdn.deezer.com/yorushika_official.jpg".into(),
                deezer_id: "14873123".into(),
                ..Default::default()
            })
            .unwrap();
        store
            .upsert_artist_meta(&ArtistSummary {
                id: "ar:yorushika".into(),
                name: "Yorushika".into(),
                cover_url: "https://local.example/yorushika_alt.jpg".into(),
                ..Default::default()
            })
            .unwrap();
        store.follow_artist(u.id, "ar:yorushika").unwrap();

        // Reset the v2 migration marker and re-run migration.
        {
            let mut conn = store.lock();
            conn.execute("DELETE FROM kv WHERE key = 'artist_disambig_v2'", [])
                .unwrap();
            migrate_artist_disambiguation_v2(&mut conn).unwrap();
        }

        let followed = store.followed_artist_ids(u.id).unwrap();
        assert!(followed.contains("ar:dz:14873123"));
        assert!(!followed.contains("ar:yorushika"));
        assert!(store.get_artist_meta("ar:yorushika").unwrap().is_none());
    }

    #[test]
    fn candidate_tracks_isolate_users_and_serve_popular_charts_to_new_users() {
        let store = Store::open_memory().unwrap();
        let (u1, _) = store.authenticate_or_create_user("user1", "pw").unwrap();
        let (u2, _) = store.authenticate_or_create_user("user2", "pw").unwrap();
        let (u_new, _) = store.authenticate_or_create_user("brandnew", "pw").unwrap();

        let mut charts = Vec::new();
        for i in 0..25 {
            let mut t = sample_track(
                &format!("dz:chart_{}", i),
                &format!("Global Hit {}", i),
                &format!("Chart Artist {}", i),
                "Top Charts",
            );
            t.genre = "Pop".into();
            t.created = 1000 + i;
            charts.push(t);
        }
        store.upsert_tracks(&charts).unwrap();
        store.record_chart_tracks(&charts, 1).unwrap();

        // User 1 searches/listens to a niche track (newer created timestamp)
        let mut u1_niche = sample_track("ytm:u1_niche", "Obscure B-Side", "Niche Act 1", "Deep Cuts");
        u1_niche.source = "ytm".into();
        u1_niche.genre = "Experimental".into();
        u1_niche.created = 9000;

        // User 2 searches/listens to a different niche track (even newer created timestamp)
        let mut u2_niche = sample_track("ytm:u2_niche", "Mom Favorite", "Niche Act 2", "Classics");
        u2_niche.source = "ytm".into();
        u2_niche.genre = "Country".into();
        u2_niche.created = 9500;

        store.upsert_tracks(&[u1_niche.clone(), u2_niche.clone()]).unwrap();
        store.record_event(u1.id, &u1_niche.id, "play", 1.0).unwrap();
        store.record_event(u2.id, &u2_niche.id, "play", 1.0).unwrap();

        // Brand-new user should only see popular chart tracks, not u1's or u2's cached listens.
        let new_pool = store.candidate_tracks_for_user(u_new.id, 100).unwrap();
        let new_ids: HashSet<&str> = new_pool.iter().map(|t| t.id.as_str()).collect();
        assert!(new_ids.contains("dz:chart_0"));
        assert!(!new_ids.contains("ytm:u1_niche"));
        assert!(!new_ids.contains("ytm:u2_niche"));

        // User 1 should see their own track + charts, never User 2's niche listen.
        let u1_pool = store.candidate_tracks_for_user(u1.id, 100).unwrap();
        let u1_ids: HashSet<&str> = u1_pool.iter().map(|t| t.id.as_str()).collect();
        assert!(u1_ids.contains("ytm:u1_niche"));
        assert!(!u1_ids.contains("ytm:u2_niche"));
    }

    #[test]
    fn new_user_cold_slate_never_falls_back_to_cached_or_local_tracks() {
        let store = Store::open_memory().unwrap();
        let (fresh, _) = store.authenticate_or_create_user("fresh", "pw").unwrap();

        // Only cached/local songs exist — no external chart data at all.
        let mut cached = sample_track("soulseek:cached", "Cached Song", "Someone", "Album");
        cached.source = "soulseek".into();
        cached.created = 9000;
        let mut local = sample_track("local:file", "Local Song", "Local Artist", "Album");
        local.source = "local".into();
        local.created = 9500;
        store.upsert_tracks(&[cached.clone(), local.clone()]).unwrap();

        assert!(!store.has_listening_history(fresh.id).unwrap());
        let pool = store.candidate_tracks_for_user(fresh.id, 50).unwrap();
        assert!(pool.is_empty(), "cold users must not receive cached/local songs");

        // When external charts arrive, only those fill the starter slate.
        let chart = sample_track("dz:chart", "Popular Now", "Chart Artist", "Charts");
        store.upsert_tracks(&[chart.clone()]).unwrap();
        store.record_chart_tracks(&[chart.clone()], 1).unwrap();
        let pool = store.candidate_tracks_for_user(fresh.id, 50).unwrap();
        let ids: HashSet<&str> = pool.iter().map(|t| t.id.as_str()).collect();
        assert!(ids.contains("dz:chart"));
        assert!(!ids.contains("soulseek:cached"));
        assert!(!ids.contains("local:file"));

        // Once the user listens to something they are no longer on the cold slate.
        store.record_event(fresh.id, &local.id, "play", 1.0).unwrap();
        assert!(store.has_listening_history(fresh.id).unwrap());
        let pool = store.candidate_tracks_for_user(fresh.id, 50).unwrap();
        assert!(pool.iter().any(|t| t.id == local.id));
    }

    #[test]
    fn relevant_artists_for_releases_prioritises_followed_then_listened() {
        let store = Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("listener", "pw").unwrap();

        store
            .upsert_artist_meta(&ArtistSummary {
                id: "ar:dz:111".into(),
                name: "Followed Artist".into(),
                deezer_id: "111".into(),
                ..Default::default()
            })
            .unwrap();
        store
            .upsert_artist_meta(&ArtistSummary {
                id: "ar:dz:222".into(),
                name: "Listened Artist".into(),
                deezer_id: "222".into(),
                ..Default::default()
            })
            .unwrap();

        let mut followed_track = sample_track("dz:1", "Song One", "Followed Artist", "Album A");
        followed_track.meta = r#"{"deezer_artist_id":111}"#.into();
        let mut listened_track = sample_track("dz:2", "Song Two", "Listened Artist", "Album B");
        listened_track.meta = r#"{"deezer_artist_id":222}"#.into();
        store
            .upsert_tracks(&[followed_track, listened_track.clone()])
            .unwrap();

        // Follow one artist, but only listen to the other.
        store.follow_artist(u.id, "ar:dz:111").unwrap();
        for _ in 0..4 {
            store.record_event(u.id, &listened_track.id, "play", 1.0).unwrap();
        }

        let artists = store.relevant_artists_for_releases(u.id, 8).unwrap();
        assert_eq!(artists.len(), 2);
        assert_eq!(artists[0].id, "ar:dz:111", "followed artist should rank first");
        assert_eq!(artists[1].id, "ar:dz:222", "listened artist should follow");
        assert!(artists.iter().all(|a| !a.deezer_id.is_empty()));
    }

    #[test]
    fn ytm_artist_and_album_ids_resolve_cleanly_after_deezer_disambiguation() {
        let store = Store::open_memory().unwrap();

        let mut ytm_track = sample_track(
            "ytm:kyouran",
            "狂乱 Hey Kids!! - Kyouran Hey Kids!!",
            "THE ORAL CIGARETTES",
            "FIXION",
        );
        ytm_track.source = "ytm".into();
        ytm_track.meta = r#"{"ytm_artist_id":"UC_ORAL_CIG","deezer_artist_id":12556256}"#.into();

        let mut be_track = sample_track("ytm:polaris", "Polaris", "BLUE ENCOUNT", "VECTOR");
        be_track.source = "ytm".into();
        be_track.meta = r#"{"ytm_artist_id":"UC_BLUE_ENC"}"#.into();

        store.upsert_tracks(&[ytm_track.clone(), be_track.clone()]).unwrap();
        store
            .upsert_artist_meta(&ArtistSummary {
                id: "ar:dz:5482289".into(),
                name: "Blue Encount".into(),
                deezer_id: "5482289".into(),
                ..Default::default()
            })
            .unwrap();

        // Even when navigated via the pre-disambiguation YTM artist/album IDs, lookups succeed.
        let oral_summary = store.artist_summary_by_id("ar:ytm:UC_ORAL_CIG").unwrap();
        assert!(oral_summary.is_some());
        assert_eq!(oral_summary.unwrap().id, "ar:dz:12556256");
        assert_eq!(store.artist_tracks("ar:ytm:UC_ORAL_CIG").unwrap().len(), 1);

        let fixion_tracks = store.album_tracks("al:ytm-uc-oral-cig-fixion").unwrap();
        assert_eq!(fixion_tracks.len(), 1);
        assert!(store.album_summary_by_id(0, "al:ytm-uc-oral-cig-fixion").unwrap().is_some());

        let be_summary = store.artist_summary_by_id("ar:ytm:UC_BLUE_ENC").unwrap();
        assert!(be_summary.is_some());
        assert_eq!(be_summary.unwrap().id, "ar:dz:5482289");
        assert_eq!(store.artist_tracks("ar:ytm:UC_BLUE_ENC").unwrap().len(), 1);
    }

    #[test]
    fn cool_cache_registry_namespaces_asset_kinds_without_collisions() {
        let store = Store::open_memory().unwrap();

        store
            .upsert_remote_cache_entry("track:1", "prefix/audio", 1000)
            .unwrap();
        store
            .upsert_asset_cache_entry("embed", "track:1", "prefix/embed/track1", 2000)
            .unwrap();
        store
            .upsert_asset_cache_entry("cover", "al:1", "prefix/cover/al1", 3000)
            .unwrap();

        // The embed shares its track id with the audio entry but must not
        // overwrite it.
        assert_eq!(
            store.get_remote_cache_entry("track:1").unwrap().unwrap().0,
            "prefix/audio"
        );
        assert_eq!(
            store
                .get_asset_cache_entry("embed", "track:1")
                .unwrap()
                .unwrap()
                .0,
            "prefix/embed/track1"
        );
        assert_eq!(store.remote_cache_count().unwrap(), 3);
        assert_eq!(store.remote_cache_total_bytes().unwrap(), 6000);

        let candidates = store.remote_cache_eviction_candidates(10).unwrap();
        assert!(candidates
            .iter()
            .any(|(kind, id, key, _)| kind == "audio" && id == "track:1" && key == "prefix/audio"));
        assert!(candidates.iter().any(|(kind, id, key, _)| kind == "embed"
            && id == "track:1"
            && key == "prefix/embed/track1"));

        store.delete_asset_cache_entry("embed", "track:1").unwrap();
        assert!(store.get_asset_cache_entry("embed", "track:1").unwrap().is_none());
        assert!(store.get_remote_cache_entry("track:1").unwrap().is_some());
    }

    #[test]
    fn cool_cache_legacy_schema_migrates_and_keeps_audio_entries() {
        let dir = std::env::temp_dir().join(format!("beatlab_cool_mig_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("beatlab.db");

        {
            let conn = rusqlite::Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE remote_cache_entries (
                    track_id TEXT PRIMARY KEY,
                    object_key TEXT NOT NULL,
                    bytes INTEGER NOT NULL,
                    created INTEGER NOT NULL
                 );
                 CREATE INDEX idx_remote_cache_created ON remote_cache_entries(created ASC);
                 INSERT INTO remote_cache_entries(track_id, object_key, bytes, created)
                 VALUES ('track:old', 'legacy/key', 4096, 1234567890);",
            )
            .unwrap();
        }

        let store = Store::open(&db_path).unwrap();
        assert_eq!(
            store.get_remote_cache_entry("track:old").unwrap().unwrap().0,
            "legacy/key"
        );
        let candidates = store.remote_cache_eviction_candidates(10).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].0, "audio");
        assert_eq!(candidates[0].1, "track:old");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
