package subsonic

// Read endpoints of the Navidrome native API: song / album /
// artist / genre / tag lists and details. Filters follow
// Navidrome's parameter names; ids are Beatlab's own (the same
// ids the Subsonic API uses), so clients can mix both APIs.

import (
	"net/http"
	"sort"
	"strconv"
	"strings"

	"beatlab/internal/store"
)

func inSet(vals []string, v string) bool {
	for _, x := range vals {
		if x == v {
			return true
		}
	}
	return false
}

func (a *API) ndSongList(w http.ResponseWriter, r *http.Request) {
	c, ok := a.ndAuth(w, r)
	if !ok {
		return
	}
	q := r.URL.Query()
	tracks, _ := a.store.QueryTracks(store.Query{Limit: 4000})
	albumIDs := append(q["album_id"], q["albumId"]...)
	artistIDs := append(append(q["artist_id"], q["album_artist_id"]...), q["artists_id"]...)
	genreIDs := q["genre_id"]
	title := strings.ToLower(q.Get("title"))
	year := q.Get("year")
	starredOnly := q.Get("starred") == "true"

	out := []map[string]any{}
	for _, t := range tracks {
		if len(albumIDs) > 0 && !inSet(albumIDs, t.AlbumID()) {
			continue
		}
		if len(artistIDs) > 0 && !inSet(artistIDs, t.ArtistID()) {
			continue
		}
		if len(genreIDs) > 0 && !inSet(genreIDs, t.Genre) {
			continue
		}
		if title != "" && !strings.Contains(strings.ToLower(t.Title), title) {
			continue
		}
		if year != "" {
			if y, err := strconv.Atoi(year); err == nil && t.Year != y {
				continue
			}
		}
		if starredOnly && !c.starred[t.ID] {
			continue
		}
		out = append(out, c.song(t))
	}
	ndSortSongs(out, q)
	start, end := ndPage(w, r, len(out))
	ndJSON(w, http.StatusOK, out[start:end])
}

func (a *API) ndSongDetail(w http.ResponseWriter, r *http.Request) {
	c, ok := a.ndAuth(w, r)
	if !ok {
		return
	}
	t, found := a.store.Track(r.PathValue("id"))
	if !found {
		ndErr(w, http.StatusNotFound, "song not found")
		return
	}
	ndJSON(w, http.StatusOK, c.song(*t))
}

func (a *API) ndAlbumList(w http.ResponseWriter, r *http.Request) {
	c, ok := a.ndAuth(w, r)
	if !ok {
		return
	}
	q := r.URL.Query()
	tracks, _ := a.store.QueryTracks(store.Query{Limit: 4000})
	groups := a.groupAlbums(tracks)
	artistIDs := append(q["artist_id"], q["album_artist_id"]...)
	genreIDs := q["genre_id"]
	name := strings.ToLower(q.Get("name"))
	starredOnly := q.Get("starred") == "true"
	year := q.Get("year")

	out := []map[string]any{}
	for _, g := range groups {
		at := store.Track{Artist: g.artist}
		if len(artistIDs) > 0 && !inSet(artistIDs, at.ArtistID()) {
			continue
		}
		if name != "" && !strings.Contains(strings.ToLower(g.name), name) {
			continue
		}
		m := c.album(g)
		if len(genreIDs) > 0 && !inSet(genreIDs, m["genre"].(string)) {
			continue
		}
		if starredOnly && m["starred"] != true {
			continue
		}
		if year != "" {
			if y, err := strconv.Atoi(year); err == nil && (m["minYear"].(int) > y || m["maxYear"].(int) < y) {
				continue
			}
		}
		out = append(out, m)
	}
	// Sorting: name / album_artist / year / createdAt / songCount.
	key := q.Get("_sort")
	desc := strings.EqualFold(q.Get("_order"), "DESC")
	sort.SliceStable(out, func(i, j int) bool {
		var less bool
		switch key {
		case "album_artist", "artist":
			less = strings.ToLower(out[i]["albumArtist"].(string)) < strings.ToLower(out[j]["albumArtist"].(string))
		case "year":
			less = out[i]["maxYear"].(int) < out[j]["maxYear"].(int)
		case "createdAt":
			less = out[i]["createdAt"].(string) < out[j]["createdAt"].(string)
		case "songCount":
			less = out[i]["songCount"].(int) < out[j]["songCount"].(int)
		default:
			less = strings.ToLower(out[i]["name"].(string)) < strings.ToLower(out[j]["name"].(string))
		}
		if desc {
			return !less
		}
		return less
	})
	start, end := ndPage(w, r, len(out))
	ndJSON(w, http.StatusOK, out[start:end])
}

func (a *API) ndAlbumDetail(w http.ResponseWriter, r *http.Request) {
	c, ok := a.ndAuth(w, r)
	if !ok {
		return
	}
	id := r.PathValue("id")
	tracks, _ := a.store.QueryTracks(store.Query{AlbumID: id, Limit: 1000})
	if len(tracks) == 0 {
		ndErr(w, http.StatusNotFound, "album not found")
		return
	}
	ndJSON(w, http.StatusOK, c.album(a.groupAlbums(tracks)[0]))
}

func (a *API) ndArtistList(w http.ResponseWriter, r *http.Request) {
	c, ok := a.ndAuth(w, r)
	if !ok {
		return
	}
	q := r.URL.Query()
	name := strings.ToLower(q.Get("name"))
	out := []map[string]any{}
	for _, n := range a.store.Artists() {
		if n == "" || strings.EqualFold(n, "Unknown Artist") {
			continue
		}
		if name != "" && !strings.Contains(strings.ToLower(n), name) {
			continue
		}
		out = append(out, c.artist(n))
	}
	desc := strings.EqualFold(q.Get("_order"), "DESC")
	sort.SliceStable(out, func(i, j int) bool {
		less := strings.ToLower(out[i]["name"].(string)) < strings.ToLower(out[j]["name"].(string))
		if desc {
			return !less
		}
		return less
	})
	start, end := ndPage(w, r, len(out))
	ndJSON(w, http.StatusOK, out[start:end])
}

func (a *API) ndArtistDetail(w http.ResponseWriter, r *http.Request) {
	c, ok := a.ndAuth(w, r)
	if !ok {
		return
	}
	name, found := store.ArtistNameFromID(a.store, r.PathValue("id"))
	if !found {
		ndErr(w, http.StatusNotFound, "artist not found")
		return
	}
	ndJSON(w, http.StatusOK, c.artist(name))
}

func (a *API) ndGenreList(w http.ResponseWriter, r *http.Request) {
	if _, ok := a.ndAuth(w, r); !ok {
		return
	}
	genres := a.store.Genres() // name -> song count
	// Album counts per genre.
	albumSets := map[string]map[string]bool{}
	tracks, _ := a.store.QueryTracks(store.Query{Limit: 4000})
	for _, t := range tracks {
		if t.Genre == "" {
			continue
		}
		if albumSets[t.Genre] == nil {
			albumSets[t.Genre] = map[string]bool{}
		}
		albumSets[t.Genre][t.AlbumID()] = true
	}
	out := []map[string]any{}
	for name, count := range genres {
		out = append(out, map[string]any{
			"id": name, "name": name,
			"songCount": count, "albumCount": len(albumSets[name]),
		})
	}
	sort.Slice(out, func(i, j int) bool { return out[i]["name"].(string) < out[j]["name"].(string) })
	start, end := ndPage(w, r, len(out))
	ndJSON(w, http.StatusOK, out[start:end])
}

func (a *API) ndTagList(w http.ResponseWriter, r *http.Request) {
	if _, ok := a.ndAuth(w, r); !ok {
		return
	}
	tagName := r.URL.Query().Get("tag_name")
	if tagName == "" {
		tagName = "genre"
	}
	out := []map[string]any{}
	if tagName == "genre" {
		for name, count := range a.store.Genres() {
			out = append(out, map[string]any{
				"id": name, "tagName": "genre", "tagValue": name, "songCount": count,
			})
		}
	}
	sort.Slice(out, func(i, j int) bool { return out[i]["tagValue"].(string) < out[j]["tagValue"].(string) })
	start, end := ndPage(w, r, len(out))
	ndJSON(w, http.StatusOK, out[start:end])
}
