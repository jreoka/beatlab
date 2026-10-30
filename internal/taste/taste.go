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
func (e *Engine) Similar(ctx context.Context, userID int64, seed store.Track, count int) []store.Track {
	p := e.BuildProfile(userID)
	// Targeted queries, not a catalog window: QueryTracks'
	// default order is alphabetical by artist, so a flat
	// Limit-N window silently exiles every late-alphabet
	// artist once the catalog outgrows it.
	var cands []store.Track
	if seed.Artist != "" {
		rows, _ := e.store.QueryTracks(store.Query{Artist: seed.Artist, Limit: 500})
		cands = append(cands, rows...)
	}
	if seed.Genre != "" {
		rows, _ := e.store.QueryTracks(store.Query{Genre: seed.Genre, Limit: 500, Order: "shuffle", Seed: time.Now().Unix() / 86400})
		cands = append(cands, rows...)
	}
	seen := map[string]bool{seed.ID: true}
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
		switch {
		case strings.EqualFold(t.Artist, seed.Artist) && seed.Artist != "":
			add(t, 4)
		case seed.Genre != "" && strings.EqualFold(t.Genre, seed.Genre):
			add(t, 1.5)
		}
	}
	if len(list) < count && seed.Artist != "" {
		for _, t := range e.registry.SearchAll(ctx, seed.Artist, 25) {
			e.store.UpsertTrack(&t)
			// A bare artist-name search drags in everything the
			// name touches — for "Kansas", politics podcasts
			// about the state. Expansion results must look like
			// songs: song-length, and (for other artists) in the
			// seed's genre.
			if t.Duration > 1200 {
				continue
			}
			if strings.EqualFold(t.Artist, seed.Artist) {
				add(t, 3.5)
			} else if seed.Genre != "" && strings.EqualFold(t.Genre, seed.Genre) {
				add(t, 1)
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
	keys := map[string]bool{}
	var out []store.Track
	// Pass 1: per-artist caps + one version per song. Pass 2:
	// caps off, still one version per song — a queue with the
	// seed song three times in different versions is the bug
	// this prevents, and a shorter queue beats a repeated song.
	for pass := 0; pass < 2; pass++ {
		for _, s := range list {
			if len(out) >= count {
				return out
			}
			if taken[s.t.ID] {
				continue
			}
			sk := s.t.SongKey()
			if keys[sk] {
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
			keys[sk] = true
			taken[s.t.ID] = true
			out = append(out, s.t)
		}
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
		w := p.scoreTrack(t) + rng.Float64()*3.5
		if mode == "discovery" && p.TrackPlays[t.ID] == 0 {
			w += 2
		}
		items = append(items, item{t, w})
	}
	sort.Slice(items, func(i, j int) bool { return items[i].w > items[j].w })
	var out []store.Track
	songKeys := map[string]bool{}
	// Pass 1 enforces the per-artist cap; pass 2 relaxes the
	// cap for thin pools. The one-version-per-song rule holds
	// in both — a shorter mix beats a repeated song.
	for _, capped := range []bool{true, false} {
		for _, it := range items {
			if len(out) >= count {
				break
			}
			sk := it.t.SongKey()
			if songKeys[sk] {
				continue
			}
			key := strings.ToLower(it.t.Artist)
			if capped && artistCount[key] >= 3 {
				continue
			}
			artistCount[key]++
			songKeys[sk] = true
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
