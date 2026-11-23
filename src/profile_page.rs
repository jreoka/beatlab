//! Public, sign-in-free profile pages served at `/u/{username}` and its tabs:
//!
//! | URL                         | Content                                         |
//! |-----------------------------|-------------------------------------------------|
//! | `/u/{user}`                 | Overview: highlights, top tracks, playlists     |
//! | `/u/{user}/stats?range=30d` | Listening stats for 7d / 30d / 90d / 365d / all |
//! | `/u/{user}/playlists`       | All public playlists                            |
//! | `/u/{user}/followers`       | Followers who also have a public page           |
//! | `/u/{user}/following`       | Followed users who also have a public page      |
//!
//! Only information already visible to every signed-in user on the in-app
//! profile is rendered, reduced to privacy-safe aggregates (see
//! `stats::PublicStats` / `stats::PublicRangeStats`). Every page carries Open
//! Graph / Twitter meta so links unfurl nicely in Discord, iMessage, Slack, etc.

use crate::models::{Playlist, PublicUser};
use crate::share_page::html_escape;
use crate::stats::{PublicRangeStats, PublicShare, PublicStats};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileTab {
    Overview,
    Stats,
    Playlists,
    Followers,
    Following,
}

impl ProfileTab {
    pub const ALL: [ProfileTab; 5] = [
        ProfileTab::Overview,
        ProfileTab::Stats,
        ProfileTab::Playlists,
        ProfileTab::Followers,
        ProfileTab::Following,
    ];

    /// `None` (no sub-path) is the overview; an unknown slug is `None`.
    pub fn from_slug(slug: Option<&str>) -> Option<Self> {
        match slug.map(|s| s.trim().trim_end_matches('/').to_ascii_lowercase()) {
            None => Some(Self::Overview),
            Some(s) => match s.as_str() {
                "" | "overview" => Some(Self::Overview),
                "stats" => Some(Self::Stats),
                "playlists" => Some(Self::Playlists),
                "followers" => Some(Self::Followers),
                "following" => Some(Self::Following),
                _ => None,
            },
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            Self::Overview => "",
            Self::Stats => "stats",
            Self::Playlists => "playlists",
            Self::Followers => "followers",
            Self::Following => "following",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Stats => "Stats",
            Self::Playlists => "Playlists",
            Self::Followers => "Followers",
            Self::Following => "Following",
        }
    }
}

/// Tab-specific data for a page.
pub enum ProfileBody<'a> {
    Overview {
        stats: &'a PublicStats,
        playlists: &'a [Playlist],
    },
    Stats(&'a PublicRangeStats),
    Playlists(&'a [Playlist]),
    /// Visible users plus the number left out because their page is off.
    Followers(&'a [PublicUser], usize),
    Following(&'a [PublicUser], usize),
}

impl ProfileBody<'_> {
    fn tab(&self) -> ProfileTab {
        match self {
            Self::Overview { .. } => ProfileTab::Overview,
            Self::Stats(_) => ProfileTab::Stats,
            Self::Playlists(_) => ProfileTab::Playlists,
            Self::Followers(..) => ProfileTab::Followers,
            Self::Following(..) => ProfileTab::Following,
        }
    }
}

const STATS_RANGES: [(&str, &str); 5] = [
    ("7d", "7 days"),
    ("30d", "30 days"),
    ("90d", "90 days"),
    ("365d", "12 months"),
    ("all", "All time"),
];

fn safe_color(raw: &str) -> &str {
    let s = raw.trim();
    let ok = s.len() >= 4
        && s.len() <= 9
        && s.starts_with('#')
        && s[1..].chars().all(|c| c.is_ascii_hexdigit());
    if ok {
        s
    } else {
        "#1ed760"
    }
}

fn fmt_month_year(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|d| d.format("%B %Y").to_string())
        .unwrap_or_default()
}

fn fmt_listening(secs: i64) -> String {
    let hrs = secs / 3600;
    if hrs >= 1 {
        format!("{} hr", fmt_count(hrs))
    } else if secs > 0 {
        format!("{} min", (secs / 60).max(1))
    } else {
        "0 min".to_string()
    }
}

fn fmt_count(n: i64) -> String {
    let digits = n.abs().to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    if n < 0 {
        format!("-{out}")
    } else {
        out
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{} {}", fmt_count(n as i64), if n == 1 { one } else { many })
}

fn display_name(u: &PublicUser) -> &str {
    if u.display_name.trim().is_empty() {
        &u.username
    } else {
        u.display_name.trim()
    }
}

fn avatar_inner(u: &PublicUser) -> String {
    if !u.avatar_url.trim().is_empty() {
        return format!(r#"<img src="{}" alt="" loading="lazy" />"#, html_escape(&u.avatar_url));
    }
    let letter = if u.avatar_letter.trim().is_empty() {
        display_name(u)
            .chars()
            .next()
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_default()
    } else {
        u.avatar_letter.clone()
    };
    html_escape(&letter)
}

/// Relative URL of a public profile tab, e.g. `/u/emily/stats?range=30d`.
pub fn profile_path(username: &str, tab: ProfileTab, range: Option<&str>) -> String {
    let mut p = format!("/u/{}", urlencoding::encode(username));
    if tab != ProfileTab::Overview {
        p.push('/');
        p.push_str(tab.slug());
    }
    if let Some(r) = range.filter(|r| tab == ProfileTab::Stats && *r != "all") {
        p.push_str("?range=");
        p.push_str(r);
    }
    p
}

/// Title + description used for the overview link preview.
pub fn build_profile_meta(u: &PublicUser, stats: &PublicStats) -> (String, String) {
    let title = format!("{} (@{}) on Beatlab", display_name(u), u.username);
    let mut parts: Vec<String> = Vec::new();
    if !u.bio.trim().is_empty() {
        parts.push(u.bio.trim().to_string());
    }
    let mut facts: Vec<String> = vec![plural(u.followers_count.max(0) as usize, "follower", "followers")];
    if stats.total_streams > 0 {
        facts.push(format!("{} streams", fmt_count(stats.total_streams as i64)));
    }
    if let Some(a) = stats.top_artist.as_deref().filter(|a| !a.trim().is_empty()) {
        facts.push(format!("Top artist: {a}"));
    }
    parts.push(facts.join(" · "));
    (title, parts.join("\n"))
}

fn build_meta(u: &PublicUser, body: &ProfileBody) -> (String, String) {
    let name = display_name(u);
    match body {
        ProfileBody::Overview { stats, .. } => build_profile_meta(u, stats),
        ProfileBody::Stats(s) => {
            let title = format!("{name}'s listening stats (@{}) · Beatlab", u.username);
            let mut facts = vec![
                s.range_label.to_string(),
                format!("{} streams", fmt_count(s.total_streams as i64)),
                fmt_listening(s.total_listening_secs),
            ];
            if let Some(a) = s.top_artists.first() {
                facts.push(format!("Top artist: {}", a.name));
            }
            (title, facts.join(" · "))
        }
        ProfileBody::Playlists(p) => (
            format!("{name}'s playlists (@{}) · Beatlab", u.username),
            format!("{} on Beatlab", plural(p.len(), "public playlist", "public playlists")),
        ),
        ProfileBody::Followers(..) => (
            format!("People following {name} (@{}) · Beatlab", u.username),
            plural(u.followers_count.max(0) as usize, "follower", "followers"),
        ),
        ProfileBody::Following(..) => (
            format!("People {name} follows (@{}) · Beatlab", u.username),
            format!("Following {}", plural(u.following_count.max(0) as usize, "listener", "listeners")),
        ),
    }
}

// ---------------------------------------------------------------------------
// Tab bodies
// ---------------------------------------------------------------------------

fn section(title: &str, more: Option<(&str, &str)>, inner: &str) -> String {
    let more_html = more
        .map(|(href, label)| format!(r#"<a class="more" href="{}">{}</a>"#, html_escape(href), html_escape(label)))
        .unwrap_or_default();
    format!(r#"<section><div class="sec-head"><h2>{}</h2>{}</div>{}</section>"#, html_escape(title), more_html, inner)
}

fn tiles(items: &[(String, String)]) -> String {
    if items.is_empty() {
        return String::new();
    }
    format!(
        r#"<div class="hl-grid">{}</div>"#,
        items
            .iter()
            .map(|(k, v)| format!(
                r#"<div class="hl"><span>{}</span><strong>{}</strong></div>"#,
                html_escape(k),
                html_escape(v)
            ))
            .collect::<String>()
    )
}

fn track_list(tracks: &[crate::stats::PublicTopTrack]) -> String {
    format!(
        r#"<ol class="list">{}</ol>"#,
        tracks
            .iter()
            .map(|t| format!(
                r#"<li><div class="li-main"><div class="li-title">{}</div><div class="li-sub">{}</div></div><div class="li-meta">{}</div></li>"#,
                html_escape(&t.title),
                html_escape(&t.artist),
                plural(t.streams, "play", "plays")
            ))
            .collect::<String>()
    )
}

fn bar_list(items: &[PublicShare]) -> String {
    let max = items.iter().map(|i| i.pct).fold(0.0_f64, f64::max).max(0.1);
    format!(
        r#"<ol class="list bars">{}</ol>"#,
        items
            .iter()
            .map(|i| format!(
                r#"<li><div class="li-main"><div class="li-title">{}</div><div class="bar"><i style="width:{:.1}%"></i></div></div><div class="li-meta">{} · {:.0}%</div></li>"#,
                html_escape(&i.name),
                (i.pct / max * 100.0).clamp(2.0, 100.0),
                plural(i.streams, "play", "plays"),
                i.pct
            ))
            .collect::<String>()
    )
}

fn playlist_list(playlists: &[Playlist]) -> String {
    format!(
        r#"<ul class="list">{}</ul>"#,
        playlists
            .iter()
            .map(|p| format!(
                r#"<li><a class="li-main" href="/#playlist/{}"><div class="li-title">{}</div><div class="li-sub">{}</div></a></li>"#,
                html_escape(&urlencoding::encode(&p.id)),
                html_escape(&p.name),
                plural(p.track_count, "track", "tracks")
            ))
            .collect::<String>()
    )
}

fn empty(msg: &str) -> String {
    format!(r#"<div class="empty">{}</div>"#, html_escape(msg))
}

fn overview_body(u: &PublicUser, stats: &PublicStats, playlists: &[Playlist]) -> String {
    let mut out = String::new();
    let mut highlights: Vec<(String, String)> = Vec::new();
    if stats.total_streams > 0 {
        highlights.push(("Streams".into(), fmt_count(stats.total_streams as i64)));
        highlights.push(("Listening time".into(), fmt_listening(stats.total_listening_secs)));
    }
    if let Some(a) = stats.top_artist.as_deref().filter(|s| !s.trim().is_empty()) {
        highlights.push(("Top artist".into(), a.to_string()));
    }
    if let Some(g) = stats.top_genre.as_deref().filter(|s| !s.trim().is_empty()) {
        highlights.push(("Top genre".into(), g.to_string()));
    }
    if stats.unique_artists > 0 {
        highlights.push(("Artists discovered".into(), fmt_count(stats.unique_artists as i64)));
    }
    if stats.longest_streak > 1 {
        highlights.push(("Longest streak".into(), format!("{} days", stats.longest_streak)));
    }
    let stats_href = profile_path(&u.username, ProfileTab::Stats, None);
    if !highlights.is_empty() {
        out.push_str(&section("Highlights", Some((&stats_href, "Full stats →")), &tiles(&highlights)));
    }
    if !stats.top_tracks.is_empty() {
        out.push_str(&section("Top tracks", Some((&stats_href, "See more →")), &track_list(&stats.top_tracks)));
    }
    if !playlists.is_empty() {
        let shown = &playlists[..playlists.len().min(6)];
        let all_href = profile_path(&u.username, ProfileTab::Playlists, None);
        let more = (playlists.len() > shown.len()).then_some((all_href.as_str(), "See all →"));
        out.push_str(&section("Public playlists", more, &playlist_list(shown)));
    }
    if out.is_empty() {
        out = empty("Nothing to show yet. Check back once they've listened to some music.");
    }
    out
}

fn stats_body(u: &PublicUser, s: &PublicRangeStats) -> String {
    let chips: String = STATS_RANGES
        .iter()
        .map(|(key, label)| {
            format!(
                r#"<a class="chip{}" href="{}">{}</a>"#,
                if *key == s.range_key { " active" } else { "" },
                html_escape(&profile_path(&u.username, ProfileTab::Stats, Some(key))),
                label
            )
        })
        .collect();
    let mut out = format!(r#"<nav class="chips" aria-label="Time range">{chips}</nav>"#);
    if s.total_streams == 0 {
        out.push_str(&empty(&format!("No listening in this period ({}).", s.range_label)));
        return out;
    }
    let mut summary = vec![
        ("Streams".to_string(), fmt_count(s.total_streams as i64)),
        ("Listening time".to_string(), fmt_listening(s.total_listening_secs)),
        ("Tracks".to_string(), fmt_count(s.unique_tracks as i64)),
        ("Artists".to_string(), fmt_count(s.unique_artists as i64)),
        ("Albums".to_string(), fmt_count(s.unique_albums as i64)),
        ("Active days".to_string(), fmt_count(s.active_days as i64)),
    ];
    if s.new_artists_discovered > 0 {
        summary.push(("New artists".into(), fmt_count(s.new_artists_discovered as i64)));
    }
    if s.longest_streak > 1 {
        summary.push(("Longest streak".into(), format!("{} days", s.longest_streak)));
    }
    out.push_str(&section(s.range_label, None, &tiles(&summary)));
    if !s.top_artists.is_empty() {
        out.push_str(&section("Top artists", None, &bar_list(&s.top_artists)));
    }
    if !s.top_tracks.is_empty() {
        out.push_str(&section("Top tracks", None, &track_list(&s.top_tracks)));
    }
    if !s.top_albums.is_empty() {
        let inner = format!(
            r#"<ol class="list">{}</ol>"#,
            s.top_albums
                .iter()
                .map(|a| format!(
                    r#"<li><div class="li-main"><div class="li-title">{}</div><div class="li-sub">{}</div></div><div class="li-meta">{}</div></li>"#,
                    html_escape(&a.name),
                    html_escape(&a.artist),
                    plural(a.streams, "play", "plays")
                ))
                .collect::<String>()
        );
        out.push_str(&section("Top albums", None, &inner));
    }
    if !s.genres.is_empty() {
        out.push_str(&section("Genres", None, &bar_list(&s.genres)));
    }
    if !s.moods.is_empty() {
        out.push_str(&section("Moods", None, &bar_list(&s.moods)));
    }
    if !s.decades.is_empty() {
        out.push_str(&section("Decades", None, &bar_list(&s.decades)));
    }
    out
}

fn people_body(users: &[PublicUser], hidden: usize, empty_msg: &str) -> String {
    let mut out = String::new();
    if users.is_empty() {
        out.push_str(&empty(empty_msg));
    } else {
        out.push_str(&format!(
            r#"<ul class="list">{}</ul>"#,
            users
                .iter()
                .map(|p| format!(
                    r#"<li><a class="li-main person" href="{}"><span class="mini-av" style="background:{}">{}</span><span class="person-text"><span class="li-title">{}</span><span class="li-sub">@{}</span></span></a></li>"#,
                    html_escape(&profile_path(&p.username, ProfileTab::Overview, None)),
                    safe_color(&p.avatar_color),
                    avatar_inner(p),
                    html_escape(display_name(p)),
                    html_escape(&p.username)
                ))
                .collect::<String>()
        ));
    }
    if hidden > 0 {
        out.push_str(&format!(
            r#"<p class="note">{} without a public profile {} not listed.</p>"#,
            plural(hidden, "listener", "listeners"),
            if hidden == 1 { "is" } else { "are" }
        ));
    }
    out
}

// ---------------------------------------------------------------------------
// Page shell
// ---------------------------------------------------------------------------

pub fn render_profile_page(origin: &str, u: &PublicUser, body: &ProfileBody) -> String {
    let tab = body.tab();
    let (og_title, og_desc) = build_meta(u, body);
    let range = match body {
        ProfileBody::Stats(s) => Some(s.range_key),
        _ => None,
    };
    let page_url = format!("{}{}", origin, profile_path(&u.username, tab, range));
    // In-app the profile opens on its Stats tab, so the overview maps to the
    // bare profile route.
    let app_url = if tab == ProfileTab::Overview {
        format!("{}/#user/{}", origin, urlencoding::encode(&u.username))
    } else {
        format!("{}/#user/{}/{}", origin, urlencoding::encode(&u.username), tab.slug())
    };
    let color = safe_color(&u.avatar_color);
    let name = display_name(u);
    let og_image = if u.avatar_url.trim().is_empty() {
        format!("{}/web/icon-512.png", origin)
    } else {
        format!("{}{}", origin, u.avatar_url)
    };

    let joined = fmt_month_year(u.created);
    let mut sub = format!("@{}", html_escape(&u.username));
    if !joined.is_empty() {
        sub.push_str(&format!(" · Joined {}", html_escape(&joined)));
    }
    let bio_html = if u.bio.trim().is_empty() {
        String::new()
    } else {
        format!(r#"<p class="bio">{}</p>"#, html_escape(u.bio.trim()))
    };
    let loc_html = if u.location.trim().is_empty() {
        String::new()
    } else {
        format!(r#"<div class="loc">📍 {}</div>"#, html_escape(u.location.trim()))
    };
    let counters_html = [
        (fmt_count(u.followers_count), "Followers", ProfileTab::Followers),
        (fmt_count(u.following_count), "Following", ProfileTab::Following),
    ]
    .iter()
    .map(|(v, l, t)| {
        format!(
            r#"<a class="stat" href="{}"><strong>{}</strong><span>{}</span></a>"#,
            html_escape(&profile_path(&u.username, *t, None)),
            html_escape(v),
            l
        )
    })
    .collect::<String>();
    let tabs_html: String = ProfileTab::ALL
        .iter()
        .map(|t| {
            format!(
                r#"<a class="tab{}" href="{}"{}>{}</a>"#,
                if *t == tab { " active" } else { "" },
                html_escape(&profile_path(&u.username, *t, None)),
                if *t == tab { r#" aria-current="page""# } else { "" },
                t.label()
            )
        })
        .collect();

    let body_html = match body {
        ProfileBody::Overview { stats, playlists } => overview_body(u, stats, playlists),
        ProfileBody::Stats(s) => stats_body(u, s),
        ProfileBody::Playlists(p) => {
            if p.is_empty() {
                empty("No public playlists yet.")
            } else {
                section("Public playlists", None, &playlist_list(p))
            }
        }
        ProfileBody::Followers(users, hidden) => {
            people_body(users, *hidden, "No followers to show yet.")
        }
        ProfileBody::Following(users, hidden) => {
            people_body(users, *hidden, "Not following anyone with a public profile yet.")
        }
    };

    format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0, viewport-fit=cover" />
  <meta name="theme-color" content="#0c0e13" />
  <title>{title}</title>
  <meta name="description" content="{desc}" />
  <link rel="canonical" href="{page_url}" />
  <link rel="icon" type="image/svg+xml" href="/web/favicon.svg" />
  <link rel="apple-touch-icon" sizes="180x180" href="/web/apple-touch-icon.png" />
  <meta property="og:site_name" content="Beatlab" />
  <meta property="og:type" content="profile" />
  <meta property="og:title" content="{title}" />
  <meta property="og:description" content="{desc}" />
  <meta property="og:url" content="{page_url}" />
  <meta property="og:image" content="{og_image}" />
  <meta property="profile:username" content="{username}" />
  <meta name="twitter:card" content="summary" />
  <meta name="twitter:title" content="{title}" />
  <meta name="twitter:description" content="{desc}" />
  <meta name="twitter:image" content="{og_image}" />
  <style>
    * {{ box-sizing:border-box; }}
    body {{ margin:0; min-height:100vh; background:#0c0e13; color:#f3f5f8; font-family:system-ui,-apple-system,"Segoe UI",sans-serif; line-height:1.4; }}
    a {{ color:inherit; }}
    .wrap {{ max-width:760px; margin:0 auto; padding:24px 16px 48px; }}
    .brand {{ display:inline-flex; align-items:center; gap:8px; color:#98a2b3; text-decoration:none; font-weight:700; font-size:14px; margin-bottom:20px; }}
    .brand img {{ width:22px; height:22px; }}
    .hero {{ border:1px solid {color}55; background:{color}14; border-radius:16px; padding:24px; display:flex; gap:20px; align-items:center; flex-wrap:wrap; }}
    .avatar {{ width:112px; height:112px; border-radius:50%; background:{color}; color:#06120b; display:grid; place-items:center; font-size:46px; font-weight:800; overflow:hidden; flex-shrink:0; }}
    .avatar img, .mini-av img {{ width:100%; height:100%; object-fit:cover; }}
    .info {{ flex:1; min-width:220px; }}
    .kicker {{ font-size:11px; letter-spacing:.08em; text-transform:uppercase; color:#98a2b3; font-weight:700; }}
    h1 {{ font-size:32px; margin:4px 0 2px; word-break:break-word; }}
    .sub {{ color:#98a2b3; font-size:14px; }}
    .bio {{ margin:10px 0 0; white-space:pre-wrap; word-break:break-word; }}
    .loc {{ margin-top:6px; color:#98a2b3; font-size:14px; }}
    .stats {{ display:flex; flex-wrap:wrap; gap:18px; margin-top:14px; }}
    .stat {{ text-decoration:none; }}
    .stat strong {{ display:block; font-size:18px; }}
    .stat span {{ color:#98a2b3; font-size:12px; }}
    .stat:hover span {{ text-decoration:underline; }}
    .actions {{ display:flex; flex-wrap:wrap; gap:10px; margin-top:18px; }}
    .btn {{ display:inline-block; background:#1ed760; color:#06120b; font-weight:700; padding:10px 18px; border-radius:999px; text-decoration:none; border:0; font-size:14px; cursor:pointer; font-family:inherit; }}
    .btn.ghost {{ background:transparent; color:#f3f5f8; border:1px solid #2d3341; }}
    .tabs {{ display:flex; gap:6px; margin:22px 0 4px; overflow-x:auto; scrollbar-width:none; border-bottom:1px solid #232834; }}
    .tabs::-webkit-scrollbar {{ display:none; }}
    .tab {{ padding:10px 14px; text-decoration:none; color:#98a2b3; font-weight:600; font-size:14px; white-space:nowrap; border-bottom:2px solid transparent; margin-bottom:-1px; }}
    .tab:hover {{ color:#f3f5f8; }}
    .tab.active {{ color:#f3f5f8; border-bottom-color:{color}; }}
    .chips {{ display:flex; flex-wrap:wrap; gap:8px; margin-top:18px; }}
    .chip {{ padding:6px 12px; border-radius:999px; border:1px solid #2d3341; text-decoration:none; font-size:13px; color:#98a2b3; }}
    .chip.active {{ background:#f3f5f8; color:#0c0e13; border-color:#f3f5f8; font-weight:600; }}
    section {{ margin-top:28px; }}
    .sec-head {{ display:flex; align-items:baseline; justify-content:space-between; gap:12px; margin-bottom:12px; }}
    h2 {{ font-size:18px; margin:0; }}
    .more {{ color:#98a2b3; font-size:13px; text-decoration:none; white-space:nowrap; }}
    .more:hover {{ color:#f3f5f8; }}
    .hl-grid {{ display:grid; grid-template-columns:repeat(auto-fill,minmax(150px,1fr)); gap:10px; }}
    .hl {{ background:#12151c; border:1px solid #232834; border-radius:12px; padding:12px 14px; min-width:0; }}
    .hl span {{ display:block; color:#98a2b3; font-size:12px; }}
    .hl strong {{ display:block; margin-top:2px; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }}
    .list {{ list-style:none; margin:0; padding:0; background:#12151c; border:1px solid #232834; border-radius:12px; overflow:hidden; counter-reset:n; }}
    .list li {{ display:flex; align-items:center; gap:12px; padding:12px 14px; border-top:1px solid #1b1f29; }}
    .list li:first-child {{ border-top:0; }}
    ol.list li::before {{ counter-increment:n; content:counter(n); color:#98a2b3; min-width:18px; text-align:right; font-variant-numeric:tabular-nums; }}
    .li-main {{ flex:1; min-width:0; text-decoration:none; }}
    .li-title {{ display:block; font-weight:600; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }}
    .li-sub, .li-meta {{ color:#98a2b3; font-size:13px; }}
    .li-sub {{ display:block; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }}
    .li-meta {{ flex-shrink:0; text-align:right; }}
    .bar {{ height:6px; border-radius:3px; background:#1b1f29; margin-top:6px; overflow:hidden; }}
    .bar i {{ display:block; height:100%; background:{color}; border-radius:3px; }}
    .person {{ display:flex; align-items:center; gap:12px; }}
    .person-text {{ min-width:0; }}
    .mini-av {{ width:40px; height:40px; border-radius:50%; display:grid; place-items:center; color:#06120b; font-weight:800; overflow:hidden; flex-shrink:0; }}
    .empty {{ margin-top:24px; padding:28px 16px; text-align:center; color:#98a2b3; background:#12151c; border:1px solid #232834; border-radius:12px; }}
    .note {{ color:#6b7485; font-size:13px; margin-top:10px; }}
    .foot {{ margin-top:36px; text-align:center; color:#6b7485; font-size:13px; }}
    .foot a {{ color:#98a2b3; }}
    @media (max-width:480px) {{ .hero {{ padding:18px; }} .avatar {{ width:88px; height:88px; font-size:36px; }} h1 {{ font-size:26px; }} .li-meta {{ font-size:12px; }} }}
  </style>
</head>
<body>
  <main class="wrap">
    <a class="brand" href="/"><img src="/web/favicon.svg" alt="" />Beatlab</a>
    <div class="hero">
      <div class="avatar">{avatar_html}</div>
      <div class="info">
        <div class="kicker">Beatlab listener</div>
        <h1>{name}</h1>
        <div class="sub">{sub}</div>
        {bio_html}
        {loc_html}
        <div class="stats">{counters_html}</div>
        <div class="actions">
          <a class="btn" href="{app_url}">Open in Beatlab</a>
          <button class="btn ghost" id="copy-btn" type="button">Copy link</button>
        </div>
      </div>
    </div>
    <nav class="tabs" aria-label="Profile sections">{tabs_html}</nav>
    {body_html}
    <div class="foot">Listen, follow @{username} and play their radio on <a href="/">Beatlab</a>.</div>
  </main>
  <script>
    (function () {{
      var btn = document.getElementById('copy-btn');
      var url = {page_url_js};
      btn.addEventListener('click', function () {{
        var done = function () {{ btn.textContent = 'Copied!'; setTimeout(function () {{ btn.textContent = 'Copy link'; }}, 1600); }};
        if (navigator.share && /Mobi|Android/i.test(navigator.userAgent)) {{
          navigator.share({{ title: document.title, url: url }}).catch(function () {{}});
        }} else if (navigator.clipboard && navigator.clipboard.writeText) {{
          navigator.clipboard.writeText(url).then(done, function () {{ prompt('Copy this link', url); }});
        }} else {{
          prompt('Copy this link', url);
        }}
      }});
    }})();
  </script>
</body>
</html>"##,
        title = html_escape(&og_title),
        desc = html_escape(&og_desc),
        page_url = html_escape(&page_url),
        page_url_js = serde_json::Value::String(page_url.clone())
            .to_string()
            .replace("</", "<\\/"),
        app_url = html_escape(&app_url),
        og_image = html_escape(&og_image),
        username = html_escape(&u.username),
        color = color,
        avatar_html = avatar_inner(u),
        name = html_escape(name),
        sub = sub,
        bio_html = bio_html,
        loc_html = loc_html,
        counters_html = counters_html,
        tabs_html = tabs_html,
        body_html = body_html,
    )
}

pub fn render_profile_not_found_html() -> String {
    r##"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0" />
  <meta name="theme-color" content="#0c0e13" />
  <title>Profile Not Found — Beatlab</title>
  <style>
    body { margin:0; min-height:100vh; background:#0c0e13; color:#f3f5f8; font-family:system-ui,-apple-system,sans-serif; display:grid; place-items:center; padding:20px; }
    .box { max-width:420px; text-align:center; background:#12151c; border:1px solid #232834; border-radius:14px; padding:32px 24px; }
    h1 { font-size:22px; margin-bottom:8px; }
    p { color:#98a2b3; font-size:14px; margin-bottom:20px; }
    a { display:inline-block; background:#1ed760; color:#06120b; font-weight:700; padding:10px 20px; border-radius:8px; text-decoration:none; }
  </style>
</head>
<body>
  <div class="box">
    <h1>Profile not found</h1>
    <p>This Beatlab user doesn't exist or their account has been removed.</p>
    <a href="/">Go to Beatlab</a>
  </div>
</body>
</html>"##
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user() -> PublicUser {
        PublicUser {
            id: 7,
            username: "emily".into(),
            display_name: "Emily <C>".into(),
            bio: "Synth-pop \"obsessive\"".into(),
            avatar_color: "red;position:fixed".into(),
            avatar_letter: "E".into(),
            avatar_url: String::new(),
            location: "Brooklyn".into(),
            created: 1_700_000_000,
            followers_count: 1234,
            following_count: 1,
            is_following: false,
            is_self: false,
        }
    }

    fn range_stats() -> PublicRangeStats {
        PublicRangeStats {
            range_key: "30d",
            range_label: "Last 30 Days",
            total_streams: 42,
            total_listening_secs: 7200,
            unique_tracks: 30,
            unique_artists: 12,
            unique_albums: 20,
            active_days: 9,
            new_artists_discovered: 3,
            longest_streak: 4,
            top_tracks: vec![],
            top_artists: vec![PublicShare { name: "Yorushika <3".into(), streams: 10, pct: 23.8 }],
            top_albums: vec![],
            genres: vec![],
            moods: vec![],
            decades: vec![],
        }
    }

    #[test]
    fn renders_escaped_overview_with_meta() {
        let stats = PublicStats::default();
        let html = render_profile_page(
            "https://beat.example",
            &user(),
            &ProfileBody::Overview { stats: &stats, playlists: &[] },
        );
        assert!(html.contains("Emily &lt;C&gt;"));
        assert!(!html.contains("Emily <C>"));
        assert!(html.contains(r#"property="og:url" content="https://beat.example/u/emily""#));
        assert!(html.contains("/web/icon-512.png"));
        assert!(html.contains("1,234"));
        // Invalid colors never reach the stylesheet.
        assert!(!html.contains("position:fixed"));
        // Every tab is linked from the tab bar.
        for slug in ["stats", "playlists", "followers", "following"] {
            assert!(html.contains(&format!(r#"href="/u/emily/{slug}""#)), "{slug}");
        }
    }

    #[test]
    fn renders_stats_tab_with_range_links() {
        let s = range_stats();
        let html = render_profile_page("https://beat.example", &user(), &ProfileBody::Stats(&s));
        assert!(html.contains(r#"content="https://beat.example/u/emily/stats?range=30d""#));
        assert!(html.contains(r#"class="chip active" href="/u/emily/stats?range=30d""#));
        // "All time" is the canonical, query-less stats URL.
        assert!(html.contains(r#"href="/u/emily/stats">All time"#));
        assert!(html.contains("Yorushika &lt;3"));
        assert!(html.contains("2 hr"));
        assert!(html.contains("/#user/emily/stats"));
    }

    #[test]
    fn people_tab_reports_hidden_listeners() {
        let friend = PublicUser { username: "zoe".into(), display_name: String::new(), ..user() };
        let html = render_profile_page(
            "https://beat.example",
            &user(),
            &ProfileBody::Followers(std::slice::from_ref(&friend), 2),
        );
        assert!(html.contains(r#"href="/u/zoe""#));
        assert!(html.contains("2 listeners without a public profile are not listed"));
    }

    #[test]
    fn tab_slugs_roundtrip() {
        for t in ProfileTab::ALL {
            let slug = if t == ProfileTab::Overview { None } else { Some(t.slug()) };
            assert_eq!(ProfileTab::from_slug(slug), Some(t));
        }
        assert_eq!(ProfileTab::from_slug(Some("Stats/")), Some(ProfileTab::Stats));
        assert_eq!(ProfileTab::from_slug(Some("radio")), None);
    }

    #[test]
    fn count_formatting() {
        assert_eq!(fmt_count(0), "0");
        assert_eq!(fmt_count(999), "999");
        assert_eq!(fmt_count(1000), "1,000");
        assert_eq!(fmt_count(1234567), "1,234,567");
    }
}
