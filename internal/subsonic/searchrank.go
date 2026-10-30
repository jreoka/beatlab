package subsonic

// Relevance ranking for song searches. A title search on a
// catalog fed by YouTube/archive.org returns every version of a
// song ever uploaded — piano covers, instrumentals, TV-size
// edits, nightcore — and the original recording drowns in them
// under plain alphabetical or source order. rankByRelevance
// scores each track against the query so the canonical version
// surfaces first; alternate versions stay in the list, just
// below, and are never penalized when the query itself asks
// for them ("harenohini instrumental" still finds the
// instrumental first).

import (
	"sort"
	"strings"

	"beatlab/internal/store"
)

// versionKeywords mark a track as an alternate take on a song.
// The penalty applies only when the keyword does not appear in
// the query itself.
var versionKeywords = []struct {
	word    string
	penalty int
}{
	{"karaoke", 60},
	{"カラオケ", 60},
	{"tribute", 55},
	{"piano cover", 55},
	{"cover", 45},
	{"instrumental", 45},
	{"インスト", 45},
	{"off vocal", 45},
	{"nightcore", 50},
	{"sped up", 45},
	{"slowed", 45},
	{"8d audio", 40},
	{"piano", 35},
	{"ピアノ", 35},
	{"tv size", 35},
	{"reverb", 30},
	{"remix", 20},
	{"live", 12},
}

func relevanceScore(t store.Track, query string) int {
	q := strings.ToLower(strings.TrimSpace(query))
	title := strings.ToLower(t.Title)
	artist := strings.ToLower(t.Artist)
	if q == "" {
		return 0
	}
	score := 0
	switch {
	case title == q:
		score += 100
	case strings.HasPrefix(title, q):
		score += 45
	case strings.Contains(title, q):
		score += 30
	}
	// The artist being in the query (or the query in the artist)
	// is a strong "this is the one" signal.
	if artist != "" && (strings.Contains(q, artist) || strings.Contains(artist, q)) {
		score += 35
	}
	// YouTube's auto-generated "Topic" uploads are the official
	// audio; in Beatlab's mapping they land with album == artist.
	if t.Source == "youtube" && t.Album != "" && t.Album == t.Artist {
		score += 20
	}
	for _, vk := range versionKeywords {
		if strings.Contains(title, vk.word) && !strings.Contains(q, vk.word) {
			score -= vk.penalty
		}
	}
	return score
}

// rankByRelevance stable-sorts tracks by descending relevance
// to the query, preserving the incoming order among equals.
func rankByRelevance(tracks []store.Track, query string) {
	sort.SliceStable(tracks, func(i, j int) bool {
		return relevanceScore(tracks[i], query) > relevanceScore(tracks[j], query)
	})
}
