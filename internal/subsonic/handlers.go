package subsonic

import (
	"encoding/json"
	"fmt"
	"net/http"
	"sort"
	"strconv"
	"strings"
	"time"

	"beatlab/internal/store"
)

// ---------- users ----------

func userElem(u *store.User) *Elem {
	e := E("user").
		A("username", u.Username).
		A("scrobblingEnabled", true).
		A("adminRole", u.IsAdmin).
		A("settingsRole", true).
		A("downloadRole", true).
		A("uploadRole", false).
		A("playlistRole", true).
		A("coverArtRole", true).
		A("commentRole", true).
		A("podcastRole", false).
		A("streamRole", true).
		A("jukeboxRole", false).
		A("shareRole", false).
		A("videoConversionRole", false).
		A("avatarLastChanged", "2024-01-01T00:00:00.000Z")
	e.Add(E("folder").TextOf("1"))
	return e
}

func (a *API) hGetUser(r *http.Request, user *store.User) *Elem {
	name := r.URL.Query().Get("username")
	if name == "" || name == user.Username {
		return a.ok(userElem(user))
	}
	if !user.IsAdmin {
		return a.fail(50, "Not authorized")
	}
	if other, ok := a.store.UserByName(name); ok {
		return a.ok(userElem(other))
	}
	return a.fail(70, "User not found")
}

func (a *API) hGetUsers(r *http.Request, user *store.User) *Elem {
	root := E("users")
	if user.IsAdmin {
		for _, u := range a.store.Users() {
			uu := u
			root.Add(userElem(&uu))
		}
	} else {
		root.Add(userElem(user))
	}
	return a.ok(root)
}

// ---------- browsing ----------

func artistElem(name string, albumCount int) *Elem {
	t := store.Track{Artist: name}
	return E("artist").
		A("id", t.ArtistID()).
		A("name", name).
		A("albumCount", albumCount).
		A("coverArt", t.ArtistID())
}

func (a *API) albumsOf(artist string) []albumGroup {
	tracks, _ := a.store.QueryTracks(store.Query{Artist: artist, Limit: 2000})
	return a.groupAlbums(tracks)
}

func (a *API) hArtists(r *http.Request, user *store.User) *Elem {
	root := E("artists").A("ignoredArticles", "")
	byLetter := map[string][]string{}
	var letters []string
	for _, name := range a.store.Artists() {
		if name == "" || strings.EqualFold(name, "Unknown Artist") {
			continue
		}
		letter := "#"
		if name != "" {
			letter = strings.ToUpper(name[:1])
		}
		if _, ok := byLetter[letter]; !ok {
			letters = append(letters, letter)
		}
		byLetter[letter] = append(byLetter[letter], name)
	}
	sort.Strings(letters)
	for _, letter := range letters {
		idx := E("index").A("name", letter)
		for _, name := range byLetter[letter] {
			idx.Add(artistElem(name, len(a.albumsOf(name))))
		}
		root.Add(idx)
	}
	return a.ok(root)
}

func (a *API) hDirectory(r *http.Request, user *store.User) *Elem {
	id := r.URL.Query().Get("id")
	// Root or an artist: list albums (and artists at root).
	if id == "" || id == "root" || id == "1" {
		dir := E("directory").A("id", "root").A("name", "Beatlab")
		for _, name := range a.store.Artists() {
			dir.Add(artistElem(name, len(a.albumsOf(name))))
		}
		return a.ok(dir)
	}
	if strings.HasPrefix(id, "ar:") {
		name, ok := store.ArtistNameFromID(a.store, id)
		if !ok {
			return a.fail(70, "Artist not found")
		}
		dir := E("directory").A("id", id).A("name", name)
		for _, g := range a.albumsOf(name) {
			dir.Add(a.albumElem(g, user))
		}
		return a.ok(dir)
	}
	if strings.HasPrefix(id, "al:") {
		tracks, _ := a.store.QueryTracks(store.Query{AlbumID: id, Limit: 500})
		if len(tracks) == 0 {
			return a.fail(70, "Album not found")
		}
		dir := E("directory").A("id", id).A("name", tracks[0].Album).A("parent", tracks[0].ArtistID())
		for _, t := range tracks {
			dir.Add(a.songElem(t, user))
		}
		return a.ok(dir)
	}
	return a.fail(70, "Not found")
}

func (a *API) hArtist(r *http.Request, user *store.User) *Elem {
	id := r.URL.Query().Get("id")
	name, ok := store.ArtistNameFromID(a.store, id)
	if !ok {
		return a.fail(70, "Artist not found")
	}
	root := artistElem(name, 0)
	albums := a.albumsOf(name)
	root.Attrs = root.Attrs[:0]
	t := store.Track{Artist: name}
	root.A("id", t.ArtistID()).A("name", name).A("albumCount", len(albums)).A("coverArt", t.ArtistID())
	for _, g := range albums {
		root.Add(a.albumElem(g, user))
	}
	return a.ok(root)
}

func (a *API) hAlbum(r *http.Request, user *store.User) *Elem {
	id := r.URL.Query().Get("id")
	tracks, _ := a.store.QueryTracks(store.Query{AlbumID: id, Limit: 500})
	if len(tracks) == 0 {
		return a.fail(70, "Album not found")
	}
	groups := a.groupAlbums(tracks)
	root := a.albumElem(groups[0], user)
	for _, t := range tracks {
		root.Add(a.songElem(t, user))
	}
	return a.ok(root)
}

func (a *API) hSong(r *http.Request, user *store.User) *Elem {
	id := r.URL.Query().Get("id")
	t, ok := a.store.Track(id)
	if !ok {
		return a.fail(70, "Song not found")
	}
	return a.ok(a.songElem(*t, user))
}

// ---------- search ----------

func (a *API) hSearch(r *http.Request, user *store.User) *Elem {
	q := r.URL.Query()
	query := q.Get("query")
	artistCount := paramInt(r, "artistCount", 20)
	albumCount := paramInt(r, "albumCount", 20)
	songCount := paramInt(r, "songCount", 20)
	artistOffset := paramInt(r, "artistOffset", 0)
	albumOffset := paramInt(r, "albumOffset", 0)
	songOffset := paramInt(r, "songOffset", 0)

	// Live source search feeds the catalog (Bitlab-style: results
	// warm into the local library as they're found). The merged
	// result list leads with what the sources returned, in source
	// order — those matched by the catalog's own indexing, which a
	// literal LIKE over local fields would miss.
	var tracks []store.Track
	seenIDs := map[string]bool{}
	if query != "" {
		for _, t := range a.registry.SearchAll(r.Context(), query, 40) {
			tt := t
			a.store.UpsertTrack(&tt)
			if !seenIDs[tt.ID] {
				seenIDs[tt.ID] = true
				tracks = append(tracks, tt)
			}
		}
	}
	if more, _ := a.store.QueryTracks(store.Query{Text: query, Limit: 500}); len(more) > 0 {
		for _, t := range more {
			if !seenIDs[t.ID] {
				seenIDs[t.ID] = true
				tracks = append(tracks, t)
			}
		}
	}
	root := E("searchResult3")

	artistSet := []string{}
	seenA := map[string]bool{}
	for _, t := range tracks {
		if !seenA[t.Artist] && t.Artist != "" {
			seenA[t.Artist] = true
			artistSet = append(artistSet, t.Artist)
		}
	}
	for i, name := range artistSet {
		if i < artistOffset || len(root.Children) >= artistOffset+artistCount {
			continue
		}
		root.Add(artistElem(name, len(a.albumsOf(name))))
	}
	albums := a.groupAlbums(tracks)
	albumRoot := E("searchResult3")
	_ = albumRoot
	albumElems := []*Elem{}
	for i, g := range albums {
		if i < albumOffset {
			continue
		}
		if len(albumElems) >= albumCount {
			break
		}
		albumElems = append(albumElems, a.albumElem(g, user))
	}
	root.Add(albumElems...)
	songN := 0
	for i, t := range tracks {
		if i < songOffset {
			continue
		}
		if songN >= songCount {
			break
		}
		songN++
		root.Add(a.songElem(t, user))
	}
	return a.ok(root)
}

// ---------- album lists ----------

func (a *API) hAlbumList(r *http.Request, user *store.User) *Elem {
	typ := r.URL.Query().Get("type")
	size := paramInt(r, "size", 10)
	offset := paramInt(r, "offset", 0)
	var tracks []store.Track
	switch typ {
	case "recent":
		tracks, _ = a.store.QueryTracks(store.Query{Order: "last_played DESC", Limit: 2000})
	case "newest":
		tracks, _ = a.store.QueryTracks(store.Query{Order: "created DESC", Limit: 2000})
	case "frequent":
		tracks, _ = a.store.QueryTracks(store.Query{Order: "play_count DESC", Limit: 2000})
	case "random":
		tracks, _ = a.store.QueryTracks(store.Query{Order: "shuffle", Seed: time.Now().Unix(), Limit: 2000})
	case "starred":
		starred := a.store.Starred(user.ID)
		all, _ := a.store.QueryTracks(store.Query{Limit: 2000})
		for _, t := range all {
			if _, ok := starred[t.ID]; ok {
				tracks = append(tracks, t)
			}
		}
	case "byGenre":
		tracks, _ = a.store.QueryTracks(store.Query{Genre: r.URL.Query().Get("genre"), Limit: 2000})
	case "byYear":
		from := paramInt(r, "fromYear", 0)
		to := paramInt(r, "toYear", 9999)
		all, _ := a.store.QueryTracks(store.Query{Limit: 2000})
		for _, t := range all {
			if t.Year >= from && t.Year <= to {
				tracks = append(tracks, t)
			}
		}
	default: // alphabeticalByName / alphabeticalByArtist
		tracks, _ = a.store.QueryTracks(store.Query{Limit: 2000})
	}
	albums := a.groupAlbums(tracks)
	// Browse lists are discovery surfaces: unattributed uploads
	// ("Unknown Artist" one-offs) are noise there. They stay in
	// the catalog and in search — just not on the home shelves.
	if typ != "starred" {
		kept := albums[:0]
		for _, g := range albums {
			if !strings.EqualFold(g.artist, "Unknown Artist") && g.artist != "" {
				kept = append(kept, g)
			}
		}
		albums = kept
	}
	if typ == "alphabeticalByArtist" {
		sort.Slice(albums, func(i, j int) bool { return albums[i].artist < albums[j].artist })
	}
	root := E("albumList2")
	for i, g := range albums {
		if i < offset {
			continue
		}
		if i-offset >= size {
			break
		}
		root.Add(a.albumElem(g, user))
	}
	return a.ok(root)
}

// ---------- songs ----------

func (a *API) hRandomSongs(r *http.Request, user *store.User) *Elem {
	size := paramInt(r, "size", 10)
	genre := r.URL.Query().Get("genre")
	tracks, _ := a.store.QueryTracks(store.Query{Genre: genre, Order: "shuffle", Seed: time.Now().UnixNano(), Limit: 500})
	root := E("randomSongs")
	for i, t := range tracks {
		if i >= size {
			break
		}
		root.Add(a.songElem(t, user))
	}
	return a.ok(root)
}

func (a *API) hSongsByGenre(r *http.Request, user *store.User) *Elem {
	genre := r.URL.Query().Get("genre")
	count := paramInt(r, "count", 10)
	offset := paramInt(r, "offset", 0)
	tracks, _ := a.store.QueryTracks(store.Query{Genre: genre, Limit: 1000})
	root := E("songsByGenre")
	for i, t := range tracks {
		if i < offset {
			continue
		}
		if i-offset >= count {
			break
		}
		root.Add(a.songElem(t, user))
	}
	return a.ok(root)
}

func (a *API) hGenres(r *http.Request, user *store.User) *Elem {
	root := E("genres")
	counts := a.store.Genres()
	albumCounts := map[string]int{}
	all, _ := a.store.QueryTracks(store.Query{Limit: 5000})
	seen := map[string]bool{}
	for _, t := range all {
		k := t.Genre + "|" + t.AlbumID()
		if !seen[k] {
			seen[k] = true
			albumCounts[t.Genre]++
		}
	}
	var names []string
	for g := range counts {
		names = append(names, g)
	}
	sort.Strings(names)
	for _, g := range names {
		root.Add(E("genre").A("songCount", counts[g]).A("albumCount", albumCounts[g]).TextOf(g))
	}
	return a.ok(root)
}

func (a *API) hStarred(r *http.Request, user *store.User) *Elem {
	starred := a.store.Starred(user.ID)
	root := E("starred2")
	var tracks []store.Track
	for id := range starred {
		if t, ok := a.store.Track(id); ok {
			tracks = append(tracks, *t)
		}
	}
	sort.Slice(tracks, func(i, j int) bool { return tracks[i].Artist < tracks[j].Artist })
	seenA := map[string]bool{}
	for _, t := range tracks {
		if !seenA[t.Artist] {
			seenA[t.Artist] = true
			root.Add(artistElem(t.Artist, len(a.albumsOf(t.Artist))))
		}
	}
	for _, g := range a.groupAlbums(tracks) {
		root.Add(a.albumElem(g, user))
	}
	for _, t := range tracks {
		root.Add(a.songElem(t, user))
	}
	return a.ok(root)
}

func (a *API) hStar(r *http.Request, user *store.User, star bool) *Elem {
	q := r.URL.Query()
	var ids []string
	ids = append(ids, q["id"]...)
	ids = append(ids, q["albumId"]...)
	ids = append(ids, q["artistId"]...)
	for _, id := range ids {
		if strings.HasPrefix(id, "al:") {
			tracks, _ := a.store.QueryTracks(store.Query{AlbumID: id, Limit: 500})
			for _, t := range tracks {
				if star {
					a.store.Star(user.ID, t.ID)
				} else {
					a.store.Unstar(user.ID, t.ID)
				}
			}
		} else if strings.HasPrefix(id, "ar:") {
			if name, ok := store.ArtistNameFromID(a.store, id); ok {
				tracks, _ := a.store.QueryTracks(store.Query{Artist: name, Limit: 500})
				for _, t := range tracks {
					if star {
						a.store.Star(user.ID, t.ID)
					} else {
						a.store.Unstar(user.ID, t.ID)
					}
				}
			}
		} else if star {
			a.store.Star(user.ID, id)
		} else {
			a.store.Unstar(user.ID, id)
		}
	}
	return a.ok(nil)
}

func (a *API) hSetRating(r *http.Request, user *store.User) *Elem {
	id := r.URL.Query().Get("id")
	rating := paramInt(r, "rating", 0)
	if id != "" {
		a.store.SetRating(user.ID, id, rating)
	}
	return a.ok(nil)
}

// ---------- scrobble / learning ----------

func (a *API) hScrobble(r *http.Request, user *store.User) *Elem {
	q := r.URL.Query()
	ids := q["id"]
	submission := q.Get("submission") != "false"
	player := q.Get("c")
	if len(ids) == 0 {
		return a.ok(nil)
	}
	id := ids[0]
	t, ok := a.store.Track(id)
	if !ok {
		return a.ok(nil)
	}
	a.mu.Lock()
	defer a.mu.Unlock()
	if !submission {
		// Now-playing start: a quick switch away from the previous
		// track counts as a skip for the taste engine.
		if prev, playing := a.playing[user.ID]; playing && prev.track.ID != id {
			elapsed := time.Since(a.started[user.ID]).Seconds()
			if prev.track.Duration > 0 && elapsed < 0.33*float64(prev.track.Duration) {
				a.store.AddEvent(user.ID, prev.track.ID, "skip", 1)
			}
		}
		a.playing[user.ID] = &nowPlaying{track: *t, started: time.Now(), player: player}
		a.started[user.ID] = time.Now()
		return a.ok(nil)
	}
	// Completed play.
	a.store.AddEvent(user.ID, id, "play", 1.0)
	a.store.RecordPlay(id)
	delete(a.playing, user.ID)
	return a.ok(nil)
}

func (a *API) hNowPlaying(r *http.Request, user *store.User) *Elem {
	root := E("nowPlaying")
	a.mu.Lock()
	defer a.mu.Unlock()
	for _, np := range a.playing {
		mins := int(time.Since(np.started).Minutes())
		e := a.songElem(np.track, user)
		e.Name = "entry"
		e.A("username", user.Username).A("minutesAgo", mins).A("playerId", 1).A("playerName", np.player)
		root.Add(e)
	}
	return a.ok(root)
}

// ---------- similar / info / top ----------

func (a *API) hSimilar(r *http.Request, user *store.User, v2 bool) *Elem {
	id := r.URL.Query().Get("id")
	count := paramInt(r, "count", 50)
	t, ok := a.store.Track(id)
	if !ok {
		// id may be an artist or album id: take its first track
		if strings.HasPrefix(id, "ar:") {
			if name, ok2 := store.ArtistNameFromID(a.store, id); ok2 {
				if ts, _ := a.store.QueryTracks(store.Query{Artist: name, Limit: 1}); len(ts) > 0 {
					t = &ts[0]
					ok = true
				}
			}
		}
	}
	if !ok {
		return a.fail(70, "Not found")
	}
	name := "similarSongs"
	if v2 {
		name = "similarSongs2"
	}
	root := E(name)
	for _, s := range a.taste.Similar(r.Context(), user.ID, *t, count) {
		root.Add(a.songElem(s, user))
	}
	return a.ok(root)
}

func (a *API) hArtistInfo(r *http.Request, user *store.User) *Elem {
	id := r.URL.Query().Get("id")
	name, _ := store.ArtistNameFromID(a.store, id)
	root := E("artistInfo2").A("biography", "").A("musicBrainzId", "").A("lastFmUrl", "").A("smallImageUrl", "").A("mediumImageUrl", "").A("largeImageUrl", "")
	for _, similar := range a.taste.RecommendedArtists(user.ID, 10) {
		if similar != name {
			root.Add(artistElem(similar, len(a.albumsOf(similar))))
		}
	}
	// rename children to similarArtist
	for _, c := range root.Children {
		c.Name = "similarArtist"
	}
	return a.ok(root)
}

func (a *API) hTopSongs(r *http.Request, user *store.User) *Elem {
	artist := r.URL.Query().Get("artist")
	count := paramInt(r, "count", 50)
	tracks, _ := a.store.QueryTracks(store.Query{Artist: artist, Order: "play_count DESC", Limit: count})
	root := E("topSongs")
	for _, t := range tracks {
		root.Add(a.songElem(t, user))
	}
	return a.ok(root)
}

// ---------- playlists ----------

var radioPlaylists = []struct{ mode, name string }{
	{"personal", "Beatlab Radio — Personal"},
	{"discovery", "Beatlab Radio — Discovery"},
	{"favorites", "Beatlab Radio — Favorites"},
}

func (a *API) ensureRadioPlaylists(r *http.Request, user *store.User, force bool) {
	existing := map[string]*store.Playlist{}
	for _, p := range a.store.Playlists(user.ID) {
		if p.Kind == "radio" {
			pp := p
			existing[p.Seed] = &pp
		}
	}
	for _, rp := range radioPlaylists {
		cur, ok := existing[rp.mode]
		if ok && !force && time.Since(cur.Updated) < 6*time.Hour {
			continue
		}
		tracks := a.taste.Radio(r.Context(), user.ID, rp.mode, 50)
		ids := make([]string, 0, len(tracks))
		for _, t := range tracks {
			ids = append(ids, t.ID)
		}
		np := &store.Playlist{Owner: user.ID, Name: rp.name, Kind: "radio", Seed: rp.mode, SongIDs: ids}
		if ok {
			np.ID = cur.ID
		}
		a.store.SavePlaylist(np)
	}
}

func playlistElem(p *store.Playlist, dur int) *Elem {
	return E("playlist").
		A("id", p.ID).
		A("name", p.Name).
		A("songCount", len(p.SongIDs)).
		A("duration", dur).
		A("public", false).
		A("created", p.Updated.Format("2006-01-02T15:04:05.000Z")).
		A("changed", p.Updated.Format("2006-01-02T15:04:05.000Z"))
}

func (a *API) playlistDuration(p *store.Playlist) int {
	d := 0
	for _, id := range p.SongIDs {
		if t, ok := a.store.Track(id); ok {
			d += t.Duration
		}
	}
	return d
}

func (a *API) hGetPlaylists(r *http.Request, user *store.User) *Elem {
	a.ensureRadioPlaylists(r, user, false)
	root := E("playlists")
	for _, p := range a.store.Playlists(user.ID) {
		pp := p
		root.Add(playlistElem(&pp, a.playlistDuration(&pp)))
	}
	return a.ok(root)
}

func (a *API) hGetPlaylist(r *http.Request, user *store.User) *Elem {
	id, _ := strconv.ParseInt(r.URL.Query().Get("id"), 10, 64)
	p, ok := a.store.Playlist(id)
	if !ok || p.Owner != user.ID {
		return a.fail(70, "Playlist not found")
	}
	if p.Kind == "radio" && time.Since(p.Updated) > 6*time.Hour {
		a.ensureRadioPlaylists(r, user, true)
		p, _ = a.store.Playlist(id)
	}
	root := playlistElem(p, a.playlistDuration(p))
	for _, sid := range p.SongIDs {
		if t, ok := a.store.Track(sid); ok {
			e := a.songElem(*t, user)
			e.Name = "entry" // playlist children are <entry> in the Subsonic schema
			root.Add(e)
		}
	}
	return a.ok(root)
}

func (a *API) hCreatePlaylist(r *http.Request, user *store.User) *Elem {
	name := r.URL.Query().Get("name")
	if name == "" {
		return a.fail(10, "Missing name")
	}
	p := &store.Playlist{Owner: user.ID, Name: name, Kind: "manual", SongIDs: r.URL.Query()["songId"]}
	a.store.SavePlaylist(p)
	root := playlistElem(p, a.playlistDuration(p))
	for _, sid := range p.SongIDs {
		if t, ok := a.store.Track(sid); ok {
			e := a.songElem(*t, user)
			e.Name = "entry"
			root.Add(e)
		}
	}
	return a.ok(root)
}

func (a *API) hUpdatePlaylist(r *http.Request, user *store.User) *Elem {
	q := r.URL.Query()
	id, _ := strconv.ParseInt(q.Get("playlistId"), 10, 64)
	p, ok := a.store.Playlist(id)
	if !ok || p.Owner != user.ID {
		return a.fail(70, "Playlist not found")
	}
	if name := q.Get("name"); name != "" {
		p.Name = name
	}
	// removals first (indexes refer to the pre-update list)
	var remove []int
	for _, s := range q["songIndexToRemove"] {
		if n, err := strconv.Atoi(s); err == nil {
			remove = append(remove, n)
		}
	}
	sort.Sort(sort.Reverse(sort.IntSlice(remove)))
	for _, idx := range remove {
		if idx >= 0 && idx < len(p.SongIDs) {
			p.SongIDs = append(p.SongIDs[:idx], p.SongIDs[idx+1:]...)
		}
	}
	p.SongIDs = append(p.SongIDs, q["songIdToAdd"]...)
	a.store.SavePlaylist(p)
	return a.ok(nil)
}

func (a *API) hDeletePlaylist(r *http.Request, user *store.User) *Elem {
	id, _ := strconv.ParseInt(r.URL.Query().Get("id"), 10, 64)
	if p, ok := a.store.Playlist(id); ok && p.Owner == user.ID {
		a.store.DeletePlaylist(id)
	}
	return a.ok(nil)
}

// ---------- play queue ----------

func (a *API) hSavePlayQueue(r *http.Request, user *store.User) *Elem {
	q := r.URL.Query()
	data := map[string]any{
		"current":  q.Get("current"),
		"position": paramInt(r, "position", 0),
		"ids":      q["id"],
	}
	raw, _ := json.Marshal(data)
	a.store.KVSet(fmt.Sprintf("playqueue:%d", user.ID), string(raw))
	return a.ok(nil)
}

func (a *API) hGetPlayQueue(r *http.Request, user *store.User) *Elem {
	root := E("playQueue")
	if raw, ok := a.store.KVGet(fmt.Sprintf("playqueue:%d", user.ID)); ok {
		var data struct {
			Current  string   `json:"current"`
			Position int      `json:"position"`
			IDs      []string `json:"ids"`
		}
		if json.Unmarshal([]byte(raw), &data) == nil {
			root.A("current", data.Current).A("position", data.Position).
				A("username", user.Username).A("changed", time.Now().Format("2006-01-02T15:04:05.000Z")).A("changedBy", "beatlab")
			for _, id := range data.IDs {
				if t, ok := a.store.Track(id); ok {
					e := a.songElem(*t, user)
					e.Name = "entry"
					root.Add(e)
				}
			}
		}
	}
	return a.ok(root)
}

// ---------- binary endpoints ----------

func (a *API) handleStream(w http.ResponseWriter, r *http.Request) {
	id := r.URL.Query().Get("id")
	t, ok := a.store.Track(id)
	if !ok {
		http.Error(w, "not found", http.StatusNotFound)
		return
	}
	a.engine.ServeTrack(w, r, t)
}

func (a *API) handleCover(w http.ResponseWriter, r *http.Request) {
	id := r.URL.Query().Get("id")
	if strings.HasPrefix(id, "al:") {
		tracks, _ := a.store.QueryTracks(store.Query{AlbumID: id, Limit: 1})
		if len(tracks) > 0 {
			a.engine.ServeCover(w, r, &tracks[0])
			return
		}
	}
	if strings.HasPrefix(id, "ar:") {
		if name, ok := store.ArtistNameFromID(a.store, id); ok {
			tracks, _ := a.store.QueryTracks(store.Query{Artist: name, Limit: 1})
			if len(tracks) > 0 {
				a.engine.ServeCover(w, r, &tracks[0])
				return
			}
		}
	}
	if t, ok := a.store.Track(id); ok {
		a.engine.ServeCover(w, r, t)
		return
	}
	http.NotFound(w, r)
}
