package taste

import (
	"context"
	"fmt"
	"testing"

	"beatlab/internal/sources"
	"beatlab/internal/store"
)

// A radio queue built from Similar must not be one artist wall
// to wall, even when the seed artist dominates the catalog.
func TestSimilarCapsSeedArtist(t *testing.T) {
	st, err := store.Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	eng := New(st, sources.NewRegistry())
	// Seed artist with 30 catalog tracks; ten genre-mates with
	// three each.
	for i := 0; i < 30; i++ {
		tr := store.Track{ID: fmt.Sprintf("a:%d", i), Source: "test", Title: fmt.Sprintf("Seed Song %d", i), Artist: "Seed Artist", Album: "Seed", Genre: "Jazz"}
		if err := st.UpsertTrack(&tr); err != nil {
			t.Fatal(err)
		}
	}
	for a := 0; a < 10; a++ {
		for i := 0; i < 3; i++ {
			tr := store.Track{ID: fmt.Sprintf("b:%d:%d", a, i), Source: "test", Title: fmt.Sprintf("Mate %d Song %d", a, i), Artist: fmt.Sprintf("Mate %d", a), Album: "M", Genre: "Jazz"}
			if err := st.UpsertTrack(&tr); err != nil {
				t.Fatal(err)
			}
		}
	}
	seed := store.Track{ID: "a:0", Title: "Seed Song 0", Artist: "Seed Artist", Genre: "Jazz"}
	out := eng.Similar(context.Background(), 1, seed, 20)
	if len(out) != 20 {
		t.Fatalf("expected 20 similar tracks, got %d", len(out))
	}
	seedCount := 0
	artists := map[string]int{}
	for _, tr := range out {
		artists[tr.Artist]++
		if tr.Artist == "Seed Artist" {
			seedCount++
		}
	}
	if seedCount > 4 {
		t.Fatalf("seed artist took %d of 20 slots (cap 4): %v", seedCount, artists)
	}
	if len(artists) < 6 {
		t.Fatalf("only %d distinct artists in similar list: %v", len(artists), artists)
	}
}

// Radio playlists already cap per artist; keep them honest too.
func TestRadioCapsArtists(t *testing.T) {
	st, err := store.Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	eng := New(st, sources.NewRegistry())
	for i := 0; i < 40; i++ {
		tr := store.Track{ID: fmt.Sprintf("x:%d", i), Source: "test", Title: fmt.Sprintf("Song %d", i), Artist: "One Artist", Album: "A", Genre: "Rock"}
		if err := st.UpsertTrack(&tr); err != nil {
			t.Fatal(err)
		}
	}
	for a := 0; a < 8; a++ {
		for i := 0; i < 4; i++ {
			tr := store.Track{ID: fmt.Sprintf("y:%d:%d", a, i), Source: "test", Title: fmt.Sprintf("Other %d %d", a, i), Artist: fmt.Sprintf("Other %d", a), Album: "B", Genre: "Rock"}
			if err := st.UpsertTrack(&tr); err != nil {
				t.Fatal(err)
			}
		}
	}
	out := eng.Radio(context.Background(), 1, "personal", 25)
	artists := map[string]int{}
	for _, tr := range out {
		artists[tr.Artist]++
	}
	if artists["One Artist"] > 3 {
		t.Fatalf("radio let one artist take %d slots: %v", artists["One Artist"], artists)
	}
}

// Discovery radio must include artists the user has never
// played, drawn from their top genres — not just more tracks
// by the artists they already play.
func TestRadioDiscoveryIncludesUnplayedArtists(t *testing.T) {
	st, err := store.Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	eng := New(st, sources.NewRegistry())
	// Played artist: 5 jazz tracks, each with play events.
	for i := 0; i < 5; i++ {
		tr := store.Track{ID: fmt.Sprintf("p:%d", i), Source: "test", Title: fmt.Sprintf("Played %d", i), Artist: "Known Player", Album: "K", Genre: "Jazz"}
		if err := st.UpsertTrack(&tr); err != nil {
			t.Fatal(err)
		}
		st.AddEvent(1, tr.ID, "play", 1.0)
	}
	// Unplayed artist, same genre.
	for i := 0; i < 5; i++ {
		tr := store.Track{ID: fmt.Sprintf("u:%d", i), Source: "test", Title: fmt.Sprintf("Fresh %d", i), Artist: "Fresh Face", Album: "F", Genre: "Jazz"}
		if err := st.UpsertTrack(&tr); err != nil {
			t.Fatal(err)
		}
	}
	out := eng.Radio(context.Background(), 1, "discovery", 12)
	found := false
	for _, tr := range out {
		if tr.Artist == "Fresh Face" {
			found = true
		}
	}
	if !found {
		t.Fatalf("discovery mix has no unplayed-artist tracks: %v", out)
	}
}

type fakeSource struct{ tracks []store.Track }

func (f fakeSource) Name() string    { return "fake" }
func (f fakeSource) Available() bool { return true }
func (f fakeSource) Status() string  { return "fake" }
func (f fakeSource) Search(ctx context.Context, q string, limit int) ([]store.Track, error) {
	return f.tracks, nil
}

// The Kansas problem: a radio queue seeded from "Carry On
// Wayward Son" must contain that song ONCE (not the original +
// remaster + live take), and a bare artist-name search must
// not pull politics podcasts about the state into the queue.
func TestSimilarNoRepeatSongNoPodcasts(t *testing.T) {
	st, err := store.Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	versions := []store.Track{
		{ID: "k:1", Source: "test", Title: "Carry On Wayward Son", Artist: "Kansas", Album: "Leftoverture", Genre: "Rock", Duration: 323},
		{ID: "k:2", Source: "test", Title: "Carry On Wayward Son (Remastered 2011)", Artist: "Kansas", Album: "Leftoverture", Genre: "Rock", Duration: 324},
		{ID: "k:3", Source: "test", Title: "Carry On Wayward Son - Live", Artist: "Kansas", Album: "Live", Genre: "Rock", Duration: 350},
		{ID: "k:4", Source: "test", Title: "Dust in the Wind", Artist: "Kansas", Album: "Point of Know Return", Genre: "Rock", Duration: 208},
		{ID: "k:5", Source: "test", Title: "Dust in the Wind (Remaster)", Artist: "Kansas", Album: "Point of Know Return", Genre: "Rock", Duration: 209},
	}
	for i := 0; i < 8; i++ {
		versions = append(versions, store.Track{ID: fmt.Sprintf("r:%d", i), Source: "test", Title: fmt.Sprintf("Rock Song %d", i), Artist: fmt.Sprintf("Rocker %d", i), Album: "R", Genre: "Rock", Duration: 200})
	}
	for _, tr := range versions {
		tr := tr
		if err := st.UpsertTrack(&tr); err != nil {
			t.Fatal(err)
		}
	}
	podcast := store.Track{ID: "pod:1", Source: "fake", Title: "Democrats Flip Kansas? - Daily Politics Podcast", Artist: "MSNBC Daily", Album: "Podcast", Genre: "", Duration: 2700}
	// The screenshot cases: same-song re-uploads in different
	// dress, and not-a-song uploads wearing the artist's name.
	extra := []store.Track{
		{ID: "k:6", Source: "test", Title: "Carry On Wayward Son", Artist: "Kansas", Album: "Greatest Hits", Genre: "Rock", Duration: 325},
		{ID: "k:7", Source: "test", Title: "Carry On Wayward Son HD (Live - 2009)", Artist: "Kansas", Album: "YouTube", Genre: "Rock", Duration: 403},
		{ID: "k:8", Source: "test", Title: "Carry On My Wayward Son", Artist: "Kansas", Album: "YouTube", Genre: "Rock", Duration: 321},
		{ID: "k:9", Source: "test", Title: "Carry On Wayward Son - Reaction Compilation", Artist: "Kansas", Album: "YouTube", Genre: "Rock", Duration: 687},
		{ID: "k:10", Source: "test", Title: "Carry On Wayward Son - Guitar Tab | Lesson | Cover | Tutorial", Artist: "Kansas", Album: "YouTube", Genre: "Rock", Duration: 312},
	}
	for _, tr := range extra {
		tr := tr
		if err := st.UpsertTrack(&tr); err != nil {
			t.Fatal(err)
		}
	}
	eng := New(st, sources.NewRegistry(fakeSource{tracks: []store.Track{podcast}}))
	seed := versions[0]
	out := eng.Similar(context.Background(), 1, seed, 12)
	wayward, dust := 0, 0
	for _, tr := range out {
		if tr.ID == "pod:1" {
			t.Fatal("politics podcast entered the similar queue")
		}
		if tr.ID == "k:9" || tr.ID == "k:10" {
			t.Fatalf("not-a-song upload entered the similar queue: %s", tr.Title)
		}
		switch tr.SongKey() {
		case seed.SongKey():
			wayward++
		}
		if tr.Title == "Dust in the Wind" || tr.Title == "Dust in the Wind (Remaster)" {
			dust++
		}
	}
	// The seed song itself must not come back in its own
	// queue in any dress — the listener just heard it; that
	// repetition was the original complaint.
	if wayward != 0 {
		t.Fatalf("seed song appears %d times in its own radio queue, want 0", wayward)
	}
	if dust > 1 {
		t.Fatalf("Dust in the Wind appears %d times, want at most 1", dust)
	}
}

type queryAwareSource struct {
	byQuery map[string][]store.Track
}

func (f queryAwareSource) Name() string    { return "qa" }
func (f queryAwareSource) Available() bool { return true }
func (f queryAwareSource) Status() string  { return "qa" }
func (f queryAwareSource) Search(ctx context.Context, q string, limit int) ([]store.Track, error) {
	return f.byQuery[q], nil
}

// The genre-less seed: a YouTube-sourced Kansas track has no
// genre, and the local catalog holds only a few Kansas songs.
// Radio must still become a classic-rock mix — the engine
// derives the artist's genre from their catalogued tracks
// and expands through a genre search, not just the artist's
// name.
func TestSimilarExpandsBeyondSeedArtist(t *testing.T) {
	st, err := store.Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	seedTracks := []store.Track{
		{ID: "k:1", Source: "youtube", Title: "Carry On Wayward Son", Artist: "Kansas", Album: "Kansas", Genre: "", Duration: 324},
		{ID: "k:2", Source: "youtube", Title: "Dust in the Wind", Artist: "Kansas", Album: "Kansas", Genre: "", Duration: 208},
		{ID: "k:3", Source: "youtube", Title: "Point of No Return", Artist: "Kansas", Album: "Kansas", Genre: "", Duration: 174},
		{ID: "k:4", Source: "archive", Title: "Miracles out of Nowhere", Artist: "Kansas", Album: "Leftoverture", Genre: "Rock", Duration: 387},
		{ID: "k:5", Source: "archive", Title: "Song for America", Artist: "Kansas", Album: "Kansas", Genre: "Rock", Duration: 600},
	}
	for _, tr := range seedTracks {
		tr := tr
		if err := st.UpsertTrack(&tr); err != nil {
			t.Fatal(err)
		}
	}
	genreResults := []store.Track{
		{ID: "e:1", Source: "fake", Title: "Hotel California", Artist: "Eagles", Album: "Eagles", Genre: "", Duration: 390},
		{ID: "e:2", Source: "fake", Title: "Take It Easy", Artist: "Eagles", Album: "Eagles", Genre: "Rock", Duration: 192},
		{ID: "b:1", Source: "fake", Title: "More Than a Feeling", Artist: "Boston", Album: "Boston", Genre: "", Duration: 285},
		{ID: "b:2", Source: "fake", Title: "Peace of Mind", Artist: "Boston", Album: "Boston", Genre: "Rock", Duration: 300},
		{ID: "f:1", Source: "fake", Title: "Dreams", Artist: "Fleetwood Mac", Album: "Fleetwood Mac", Genre: "", Duration: 254},
		{ID: "f:2", Source: "fake", Title: "Go Your Own Way", Artist: "Fleetwood Mac", Album: "Fleetwood Mac", Genre: "Rock", Duration: 218},
		{ID: "j:1", Source: "fake", Title: "Dream On", Artist: "Aerosmith", Album: "Aerosmith", Genre: "", Duration: 268},
		{ID: "j:2", Source: "fake", Title: "Sweet Emotion", Artist: "Aerosmith", Album: "Aerosmith", Genre: "Rock", Duration: 260},
		{ID: "z:1", Source: "fake", Title: "La Grange", Artist: "ZZ Top", Album: "ZZ Top", Genre: "", Duration: 230},
		{ID: "z:2", Source: "fake", Title: "Tush", Artist: "ZZ Top", Album: "ZZ Top", Genre: "Rock", Duration: 147},
		{ID: "q:1", Source: "fake", Title: "Another One Bites the Dust", Artist: "Queen", Album: "Queen", Genre: "", Duration: 215},
		{ID: "q:2", Source: "fake", Title: "Crazy Little Thing Called Love", Artist: "Queen", Album: "Queen", Genre: "Rock", Duration: 164},
	}
	eng := New(st, sources.NewRegistry(queryAwareSource{byQuery: map[string][]store.Track{
		"Rock": genreResults,
	}}))
	out := eng.Similar(context.Background(), 1, seedTracks[0], 20)
	if len(out) < 12 {
		t.Fatalf("similar returned %d tracks, want a full mix (>= 12)", len(out))
	}
	artists := map[string]int{}
	for _, tr := range out {
		artists[tr.Artist]++
	}
	if len(artists) < 5 {
		t.Fatalf("similar mix has only %d artists: %v", len(artists), artists)
	}
	if artists["Kansas"] > 4 {
		t.Fatalf("seed artist took %d slots (cap 4): %v", artists["Kansas"], artists)
	}
}

// An ungenred seed with an empty source registry: radio must
// still mix, drawing other artists from the listener's own
// catalog by affinity (rung D) instead of collapsing to the
// seed artist.
func TestSimilarCatalogAffinityFallback(t *testing.T) {
	st, err := store.Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	eng := New(st, sources.NewRegistry())
	seed := store.Track{ID: "s:1", Source: "youtube", Title: "Solo Song", Artist: "Solo Act", Album: "Solo Act", Genre: "", Duration: 200}
	others := []store.Track{seed,
		{ID: "s:2", Source: "youtube", Title: "Solo Two", Artist: "Solo Act", Album: "Solo Act", Genre: "", Duration: 200},
	}
	for i := 0; i < 12; i++ {
		others = append(others, store.Track{ID: fmt.Sprintf("o:%d", i), Source: "test", Title: fmt.Sprintf("Other Song %d", i), Artist: fmt.Sprintf("Other Artist %d", i%6), Album: "O", Genre: "", Duration: 200})
	}
	for _, tr := range others {
		tr := tr
		if err := st.UpsertTrack(&tr); err != nil {
			t.Fatal(err)
		}
	}
	// The listener actually plays the other artists.
	for i := 0; i < 12; i++ {
		st.AddEvent(1, fmt.Sprintf("o:%d", i), "play", 1.0)
	}
	out := eng.Similar(context.Background(), 1, seed, 15)
	artists := map[string]int{}
	for _, tr := range out {
		artists[tr.Artist]++
	}
	if len(artists) < 5 {
		t.Fatalf("ungenred radio collapsed to %d artists: %v (out=%d)", len(artists), artists, len(out))
	}
}

// The Yorushika screenshot: an ungenred Japanese seed whose
// catalog fallback filled the queue with Woody Guthrie,
// Chopin and classical-label uploads from the warming pool.
// The fallback must draw on affinity first, then same-script
// tracks — never the whole catalog at a flat score — and the
// result must interleave artists instead of front-loading
// the seed artist.
func TestSimilarFallbackStaysInWorld(t *testing.T) {
	st, err := store.Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	eng := New(st, sources.NewRegistry())
	seed := store.Track{ID: "y:1", Source: "youtube", Title: "Say It.", Artist: "Yorushika", Album: "Yorushika", Genre: "", Duration: 244}
	catalog := []store.Track{seed}
	for i := 0; i < 5; i++ {
		catalog = append(catalog, store.Track{ID: fmt.Sprintf("y:%d", i+2), Source: "youtube", Title: fmt.Sprintf("ヨルシカ曲 %d", i), Artist: "Yorushika", Album: "Yorushika", Genre: "", Duration: 200})
	}
	// Affinity artist: Japanese, and the listener plays her.
	for i := 0; i < 4; i++ {
		tr := store.Track{ID: fmt.Sprintf("r:%d", i), Source: "youtube", Title: fmt.Sprintf("汐れいら Song %d", i), Artist: "汐れいら", Album: "汐れいら", Genre: "", Duration: 200}
		catalog = append(catalog, tr)
	}
	// Other Japanese artists in the catalog, unplayed.
	for i := 0; i < 6; i++ {
		catalog = append(catalog, store.Track{ID: fmt.Sprintf("j:%d", i), Source: "youtube", Title: fmt.Sprintf("日本の曲 %d", i), Artist: fmt.Sprintf("日本のバンド %d", i%3), Album: "J", Genre: "", Duration: 200})
	}
	// A romaji-titled Japanese artist on YouTube: in the
	// seed's world by source, and she must get in.
	catalog = append(catalog, store.Track{ID: "ado:1", Source: "youtube", Title: "Usseewa", Artist: "Ado", Album: "Ado", Genre: "", Duration: 200})
	// A played Japanese track tagged Jazz — gives the
	// profile a stray Jazz affinity, which must NOT become
	// a ticket for latin jazz/classical archive tracks.
	catalog = append(catalog, store.Track{ID: "jz:1", Source: "archive", Title: "ジャズ入門", Artist: "日本のジャズ", Album: "J", Genre: "Jazz", Duration: 200})
	// The warming-catalog debris: latin, unplayed, unrelated.
	debris := []store.Track{
		{ID: "d:1", Source: "archive", Title: "This Land Is Your Land", Artist: "rutaloot", Album: "Woody Guthrie", Genre: "Folk", Duration: 271},
		{ID: "d:2", Source: "archive", Title: "Ballade op. 23, n. 1 (in G min.)", Artist: "Primrose Classical White Label", Album: "Chopin", Genre: "Classical", Duration: 543},
		{ID: "d:3", Source: "archive", Title: "Nocturne No. 6 in F", Artist: "S D Rodrian", Album: "Chopin", Genre: "Classical", Duration: 146},
		{ID: "d:4", Source: "archive", Title: "In A Sentimental Mood", Artist: "John Coltrane", Album: "Coltrane", Genre: "Jazz", Duration: 257},
		{ID: "d:5", Source: "archive", Title: "Dual Mr. Tillman Anthony (New Mix)", Artist: "Miles Davis", Album: "Miles", Genre: "Jazz", Duration: 800},
		{ID: "d:6", Source: "archive", Title: "Laudate Dominum KV 339", Artist: "Wolfgang Amadeus Mozart", Album: "Mozart", Genre: "Classical", Duration: 316},
		{ID: "d:7", Source: "archive", Title: "Turn On Your Lovelight", Artist: "Grateful Dead", Album: "Dead", Genre: "Rock", Duration: 574},
	}
	catalog = append(catalog, debris...)
	for _, tr := range catalog {
		tr := tr
		if err := st.UpsertTrack(&tr); err != nil {
			t.Fatal(err)
		}
	}
	for i := 0; i < 4; i++ {
		st.AddEvent(1, fmt.Sprintf("r:%d", i), "play", 1.0)
	}
	st.AddEvent(1, "jz:1", "play", 1.0)
	out := eng.Similar(context.Background(), 1, seed, 20)
	if len(out) < 10 {
		t.Fatalf("queue too short: %d", len(out))
	}
	for _, tr := range out {
		for _, d := range debris {
			if tr.ID == d.ID {
				t.Fatalf("catalog debris entered the queue: %s by %s", tr.Title, tr.Artist)
			}
		}
	}
	foundAffinity := false
	for _, tr := range out {
		if tr.Artist == "汐れいら" {
			foundAffinity = true
		}
	}
	if !foundAffinity {
		t.Fatal("affinity artist missing from the queue")
	}
	foundAdo := false
	for _, tr := range out {
		if tr.ID == "ado:1" {
			foundAdo = true
		}
	}
	if !foundAdo {
		t.Fatal("romaji YouTube J-pop (Ado) missing from the queue")
	}
	for i := 1; i < len(out); i++ {
		if out[i].Artist == out[i-1].Artist && len(out) > 2 {
			// Adjacent duplicates are only acceptable when
			// nothing else remains — with this catalog there
			// is always an alternative early on.
			if i < 6 {
				t.Fatalf("queue front-loads one artist at position %d: %v", i, out)
			}
		}
	}
}
