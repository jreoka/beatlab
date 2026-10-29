// Package store is Beatlab's persistence layer: a single SQLite file
// (pure-Go driver, so the binary stays static) holding users, the
// track catalog learned from sources, listening events (the raw
// material the taste engine learns from), stars/ratings, playlists
// and the stream-cache index.
package store

import (
	"database/sql"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"

	_ "modernc.org/sqlite"
)

type Store struct {
	db *sql.DB
}

type User struct {
	ID       int64
	Username string
	Password string // Subsonic API password (see README: token auth needs it recoverable)
	IsAdmin  bool
}

type Track struct {
	ID        string // "<source>:<source-track-id>"
	Source    string
	Title     string
	Artist    string
	Album     string
	Genre     string
	Year      int
	TrackNo   int
	Duration  int // seconds
	Bitrate   int // kbps
	Format    string // suffix, e.g. mp3
	Size      int64
	StreamURL string // direct upstream URL (archive.org download / resolved)
	CoverURL  string
	Meta      string // source-specific JSON
	PlayCount int
	LastPlayed time.Time
	Created   time.Time
}

func (t *Track) ArtistID() string { return "ar:" + slug(t.Artist) }
func (t *Track) AlbumID() string  { return "al:" + slug(t.Artist) + "|" + slug(t.Album) }

func slug(s string) string {
	s = strings.ToLower(strings.TrimSpace(s))
	var b strings.Builder
	for _, r := range s {
		if (r >= 'a' && r <= 'z') || (r >= '0' && r <= '9') {
			b.WriteRune(r)
		} else if r == ' ' || r == '-' || r == '_' {
			b.WriteByte('-')
		}
	}
	out := strings.Trim(b.String(), "-")
	if out == "" {
		out = "unknown"
	}
	return out
}

// ArtistNameFromID and AlbumPartsFromID invert the virtual IDs.
func ArtistNameFromID(db *Store, id string) (string, bool) {
	rows, err := db.QueryTracks(Query{ArtistID: id, Limit: 1})
	if err != nil || len(rows) == 0 {
		return "", false
	}
	return rows[0].Artist, true
}

const schema = `
CREATE TABLE IF NOT EXISTS users(
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  username TEXT UNIQUE NOT NULL,
  password TEXT NOT NULL,
  is_admin INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS tracks(
  id TEXT PRIMARY KEY,
  source TEXT NOT NULL,
  title TEXT NOT NULL,
  artist TEXT NOT NULL DEFAULT '',
  album TEXT NOT NULL DEFAULT '',
  genre TEXT NOT NULL DEFAULT '',
  year INTEGER NOT NULL DEFAULT 0,
  track_no INTEGER NOT NULL DEFAULT 0,
  duration INTEGER NOT NULL DEFAULT 0,
  bitrate INTEGER NOT NULL DEFAULT 0,
  format TEXT NOT NULL DEFAULT '',
  size INTEGER NOT NULL DEFAULT 0,
  stream_url TEXT NOT NULL DEFAULT '',
  cover_url TEXT NOT NULL DEFAULT '',
  meta TEXT NOT NULL DEFAULT '',
  play_count INTEGER NOT NULL DEFAULT 0,
  last_played INTEGER NOT NULL DEFAULT 0,
  created INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_tracks_artist ON tracks(artist);
CREATE INDEX IF NOT EXISTS idx_tracks_album ON tracks(artist, album);
CREATE TABLE IF NOT EXISTS events(
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  user_id INTEGER NOT NULL,
  track_id TEXT NOT NULL,
  kind TEXT NOT NULL,           -- play | skip | star | unstar | rate
  value REAL NOT NULL DEFAULT 0,
  created INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_events_user ON events(user_id, created);
CREATE TABLE IF NOT EXISTS stars(
  user_id INTEGER NOT NULL, track_id TEXT NOT NULL, created INTEGER NOT NULL,
  PRIMARY KEY(user_id, track_id)
);
CREATE TABLE IF NOT EXISTS ratings(
  user_id INTEGER NOT NULL, track_id TEXT NOT NULL, rating INTEGER NOT NULL,
  PRIMARY KEY(user_id, track_id)
);
CREATE TABLE IF NOT EXISTS playlists(
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  owner INTEGER NOT NULL,
  name TEXT NOT NULL,
  kind TEXT NOT NULL DEFAULT 'manual',  -- manual | radio
  seed TEXT NOT NULL DEFAULT '',
  song_ids TEXT NOT NULL DEFAULT '[]',
  updated INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS cache_files(
  track_id TEXT PRIMARY KEY,
  path TEXT NOT NULL,
  bytes INTEGER NOT NULL,
  last_access INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS kv(
  k TEXT PRIMARY KEY, v TEXT NOT NULL
);
`

func Open(dataDir string) (*Store, error) {
	if err := os.MkdirAll(dataDir, 0o755); err != nil {
		return nil, err
	}
	db, err := sql.Open("sqlite", filepath.Join(dataDir, "beatlab.db")+"?_pragma=busy_timeout(5000)&_pragma=journal_mode(WAL)&_pragma=synchronous(NORMAL)")
	if err != nil {
		return nil, err
	}
	db.SetMaxOpenConns(1)
	if _, err := db.Exec(schema); err != nil {
		return nil, fmt.Errorf("schema: %w", err)
	}
	return &Store{db: db}, nil
}

func (s *Store) Close() error { return s.db.Close() }
func (s *Store) DB() *sql.DB  { return s.db }

// ---- users ----

func (s *Store) CreateUser(username, password string, admin bool) (*User, error) {
	a := 0
	if admin {
		a = 1
	}
	res, err := s.db.Exec(`INSERT INTO users(username, password, is_admin) VALUES(?,?,?)`, username, password, a)
	if err != nil {
		return nil, err
	}
	id, _ := res.LastInsertId()
	return &User{ID: id, Username: username, Password: password, IsAdmin: admin}, nil
}

func (s *Store) UserByName(username string) (*User, bool) {
	var u User
	var adm int
	err := s.db.QueryRow(`SELECT id, username, password, is_admin FROM users WHERE username=?`, username).
		Scan(&u.ID, &u.Username, &u.Password, &adm)
	if err != nil {
		return nil, false
	}
	u.IsAdmin = adm != 0
	return &u, true
}

func (s *Store) Users() []User {
	rows, err := s.db.Query(`SELECT id, username, password, is_admin FROM users ORDER BY username`)
	if err != nil {
		return nil
	}
	defer rows.Close()
	var out []User
	for rows.Next() {
		var u User
		var adm int
		if rows.Scan(&u.ID, &u.Username, &u.Password, &adm) == nil {
			u.IsAdmin = adm != 0
			out = append(out, u)
		}
	}
	return out
}

func (s *Store) DeleteUser(id int64) error {
	_, err := s.db.Exec(`DELETE FROM users WHERE id=?`, id)
	return err
}

func (s *Store) SetPassword(id int64, password string) error {
	_, err := s.db.Exec(`UPDATE users SET password=? WHERE id=?`, password, id)
	return err
}

func (s *Store) UserCount() int {
	var n int
	s.db.QueryRow(`SELECT COUNT(*) FROM users`).Scan(&n)
	return n
}

// ---- tracks ----

const trackCols = `id, source, title, artist, album, genre, year, track_no, duration, bitrate, format, size, stream_url, cover_url, meta, play_count, last_played, created`

func scanTrack(rows *sql.Rows) (Track, error) {
	var t Track
	var lp, cr int64
	err := rows.Scan(&t.ID, &t.Source, &t.Title, &t.Artist, &t.Album, &t.Genre, &t.Year,
		&t.TrackNo, &t.Duration, &t.Bitrate, &t.Format, &t.Size, &t.StreamURL, &t.CoverURL,
		&t.Meta, &t.PlayCount, &lp, &cr)
	if lp > 0 {
		t.LastPlayed = time.Unix(lp, 0)
	}
	if cr > 0 {
		t.Created = time.Unix(cr, 0)
	}
	return t, err
}

func (s *Store) UpsertTrack(t *Track) error {
	now := time.Now().Unix()
	if t.Created.IsZero() {
		t.Created = time.Unix(now, 0)
	}
	_, err := s.db.Exec(`INSERT INTO tracks(`+trackCols+`) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)
		ON CONFLICT(id) DO UPDATE SET title=excluded.title, artist=excluded.artist, album=excluded.album,
		genre=excluded.genre, year=excluded.year, track_no=excluded.track_no, duration=excluded.duration,
		bitrate=excluded.bitrate, format=excluded.format, size=excluded.size, stream_url=excluded.stream_url,
		cover_url=excluded.cover_url, meta=excluded.meta`,
		t.ID, t.Source, t.Title, t.Artist, t.Album, t.Genre, t.Year, t.TrackNo, t.Duration,
		t.Bitrate, t.Format, t.Size, t.StreamURL, t.CoverURL, t.Meta, t.PlayCount,
		t.LastPlayed.Unix(), t.Created.Unix())
	return err
}

func (s *Store) Track(id string) (*Track, bool) {
	rows, err := s.db.Query(`SELECT `+trackCols+` FROM tracks WHERE id=?`, id)
	if err != nil {
		return nil, false
	}
	defer rows.Close()
	if rows.Next() {
		t, err := scanTrack(rows)
		if err == nil {
			return &t, true
		}
	}
	return nil, false
}

type Query struct {
	Text     string // LIKE over title/artist/album
	Artist   string
	ArtistID string
	AlbumID  string
	Genre    string
	Order    string // column expression, e.g. "play_count DESC"
	Limit    int
	Offset   int
	Seed     int64 // for ORDER BY random-ish stable shuffle when Order=="shuffle"
}

func (s *Store) QueryTracks(q Query) ([]Track, error) {
	var where []string
	var args []any
	if q.Text != "" {
		where = append(where, `(title LIKE ? OR artist LIKE ? OR album LIKE ?)`)
		like := "%" + q.Text + "%"
		args = append(args, like, like, like)
	}
	if q.Artist != "" {
		where = append(where, `artist=?`)
		args = append(args, q.Artist)
	}
	if q.Genre != "" {
		where = append(where, `genre=?`)
		args = append(args, q.Genre)
	}
	sqlStr := `SELECT ` + trackCols + ` FROM tracks`
	if len(where) > 0 {
		sqlStr += ` WHERE ` + strings.Join(where, " AND ")
	}
	switch q.Order {
	case "":
		sqlStr += ` ORDER BY artist, album, track_no, title`
	case "shuffle":
		sqlStr += ` ORDER BY ((CAST(substr(id,1,4) AS BLOB) IS NOT NULL), (rowid * 2654435761 + ?) % 1000003)`
		args = append(args, q.Seed)
	default:
		sqlStr += ` ORDER BY ` + q.Order
	}
	if q.Limit > 0 {
		sqlStr += fmt.Sprintf(` LIMIT %d`, q.Limit)
		if q.Offset > 0 {
			sqlStr += fmt.Sprintf(` OFFSET %d`, q.Offset)
		}
	}
	rows, err := s.db.Query(sqlStr, args...)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []Track
	for rows.Next() {
		t, err := scanTrack(rows)
		if err != nil {
			return nil, err
		}
		// virtual-ID filters applied post-query (they derive from fields)
		if q.ArtistID != "" && t.ArtistID() != q.ArtistID {
			continue
		}
		if q.AlbumID != "" && t.AlbumID() != q.AlbumID {
			continue
		}
		out = append(out, t)
	}
	return out, nil
}

func (s *Store) TrackCount() int {
	var n int
	s.db.QueryRow(`SELECT COUNT(*) FROM tracks`).Scan(&n)
	return n
}

func (s *Store) Artists() []string {
	rows, err := s.db.Query(`SELECT DISTINCT artist FROM tracks WHERE artist<>'' ORDER BY artist COLLATE NOCASE`)
	if err != nil {
		return nil
	}
	defer rows.Close()
	var out []string
	for rows.Next() {
		var a string
		if rows.Scan(&a) == nil {
			out = append(out, a)
		}
	}
	return out
}

func (s *Store) Genres() map[string]int {
	rows, err := s.db.Query(`SELECT genre, COUNT(*) FROM tracks WHERE genre<>'' GROUP BY genre ORDER BY COUNT(*) DESC`)
	if err != nil {
		return nil
	}
	defer rows.Close()
	out := map[string]int{}
	for rows.Next() {
		var g string
		var n int
		if rows.Scan(&g, &n) == nil {
			out[g] = n
		}
	}
	return out
}

func (s *Store) RecordPlay(trackID string) {
	s.db.Exec(`UPDATE tracks SET play_count=play_count+1, last_played=? WHERE id=?`, time.Now().Unix(), trackID)
}

// ---- events / learning ----

func (s *Store) AddEvent(userID int64, trackID, kind string, value float64) error {
	_, err := s.db.Exec(`INSERT INTO events(user_id, track_id, kind, value, created) VALUES(?,?,?,?,?)`,
		userID, trackID, kind, value, time.Now().Unix())
	return err
}

type Event struct {
	TrackID string
	Kind    string
	Value   float64
	Created time.Time
}

func (s *Store) Events(userID int64, since time.Time) []Event {
	rows, err := s.db.Query(`SELECT track_id, kind, value, created FROM events WHERE user_id=? AND created>=? ORDER BY created`,
		userID, since.Unix())
	if err != nil {
		return nil
	}
	defer rows.Close()
	var out []Event
	for rows.Next() {
		var e Event
		var cr int64
		if rows.Scan(&e.TrackID, &e.Kind, &e.Value, &cr) == nil {
			e.Created = time.Unix(cr, 0)
			out = append(out, e)
		}
	}
	return out
}

// ---- stars & ratings ----

func (s *Store) Star(userID int64, trackID string) {
	s.db.Exec(`INSERT OR REPLACE INTO stars(user_id, track_id, created) VALUES(?,?,?)`, userID, trackID, time.Now().Unix())
	s.AddEvent(userID, trackID, "star", 1)
}
func (s *Store) Unstar(userID int64, trackID string) {
	s.db.Exec(`DELETE FROM stars WHERE user_id=? AND track_id=?`, userID, trackID)
	s.AddEvent(userID, trackID, "unstar", 1)
}
func (s *Store) Starred(userID int64) map[string]time.Time {
	rows, err := s.db.Query(`SELECT track_id, created FROM stars WHERE user_id=?`, userID)
	if err != nil {
		return nil
	}
	defer rows.Close()
	out := map[string]time.Time{}
	for rows.Next() {
		var id string
		var cr int64
		if rows.Scan(&id, &cr) == nil {
			out[id] = time.Unix(cr, 0)
		}
	}
	return out
}
func (s *Store) SetRating(userID int64, trackID string, rating int) {
	if rating <= 0 {
		s.db.Exec(`DELETE FROM ratings WHERE user_id=? AND track_id=?`, userID, trackID)
		return
	}
	s.db.Exec(`INSERT OR REPLACE INTO ratings(user_id, track_id, rating) VALUES(?,?,?)`, userID, trackID, rating)
	s.AddEvent(userID, trackID, "rate", float64(rating))
}
func (s *Store) Ratings(userID int64) map[string]int {
	rows, err := s.db.Query(`SELECT track_id, rating FROM ratings WHERE user_id=?`, userID)
	if err != nil {
		return nil
	}
	defer rows.Close()
	out := map[string]int{}
	for rows.Next() {
		var id string
		var r int
		if rows.Scan(&id, &r) == nil {
			out[id] = r
		}
	}
	return out
}

// ---- playlists ----

type Playlist struct {
	ID      int64
	Owner   int64
	Name    string
	Kind    string
	Seed    string
	SongIDs []string
	Updated time.Time
}

func (s *Store) SavePlaylist(p *Playlist) error {
	ids, _ := json.Marshal(p.SongIDs)
	p.Updated = time.Now()
	if p.ID == 0 {
		res, err := s.db.Exec(`INSERT INTO playlists(owner, name, kind, seed, song_ids, updated) VALUES(?,?,?,?,?,?)`,
			p.Owner, p.Name, p.Kind, p.Seed, string(ids), p.Updated.Unix())
		if err != nil {
			return err
		}
		p.ID, _ = res.LastInsertId()
		return nil
	}
	_, err := s.db.Exec(`UPDATE playlists SET name=?, kind=?, seed=?, song_ids=?, updated=? WHERE id=?`,
		p.Name, p.Kind, p.Seed, string(ids), p.Updated.Unix(), p.ID)
	return err
}

func scanPlaylist(rows *sql.Rows) (Playlist, error) {
	var p Playlist
	var ids string
	var up int64
	err := rows.Scan(&p.ID, &p.Owner, &p.Name, &p.Kind, &p.Seed, &ids, &up)
	json.Unmarshal([]byte(ids), &p.SongIDs)
	p.Updated = time.Unix(up, 0)
	return p, err
}

func (s *Store) Playlists(owner int64) []Playlist {
	rows, err := s.db.Query(`SELECT id, owner, name, kind, seed, song_ids, updated FROM playlists WHERE owner=? ORDER BY name`, owner)
	if err != nil {
		return nil
	}
	defer rows.Close()
	var out []Playlist
	for rows.Next() {
		if p, err := scanPlaylist(rows); err == nil {
			out = append(out, p)
		}
	}
	return out
}

func (s *Store) Playlist(id int64) (*Playlist, bool) {
	rows, err := s.db.Query(`SELECT id, owner, name, kind, seed, song_ids, updated FROM playlists WHERE id=?`, id)
	if err != nil {
		return nil, false
	}
	defer rows.Close()
	if rows.Next() {
		p, err := scanPlaylist(rows)
		if err == nil {
			return &p, true
		}
	}
	return nil, false
}

func (s *Store) DeletePlaylist(id int64) {
	s.db.Exec(`DELETE FROM playlists WHERE id=?`, id)
}

// ---- cache index ----

type CacheEntry struct {
	TrackID    string
	Path       string
	Bytes      int64
	LastAccess time.Time
}

func (s *Store) CachePut(trackID, path string, bytes int64) {
	s.db.Exec(`INSERT OR REPLACE INTO cache_files(track_id, path, bytes, last_access) VALUES(?,?,?,?)`,
		trackID, path, bytes, time.Now().Unix())
}
func (s *Store) CacheGet(trackID string) (*CacheEntry, bool) {
	var e CacheEntry
	var la int64
	err := s.db.QueryRow(`SELECT track_id, path, bytes, last_access FROM cache_files WHERE track_id=?`, trackID).
		Scan(&e.TrackID, &e.Path, &e.Bytes, &la)
	if err != nil {
		return nil, false
	}
	e.LastAccess = time.Unix(la, 0)
	return &e, true
}
func (s *Store) CacheTouch(trackID string) {
	s.db.Exec(`UPDATE cache_files SET last_access=? WHERE track_id=?`, time.Now().Unix(), trackID)
}
func (s *Store) CacheDelete(trackID string) {
	s.db.Exec(`DELETE FROM cache_files WHERE track_id=?`, trackID)
}
func (s *Store) CacheEntries() []CacheEntry {
	rows, err := s.db.Query(`SELECT track_id, path, bytes, last_access FROM cache_files ORDER BY last_access ASC`)
	if err != nil {
		return nil
	}
	defer rows.Close()
	var out []CacheEntry
	for rows.Next() {
		var e CacheEntry
		var la int64
		if rows.Scan(&e.TrackID, &e.Path, &e.Bytes, &la) == nil {
			e.LastAccess = time.Unix(la, 0)
			out = append(out, e)
		}
	}
	return out
}
func (s *Store) CacheBytes() int64 {
	var n sql.NullInt64
	s.db.QueryRow(`SELECT SUM(bytes) FROM cache_files`).Scan(&n)
	return n.Int64
}

// ---- kv ----

func (s *Store) KVGet(k string) (string, bool) {
	var v string
	err := s.db.QueryRow(`SELECT v FROM kv WHERE k=?`, k).Scan(&v)
	return v, err == nil
}
func (s *Store) KVSet(k, v string) {
	s.db.Exec(`INSERT OR REPLACE INTO kv(k, v) VALUES(?,?)`, k, v)
}
