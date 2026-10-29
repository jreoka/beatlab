// Package webui is Beatlab's embedded admin console: server
// status, source health, API-user management and a view into what
// the taste engine has learned. Server-rendered, no build step —
// it ships inside the single binary via embed.
package webui

import (
	"crypto/rand"
	"encoding/hex"
	"fmt"
	"html/template"
	"net/http"
	"strings"
	"sync"

	"beatlab/internal/sources"
	"beatlab/internal/store"
	"beatlab/internal/taste"
)

type UI struct {
	store    *store.Store
	registry *sources.Registry
	taste    *taste.Engine
	radio    interface {
		RefreshRadioFor(r *http.Request, user *store.User)
	}
	adminPassword string
	cacheGB       int

	mu       sync.Mutex
	sessions map[string]bool
}

func New(st *store.Store, reg *sources.Registry, te *taste.Engine, adminPassword string, cacheGB int) *UI {
	return &UI{store: st, registry: reg, taste: te, adminPassword: adminPassword,
		cacheGB: cacheGB, sessions: map[string]bool{}}
}

func (u *UI) SetRadioRefresher(r interface {
	RefreshRadioFor(r *http.Request, user *store.User)
}) {
	u.radio = r
}

func (u *UI) Handler() http.Handler {
	mux := http.NewServeMux()
	mux.HandleFunc("/", func(w http.ResponseWriter, r *http.Request) {
		http.Redirect(w, r, "/admin", http.StatusFound)
	})
	mux.HandleFunc("/admin/login", u.login)
	mux.HandleFunc("/admin/logout", u.logout)
	mux.HandleFunc("/admin", u.requireAuth(u.dashboard))
	mux.HandleFunc("/admin/users", u.requireAuth(u.users))
	mux.HandleFunc("/admin/radio", u.requireAuth(u.radioRefresh))
	return mux
}

func (u *UI) authed(r *http.Request) bool {
	c, err := r.Cookie("beatlab_admin")
	if err != nil {
		return false
	}
	u.mu.Lock()
	defer u.mu.Unlock()
	return u.sessions[c.Value]
}

func (u *UI) requireAuth(h http.HandlerFunc) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		if !u.authed(r) {
			http.Redirect(w, r, "/admin/login", http.StatusFound)
			return
		}
		h(w, r)
	}
}

const css = `
:root{color-scheme:dark;--bg:#0a0a0b;--panel:#121214;--line:#232326;--text:#e8e8ea;--dim:#9a9aa2;--accent:#6c72ff}
*{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--text);font:14px/1.5 system-ui,sans-serif}
a{color:var(--accent)}header{display:flex;justify-content:space-between;align-items:center;padding:14px 22px;border-bottom:1px solid var(--line)}
header b{font-size:17px}header span{color:var(--dim)}
main{max-width:960px;margin:0 auto;padding:22px}
.grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(200px,1fr));gap:14px;margin-bottom:14px}
.card{background:var(--panel);border:1px solid var(--line);border-radius:8px;padding:16px}
.card h3{margin:0 0 10px;font-size:13px;text-transform:uppercase;letter-spacing:.06em;color:var(--dim)}
.big{font-size:26px;font-weight:650}
table{width:100%;border-collapse:collapse}td,th{padding:6px 8px;border-bottom:1px solid var(--line);text-align:left}
input,select{background:#0d0d0f;border:1px solid var(--line);color:var(--text);border-radius:6px;padding:7px 9px}
button{background:var(--accent);color:#fff;border:0;border-radius:6px;padding:7px 14px;cursor:pointer}
button.ghost{background:transparent;border:1px solid var(--line);color:var(--text)}
.bar{height:8px;background:#1d1d20;border-radius:4px;overflow:hidden}.bar i{display:block;height:100%;background:var(--accent)}
.row{display:flex;justify-content:space-between;gap:10px;align-items:center;margin:5px 0}
.center{max-width:340px;margin:12vh auto}
.err{color:#ff7a7a}`

var loginTpl = template.Must(template.New("login").Parse(`<!doctype html><meta charset=utf-8>
<title>Beatlab — sign in</title><style>` + css + `</style>
<div class="center card"><h2>🎵 Beatlab</h2>
{{if .Err}}<p class=err>Wrong password.</p>{{end}}
<form method=post><p><input type=password name=password placeholder="Admin password" style="width:100%"></p>
<p><button style="width:100%">Sign in</button></p></form>
<p style="color:var(--dim)">Clients connect with a Subsonic app to <code>/rest</code> using an API user below.</p></div>`))

func (u *UI) login(w http.ResponseWriter, r *http.Request) {
	if r.Method == http.MethodPost {
		if r.FormValue("password") == u.adminPassword {
			tok := make([]byte, 16)
			rand.Read(tok)
			s := hex.EncodeToString(tok)
			u.mu.Lock()
			u.sessions[s] = true
			u.mu.Unlock()
			http.SetCookie(w, &http.Cookie{Name: "beatlab_admin", Value: s, Path: "/", HttpOnly: true})
			http.Redirect(w, r, "/admin", http.StatusFound)
			return
		}
		loginTpl.Execute(w, map[string]any{"Err": true})
		return
	}
	loginTpl.Execute(w, map[string]any{})
}

func (u *UI) logout(w http.ResponseWriter, r *http.Request) {
	if c, err := r.Cookie("beatlab_admin"); err == nil {
		u.mu.Lock()
		delete(u.sessions, c.Value)
		u.mu.Unlock()
	}
	http.Redirect(w, r, "/admin/login", http.StatusFound)
}

func humanBytes(n int64) string {
	const u = 1024
	if n < u {
		return fmt.Sprintf("%d B", n)
	}
	div, exp := int64(u), 0
	for x := n / u; x >= u; x /= u {
		div *= u
		exp++
	}
	return fmt.Sprintf("%.1f %cB", float64(n)/float64(div), "KMGT"[exp])
}

func (u *UI) dashboard(w http.ResponseWriter, r *http.Request) {
	users := u.store.Users()
	selected := users[0]
	if name := r.URL.Query().Get("user"); name != "" {
		if usr, ok := u.store.UserByName(name); ok {
			selected = *usr
		}
	}
	profile := u.taste.BuildProfile(selected.ID)

	var srcRows strings.Builder
	for _, s := range u.registry.All() {
		mark := "✅"
		if !s.Available() {
			mark = "⚪"
		}
		fmt.Fprintf(&srcRows, "<tr><td>%s %s</td><td>%s</td></tr>", mark, template.HTMLEscapeString(s.Name()), template.HTMLEscapeString(s.Status()))
	}
	var userRows strings.Builder
	for _, usr := range users {
		admin := ""
		if usr.IsAdmin {
			admin = " · admin"
		}
		fmt.Fprintf(&userRows, `<tr><td>%s%s</td><td><form method=post action=/admin/users style="display:inline">
<input type=hidden name=action value=password><input type=hidden name=id value="%d">
<input type=password name=password placeholder="new password" size=14> <button class=ghost>Set</button></form>
<form method=post action=/admin/users style="display:inline" onsubmit="return confirm('Delete user?')">
<input type=hidden name=action value=delete><input type=hidden name=id value="%d"><button class=ghost>Delete</button></form></td></tr>`,
			template.HTMLEscapeString(usr.Username), admin, usr.ID, usr.ID)
	}
	bar := func(label string, score, max float64) string {
		pct := 0.0
		if max > 0 {
			pct = score / max * 100
		}
		return fmt.Sprintf(`<div class=row><span style="flex:0 0 40%%">%s</span><span class=bar style="flex:1"><i style="width:%.0f%%"></i></span><span>%.1f</span></div>`,
			template.HTMLEscapeString(label), pct, score)
	}
	var tasteRows strings.Builder
	maxA := 0.0
	for _, v := range profile.ArtistScore {
		if v > maxA {
			maxA = v
		}
	}
	for _, a := range profile.TopArtists(8) {
		tasteRows.WriteString(bar(a, profile.ArtistScore[a], maxA))
	}
	if len(profile.TopArtists(1)) == 0 {
		tasteRows.WriteString(`<p style="color:var(--dim)">Nothing yet — play some music from a Subsonic client and Beatlab starts learning.</p>`)
	}
	var genreLine strings.Builder
	for _, g := range profile.TopGenres(6) {
		fmt.Fprintf(&genreLine, "%s · ", template.HTMLEscapeString(g))
	}

	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	fmt.Fprintf(w, `<!doctype html><meta charset=utf-8><title>Beatlab</title><style>%s</style>
<header><b>🎵 Beatlab</b><span>%d tracks · %d plays learned · <a href=/admin/logout>sign out</a></span></header>
<main>
<div class=grid>
 <div class=card><h3>Catalog</h3><div class=big>%d</div><div style="color:var(--dim)">tracks known</div></div>
 <div class=card><h3>Audio cache</h3><div class=big>%s</div><div style="color:var(--dim)">of %d GB budget</div></div>
 <div class=card><h3>API users</h3><div class=big>%d</div><div style="color:var(--dim)">Subsonic clients sign in as these</div></div>
 <div class=card><h3>Radio</h3><form method=post action=/admin/radio><input type=hidden name=user value="%s"><button>↻ Refresh mixes</button></form>
 <div style="color:var(--dim);margin-top:6px">Personal · Discovery · Favorites playlists rebuild from the taste profile</div></div>
</div>
<div class=grid>
 <div class=card><h3>Sources</h3><table>%s</table></div>
 <div class=card><h3>What %s loves</h3>%s<p style="color:var(--dim)">Genres: %s</p>
 <form method=get action=/admin>User: <select name=user>%s</select> <button class=ghost>View</button></form></div>
</div>
<div class=card><h3>API users</h3><table>%s</table>
<form method=post action=/admin/users style="margin-top:10px"><input type=hidden name=action value=create>
<input name=username placeholder="username" required> <input name=password placeholder="password" required>
<label><input type=checkbox name=admin> admin</label> <button>Add user</button></form>
<p style="color:var(--dim)">Connect a Subsonic/OpenSubsonic client (Feishin, Symfonium, play:Sub…) to this server's <code>/rest</code> with a user's name + password.</p></div>
</main>`,
		css, u.store.TrackCount(), profile.Plays,
		u.store.TrackCount(), humanBytes(u.store.CacheBytes()), u.cacheGB,
		len(users), template.HTMLEscapeString(selected.Username),
		srcRows.String(),
		template.HTMLEscapeString(selected.Username), tasteRows.String(), genreLine.String(),
		userOptions(users, selected.Username),
		userRows.String())
}

func userOptions(users []store.User, sel string) string {
	var b strings.Builder
	for _, usr := range users {
		s := ""
		if usr.Username == sel {
			s = " selected"
		}
		fmt.Fprintf(&b, `<option%s>%s</option>`, s, template.HTMLEscapeString(usr.Username))
	}
	return b.String()
}

func (u *UI) users(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Redirect(w, r, "/admin", http.StatusFound)
		return
	}
	switch r.FormValue("action") {
	case "create":
		username := strings.TrimSpace(r.FormValue("username"))
		password := r.FormValue("password")
		if username != "" && password != "" {
			u.store.CreateUser(username, password, r.FormValue("admin") != "")
		}
	case "delete":
		var id int64
		fmt.Sscanf(r.FormValue("id"), "%d", &id)
		if id != 0 && u.store.UserCount() > 1 {
			u.store.DeleteUser(id)
		}
	case "password":
		var id int64
		fmt.Sscanf(r.FormValue("id"), "%d", &id)
		if id != 0 && r.FormValue("password") != "" {
			u.store.SetPassword(id, r.FormValue("password"))
		}
	}
	http.Redirect(w, r, "/admin", http.StatusFound)
}

func (u *UI) radioRefresh(w http.ResponseWriter, r *http.Request) {
	if r.Method == http.MethodPost && u.radio != nil {
		if usr, ok := u.store.UserByName(r.FormValue("user")); ok {
			u.radio.RefreshRadioFor(r, usr)
		}
	}
	http.Redirect(w, r, "/admin", http.StatusFound)
}
