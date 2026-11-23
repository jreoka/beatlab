//! Public, sign-in-free profile pages served at `/u/{username}` and its tabs:
//!
//! | URL                         | Content                                         |
//! |-----------------------------|-------------------------------------------------|
//! | `/u/{user}`                 | Overview: highlights, top tracks, playlists     |
//! | `/u/{user}/stats?range=30d` | Listening stats & graphs for 7d/30d/90d/365d/all|
//! | `/u/{user}/playlists`       | All public playlists                            |
//! | `/u/{user}/followers`       | Followers who also have a public page           |
//! | `/u/{user}/following`       | Followed users who also have a public page      |
//!
//! Every page carries Open Graph / Twitter meta so links unfurl nicely in
//! Discord, iMessage, Slack, etc.

use crate::models::{Playlist, PublicUser};
use crate::share_page::html_escape;
use crate::stats::{ComputedStats, HeatmapStats, HourStat, MoodStat, PublicStats, TimelineStats};
use chrono::{Datelike, Duration, NaiveDate};
use std::collections::HashMap;
use std::f64::consts::PI;

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
    Stats {
        stats: &'a ComputedStats,
        /// `"streams"` or `"mins"`.
        metric: &'a str,
    },
    Playlists(&'a [Playlist]),
    /// Visible users plus the number left out because their page is off.
    Followers(&'a [PublicUser], usize),
    Following(&'a [PublicUser], usize),
}

impl ProfileBody<'_> {
    fn tab(&self) -> ProfileTab {
        match self {
            Self::Overview { .. } => ProfileTab::Overview,
            Self::Stats { .. } => ProfileTab::Stats,
            Self::Playlists(_) => ProfileTab::Playlists,
            Self::Followers(..) => ProfileTab::Followers,
            Self::Following(..) => ProfileTab::Following,
        }
    }
}

const STATS_RANGES: [(&str, &str); 5] = [
    ("7d", "7 Days"),
    ("30d", "30 Days"),
    ("90d", "90 Days"),
    ("365d", "1 Year"),
    ("all", "All Time"),
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

fn fmt_date_label(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|d| d.format("%b %-d, %Y").to_string())
        .unwrap_or_default()
}

fn fmt_short_day_iso(iso: &str) -> String {
    NaiveDate::parse_from_str(iso, "%Y-%m-%d")
        .map(|d| d.format("%b %-d, %Y").to_string())
        .unwrap_or_else(|_| iso.to_string())
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

fn fmt_stats_duration(secs: i64) -> String {
    let s = secs.max(0);
    if s < 60 {
        return format!("{s}s");
    }
    let total_mins = s / 60;
    let hrs = total_mins / 60;
    let mins = total_mins % 60;
    if hrs >= 24 {
        let days = hrs / 24;
        let rem_h = hrs % 24;
        if rem_h > 0 {
            format!("{}d {}h", fmt_count(days), rem_h)
        } else {
            format!("{}d", fmt_count(days))
        }
    } else if hrs >= 1 {
        if mins > 0 {
            format!("{hrs}h {mins}m")
        } else {
            format!("{hrs}h")
        }
    } else {
        format!("{mins}m")
    }
}

fn fmt_track_time(secs: i64) -> String {
    let s = secs.max(0);
    format!("{}:{:02}", s / 60, s % 60)
}

fn fmt_hour_long(h: usize) -> String {
    let h = h % 24;
    if h == 0 {
        "12 AM".to_string()
    } else if h < 12 {
        format!("{h} AM")
    } else if h == 12 {
        "12 PM".to_string()
    } else {
        format!("{} PM", h - 12)
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

fn stats_path(username: &str, range: &str, metric: &str) -> String {
    let mut p = format!("/u/{}/stats", urlencoding::encode(username));
    let mut params: Vec<String> = Vec::new();
    if range != "all" {
        params.push(format!("range={range}"));
    }
    if metric == "mins" {
        params.push("metric=mins".to_string());
    }
    if !params.is_empty() {
        p.push('?');
        p.push_str(&params.join("&"));
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
        ProfileBody::Stats { stats: s, .. } => {
            let title = format!("{name}'s listening stats (@{}) · Beatlab", u.username);
            let mut facts = vec![
                s.range.label().to_string(),
                format!("{} streams", fmt_count(s.summary.total_streams as i64)),
                fmt_listening(s.summary.total_listening_secs),
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
// SVG chart renderers (mirroring the personal stats page)
// ---------------------------------------------------------------------------

fn svg_donut_slice_path(cx: f64, cy: f64, r_outer: f64, r_inner: f64, start_deg: f64, end_deg: f64) -> String {
    let to_rad = |deg: f64| deg * PI / 180.0;
    let a1 = to_rad(start_deg);
    let a2 = to_rad(end_deg);
    let x1 = cx + r_outer * a1.cos();
    let y1 = cy + r_outer * a1.sin();
    let x2 = cx + r_outer * a2.cos();
    let y2 = cy + r_outer * a2.sin();
    let x3 = cx + r_inner * a2.cos();
    let y3 = cy + r_inner * a2.sin();
    let x4 = cx + r_inner * a1.cos();
    let y4 = cy + r_inner * a1.sin();
    let large = if (end_deg - start_deg).abs() > 180.0 { 1 } else { 0 };
    format!(
        "M {x1:.2} {y1:.2} A {r_outer:.2} {r_outer:.2} 0 {large} 1 {x2:.2} {y2:.2} L {x3:.2} {y3:.2} A {r_inner:.2} {r_inner:.2} 0 {large} 0 {x4:.2} {y4:.2} Z"
    )
}

fn render_stats_timeline_svg(timeline: &TimelineStats, metric: &str) -> String {
    let points = &timeline.points;
    if points.is_empty() {
        return r#"<p class="muted-center">No timeline points yet.</p>"#.to_string();
    }
    let vals: Vec<i64> = points
        .iter()
        .map(|p| if metric == "mins" { (p.secs + 30) / 60 } else { p.streams as i64 })
        .collect();
    let max_val = vals.iter().copied().max().unwrap_or(0).max(1) as f64;
    let peak_val = vals.iter().copied().max().unwrap_or(0);

    let w = 760.0_f64;
    let h = 228.0_f64;
    let pad_l = 42.0_f64;
    let pad_r = 14.0_f64;
    let pad_t = 18.0_f64;
    let pad_b = 30.0_f64;
    let plot_w = w - pad_l - pad_r;
    let plot_h = h - pad_t - pad_b;
    let base_y = pad_t + plot_h;
    let n = points.len();
    let slot_w = plot_w / (n as f64);
    let bar_w = (slot_w * 0.54).clamp(3.0, 28.0);

    let coords: Vec<(f64, f64, i64)> = vals
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            let x = pad_l + (i as f64 + 0.5) * slot_w;
            let y = base_y - (v as f64 / max_val) * plot_h;
            (x, y, v)
        })
        .collect();

    let unit_suffix = if metric == "mins" { "m" } else { "" };
    let grid_svg: String = [0.0_f64, 0.5, 1.0]
        .iter()
        .map(|&t| {
            let y = base_y - t * plot_h;
            let val = (t * max_val).round() as i64;
            let dash = if t == 0.0 { "0" } else { "3 4" };
            format!(
                r##"<line x1="{pad_l}" y1="{y:.1}" x2="{x2}" y2="{y:.1}" stroke="#232834" stroke-dasharray="{dash}" stroke-width="1" /><text x="{tx}" y="{ty:.1}" text-anchor="end" fill="#6b7485" font-size="10.5">{val}{unit_suffix}</text>"##,
                x2 = w - pad_r,
                tx = pad_l - 8.0,
                ty = y + 4.0,
            )
        })
        .collect();

    let (line_path, area_path) = if coords.len() == 1 {
        (format!("M {:.1} {:.1}", coords[0].0, coords[0].1), String::new())
    } else {
        let mut lp = String::new();
        for (i, &(x, y, _)) in coords.iter().enumerate() {
            if i == 0 {
                lp.push_str(&format!("M {x:.1} {y:.1}"));
            } else {
                let (px, py, _) = coords[i - 1];
                let mid_x = (px + x) / 2.0;
                lp.push_str(&format!(" C {mid_x:.1} {py:.1}, {mid_x:.1} {y:.1}, {x:.1} {y:.1}"));
            }
        }
        let last_x = coords.last().unwrap().0;
        let first_x = coords[0].0;
        let ap = format!("{lp} L {last_x:.1} {base_y:.1} L {first_x:.1} {base_y:.1} Z");
        (lp, ap)
    };

    let label_step = (n.div_ceil(8)).max(1);
    let bars_and_hits: String = coords
        .iter()
        .enumerate()
        .map(|(i, &(x, y, v))| {
            let p = &points[i];
            let bar_h = if v > 0 { (base_y - y).max(3.0) } else { 0.0 };
            let bx = x - bar_w / 2.0;
            let is_peak = peak_val > 0 && v == peak_val;
            let tip_title = if p.label.is_empty() { &p.key } else { &p.label };
            let tip_body = format!(
                "{} · {} ({} completed, {} skipped)",
                plural(p.streams, "stream", "streams"),
                fmt_stats_duration(p.secs),
                p.completed,
                p.skipped
            );
            let x_label = if i % label_step == 0 || i + 1 == n {
                format!(
                    r##"<text x="{x:.1}" y="{ly}" text-anchor="middle" fill="#6b7485" font-size="10.5">{}</text>"##,
                    html_escape(&p.short_label),
                    ly = h - 8.0,
                )
            } else {
                String::new()
            };
            let peak_dot = if is_peak {
                format!(r##"<circle cx="{x:.1}" cy="{y:.1}" r="4.2" fill="#1ed760" stroke="#0c0e13" stroke-width="2" />"##)
            } else {
                String::new()
            };
            let fill = if is_peak { "#1ed760" } else { "rgba(56, 189, 248, 0.38)" };
            let hit_x = pad_l + (i as f64) * slot_w;
            format!(
                r#"<rect x="{bx:.1}" y="{y:.1}" width="{bar_w:.1}" height="{bar_h:.1}" rx="2" fill="{fill}" />{peak_dot}{x_label}<rect class="stats-hit-col stats-tip-target" x="{hit_x:.1}" y="{pad_t}" width="{slot_w:.1}" height="{plot_h}" fill="transparent" data-tip-title="{}" data-tip-body="{}" />"#,
                html_escape(tip_title),
                html_escape(&tip_body),
            )
        })
        .collect();

    let area_el = if area_path.is_empty() {
        String::new()
    } else {
        format!(r#"<path d="{area_path}" fill="url(#statsTimelineGrad)" />"#)
    };
    let line_el = if line_path.is_empty() {
        String::new()
    } else {
        format!(r##"<path d="{line_path}" fill="none" stroke="#1ed760" stroke-width="2.2" stroke-linecap="round" />"##)
    };

    format!(
        r##"<svg class="stats-svg-chart" viewBox="0 0 {w} {h}" preserveAspectRatio="xMidYMid meet"><defs><linearGradient id="statsTimelineGrad" x1="0" y1="0" x2="0" y2="1"><stop offset="0%" stop-color="#1ed760" stop-opacity="0.28" /><stop offset="100%" stop-color="#1ed760" stop-opacity="0.01" /></linearGradient></defs>{grid_svg}{area_el}{line_el}{bars_and_hits}</svg>"##
    )
}

fn render_stats_clock_svg(hourly: &[HourStat], metric: &str, peak_hour: Option<usize>) -> String {
    let vals: Vec<i64> = hourly
        .iter()
        .map(|h| if metric == "mins" { (h.secs + 30) / 60 } else { h.streams as i64 })
        .collect();
    let max_val = vals.iter().copied().max().unwrap_or(0).max(1) as f64;
    let w = 280.0_f64;
    let h = 280.0_f64;
    let cx = 140.0_f64;
    let cy = 140.0_f64;
    let r_inner = 46.0_f64;
    let r_max = 112.0_f64;

    let wedges: String = hourly
        .iter()
        .enumerate()
        .map(|(i, hr)| {
            let start_deg = (i as f64) * 15.0 - 90.0 + 1.1;
            let end_deg = ((i + 1) as f64) * 15.0 - 90.0 - 1.1;
            let v = vals[i];
            let ratio = v as f64 / max_val;
            let r_outer = if v > 0 {
                r_inner + 10.0 + ratio * (r_max - r_inner - 10.0)
            } else {
                r_inner + 4.0
            };
            let (fill, opacity) = if v == 0 {
                ("#232834", 0.85_f64)
            } else if peak_hour == Some(i) {
                ("#1ed760", 1.0)
            } else if i >= 22 || i <= 4 {
                ("#38bdf8", 0.35 + 0.65 * ratio)
            } else if (17..=21).contains(&i) {
                ("#a855f7", 0.35 + 0.65 * ratio)
            } else {
                ("#1ed760", 0.30 + 0.60 * ratio)
            };
            let tip_title = format!("{} – {}", fmt_hour_long(i), fmt_hour_long((i + 1) % 24));
            let tip_body = format!(
                "{} ({:.1}%) · {}",
                plural(hr.streams, "stream", "streams"),
                hr.pct,
                fmt_stats_duration(hr.secs)
            );
            format!(
                r#"<path d="{}" fill="{fill}" fill-opacity="{opacity:.2}" /><path class="stats-clock-hit stats-tip-target" d="{}" fill="transparent" data-tip-title="{}" data-tip-body="{}" />"#,
                svg_donut_slice_path(cx, cy, r_outer, r_inner, start_deg, end_deg),
                svg_donut_slice_path(cx, cy, r_max + 4.0, r_inner, start_deg, end_deg),
                html_escape(&tip_title),
                html_escape(&tip_body),
            )
        })
        .collect();

    let peak_label = peak_hour.map(fmt_hour_long).unwrap_or_else(|| "—".to_string());
    let peak_pct = peak_hour
        .and_then(|idx| hourly.get(idx))
        .map(|hr| format!("{:.1}% of plays", hr.pct))
        .unwrap_or_else(|| "No streams".to_string());

    format!(
        r##"<svg class="stats-clock-svg" viewBox="0 0 {w} {h}"><circle cx="{cx}" cy="{cy}" r="{r_max}" fill="none" stroke="#232834" stroke-dasharray="3 4" /><circle cx="{cx}" cy="{cy}" r="{r_mid:.1}" fill="none" stroke="#1e2330" stroke-dasharray="2 4" /><text x="{cx}" y="16" text-anchor="middle" fill="#98a2b3" font-size="11" font-weight="700">12 AM</text><text x="{x_r}" y="{y_c}" text-anchor="end" fill="#98a2b3" font-size="11" font-weight="700">6 AM</text><text x="{cx}" y="{y_b}" text-anchor="middle" fill="#98a2b3" font-size="11" font-weight="700">12 PM</text><text x="8" y="{y_c}" text-anchor="start" fill="#98a2b3" font-size="11" font-weight="700">6 PM</text>{wedges}<circle cx="{cx}" cy="{cy}" r="{r_c}" fill="#12151c" stroke="#232834" /><text x="{cx}" y="{y_t1}" text-anchor="middle" fill="#f3f5f8" font-size="15" font-weight="800">{peak_label}</text><text x="{cx}" y="{y_t2}" text-anchor="middle" fill="#98a2b3" font-size="10">{peak_pct}</text></svg>"##,
        r_mid = (r_inner + r_max) / 2.0,
        x_r = w - 8.0,
        y_c = cy + 4.0,
        y_b = h - 8.0,
        r_c = r_inner - 4.0,
        y_t1 = cy - 3.0,
        y_t2 = cy + 14.0,
        peak_label = html_escape(&peak_label),
        peak_pct = html_escape(&peak_pct),
    )
}

fn render_stats_heatmap(heatmap: &HeatmapStats) -> String {
    let Ok(start_date) = NaiveDate::parse_from_str(&heatmap.start_date, "%Y-%m-%d") else {
        return String::new();
    };
    let Ok(end_date) = NaiveDate::parse_from_str(&heatmap.end_date, "%Y-%m-%d") else {
        return String::new();
    };
    let day_map: HashMap<&str, (usize, i64)> = heatmap
        .days
        .iter()
        .map(|d| (d.date.as_str(), (d.streams, d.secs)))
        .collect();
    let max_streams = heatmap.max_streams.max(1) as f64;

    // Align grid start back to Monday (0=Mon..6=Sun).
    let start_dow = start_date.weekday().num_days_from_monday() as i64;
    let mut cur = start_date - Duration::days(start_dow);

    let mut weeks: Vec<Vec<(NaiveDate, bool, usize, i64)>> = Vec::new();
    while cur <= end_date {
        let mut col = Vec::with_capacity(7);
        for r in 0..7 {
            let d = cur + Duration::days(r);
            let in_range = d >= start_date && d <= end_date;
            let (streams, secs) = if in_range {
                let iso = d.format("%Y-%m-%d").to_string();
                day_map.get(iso.as_str()).copied().unwrap_or((0, 0))
            } else {
                (0, 0)
            };
            col.push((d, in_range, streams, secs));
        }
        weeks.push(col);
        cur += Duration::days(7);
    }

    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let mut last_month: i32 = -1;
    let mut last_label_col: Option<usize> = None;
    let month_labels: String = weeks
        .iter()
        .enumerate()
        .map(|(col_idx, col)| {
            let first = col
                .iter()
                .find(|(d, in_r, _, _)| *in_r && d.day() <= 7)
                .or_else(|| col.iter().find(|(_, in_r, _, _)| *in_r));
            let mut label = "";
            if let Some((d, _, _, _)) = first {
                let m = d.month0() as i32;
                let has_room = last_label_col.map_or(true, |prev| col_idx >= prev + 2);
                let show_first = last_month == -1 && d.day() <= 21;
                if m != last_month && has_room && (d.day() <= 7 || show_first) {
                    label = MONTHS[m as usize];
                    last_month = m;
                    last_label_col = Some(col_idx);
                }
            }
            format!(r#"<div class="stats-hm-month">{label}</div>"#)
        })
        .collect();

    let cols_html: String = weeks
        .iter()
        .map(|col| {
            let cells: String = col
                .iter()
                .map(|(d, in_range, streams, secs)| {
                    if !*in_range {
                        return r#"<div class="stats-hm-cell empty"></div>"#.to_string();
                    }
                    let lvl = if *streams == 0 {
                        0
                    } else {
                        let ratio = *streams as f64 / max_streams;
                        if ratio <= 0.25 {
                            1
                        } else if ratio <= 0.50 {
                            2
                        } else if ratio <= 0.75 {
                            3
                        } else {
                            4
                        }
                    };
                    let tip_title = d.format("%b %-d, %Y").to_string();
                    let tip_body = if *streams > 0 {
                        format!(
                            "{} · {}",
                            plural(*streams, "stream", "streams"),
                            fmt_stats_duration(*secs)
                        )
                    } else {
                        "No streams logged".to_string()
                    };
                    format!(
                        r#"<div class="stats-hm-cell lv-{lvl} stats-tip-target" data-tip-title="{}" data-tip-body="{}"></div>"#,
                        html_escape(&tip_title),
                        html_escape(&tip_body),
                    )
                })
                .collect();
            format!(r#"<div class="stats-hm-col">{cells}</div>"#)
        })
        .collect();

    let week_count = weeks.len().max(1);
    format!(
        r#"<div class="stats-hm-scroll"><div class="stats-hm-inner" style="--hm-weeks:{week_count}"><div class="stats-hm-months-row"><div class="stats-hm-wd-spacer"></div><div class="stats-hm-months" style="grid-template-columns:repeat({week_count},12px)">{month_labels}</div></div><div class="stats-hm-body"><div class="stats-hm-weekdays"><span>Mon</span><span></span><span>Wed</span><span></span><span>Fri</span><span></span><span></span></div><div class="stats-hm-grid" style="grid-template-columns:repeat({week_count},12px)">{cols_html}</div></div></div></div><div class="stats-hm-legend"><span>Less</span><span class="stats-hm-cell lv-0"></span><span class="stats-hm-cell lv-1"></span><span class="stats-hm-cell lv-2"></span><span class="stats-hm-cell lv-3"></span><span class="stats-hm-cell lv-4"></span><span>More</span></div>"#
    )
}

fn render_stats_mood_radar_svg(moods: &[MoodStat]) -> String {
    if moods.is_empty() {
        return String::new();
    }
    let w = 340.0_f64;
    let h = 286.0_f64;
    let cx = 170.0_f64;
    let cy = 143.0_f64;
    let r_max = 88.0_f64;
    let n = moods.len();
    let max_streams = moods.iter().map(|m| m.streams).max().unwrap_or(0).max(1) as f64;
    let has_any = moods.iter().any(|m| m.streams > 0);

    let point_at = |idx: usize, radius: f64| -> (f64, f64, f64) {
        let ang = -PI / 2.0 + (idx as f64 * 2.0 * PI) / (n as f64);
        (cx + radius * ang.cos(), cy + radius * ang.sin(), ang)
    };

    let rings: String = [0.25_f64, 0.5, 0.75, 1.0]
        .iter()
        .map(|&frac| {
            let pts: String = (0..n)
                .map(|i| {
                    let (x, y, _) = point_at(i, r_max * frac);
                    format!("{x:.1},{y:.1}")
                })
                .collect::<Vec<_>>()
                .join(" ");
            format!(r##"<polygon points="{pts}" fill="none" stroke="#232834" stroke-width="1" />"##)
        })
        .collect();

    let spokes: String = (0..n)
        .map(|i| {
            let (x, y, _) = point_at(i, r_max);
            format!(r##"<line x1="{cx}" y1="{cy}" x2="{x:.1}" y2="{y:.1}" stroke="#1e2330" stroke-width="1" />"##)
        })
        .collect();

    let data_pts: Vec<(f64, f64, f64)> = moods
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let ratio = if has_any && m.streams > 0 {
                (m.streams as f64 / max_streams).max(0.14)
            } else {
                0.05
            };
            point_at(i, r_max * ratio)
        })
        .collect();

    let poly_str: String = data_pts
        .iter()
        .map(|(x, y, _)| format!("{x:.1},{y:.1}"))
        .collect::<Vec<_>>()
        .join(" ");

    let labels_and_dots: String = moods
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let (dx, dy, _) = data_pts[i];
            let (lx, ly, ang) = point_at(i, r_max + 18.0);
            let cos = ang.cos();
            let anchor = if cos.abs() < 0.25 {
                "middle"
            } else if cos > 0.0 {
                "start"
            } else {
                "end"
            };
            let short_lbl = m
                .label
                .replace(" & Lo-Fi", "")
                .replace(" & Dance", "")
                .replace(" & Indie", "");
            let tip_body = format!(
                "{} ({:.1}%) · {}",
                plural(m.streams, "stream", "streams"),
                m.pct,
                fmt_stats_duration(m.secs)
            );
            let dot_r = if m.streams > 0 { "4.2" } else { "2.5" };
            let dot_fill = if m.streams > 0 { safe_color(&m.c1) } else { "#475569" };
            let lbl_fill = if m.streams > 0 { "#f3f5f8" } else { "#6b7485" };
            format!(
                r##"<circle class="stats-tip-target" cx="{dx:.1}" cy="{dy:.1}" r="{dot_r}" fill="{dot_fill}" stroke="#0c0e13" stroke-width="1.5" data-tip-title="{}" data-tip-body="{}" /><text x="{lx:.1}" y="{ly_t:.1}" text-anchor="{anchor}" fill="{lbl_fill}" font-size="10.5" font-weight="600">{}</text>"##,
                html_escape(&m.label),
                html_escape(&tip_body),
                html_escape(&short_lbl),
                ly_t = ly + 3.5,
            )
        })
        .collect();

    let poly_el = if has_any {
        format!(r##"<polygon points="{poly_str}" fill="rgba(30, 215, 96, 0.22)" stroke="#1ed760" stroke-width="2.1" stroke-linejoin="round" />"##)
    } else {
        String::new()
    };

    format!(r#"<svg class="stats-radar-svg" viewBox="0 0 {w} {h}">{rings}{spokes}{poly_el}{labels_and_dots}</svg>"#)
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

fn cover_url(id: &str) -> String {
    format!("/api/web/cover/{}?v=2", urlencoding::encode(id))
}

fn track_list(tracks: &[crate::stats::PublicTopTrack]) -> String {
    format!(
        r#"<ol class="list">{}</ol>"#,
        tracks
            .iter()
            .map(|t| {
                let art = if t.id.trim().is_empty() {
                    String::new()
                } else {
                    format!(
                        r#"<img class="stats-track-art" src="{}" loading="lazy" alt="" />"#,
                        html_escape(&cover_url(&t.id))
                    )
                };
                format!(
                    r#"<li>{art}<div class="li-main"><div class="li-title">{}</div><div class="li-sub">{}</div></div><div class="li-meta">{}</div></li>"#,
                    html_escape(&t.title),
                    html_escape(&t.artist),
                    plural(t.streams, "play", "plays")
                )
            })
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

fn stats_body(u: &PublicUser, d: &ComputedStats, metric: &str) -> String {
    let metric = if metric == "mins" { "mins" } else { "streams" };
    let range_key = d.range.key();
    let range_label = d.range.label();
    let owner_name = display_name(u);

    let range_chips: String = STATS_RANGES
        .iter()
        .map(|(key, label)| {
            format!(
                r#"<a class="chip{}" href="{}">{}</a>"#,
                if *key == range_key { " active" } else { "" },
                html_escape(&stats_path(&u.username, key, metric)),
                label
            )
        })
        .collect();

    let metric_toggle = format!(
        r#"<div class="stats-seg-toggle" role="group" aria-label="Chart Metric"><a class="stats-seg-btn{}" href="{}">Streams</a><a class="stats-seg-btn{}" href="{}">Minutes</a></div>"#,
        if metric == "streams" { " active" } else { "" },
        html_escape(&stats_path(&u.username, range_key, "streams")),
        if metric == "mins" { " active" } else { "" },
        html_escape(&stats_path(&u.username, range_key, "mins")),
    );

    let mut out = format!(
        r#"<div class="stats-controls-bar"><nav class="chips" aria-label="Time range">{range_chips}</nav>{metric_toggle}</div>"#
    );

    // Listener Persona banner
    let badges_html: String = d
        .persona
        .badges
        .iter()
        .map(|b| format!(r#"<span class="stats-vibe-chip">{}</span>"#, html_escape(b)))
        .collect();
    if !d.persona.title.is_empty() || !d.persona.headline.is_empty() {
        out.push_str(&format!(
            r#"<div class="stats-persona-card"><div class="stats-persona-row"><span class="stats-persona-pill">{}</span>{badges_html}</div>{}</div>"#,
            html_escape(&d.persona.title),
            if d.persona.headline.is_empty() {
                String::new()
            } else {
                format!(r#"<p class="stats-persona-headline">{}</p>"#, html_escape(&d.persona.headline))
            }
        ));
    }

    // 8-Card KPI Grid
    let s = &d.summary;
    let streaks = &d.streaks;
    let lib = &d.library;
    let active_dot = if streaks.streak_active_today {
        r#"<span class="stats-live-dot" title="Active today"></span>"#
    } else {
        ""
    };
    let listen_sub = if s.active_days > 0 {
        format!(
            "{}/active day · {} avg track",
            fmt_stats_duration(s.avg_daily_secs),
            fmt_track_time(s.avg_track_duration_secs)
        )
    } else {
        "No active days in this range".to_string()
    };
    let fidelity_sub = if s.total_streams > 0 {
        if s.avg_bitrate > 0 {
            format!(
                "{} kbps avg · {} of {} streams",
                s.avg_bitrate,
                fmt_count(s.lossless_streams as i64),
                fmt_count(s.total_streams as i64)
            )
        } else {
            format!(
                "{} of {} streams",
                fmt_count(s.lossless_streams as i64),
                fmt_count(s.total_streams as i64)
            )
        }
    } else {
        "No streams logged".to_string()
    };
    let rating_suffix = if lib.rated_tracks > 0 {
        format!(" · {:.1}★", lib.avg_rating)
    } else {
        String::new()
    };

    out.push_str(&format!(
        r#"<section class="stats-kpi-grid">
  <div class="stats-kpi-card accent-green">
    <div class="stats-kpi-label">Listening Time</div>
    <div class="stats-kpi-val">{}</div>
    <div class="stats-kpi-sub">{}</div>
  </div>
  <div class="stats-kpi-card">
    <div class="stats-kpi-label">Total Streams</div>
    <div class="stats-kpi-val">{}</div>
    <div class="stats-kpi-sub">{} completed ({:.0}%)</div>
    <div class="stats-kpi-bar"><span style="width:{:.1}%"></span></div>
  </div>
  <div class="stats-kpi-card accent-blue">
    <div class="stats-kpi-label">Unique Songs</div>
    <div class="stats-kpi-val">{}</div>
    <div class="stats-kpi-sub">{:.1}× avg replay · {} new ({:.0}%)</div>
  </div>
  <div class="stats-kpi-card">
    <div class="stats-kpi-label">Artists &amp; Albums</div>
    <div class="stats-kpi-val">{} <span class="stats-kpi-unit">artists</span></div>
    <div class="stats-kpi-sub">{} albums · {} newly discovered</div>
  </div>
  <div class="stats-kpi-card accent-amber">
    <div class="stats-kpi-label">Daily Streak {active_dot}</div>
    <div class="stats-kpi-val">{}d</div>
    <div class="stats-kpi-sub">Best streak: {}d · {} days all-time</div>
  </div>
  <div class="stats-kpi-card">
    <div class="stats-kpi-label">Completion vs Skips</div>
    <div class="stats-kpi-val">{:.0}% <span class="stats-kpi-unit">skip</span></div>
    <div class="stats-kpi-sub">{} skipped · {} partial plays</div>
  </div>
  <div class="stats-kpi-card accent-purple">
    <div class="stats-kpi-label">Audio Fidelity</div>
    <div class="stats-kpi-val">{:.0}% <span class="stats-kpi-unit">lossless</span></div>
    <div class="stats-kpi-sub">{}</div>
  </div>
  <div class="stats-kpi-card">
    <div class="stats-kpi-label">Library &amp; Taste</div>
    <div class="stats-kpi-val">{} <span class="stats-kpi-unit">likes</span></div>
    <div class="stats-kpi-sub">{} playlists · {} followed{}</div>
  </div>
</section>"#,
        html_escape(&fmt_stats_duration(s.total_listening_secs)),
        html_escape(&listen_sub),
        fmt_count(s.total_streams as i64),
        fmt_count(s.completed_streams as i64),
        s.completion_rate,
        s.completion_rate.clamp(0.0, 100.0),
        fmt_count(s.unique_tracks as i64),
        s.repeat_ratio,
        fmt_count(s.new_tracks_discovered as i64),
        s.discovery_rate_pct,
        fmt_count(s.unique_artists as i64),
        fmt_count(s.unique_albums as i64),
        fmt_count(s.new_artists_discovered as i64),
        streaks.current_days,
        streaks.longest_days,
        fmt_count(streaks.active_days_all_time as i64),
        s.skip_rate,
        fmt_count(s.skipped_streams as i64),
        fmt_count(s.partial_streams as i64),
        s.lossless_pct,
        html_escape(&fidelity_sub),
        fmt_count(lib.liked_tracks as i64),
        fmt_count(lib.manual_playlists as i64),
        fmt_count(lib.followed_artists as i64),
        html_escape(&rating_suffix),
    ));

    if s.total_streams == 0 {
        out.push_str(&empty(&format!("No listening in this period ({range_label}).")));
        return out;
    }

    // Row 1: Listening Timeline + 24-Hour Clock
    let gran_label = match d.timeline.granularity.as_str() {
        "month" => "Monthly",
        "week" => "Weekly",
        _ => "Daily",
    };
    let metric_word = if metric == "mins" { "minutes listened" } else { "streams" };
    let timeline_sub = format!(
        "{gran_label} {metric_word} · {} ({:.1}/day avg)",
        plural(s.active_days, "active day", "active days"),
        s.avg_streams_per_active_day
    );
    let clock_sub = if d.persona.chronotype_desc.is_empty() {
        "Hourly distribution"
    } else {
        &d.persona.chronotype_desc
    };
    let chronotype_badge = if d.persona.chronotype.is_empty() {
        String::new()
    } else {
        format!(r#"<span class="stats-badge green">{}</span>"#, html_escape(&d.persona.chronotype))
    };

    out.push_str(&format!(
        r#"<section class="stats-grid-hero-charts">
  <div class="stats-card">
    <div class="stats-card-head">
      <div><h2>Listening Activity</h2><p class="stats-card-sub">{}</p></div>
      <span class="stats-badge">{gran_label}</span>
    </div>
    <div class="stats-chart-wrap">{}</div>
  </div>
  <div class="stats-card">
    <div class="stats-card-head">
      <div><h2>24-Hour Clock</h2><p class="stats-card-sub">{}</p></div>
      {chronotype_badge}
    </div>
    <div class="stats-clock-wrap">{}</div>
  </div>
</section>"#,
        html_escape(&timeline_sub),
        render_stats_timeline_svg(&d.timeline, metric),
        html_escape(clock_sub),
        render_stats_clock_svg(&d.hourly, metric, d.peak_hour),
    ));

    // 365-Day Listening Consistency Heatmap
    let heatmap_html = render_stats_heatmap(&d.heatmap);
    if !heatmap_html.is_empty() {
        let best_day_text = streaks
            .best_day
            .as_ref()
            .map(|b| {
                format!(
                    "Best day: {} · {} ({})",
                    fmt_short_day_iso(&b.date),
                    plural(b.streams, "stream", "streams"),
                    fmt_stats_duration(b.secs)
                )
            })
            .unwrap_or_else(|| "Consistency calendar".to_string());
        out.push_str(&format!(
            r#"<section class="stats-card">
  <div class="stats-card-head">
    <div><h2>Listening Consistency (Past 365 Days)</h2><p class="stats-card-sub">{}</p></div>
    <div class="stats-inline-chips">
      <span class="stats-badge">Current: {}d</span>
      <span class="stats-badge">Best: {}d</span>
    </div>
  </div>
  {heatmap_html}
</section>"#,
            html_escape(&best_day_text),
            streaks.current_days,
            streaks.longest_days,
        ));
    }

    // Row 2: Mood & Vibe DNA Radar + Weekly Rhythm & Eras
    let mut active_moods: Vec<&MoodStat> = d.moods.iter().filter(|m| m.streams > 0).collect();
    active_moods.sort_by(|a, b| b.streams.cmp(&a.streams));
    let mood_bars_html = if active_moods.is_empty() {
        r#"<p class="muted-center">Mood breakdown appears as tracks are analyzed.</p>"#.to_string()
    } else {
        active_moods
            .iter()
            .take(6)
            .map(|m| {
                format!(
                    r#"<div class="stats-hbar-row"><div class="stats-hbar-top"><span class="stats-hbar-name">{}</span><span class="stats-hbar-val">{} · {:.0}%</span></div><div class="stats-hbar-track"><div class="stats-hbar-fill" style="width:{:.1}%;background:{}"></div></div></div>"#,
                    html_escape(&m.label),
                    fmt_count(m.streams as i64),
                    m.pct,
                    m.pct.max(4.0).min(100.0),
                    safe_color(&m.c1),
                )
            })
            .collect()
    };

    let max_wd = d
        .weekdays
        .iter()
        .map(|w| if metric == "mins" { (w.secs + 30) / 60 } else { w.streams as i64 })
        .max()
        .unwrap_or(0)
        .max(1) as f64;
    let weekday_bars: String = d
        .weekdays
        .iter()
        .map(|w| {
            let v = if metric == "mins" { (w.secs + 30) / 60 } else { w.streams as i64 };
            let h_pct = if v > 0 { (v as f64 / max_wd * 100.0).max(8.0) } else { 3.0 };
            let is_peak = d.peak_weekday.as_deref() == Some(w.full_day.as_str()) && w.streams > 0;
            let tip_body = format!(
                "{} ({:.1}%) · {}",
                plural(w.streams, "stream", "streams"),
                w.pct,
                fmt_stats_duration(w.secs)
            );
            let num = if v > 0 { fmt_count(v) } else { String::new() };
            format!(
                r#"<div class="stats-vbar-col stats-tip-target" data-tip-title="{}" data-tip-body="{}"><div class="stats-vbar-num">{num}</div><div class="stats-vbar-track"><div class="stats-vbar-fill{}" style="height:{h_pct:.1}%"></div></div><div class="stats-vbar-lbl">{}</div></div>"#,
                html_escape(&w.full_day),
                html_escape(&tip_body),
                if is_peak { " peak" } else { "" },
                html_escape(&w.day),
            )
        })
        .collect();

    let decades_html = if d.decades.is_empty() {
        String::new()
    } else {
        let max_dec = d
            .decades
            .iter()
            .map(|dec| if metric == "mins" { (dec.secs + 30) / 60 } else { dec.streams as i64 })
            .max()
            .unwrap_or(0)
            .max(1) as f64;
        let dec_bars: String = d
            .decades
            .iter()
            .map(|dec| {
                let v = if metric == "mins" { (dec.secs + 30) / 60 } else { dec.streams as i64 };
                let h_pct = if v > 0 { (v as f64 / max_dec * 100.0).max(10.0) } else { 4.0 };
                let tip_body = format!(
                    "{} ({:.1}%) · {}",
                    plural(dec.streams, "stream", "streams"),
                    dec.pct,
                    fmt_stats_duration(dec.secs)
                );
                format!(
                    r#"<div class="stats-vbar-col stats-tip-target" data-tip-title="{}" data-tip-body="{}"><div class="stats-vbar-num">{:.0}%</div><div class="stats-vbar-track"><div class="stats-vbar-fill blue" style="height:{h_pct:.1}%"></div></div><div class="stats-vbar-lbl">{}</div></div>"#,
                    html_escape(&dec.label),
                    html_escape(&tip_body),
                    dec.pct,
                    html_escape(&dec.label),
                )
            })
            .collect();
        let footnotes = match (&d.oldest_track, &d.newest_track) {
            (None, None) => String::new(),
            (old, new) => {
                let mut parts = String::new();
                if let Some(o) = old {
                    parts.push_str(&format!(
                        "<span>Oldest: <strong>{}</strong> ({})</span>",
                        o.year,
                        html_escape(&o.title)
                    ));
                }
                if let Some(n) = new {
                    parts.push_str(&format!(
                        "<span>Newest: <strong>{}</strong> ({})</span>",
                        n.year,
                        html_escape(&n.title)
                    ));
                }
                format!(r#"<div class="stats-era-footnotes">{parts}</div>"#)
            }
        };
        format!(
            r#"<div class="stats-subblock" style="margin-top:16px"><div class="stats-subblock-title">Music Eras &amp; Release Decades</div><div class="stats-vbar-chart compact">{dec_bars}</div>{footnotes}</div>"#
        )
    };

    let rhythm_sub = d
        .peak_weekday
        .as_deref()
        .map(|day| format!("Peak day: {day}"))
        .unwrap_or_else(|| "Day-of-week & release decades".to_string());

    out.push_str(&format!(
        r#"<section class="stats-grid-2">
  <div class="stats-card">
    <div class="stats-card-head">
      <div><h2>Mood &amp; Vibe DNA</h2><p class="stats-card-sub">{}'s emotional listening fingerprint across Beatlab's 10 vibes</p></div>
    </div>
    <div class="stats-mood-split">
      <div class="stats-radar-wrap">{}</div>
      <div class="stats-mood-bars">{mood_bars_html}</div>
    </div>
  </div>
  <div class="stats-card">
    <div class="stats-card-head">
      <div><h2>Weekly Rhythm &amp; Eras</h2><p class="stats-card-sub">{}</p></div>
    </div>
    <div class="stats-subblock">
      <div class="stats-subblock-title">Day of Week ({})</div>
      <div class="stats-vbar-chart">{weekday_bars}</div>
    </div>
    {decades_html}
  </div>
</section>"#,
        html_escape(owner_name),
        render_stats_mood_radar_svg(&d.moods),
        html_escape(&rhythm_sub),
        if metric == "mins" { "Minutes" } else { "Streams" },
    ));

    // Row 3: Top Genres
    if !d.genres.is_empty() {
        let max_g = d.genres.iter().map(|g| g.streams).max().unwrap_or(1).max(1) as f64;
        let genre_rows: String = d
            .genres
            .iter()
            .take(10)
            .enumerate()
            .map(|(idx, g)| {
                let w_pct = (g.streams as f64 / max_g * 100.0).max(6.0);
                let bar_color = if idx == 0 {
                    "#1ed760"
                } else if idx < 3 {
                    "#38bdf8"
                } else {
                    "#a855f7"
                };
                format!(
                    r#"<div class="stats-hbar-row"><div class="stats-hbar-top"><span class="stats-hbar-name"><span class="stats-rank-num">#{}</span> {}</span><span class="stats-hbar-val">{} · {} ({:.0}%)</span></div><div class="stats-hbar-track"><div class="stats-hbar-fill" style="width:{w_pct:.1}%;background:{bar_color}"></div></div></div>"#,
                    idx + 1,
                    html_escape(&g.name),
                    fmt_count(g.streams as i64),
                    html_escape(&fmt_stats_duration(g.secs)),
                    g.pct,
                )
            })
            .collect();
        out.push_str(&format!(
            r#"<section class="stats-card"><div class="stats-card-head"><div><h2>Top Genres</h2><p class="stats-card-sub">Genre tags from streamed tracks in {range_label}</p></div></div><div class="stats-genre-list">{genre_rows}</div></section>"#
        ));
    }

    // Row 4: Top Artists & Top Albums Leaderboards
    if !d.top_artists.is_empty() || !d.top_albums.is_empty() {
        let max_a = d.top_artists.iter().map(|a| a.streams).max().unwrap_or(1).max(1) as f64;
        let artists_html = if d.top_artists.is_empty() {
            r#"<p class="muted-center">No artist plays yet.</p>"#.to_string()
        } else {
            let rows: String = d
                .top_artists
                .iter()
                .take(10)
                .enumerate()
                .map(|(idx, a)| {
                    let w_pct = (a.streams as f64 / max_a * 100.0).max(4.0);
                    format!(
                        r#"<div class="stats-lb-row"><span class="stats-lb-rank{}">{}</span><img class="stats-lb-img round" src="{}" loading="lazy" alt="" /><div class="stats-lb-info"><div class="stats-lb-title">{}</div><div class="stats-lb-sub">{} · {}</div><div class="stats-lb-meter"><span style="width:{w_pct:.1}%"></span></div></div><div class="stats-lb-count"><strong>{}</strong><span>{}</span></div></div>"#,
                        if idx < 3 { " top" } else { "" },
                        idx + 1,
                        html_escape(&cover_url(&a.id)),
                        html_escape(&a.name),
                        plural(a.unique_tracks, "track", "tracks"),
                        html_escape(&fmt_stats_duration(a.listened_secs)),
                        fmt_count(a.streams as i64),
                        if a.streams == 1 { "play" } else { "plays" },
                    )
                })
                .collect();
            format!(r#"<div class="stats-lb-list">{rows}</div>"#)
        };

        let max_al = d.top_albums.iter().map(|a| a.streams).max().unwrap_or(1).max(1) as f64;
        let albums_html = if d.top_albums.is_empty() {
            r#"<p class="muted-center">No album plays yet.</p>"#.to_string()
        } else {
            let rows: String = d
                .top_albums
                .iter()
                .take(10)
                .enumerate()
                .map(|(idx, al)| {
                    let w_pct = (al.streams as f64 / max_al * 100.0).max(4.0);
                    let year_part = if al.year > 0 { format!(" · {}", al.year) } else { String::new() };
                    format!(
                        r#"<div class="stats-lb-row"><span class="stats-lb-rank{}">{}</span><img class="stats-lb-img" src="{}" loading="lazy" alt="" /><div class="stats-lb-info"><div class="stats-lb-title">{}</div><div class="stats-lb-sub">{}{} · {}</div><div class="stats-lb-meter"><span style="width:{w_pct:.1}%;background:#38bdf8"></span></div></div><div class="stats-lb-count"><strong>{}</strong><span>{}</span></div></div>"#,
                        if idx < 3 { " top" } else { "" },
                        idx + 1,
                        html_escape(&cover_url(&al.id)),
                        html_escape(&al.name),
                        html_escape(&al.artist),
                        year_part,
                        html_escape(&fmt_stats_duration(al.listened_secs)),
                        fmt_count(al.streams as i64),
                        if al.streams == 1 { "play" } else { "plays" },
                    )
                })
                .collect();
            format!(r#"<div class="stats-lb-list">{rows}</div>"#)
        };

        out.push_str(&format!(
            r#"<section class="stats-grid-2">
  <div class="stats-card">
    <div class="stats-card-head"><div><h2>Top Artists</h2><p class="stats-card-sub">Ranked by stream count in {range_label}</p></div></div>
    {artists_html}
  </div>
  <div class="stats-card">
    <div class="stats-card-head"><div><h2>Top Albums</h2><p class="stats-card-sub">Most spun records in {range_label}</p></div></div>
    {albums_html}
  </div>
</section>"#
        ));
    }

    // Most Played Tracks
    if !d.top_tracks.is_empty() {
        let shown = d.top_tracks.len().min(15);
        let rows: String = d
            .top_tracks
            .iter()
            .take(shown)
            .enumerate()
            .map(|(idx, item)| {
                let t = &item.track;
                let sub = if t.album.trim().is_empty() {
                    html_escape(&t.artist)
                } else {
                    format!("{} · {}", html_escape(&t.artist), html_escape(&t.album))
                };
                format!(
                    r#"<div class="stats-track-row"><span class="stats-lb-rank{}">{}</span><img class="stats-track-art" src="{}" loading="lazy" alt="" /><div class="li-main"><div class="li-title">{}</div><div class="li-sub">{sub}</div></div><span class="stats-play-pill">{} · {}</span><span class="stats-track-dur">{}</span></div>"#,
                    if idx < 3 { " top" } else { "" },
                    idx + 1,
                    html_escape(&cover_url(&t.id)),
                    html_escape(&t.title),
                    plural(item.streams, "play", "plays"),
                    html_escape(&fmt_stats_duration(item.listened_secs)),
                    fmt_track_time(t.duration as i64),
                )
            })
            .collect();
        out.push_str(&format!(
            r#"<section class="stats-card"><div class="stats-card-head"><div><h2>Most Played Tracks</h2><p class="stats-card-sub">{}'s top {shown} heavy rotation songs</p></div></div><div class="stats-track-list">{rows}</div></section>"#,
            html_escape(owner_name),
        ));
    }

    // Milestones & Quirks + Star Ratings
    let hl = &d.highlights;
    let mut ms_tiles = String::new();
    if let Some((ref t, completed)) = hl.most_completed_track {
        ms_tiles.push_str(&format!(
            r#"<div class="stats-ms-tile"><div class="stats-ms-kicker">MOST COMPLETED</div><div class="stats-ms-title">{}</div><div class="stats-ms-sub">{} · {}</div></div>"#,
            html_escape(&t.title),
            html_escape(&t.artist),
            plural(completed, "full listen", "full listens"),
        ));
    }
    if let Some(ref t) = hl.longest_track {
        ms_tiles.push_str(&format!(
            r#"<div class="stats-ms-tile"><div class="stats-ms-kicker">LONGEST TRACK</div><div class="stats-ms-title">{}</div><div class="stats-ms-sub">{} · {}</div></div>"#,
            html_escape(&t.title),
            html_escape(&t.artist),
            fmt_track_time(t.duration as i64),
        ));
    }
    if let Some(ref t) = hl.shortest_track {
        ms_tiles.push_str(&format!(
            r#"<div class="stats-ms-tile"><div class="stats-ms-kicker">SHORTEST TRACK</div><div class="stats-ms-title">{}</div><div class="stats-ms-sub">{} · {}</div></div>"#,
            html_escape(&t.title),
            html_escape(&t.artist),
            fmt_track_time(t.duration as i64),
        ));
    }
    if let Some((ref t, at)) = hl.first_track_ever {
        ms_tiles.push_str(&format!(
            r#"<div class="stats-ms-tile"><div class="stats-ms-kicker">FIRST TRACK LOGGED</div><div class="stats-ms-title">{}</div><div class="stats-ms-sub">{} · {}</div></div>"#,
            html_escape(&t.title),
            html_escape(&t.artist),
            html_escape(&fmt_date_label(at)),
        ));
    }

    let ratings_html = if lib.rated_tracks > 0 {
        let max_r = lib.ratings_dist.iter().copied().max().unwrap_or(1).max(1) as f64;
        let rows: String = [5usize, 4, 3, 2, 1]
            .iter()
            .map(|&star| {
                let cnt = lib.ratings_dist[star - 1];
                let w_pct = if cnt > 0 { (cnt as f64 / max_r * 100.0).max(4.0) } else { 0.0 };
                format!(
                    r#"<div class="stats-rating-row"><span>{star}★</span><div class="stats-hbar-track"><div class="stats-hbar-fill" style="width:{w_pct:.1}%;background:#fbbf24"></div></div><span>{}</span></div>"#,
                    fmt_count(cnt as i64)
                )
            })
            .collect();
        format!(
            r#"<div class="stats-subblock" style="margin-top:16px"><div class="stats-subblock-title">Star Ratings Distribution ({} rated · {:.1}★ avg)</div><div class="stats-ratings-bars">{rows}</div></div>"#,
            fmt_count(lib.rated_tracks as i64),
            lib.avg_rating,
        )
    } else {
        String::new()
    };

    if !ms_tiles.is_empty() || !ratings_html.is_empty() {
        out.push_str(&format!(
            r#"<section class="stats-card"><div class="stats-card-head"><div><h2>Milestones &amp; Quirks</h2><p class="stats-card-sub">Standout moments from {}'s listening history</p></div></div>{}{ratings_html}</section>"#,
            html_escape(owner_name),
            if ms_tiles.is_empty() {
                String::new()
            } else {
                format!(r#"<div class="stats-milestones-grid">{ms_tiles}</div>"#)
            },
        ));
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
    let page_url = match body {
        ProfileBody::Stats { stats, metric } => {
            format!("{}{}", origin, stats_path(&u.username, stats.range.key(), metric))
        }
        _ => format!("{}{}", origin, profile_path(&u.username, tab, None)),
    };
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
        ProfileBody::Stats { stats, metric } => stats_body(u, stats, metric),
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
    let wrap_class = if tab == ProfileTab::Stats { "wrap wide" } else { "wrap" };

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
    .wrap.wide {{ max-width:980px; }}
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
    .stats-controls-bar {{ display:flex; align-items:center; justify-content:space-between; gap:12px; flex-wrap:wrap; margin-top:18px; }}
    .chips {{ display:flex; flex-wrap:wrap; gap:8px; }}
    .chip {{ padding:6px 12px; border-radius:999px; border:1px solid #2d3341; text-decoration:none; font-size:13px; color:#98a2b3; }}
    .chip:hover {{ color:#f3f5f8; border-color:#3f4758; }}
    .chip.active {{ background:#f3f5f8; color:#0c0e13; border-color:#f3f5f8; font-weight:600; }}
    .stats-seg-toggle {{ display:inline-flex; background:#12151c; border:1px solid #232834; border-radius:10px; padding:3px; gap:2px; }}
    .stats-seg-btn {{ padding:5px 11px; border-radius:7px; font-size:12px; font-weight:700; color:#98a2b3; text-decoration:none; }}
    .stats-seg-btn:hover {{ color:#f3f5f8; }}
    .stats-seg-btn.active {{ background:#232834; color:#f3f5f8; }}
    .stats-persona-card {{ margin-top:16px; background:#12151c; border:1px solid #232834; border-radius:12px; padding:14px 16px; display:flex; flex-direction:column; gap:8px; }}
    .stats-persona-row {{ display:flex; flex-wrap:wrap; align-items:center; gap:7px; }}
    .stats-persona-pill {{ font-size:12px; font-weight:800; letter-spacing:.02em; padding:4px 11px; border-radius:999px; background:#1ed760; color:#06120b; }}
    .stats-vibe-chip {{ font-size:11.5px; font-weight:600; padding:3px 9px; border-radius:999px; background:#181c26; border:1px solid #232834; color:#98a2b3; }}
    .stats-persona-headline {{ font-size:13.5px; color:#98a2b3; margin:0; line-height:1.45; }}
    section {{ margin-top:18px; }}
    .sec-head {{ display:flex; align-items:baseline; justify-content:space-between; gap:12px; margin-bottom:12px; }}
    h2 {{ font-size:17px; margin:0; font-weight:700; }}
    .more {{ color:#98a2b3; font-size:13px; text-decoration:none; white-space:nowrap; }}
    .more:hover {{ color:#f3f5f8; }}
    .hl-grid {{ display:grid; grid-template-columns:repeat(auto-fill,minmax(150px,1fr)); gap:10px; }}
    .hl {{ background:#12151c; border:1px solid #232834; border-radius:12px; padding:12px 14px; min-width:0; }}
    .hl span {{ display:block; color:#98a2b3; font-size:12px; }}
    .hl strong {{ display:block; margin-top:2px; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }}
    .stats-kpi-grid {{ display:grid; grid-template-columns:repeat(auto-fit,minmax(200px,1fr)); gap:12px; }}
    .stats-kpi-card {{ background:#12151c; border:1px solid #232834; border-radius:12px; padding:14px 16px; display:flex; flex-direction:column; gap:4px; min-width:0; }}
    .stats-kpi-card.accent-green {{ border-top:2px solid #1ed760; }}
    .stats-kpi-card.accent-blue {{ border-top:2px solid #38bdf8; }}
    .stats-kpi-card.accent-amber {{ border-top:2px solid #fbbf24; }}
    .stats-kpi-card.accent-purple {{ border-top:2px solid #a855f7; }}
    .stats-kpi-label {{ display:flex; align-items:center; gap:6px; font-size:11px; font-weight:700; letter-spacing:.04em; text-transform:uppercase; color:#6b7485; }}
    .stats-kpi-val {{ font-size:24px; font-weight:800; letter-spacing:-.02em; font-variant-numeric:tabular-nums; line-height:1.2; }}
    .stats-kpi-unit {{ font-size:13px; font-weight:600; color:#98a2b3; }}
    .stats-kpi-sub {{ font-size:12px; color:#98a2b3; line-height:1.4; }}
    .stats-kpi-bar {{ margin-top:5px; height:4px; border-radius:999px; background:#232834; overflow:hidden; }}
    .stats-kpi-bar span {{ display:block; height:100%; background:#1ed760; border-radius:999px; }}
    .stats-live-dot {{ width:7px; height:7px; border-radius:50%; background:#1ed760; box-shadow:0 0 8px #1ed760; display:inline-block; }}
    .stats-card {{ background:#12151c; border:1px solid #232834; border-radius:12px; padding:18px; min-width:0; }}
    .stats-card-head {{ display:flex; align-items:flex-start; justify-content:space-between; gap:12px; flex-wrap:wrap; margin-bottom:14px; }}
    .stats-card-sub {{ font-size:12.5px; color:#98a2b3; margin:2px 0 0; }}
    .stats-inline-chips {{ display:flex; gap:6px; flex-wrap:wrap; }}
    .stats-badge {{ font-size:11px; font-weight:700; padding:3px 9px; border-radius:999px; background:#181c26; border:1px solid #232834; color:#98a2b3; white-space:nowrap; }}
    .stats-badge.green {{ background:rgba(30,215,96,.12); border-color:rgba(30,215,96,.35); color:#1ed760; }}
    .stats-grid-hero-charts {{ display:grid; grid-template-columns:minmax(0,1.65fr) minmax(260px,1fr); gap:14px; }}
    .stats-grid-2 {{ display:grid; grid-template-columns:repeat(2,minmax(0,1fr)); gap:14px; }}
    .stats-chart-wrap {{ width:100%; overflow:hidden; }}
    .stats-svg-chart {{ width:100%; height:auto; display:block; }}
    .stats-hit-col {{ cursor:pointer; }}
    .stats-hit-col:hover {{ fill:rgba(255,255,255,.04); }}
    .stats-clock-wrap {{ display:flex; justify-content:center; align-items:center; }}
    .stats-clock-svg {{ width:100%; max-width:268px; height:auto; display:block; }}
    .stats-clock-hit {{ cursor:pointer; }}
    .stats-clock-hit:hover {{ fill:rgba(255,255,255,.06); }}
    .stats-hm-scroll {{ overflow-x:auto; padding-bottom:6px; }}
    .stats-hm-inner {{ width:max-content; display:flex; flex-direction:column; gap:4px; }}
    .stats-hm-months-row, .stats-hm-body {{ display:flex; align-items:flex-start; gap:6px; }}
    .stats-hm-months-row {{ align-items:center; }}
    .stats-hm-wd-spacer {{ width:28px; flex-shrink:0; }}
    .stats-hm-months {{ display:grid; gap:3px; }}
    .stats-hm-month {{ font-size:10px; color:#6b7485; font-weight:600; white-space:nowrap; overflow:visible; min-width:0; line-height:1.2; }}
    .stats-hm-weekdays {{ width:28px; flex-shrink:0; display:grid; grid-template-rows:repeat(7,12px); gap:3px; font-size:9.5px; color:#6b7485; align-items:center; line-height:1; }}
    .stats-hm-grid {{ display:grid; gap:3px; }}
    .stats-hm-col {{ display:grid; grid-template-rows:repeat(7,12px); gap:3px; }}
    .stats-hm-cell {{ width:12px; height:12px; border-radius:2px; background:#181c26; border:1px solid rgba(255,255,255,.03); }}
    .stats-hm-cell.empty {{ background:transparent; border-color:transparent; }}
    .stats-hm-cell.lv-0 {{ background:#181c26; }}
    .stats-hm-cell.lv-1 {{ background:#0e4429; }}
    .stats-hm-cell.lv-2 {{ background:#147a3b; }}
    .stats-hm-cell.lv-3 {{ background:#19b454; }}
    .stats-hm-cell.lv-4 {{ background:#1ed760; }}
    .stats-hm-cell.stats-tip-target {{ cursor:pointer; }}
    .stats-hm-cell.stats-tip-target:hover {{ outline:1.5px solid #f3f5f8; }}
    .stats-hm-legend {{ display:flex; align-items:center; justify-content:flex-end; gap:4px; margin-top:8px; font-size:11px; color:#6b7485; }}
    .stats-hm-legend .stats-hm-cell {{ width:11px; height:11px; }}
    .stats-mood-split {{ display:grid; grid-template-columns:minmax(200px,1.15fr) minmax(0,1fr); gap:14px; align-items:center; }}
    .stats-radar-wrap {{ display:flex; justify-content:center; }}
    .stats-radar-svg {{ width:100%; max-width:340px; height:auto; display:block; }}
    .stats-mood-bars {{ display:flex; flex-direction:column; gap:10px; }}
    .stats-genre-list {{ display:grid; grid-template-columns:repeat(2,minmax(0,1fr)); gap:12px 24px; }}
    .stats-hbar-row {{ display:flex; flex-direction:column; gap:4px; }}
    .stats-hbar-top {{ display:flex; justify-content:space-between; align-items:center; gap:8px; font-size:12.5px; }}
    .stats-hbar-name {{ font-weight:600; white-space:nowrap; overflow:hidden; text-overflow:ellipsis; }}
    .stats-rank-num {{ color:#6b7485; font-variant-numeric:tabular-nums; margin-right:3px; }}
    .stats-hbar-val {{ font-size:11.5px; color:#98a2b3; font-variant-numeric:tabular-nums; flex-shrink:0; }}
    .stats-hbar-track {{ height:6px; border-radius:999px; background:#232834; overflow:hidden; }}
    .stats-hbar-fill {{ height:100%; border-radius:999px; }}
    .stats-subblock-title {{ font-size:11px; font-weight:700; letter-spacing:.05em; text-transform:uppercase; color:#6b7485; margin-bottom:10px; }}
    .stats-vbar-chart {{ display:grid; grid-template-columns:repeat(7,minmax(0,1fr)); gap:8px; height:132px; align-items:stretch; }}
    .stats-vbar-chart.compact {{ grid-template-columns:repeat(auto-fit,minmax(36px,1fr)); height:108px; }}
    .stats-vbar-col {{ display:flex; flex-direction:column; align-items:center; gap:5px; cursor:pointer; }}
    .stats-vbar-num {{ font-size:10.5px; color:#98a2b3; font-variant-numeric:tabular-nums; min-height:15px; }}
    .stats-vbar-track {{ flex:1; width:100%; max-width:30px; background:#181c26; border-radius:4px; display:flex; align-items:flex-end; overflow:hidden; }}
    .stats-vbar-fill {{ width:100%; background:rgba(30,215,96,.48); border-radius:4px 4px 0 0; }}
    .stats-vbar-fill.peak {{ background:#1ed760; }}
    .stats-vbar-fill.blue {{ background:rgba(56,189,248,.7); }}
    .stats-vbar-lbl {{ font-size:11px; font-weight:600; color:#98a2b3; }}
    .stats-era-footnotes {{ display:flex; flex-wrap:wrap; gap:8px 16px; margin-top:10px; font-size:12px; color:#98a2b3; }}
    .stats-lb-list {{ display:flex; flex-direction:column; gap:6px; }}
    .stats-lb-row {{ display:flex; align-items:center; gap:11px; padding:7px 8px; border-radius:8px; }}
    .stats-lb-row:hover {{ background:#181c26; }}
    .stats-lb-rank {{ width:22px; text-align:center; font-size:13px; font-weight:700; color:#6b7485; font-variant-numeric:tabular-nums; flex-shrink:0; }}
    .stats-lb-rank.top {{ color:#1ed760; }}
    .stats-lb-img {{ width:44px; height:44px; border-radius:6px; object-fit:cover; flex-shrink:0; background:#181c26; border:1px solid #232834; }}
    .stats-lb-img.round {{ border-radius:50%; }}
    .stats-lb-info {{ flex:1; min-width:0; display:flex; flex-direction:column; gap:3px; }}
    .stats-lb-title {{ font-size:13.5px; font-weight:600; white-space:nowrap; overflow:hidden; text-overflow:ellipsis; }}
    .stats-lb-sub {{ font-size:11.5px; color:#98a2b3; white-space:nowrap; overflow:hidden; text-overflow:ellipsis; }}
    .stats-lb-meter {{ height:4px; border-radius:999px; background:#232834; overflow:hidden; }}
    .stats-lb-meter span {{ display:block; height:100%; background:#1ed760; border-radius:999px; }}
    .stats-lb-count {{ text-align:right; flex-shrink:0; display:flex; flex-direction:column; line-height:1.15; }}
    .stats-lb-count strong {{ font-size:14px; font-variant-numeric:tabular-nums; }}
    .stats-lb-count span {{ font-size:10.5px; color:#6b7485; }}
    .stats-track-list {{ display:flex; flex-direction:column; }}
    .stats-track-row {{ display:flex; align-items:center; gap:12px; padding:9px 8px; border-top:1px solid #1b1f29; }}
    .stats-track-row:first-child {{ border-top:0; }}
    .stats-track-art {{ width:40px; height:40px; border-radius:6px; object-fit:cover; flex-shrink:0; background:#181c26; border:1px solid #232834; }}
    .stats-play-pill {{ font-size:11px; font-weight:600; padding:3px 8px; border-radius:999px; background:#181c26; border:1px solid #232834; color:#98a2b3; white-space:nowrap; font-variant-numeric:tabular-nums; flex-shrink:0; }}
    .stats-track-dur {{ font-size:12px; color:#6b7485; font-variant-numeric:tabular-nums; min-width:36px; text-align:right; flex-shrink:0; }}
    .stats-milestones-grid {{ display:grid; grid-template-columns:repeat(auto-fit,minmax(190px,1fr)); gap:10px; }}
    .stats-ms-tile {{ background:#181c26; border:1px solid #232834; border-radius:8px; padding:11px 12px; min-width:0; }}
    .stats-ms-kicker {{ font-size:10px; font-weight:700; letter-spacing:.06em; color:#1ed760; margin-bottom:3px; }}
    .stats-ms-title {{ font-size:13.5px; font-weight:700; overflow-wrap:break-word; line-height:1.35; }}
    .stats-ms-sub {{ font-size:11.5px; color:#98a2b3; overflow-wrap:break-word; line-height:1.35; margin-top:2px; }}
    .stats-ratings-bars {{ display:flex; flex-direction:column; gap:6px; }}
    .stats-rating-row {{ display:grid; grid-template-columns:30px minmax(0,1fr) 36px; align-items:center; gap:8px; font-size:12px; color:#98a2b3; font-variant-numeric:tabular-nums; }}
    .muted-center {{ color:#98a2b3; font-size:13px; text-align:center; padding:18px 0; margin:0; }}
    .stats-tooltip {{ position: fixed; z-index:999; pointer-events:none; background:#181c26; border:1px solid #323949; border-radius:8px; padding:7px 10px; box-shadow:0 8px 24px rgba(0,0,0,.5); opacity:0; transition:opacity .1s; max-width:260px; }}
    .stats-tooltip.visible {{ opacity:1; }}
    .stats-tip-title {{ font-size:12px; font-weight:700; color:#f3f5f8; }}
    .stats-tip-body {{ font-size:11.5px; color:#98a2b3; margin-top:2px; }}
    .list {{ list-style:none; margin:0; padding:0; background:#12151c; border:1px solid #232834; border-radius:12px; overflow:hidden; counter-reset:n; }}
    .list li {{ display:flex; align-items:center; gap:12px; padding:12px 14px; border-top:1px solid #1b1f29; }}
    .list li:first-child {{ border-top:0; }}
    ol.list li::before {{ counter-increment:n; content:counter(n); color:#98a2b3; min-width:18px; text-align:right; font-variant-numeric:tabular-nums; }}
    .li-main {{ flex:1; min-width:0; text-decoration:none; }}
    .li-title {{ display:block; font-weight:600; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }}
    .li-sub, .li-meta {{ color:#98a2b3; font-size:13px; }}
    .li-sub {{ display:block; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }}
    .li-meta {{ flex-shrink:0; text-align:right; }}
    .person {{ display:flex; align-items:center; gap:12px; }}
    .person-text {{ min-width:0; }}
    .mini-av {{ width:40px; height:40px; border-radius:50%; display:grid; place-items:center; color:#06120b; font-weight:800; overflow:hidden; flex-shrink:0; }}
    .empty {{ margin-top:24px; padding:28px 16px; text-align:center; color:#98a2b3; background:#12151c; border:1px solid #232834; border-radius:12px; }}
    .note {{ color:#6b7485; font-size:13px; margin-top:10px; }}
    .foot {{ margin-top:36px; text-align:center; color:#6b7485; font-size:13px; }}
    .foot a {{ color:#98a2b3; }}
    @media (max-width:780px) {{
      .stats-grid-hero-charts, .stats-grid-2, .stats-mood-split, .stats-genre-list {{ grid-template-columns:1fr; }}
    }}
    @media (max-width:480px) {{
      .hero {{ padding:18px; }}
      .avatar {{ width:88px; height:88px; font-size:36px; }}
      h1 {{ font-size:26px; }}
      .li-meta {{ font-size:12px; }}
      .stats-track-dur {{ display:none; }}
    }}
  </style>
</head>
<body>
  <main class="{wrap_class}">
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
  <div id="stats-tooltip" class="stats-tooltip" role="tooltip"></div>
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
      var tip = document.getElementById('stats-tooltip');
      if (tip) {{
        var hide = function () {{ tip.classList.remove('visible'); }};
        var show = function (el, x, y) {{
          var t = el.getAttribute('data-tip-title') || '';
          var b = el.getAttribute('data-tip-body') || '';
          if (!t && !b) {{ hide(); return; }}
          tip.innerHTML = '';
          if (t) {{ var d1 = document.createElement('div'); d1.className = 'stats-tip-title'; d1.textContent = t; tip.appendChild(d1); }}
          if (b) {{ var d2 = document.createElement('div'); d2.className = 'stats-tip-body'; d2.textContent = b; tip.appendChild(d2); }}
          tip.classList.add('visible');
          var r = tip.getBoundingClientRect();
          var left = Math.min(Math.max(8, x + 12), window.innerWidth - r.width - 8);
          var top = y - r.height - 12 < 8 ? y + 16 : y - r.height - 12;
          tip.style.left = left + 'px';
          tip.style.top = top + 'px';
        }};
        document.addEventListener('pointermove', function (e) {{
          var t = e.target && e.target.closest ? e.target.closest('.stats-tip-target') : null;
          if (t) show(t, e.clientX, e.clientY); else hide();
        }});
        document.addEventListener('click', function (e) {{
          var t = e.target && e.target.closest ? e.target.closest('.stats-tip-target') : null;
          if (t) show(t, e.clientX, e.clientY); else hide();
        }});
        window.addEventListener('scroll', hide, {{ passive: true }});
      }}
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
        wrap_class = wrap_class,
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
    use crate::models::Track;
    use crate::stats::{
        BestDayStat, DecadeStat, GenreStat, HeatmapDay, HighlightsData, LibraryStats, PersonaStats,
        PublicTopTrack, StatsRange, StreakStats, SummaryStats, TimelinePoint, TopAlbumStat,
        TopArtistStat, TopTrackStat, WeekdayStat,
    };

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

    fn computed_stats() -> ComputedStats {
        let hourly = (0..24)
            .map(|h| HourStat {
                hour: h,
                label: fmt_hour_long(h),
                streams: if h == 22 { 12 } else { 1 },
                secs: if h == 22 { 2400 } else { 180 },
                pct: if h == 22 { 34.3 } else { 2.9 },
            })
            .collect();
        let weekdays = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
            .iter()
            .enumerate()
            .map(|(i, d)| WeekdayStat {
                day: (*d).into(),
                full_day: format!("{d}day"),
                streams: if i == 4 { 15 } else { 4 },
                secs: if i == 4 { 3000 } else { 700 },
                pct: if i == 4 { 38.5 } else { 10.2 },
            })
            .collect();
        ComputedStats {
            range: StatsRange::Days30,
            tz_offset_min: 0,
            generated_at: 1_750_000_000,
            member_since: 1_700_000_000,
            first_listen_at: Some(1_700_100_000),
            last_listen_at: Some(1_750_000_000),
            summary: SummaryStats {
                total_streams: 42,
                completed_streams: 36,
                skipped_streams: 4,
                partial_streams: 2,
                completion_rate: 85.7,
                skip_rate: 9.5,
                total_listening_secs: 7200,
                avg_daily_secs: 800,
                avg_streams_per_active_day: 4.7,
                avg_track_duration_secs: 205,
                unique_tracks: 30,
                unique_artists: 12,
                unique_albums: 20,
                repeat_ratio: 1.4,
                active_days: 9,
                span_days: 30,
                new_tracks_discovered: 10,
                new_artists_discovered: 3,
                discovery_rate_pct: 33.3,
                lossless_streams: 21,
                lossless_pct: 50.0,
                avg_bitrate: 920,
            },
            streaks: StreakStats {
                current_days: 2,
                longest_days: 4,
                streak_active_today: true,
                active_days_all_time: 50,
                best_day: Some(BestDayStat {
                    date: "2026-05-01".into(),
                    streams: 12,
                    secs: 2400,
                }),
            },
            library: LibraryStats {
                liked_tracks: 18,
                saved_albums: 5,
                followed_artists: 7,
                manual_playlists: 3,
                auto_playlists: 1,
                curated_tracks: 40,
                hidden_tracks: 0,
                rated_tracks: 0,
                avg_rating: 0.0,
                ratings_dist: [0; 5],
            },
            persona: PersonaStats {
                title: "Night Owl Explorer".into(),
                chronotype: "Night Owl".into(),
                chronotype_desc: "Most active late at night".into(),
                style: "Explorer".into(),
                headline: "Late-night synth-pop sessions.".into(),
                badges: vec!["Lossless Fan".into()],
            },
            timeline: TimelineStats {
                granularity: "day".into(),
                points: vec![
                    TimelinePoint {
                        key: "2026-05-01".into(),
                        label: "May 1, 2026".into(),
                        short_label: "May 1".into(),
                        streams: 12,
                        completed: 10,
                        skipped: 1,
                        secs: 2400,
                    },
                    TimelinePoint {
                        key: "2026-05-02".into(),
                        label: "May 2, 2026".into(),
                        short_label: "May 2".into(),
                        streams: 8,
                        completed: 7,
                        skipped: 1,
                        secs: 1600,
                    },
                ],
            },
            heatmap: HeatmapStats {
                start_date: "2026-04-20".into(),
                end_date: "2026-05-03".into(),
                max_streams: 12,
                days: vec![HeatmapDay {
                    date: "2026-05-01".into(),
                    streams: 12,
                    secs: 2400,
                }],
            },
            hourly,
            peak_hour: Some(22),
            weekdays,
            peak_weekday: Some("Friday".into()),
            moods: vec![
                MoodStat {
                    slug: "chill".into(),
                    label: "Chill & Lo-Fi".into(),
                    c1: "#38bdf8".into(),
                    c2: "#0ea5e9".into(),
                    streams: 15,
                    secs: 3000,
                    pct: 45.0,
                },
                MoodStat {
                    slug: "party".into(),
                    label: "Party & Dance".into(),
                    c1: "#f472b6".into(),
                    c2: "#ec4899".into(),
                    streams: 5,
                    secs: 900,
                    pct: 15.0,
                },
                MoodStat {
                    slug: "focus".into(),
                    label: "Focus".into(),
                    c1: "#a855f7".into(),
                    c2: "#7c3aed".into(),
                    streams: 0,
                    secs: 0,
                    pct: 0.0,
                },
            ],
            genres: vec![GenreStat {
                name: "J-Pop".into(),
                streams: 20,
                secs: 4000,
                pct: 47.6,
                unique_tracks: 12,
            }],
            decades: vec![DecadeStat {
                decade: 2020,
                label: "2020s".into(),
                streams: 30,
                secs: 6000,
                pct: 71.4,
            }],
            oldest_track: None,
            newest_track: None,
            top_tracks: vec![TopTrackStat {
                track: Track {
                    id: "trk:42".into(),
                    title: "Say It.".into(),
                    artist: "Yorushika".into(),
                    album: "Natsukusa".into(),
                    duration: 242,
                    ..Track::default()
                },
                streams: 9,
                completed: 8,
                skips: 1,
                listened_secs: 2100,
                last_played: 1_750_000_000,
            }],
            top_artists: vec![TopArtistStat {
                id: "ar:1".into(),
                name: "Yorushika <3".into(),
                streams: 10,
                listened_secs: 2100,
                unique_tracks: 6,
                pct: 23.8,
                is_followed: false,
            }],
            top_albums: vec![TopAlbumStat {
                id: "al:7".into(),
                name: "Natsukusa".into(),
                artist: "Yorushika".into(),
                artist_id: "ar:1".into(),
                year: 2017,
                streams: 9,
                listened_secs: 2100,
                unique_tracks: 4,
                is_saved: false,
            }],
            most_skipped: vec![],
            highlights: HighlightsData {
                longest_track: None,
                shortest_track: None,
                first_track_ever: None,
                most_completed_track: None,
            },
            recent_events: vec![],
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
    fn renders_stats_tab_with_graphs_and_range_links() {
        let s = computed_stats();
        let html = render_profile_page(
            "https://beat.example",
            &user(),
            &ProfileBody::Stats { stats: &s, metric: "streams" },
        );
        assert!(html.contains(r#"content="https://beat.example/u/emily/stats?range=30d""#));
        assert!(html.contains(r#"class="chip active" href="/u/emily/stats?range=30d""#));
        // "All Time" is the canonical, query-less stats URL.
        assert!(html.contains(r#"href="/u/emily/stats">All Time"#));
        // Metric toggle preserves the selected range.
        assert!(html.contains(r#"href="/u/emily/stats?range=30d&amp;metric=mins">Minutes"#));
        assert!(html.contains("Yorushika &lt;3"));
        assert!(html.contains("2h"));
        assert!(html.contains("/#user/emily/stats"));
        // Graphs & personal-style sections are rendered.
        assert!(html.contains("stats-svg-chart"));
        assert!(html.contains("stats-clock-svg"));
        assert!(html.contains("stats-hm-grid"));
        assert!(html.contains("stats-radar-svg"));
        assert!(html.contains("stats-vbar-chart"));
        assert!(html.contains("Night Owl Explorer"));
        // Cover art is rendered for tracks, artists, and albums.
        assert!(html.contains(r#"src="/api/web/cover/trk%3A42?v=2""#));
        assert!(html.contains(r#"src="/api/web/cover/ar%3A1?v=2""#));
        assert!(html.contains(r#"src="/api/web/cover/al%3A7?v=2""#));
    }

    #[test]
    fn overview_top_tracks_render_cover_art() {
        let stats = PublicStats {
            top_tracks: vec![PublicTopTrack {
                id: "trk:99".into(),
                title: "Hitchcock".into(),
                artist: "Yorushika".into(),
                streams: 14,
            }],
            ..PublicStats::default()
        };
        let html = render_profile_page(
            "https://beat.example",
            &user(),
            &ProfileBody::Overview { stats: &stats, playlists: &[] },
        );
        assert!(html.contains(r#"class="stats-track-art" src="/api/web/cover/trk%3A99?v=2""#));
    }

    #[test]
    fn heatmap_skips_colliding_first_month_label() {
        let hm = HeatmapStats {
            start_date: "2025-11-24".into(),
            end_date: "2026-11-23".into(),
            max_streams: 10,
            days: vec![],
        };
        let html = render_stats_heatmap(&hm);
        assert!(!html.contains(r#"<div class="stats-hm-month">Nov</div><div class="stats-hm-month">Dec</div>"#));
        assert!(html.contains("grid-template-columns:repeat(53,12px)"));
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
        assert!(html.contains("2 listeners without a public profile are not listed."));
    }
}
