package subsonic

// Write endpoints of the Navidrome native API: playlists (CRUD +
// track add/remove/move), the user list, the play queue, and
// internet radio stations. Radio stations are shared with the
// Subsonic API (getInternetRadioStations reads the same list).

import (
	"encoding/json"
	"net/http"
	"strconv"
	"strings"
	"time"

	"beatlab/internal/store"
)

// ---------- playlist sidecar (comment / public live outside the store) ----------

type ndPlaylistMeta struct {
	Comment string `json:"comment"`
	Public  bool   `json:"public"`
}

func (a *API) ndPlaylistMeta(id int64) ndPlaylistMeta {
	var m ndPlaylistMeta
	if raw, ok := a.store.KVGet("ndpl:" + strconv.FormatInt(id, 10)); ok {
		json.Unmarshal([]byte(raw), &m)
	}
	return m
}

func (a *API) setNDPlaylistMeta(id int64, m ndPlaylistMeta) {
	raw, _ := json.Marshal(m)
	a.store.KVSet("ndpl:"+strconv.FormatInt(id, 10), string(raw))
}

func (a *API) ndPlaylistObj(c *ndCtx, p *store.Playlist) map[string]any {
	meta := a.ndPlaylistMeta(p.ID)
	var duration float64
	var size int64
	for _, id := range p.SongIDs {
		if t, ok := a.store.Track(id); ok {
			duration += float64(t.Duration)
			size += t.Size
		}
	}
	ownerName := ""
	if owner, ok := a.store.UserByID(p.Owner); ok {
		ownerName = owner.Username
	}
	return map[string]any{
		"id":          strconv.FormatInt(p.ID, 10),
		"name":        p.Name,
		"comment":     meta.Comment,
		"duration":    duration,
		"size":        size,
		"songCount":   len(p.SongIDs),
		"public":      meta.Public,
		"ownerId":     strconv.FormatInt(p.Owner, 10),
		"ownerName":   ownerName,
		"path":        "",
		"sync":        false,
		"rules":       nil,
		"evaluatedAt": nil,
		"createdAt":   ndTS(p.Updated),
		"updatedAt":   ndTS(p.Updated),
	}
}

func (a *API) ndLoadPlaylist(w http.ResponseWriter, idStr string, c *ndCtx, write bool) (*store.Playlist, bool) {
	id, err := strconv.ParseInt(idStr, 10, 64)
	if err != nil {
		ndErr(w, http.StatusNotFound, "playlist not found")
		return nil, false
	}
	p, ok := a.store.Playlist(id)
	if !ok {
		ndErr(w, http.StatusNotFound, "playlist not found")
		return nil, false
	}
	if write && p.Owner != c.user.ID && !c.user.IsAdmin {
		ndErr(w, http.StatusForbidden, "not your playlist")
		return nil, false
	}
	if !write && p.Owner != c.user.ID && !a.ndPlaylistMeta(p.ID).Public && !c.user.IsAdmin {
		ndErr(w, http.StatusNotFound, "playlist not found")
		return nil, false
	}
	return p, true
}

func (a *API) ndPlaylistList(w http.ResponseWriter, r *http.Request) {
	c, ok := a.ndAuth(w, r)
	if !ok {
		return
	}
	q := strings.ToLower(r.URL.Query().Get("q"))
	out := []map[string]any{}
	for _, usr := range a.store.Users() {
		for _, p := range a.store.Playlists(usr.ID) {
			pl := p
			if pl.Owner != c.user.ID && !a.ndPlaylistMeta(pl.ID).Public {
				continue
			}
			if q != "" && !strings.Contains(strings.ToLower(pl.Name), q) {
				continue
			}
			out = append(out, a.ndPlaylistObj(c, &pl))
		}
	}
	start, end := ndPage(w, r, len(out))
	ndJSON(w, http.StatusOK, out[start:end])
}

func (a *API) ndPlaylistDetail(w http.ResponseWriter, r *http.Request) {
	c, ok := a.ndAuth(w, r)
	if !ok {
		return
	}
	p, ok := a.ndLoadPlaylist(w, r.PathValue("id"), c, false)
	if !ok {
		return
	}
	ndJSON(w, http.StatusOK, a.ndPlaylistObj(c, p))
}

func (a *API) ndPlaylistCreate(w http.ResponseWriter, r *http.Request) {
	c, ok := a.ndAuth(w, r)
	if !ok {
		return
	}
	var body struct {
		Name    string  `json:"name"`
		Comment *string `json:"comment"`
		Public  *bool   `json:"public"`
	}
	ndDecode(w, r, &body)
	if strings.TrimSpace(body.Name) == "" {
		ndErr(w, http.StatusBadRequest, "name is required")
		return
	}
	p := &store.Playlist{Owner: c.user.ID, Name: body.Name, Kind: "manual"}
	if err := a.store.SavePlaylist(p); err != nil {
		ndErr(w, http.StatusInternalServerError, "could not save playlist")
		return
	}
	meta := ndPlaylistMeta{}
	if body.Comment != nil {
		meta.Comment = *body.Comment
	}
	if body.Public != nil {
		meta.Public = *body.Public
	}
	a.setNDPlaylistMeta(p.ID, meta)
	ndJSON(w, http.StatusOK, a.ndPlaylistObj(c, p))
}

func (a *API) ndPlaylistUpdate(w http.ResponseWriter, r *http.Request) {
	c, ok := a.ndAuth(w, r)
	if !ok {
		return
	}
	p, ok := a.ndLoadPlaylist(w, r.PathValue("id"), c, true)
	if !ok {
		return
	}
	var body struct {
		Name    *string `json:"name"`
		Comment *string `json:"comment"`
		Public  *bool   `json:"public"`
	}
	ndDecode(w, r, &body)
	if body.Name != nil && strings.TrimSpace(*body.Name) != "" {
		p.Name = *body.Name
		a.store.SavePlaylist(p)
	}
	meta := a.ndPlaylistMeta(p.ID)
	if body.Comment != nil {
		meta.Comment = *body.Comment
	}
	if body.Public != nil {
		meta.Public = *body.Public
	}
	a.setNDPlaylistMeta(p.ID, meta)
	fresh, ok2 := a.store.Playlist(p.ID)
	if !ok2 {
		ndErr(w, http.StatusNotFound, "playlist gone")
		return
	}
	ndJSON(w, http.StatusOK, a.ndPlaylistObj(c, fresh))
}

func (a *API) ndPlaylistDelete(w http.ResponseWriter, r *http.Request) {
	c, ok := a.ndAuth(w, r)
	if !ok {
		return
	}
	p, ok := a.ndLoadPlaylist(w, r.PathValue("id"), c, true)
	if !ok {
		return
	}
	a.store.DeletePlaylist(p.ID)
	ndJSON(w, http.StatusOK, map[string]any{})
}

// ndPlaylistSongs returns playlist entries: each entry's id is its
// index in the playlist (the "playlist item id" clients use for
// removal), mediaFileId is the real track id.
func (a *API) ndPlaylistSongs(w http.ResponseWriter, r *http.Request) {
	c, ok := a.ndAuth(w, r)
	if !ok {
		return
	}
	p, ok := a.ndLoadPlaylist(w, r.PathValue("id"), c, false)
	if !ok {
		return
	}
	out := []map[string]any{}
	for i, id := range p.SongIDs {
		t, found := a.store.Track(id)
		if !found {
			continue
		}
		m := c.song(*t)
		m["id"] = strconv.Itoa(i)
		m["mediaFileId"] = t.ID
		m["playlistId"] = strconv.FormatInt(p.ID, 10)
		out = append(out, m)
	}
	start, end := ndPage(w, r, len(out))
	ndJSON(w, http.StatusOK, out[start:end])
}

func (a *API) ndPlaylistAdd(w http.ResponseWriter, r *http.Request) {
	c, ok := a.ndAuth(w, r)
	if !ok {
		return
	}
	p, ok := a.ndLoadPlaylist(w, r.PathValue("id"), c, true)
	if !ok {
		return
	}
	var body struct {
		IDs []string `json:"ids"`
	}
	ndDecode(w, r, &body)
	added := 0
	for _, id := range body.IDs {
		if _, found := a.store.Track(id); found {
			p.SongIDs = append(p.SongIDs, id)
			added++
		}
	}
	if added > 0 {
		a.store.SavePlaylist(p)
	}
	ndJSON(w, http.StatusOK, map[string]any{"added": added})
}

func (a *API) ndPlaylistRemove(w http.ResponseWriter, r *http.Request) {
	c, ok := a.ndAuth(w, r)
	if !ok {
		return
	}
	p, ok := a.ndLoadPlaylist(w, r.PathValue("id"), c, true)
	if !ok {
		return
	}
	// Clients pass playlist item ids (entry indexes). Anything
	// that isn't a valid index is treated as a track id and all
	// its occurrences go.
	removeIdx := map[int]bool{}
	removeTracks := map[string]bool{}
	for _, v := range r.URL.Query()["id"] {
		if n, err := strconv.Atoi(v); err == nil && n >= 0 && n < len(p.SongIDs) {
			removeIdx[n] = true
		} else {
			removeTracks[v] = true
		}
	}
	kept := p.SongIDs[:0]
	removed := []string{}
	for i, id := range p.SongIDs {
		if removeIdx[i] || removeTracks[id] {
			removed = append(removed, id)
			continue
		}
		kept = append(kept, id)
	}
	p.SongIDs = kept
	a.store.SavePlaylist(p)
	ndJSON(w, http.StatusOK, map[string]any{"ids": removed})
}

func (a *API) ndPlaylistMove(w http.ResponseWriter, r *http.Request) {
	c, ok := a.ndAuth(w, r)
	if !ok {
		return
	}
	p, ok := a.ndLoadPlaylist(w, r.PathValue("playlistId"), c, true)
	if !ok {
		return
	}
	from, err := strconv.Atoi(r.PathValue("trackNumber"))
	if err != nil || from < 0 || from >= len(p.SongIDs) {
		ndErr(w, http.StatusBadRequest, "invalid track number")
		return
	}
	var body struct {
		InsertBefore string `json:"insert_before"`
	}
	ndDecode(w, r, &body)
	to, err := strconv.Atoi(body.InsertBefore)
	if err != nil || to < 0 || to > len(p.SongIDs) {
		ndErr(w, http.StatusBadRequest, "invalid insert_before")
		return
	}
	item := p.SongIDs[from]
	p.SongIDs = append(p.SongIDs[:from], p.SongIDs[from+1:]...)
	if to > from {
		to--
	}
	p.SongIDs = append(p.SongIDs, "")
	copy(p.SongIDs[to+1:], p.SongIDs[to:])
	p.SongIDs[to] = item
	a.store.SavePlaylist(p)
	ndJSON(w, http.StatusOK, map[string]any{})
}

// ---------- users ----------

func (a *API) ndUserList(w http.ResponseWriter, r *http.Request) {
	c, ok := a.ndAuth(w, r)
	if !ok {
		return
	}
	if !c.user.IsAdmin {
		ndErr(w, http.StatusForbidden, "admin only")
		return
	}
	out := []map[string]any{}
	for _, u := range a.store.Users() {
		out = append(out, map[string]any{
			"id":           strconv.FormatInt(u.ID, 10),
			"userName":     u.Username,
			"name":         u.Username,
			"isAdmin":      u.IsAdmin,
			"createdAt":    ndTS(time.Time{}),
			"updatedAt":    ndTS(time.Time{}),
			"lastLoginAt":  ndTS(time.Time{}),
			"lastAccessAt": ndTS(time.Time{}),
		})
	}
	start, end := ndPage(w, r, len(out))
	ndJSON(w, http.StatusOK, out[start:end])
}

// ---------- play queue ----------

type ndQueue struct {
	IDs       []string `json:"ids"`
	Current   int      `json:"current"`
	Position  int      `json:"position"`
	ChangedBy string   `json:"changedBy"`
	UpdatedAt string   `json:"updatedAt"`
}

func (a *API) ndQueueLoad(userID int64) ndQueue {
	var q ndQueue
	if raw, ok := a.store.KVGet("ndqueue:" + strconv.FormatInt(userID, 10)); ok {
		json.Unmarshal([]byte(raw), &q)
	}
	if q.IDs == nil {
		q.IDs = []string{}
	}
	return q
}

func (a *API) ndQueueGet(w http.ResponseWriter, r *http.Request) {
	c, ok := a.ndAuth(w, r)
	if !ok {
		return
	}
	q := a.ndQueueLoad(c.user.ID)
	items := []map[string]any{}
	for _, id := range q.IDs {
		if t, found := a.store.Track(id); found {
			items = append(items, c.song(*t))
		}
	}
	updated := q.UpdatedAt
	if updated == "" {
		updated = ndTS(time.Time{})
	}
	cur := q.Current
	if cur < 0 || cur >= len(items) {
		cur = 0
		if len(items) > 0 {
			if q.Current >= len(items) {
				cur = len(items) - 1
			}
		}
	}
	pos := q.Position
	if pos < 0 {
		pos = 0
	}
	ndJSON(w, http.StatusOK, map[string]any{
		"id":        strconv.FormatInt(c.user.ID, 10),
		"userId":    strconv.FormatInt(c.user.ID, 10),
		"current":   cur,
		"position":  pos,
		"changedBy": q.ChangedBy,
		"createdAt": updated,
		"updatedAt": updated,
		"items":     items,
	})
}

func (a *API) ndQueueSave(w http.ResponseWriter, r *http.Request) {
	c, ok := a.ndAuth(w, r)
	if !ok {
		return
	}
	var body struct {
		IDs      *[]string `json:"ids"`
		Current  *int      `json:"current"`
		Position *int      `json:"position"`
	}
	ndDecode(w, r, &body)
	q := a.ndQueueLoad(c.user.ID)
	if body.IDs != nil {
		q.IDs = *body.IDs
	}
	if body.Current != nil {
		q.Current = *body.Current
	}
	if body.Position != nil {
		q.Position = *body.Position
	}
	if q.Current < 0 {
		q.Current = 0
	}
	if len(q.IDs) > 0 && q.Current >= len(q.IDs) {
		q.Current = len(q.IDs) - 1
	}
	if len(q.IDs) == 0 {
		q.Current = 0
	}
	if q.Position < 0 {
		q.Position = 0
	}
	q.ChangedBy = c.user.Username
	q.UpdatedAt = ndTS(time.Now())
	raw, _ := json.Marshal(q)
	a.store.KVSet("ndqueue:"+strconv.FormatInt(c.user.ID, 10), string(raw))
	ndJSON(w, http.StatusOK, map[string]any{})
}

// ---------- internet radio ----------

type RadioStation struct {
	ID          string `json:"id"`
	Name        string `json:"name"`
	StreamURL   string `json:"streamUrl"`
	HomePageURL string `json:"homePageUrl,omitempty"`
	CreatedAt   string `json:"createdAt"`
	UpdatedAt   string `json:"updatedAt"`
}

func (a *API) RadioStations() []RadioStation {
	out := []RadioStation{}
	if raw, ok := a.store.KVGet("radio_stations"); ok {
		json.Unmarshal([]byte(raw), &out)
	}
	if out == nil {
		out = []RadioStation{}
	}
	return out
}

func (a *API) saveRadioStations(list []RadioStation) {
	raw, _ := json.Marshal(list)
	a.store.KVSet("radio_stations", string(raw))
}

func (a *API) ndRadioList(w http.ResponseWriter, r *http.Request) {
	if _, ok := a.ndAuth(w, r); !ok {
		return
	}
	stations := a.RadioStations()
	out := make([]map[string]any, 0, len(stations))
	for _, s := range stations {
		out = append(out, map[string]any{
			"id": s.ID, "name": s.Name, "streamUrl": s.StreamURL,
			"homePageUrl": s.HomePageURL, "createdAt": s.CreatedAt, "updatedAt": s.UpdatedAt,
		})
	}
	start, end := ndPage(w, r, len(out))
	ndJSON(w, http.StatusOK, out[start:end])
}

func (a *API) ndRadioCreate(w http.ResponseWriter, r *http.Request) {
	c, ok := a.ndAuth(w, r)
	if !ok {
		return
	}
	if !c.user.IsAdmin {
		ndErr(w, http.StatusForbidden, "admin only")
		return
	}
	var body struct {
		Name        string `json:"name"`
		StreamURL   string `json:"streamUrl"`
		HomePageURL string `json:"homePageUrl"`
	}
	ndDecode(w, r, &body)
	if body.Name == "" || body.StreamURL == "" {
		ndErr(w, http.StatusBadRequest, "name and streamUrl are required")
		return
	}
	now := ndTS(time.Now())
	s := RadioStation{
		ID:   "rs" + strconv.FormatInt(time.Now().UnixNano(), 36),
		Name: body.Name, StreamURL: body.StreamURL, HomePageURL: body.HomePageURL,
		CreatedAt: now, UpdatedAt: now,
	}
	a.saveRadioStations(append(a.RadioStations(), s))
	ndJSON(w, http.StatusOK, s)
}

func (a *API) ndRadioUpdate(w http.ResponseWriter, r *http.Request) {
	c, ok := a.ndAuth(w, r)
	if !ok {
		return
	}
	if !c.user.IsAdmin {
		ndErr(w, http.StatusForbidden, "admin only")
		return
	}
	id := r.PathValue("id")
	var body struct {
		Name        string `json:"name"`
		StreamURL   string `json:"streamUrl"`
		HomePageURL string `json:"homePageUrl"`
	}
	ndDecode(w, r, &body)
	stations := a.RadioStations()
	for i, s := range stations {
		if s.ID == id {
			stations[i].Name = body.Name
			stations[i].StreamURL = body.StreamURL
			stations[i].HomePageURL = body.HomePageURL
			stations[i].UpdatedAt = ndTS(time.Now())
			a.saveRadioStations(stations)
			ndJSON(w, http.StatusOK, stations[i])
			return
		}
	}
	ndErr(w, http.StatusNotFound, "station not found")
}

func (a *API) ndRadioDelete(w http.ResponseWriter, r *http.Request) {
	c, ok := a.ndAuth(w, r)
	if !ok {
		return
	}
	if !c.user.IsAdmin {
		ndErr(w, http.StatusForbidden, "admin only")
		return
	}
	id := r.PathValue("id")
	stations := a.RadioStations()
	kept := stations[:0]
	for _, s := range stations {
		if s.ID != id {
			kept = append(kept, s)
		}
	}
	a.saveRadioStations(kept)
	ndJSON(w, http.StatusOK, map[string]any{})
}
