// Package engine moves audio: it resolves a track's stream URL,
// proxies it to the client, and caches completed downloads to disk
// under a size budget with LRU eviction — the same warm-cache model
// Bitlab uses for video, scaled to audio-sized files.
package engine

import (
	"bytes"
	"context"
	"crypto/sha1"
	"fmt"
	"hash/fnv"
	"image"
	"image/color"
	"image/png"
	"io"
	"math"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"sync"
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

	genMu    sync.Mutex
	genCache map[string][]byte
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

// ServeCover returns a track's real cover art, disk-cached after
// first fetch. It reports whether it served anything — callers
// fall back to generated art when it didn't.
func (e *Engine) ServeCover(w http.ResponseWriter, r *http.Request, t *store.Track) bool {
	if t.CoverURL == "" {
		return false
	}
	path := filepath.Join(e.coverDir, cacheName(t.ID)+".img")
	if data, err := os.ReadFile(path); err == nil && len(data) > 0 {
		w.Header().Set("Content-Type", http.DetectContentType(data))
		w.Header().Set("Cache-Control", "public, max-age=86400")
		w.Write(data)
		return true
	}
	req, _ := http.NewRequestWithContext(r.Context(), "GET", t.CoverURL, nil)
	req.Header.Set("User-Agent", e.userAgent)
	resp, err := e.http.Do(req)
	if err != nil || resp.StatusCode != http.StatusOK {
		if resp != nil {
			resp.Body.Close()
		}
		return false
	}
	defer resp.Body.Close()
	data, err := io.ReadAll(io.LimitReader(resp.Body, 8<<20))
	if err != nil || len(data) == 0 {
		return false
	}
	os.WriteFile(path, data, 0o644)
	ct := resp.Header.Get("Content-Type")
	if ct == "" {
		ct = http.DetectContentType(data)
	}
	w.Header().Set("Content-Type", ct)
	w.Header().Set("Cache-Control", "public, max-age=86400")
	w.Write(data)
	return true
}

// ServeGenerated serves deterministic generated artwork for a
// seed (usually an album id): a deep two-tone gradient in a
// hash-derived hue with the equalizer-bars motif. Much of the
// open catalog (live recordings especially) simply has no cover
// art — a designed tile beats a broken-image placeholder, and
// because it's derived from the album id, every track and view
// of the album shows the same art.
func (e *Engine) ServeGenerated(w http.ResponseWriter, seed string) {
	e.genMu.Lock()
	if e.genCache == nil {
		e.genCache = map[string][]byte{}
	}
	data, ok := e.genCache[seed]
	e.genMu.Unlock()
	if !ok {
		data = renderGeneratedCover(seed)
		e.genMu.Lock()
		if len(e.genCache) < 512 {
			e.genCache[seed] = data
		}
		e.genMu.Unlock()
	}
	w.Header().Set("Content-Type", "image/png")
	w.Header().Set("Cache-Control", "public, max-age=86400")
	w.Write(data)
}

func hsl(h, s, l float64) color.RGBA {
	// h in [0,360), s and l in [0,1]
	c := (1 - math.Abs(2*l-1)) * s
	x := c * (1 - math.Abs(math.Mod(h/60, 2)-1))
	m := l - c/2
	var r, g, b float64
	switch {
	case h < 60:
		r, g, b = c, x, 0
	case h < 120:
		r, g, b = x, c, 0
	case h < 180:
		r, g, b = 0, c, x
	case h < 240:
		r, g, b = 0, x, c
	case h < 300:
		r, g, b = x, 0, c
	default:
		r, g, b = c, 0, x
	}
	return color.RGBA{uint8((r + m) * 255), uint8((g + m) * 255), uint8((b + m) * 255), 255}
}

func renderGeneratedCover(seed string) []byte {
	const n = 400
	h := fnv.New32a()
	h.Write([]byte(seed))
	sum := h.Sum32()
	hue := float64(sum % 360)
	hue2 := float64((sum / 360) % 360)
	rng := sum
	next := func() uint32 { rng = rng*1664525 + 1013904223; return rng >> 8 }

	img := image.NewRGBA(image.Rect(0, 0, n, n))
	top := hsl(hue, 0.45, 0.10)
	bottom := hsl(hue2, 0.5, 0.20)
	for y := 0; y < n; y++ {
		f := float64(y) / n
		c := color.RGBA{
			uint8(float64(top.R)*(1-f) + float64(bottom.R)*f),
			uint8(float64(top.G)*(1-f) + float64(bottom.G)*f),
			uint8(float64(top.B)*(1-f) + float64(bottom.B)*f),
			255,
		}
		for x := 0; x < n; x++ {
			img.Set(x, y, c)
		}
	}
	// Equalizer bars, heights from the hash — the favicon motif
	// at album scale.
	accent := hsl(hue, 0.85, 0.68)
	bars := 5
	barW, gap := 34, 22
	totalW := bars*barW + (bars-1)*gap
	x0 := (n - totalW) / 2
	for i := 0; i < bars; i++ {
		bh := 60 + int(next()%140)
		x1 := x0 + i*(barW+gap)
		y1 := (n - bh) / 2
		for y := y1; y < y1+bh; y++ {
			for x := x1; x < x1+barW; x++ {
				// rounded caps
				if (y-y1 < 8 || y1+bh-y <= 8) && (x-x1 < 6 || x1+barW-x <= 6) {
					continue
				}
				img.Set(x, y, accent)
			}
		}
	}
	var buf bytes.Buffer
	png.Encode(&buf, img)
	return buf.Bytes()
}

var _ = time.Now
