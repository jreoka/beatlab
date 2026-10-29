// Package engine moves audio: it resolves a track's stream URL,
// proxies it to the client, and caches completed downloads to disk
// under a size budget with LRU eviction — the same warm-cache model
// Bitlab uses for video, scaled to audio-sized files.
package engine

import (
	"context"
	"crypto/sha1"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"time"

	"beatlab/internal/sources"
	"beatlab/internal/store"
)

type Engine struct {
	store     *store.Store
	registry  *sources.Registry
	cacheDir  string
	coverDir  string
	budget    int64
	http      *http.Client
	userAgent string
}

func New(st *store.Store, reg *sources.Registry, dataDir string, cacheGB int) *Engine {
	e := &Engine{
		store:     st,
		registry:  reg,
		cacheDir:  filepath.Join(dataDir, "cache", "audio"),
		coverDir:  filepath.Join(dataDir, "cache", "covers"),
		budget:    int64(cacheGB) << 30,
		http:      &http.Client{Timeout: 0},
		userAgent: "Beatlab/0.1",
	}
	os.MkdirAll(e.cacheDir, 0o755)
	os.MkdirAll(e.coverDir, 0o755)
	return e
}

func ContentType(format string) string {
	switch strings.ToLower(format) {
	case "mp3":
		return "audio/mpeg"
	case "flac":
		return "audio/flac"
	case "ogg", "opus":
		return "audio/ogg"
	case "m4a", "aac":
		return "audio/mp4"
	case "wav":
		return "audio/wav"
	default:
		return "application/octet-stream"
	}
}

func cacheName(trackID string) string {
	h := sha1.Sum([]byte(trackID))
	return fmt.Sprintf("%x", h)
}

func (e *Engine) resolve(ctx context.Context, t *store.Track) (string, error) {
	if strings.HasPrefix(t.StreamURL, "ytdlp:") {
		src, ok := e.registry.Get("youtube")
		if !ok {
			return "", fmt.Errorf("youtube source missing")
		}
		if resolver, ok := src.(interface {
			Resolve(context.Context, string) (string, error)
		}); ok {
			return resolver.Resolve(ctx, t.StreamURL)
		}
		return "", fmt.Errorf("youtube source cannot resolve streams")
	}
	return t.StreamURL, nil
}

// ServeTrack streams a track to the client, caching full downloads.
func (e *Engine) ServeTrack(w http.ResponseWriter, r *http.Request, t *store.Track) {
	if ent, ok := e.store.CacheGet(t.ID); ok {
		if f, err := os.Open(ent.Path); err == nil {
			defer f.Close()
			if st, err := f.Stat(); err == nil {
				e.store.CacheTouch(t.ID)
				w.Header().Set("Content-Type", ContentType(t.Format))
				http.ServeContent(w, r, t.Title+"."+t.Format, st.ModTime(), f)
				return
			}
		}
		e.store.CacheDelete(t.ID)
	}

	url, err := e.resolve(r.Context(), t)
	if err != nil || url == "" {
		http.Error(w, "stream unavailable", http.StatusBadGateway)
		return
	}
	upReq, _ := http.NewRequestWithContext(r.Context(), "GET", url, nil)
	upReq.Header.Set("User-Agent", e.userAgent)
	clientRange := r.Header.Get("Range")
	if clientRange != "" {
		upReq.Header.Set("Range", clientRange)
	}
	resp, err := e.http.Do(upReq)
	if err != nil {
		http.Error(w, "upstream error", http.StatusBadGateway)
		return
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK && resp.StatusCode != http.StatusPartialContent {
		http.Error(w, "upstream returned "+resp.Status, http.StatusBadGateway)
		return
	}
	for _, h := range []string{"Content-Length", "Content-Range", "Accept-Ranges", "Last-Modified", "ETag"} {
		if v := resp.Header.Get(h); v != "" {
			w.Header().Set(h, v)
		}
	}
	ct := resp.Header.Get("Content-Type")
	if ct == "" || ct == "application/octet-stream" {
		ct = ContentType(t.Format)
	}
	w.Header().Set("Content-Type", ct)
	w.WriteHeader(resp.StatusCode)

	// Cache only clean, complete, non-range downloads.
	cacheable := clientRange == "" && resp.StatusCode == http.StatusOK && resp.ContentLength > 0
	if !cacheable {
		io.Copy(w, resp.Body)
		return
	}
	tmp := filepath.Join(e.cacheDir, cacheName(t.ID)+".part")
	f, err := os.Create(tmp)
	if err != nil {
		io.Copy(w, resp.Body)
		return
	}
	written, copyErr := io.Copy(io.MultiWriter(w, f), resp.Body)
	f.Close()
	if copyErr == nil && written == resp.ContentLength {
		final := filepath.Join(e.cacheDir, cacheName(t.ID)+"."+t.Format)
		if os.Rename(tmp, final) == nil {
			e.store.CachePut(t.ID, final, written)
			e.evict()
		} else {
			os.Remove(tmp)
		}
	} else {
		os.Remove(tmp)
	}
}

func (e *Engine) evict() {
	if e.budget <= 0 {
		return
	}
	for e.store.CacheBytes() > e.budget {
		entries := e.store.CacheEntries() // oldest first
		if len(entries) == 0 {
			return
		}
		oldest := entries[0]
		os.Remove(oldest.Path)
		e.store.CacheDelete(oldest.TrackID)
	}
}

// ServeCover returns a track's cover art, disk-cached after first fetch.
func (e *Engine) ServeCover(w http.ResponseWriter, r *http.Request, t *store.Track) {
	if t.CoverURL == "" {
		http.NotFound(w, r)
		return
	}
	path := filepath.Join(e.coverDir, cacheName(t.ID)+".img")
	if data, err := os.ReadFile(path); err == nil && len(data) > 0 {
		w.Header().Set("Content-Type", http.DetectContentType(data))
		w.Header().Set("Cache-Control", "public, max-age=86400")
		w.Write(data)
		return
	}
	req, _ := http.NewRequestWithContext(r.Context(), "GET", t.CoverURL, nil)
	req.Header.Set("User-Agent", e.userAgent)
	resp, err := e.http.Do(req)
	if err != nil || resp.StatusCode != http.StatusOK {
		if resp != nil {
			resp.Body.Close()
		}
		http.NotFound(w, r)
		return
	}
	defer resp.Body.Close()
	data, err := io.ReadAll(io.LimitReader(resp.Body, 8<<20))
	if err != nil || len(data) == 0 {
		http.NotFound(w, r)
		return
	}
	os.WriteFile(path, data, 0o644)
	ct := resp.Header.Get("Content-Type")
	if ct == "" {
		ct = http.DetectContentType(data)
	}
	w.Header().Set("Content-Type", ct)
	w.Header().Set("Cache-Control", "public, max-age=86400")
	w.Write(data)
}

var _ = time.Now
