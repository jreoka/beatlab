package sources

import (
	"context"

	"beatlab/internal/store"
)

// TorrentSource is the direct analog of Bitlab's engine: search
// user-configured Torznab indexers for music releases and stream
// audio files out of the swarm with anacrolix/torrent, caching to
// disk. The wiring point exists (it registers in the source list
// and reports its status), but indexer configuration and the
// streaming engine land in phase 2 — until then it reports itself
// unavailable rather than silently returning nothing.
type TorrentSource struct {
	indexers []string // Torznab base URLs (phase 2)
}

func NewTorrent() *TorrentSource { return &TorrentSource{} }

func (t *TorrentSource) Name() string    { return "torrent" }
func (t *TorrentSource) Available() bool { return false }
func (t *TorrentSource) Status() string {
	return "torrent indexers — phase 2 (Torznab search + swarm streaming, Bitlab-style)"
}

func (t *TorrentSource) Search(ctx context.Context, query string, limit int) ([]store.Track, error) {
	return nil, ErrNotConfigured
}
