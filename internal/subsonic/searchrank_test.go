package subsonic

import (
	"testing"

	"beatlab/internal/store"
)

// Mirrors the result set from a real "harenohini" search:
// piano covers and alternate takes crowding out the original.
func TestRankByRelevanceOriginalFirst(t *testing.T) {
	tracks := []store.Track{
		{ID: "1", Source: "youtube", Title: "(Full) The Fragrant Flower Blooms with Dignity ED Piano", Artist: "YXY Piano Covers", Album: "YXY Piano Covers"},
		{ID: "2", Source: "youtube", Title: "Harenohini Instrumental", Artist: "Ushio Reira", Album: "Ushio Reira"},
		{ID: "3", Source: "youtube", Title: "Harenohini TV size", Artist: "Ushio Reira", Album: "Ushio Reira"},
		{ID: "4", Source: "youtube", Title: "Harenohini", Artist: "Ushio Reira", Album: "Ushio Reira"},
		{ID: "5", Source: "youtube", Title: "Harenohini Piano Cover", Artist: "Justin Wai", Album: "Justin Wai"},
	}
	rankByRelevance(tracks, "harenohini")
	if tracks[0].ID != "4" {
		t.Fatalf("expected the original (id 4) first, got order %v", ids(tracks))
	}
	if tracks[len(tracks)-1].ID != "5" {
		t.Fatalf("expected the piano cover last, got order %v", ids(tracks))
	}
}

// Asking for the instrumental must surface the instrumental.
func TestRankByRelevanceQueryKeywordsNotPenalized(t *testing.T) {
	tracks := []store.Track{
		{ID: "4", Source: "youtube", Title: "Harenohini", Artist: "Ushio Reira", Album: "Ushio Reira"},
		{ID: "2", Source: "youtube", Title: "Harenohini Instrumental", Artist: "Ushio Reira", Album: "Ushio Reira"},
	}
	rankByRelevance(tracks, "harenohini instrumental")
	if tracks[0].ID != "2" {
		t.Fatalf("expected the instrumental first, got %v", ids(tracks))
	}
}

func ids(ts []store.Track) []string {
	out := []string{}
	for _, t := range ts {
		out = append(out, t.ID)
	}
	return out
}
