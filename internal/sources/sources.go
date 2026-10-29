// Package sources defines where Beatlab fetches music from. A Source
// searches a catalog and yields normalized tracks whose StreamURL
// can be played (and cached) by the engine. This mirrors Bitlab's
// indexer model: many sources, one normalized result stream.
package sources

import (
	"context"
	"errors"

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

// SearchAll fans a query out to every available source and merges
// the results (first source wins on duplicate track IDs).
func (r *Registry) SearchAll(ctx context.Context, query string, limit int) []store.Track {
	// Query every source, then merge round-robin: filling the
	// result set from the first source alone would starve the
	// others (archive.org nearly always fills the limit by
	// itself, which silently hid YouTube results entirely).
	var perSource [][]store.Track
	for _, s := range r.list {
		if !s.Available() {
			continue
		}
		tracks, err := s.Search(ctx, query, limit)
		if err != nil {
			continue
		}
		perSource = append(perSource, tracks)
	}
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
