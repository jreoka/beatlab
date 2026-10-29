package sources

import (
	"context"
	"encoding/json"
	"fmt"
	"os/exec"
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
		URL       string  `json:"url"`
		Webpage   string  `json:"webpage_url"`
	} `json:"entries"`
}

func (y *YtDlpSource) Search(ctx context.Context, query string, limit int) ([]store.Track, error) {
	if y.bin == "" {
		return nil, ErrNotConfigured
	}
	n := limit
	if n > 15 {
		n = 15
	}
	cmd := exec.CommandContext(ctx, y.bin, "--flat-playlist", "-J", "--no-warnings",
		fmt.Sprintf("ytsearch%d:%s", n, query))
	out, err := cmd.Output()
	if err != nil {
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
		artist := e.Uploader
		if artist == "" {
			artist = e.Channel
		}
		artist = strings.TrimSuffix(artist, " - Topic")
		page := e.Webpage
		if page == "" {
			page = "https://www.youtube.com/watch?v=" + e.ID
		}
		tracks = append(tracks, store.Track{
			ID:        "youtube:" + e.ID,
			Source:    "youtube",
			Title:     e.Title,
			Artist:    artist,
			Album:     "YouTube",
			Duration:  int(e.Duration),
			Format:    "opus",
			CoverURL:  e.Thumbnail,
			StreamURL: "ytdlp:" + page, // resolved on demand by Resolve
			Meta:      page,
		})
	}
	return tracks, nil
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
	cmd := exec.CommandContext(ctx, y.bin, "-g", "-f", "bestaudio/best", "--no-warnings", page)
	out, err := cmd.Output()
	if err != nil {
		return "", err
	}
	u := strings.TrimSpace(strings.Split(string(out), "\n")[0])
	if u == "" {
		return "", fmt.Errorf("yt-dlp returned no stream URL")
	}
	y.mu.Lock()
	y.resolved[page] = resolvedURL{url: u, at: time.Now()}
	y.mu.Unlock()
	return u, nil
}
