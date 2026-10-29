package sources

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/url"
	"sort"
	"strings"
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

func (a *ArchiveSource) Name() string     { return "archive" }
func (a *ArchiveSource) Available() bool  { return true }
func (a *ArchiveSource) Status() string   { return "archive.org — always on, no key needed" }

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
		Title   string `json:"title"`
		Creator any    `json:"creator"` // string or []string
		Year    string `json:"date"`
		Subject any    `json:"subject"`
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

	var out []store.Track
	for _, doc := range sr.Response.Docs {
		tracks, err := a.itemTracks(ctx, doc.Identifier)
		if err != nil {
			continue
		}
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
	album := strings.TrimSpace(md.Metadata.Title)
	artist := creatorString(md.Metadata.Creator)
	if artist == "" {
		artist = "Unknown Artist"
	}
	year := 0
	if len(md.Metadata.Year) >= 4 {
		fmt.Sscanf(md.Metadata.Year[:4], "%d", &year)
	}
	genre := ""
	if subs, ok := md.Metadata.Subject.([]any); ok && len(subs) > 0 {
		if s, ok := subs[0].(string); ok {
			genre = s
		}
	} else if s, ok := md.Metadata.Subject.(string); ok {
		genre = s
	}

	// cover: prefer an image file in the item, else the item image service
	cover := "https://archive.org/services/img/" + identifier
	for _, f := range md.Files {
		e := extOf(f.Name)
		if e == "jpg" || e == "jpeg" || e == "png" {
			cover = "https://archive.org/download/" + identifier + "/" + url.PathEscape(f.Name)
			break
		}
	}

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
			trackArtist = f.Artist
		} else if f.Creator != "" {
			trackArtist = f.Creator
		}
		trackAlbum := album
		if f.Album != "" {
			trackAlbum = f.Album
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
