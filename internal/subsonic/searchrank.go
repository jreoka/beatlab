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
	"regexp"
	"sort"
	"strings"
	"unicode"

	"beatlab/internal/store"
)

// titleWords tokenizes a lowercased title on non-letter/digit
// boundaries for word-exact keyword matching.
func titleWords(s string) []string {
	return strings.FieldsFunc(s, func(r rune) bool {
		return !unicode.IsLetter(r) && !unicode.IsDigit(r)
	})
}

// hasKeyword reports whether kw appears in the title: as a
// token sequence for ASCII keywords — so "cover" does not hit
// "Discover" and "live" does not hit "Alive" — and as a raw
// substring for CJK keywords, which have no word boundaries.
func hasKeyword(title string, words []string, kw string) bool {
	ascii := true
	for _, r := range kw {
		if r > 127 {
			ascii = false
			break
		}
	}
	if !ascii {
		return strings.Contains(title, kw)
	}
	kwWords := strings.Fields(kw)
	for i := 0; i+len(kwWords) <= len(words); i++ {
		ok := true
		for j, w := range kwWords {
			if words[i+j] != w {
				ok = false
				break
			}
		}
		if ok {
			return true
		}
	}
	return false
}

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
	{"歌ってみた", 45},
	{"弾いてみた", 45},
	{"踊ってみた", 45},
	{"cover", 45},
	{"instrumental", 45},
	{"インスト", 45},
	{"off vocal", 45},
	{"弾き語り", 40},
	{"music box", 40},
	{"tutorial", 45},
	{"fingerstyle", 40},
	{"nightcore", 50},
	{"sped up", 45},
	{"slowed", 45},
	{"8d audio", 40},
	{"amv", 35},
	{"piano", 35},
	{"ピアノ", 35},
	{"tv size", 35},
	{"reverb", 30},
	{"lofi", 25},
	{"remix", 20},
	{"playing video", 15},
	{"fest", 15},
	{"lyric", 12},
	{"가사", 12},
	{"歌詞", 12},
	{"live", 12},
	{"the first take", 10},
}

// concertDate matches the date stamps live recordings carry in
// their titles ("2026.04.04 CENTRAL FEST", "2025-11-08 ...").
var concertDate = regexp.MustCompile(`20\d\d[./-]\d{1,2}[./-]\d{1,2}`)

// coreTitle strips bracketed segments (【Music Video】,
// [piano], 『anime title』, (Official Video)) so title/query
// comparisons judge the song name itself, not its decorations.
func coreTitle(title string) string {
	var b strings.Builder
	depth := 0
	for _, r := range title {
		switch r {
		case '(', '[', '【', '『', '「':
			depth++
		case ')', ']', '】', '』', '」':
			if depth > 0 {
				depth--
			}
		default:
			if depth == 0 {
				b.WriteRune(r)
			}
		}
	}
	return strings.Join(strings.Fields(strings.ToLower(b.String())), " ")
}

func relevanceScore(t store.Track, query string) int {
	q := strings.ToLower(strings.TrimSpace(query))
	title := strings.ToLower(t.Title)
	words := titleWords(title)
	artist := strings.ToLower(t.Artist)
	if q == "" {
		return 0
	}
	score := 0
	core := coreTitle(t.Title)
	// A title that starts with the query but then keeps going
	// for several more words is a different, longer work that
	// merely shares the name ("Harenohini Amagasa Sasuyatsuniwa
	// Goyoujin" is not "Harenohini") — it must not outrank the
	// song itself on the prefix match or the channel boost. A
	// tail that names a version ("... / THE FIRST TAKE") still
	// describes this song, so it doesn't count as a long tail.
	longTail := false
	if strings.HasPrefix(core, q) {
		var tailWords []string
		for _, wd := range strings.Fields(strings.TrimPrefix(core, q)) {
			if strings.IndexFunc(wd, func(r rune) bool { return unicode.IsLetter(r) || unicode.IsDigit(r) }) >= 0 {
				tailWords = append(tailWords, wd)
			}
		}
		tailStr := strings.Join(tailWords, " ")
		explained := false
		for _, vk := range versionKeywords {
			if hasKeyword(tailStr, tailWords, vk.word) {
				explained = true
				break
			}
		}
		longTail = len(tailWords) >= 3 && !explained
	}
	switch {
	case core == q:
		score += 100
	case strings.HasPrefix(core, q) && !longTail:
		score += 45
	case strings.Contains(title, q):
		score += 30
	}
	if longTail {
		score -= 25
	}
	// The artist being in the query (or the query in the artist)
	// is a strong "this is the one" signal.
	if artist != "" && (strings.Contains(q, artist) || strings.Contains(artist, q)) {
		score += 35
	}
	// Uploads from the artist's own channel (YouTube Topic and
	// official artist channels both land with album == artist in
	// Beatlab's mapping) are the official audio — unless the
	// title says it's a different work sharing the name.
	if t.Source == "youtube" && t.Album != "" && t.Album == t.Artist && !longTail {
		score += 20
	}
	// For much of the catalog the Music Video IS the official
	// release of the song; it should sit with the originals, not
	// below live recordings and lyric slideshows.
	if hasKeyword(title, words, "music video") || hasKeyword(title, words, "official video") {
		score += 15
	}
	if concertDate.MatchString(title) {
		score -= 20
	}
	for _, vk := range versionKeywords {
		if hasKeyword(title, words, vk.word) && !strings.Contains(q, vk.word) {
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
