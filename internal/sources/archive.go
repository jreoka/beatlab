package sources

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/url"
	"sort"
	"strings"
	"sync"
	"time"

	"beatlab/internal/store"
)

// ArchiveSource fetches music from the Internet Archive
// (archive.org): a huge, keyless, legally-open audio catalog with
// direct download URLs that support HTTP range requests — ideal as
// Beatlab's always-on default source.
type ArchiveSource struct {
	http *http.Client
}

func NewArchive() *ArchiveSource {
	return &ArchiveSource{http: &http.Client{Timeout: 30 * time.Second}}
}

func (a *ArchiveSource) Name() string    { return "archive" }
func (a *ArchiveSource) Available() bool { return true }
func (a *ArchiveSource) Status() string  { return "archive.org — always on, no key needed" }

type iaSearchResponse struct {
	Response struct {
		Docs []struct {
			Identifier string `json:"identifier"`
			Title      string `json:"title"`
			// archive.org returns creator as a string OR an array
			// depending on the item — decode tolerantly or the
			// whole response fails to unmarshal.
			Creator any `json:"creator"`
			Year    any `json:"year"`
		} `json:"docs"`
	} `json:"response"`
}

type iaMetadata struct {
	Metadata struct {
		Title      string `json:"title"`
		Creator    any    `json:"creator"` // string or []string
		Year       string `json:"date"`
		Subject    any    `json:"subject"`
		Collection any    `json:"collection"` // string or []string
	} `json:"metadata"`
	Files []struct {
		Name    string `json:"name"`
		Format  string `json:"format"`
		Length  string `json:"length"` // "3:47" or seconds
		Size    string `json:"size"`
		Track   string `json:"track"`
		Title   string `json:"title"`
		Artist  string `json:"artist"`
		Album   string `json:"album"`
		Creator string `json:"creator"`
	} `json:"files"`
}

// canonicalGenres maps archive.org's free-text subject tags onto
// a small set of real music genres. Subjects are uploader chaos —
// semicolon-joined lists, reversed artist names ("Holiday,
// Billie"), event descriptions — so anything that isn't clearly
// a genre is dropped rather than surfaced as one.
var canonicalGenres = map[string]string{
	"jazz": "Jazz", "bebop": "Jazz", "swing": "Jazz", "big band": "Jazz",
	"blues": "Blues", "rock": "Rock", "rock and roll": "Rock", "rock & roll": "Rock",
	"pop": "Pop", "classical": "Classical", "classic": "Classical", "opera": "Classical",
	"electronic": "Electronic", "techno": "Electronic", "house": "Electronic",
	"ambient": "Ambient", "hip hop": "Hip-Hop", "hip-hop": "Hip-Hop", "rap": "Hip-Hop",
	"folk": "Folk", "country": "Country", "bluegrass": "Country",
	"soul": "Soul", "funk": "Funk", "r&b": "R&B", "rhythm and blues": "R&B",
	"reggae": "Reggae", "metal": "Metal", "punk": "Punk",
	"latin": "Latin", "gospel": "Gospel", "soundtrack": "Soundtrack",
	"world": "World", "singer-songwriter": "Folk",
}

func canonicalGenre(subject any) string {
	var raw []string
	switch s := subject.(type) {
	case string:
		raw = append(raw, s)
	case []any:
		for _, p := range s {
			if str, ok := p.(string); ok {
				raw = append(raw, str)
			}
		}
	}
	for _, r := range raw {
		for _, part := range strings.FieldsFunc(r, func(c rune) bool { return c == ';' || c == ',' }) {
			key := strings.TrimSpace(strings.ToLower(part))
			if g, ok := canonicalGenres[key]; ok {
				return g
			}
		}
	}
	return ""
}

func creatorString(v any) string {
	switch c := v.(type) {
	case string:
		return c
	case []any:
		var parts []string
		for _, p := range c {
			if s, ok := p.(string); ok {
				parts = append(parts, s)
			}
		}
		return strings.Join(parts, ", ")
	}
	return ""
}

func parseLength(s string) int {
	if s == "" {
		return 0
	}
	parts := strings.Split(s, ":")
	total := 0
	for _, p := range parts {
		n := 0
		fmt.Sscanf(p, "%d", &n)
		total = total*60 + n
	}
	return total
}

var audioFormats = map[string]bool{
	"VBR MP3": true, "MP3": true, "Ogg Vorbis": true, "FLAC": true,
	"128Kbps MP3": true, "64Kbps MP3": true, "256Kbps MP3": true, "320Kbps MP3": true,
}

func extOf(name string) string {
	i := strings.LastIndex(name, ".")
	if i < 0 {
		return ""
	}
	return strings.ToLower(name[i+1:])
}

func (a *ArchiveSource) Search(ctx context.Context, query string, limit int) ([]store.Track, error) {
	// Phrase-match the whole query against creator/title: a bare
	// multi-word Lucene query ORs the words and drowns in radio
	// airchecks and other spoken-word noise.
	phrase := `"` + strings.ReplaceAll(query, `"`, "") + `"`
	q := url.QueryEscape(fmt.Sprintf(`mediatype:audio AND (creator:%s OR title:%s)`, phrase, phrase))
	searchURL := fmt.Sprintf("https://archive.org/advancedsearch.php?q=%s&fl[]=identifier&fl[]=title&fl[]=creator&fl[]=year&rows=12&sort[]=downloads+desc&output=json", q)
	req, _ := http.NewRequestWithContext(ctx, "GET", searchURL, nil)
	req.Header.Set("User-Agent", "Beatlab/0.1")
	resp, err := a.http.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	var sr iaSearchResponse
	if err := json.NewDecoder(resp.Body).Decode(&sr); err != nil {
		return nil, err
	}

	// Fetch item metadata concurrently (order preserved): done
	// sequentially, a dozen archive.org round trips were most of
	// a search's latency.
	perDoc := make([][]store.Track, len(sr.Response.Docs))
	sem := make(chan struct{}, 6)
	var wg sync.WaitGroup
	for i, doc := range sr.Response.Docs {
		wg.Add(1)
		go func(i int, id string) {
			defer wg.Done()
			select {
			case sem <- struct{}{}:
				defer func() { <-sem }()
			case <-ctx.Done():
				return
			}
			if tracks, err := a.itemTracks(ctx, id); err == nil {
				perDoc[i] = tracks
			}
		}(i, doc.Identifier)
	}
	wg.Wait()

	var out []store.Track
	for _, tracks := range perDoc {
		out = append(out, tracks...)
		if len(out) >= limit {
			break
		}
	}
	if len(out) > limit {
		out = out[:limit]
	}
	return out, nil
}

// junkCollections are archive.org collections that are spoken
// word, not music — audiobooks, podcasts, old-time radio. Items
// from them never enter the catalog.
var junkCollections = []string{"librivox", "audiobook", "audio_book", "podcast", "old_time_radio", "radioprogram", "radio_show"}

func isJunkItem(md iaMetadata) bool {
	coll := strings.ToLower(creatorString(md.Metadata.Collection))
	for _, j := range junkCollections {
		if strings.Contains(coll, j) {
			return true
		}
	}
	subj := strings.ToLower(creatorString(md.Metadata.Subject))
	return strings.Contains(subj, "librivox") || strings.Contains(subj, "audiobook") || strings.Contains(subj, "audio book")
}

// pickCover chooses the item's real artwork: image files whose
// names look like cover art first, then the largest image — but
// never archive.org's auto-generated spectrogram/waveform renders,
// which masquerade as art for items that have none. Returns ""
// when the item has no real art (clients show their placeholder).
func pickCover(identifier string, md iaMetadata) string {
	type img struct {
		url   string
		score int
		size  int64
	}
	audioStems := map[string]bool{}
	for _, f := range md.Files {
		if audioFormats[f.Format] {
			audioStems[strings.ToLower(strings.TrimSuffix(f.Name, "."+extOf(f.Name)))] = true
		}
	}
	var best *img
	for _, f := range md.Files {
		e := extOf(f.Name)
		if e != "jpg" && e != "jpeg" && e != "png" {
			continue
		}
		low := strings.ToLower(f.Name)
		if strings.Contains(low, "spectrogram") || strings.Contains(low, "waveform") || strings.Contains(low, "__ia_thumb") {
			continue
		}
		score := 0
		for _, kw := range []string{"cover", "front", "folder", "album", "artwork"} {
			if strings.Contains(low, kw) {
				score += 10
				break
			}
		}
		if audioStems[strings.TrimSuffix(low, "."+e)] {
			score -= 5 // per-track image (photo/scan of one song), not album art
		}
		size := int64(0)
		fmt.Sscanf(f.Size, "%d", &size)
		cand := img{url: "https://archive.org/download/" + identifier + "/" + url.PathEscape(f.Name), score: score, size: size}
		if best == nil || cand.score > best.score || (cand.score == best.score && cand.size > best.size) {
			c := cand
			best = &c
		}
	}
	if best == nil {
		return ""
	}
	return best.url
}

// itemTracks expands one archive.org item into its audio tracks.
func (a *ArchiveSource) itemTracks(ctx context.Context, identifier string) ([]store.Track, error) {
	req, _ := http.NewRequestWithContext(ctx, "GET", "https://archive.org/metadata/"+identifier, nil)
	req.Header.Set("User-Agent", "Beatlab/0.1")
	resp, err := a.http.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	var md iaMetadata
	if err := json.NewDecoder(resp.Body).Decode(&md); err != nil {
		return nil, err
	}
	if isJunkItem(md) {
		return nil, nil
	}
	album := strings.TrimSpace(md.Metadata.Title)
	artist := creatorString(md.Metadata.Creator)
	if artist == "" {
		artist = "Unknown Artist"
	}
	year := 0
	if len(md.Metadata.Year) >= 4 {
		fmt.Sscanf(md.Metadata.Year[:4], "%d", &year)
	}
	genre := canonicalGenre(md.Metadata.Subject)

	cover := pickCover(identifier, md)

	var tracks []store.Track
	for _, f := range md.Files {
		ext := extOf(f.Name)
		if !audioFormats[f.Format] && ext != "mp3" && ext != "ogg" && ext != "flac" && ext != "m4a" {
			continue
		}
		dur := parseLength(f.Length)
		// Songs, not broadcasts: hour-long files are radio
		// airchecks, audiobooks and podcasts — drop them when the
		// duration is known.
		if dur > 1200 {
			continue
		}
		// skip derivative duplicates: keep the best single encoding per title
		title := strings.TrimSpace(f.Title)
		if title == "" {
			title = strings.TrimSuffix(f.Name, "."+ext)
		}
		trackArtist := artist
		if f.Artist != "" {
			trackArtist = strings.TrimSpace(f.Artist)
		} else if f.Creator != "" {
			trackArtist = strings.TrimSpace(f.Creator)
		}
		if trackArtist == "Unknown Artist" {
			// No attribution anywhere — these unattributed
			// uploads are what filled fresh catalogs with junk.
			continue
		}
		trackAlbum := album
		if f.Album != "" {
			trackAlbum = strings.TrimSpace(f.Album)
		}
		trackNo := 0
		fmt.Sscanf(f.Track, "%d", &trackNo)
		size := int64(0)
		fmt.Sscanf(f.Size, "%d", &size)
		id := "archive:" + identifier + ":" + f.Name
		tracks = append(tracks, store.Track{
			ID:        id,
			Source:    "archive",
			Title:     title,
			Artist:    trackArtist,
			Album:     trackAlbum,
			Genre:     genre,
			Year:      year,
			TrackNo:   trackNo,
			Duration:  parseLength(f.Length),
			Format:    ext,
			Size:      size,
			StreamURL: "https://archive.org/download/" + identifier + "/" + url.PathEscape(f.Name),
			CoverURL:  cover,
		})
	}
	// Dedupe same title+track (multiple encodings): keep the largest file.
	best := map[string]store.Track{}
	for _, t := range tracks {
		k := fmt.Sprintf("%d|%s", t.TrackNo, strings.ToLower(t.Title))
		if old, ok := best[k]; !ok || t.Size > old.Size {
			best[k] = t
		}
	}
	out := make([]store.Track, 0, len(best))
	for _, t := range best {
		out = append(out, t)
	}
	sort.Slice(out, func(i, j int) bool {
		if out[i].TrackNo != out[j].TrackNo {
			return out[i].TrackNo < out[j].TrackNo
		}
		return out[i].Title < out[j].Title
	})
	return out, nil
}
