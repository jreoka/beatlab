// Package sources defines where Beatlab fetches music from. A Source
// searches a catalog and yields normalized tracks whose StreamURL
// can be played (and cached) by the engine. This mirrors Bitlab's
// indexer model: many sources, one normalized result stream.
package sources

import (
	"context"
	"errors"
	"time"

	"beatlab/internal/store"
)

// ErrNotConfigured marks a source that exists in the registry but is
// not usable in this deployment (e.g. torrent source without any
// configured indexers).
var ErrNotConfigured = errors.New("source not configured")

type Source interface {
	// Name is the stable source id used in track IDs.
	Name() string
	// Available reports whether the source can serve requests now.
	Available() bool
	// Status is a human-readable line for the admin console.
	Status() string
	// Search returns up to limit tracks matching the query.
	Search(ctx context.Context, query string, limit int) ([]store.Track, error)
}

type Registry struct {
	list []Source

	// Sink, when set, receives tracks from sources that finish
	// after SearchAll's soft deadline — late results still land
	// in the catalog, so a repeated search finds them instantly.
	Sink func(store.Track)
}

func NewRegistry(list ...Source) *Registry { return &Registry{list: list} }

func (r *Registry) All() []Source { return r.list }

func (r *Registry) Get(name string) (Source, bool) {
	for _, s := range r.list {
		if s.Name() == name {
			return s, true
		}
	}
	return nil, false
}

// SearchSoftDeadline is how long SearchAll waits for sources
// before answering with what has arrived. Sources get a longer
// hard timeout of their own; whoever misses the soft deadline
// keeps running in the background and their results are fed to
// the registry Sink (the catalog), so the next search for the
// same thing is instant. Without this, one stuck source — a
// yt-dlp process waiting on a throttled YouTube, say — hangs
// the client's search spinner forever.
var SearchSoftDeadline = 6 * time.Second

const searchSourceTimeout = 30 * time.Second

func (r *Registry) SearchAll(ctx context.Context, query string, limit int) []store.Track {
	var avail []Source
	for _, s := range r.list {
		if s.Available() {
			avail = append(avail, s)
		}
	}
	if len(avail) == 0 {
		return nil
	}
	type result struct{ tracks []store.Track }
	ch := make(chan result, len(avail))
	for _, s := range avail {
		go func(s Source) {
			sctx, cancel := context.WithTimeout(ctx, searchSourceTimeout)
			defer cancel()
			tracks, err := s.Search(sctx, query, limit)
			if err != nil {
				tracks = nil
			}
			ch <- result{tracks}
		}(s)
	}

	var perSource [][]store.Track
	remaining := len(avail)
	timer := time.NewTimer(SearchSoftDeadline)
	defer timer.Stop()
loop:
	for remaining > 0 {
		select {
		case res := <-ch:
			remaining--
			if len(res.tracks) > 0 {
				perSource = append(perSource, res.tracks)
			}
		case <-timer.C:
			break loop
		case <-ctx.Done():
			break loop
		}
	}
	if remaining > 0 && r.Sink != nil {
		go func(left int) {
			for ; left > 0; left-- {
				res := <-ch
				for _, t := range res.tracks {
					r.Sink(t)
				}
			}
		}(remaining)
	}
	return mergeRoundRobin(perSource, limit)
}

// mergeRoundRobin interleaves per-source lists: filling the
// result set from the first source alone would starve the
// others (archive.org nearly always fills the limit by itself,
// which silently hid YouTube results entirely).
func mergeRoundRobin(perSource [][]store.Track, limit int) []store.Track {
	seen := map[string]bool{}
	var out []store.Track
	for i := 0; ; i++ {
		added := false
		for _, tracks := range perSource {
			if i >= len(tracks) {
				continue
			}
			added = true
			t := tracks[i]
			if seen[t.ID] {
				continue
			}
			seen[t.ID] = true
			out = append(out, t)
			if len(out) >= limit {
				return out
			}
		}
		if !added {
			return out
		}
	}
}
