// Package webui is Beatlab's embedded admin console: server
// status, source health, API-user management and a view into what
// the taste engine has learned. Server-rendered, no build step —
// it ships inside the single binary via embed.
package webui

import (
	"crypto/rand"
	_ "embed"
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

//go:embed favicon.png
var faviconPNG []byte

//go:embed favicon.svg
var faviconSVG []byte

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
		if r.URL.Path == "/favicon.ico" {
			w.Header().Set("Content-Type", "image/png")
			w.Header().Set("Cache-Control", "public, max-age=86400")
			w.Write(faviconPNG)
			return
		}
		if r.URL.Path == "/favicon.svg" {
			w.Header().Set("Content-Type", "image/svg+xml")
			w.Header().Set("Cache-Control", "public, max-age=86400")
			w.Write(faviconSVG)
			return
		}
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

const logoSVG = `<svg width="30" height="30" viewBox="0 0 64 64" aria-hidden="true"><rect x="2" y="2" width="60" height="60" rx="14" fill="#6c72ff"/><g fill="#fff"><rect x="13" y="23" width="6" height="18" rx="3"/><rect x="23" y="16" width="6" height="32" rx="3"/><rect x="33" y="20" width="6" height="24" rx="3"/><rect x="43" y="13" width="6" height="38" rx="3"/></g></svg>`

const css = `
:root{color-scheme:dark;--bg:#08080c;--bg2:#0d0d13;--panel:#101017;--panel2:#15151f;--line:#1e1e2a;--line2:#2a2a3a;
--text:#eceef4;--dim:#8e93a5;--faint:#5b6070;--accent:#6c72ff;--accent2:#9d6cff;--good:#3ecf8e;--bad:#ff7a7a;--radius:14px}
*{box-sizing:border-box}
body{margin:0;background:radial-gradient(1200px 500px at 70% -10%,#171430 0%,var(--bg) 55%);color:var(--text);
font:14px/1.55 "Inter",system-ui,-apple-system,"Segoe UI",sans-serif;-webkit-font-smoothing:antialiased}
a{color:var(--accent);text-decoration:none}a:hover{text-decoration:underline}
code{background:#0a0a10;border:1px solid var(--line);border-radius:6px;padding:1px 6px;font-size:12.5px}
header{position:sticky;top:0;z-index:5;backdrop-filter:blur(10px);background:rgba(8,8,12,.75);border-bottom:1px solid var(--line)}
.hwrap{max-width:1080px;margin:0 auto;padding:13px 24px;display:flex;align-items:center;gap:12px}
.brand{display:flex;align-items:center;gap:11px;font-weight:700;font-size:17px;letter-spacing:-.01em;color:var(--text)}
.brand small{font-weight:500;color:var(--dim);font-size:12px;border:1px solid var(--line2);padding:2px 8px;border-radius:99px}
.hspace{flex:1}
.hwrap .meta{color:var(--dim);font-size:13px}
main{max-width:1080px;margin:0 auto;padding:26px 24px 60px}
.stats{display:grid;grid-template-columns:repeat(auto-fit,minmax(180px,1fr));gap:14px;margin-bottom:14px}
.stat{background:linear-gradient(180deg,var(--panel2),var(--panel));border:1px solid var(--line);border-radius:var(--radius);padding:16px 18px}
.stat .k{font-size:11.5px;text-transform:uppercase;letter-spacing:.09em;color:var(--faint);font-weight:600}
.stat .v{font-size:27px;font-weight:700;letter-spacing:-.02em;margin:2px 0 1px}
.stat .s{color:var(--dim);font-size:12.5px}
.meter{height:6px;background:#0a0a10;border-radius:99px;overflow:hidden;margin-top:9px}
.meter i{display:block;height:100%;background:linear-gradient(90deg,var(--accent),var(--accent2));border-radius:99px}
.cols{display:grid;grid-template-columns:1fr 1fr;gap:14px;margin-bottom:14px}
@media(max-width:820px){.cols{grid-template-columns:1fr}}
.card{background:linear-gradient(180deg,var(--panel2),var(--panel));border:1px solid var(--line);border-radius:var(--radius);padding:20px}
.card h3{margin:0 0 4px;font-size:15px;font-weight:650;letter-spacing:-.01em;display:flex;align-items:center;gap:8px}
.card .sub{color:var(--dim);font-size:12.5px;margin:0 0 14px}
.src{display:flex;align-items:flex-start;gap:10px;padding:9px 0;border-bottom:1px solid var(--line)}
.src:last-child{border-bottom:0}
.dot{width:8px;height:8px;border-radius:99px;margin-top:6px;flex:none}
.dot.on{background:var(--good);box-shadow:0 0 8px rgba(62,207,142,.7)}
.dot.off{background:#3a3a48}
.src b{font-size:13.5px}.src span{display:block;color:var(--dim);font-size:12.5px}
.affinity{margin:7px 0}
.affinity .row{display:flex;justify-content:space-between;font-size:13px;margin-bottom:4px}
.affinity .row span:last-child{color:var(--dim);font-variant-numeric:tabular-nums}
.bar{height:7px;background:#0a0a10;border-radius:99px;overflow:hidden}
.bar i{display:block;height:100%;background:linear-gradient(90deg,var(--accent),var(--accent2));border-radius:99px}
.chips{display:flex;flex-wrap:wrap;gap:6px;margin-top:12px}
.chip{border:1px solid var(--line2);background:#0c0c13;color:#c6c9d6;border-radius:99px;padding:3px 10px;font-size:12px}
.empty{color:var(--dim);font-size:13px;border:1px dashed var(--line2);border-radius:10px;padding:14px;text-align:center}
.strip{display:grid;grid-template-columns:repeat(auto-fill,minmax(104px,1fr));gap:12px}
.album img,.album .ph{width:100%;aspect-ratio:1;object-fit:cover;border-radius:10px;border:1px solid var(--line);display:block}
.album .ph{background:linear-gradient(135deg,#1c1c2c,#12121e);display:flex;align-items:center;justify-content:center;font-size:26px;color:var(--accent);font-weight:700}
.album .t{font-size:12px;font-weight:600;margin-top:6px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.album .a{font-size:11.5px;color:var(--dim);white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
table{width:100%;border-collapse:collapse}
td,th{padding:9px 8px;border-bottom:1px solid var(--line);text-align:left;font-size:13.5px;vertical-align:middle}
tr:last-child td{border-bottom:0}
.avatar{width:30px;height:30px;border-radius:99px;background:linear-gradient(135deg,var(--accent),var(--accent2));color:#fff;
display:inline-flex;align-items:center;justify-content:center;font-weight:700;font-size:13px;margin-right:9px;vertical-align:-9px}
.badge{font-size:11px;color:#bdb2ff;border:1px solid #37306b;background:#1b1836;border-radius:99px;padding:1px 8px;margin-left:6px}
input,select{background:#0a0a10;border:1px solid var(--line2);color:var(--text);border-radius:9px;padding:8px 11px;font:inherit;font-size:13.5px}
input:focus,select:focus{outline:2px solid rgba(108,114,255,.45);border-color:var(--accent)}
button{background:linear-gradient(135deg,var(--accent),var(--accent2));color:#fff;border:0;border-radius:9px;padding:8px 15px;
cursor:pointer;font:inherit;font-size:13.5px;font-weight:600}
button:hover{filter:brightness(1.1)}
button.ghost{background:transparent;border:1px solid var(--line2);color:var(--text);font-weight:500}
button.danger:hover{border-color:var(--bad);color:var(--bad)}
.inline{display:inline-flex;gap:6px;align-items:center}
.addform{display:flex;flex-wrap:wrap;gap:8px;align-items:center;margin-top:14px;padding-top:14px;border-top:1px solid var(--line)}
.check{display:flex;gap:6px;align-items:center;color:var(--dim);font-size:13px}
.center{max-width:380px;margin:11vh auto;padding:0 16px}
.login-card{background:linear-gradient(180deg,var(--panel2),var(--panel));border:1px solid var(--line);border-radius:18px;padding:30px 28px;text-align:center}
.login-card h1{font-size:21px;margin:14px 0 4px;letter-spacing:-.01em}
.login-card p{color:var(--dim);font-size:13px;margin:0 0 18px}
.login-card input{width:100%;margin-bottom:10px;padding:11px 13px}
.login-card button{width:100%;padding:11px}
.err{color:var(--bad);font-size:13px;margin:0 0 10px}
.radio-mixes{display:flex;flex-direction:column;gap:8px;margin-bottom:14px}
.mix{display:flex;justify-content:space-between;border:1px solid var(--line);background:#0c0c13;border-radius:10px;padding:9px 13px;font-size:13.5px}
.mix span{color:var(--dim);font-size:12.5px}
footer{color:var(--faint);font-size:12px;text-align:center;padding:0 24px 34px}`

const headHTML = `<!doctype html><meta charset=utf-8><meta name=viewport content="width=device-width,initial-scale=1">
<link rel="icon" type="image/svg+xml" href="/favicon.svg"><link rel="icon" type="image/png" href="/favicon.ico">
<title>Beatlab</title><style>` + css + `</style>`

var loginTpl = template.Must(template.New("login").Parse(headHTML + `
<div class="center"><div class="login-card">
<div style="display:flex;justify-content:center">` + logoSVG + `</div>
<h1>Beatlab</h1><p>Your music, fetched and learned.<br>Sign in with the admin password.</p>
{{if .Err}}<p class=err>Wrong password — try again.</p>{{end}}
<form method=post><input type=password name=password placeholder="Admin password" autofocus>
<button>Sign in</button></form>
<p style="margin:16px 0 0;font-size:12.5px">Clients connect to <code>/rest</code> with an API user.<br>Navidrome-mode clients can also use <code>/auth/login</code>.</p>
</div></div>`))

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

type srcRow struct {
	Name, Status string
	On           bool
}
type affRow struct {
	Name  string
	Score float64
	Pct   float64
}
type albumCard struct{ Title, Artist, Cover, Initial string }
type mixRow struct {
	Name  string
	Songs int
}

type dashData struct {
	Tracks, Artists, Users, Plays int
	CacheHuman                    string
	CachePct                      float64
	CacheGB                       int
	Sources                       []srcRow
	Selected                      string
	UserNames                     []string
	Affinity                      []affRow
	Genres                        []string
	Recent                        []albumCard
	Mixes                         []mixRow
	Cold                          bool
}

var dashTpl = template.Must(template.New("dash").Funcs(template.FuncMap{
	"initial": func(s string) string {
		if s == "" {
			return "?"
		}
		return strings.ToUpper(s[:1])
	},
}).Parse(headHTML + `
<header><div class="hwrap">
<a class="brand" href="/admin">` + logoSVG + `Beatlab <small>admin</small></a>
<div class="hspace"></div>
<span class="meta">{{.Tracks}} tracks · {{.Plays}} plays learned</span>
<a href="/admin/logout" style="margin-left:16px">Sign out</a>
</div></header>
<main>
<div class="stats">
 <div class="stat"><div class="k">Catalog</div><div class="v">{{.Tracks}}</div><div class="s">tracks known</div></div>
 <div class="stat"><div class="k">Artists</div><div class="v">{{.Artists}}</div><div class="s">in the catalog</div></div>
 <div class="stat"><div class="k">Audio cache</div><div class="v">{{.CacheHuman}}</div><div class="s">of {{.CacheGB}} GB budget</div><div class="meter"><i style="width:{{printf "%.0f" .CachePct}}%"></i></div></div>
 <div class="stat"><div class="k">API users</div><div class="v">{{.Users}}</div><div class="s">Subsonic sign-ins</div></div>
 <div class="stat"><div class="k">Learned plays</div><div class="v">{{.Plays}}</div><div class="s">completed listens folded into taste</div></div>
</div>

<div class="cols">
 <div class="card"><h3>Sources</h3><p class="sub">Where music is fetched from, on demand.</p>
 {{range .Sources}}<div class="src"><span class="dot {{if .On}}on{{else}}off{{end}}"></span><div><b>{{.Name}}</b><span>{{.Status}}</span></div></div>{{end}}
 </div>
 <div class="card"><h3>Radio mixes</h3><p class="sub">Auto-playlists in every client, rebuilt from the taste profile every 6 hours.</p>
 <div class="radio-mixes">{{range .Mixes}}<div class="mix"><b>{{.Name}}</b><span>{{.Songs}} songs</span></div>{{end}}</div>
 <form method="post" action="/admin/radio"><input type="hidden" name="user" value="{{.Selected}}"><button type="submit">↻ Refresh mixes now</button></form>
 </div>
</div>

<div class="card" style="margin-bottom:14px"><h3>What {{.Selected}} loves</h3>
<p class="sub">Affinity learned from plays, skips, stars and ratings (90-day decay).{{if .Cold}} Nothing yet — fresh profiles start from the trending pool.{{end}}</p>
{{if .Affinity}}{{range .Affinity}}<div class="affinity"><div class="row"><span>{{.Name}}</span><span>{{printf "%.1f" .Score}}</span></div><div class="bar"><i style="width:{{printf "%.0f" .Pct}}%"></i></div></div>{{end}}
{{if .Genres}}<div class="chips">{{range .Genres}}<span class="chip">{{.}}</span>{{end}}</div>{{end}}
{{else}}<div class="empty">Play some music from a client and Beatlab starts learning this listener.</div>{{end}}
<form method="get" action="/admin" style="margin-top:14px" class="inline"><span style="color:var(--dim)">Listener:</span>
<select name="user">{{range .UserNames}}<option {{if eq . $.Selected}}selected{{end}}>{{.}}</option>{{end}}</select>
<button class="ghost" type="submit">View</button></form>
</div>

{{if .Recent}}<div class="card" style="margin-bottom:14px"><h3>Recently catalogued</h3><p class="sub">The newest arrivals from the sources.</p>
<div class="strip">{{range .Recent}}<div class="album">{{if .Cover}}<img src="{{.Cover}}" loading="lazy" alt="">{{else}}<div class="ph">{{.Initial}}</div>{{end}}<div class="t">{{.Title}}</div><div class="a">{{.Artist}}</div></div>{{end}}</div></div>{{end}}

<div class="card"><h3>API users</h3><p class="sub">Clients (Feishin, Symfonium, play:Sub, any Navidrome/Subsonic app) sign in with these. Point them at this server — Subsonic mode uses <code>/rest</code>, Navidrome mode uses <code>/auth/login</code>.</p>
<table>{{range .UserRows}}<tr><td><span class="avatar">{{initial .Username}}</span>{{.Username}}{{if .IsAdmin}}<span class="badge">admin</span>{{end}}</td>
<td style="text-align:right;white-space:nowrap"><form method="post" action="/admin/users" class="inline">
<input type="hidden" name="action" value="password"><input type="hidden" name="id" value="{{.ID}}">
<input type="password" name="password" placeholder="new password" size="13"> <button class="ghost" type="submit">Set</button></form>
<form method="post" action="/admin/users" class="inline" onsubmit="return confirm('Delete this user?')">
<input type="hidden" name="action" value="delete"><input type="hidden" name="id" value="{{.ID}}"><button class="ghost danger" type="submit">Delete</button></form></td></tr>{{end}}</table>
<form method="post" action="/admin/users" class="addform"><input type="hidden" name="action" value="create">
<input name="username" placeholder="username" required> <input name="password" placeholder="password" required>
<label class="check"><input type="checkbox" name="admin"> admin</label> <button type="submit">Add user</button></form>
</div>
</main>
<footer>Beatlab · single binary · Subsonic / OpenSubsonic API on <code>/rest</code></footer>`))

func (u *UI) dashboard(w http.ResponseWriter, r *http.Request) {
	users := u.store.Users()
	if len(users) == 0 {
		http.Redirect(w, r, "/admin/login", http.StatusFound)
		return
	}
	selected := users[0]
	if name := r.URL.Query().Get("user"); name != "" {
		if usr, ok := u.store.UserByName(name); ok {
			selected = *usr
		}
	}
	profile := u.taste.BuildProfile(selected.ID)

	d := dashData{
		Tracks:   u.store.TrackCount(),
		Artists:  len(u.store.Artists()),
		Users:    len(users),
		Plays:    profile.Plays,
		CacheGB:  u.cacheGB,
		Selected: selected.Username,
		Cold:     profile.Cold(),
	}
	d.CacheHuman = humanBytes(u.store.CacheBytes())
	if u.cacheGB > 0 {
		d.CachePct = float64(u.store.CacheBytes()) / float64(int64(u.cacheGB)<<30) * 100
		if d.CachePct > 100 {
			d.CachePct = 100
		}
	}
	for _, s := range u.registry.All() {
		d.Sources = append(d.Sources, srcRow{Name: s.Name(), Status: s.Status(), On: s.Available()})
	}
	for _, usr := range users {
		d.UserNames = append(d.UserNames, usr.Username)
	}
	maxA := 0.0
	for _, v := range profile.ArtistScore {
		if v > maxA {
			maxA = v
		}
	}
	for _, a := range profile.TopArtists(8) {
		pct := 0.0
		if maxA > 0 {
			pct = profile.ArtistScore[a] / maxA * 100
		}
		d.Affinity = append(d.Affinity, affRow{Name: a, Score: profile.ArtistScore[a], Pct: pct})
	}
	d.Genres = profile.TopGenres(6)

	// Recently catalogued: newest album per cover card.
	recent, _ := u.store.QueryTracks(store.Query{Order: "created DESC", Limit: 60})
	seenAlbum := map[string]bool{}
	for _, t := range recent {
		key := t.AlbumID()
		if seenAlbum[key] || len(d.Recent) >= 12 {
			continue
		}
		seenAlbum[key] = true
		initial := "♪"
		if t.Album != "" {
			initial = strings.ToUpper(t.Album[:1])
		}
		d.Recent = append(d.Recent, albumCard{Title: t.Album, Artist: t.Artist, Cover: t.CoverURL, Initial: initial})
	}

	for _, p := range u.store.Playlists(selected.ID) {
		if p.Kind == "radio" {
			d.Mixes = append(d.Mixes, mixRow{Name: strings.TrimPrefix(p.Name, "Beatlab Radio — "), Songs: len(p.SongIDs)})
		}
	}

	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	data := struct {
		dashData
		UserRows []store.User
	}{d, users}
	dashTpl.Execute(w, data)
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
