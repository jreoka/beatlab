// Beatlab — Bitlab for music. A single-binary, self-hosted music
// streaming server speaking the Subsonic/OpenSubsonic API (so
// Navidrome clients just work), fetching music on demand from
// multiple sources, caching it locally, and learning each
// listener's taste to power recommendations and radio auto-mixes.
package main

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"log"
	"net/http"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"time"

	"beatlab/internal/engine"
	"beatlab/internal/sources"
	"beatlab/internal/store"
	"beatlab/internal/subsonic"
	"beatlab/internal/taste"
	"beatlab/internal/webui"
)

func env(key, def string) string {
	if v := os.Getenv(key); v != "" {
		return v
	}
	return def
}

func adminPassword(dataDir string) string {
	if v := os.Getenv("BEATLAB_ADMIN_PASSWORD"); v != "" {
		return v
	}
	path := filepath.Join(dataDir, "admin-password.txt")
	if raw, err := os.ReadFile(path); err == nil {
		if p := strings.TrimSpace(string(raw)); p != "" {
			return p
		}
	}
	buf := make([]byte, 9)
	rand.Read(buf)
	p := hex.EncodeToString(buf)
	os.MkdirAll(dataDir, 0o755)
	if err := os.WriteFile(path, []byte(p+"\n"), 0o600); err == nil {
		log.Printf("[beatlab] generated admin password, saved to %s", path)
	}
	return p
}

// version identifies the build in the startup log and /healthz,
// so it's always obvious which binary is actually running.
const version = "0.1.0-radiofix5"

// statusWriter captures the response status for request logging.
type statusWriter struct {
	http.ResponseWriter
	status int
}

func (w *statusWriter) WriteHeader(code int) {
	w.status = code
	w.ResponseWriter.WriteHeader(code)
}

// logRequests logs one line per API request (method, path,
// status, duration) when it completes — streams included, so
// playback problems are diagnosable from the log too. The
// point: if a client spins, the log shows whether the request
// arrived and whether it ever finished.
func logRequests(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		p := r.URL.Path
		start := time.Now()
		sw := &statusWriter{ResponseWriter: w, status: 200}
		next.ServeHTTP(sw, r)
		log.Printf("[beatlab] %s %s -> %d in %s", r.Method, p, sw.status, time.Since(start).Round(time.Millisecond))
	})
}

func main() {
	addr := env("BEATLAB_ADDR", ":4533")
	dataDir := env("BEATLAB_DATA", "./beatlab-data")
	cacheGB, _ := strconv.Atoi(env("BEATLAB_CACHE_GB", "10"))
	if cacheGB <= 0 {
		cacheGB = 10
	}

	st, err := store.Open(dataDir)
	if err != nil {
		log.Fatalf("[beatlab] open store: %v", err)
	}
	defer st.Close()

	password := adminPassword(dataDir)

	// Seed the first API user so clients can connect immediately:
	// username BEATLAB_USER (default "admin"), password = the admin
	// password. More users are managed in the console.
	if st.UserCount() == 0 {
		username := env("BEATLAB_USER", "admin")
		if _, err := st.CreateUser(username, password, true); err != nil {
			log.Fatalf("[beatlab] seed user: %v", err)
		}
		log.Printf("[beatlab] seeded API user %q (password = admin password)", username)
	}

	registry := sources.NewRegistry(
		sources.NewArchive(),
		sources.NewYtDlp(env("BEATLAB_YTDLP", "")),
		sources.NewTorrent(),
	)
	// Sources that miss a search's soft deadline keep running in
	// the background; their tracks land here so the catalog keeps
	// warming even after the client got its answer.
	registry.Sink = func(t store.Track) {
		tt := t
		st.UpsertTrack(&tt)
	}
	eng := engine.New(st, registry, dataDir, cacheGB)
	te := taste.New(st, registry)
	api := subsonic.New(st, registry, eng, te)
	ui := webui.New(st, registry, te, password, cacheGB)
	ui.SetRadioRefresher(api)

	// Seed a fresh catalog with popular music in the background so
	// a new account opens onto trending tracks, not an empty shelf.
	go te.Warm(context.Background())

	mux := http.NewServeMux()
	mux.Handle("/rest/", api.Handler())
	mux.HandleFunc("/auth/login", api.NDLoginHandler())
	mux.Handle("/api/", api.NDHandler())
	mux.HandleFunc("/healthz", func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		w.Write([]byte(`{"ok":true,"version":"` + version + `"}`))
	})
	mux.Handle("/", ui.Handler())

	log.Printf("[beatlab] beatlab %s", version)
	log.Printf("[beatlab] data dir: %s", dataDir)
	log.Printf("[beatlab] sources:")
	for _, s := range registry.All() {
		log.Printf("[beatlab]   - %s: %s", s.Name(), s.Status())
	}
	log.Printf("[beatlab] admin console: http://localhost%s/admin", addr)
	log.Printf("[beatlab] Subsonic API:  http://localhost%s/rest (user: %s)", addr, env("BEATLAB_USER", "admin"))
	log.Printf("[beatlab] listening on %s", addr)
	srv := &http.Server{
		Addr:              addr,
		Handler:           logRequests(mux),
		ReadHeaderTimeout: 10 * time.Second,
	}
	if err := srv.ListenAndServe(); err != nil {
		log.Fatal(err)
	}
}
