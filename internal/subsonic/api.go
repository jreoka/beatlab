// Package subsonic implements the Subsonic / OpenSubsonic REST API
// surface that Navidrome-compatible clients speak, backed by
// Beatlab's sources, cache engine and taste engine.
package subsonic

import (
	"crypto/md5"
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"net/http"
	"strconv"
	"strings"
	"sync"
	"time"

	"beatlab/internal/engine"
	"beatlab/internal/sources"
	"beatlab/internal/store"
	"beatlab/internal/taste"
)

const (
	apiVersion    = "1.16.1"
	serverType    = "beatlab"
	serverVersion = "0.1.0"
)

// ---------- response tree ----------

// Elem is a tiny ordered XML/JSON tree. Subsonic's JSON form maps
// elements to objects, repeated children to arrays, attributes to
// fields; the renderer below applies those conventions.
type Elem struct {
	Name     string
	Attrs    []attr
	Children []*Elem
	Text     string
}

type attr struct {
	k string
	v any
}

func E(name string, children ...*Elem) *Elem { return &Elem{Name: name, Children: children} }

func (e *Elem) A(key string, val any) *Elem {
	if val == nil {
		return e
	}
	switch v := val.(type) {
	case string:
		if v == "" {
			return e
		}
	case *string:
		if v == nil || *v == "" {
			return e
		}
		val = *v
	}
	e.Attrs = append(e.Attrs, attr{key, val})
	return e
}

func (e *Elem) Add(children ...*Elem) *Elem {
	e.Children = append(e.Children, children...)
	return e
}

func (e *Elem) TextOf(text string) *Elem { e.Text = text; return e }

func xmlEscape(s string) string {
	r := strings.NewReplacer("&", "&amp;", "<", "&lt;", ">", "&gt;", `"`, "&quot;", "'", "&apos;")
	return r.Replace(s)
}

func (e *Elem) writeXML(sb *strings.Builder) {
	sb.WriteString("<" + e.Name)
	for _, a := range e.Attrs {
		sb.WriteString(fmt.Sprintf(` %s="%s"`, a.k, xmlEscape(fmt.Sprint(a.v))))
	}
	if len(e.Children) == 0 && e.Text == "" {
		sb.WriteString("/>")
		return
	}
	sb.WriteString(">")
	sb.WriteString(xmlEscape(e.Text))
	for _, c := range e.Children {
		c.writeXML(sb)
	}
	sb.WriteString("</" + e.Name + ">")
}

func (e *Elem) toJSON() any {
	obj := map[string]any{}
	for _, a := range e.Attrs {
		obj[a.k] = a.v
	}
	if e.Text != "" {
		obj["value"] = e.Text
	}
	groups := map[string][]any{}
	var order []string
	for _, c := range e.Children {
		if _, ok := groups[c.Name]; !ok {
			order = append(order, c.Name)
		}
		groups[c.Name] = append(groups[c.Name], c.toJSON())
	}
	for _, name := range order {
		items := groups[name]
		if len(items) == 1 {
			obj[name] = items[0]
		} else {
			obj[name] = items
		}
	}
	return obj
}

// ---------- API ----------

type nowPlaying struct {
	track   store.Track
	started time.Time
	player  string
}

type API struct {
	store    *store.Store
	registry *sources.Registry
	engine   *engine.Engine
	taste    *taste.Engine

	mu      sync.Mutex
	playing map[int64]*nowPlaying // userID -> now playing
	started map[int64]time.Time
}

func New(st *store.Store, reg *sources.Registry, eng *engine.Engine, te *taste.Engine) *API {
	return &API{
		store: st, registry: reg, engine: eng, taste: te,
		playing: map[int64]*nowPlaying{},
		started: map[int64]time.Time{},
	}
}

func (a *API) ok(root *Elem, extraAttrs ...attr) *Elem {
	resp := &Elem{Name: "subsonic-response"}
	resp.Attrs = []attr{
		{"status", "ok"}, {"version", apiVersion}, {"type", serverType},
		{"serverVersion", serverVersion}, {"openSubsonic", true},
	}
	resp.Attrs = append(resp.Attrs, extraAttrs...)
	if root != nil {
		resp.Children = append(resp.Children, root)
	}
	return resp
}

func (a *API) fail(code int, msg string) *Elem {
	resp := a.ok(nil)
	resp.Attrs[0].v = "failed"
	err := E("error").A("code", code).A("message", msg)
	resp.Children = append(resp.Children, err)
	return resp
}

func (a *API) write(w http.ResponseWriter, r *http.Request, resp *Elem) {
	if wantsJSON(r) {
		w.Header().Set("Content-Type", "application/json")
		body := map[string]any{"subsonic-response": resp.toJSON()}
		json.NewEncoder(w).Encode(body)
		return
	}
	w.Header().Set("Content-Type", "text/xml; charset=utf-8")
	var sb strings.Builder
	sb.WriteString(`<?xml version="1.0" encoding="UTF-8"?>` + "\n")
	// xmlns goes on the root
	root := *resp
	root.Attrs = append([]attr{{"xmlns", "http://subsonic.org/restapi"}}, resp.Attrs...)
	root.writeXML(&sb)
	w.Write([]byte(sb.String()))
}

func wantsJSON(r *http.Request) bool {
	return r.URL.Query().Get("f") == "json"
}

// ---------- auth ----------

// ---------- Navidrome native login (Feishin "Navidrome" mode) ----------

// Some clients (Feishin in its Navidrome server mode) don't log in
// with Subsonic credentials — they POST /auth/login like
// Navidrome's own web UI and expect {id, username, token} back.
// Feishin rejects the login with "no user id or username
// available" when those fields are missing. The returned token is
// then presented on API calls in the X-ND-Authorization header.
func (a *API) NDLoginHandler() http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		var creds struct {
			Username string `json:"username"`
			Password string `json:"password"`
		}
		if r.Method == http.MethodPost {
			// JSON body (Navidrome clients) or form fallback.
			json.NewDecoder(r.Body).Decode(&creds)
			if creds.Username == "" {
				r.ParseForm()
				creds.Username = r.FormValue("username")
				creds.Password = r.FormValue("password")
			}
		}
		user, ok := a.store.UserByName(creds.Username)
		if !ok || creds.Password == "" || creds.Password != user.Password {
			w.WriteHeader(http.StatusUnauthorized)
			w.Write([]byte(`{"error":"Invalid username or password"}`))
			return
		}
		buf := make([]byte, 32)
		rand.Read(buf)
		token := hex.EncodeToString(buf)
		a.store.KVSet("ndtoken:"+token, strconv.FormatInt(user.ID, 10))
		json.NewEncoder(w).Encode(map[string]any{
			"id":       strconv.FormatInt(user.ID, 10),
			"username": user.Username,
			"name":     user.Username,
			"token":    token,
			"isAdmin":  user.IsAdmin,
		})
	}
}

// ndUser resolves a Navidrome session token to its user.
func (a *API) ndUser(token string) (*store.User, bool) {
	token = strings.TrimSpace(strings.TrimPrefix(token, "Bearer "))
	if token == "" {
		return nil, false
	}
	idStr, ok := a.store.KVGet("ndtoken:" + token)
	if !ok {
		return nil, false
	}
	id, err := strconv.ParseInt(idStr, 10, 64)
	if err != nil {
		return nil, false
	}
	return a.store.UserByID(id)
}

// authenticate validates Subsonic credentials: password (plain or
// enc:hex), token md5(password+salt), or apiKey (treated as the
// account password in this build). A Navidrome session token (from
// /auth/login) is also accepted, via the X-ND-Authorization header
// or in place of the password.
func (a *API) authenticate(r *http.Request) (*store.User, *Elem) {
	if u, ok := a.ndUser(r.Header.Get("X-ND-Authorization")); ok {
		return u, nil
	}
	if u, ok := a.ndUser(r.Header.Get("Authorization")); ok {
		return u, nil
	}
	q := r.URL.Query()
	username := q.Get("u")
	if username == "" {
		username = q.Get("username")
	}
	user, ok := a.store.UserByName(username)
	if !ok {
		return nil, a.fail(40, "Wrong username or password")
	}
	if p := q.Get("p"); p != "" {
		if u, ok := a.ndUser(p); ok && u.ID == user.ID {
			return user, nil
		}
	}
	p := q.Get("p")
	if strings.HasPrefix(p, "enc:") {
		if raw, err := hex.DecodeString(strings.TrimPrefix(p, "enc:")); err == nil {
			p = string(raw)
		}
	}
	if p != "" && p == user.Password {
		return user, nil
	}
	if t, s := q.Get("t"), q.Get("s"); t != "" && s != "" {
		sum := md5.Sum([]byte(user.Password + s))
		if hex.EncodeToString(sum[:]) == strings.ToLower(t) {
			return user, nil
		}
	}
	if k := q.Get("apiKey"); k != "" && k == user.Password {
		return user, nil
	}
	return nil, a.fail(40, "Wrong username or password")
}

// ---------- routing ----------

// RefreshRadioFor rebuilds a user's radio playlists immediately
// (used by the admin console).
func (a *API) RefreshRadioFor(r *http.Request, user *store.User) {
	a.ensureRadioPlaylists(r, user, true)
}

func (a *API) Handler() http.Handler {
	mux := http.NewServeMux()
	mux.HandleFunc("/rest/", a.dispatch)
	return mux
}

func (a *API) dispatch(w http.ResponseWriter, r *http.Request) {
	name := strings.TrimPrefix(r.URL.Path, "/rest/")
	name = strings.TrimSuffix(name, ".view")
	name = strings.TrimSuffix(name, ".json")

	// binary endpoints authenticate but answer raw
	switch name {
	case "stream", "download":
		user, fail := a.authenticate(r)
		if fail != nil {
			a.write(w, r, fail)
			return
		}
		_ = user
		a.handleStream(w, r)
		return
	case "getCoverArt":
		if _, fail := a.authenticate(r); fail != nil {
			a.write(w, r, fail)
			return
		}
		a.handleCover(w, r)
		return
	case "getOpenSubsonicExtensions":
		a.handleExtensions(w, r)
		return
	}

	user, fail := a.authenticate(r)
	if fail != nil {
		a.write(w, r, fail)
		return
	}

	var resp *Elem
	switch name {
	case "ping":
		resp = a.ok(nil)
	case "getLicense":
		resp = a.ok(E("license").A("valid", true).A("email", "beatlab@local").A("licenseExpires", "2099-01-01T00:00:00.000Z"))
	case "getMusicFolders":
		resp = a.ok(E("musicFolders", E("musicFolder").A("id", 1).A("name", "Beatlab")))
	case "getUser":
		resp = a.hGetUser(r, user)
	case "getUsers":
		resp = a.hGetUsers(r, user)
	case "getIndexes", "getMusicDirectory":
		resp = a.hDirectory(r, user)
	case "getArtists":
		resp = a.hArtists(r, user)
	case "getArtist":
		resp = a.hArtist(r, user)
	case "getAlbum":
		resp = a.hAlbum(r, user)
	case "getSong":
		resp = a.hSong(r, user)
	case "search3", "search2":
		resp = a.hSearch(r, user)
	case "getAlbumList2", "getAlbumList":
		resp = a.hAlbumList(r, user)
	case "getRandomSongs":
		resp = a.hRandomSongs(r, user)
	case "getSongsByGenre":
		resp = a.hSongsByGenre(r, user)
	case "getGenres":
		resp = a.hGenres(r, user)
	case "getStarred2", "getStarred":
		resp = a.hStarred(r, user)
	case "star":
		resp = a.hStar(r, user, true)
	case "unstar":
		resp = a.hStar(r, user, false)
	case "setRating":
		resp = a.hSetRating(r, user)
	case "scrobble":
		resp = a.hScrobble(r, user)
	case "getPlaylists":
		resp = a.hGetPlaylists(r, user)
	case "getPlaylist":
		resp = a.hGetPlaylist(r, user)
	case "createPlaylist":
		resp = a.hCreatePlaylist(r, user)
	case "updatePlaylist":
		resp = a.hUpdatePlaylist(r, user)
	case "deletePlaylist":
		resp = a.hDeletePlaylist(r, user)
	case "getSimilarSongs2", "getSimilarSongs":
		resp = a.hSimilar(r, user, name == "getSimilarSongs2")
	case "getArtistInfo2", "getArtistInfo":
		resp = a.hArtistInfo(r, user)
	case "getTopSongs":
		resp = a.hTopSongs(r, user)
	case "getNowPlaying":
		resp = a.hNowPlaying(r, user)
	case "getScanStatus":
		resp = a.ok(E("scanStatus").A("scanning", false).A("count", a.store.TrackCount()))
	case "startScan":
		a.ensureRadioPlaylists(r, user, true)
		resp = a.ok(E("scanStatus").A("scanning", false).A("count", a.store.TrackCount()))
	case "savePlayQueue":
		resp = a.hSavePlayQueue(r, user)
	case "getPlayQueue":
		resp = a.hGetPlayQueue(r, user)
	case "getLyrics":
		resp = a.ok(E("lyrics"))
	case "getInternetRadioStations":
		resp = a.ok(E("internetRadioStations"))
	case "jukeboxControl":
		resp = a.fail(0, "Jukebox is not supported by Beatlab")
	default:
		resp = a.fail(0, "Unknown endpoint: "+name)
	}
	a.write(w, r, resp)
}

// ---------- shared helpers ----------

func paramInt(r *http.Request, key string, def int) int {
	v := r.URL.Query().Get(key)
	if v == "" {
		return def
	}
	n, err := strconv.Atoi(v)
	if err != nil {
		return def
	}
	return n
}

func (a *API) songElem(t store.Track, user *store.User) *Elem {
	starred := a.store.Starred(user.ID)
	ratings := a.store.Ratings(user.ID)
	e := E("song").
		A("id", t.ID).
		A("parent", t.AlbumID()).
		A("isDir", false).
		A("title", t.Title).
		A("album", t.Album).
		A("artist", t.Artist).
		A("albumId", t.AlbumID()).
		A("artistId", t.ArtistID()).
		A("coverArt", t.ID).
		A("size", t.Size).
		A("contentType", engine.ContentType(t.Format)).
		A("suffix", t.Format).
		A("path", t.Artist+"/"+t.Album+"/"+t.Title).
		A("type", "music").
		A("isVideo", false)
	if t.TrackNo > 0 {
		e.A("track", t.TrackNo)
	}
	if t.Year > 0 {
		e.A("year", t.Year)
	}
	if t.Genre != "" {
		e.A("genre", t.Genre)
	}
	if t.Duration > 0 {
		e.A("duration", t.Duration)
	}
	if t.Bitrate > 0 {
		e.A("bitRate", t.Bitrate)
	}
	if t.PlayCount > 0 {
		e.A("playCount", t.PlayCount)
	}
	if !t.Created.IsZero() {
		e.A("created", t.Created.Format("2006-01-02T15:04:05.000Z"))
	}
	if at, ok := starred[t.ID]; ok {
		e.A("starred", at.Format("2006-01-02T15:04:05.000Z"))
	}
	if r, ok := ratings[t.ID]; ok {
		e.A("userRating", r)
	}
	return e
}

type albumGroup struct {
	id     string
	name   string
	artist string
	tracks []store.Track
}

func (a *API) groupAlbums(tracks []store.Track) []albumGroup {
	byID := map[string]*albumGroup{}
	var order []string
	for _, t := range tracks {
		g, ok := byID[t.AlbumID()]
		if !ok {
			g = &albumGroup{id: t.AlbumID(), name: t.Album, artist: t.Artist}
			byID[t.AlbumID()] = g
			order = append(order, t.AlbumID())
		}
		g.tracks = append(g.tracks, t)
	}
	out := make([]albumGroup, 0, len(order))
	for _, id := range order {
		out = append(out, *byID[id])
	}
	return out
}

func (a *API) albumElem(g albumGroup, user *store.User) *Elem {
	dur := 0
	year := 0
	genre := ""
	created := time.Time{}
	for _, t := range g.tracks {
		dur += t.Duration
		if t.Year > year {
			year = t.Year
		}
		if genre == "" {
			genre = t.Genre
		}
		if created.IsZero() || t.Created.Before(created) {
			created = t.Created
		}
	}
	artist := g.artist
	artistID := ""
	if len(g.tracks) > 0 {
		artistID = g.tracks[0].ArtistID()
	}
	e := E("album").
		A("id", g.id).
		A("parent", artistID).
		A("isDir", true).
		A("name", g.name).
		A("album", g.name).
		A("artist", artist).
		A("artistId", artistID).
		A("songCount", len(g.tracks)).
		A("duration", dur).
		A("playCount", sumPlays(g.tracks))
	if len(g.tracks) > 0 {
		e.A("coverArt", g.tracks[0].ID)
	}
	if year > 0 {
		e.A("year", year)
	}
	if genre != "" {
		e.A("genre", genre)
	}
	if !created.IsZero() {
		e.A("created", created.Format("2006-01-02T15:04:05.000Z"))
	}
	return e
}

func sumPlays(ts []store.Track) int {
	n := 0
	for _, t := range ts {
		n += t.PlayCount
	}
	return n
}

func (a *API) handleExtensions(w http.ResponseWriter, r *http.Request) {
	if wantsJSON(r) {
		w.Header().Set("Content-Type", "application/json")
		w.Write([]byte(`{"subsonic-response":{"status":"ok","version":"` + apiVersion + `","type":"` + serverType + `","serverVersion":"` + serverVersion + `","openSubsonic":true,"openSubsonicExtensions":[]}}`))
		return
	}
	w.Header().Set("Content-Type", "text/xml; charset=utf-8")
	w.Write([]byte(`<?xml version="1.0" encoding="UTF-8"?>` + "\n" +
		`<subsonic-response xmlns="http://subsonic.org/restapi" status="ok" version="` + apiVersion + `" type="` + serverType + `" serverVersion="` + serverVersion + `" openSubsonic="true"><openSubsonicExtensions/></subsonic-response>`))
}
