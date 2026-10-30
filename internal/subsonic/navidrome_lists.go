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
	searchTerm := q.Get("title")

	// A title search goes live, like the Subsonic search does:
	// fan out to the sources (soft-deadline bounded) and warm
	// the results into the catalog. The sources' own relevance
	// (YouTube search semantics) decides what comes back — those
	// results are answers by construction and are NOT re-filtered
	// by text matching: a track whose title is in another script
	// (ヨルシカ - 言って。 for "say it yorushika") can never pass a
	// latin substring test, yet it is exactly what the user
	// searched for.
	leadIDs := map[string]bool{}
	var lead []store.Track
	if searchTerm != "" {
		for _, t := range a.registry.SearchAll(r.Context(), searchTerm, 40) {
			tt := t
			a.store.UpsertTrack(&tt)
			leadIDs[tt.ID] = true
			lead = append(lead, tt)
		}
	}
	tracks, _ := a.store.QueryTracks(store.Query{Limit: 4000})
	albumIDs := append(q["album_id"], q["albumId"]...)
	artistIDs := append(append(q["artist_id"], q["album_artist_id"]...), q["artists_id"]...)
	genreIDs := q["genre_id"]
	year := q.Get("year")
	starredOnly := q.Get("starred") == "true"
	words := strings.Fields(strings.ToLower(searchTerm))

	// matches applies every filter. For the title dimension,
	// local tracks must match word-wise across title+artist+
	// album (every query word somewhere in the three fields);
	// lead tracks from the live fan-out always match.
	matches := func(t store.Track) bool {
		if len(albumIDs) > 0 && !inSet(albumIDs, t.AlbumID()) {
			return false
		}
		if len(artistIDs) > 0 && !inSet(artistIDs, t.ArtistID()) {
			return false
		}
		if len(genreIDs) > 0 && !inSet(genreIDs, t.Genre) {
			return false
		}
		if year != "" {
			if y, err := strconv.Atoi(year); err == nil && t.Year != y {
				return false
			}
		}
		if starredOnly && !c.starred[t.ID] {
			return false
		}
		if searchTerm != "" && !leadIDs[t.ID] {
			hay := strings.ToLower(t.Title + " " + t.Artist + " " + t.Album)
			for _, word := range words {
				if !strings.Contains(hay, word) {
					return false
				}
			}
		}
		return true
	}

	// Lead results first, in source order, so the stable
	// relevance sort breaks ties by the source's own ranking
	// rather than catalog insertion order.
	var matched []store.Track
	for _, t := range lead {
		if fresh, ok := a.store.Track(t.ID); ok && matches(*fresh) {
			matched = append(matched, *fresh)
		}
	}
	for _, t := range tracks {
		if leadIDs[t.ID] {
			continue
		}
		if matches(t) {
			matched = append(matched, t)
		}
	}
	// For a search, relevance orders the results (originals
	// first); plain browsing keeps the client's sort.
	if searchTerm != "" {
		rankByRelevance(matched, searchTerm)
	}
	out := []map[string]any{}
	for _, t := range matched {
		out = append(out, c.song(t))
	}
	if searchTerm == "" {
		ndSortSongs(out, q)
	}
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
		if desc {
			i, j = j, i
		}
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
		if desc {
			i, j = j, i
		}
		return strings.ToLower(out[i]["name"].(string)) < strings.ToLower(out[j]["name"].(string))
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
