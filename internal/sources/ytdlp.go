package sources

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"log"
	"os/exec"
	"regexp"
	"strings"
	"sync"
	"time"

	"beatlab/internal/store"
)

// YtDlpSource fetches from YouTube (and any other yt-dlp site) by
// driving the yt-dlp binary when one is installed. It is optional:
// Beatlab stays a single binary; yt-dlp is an external helper the
// user may drop on PATH. Search uses flat playlist extraction;
// stream URLs are resolved lazily and cached briefly because they
// expire.
type YtDlpSource struct {
	bin string

	mu       sync.Mutex
	resolved map[string]resolvedURL
}

type resolvedURL struct {
	url string
	at  time.Time
}

func NewYtDlp(path string) *YtDlpSource {
	if path == "" {
		if p, err := exec.LookPath("yt-dlp"); err == nil {
			path = p
		}
	}
	return &YtDlpSource{bin: path, resolved: map[string]resolvedURL{}}
}

func (y *YtDlpSource) Name() string    { return "youtube" }
func (y *YtDlpSource) Available() bool { return y.bin != "" }
func (y *YtDlpSource) Status() string {
	if y.bin == "" {
		return "yt-dlp not found — install yt-dlp to enable the YouTube source"
	}
	return "yt-dlp at " + y.bin
}

type ytdlpSearchJSON struct {
	Entries []struct {
		ID        string  `json:"id"`
		Title     string  `json:"title"`
		Uploader  string  `json:"uploader"`
		Channel   string  `json:"channel"`
		Duration  float64 `json:"duration"`
		Thumbnail string  `json:"thumbnail"`
		// Flat-playlist output carries thumbnails[]; the singular
		// thumbnail field is only filled by full extraction.
		Thumbnails []struct {
			URL   string `json:"url"`
			Width int    `json:"width"`
		} `json:"thumbnails"`
		URL     string `json:"url"`
		Webpage string `json:"webpage_url"`
	} `json:"entries"`
}

func (y *YtDlpSource) Search(ctx context.Context, query string, limit int) ([]store.Track, error) {
	if y.bin == "" {
		return nil, ErrNotConfigured
	}
	// yt-dlp can stall for minutes on a throttled connection; the
	// search must never wait on it unbounded.
	ctx, cancel := context.WithTimeout(ctx, 25*time.Second)
	defer cancel()
	n := limit
	if n > 30 {
		n = 30
	}
	cmd := exec.CommandContext(ctx, y.bin, "--flat-playlist", "-J", "--no-warnings",
		fmt.Sprintf("ytsearch%d:%s", n, query))
	var stderr bytes.Buffer
	cmd.Stderr = &stderr
	out, err := cmd.Output()
	if err != nil {
		// yt-dlp failing (bot wall, outdated build, no network)
		// must not be indistinguishable from "no results" — the
		// log is the only place that difference is visible.
		log.Printf("[beatlab] yt-dlp search %q failed: %v: %s", query, err, firstLine(stderr.String()))
		return nil, err
	}
	var parsed ytdlpSearchJSON
	if err := json.Unmarshal(out, &parsed); err != nil {
		return nil, err
	}
	var tracks []store.Track
	for _, e := range parsed.Entries {
		if e.ID == "" {
			continue
		}
		// Songs, not broadcasts — the same rule archive.org
		// applies. Flat results also surface mixes, livestreams
		// and full-album uploads; deleted/private entries come
		// back with placeholder titles.
		if e.Duration > 1200 {
			continue
		}
		if e.Title == "[Deleted video]" || e.Title == "[Private video]" {
			continue
		}
		channel := e.Uploader
		if channel == "" {
			channel = e.Channel
		}
		channel = strings.TrimSuffix(strings.TrimSpace(channel), " - Topic")
		artist, title := parseYtTitle(e.Title, channel)
		if artist == "" {
			continue // unattributed — the catalog drops these on the archive side too
		}
		// Album: the channel (a session/compilation source like
		// "THE FIRST TAKE" reads correctly as an album), or the
		// artist for Topic/self uploads. Never a bare "YouTube" —
		// that collapsed every upload into one junk album.
		album := channel
		if album == "" || strings.EqualFold(album, artist) {
			album = artist
		}
		page := e.Webpage
		if page == "" {
			page = "https://www.youtube.com/watch?v=" + e.ID
		}
		cover := e.Thumbnail
		if cover == "" {
			bestW := -1
			for _, th := range e.Thumbnails {
				if th.URL != "" && th.Width >= bestW {
					bestW = th.Width
					cover = th.URL
				}
			}
		}
		tracks = append(tracks, store.Track{
			ID:        "youtube:" + e.ID,
			Source:    "youtube",
			Title:     title,
			Artist:    artist,
			Album:     album,
			Duration:  int(e.Duration),
			Format:    "opus",
			CoverURL:  cover,
			StreamURL: "ytdlp:" + page, // resolved on demand by Resolve
			Meta:      page,
		})
	}
	return tracks, nil
}

var ytJunkSuffix = regexp.MustCompile(`(?i)\s*[\(\[][^\)\]]*(official|lyric|audio|video|mv|hd|4k|remaster|visualizer)[^\)\]]*[\)\]]`)

// parseYtTitle splits the ubiquitous "Artist - Title" video
// naming, strips "(Official Video)"-style suffixes, and falls
// back to the channel as artist.
// firstLine returns the first non-empty line of s, truncated —
// enough of a subprocess's stderr to diagnose a failure in the
// server log without flooding it.
func firstLine(s string) string {
	for _, ln := range strings.Split(s, "\n") {
		ln = strings.TrimSpace(ln)
		if ln != "" {
			if len(ln) > 200 {
				ln = ln[:200]
			}
			return ln
		}
	}
	return ""
}

func parseYtTitle(raw, channel string) (artist, title string) {
	// Normalize en/em-dash separators so "Artist – Title" splits
	// the same way "Artist - Title" does.
	raw = strings.ReplaceAll(strings.ReplaceAll(raw, " – ", " - "), " — ", " - ")
	title = ytJunkSuffix.ReplaceAllString(strings.TrimSpace(raw), "")
	title = strings.Join(strings.Fields(title), " ")
	// Japanese uploads commonly use "Artist / Title" (or the
	// fullwidth equivalent). The channel stays the artist of
	// record, but the title must not keep the prefix — a title
	// of "Reira Ushio / Harenohini ..." never dedupes against
	// "Harenohini". Guarded to CJK contexts so a latin title
	// like "Love / Hate" is left alone.
	title = strings.ReplaceAll(title, "\uFF0F", " / ")
	if i := strings.Index(title, " / "); i > 0 && (containsCJK(channel) || containsCJK(title[:i])) {
		title = strings.TrimSpace(title[i+3:])
	}
	if i := strings.Index(title, " - "); i > 0 {
		artist = strings.TrimSuffix(strings.TrimSpace(title[:i]), " - Topic")
		title = strings.TrimSpace(title[i+3:])
	}
	if artist == "" {
		artist = channel
	}
	if title == "" {
		title = raw
	}
	return artist, title
}

func containsCJK(s string) bool {
	for _, r := range s {
		if (r >= 0x3000 && r <= 0x30FF) || (r >= 0x4E00 && r <= 0x9FFF) || (r >= 0xFF00 && r <= 0xFFEF) {
			return true
		}
	}
	return false
}

// Resolve turns a "ytdlp:<page>" stream reference into a direct,
// short-lived media URL.
func (y *YtDlpSource) Resolve(ctx context.Context, ref string) (string, error) {
	page := strings.TrimPrefix(ref, "ytdlp:")
	y.mu.Lock()
	if r, ok := y.resolved[page]; ok && time.Since(r.at) < 25*time.Minute {
		y.mu.Unlock()
		return r.url, nil
	}
	y.mu.Unlock()
	// Same stall guard as Search: playback start must not hang
	// on a stuck yt-dlp either.
	ctx, cancel := context.WithTimeout(ctx, 20*time.Second)
	defer cancel()
	cmd := exec.CommandContext(ctx, y.bin, "-g", "-f", "bestaudio/best", "--no-warnings", page)
	var stderr bytes.Buffer
	cmd.Stderr = &stderr
	out, err := cmd.Output()
	if err != nil {
		log.Printf("[beatlab] yt-dlp resolve %s failed: %v: %s", page, err, firstLine(stderr.String()))
		return "", err
	}
	u := strings.TrimSpace(strings.Split(string(out), "\n")[0])
	if u == "" {
		return "", fmt.Errorf("yt-dlp returned no stream URL")
	}
	y.mu.Lock()
	if len(y.resolved) > 256 {
		for k, r := range y.resolved {
			if time.Since(r.at) > 25*time.Minute {
				delete(y.resolved, k)
			}
		}
	}
	y.resolved[page] = resolvedURL{url: u, at: time.Now()}
	y.mu.Unlock()
	return u, nil
}
