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

// The real YouTube result set for "harenohini" (captured live),
// as Beatlab stores the tracks. The official Music Video must
// lead; a different song whose title merely starts with the
// query ("Harenohini Amagasa...") must not ride the Topic
// boost to #1; lives, TV-size and piano covers sink.
func TestRankHarenohiniRealWorld(t *testing.T) {
	tracks := []store.Track{
		{ID: "ft", Source: "youtube", Title: "Harenohini / THE FIRST TAKE", Artist: "Ushio Reira", Album: "THE FIRST TAKE"},
		{ID: "mv", Source: "youtube", Title: "Reira Ushio / Harenohini【Music Video】TV Anime『The Fragrant Flower Blooms with Dignity』Ending Theme", Artist: "汐れいら", Album: "汐れいら"},
		{ID: "ah", Source: "youtube", Title: "Harenohini Amagasa Sasuyatsuniwa Goyoujin", Artist: "Audio Highs", Album: "Audio Highs"},
		{ID: "fest", Source: "youtube", Title: "Reira Ushio / Harenohini 2026.04.04 CENTRAL FEST 2026", Artist: "汐れいら", Album: "汐れいら"},
		{ID: "tv", Source: "youtube", Title: "Harenohini TV size", Artist: "汐れいら", Album: "汐れいら"},
		{ID: "inst", Source: "youtube", Title: "Harenohini Instrumental", Artist: "汐れいら", Album: "汐れいら"},
		{ID: "piano", Source: "youtube", Title: "(Full) The Fragrant Flower Blooms with Dignity ED『Harenohini / ハレの日に』 Reira Ushio [piano]", Artist: "YXY Piano Covers", Album: "YXY Piano Covers"},
	}
	rankByRelevance(tracks, "harenohini")
	if tracks[0].ID != "mv" {
		t.Fatalf("official MV should rank first, got %v", ids(tracks))
	}
	pos := map[string]int{}
	for i, tr := range tracks {
		pos[tr.ID] = i
	}
	if pos["ah"] < 3 {
		t.Fatalf("different-song Topic track ranked #%d, want below top 3: %v", pos["ah"]+1, ids(tracks))
	}
	if pos["piano"] < len(tracks)-2 || pos["ah"] < len(tracks)-2 {
		t.Fatalf("piano cover and the different-song track should be the bottom two: %v", ids(tracks))
	}
	if pos["tv"] < pos["mv"] || pos["inst"] < pos["mv"] {
		t.Fatalf("versions outranked the MV: %v", ids(tracks))
	}
}

func TestRelevanceKeywordsAreWordBounded(t *testing.T) {
	discover := relevanceScore(store.Track{Title: "Discover", Artist: "Some Band"}, "discovery mix")
	plain := relevanceScore(store.Track{Title: "Untitled", Artist: "Some Band"}, "discovery mix")
	if discover < plain {
		t.Fatalf(`"Discover" was penalized as a cover version (%d < %d)`, discover, plain)
	}
	alive := relevanceScore(store.Track{Title: "Alive", Artist: "Some Band"}, "golden hits")
	if alive < plain {
		t.Fatalf(`"Alive" was penalized as a live version (%d < %d)`, alive, plain)
	}
	real := relevanceScore(store.Track{Title: "Golden Hits (Piano Cover)", Artist: "Some Band"}, "golden hits")
	if real >= plain {
		t.Fatalf("a real piano cover should be penalized (%d >= %d)", real, plain)
	}
}
