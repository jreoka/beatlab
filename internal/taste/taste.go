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
	cands, _ := e.store.QueryTracks(store.Query{Limit: 400})
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
			if strings.EqualFold(t.Artist, seed.Artist) {
				add(t, 3.5)
			} else {
				add(t, 1)
			}
		}
	}
	sort.Slice(list, func(i, j int) bool { return list[i].s > list[j].s })
	var out []store.Track
	for i, s := range list {
		if i >= count {
			break
		}
		out = append(out, s.t)
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
	rng := rand.New(rand.NewSource(time.Now().Unix()/3600 + modeSeed))
	var pool []store.Track

	switch mode {
	case "favorites":
		all, _ := e.store.QueryTracks(store.Query{Limit: 500})
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
				pool = append(pool, t)
			}
		}
		if p.Cold() {
			all, _ := e.store.QueryTracks(store.Query{Limit: 500})
			pool = append(pool, all...)
		}
		// anchor with a few familiar tracks
		all, _ := e.store.QueryTracks(store.Query{Limit: 200})
		for _, t := range all {
			if p.scoreTrack(t) > 2 {
				pool = append(pool, t)
			}
		}
	default: // personal
		all, _ := e.store.QueryTracks(store.Query{Limit: 500})
		pool = append(pool, all...)
		for _, a := range p.TopArtists(2) {
			for _, t := range e.registry.SearchAll(ctx, a, 10) {
				e.store.UpsertTrack(&t)
				pool = append(pool, t)
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
		w := p.scoreTrack(t) + rng.Float64()*2.5
		if mode == "discovery" && p.TrackPlays[t.ID] == 0 {
			w += 2
		}
		items = append(items, item{t, w})
	}
	sort.Slice(items, func(i, j int) bool { return items[i].w > items[j].w })
	var out []store.Track
	for _, it := range items {
		if len(out) >= count {
			break
		}
		key := strings.ToLower(it.t.Artist)
		if artistCount[key] >= 3 {
			continue
		}
		artistCount[key]++
		out = append(out, it.t)
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
		tracks, _ := e.store.QueryTracks(store.Query{Artist: a, Limit: 1})
		if len(tracks) > 0 && topGenres[tracks[0].Genre] {
			out = append(out, a)
			seen[a] = true
			if len(out) >= count {
				break
			}
		}
	}
	return out
}
