package subsonic

// The Navidrome native API (/api/*). Clients like Feishin, when
// the server is added in "Navidrome" mode, only use the Subsonic
// API for streaming and a few calls — library browsing, playlist
// management and the user list go through Navidrome's own REST
// API. This file implements that surface on top of the same
// store, with Navidrome's object shapes (song/album/artist/
// playlist), its pagination convention (_start/_end/_sort/_order
// plus an X-Total-Count header), and its playlist-track
// convention: a playlist entry's id is the entry id, and
// mediaFileId carries the real song id.
//
// Auth is the session token from POST /auth/login, presented as
// "x-nd-authorization: Bearer <token>".
//
// Not implemented: shares and image upload/delete (501).

import (
	"encoding/json"
	"net/http"
	"sort"
	"strconv"
	"strings"
	"time"

	"beatlab/internal/store"
)

func ndTS(t time.Time) string {
	if t.IsZero() {
		return "2024-01-01T00:00:00.000Z"
	}
	return t.UTC().Format("2006-01-02T15:04:05.000Z")
}

func ndJSON(w http.ResponseWriter, status int, v any) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	if v == nil {
		w.Write([]byte("null"))
		return
	}
	json.NewEncoder(w).Encode(v)
}

func ndErr(w http.ResponseWriter, status int, msg string) {
	ndJSON(w, status, map[string]any{"error": msg})
}

// ndCtx carries the per-request user data ND objects embed.
type ndCtx struct {
	store   *store.Store
	user    *store.User
	starred map[string]bool
	ratings map[string]int
	plays   map[string]int
}

func (a *API) ndAuth(w http.ResponseWriter, r *http.Request) (*ndCtx, bool) {
	u, ok := a.ndUser(r.Header.Get("X-ND-Authorization"))
	if !ok {
		ndErr(w, http.StatusUnauthorized, "Unauthorized")
		return nil, false
	}
	starred := map[string]bool{}
	for id := range a.store.Starred(u.ID) {
		starred[id] = true
	}
	ctx := &ndCtx{
		store:   a.store,
		user:    u,
		starred: starred,
		ratings: a.store.Ratings(u.ID),
		plays:   a.taste.BuildProfile(u.ID).TrackPlays,
	}
	return ctx, true
}

func ndGenreObj(name string) map[string]any {
	return map[string]any{"id": name, "name": name}
}

func (c *ndCtx) song(t store.Track) map[string]any {
	bitRate := t.Bitrate
	if bitRate <= 0 && t.Duration > 0 && t.Size > 0 {
		bitRate = int(t.Size * 8 / int64(t.Duration) / 1000)
	}
	if bitRate <= 0 {
		bitRate = 128
	}
	var genres any
	if t.Genre != "" {
		genres = []map[string]any{ndGenreObj(t.Genre)}
	}
	m := map[string]any{
		"id":                   t.ID,
		"title":                t.Title,
		"album":                t.Album,
		"albumId":              t.AlbumID(),
		"artist":               t.Artist,
		"artistId":             t.ArtistID(),
		"albumArtist":          t.Artist,
		"albumArtistId":        t.ArtistID(),
		"bitRate":              bitRate,
		"bookmarkPosition":     0,
		"compilation":          false,
		"createdAt":            ndTS(t.Created),
		"discNumber":           1,
		"duration":             float64(t.Duration),
		"genre":                t.Genre,
		"genres":               genres,
		"hasCoverArt":          true, // real art when it exists, generated tile otherwise — getCoverArt always answers
		"orderAlbumArtistName": t.Artist,
		"orderAlbumName":       t.Album,
		"orderArtistName":      t.Artist,
		"orderTitle":           t.Title,
		"path":                 t.ID,
		"playCount":            c.plays[t.ID],
		"sampleRate":           44100,
		"size":                 t.Size,
		"starred":              c.starred[t.ID],
		"suffix":               t.Format,
		"trackNumber":          t.TrackNo,
		"updatedAt":            ndTS(t.Created),
		"year":                 t.Year,
	}
	if r := c.ratings[t.ID]; r > 0 {
		m["rating"] = r
	}
	return m
}

func (c *ndCtx) album(g albumGroup) map[string]any {
	var size int64
	var duration float64
	minYear, maxYear := 0, 0
	genre := ""
	created := time.Time{}
	starred := false
	for _, t := range g.tracks {
		size += t.Size
		duration += float64(t.Duration)
		if t.Year > 0 && (minYear == 0 || t.Year < minYear) {
			minYear = t.Year
		}
		if t.Year > maxYear {
			maxYear = t.Year
		}
		if genre == "" && t.Genre != "" {
			genre = t.Genre
		}
		if created.IsZero() || t.Created.Before(created) {
			created = t.Created
		}
		if c.starred[t.ID] {
			starred = true
		}
	}
	var genres any
	if genre != "" {
		genres = []map[string]any{ndGenreObj(genre)}
	}
	artistTrack := store.Track{Artist: g.artist}
	return map[string]any{
		"id":                    g.id,
		"name":                  g.name,
		"coverArtId":            g.id,
		"albumArtist":           g.artist,
		"albumArtistId":         artistTrack.ArtistID(),
		"artist":                g.artist,
		"artistId":              artistTrack.ArtistID(),
		"compilation":           false,
		"createdAt":             ndTS(created),
		"duration":              duration,
		"externalInfoUpdatedAt": nil,
		"genre":                 genre,
		"genres":                genres,
		"libraryId":             1,
		"libraryName":           "Beatlab",
		"libraryPath":           "",
		"maxYear":               maxYear,
		"minYear":               minYear,
		"orderAlbumArtistName":  g.artist,
		"orderAlbumName":        g.name,
		"size":                  size,
		"songCount":             len(g.tracks),
		"starred":               starred,
		"updatedAt":             ndTS(created),
	}
}

func (c *ndCtx) artist(name string) map[string]any {
	tracks, _ := c.store.QueryTracks(store.Query{Artist: name, Limit: 4000})
	albums := map[string]bool{}
	var size int64
	for _, t := range tracks {
		albums[t.AlbumID()] = true
		size += t.Size
	}
	tr := store.Track{Artist: name}
	return map[string]any{
		"id":         tr.ArtistID(),
		"name":       name,
		"albumCount": len(albums),
		"songCount":  len(tracks),
		"size":       size,
	}
}

// ---------- pagination / sorting helpers ----------

func ndPage(w http.ResponseWriter, r *http.Request, total int) (start, end int) {
	q := r.URL.Query()
	start, _ = strconv.Atoi(q.Get("_start"))
	end = total
	if v := q.Get("_end"); v != "" {
		if e, err := strconv.Atoi(v); err == nil && e >= 0 {
			end = e
		}
	}
	if start < 0 {
		start = 0
	}
	if start > total {
		start = total
	}
	if end > total {
		end = total
	}
	if end < start {
		end = start
	}
	w.Header().Set("X-Total-Count", strconv.Itoa(total))
	return start, end
}

func ndSortSongs(songs []map[string]any, q map[string][]string) {
	key := ""
	if v := q["_sort"]; len(v) > 0 {
		key = v[0]
	}
	desc := len(q["_order"]) > 0 && strings.EqualFold(q["_order"][0], "DESC")
	get := func(m map[string]any) any {
		switch key {
		case "title":
			return m["orderTitle"]
		case "album":
			return m["orderAlbumName"]
		case "artist":
			return m["orderArtistName"]
		case "year", "duration", "playCount", "trackNumber":
			return m[key]
		case "createdAt":
			return m["createdAt"]
		default:
			return m["id"]
		}
	}
	sort.SliceStable(songs, func(i, j int) bool {
		a, b := get(songs[i]), get(songs[j])
		less := false
		switch av := a.(type) {
		case string:
			bv, _ := b.(string)
			less = strings.ToLower(av) < strings.ToLower(bv)
		case int:
			bv, _ := b.(int)
			less = av < bv
		case float64:
			bv, _ := b.(float64)
			less = av < bv
		}
		if desc {
			return !less
		}
		return less
	})
}

// ---------- handler ----------

func (a *API) NDHandler() http.Handler {
	mux := http.NewServeMux()
	mux.HandleFunc("GET /api/song", a.ndSongList)
	mux.HandleFunc("GET /api/song/{id}", a.ndSongDetail)
	mux.HandleFunc("GET /api/album", a.ndAlbumList)
	mux.HandleFunc("GET /api/album/{id}", a.ndAlbumDetail)
	mux.HandleFunc("GET /api/artist", a.ndArtistList)
	mux.HandleFunc("GET /api/artist/{id}", a.ndArtistDetail)
	mux.HandleFunc("GET /api/genre", a.ndGenreList)
	mux.HandleFunc("GET /api/tag", a.ndTagList)
	mux.HandleFunc("GET /api/user", a.ndUserList)
	mux.HandleFunc("GET /api/playlist", a.ndPlaylistList)
	mux.HandleFunc("POST /api/playlist", a.ndPlaylistCreate)
	mux.HandleFunc("GET /api/playlist/{id}", a.ndPlaylistDetail)
	mux.HandleFunc("PUT /api/playlist/{id}", a.ndPlaylistUpdate)
	mux.HandleFunc("DELETE /api/playlist/{id}", a.ndPlaylistDelete)
	mux.HandleFunc("GET /api/playlist/{id}/tracks", a.ndPlaylistSongs)
	mux.HandleFunc("POST /api/playlist/{id}/tracks", a.ndPlaylistAdd)
	mux.HandleFunc("DELETE /api/playlist/{id}/tracks", a.ndPlaylistRemove)
	mux.HandleFunc("PUT /api/playlist/{playlistId}/tracks/{trackNumber}", a.ndPlaylistMove)
	mux.HandleFunc("GET /api/queue", a.ndQueueGet)
	mux.HandleFunc("POST /api/queue", a.ndQueueSave)
	mux.HandleFunc("GET /api/radio", a.ndRadioList)
	mux.HandleFunc("POST /api/radio", a.ndRadioCreate)
	mux.HandleFunc("PUT /api/radio/{id}", a.ndRadioUpdate)
	mux.HandleFunc("DELETE /api/radio/{id}", a.ndRadioDelete)
	notImpl := func(w http.ResponseWriter, r *http.Request) {
		ndErr(w, http.StatusNotImplemented, "not supported by Beatlab")
	}
	mux.HandleFunc("POST /api/playlist/{id}/image", notImpl)
	mux.HandleFunc("DELETE /api/playlist/{id}/image", notImpl)
	mux.HandleFunc("POST /api/artist/{id}/image", notImpl)
	mux.HandleFunc("DELETE /api/artist/{id}/image", notImpl)
	mux.HandleFunc("POST /api/radio/{id}/image", notImpl)
	mux.HandleFunc("DELETE /api/radio/{id}/image", notImpl)
	mux.HandleFunc("POST /api/share", notImpl)
	return mux
}
