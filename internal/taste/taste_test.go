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
