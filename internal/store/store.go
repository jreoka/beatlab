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
	"hash/fnv"
	"log"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"time"
	"unicode"

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
	ID         string // "<source>:<source-track-id>"
	Source     string
	Title      string
	Artist     string
	Album      string
	Genre      string
	Year       int
	TrackNo    int
	Duration   int    // seconds
	Bitrate    int    // kbps
	Format     string // suffix, e.g. mp3
	Size       int64
	StreamURL  string // direct upstream URL (archive.org download / resolved)
	CoverURL   string
	Meta       string // source-specific JSON
	PlayCount  int
	LastPlayed time.Time
	Created    time.Time
}

func (t *Track) ArtistID() string { return "ar:" + slug(t.Artist) }

// SongKey identifies the song itself, ignoring which upload or
// version a track happens to be: "Carry On Wayward Son", its
// remaster and a live rendition share one key. Queues that play
// tracks back to back (radio, similar) dedupe on it so the same
// song never repeats in different dress.
func (t Track) SongKey() string {
	return slug(t.Artist) + "|" + coreSongTitle(t.Title)
}

// SongAliases are the alternate song identities hidden in a
// title's bracketed segments. Japanese uploads routinely
// carry the translated title in brackets — "Itte (Say It)
// [Karaoke/Off-Vocal]" IS "Say It" — but coreSongTitle drops
// bracketed content wholesale, so the karaoke of the seed
// song survived dedupe under its Japanese name alone. Each
// bracketed segment, normalized the same way, is an alias
// key for queue dedupe.
func (t Track) SongAliases() []string {
	primary := t.SongKey()
	var out []string
	for _, seg := range bracketSegments(t.Title) {
		key := slug(t.Artist) + "|" + coreSongTitle(seg)
		if coreSongTitle(seg) != "" && key != primary {
			out = append(out, key)
		}
	}
	return out
}

// bracketSegments returns the contents of every bracketed
// segment ((), [], and the CJK bracket pairs) in s.
func bracketSegments(s string) []string {
	var segs []string
	var cur strings.Builder
	depth := 0
	for _, r := range s {
		switch r {
		case '(', '[', '\u3010', '\u300e', '\u300c':
			if depth == 0 {
				cur.Reset()
			}
			depth++
		case ')', ']', '\u3011', '\u300f', '\u300d':
			if depth > 0 {
				depth--
				if depth == 0 && cur.Len() > 0 {
					segs = append(segs, cur.String())
				}
			}
		default:
			if depth > 0 {
				cur.WriteRune(r)
			}
		}
	}
	return segs
}

// coreSongTitle normalizes a title for SongKey: brackets and
// their contents dropped, punctuation removed, and trailing
// version words (remaster, live, radio edit, a bare year, ...)
// stripped. Mid-title words are untouched — "Live and Let Die"
// keeps its "live".
func coreSongTitle(s string) string {
	var b strings.Builder
	depth := 0
	for _, r := range s {
		switch r {
		case '(', '[', '\u3010', '\u300e', '\u300c':
			depth++
		case ')', ']', '\u3011', '\u300f', '\u300d':
			if depth > 0 {
				depth--
			}
		default:
			if depth == 0 {
				b.WriteRune(r)
			}
		}
	}
	type tok struct {
		low  string
		caps bool
	}
	var toks []tok
	for _, wd := range strings.Fields(b.String()) {
		wd = strings.Trim(wd, "-/&,.:;'\"!?")
		if wd == "" {
			continue
		}
		low := strings.ToLower(wd)
		if isDateToken(low) {
			// "2026.04.04" in a title is an event date,
			// never song identity — wherever it sits.
			continue
		}
		caps := len(wd) >= 3 && wd == strings.ToUpper(wd) && strings.IndexFunc(wd, unicode.IsLetter) >= 0
		toks = append(toks, tok{low, caps})
	}
	versionWords := map[string]bool{
		"remaster": true, "remastered": true, "live": true,
		"edit": true, "version": true, "mono": true, "stereo": true,
		"mix": true, "radio": true, "single": true, "album": true,
		"instrumental": true, "inst": true, "karaoke": true,
		"tv": true, "size": true, "ver": true,
		// Upload-quality dressing, not song identity:
		// "Carry On Wayward Son HD" is the same song.
		"hd": true, "hq": true, "4k": true, "8k": true,
		"official": true, "video": true, "audio": true, "mv": true,
		"lyrics": true, "lyric": true, "visualizer": true,
	}
	for len(toks) > 1 {
		last := toks[len(toks)-1].low
		if versionWords[last] || isYearToken(last) {
			toks = toks[:len(toks)-1]
			continue
		}
		if last == "fest" || last == "fes" || last == "festival" {
			// "... CENTRAL FEST 2026": the word before FEST
			// is the event's name, not the song's.
			toks = toks[:len(toks)-1]
			if len(toks) > 1 && toks[len(toks)-1].caps {
				toks = toks[:len(toks)-1]
			}
			continue
		}
		break
	}
	words := make([]string, len(toks))
	for i, t := range toks {
		words[i] = t.low
	}
	return strings.Join(words, " ")
}

// isDateToken matches full dates like 2026.04.04, 2026-4-4
// and 2025/10/12 — not bare years (a song can be called
// "1979"; it cannot be called "1979.12.31").
func isDateToken(s string) bool {
	parts := strings.FieldsFunc(s, func(r rune) bool { return r == '.' || r == '-' || r == '/' })
	if len(parts) != 2 && len(parts) != 3 {
		return false
	}
	if !isYearToken(parts[0]) {
		return false
	}
	for _, part := range parts[1:] {
		if len(part) < 1 || len(part) > 2 {
			return false
		}
		for _, r := range part {
			if r < '0' || r > '9' {
				return false
			}
		}
	}
	return true
}

func isYearToken(s string) bool {
	if len(s) != 4 {
		return false
	}
	for _, r := range s {
		if r < '0' || r > '9' {
			return false
		}
	}
	return s >= "1900" && s <= "2099"
}
func (t *Track) AlbumID() string { return "al:" + slug(t.Artist) + "|" + slug(t.Album) }

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
		if s == "" {
			return "unknown"
		}
		// No latin letters or digits at all (ヨルシカ, a fully
		// Cyrillic name, ...): the name would collapse into the
		// shared "unknown" bucket, merging unrelated artists.
		// Fall back to a stable hash of the name instead —
		// distinct names get distinct ids, repeatably.
		h := fnv.New64a()
		h.Write([]byte(s))
		return "u" + strconv.FormatUint(h.Sum64(), 16)
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

func (s *Store) UserByID(id int64) (*User, bool) {
	var u User
	var adm int
	err := s.db.QueryRow(`SELECT id, username, password, is_admin FROM users WHERE id=?`, id).
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
	// On conflict, an incoming field only replaces the stored
	// one when it actually carries a value: a later search that
	// re-returns this track with partial metadata (no duration,
	// unresolved stream URL) must not downgrade the catalog
	// entry — worst case an emptied stream_url breaks playback.
	keepText := func(col string) string {
		return col + `=CASE WHEN excluded.` + col + `<>'' THEN excluded.` + col + ` ELSE tracks.` + col + ` END`
	}
	keepNum := func(col string) string {
		return col + `=CASE WHEN excluded.` + col + `>0 THEN excluded.` + col + ` ELSE tracks.` + col + ` END`
	}
	setClause := strings.Join([]string{
		keepText("title"), keepText("artist"), keepText("album"), keepText("genre"),
		keepNum("year"), keepNum("track_no"), keepNum("duration"), keepNum("bitrate"),
		keepText("format"), keepNum("size"), keepText("stream_url"), keepText("cover_url"),
		keepText("meta"),
	}, ", ")
	_, err := s.db.Exec(`INSERT INTO tracks(`+trackCols+`) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)
		ON CONFLICT(id) DO UPDATE SET `+setClause,
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
		// Word-wise AND across title/artist/album: a search for
		// "say it yorushika" must match title="Say It" +
		// artist="Yorushika" — a single full-phrase LIKE matches
		// almost nothing real.
		esc := strings.NewReplacer(`\`, `\\`, `%`, `\%`, `_`, `\_`)
		for _, word := range strings.Fields(q.Text) {
			where = append(where, `(title LIKE ? ESCAPE '\' OR artist LIKE ? ESCAPE '\' OR album LIKE ? ESCAPE '\')`)
			like := "%" + esc.Replace(word) + "%"
			args = append(args, like, like, like)
		}
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
	case "last_played DESC":
		sqlStr += ` ORDER BY last_played DESC`
	case "created DESC":
		sqlStr += ` ORDER BY created DESC`
	case "play_count DESC":
		sqlStr += ` ORDER BY play_count DESC`
	case "shuffle":
		sqlStr += ` ORDER BY (rowid * 2654435761 + ?) % 1000003`
		args = append(args, q.Seed)
	default:
		// Anything else falls back to the default ordering —
		// the Order field is never interpolated into SQL.
		sqlStr += ` ORDER BY artist, album, track_no, title`
	}
	// Virtual-ID filters (AlbumID/ArtistID) are applied in Go
	// after the query — a SQL LIMIT would cut the catalog down
	// BEFORE filtering and silently return nothing for most
	// albums once the catalog outgrows the limit. Only push
	// LIMIT/OFFSET into SQL when no virtual filter is present.
	virtual := q.ArtistID != "" || q.AlbumID != ""
	if !virtual && (q.Limit > 0 || q.Offset > 0) {
		if q.Limit > 0 {
			sqlStr += fmt.Sprintf(` LIMIT %d`, q.Limit)
		} else {
			sqlStr += ` LIMIT -1`
		}
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
		if q.ArtistID != "" && t.ArtistID() != q.ArtistID {
			continue
		}
		if q.AlbumID != "" && t.AlbumID() != q.AlbumID {
			continue
		}
		out = append(out, t)
	}
	if err := rows.Err(); err != nil {
		return nil, err
	}
	if virtual {
		if q.Offset > 0 {
			if q.Offset < len(out) {
				out = out[q.Offset:]
			} else {
				out = []Track{}
			}
		}
		if q.Limit > 0 && len(out) > q.Limit {
			out = out[:q.Limit]
		}
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
	if _, err := s.db.Exec(`UPDATE tracks SET play_count=play_count+1, last_played=? WHERE id=?`, time.Now().Unix(), trackID); err != nil {
		log.Printf("[beatlab] store: record play %s: %v", trackID, err)
	}
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

// starTx writes the star/rating row and its taste event in a
// single transaction, so the two can never desync (a starred
// track the taste engine never heard about, or vice versa).
func (s *Store) starTx(userID int64, trackID, stmt string, args []any, kind string, value float64) {
	tx, err := s.db.Begin()
	if err != nil {
		log.Printf("[beatlab] store: %s tx begin: %v", kind, err)
		return
	}
	defer tx.Rollback()
	if _, err := tx.Exec(stmt, args...); err != nil {
		log.Printf("[beatlab] store: %s %s: %v", kind, trackID, err)
		return
	}
	if _, err := tx.Exec(`INSERT INTO events(user_id, track_id, kind, value, created) VALUES(?,?,?,?,?)`,
		userID, trackID, kind, value, time.Now().Unix()); err != nil {
		log.Printf("[beatlab] store: %s event %s: %v", kind, trackID, err)
		return
	}
	if err := tx.Commit(); err != nil {
		log.Printf("[beatlab] store: %s commit %s: %v", kind, trackID, err)
	}
}

func (s *Store) Star(userID int64, trackID string) {
	s.starTx(userID, trackID,
		`INSERT OR REPLACE INTO stars(user_id, track_id, created) VALUES(?,?,?)`,
		[]any{userID, trackID, time.Now().Unix()}, "star", 1)
}
func (s *Store) Unstar(userID int64, trackID string) {
	s.starTx(userID, trackID,
		`DELETE FROM stars WHERE user_id=? AND track_id=?`,
		[]any{userID, trackID}, "unstar", 1)
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
		if _, err := s.db.Exec(`DELETE FROM ratings WHERE user_id=? AND track_id=?`, userID, trackID); err != nil {
			log.Printf("[beatlab] store: unrate %s: %v", trackID, err)
		}
		return
	}
	s.starTx(userID, trackID,
		`INSERT OR REPLACE INTO ratings(user_id, track_id, rating) VALUES(?,?,?)`,
		[]any{userID, trackID, rating}, "rate", float64(rating))
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
	if p.SongIDs == nil {
		p.SongIDs = []string{}
	}
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
	res, err := s.db.Exec(`UPDATE playlists SET name=?, kind=?, seed=?, song_ids=?, updated=? WHERE id=? AND owner=?`,
		p.Name, p.Kind, p.Seed, string(ids), p.Updated.Unix(), p.ID, p.Owner)
	if err != nil {
		return err
	}
	if n, _ := res.RowsAffected(); n == 0 {
		return fmt.Errorf("playlist %d not found for owner %d", p.ID, p.Owner)
	}
	return nil
}

func scanPlaylist(rows *sql.Rows) (Playlist, error) {
	var p Playlist
	var ids string
	var up int64
	err := rows.Scan(&p.ID, &p.Owner, &p.Name, &p.Kind, &p.Seed, &ids, &up)
	if uerr := json.Unmarshal([]byte(ids), &p.SongIDs); uerr != nil {
		log.Printf("[beatlab] store: playlist %d has corrupt song_ids: %v", p.ID, uerr)
	}
	if p.SongIDs == nil {
		p.SongIDs = []string{}
	}
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
	if _, err := s.db.Exec(`INSERT OR REPLACE INTO cache_files(track_id, path, bytes, last_access) VALUES(?,?,?,?)`,
		trackID, path, bytes, time.Now().Unix()); err != nil {
		log.Printf("[beatlab] store: cache put %s: %v", trackID, err)
	}
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
	if _, err := s.db.Exec(`UPDATE cache_files SET last_access=? WHERE track_id=?`, time.Now().Unix(), trackID); err != nil {
		log.Printf("[beatlab] store: cache touch %s: %v", trackID, err)
	}
}

// CacheDelete removes a cache index row and reports whether
// one existed — the eviction loop uses it to prove progress.
func (s *Store) CacheDelete(trackID string) bool {
	res, err := s.db.Exec(`DELETE FROM cache_files WHERE track_id=?`, trackID)
	if err != nil {
		log.Printf("[beatlab] store: cache delete %s: %v", trackID, err)
		return false
	}
	n, _ := res.RowsAffected()
	return n > 0
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
	if _, err := s.db.Exec(`INSERT OR REPLACE INTO kv(k, v) VALUES(?,?)`, k, v); err != nil {
		log.Printf("[beatlab] store: kv set %s: %v", k, err)
	}
}
