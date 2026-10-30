package engine

import (
	"context"
	"path/filepath"
	"sync/atomic"
	"testing"
	"time"

	"beatlab/internal/sources"
	"beatlab/internal/store"
)

type fakeResolver struct{ calls *int64 }

func (f fakeResolver) Name() string    { return "youtube" }
func (f fakeResolver) Available() bool { return true }
func (f fakeResolver) Status() string  { return "fake" }
func (f fakeResolver) Search(ctx context.Context, q string, limit int) ([]store.Track, error) {
	return nil, nil
}
func (f fakeResolver) Resolve(ctx context.Context, ref string) (string, error) {
	atomic.AddInt64(f.calls, 1)
	return "https://cdn.example/audio?id=" + ref, nil
}

// A resolved stream URL must be served from the persistent
// cache on the second play — including after a restart
// (a new Engine over the same store) — instead of paying a
// resolver call per play.
func TestResolvePersistsAcrossRestarts(t *testing.T) {
	st, err := store.Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	var calls int64
	reg := sources.NewRegistry(fakeResolver{calls: &calls})
	tr := &store.Track{ID: "youtube:x", StreamURL: "ytdlp:https://youtube.com/watch?v=x"}
	e1 := New(st, reg, t.TempDir(), 1)
	u1, err := e1.resolve(context.Background(), tr)
	if err != nil || u1 == "" {
		t.Fatalf("first resolve: %v %q", err, u1)
	}
	if _, err := e1.resolve(context.Background(), tr); err != nil {
		t.Fatal(err)
	}
	e2 := New(st, reg, filepath.Join(t.TempDir(), "other"), 1)
	if _, err := e2.resolve(context.Background(), tr); err != nil {
		t.Fatal(err)
	}
	if got := atomic.LoadInt64(&calls); got != 1 {
		t.Fatalf("resolver called %d times, want 1 (persistent cache)", got)
	}
	_ = time.Now
}
