package store

import "testing"

// Non-latin names must not all collapse into one "unknown"
// artist/album bucket.
func TestSlugNonLatinNamesStayDistinct(t *testing.T) {
	a := slug("ヨルシカ")
	b := slug("米津玄師")
	if a == "unknown" || b == "unknown" {
		t.Fatalf("non-latin names slugged to unknown: %q %q", a, b)
	}
	if a == b {
		t.Fatalf("distinct non-latin names collided: %q", a)
	}
	if slug("ヨルシカ") != a {
		t.Fatal("slug must be stable across calls")
	}
	ta := Track{Artist: "ヨルシカ"}
	tb := Track{Artist: "米津玄師"}
	if ta.ArtistID() == tb.ArtistID() {
		t.Fatal("artist ids collided for distinct non-latin artists")
	}
	if slug("") != "unknown" || slug("Reira Ushio") != "reira-ushio" {
		t.Fatal("latin/empty slug behavior changed")
	}
}

func TestUpsertDoesNotDowngradeMetadata(t *testing.T) {
	st, err := Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	full := Track{ID: "s:1", Source: "s", Title: "Song", Artist: "Artist", Album: "Album", Genre: "Jazz", Duration: 300, StreamURL: "http://x/stream", CoverURL: "http://x/c.jpg"}
	if err := st.UpsertTrack(&full); err != nil {
		t.Fatal(err)
	}
	partial := Track{ID: "s:1", Source: "s", Title: "Song", Artist: "Artist"}
	if err := st.UpsertTrack(&partial); err != nil {
		t.Fatal(err)
	}
	got, ok := st.Track("s:1")
	if !ok {
		t.Fatal("track vanished")
	}
	if got.Duration != 300 || got.StreamURL == "" || got.CoverURL == "" || got.Genre != "Jazz" || got.Album != "Album" {
		t.Fatalf("partial upsert downgraded metadata: %+v", got)
	}
	// A fuller later upsert still wins.
	fuller := Track{ID: "s:1", Source: "s", Title: "Song (Remaster)", Duration: 301}
	if err := st.UpsertTrack(&fuller); err != nil {
		t.Fatal(err)
	}
	got, _ = st.Track("s:1")
	if got.Title != "Song (Remaster)" || got.Duration != 301 {
		t.Fatalf("fuller upsert did not apply: %+v", got)
	}
}

func TestQueryTextLikeEscaped(t *testing.T) {
	st, err := Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	for _, tr := range []Track{
		{ID: "s:1", Source: "s", Title: "100% Club", Artist: "A"},
		{ID: "s:2", Source: "s", Title: "1000 Clubs", Artist: "B"},
	} {
		tr := tr
		if err := st.UpsertTrack(&tr); err != nil {
			t.Fatal(err)
		}
	}
	out, err := st.QueryTracks(Query{Text: "100%"})
	if err != nil {
		t.Fatal(err)
	}
	if len(out) != 1 || out[0].ID != "s:1" {
		t.Fatalf("LIKE percent not escaped: %v", out)
	}
	out, _ = st.QueryTracks(Query{Text: "%"})
	if len(out) != 1 || out[0].ID != "s:1" {
		t.Fatalf("bare percent matched %v, want only the literal-percent title s:1", out)
	}
}

func TestSavePlaylistOwnerChecked(t *testing.T) {
	st, err := Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	p := &Playlist{Owner: 1, Name: "mine", SongIDs: []string{"s:1"}}
	if err := st.SavePlaylist(p); err != nil {
		t.Fatal(err)
	}
	other := &Playlist{ID: p.ID, Owner: 2, Name: "hijack", SongIDs: []string{}}
	if err := st.SavePlaylist(other); err == nil {
		t.Fatal("cross-owner playlist update was accepted")
	}
	got, _ := st.Playlist(p.ID)
	if got.Name != "mine" || len(got.SongIDs) != 1 {
		t.Fatalf("playlist changed by cross-owner save: %+v", got)
	}
}

func TestSongKeyVersionsCollapse(t *testing.T) {
	k := func(artist, title string) string {
		return Track{Artist: artist, Title: title}.SongKey()
	}
	base := k("Kansas", "Carry On Wayward Son")
	if k("Kansas", "Carry On Wayward Son (Remastered 2011)") != base {
		t.Fatal("remaster should share the song key")
	}
	if k("Kansas", "Carry On Wayward Son - Live") != base {
		t.Fatal("live version should share the song key")
	}
	if k("Kansas", "Carry On Wayward Son HD") != base {
		t.Fatal("upload-quality suffix should share the song key")
	}
	if k("Kansas", "Carry On Wayward Son [Radio Edit]") != base {
		t.Fatal("radio edit should share the song key")
	}
	if k("Kansas", "Dust in the Wind") == base {
		t.Fatal("different songs must not share a key")
	}
	if k("Paul McCartney", "Live and Let Die") == k("Paul McCartney", "Live") {
		t.Fatal("mid-title version words must survive")
	}
	h := k("Reira Ushio", "Harenohini")
	if k("Reira Ushio", "Harenohini Instrumental") != h {
		t.Fatal("instrumental version should share the song key")
	}
	if k("Reira Ushio", "Harenohini TV size") != h {
		t.Fatal("TV-size version should share the song key")
	}
	if k("Reira Ushio", "Harenohini 2026.04.04 CENTRAL FEST 2026") != h {
		t.Fatal("festival upload with a date should share the song key")
	}
	if k("Other Band", "Carry On Wayward Son") == base {
		t.Fatal("different artists must not share a key")
	}
}
