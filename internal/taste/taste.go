// Package taste is Beatlab's learning layer. Every scrobble, star,
// rating and skip lands in the events table; this package folds
// those events (with time decay) into per-artist and per-genre
// affinity scores, and builds similar-song lists, recommendation
// pools and radio mixes out of them.
package taste

import (
	"context"
	"math"
	"math/rand"
	"sort"
	"strings"
	"sync/atomic"
	"time"

	"beatlab/internal/sources"
	"beatlab/internal/store"
)

type Profile struct {
	ArtistScore map[string]float64
	GenreScore  map[string]float64
	TrackPlays  map[string]int // completed plays per track
	TrackSkips  map[string]int
	Starred     map[string]bool
	Plays       int
}

type Engine struct {
	store    *store.Store
	registry *sources.Registry
	warming  atomic.Bool
}

func New(st *store.Store, reg *sources.Registry) *Engine {
	return &Engine{store: st, registry: reg}
}

func decayWeight(at time.Time) float64 {
	days := time.Since(at).Hours() / 24
	if days < 0 {
		days = 0
	}
	return math.Pow(0.5, days/90) // 90-day half-life
}

// BuildProfile folds a user's events into affinity scores.
// junkTitleWords mark a track as not-a-song for queue
// purposes: reaction videos, tutorials, lessons, interviews
// and the like. Uploaders title these with the artist's name
// ("Kansas - Carry On Wayward Son - Reaction Compilation"),
// so they otherwise walk into radio queues as "same artist"
// tracks. Word-boundary matched.
var junkTitleWords = map[string]bool{
	"reaction": true, "reactions": true, "reacts": true, "react": true,
	"tutorial": true, "lesson": true, "lessons": true,
	"tab": true, "tabs": true, "commentary": true,
	"review": true, "reviews": true, "interview": true,
	"interviews": true, "podcast": true, "trailer": true,
	"unboxing": true, "breakdown": true, "explained": true,
	"analysis": true,
}

func junkTitle(t store.Track) bool {
	title := strings.ToLower(t.Title)
	if strings.Contains(title, "how to") {
		return true
	}
	for _, wd := range strings.FieldsFunc(title, func(r rune) bool {
		return (r < 'a' || r > 'z') && (r < '0' || r > '9') && r < 128
	}) {
		if junkTitleWords[wd] {
			return true
		}
	}
	return false
}

// hasCJK reports whether a string contains CJK characters —
// a rough but effective "same musical world" signal: a
// Japanese seed's radio should stay Japanese, and a latin
// seed's should not wander into CJK uploads (or vice versa).
func hasCJK(s string) bool {
	for _, r := range s {
		if (r >= 0x3000 && r <= 0x30FF) || (r >= 0x4E00 && r <= 0x9FFF) || (r >= 0xFF00 && r <= 0xFFEF) {
			return true
		}
	}
	return false
}

// songTokens are the normalized title tokens behind
// store.SongKey, for near-duplicate comparison.
func songTokens(t store.Track) []string {
	key := t.SongKey()
	if i := strings.Index(key, "|"); i >= 0 {
		key = key[i+1:]
	}
	if key == "" {
		return nil
	}
	return strings.Fields(key)
}

var fillerWords = map[string]bool{"my": true, "the": true, "a": true, "an": true, "oh": true}

// sameSongFuzzy reports whether two token lists name the same
// song allowing one filler word inserted or dropped —
// "Carry On My Wayward Son" vs "Carry On Wayward Son" is the
// same recording mistitled, while genuinely different songs
// ("Love Me" / "Love Me Do") differ by a real word and stay
// distinct.
func sameSongFuzzy(a, b []string) bool {
	if len(a) == len(b) {
		for i := range a {
			if a[i] != b[i] {
				return false
			}
		}
		return true
	}
	long, short := a, b
	if len(short) > len(long) {
		long, short = short, long
	}
	if len(long)-len(short) != 1 {
		return false
	}
	skipped := false
	j := 0
	for i := 0; i < len(long); i++ {
		if j < len(short) && long[i] == short[j] {
			j++
			continue
		}
		if skipped || !fillerWords[long[i]] {
			return false
		}
		skipped = true
	}
	return j == len(short)
}

// songDeduper tracks which songs a queue has already taken,
// by exact SongKey and by filler-variation, per artist.
type songDeduper struct {
	keys map[string]bool
	toks map[string][][]string
}

func newSongDeduper() *songDeduper {
	return &songDeduper{keys: map[string]bool{}, toks: map[string][][]string{}}
}

func (d *songDeduper) Seen(t store.Track) bool {
	if d.keys[t.SongKey()] {
		return true
	}
	for _, alias := range t.SongAliases() {
		if d.keys[alias] {
			return true
		}
	}
	artist := strings.ToLower(t.Artist)
	toks := songTokens(t)
	for _, taken := range d.toks[artist] {
		if sameSongFuzzy(toks, taken) {
			return true
		}
	}
	return false
}

func (d *songDeduper) Mark(t store.Track) {
	d.keys[t.SongKey()] = true
	for _, alias := range t.SongAliases() {
		d.keys[alias] = true
	}
	artist := strings.ToLower(t.Artist)
	d.toks[artist] = append(d.toks[artist], songTokens(t))
}

func (e *Engine) BuildProfile(userID int64) *Profile {
	p := &Profile{
		ArtistScore: map[string]float64{},
		GenreScore:  map[string]float64{},
		TrackPlays:  map[string]int{},
		TrackSkips:  map[string]int{},
		Starred:     map[string]bool{},
	}
	for id := range e.store.Starred(userID) {
		p.Starred[id] = true
	}
	since := time.Now().AddDate(0, -6, 0)
	for _, ev := range e.store.Events(userID, since) {
		t, ok := e.store.Track(ev.TrackID)
		if !ok {
			continue
		}
		w := decayWeight(ev.Created)
		var artistDelta, genreDelta float64
		switch ev.Kind {
		case "play":
			artistDelta = ev.Value * 1.0
			genreDelta = ev.Value * 0.7
			if ev.Value >= 0.9 {
				p.TrackPlays[ev.TrackID]++
				p.Plays++
			}
		case "skip":
			artistDelta = -0.6
			genreDelta = -0.3
			p.TrackSkips[ev.TrackID]++
		case "star":
			artistDelta = 3
			genreDelta = 1.5
		case "unstar":
			artistDelta = -1
		case "rate":
			artistDelta = (ev.Value - 3) * 0.8
			genreDelta = (ev.Value - 3) * 0.4
		}
		if t.Artist != "" {
			p.ArtistScore[t.Artist] += artistDelta * w
		}
		if t.Genre != "" {
			p.GenreScore[t.Genre] += genreDelta * w
		}
	}
	return p
}

func (p *Profile) TopArtists(n int) []string {
	return topKeys(p.ArtistScore, n, 0)
}
func (p *Profile) TopGenres(n int) []string {
	return topKeys(p.GenreScore, n, 0)
}

func topKeys(m map[string]float64, n int, min float64) []string {
	type kv struct {
		k string
		v float64
	}
	var list []kv
	for k, v := range m {
		if v > min {
			list = append(list, kv{k, v})
		}
	}
	sort.Slice(list, func(i, j int) bool { return list[i].v > list[j].v })
	var out []string
	for i, kv := range list {
		if i >= n {
			break
		}
		out = append(out, kv.k)
	}
	return out
}

// scoreTrack rates a candidate for a listener with this profile.
func (p *Profile) scoreTrack(t store.Track) float64 {
	s := p.ArtistScore[t.Artist]*0.9 + p.GenreScore[t.Genre]*0.5
	s += math.Log1p(float64(t.PlayCount)) * 0.3
	if p.Starred[t.ID] {
		s += 2
	}
	if p.TrackSkips[t.ID] >= 2 {
		s -= 3
	}
	return s
}

// Similar returns songs similar to the seed: same artist and
// genre-mates first, expanded with a live source search for the
// seed artist when the local catalog is thin.
// RandomPool is the quality-controlled pool behind
// getRandomSongs. Feishin's AUTO DJ pads its queue from
// getRandomSongs whenever similar songs run short — its
// algorithm literally falls back to "random songs, limit
// 50" — so serving that endpoint a raw shuffle of the whole
// catalog is what kept injecting unplayed archive.org
// warming debris (Woody Guthrie, Chopin, Mozart, Dead
// tapes) into queues no Similar change could clean: those
// tracks never passed through Similar at all. The pool
// prefers tracks with an engagement signal — played,
// starred by this user, or YouTube-sourced (arrived via
// real searches rather than background warming) — and
// backfills from the general catalog only when the engaged
// pool cannot fill the request (a fresh server still gets
// variety).
func (e *Engine) RandomPool(userID int64, genre string, size int) []store.Track {
	if size <= 0 {
		size = 10
	}
	rows, _ := e.store.QueryTracks(store.Query{Genre: genre, Limit: 1000, Order: "shuffle", Seed: time.Now().UnixNano()})
	starred := e.store.Starred(userID)
	var engaged, rest []store.Track
	for _, t := range rows {
		if junkTitle(t) {
			continue
		}
		if t.Duration == 0 && t.Source == "youtube" {
			continue
		}
		_, isStarred := starred[t.ID]
		if t.PlayCount > 0 || isStarred || t.Source == "youtube" {
			engaged = append(engaged, t)
		} else {
			rest = append(rest, t)
		}
	}
	out := engaged
	if len(out) < size {
		out = append(out, rest...)
	}
	if len(out) > size {
		out = out[:size]
	}
	return out
}

func (e *Engine) Similar(ctx context.Context, userID int64, seed store.Track, count int) []store.Track {
	p := e.BuildProfile(userID)
	// Targeted queries, not a catalog window: QueryTracks'
	// default order is alphabetical by artist, so a flat
	// Limit-N window silently exiles every late-alphabet
	// artist once the catalog outgrows it.
	var artistRows []store.Track
	if seed.Artist != "" {
		artistRows, _ = e.store.QueryTracks(store.Query{Artist: seed.Artist, Limit: 500})
	}
	// The seed's effective genre: its own tag, or — YouTube
	// uploads carry no genre — the seed artist's most common
	// genre in the catalog. Without it a genre-less seed can
	// only ever return its own artist.
	effGenre := seed.Genre
	if effGenre == "" && len(artistRows) > 0 {
		counts := map[string]int{}
		for _, t := range artistRows {
			if t.Genre != "" {
				counts[t.Genre]++
			}
		}
		best := 0
		for g, n := range counts {
			if n > best {
				best, effGenre = n, g
			}
		}
	}
	// The seed's world: whether the seed artist's catalog is
	// majority-CJK (the seed itself may be romanized while
	// the catalog behind it is Japanese-titled). For a
	// Japanese artist, "similar" lives in a specific corner
	// of the catalog — CJK-titled tracks and YouTube-sourced
	// uploads, which is where this catalog's J-pop comes
	// from; its archive.org side skews western classical,
	// jazz and live tapes. Genre and affinity signals are
	// only trusted INSIDE that world — otherwise one stray
	// genre affinity admits Miles Davis and Mozart into a
	// Yorushika radio.
	seedCJK := hasCJK(seed.Artist) || hasCJK(seed.Title)
	if !seedCJK && len(artistRows) > 0 {
		cjk := 0
		for _, t := range artistRows {
			if hasCJK(t.Title) || hasCJK(t.Artist) {
				cjk++
			}
		}
		seedCJK = cjk*2 > len(artistRows)
	}
	worldOK := func(t store.Track) bool {
		if !seedCJK {
			return true
		}
		return hasCJK(t.Artist) || hasCJK(t.Title) || t.Source == "youtube"
	}
	cands := append([]store.Track{}, artistRows...)
	if effGenre != "" {
		rows, _ := e.store.QueryTracks(store.Query{Genre: effGenre, Limit: 500, Order: "shuffle", Seed: time.Now().Unix() / 86400})
		cands = append(cands, rows...)
	}
	seen := map[string]bool{seed.ID: true}
	dedup := newSongDeduper()
	dedup.Mark(seed)
	type scored struct {
		t store.Track
		s float64
	}
	var list []scored
	add := func(t store.Track, base float64) {
		if seen[t.ID] {
			return
		}
		seen[t.ID] = true
		list = append(list, scored{t, base + p.scoreTrack(t)})
	}
	for _, t := range cands {
		if junkTitle(t) {
			continue
		}
		if t.Duration == 0 && t.Source == "youtube" {
			continue // placeholder/premiere entries, 0:00 long
		}
		switch {
		case strings.EqualFold(t.Artist, seed.Artist) && seed.Artist != "":
			add(t, 4)
		case effGenre != "" && strings.EqualFold(t.Genre, effGenre) && worldOK(t):
			add(t, 1.5)
		}
	}
	// Expansion ladder. The trigger is not just a short list
	// but a MONO-ARTIST one: a genre-less seed in a thin
	// catalog used to return three songs by the seed artist
	// and stop — a "radio" that never leaves the artist.
	// Climb until the list is long and varied enough: the
	// artist's own name, then the genre (which is how other
	// artists enter at all), then the listener's top artists.
	distinctArtists := func() int {
		set := map[string]bool{}
		for _, sc := range list {
			set[strings.ToLower(sc.t.Artist)] = true
		}
		return len(set)
	}
	short := func() bool {
		return len(list) < count || (count >= 10 && distinctArtists() < 5)
	}
	admit := func(t store.Track, sameArtist, genreMatch, ungenred float64) {
		if t.Duration > 1200 || junkTitle(t) {
			return
		}
		if !strings.EqualFold(t.Artist, seed.Artist) && !worldOK(t) {
			return
		}
		switch {
		case strings.EqualFold(t.Artist, seed.Artist):
			add(t, sameArtist)
		case effGenre != "" && strings.EqualFold(t.Genre, effGenre):
			add(t, genreMatch)
		case t.Genre == "" && ungenred >= 0:
			add(t, ungenred)
		}
	}
	if short() && seed.Artist != "" {
		for _, t := range e.registry.SearchAll(ctx, seed.Artist, 25) {
			e.store.UpsertTrack(&t)
			admit(t, 3.5, 1, -1)
		}
	}
	if short() && effGenre != "" {
		for _, t := range e.registry.SearchAll(ctx, effGenre, 30) {
			e.store.UpsertTrack(&t)
			admit(t, 3, 1.2, 0.8)
		}
	}
	// Rung D: the listener's own catalog, other artists
	// only. Two passes with real signals, because the first
	// version of this rung admitted everything at a flat
	// base and a Yorushika radio filled with Woody Guthrie,
	// Chopin and a Biggie/Sinatra mashup from the warming
	// catalog:
	//   D1 affinity — anything the profile actually scores
	//      (artists/genres the listener plays and stars);
	//   D2 same script — tracks from the seed's musical
	//      world by writing system (CJK seed → CJK tracks),
	//      which is where his other J-pop lives.
	if short() {
		rows, _ := e.store.QueryTracks(store.Query{Limit: 400, Order: "shuffle", Seed: time.Now().Unix() / 86400})
		usable := func(t store.Track) bool {
			if strings.EqualFold(t.Artist, seed.Artist) || junkTitle(t) {
				return false
			}
			if t.Duration == 0 && t.Source == "youtube" {
				return false
			}
			return true
		}
		for _, t := range rows {
			if !usable(t) {
				continue
			}
			if sc := p.scoreTrack(t); sc > 0 && worldOK(t) {
				add(t, 1.0+sc)
			}
		}
		if short() {
			for _, t := range rows {
				if !usable(t) {
					continue
				}
				if seedCJK {
					// Inside the seed's world, affinity
					// orders; outside it, nothing enters.
					if worldOK(t) {
						add(t, 0.5+p.scoreTrack(t))
					}
				} else if hasCJK(t.Artist) || hasCJK(t.Title) {
					continue // latin seed: CJK uploads are another world
				} else {
					add(t, 0.5)
				}
			}
		}
	}
	if short() {
		added := 0
		for _, a := range p.TopArtists(4) {
			if added >= 2 || strings.EqualFold(a, seed.Artist) {
				continue
			}
			added++
			for _, t := range e.registry.SearchAll(ctx, a, 15) {
				e.store.UpsertTrack(&t)
				if t.Duration > 1200 || junkTitle(t) {
					continue
				}
				if !strings.EqualFold(t.Artist, seed.Artist) && !worldOK(t) {
					continue
				}
				switch {
				case strings.EqualFold(t.Artist, a):
					add(t, 1.2)
				case effGenre != "" && strings.EqualFold(t.Genre, effGenre):
					add(t, 0.8)
				case strings.EqualFold(t.Artist, seed.Artist):
					add(t, 1)
				}
			}
			if !short() {
				break
			}
		}
	}
	sort.Slice(list, func(i, j int) bool { return list[i].s > list[j].s })
	// "Similar" must not come back as a single-artist list: the
	// seed artist's own tracks score highest by construction, so
	// without caps a radio queue built from this is one artist
	// wall to wall. Cap the seed artist hard and every other
	// artist loosely, then backfill without caps if the catalog
	// is too thin to fill the list otherwise.
	seedCap := count / 5
	if seedCap < 2 {
		seedCap = 2
	}
	otherCap := count / 4
	if otherCap < 3 {
		otherCap = 3
	}
	counts := map[string]int{}
	taken := map[string]bool{}
	var out []store.Track
	// Pass 1: per-artist caps + one version per song. Pass 2:
	// caps off, still one version per song — a queue with the
	// seed song three times in different versions is the bug
	// this prevents, and a shorter queue beats a repeated song.
	for pass := 0; pass < 2; pass++ {
		for _, s := range list {
			if len(out) >= count {
				return interleaveByArtist(out)
			}
			if taken[s.t.ID] {
				continue
			}
			if dedup.Seen(s.t) {
				continue
			}
			if pass == 0 {
				key := strings.ToLower(s.t.Artist)
				cap := otherCap
				if strings.EqualFold(s.t.Artist, seed.Artist) {
					cap = seedCap
				}
				if counts[key] >= cap {
					continue
				}
				counts[key]++
			}
			dedup.Mark(s.t)
			taken[s.t.ID] = true
			out = append(out, s.t)
		}
	}
	return interleaveByArtist(out)
}

// interleaveByArtist reorders a similar-songs result so the
// same artist never plays twice in a row when the list has
// an alternative — score order is preserved greedily, but a
// radio queue that opens with five songs by the seed artist
// (their scores are highest by construction) feels broken
// even when every pick is defensible.
func interleaveByArtist(in []store.Track) []store.Track {
	if len(in) < 3 {
		return in
	}
	out := make([]store.Track, 0, len(in))
	rest := append([]store.Track{}, in...)
	last := ""
	for len(rest) > 0 {
		idx := 0
		for i, t := range rest {
			if !strings.EqualFold(t.Artist, last) {
				idx = i
				break
			}
		}
		out = append(out, rest[idx])
		last = rest[idx].Artist
		rest = append(rest[:idx], rest[idx+1:]...)
	}
	return out
}

// TrendingQueries seed a fresh catalog with the popular,
// heavily-downloaded music archive.org actually has, so a new
// account opens onto real music instead of whatever a raw genre
// search drags in.
var TrendingQueries = []string{
	"grateful dead", "miles davis", "john coltrane", "billie holiday",
	"louis armstrong", "ella fitzgerald", "charlie parker", "thelonious monk",
	"duke ellington", "bach", "beethoven", "chopin", "mozart",
	"muddy waters", "b.b. king", "johnny cash", "hank williams",
	"pink floyd", "bob marley", "phish", "radiohead",
	"nina simone", "frank sinatra", "woody guthrie",
}

// Warm seeds the catalog from the trending queries, once, while
// the catalog is still small. Safe to call at startup.
func (e *Engine) Warm(ctx context.Context) {
	if !e.warming.CompareAndSwap(false, true) {
		return
	}
	defer e.warming.Store(false)
	if e.store.TrackCount() >= 150 {
		return
	}
	if _, done := e.store.KVGet("warmed"); done {
		return
	}
	for _, q := range TrendingQueries {
		for _, t := range e.registry.SearchAll(ctx, q, 14) {
			t := t
			e.store.UpsertTrack(&t)
		}
	}
	e.store.KVSet("warmed", "1")
}

// Cold reports whether the profile has no listening signal yet —
// fresh accounts get the trending pool instead of personalization.
func (p *Profile) Cold() bool {
	return p.Plays == 0 && len(p.Starred) == 0 && len(p.ArtistScore) == 0
}

// Radio builds an auto-mix queue. Modes:
//
//	personal  — affinity-weighted favorites + similar expansion + a
//	            slice of discovery
//	discovery — mostly new music from sources, seeded by top genres
//	            and artists
//	favorites — starred and most-played
func (e *Engine) Radio(ctx context.Context, userID int64, mode string, count int) []store.Track {
	p := e.BuildProfile(userID)
	// Hourly reshuffle, seeded per mode so the three mixes aren't
	// the same shuffle of the same pool.
	modeSeed := int64(0)
	for _, c := range mode {
		modeSeed = modeSeed*31 + int64(c)
	}
	// Six-hour buckets match the radio playlists' refresh
	// cadence: every refresh is a genuinely new shuffle, not
	// the same mix re-served.
	bucketSeed := time.Now().Unix()/21600 + modeSeed
	rng := rand.New(rand.NewSource(bucketSeed))
	var pool []store.Track

	switch mode {
	case "favorites":
		all, _ := e.store.QueryTracks(store.Query{Limit: 500, Order: "shuffle", Seed: bucketSeed})
		for _, t := range all {
			if p.Starred[t.ID] || p.TrackPlays[t.ID] > 0 {
				pool = append(pool, t)
			}
		}
		if len(pool) == 0 {
			// Cold start: no favorites yet — serve the trending
			// pool as a starter mix instead of an empty playlist.
			pool = append(pool, all...)
		}
	case "discovery":
		// Expand only on artists the user actually listens to.
		// Raw genre-word searches ("jazz", "rock") are what filled
		// fresh catalogs with junk — never search those.
		seeds := p.TopArtists(3)
		for _, s := range seeds {
			for _, t := range e.registry.SearchAll(ctx, s, 12) {
				e.store.UpsertTrack(&t)
				if t.Duration <= 1200 {
					pool = append(pool, t)
				}
			}
		}
		if p.Cold() {
			all, _ := e.store.QueryTracks(store.Query{Limit: 500, Order: "shuffle", Seed: bucketSeed})
			pool = append(pool, all...)
		} else {
			// True discovery: catalog tracks in the user's top
			// genres by artists they have never played. (The
			// seed-artist searches above mostly return those
			// artists' own tracks — expansion, not discovery.)
			topG := map[string]bool{}
			for _, g := range p.TopGenres(3) {
				topG[g] = true
			}
			topA := map[string]bool{}
			for _, a := range p.TopArtists(10) {
				topA[strings.ToLower(a)] = true
			}
			all, _ := e.store.QueryTracks(store.Query{Limit: 500, Order: "shuffle", Seed: bucketSeed})
			for _, t := range all {
				if p.TrackPlays[t.ID] == 0 && topG[t.Genre] && !topA[strings.ToLower(t.Artist)] {
					pool = append(pool, t)
				}
			}
		}
		// anchor with a few familiar tracks
		all, _ := e.store.QueryTracks(store.Query{Limit: 200, Order: "shuffle", Seed: bucketSeed})
		for _, t := range all {
			if p.scoreTrack(t) > 2 {
				pool = append(pool, t)
			}
		}
	default: // personal
		all, _ := e.store.QueryTracks(store.Query{Limit: 500, Order: "shuffle", Seed: bucketSeed})
		pool = append(pool, all...)
		for _, a := range p.TopArtists(2) {
			for _, t := range e.registry.SearchAll(ctx, a, 10) {
				e.store.UpsertTrack(&t)
				if t.Duration <= 1200 {
					pool = append(pool, t)
				}
			}
		}
	}

	// Weighted shuffle: score + noise, per-artist cap, no repeats.
	artistCount := map[string]int{}
	used := map[string]bool{}
	type item struct {
		t store.Track
		w float64
	}
	var items []item
	for _, t := range pool {
		if used[t.ID] {
			continue
		}
		used[t.ID] = true
		// Warm profiles: no-genre, zero-affinity pool entries
		// are search debris (talk content that shares a name
		// with an artist), not music the listener wants.
		if !p.Cold() && t.Genre == "" && p.scoreTrack(t) <= 0 {
			continue
		}
		// Reaction videos, tutorials and lessons title
		// themselves with the artist's name and ride in as
		// "same artist" tracks — never queue them.
		if junkTitle(t) {
			continue
		}
		if t.Duration == 0 && t.Source == "youtube" {
			continue
		}
		w := p.scoreTrack(t) + rng.Float64()*3.5
		if mode == "discovery" && p.TrackPlays[t.ID] == 0 {
			w += 2
		}
		items = append(items, item{t, w})
	}
	sort.Slice(items, func(i, j int) bool { return items[i].w > items[j].w })
	var out []store.Track
	dedup := newSongDeduper()
	// Pass 1 enforces the per-artist cap; pass 2 relaxes the
	// cap for thin pools. The one-version-per-song rule holds
	// in both — a shorter mix beats a repeated song.
	for _, capped := range []bool{true, false} {
		for _, it := range items {
			if len(out) >= count {
				break
			}
			if dedup.Seen(it.t) {
				continue
			}
			key := strings.ToLower(it.t.Artist)
			if capped && artistCount[key] >= 3 {
				continue
			}
			artistCount[key]++
			dedup.Mark(it.t)
			out = append(out, it.t)
		}
	}
	return out
}

// RecommendedArtists: loved artists plus genre-adjacent artists
// present in the catalog that the user hasn't played yet.
func (e *Engine) RecommendedArtists(userID int64, count int) []string {
	p := e.BuildProfile(userID)
	out := p.TopArtists(count / 2)
	topGenres := map[string]bool{}
	for _, g := range p.TopGenres(4) {
		topGenres[g] = true
	}
	seen := map[string]bool{}
	for _, a := range out {
		seen[a] = true
	}
	for _, a := range e.store.Artists() {
		if seen[a] || p.ArtistScore[a] != 0 {
			continue
		}
		tracks, _ := e.store.QueryTracks(store.Query{Artist: a, Limit: 5})
		genreMatch := false
		for _, t := range tracks {
			if topGenres[t.Genre] {
				genreMatch = true
				break
			}
		}
		if genreMatch {
			out = append(out, a)
			seen[a] = true
			if len(out) >= count {
				break
			}
		}
	}
	return out
}
