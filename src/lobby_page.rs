//! Public listener page for "Listen Together" lobbies (`/l/{code}`).
//!
//! Self-contained HTML/CSS/JS so friends without a Beatlab account can join
//! from any browser. Visually it follows the main web player (Geist, flat
//! surfaces, pill buttons, light/dark themes).

use crate::share_page::html_escape;
use serde_json::{json, Value};

fn safe_json_for_script(val: &Value) -> String {
    val.to_string()
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

fn host_display(snap: &Value) -> String {
    let host = &snap["host"];
    let name = host["displayName"].as_str().unwrap_or("").trim();
    if name.is_empty() {
        host["username"].as_str().unwrap_or("A friend").to_string()
    } else {
        name.to_string()
    }
}

/// Builds `(page title, og:title, og:description)` for link previews.
fn page_meta(status: &str, snap: &Value) -> (String, String, String) {
    match status {
        "live" => {
            let host = host_display(snap);
            let lobby = snap["name"].as_str().unwrap_or("Listening lobby");
            let count = snap["listenerCount"].as_u64().unwrap_or(0);
            let mut parts = vec![lobby.to_string()];
            if let Some(t) = snap["playback"]["track"].as_object() {
                let title = t.get("title").and_then(Value::as_str).unwrap_or("");
                let artist = t.get("artist").and_then(Value::as_str).unwrap_or("");
                if !title.is_empty() {
                    parts.push(format!("Now playing: {title} — {artist}"));
                }
            }
            if count > 0 {
                parts.push(format!("{count} listening"));
            }
            parts.push("Join in your browser, no account needed".to_string());
            (
                format!("{lobby} · Listen Together"),
                format!("Listen along with {host} on Beatlab"),
                parts.join(" · "),
            )
        }
        "ended" => (
            "Lobby ended".to_string(),
            "This listening lobby has ended".to_string(),
            "Start your own Listen Together lobby on Beatlab.".to_string(),
        ),
        _ => (
            "Lobby not found".to_string(),
            "Listening lobby not found".to_string(),
            "This Listen Together link doesn't exist or has expired.".to_string(),
        ),
    }
}

pub fn render_lobby_html(origin: &str, code: &str, status: &str, snap: &Value) -> String {
    let (page_title, og_title, og_desc) = page_meta(status, snap);
    let page_url = if code.is_empty() {
        origin.to_string()
    } else {
        format!("{origin}/l/{code}")
    };
    let image = if status == "live" {
        format!("{origin}/l/{code}/cover")
    } else {
        format!("{origin}/web/icon-512.png")
    };
    let boot = json!({
        "code": code,
        "status": status,
        "state": if status == "live" { snap.clone() } else { Value::Null },
    });
    TEMPLATE
        .replace("__PAGE_TITLE__", &html_escape(&page_title))
        .replace("__OG_TITLE__", &html_escape(&og_title))
        .replace("__OG_DESC__", &html_escape(&og_desc))
        .replace("__OG_URL__", &html_escape(&page_url))
        .replace("__OG_IMAGE__", &html_escape(&image))
        .replace("__BOOT_JSON__", &safe_json_for_script(&boot))
}

const TEMPLATE: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0, viewport-fit=cover" />
  <meta name="theme-color" content="#121214" />
  <title>__PAGE_TITLE__ — Beatlab</title>
  <link rel="icon" type="image/svg+xml" href="/web/favicon.svg" />
  <link rel="apple-touch-icon" sizes="180x180" href="/web/apple-touch-icon.png" />
  <link rel="preconnect" href="https://fonts.googleapis.com" />
  <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin />
  <link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Geist:wght@400;500;600;700;800&display=swap" />
  <meta property="og:site_name" content="Beatlab" />
  <meta property="og:type" content="website" />
  <meta property="og:title" content="__OG_TITLE__" />
  <meta property="og:description" content="__OG_DESC__" />
  <meta property="og:url" content="__OG_URL__" />
  <meta property="og:image" content="__OG_IMAGE__" />
  <meta property="og:image:width" content="600" />
  <meta property="og:image:height" content="600" />
  <meta name="twitter:card" content="summary" />
  <meta name="twitter:title" content="__OG_TITLE__" />
  <meta name="twitter:description" content="__OG_DESC__" />
  <meta name="twitter:image" content="__OG_IMAGE__" />
  <script>try{if(localStorage.getItem('beatlab_theme')==='light')document.documentElement.dataset.theme='light'}catch(e){}</script>
  <style>
    :root {
      --bg: #121214;
      --surface: #1b1b1e;
      --surface-2: #242428;
      --surface-3: #303035;
      --border: #232327;
      --border-bright: #34343a;
      --text: #f4f4f5;
      --text-muted: #a0a0ab;
      --text-dim: #6c6c78;
      --accent: #1ed760;
      --accent-hover: #3be477;
      --accent-soft: rgba(30, 215, 96, 0.12);
      --accent-ink: #04120a;
      --accent-blue: #38bdf8;
      --danger: #f43f5e;
      --tint-opacity: 0.34;
      --shadow: 0 24px 60px rgba(0, 0, 0, 0.45);
      --font: "Geist", system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
      color-scheme: dark;
    }
    html[data-theme="light"] {
      --bg: #ffffff;
      --surface: #f4f4f5;
      --surface-2: #ececef;
      --surface-3: #e0e0e5;
      --border: #ececef;
      --border-bright: #dadade;
      --text: #111114;
      --text-muted: #5a5a66;
      --text-dim: #8a8a96;
      --accent: #16a34a;
      --accent-hover: #15803d;
      --accent-soft: rgba(22, 163, 74, 0.1);
      --accent-ink: #ffffff;
      --accent-blue: #0284c7;
      --danger: #e11d48;
      --tint-opacity: 0.16;
      --shadow: 0 18px 44px rgba(15, 20, 28, 0.18);
      color-scheme: light;
    }
    * { box-sizing: border-box; margin: 0; padding: 0; }
    html, body {
      min-height: 100%;
      background: var(--bg);
      color: var(--text);
      font-family: var(--font);
      font-size: 14px;
      line-height: 1.45;
      -webkit-font-smoothing: antialiased;
      -webkit-tap-highlight-color: transparent;
    }
    body { overflow-x: hidden; }
    a { color: inherit; text-decoration: none; }
    button, input { font-family: inherit; color: inherit; }
    button { cursor: pointer; border: 0; background: none; touch-action: manipulation; }
    input[type="range"] { accent-color: var(--accent); }
    .hidden { display: none !important; }
    ::-webkit-scrollbar { width: 8px; height: 8px; }
    ::-webkit-scrollbar-thumb { background: var(--border-bright); border-radius: 4px; }

    .bg-tint {
      position: fixed;
      inset: -10%;
      z-index: 0;
      background-size: cover;
      background-position: center;
      filter: blur(72px) saturate(1.5);
      opacity: 0;
      transition: opacity 0.8s ease, background-image 0.8s ease;
      pointer-events: none;
    }
    .bg-tint.on { opacity: var(--tint-opacity); }

    .shell {
      position: relative;
      z-index: 1;
      max-width: 1180px;
      margin: 0 auto;
      padding: 0 20px 28px;
      min-height: 100vh;
      min-height: 100dvh;
      display: flex;
      flex-direction: column;
    }
    .topbar {
      height: 64px;
      flex-shrink: 0;
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 12px;
    }
    .brand { display: inline-flex; align-items: center; gap: 10px; font-weight: 800; font-size: 17px; letter-spacing: -0.02em; }
    .brand svg { width: 28px; height: 28px; display: block; }
    .top-right { display: flex; align-items: center; gap: 8px; min-width: 0; }
    .live-pill {
      display: inline-flex;
      align-items: center;
      gap: 7px;
      height: 30px;
      padding: 0 12px;
      border-radius: 999px;
      background: var(--accent-soft);
      color: var(--accent);
      font-size: 11.5px;
      font-weight: 800;
      letter-spacing: 0.06em;
      white-space: nowrap;
    }
    .live-pill.off { background: var(--surface-2); color: var(--text-muted); }
    .live-dot {
      width: 7px;
      height: 7px;
      border-radius: 50%;
      background: currentColor;
      box-shadow: 0 0 0 0 currentColor;
      animation: livePulse 1.8s ease-out infinite;
    }
    .live-pill.off .live-dot { animation: none; }
    @keyframes livePulse {
      0% { box-shadow: 0 0 0 0 rgba(30, 215, 96, 0.55); }
      70% { box-shadow: 0 0 0 7px rgba(30, 215, 96, 0); }
      100% { box-shadow: 0 0 0 0 rgba(30, 215, 96, 0); }
    }

    .btn-primary, .btn-secondary {
      display: inline-flex;
      align-items: center;
      justify-content: center;
      gap: 8px;
      border-radius: 999px;
      font-weight: 700;
      font-size: 13.5px;
      padding: 9px 18px;
      white-space: nowrap;
      transition: background 0.12s, color 0.12s, transform 0.1s, opacity 0.12s;
    }
    .btn-primary { background: var(--accent); color: var(--accent-ink); }
    .btn-primary:hover { background: var(--accent-hover); }
    .btn-primary:active { transform: scale(0.98); }
    .btn-primary:disabled { opacity: 0.6; cursor: wait; transform: none; }
    .btn-secondary { background: var(--surface-2); color: var(--text); font-weight: 600; }
    .btn-secondary:hover { background: var(--surface-3); }
    .btn-lg { padding: 13px 24px; font-size: 15px; }
    .btn-sm { padding: 6px 12px; font-size: 12.5px; }
    .btn-primary svg, .btn-secondary svg { width: 18px; height: 18px; flex-shrink: 0; }
    .icon-btn {
      width: 38px;
      height: 38px;
      border-radius: 999px;
      display: inline-flex;
      align-items: center;
      justify-content: center;
      color: var(--text-muted);
      flex-shrink: 0;
      transition: background 0.12s, color 0.12s;
    }
    .icon-btn:hover { background: var(--surface-2); color: var(--text); }
    .icon-btn svg { width: 20px; height: 20px; display: block; }

    .avatar {
      width: 32px;
      height: 32px;
      border-radius: 50%;
      display: inline-grid;
      place-items: center;
      flex-shrink: 0;
      overflow: hidden;
      background: var(--accent);
      color: #fff;
      font-weight: 800;
      font-size: 13px;
      text-transform: uppercase;
    }
    .avatar img { width: 100%; height: 100%; object-fit: cover; display: block; }
    .avatar-slot { display: inline-flex; flex-shrink: 0; }
    .avatar.sm { width: 26px; height: 26px; font-size: 11px; }
    .avatar.lg { width: 44px; height: 44px; font-size: 17px; }

    /* ---------- Gate (before joining) ---------- */
    .gate, .notice {
      flex: 1;
      display: grid;
      place-items: center;
      padding: 12px 0 40px;
    }
    .gate-card, .notice-card {
      width: 100%;
      max-width: 440px;
      background: var(--surface);
      border-radius: 20px;
      padding: 28px;
      display: flex;
      flex-direction: column;
      gap: 18px;
      box-shadow: var(--shadow);
      animation: cardIn 0.35s ease;
    }
    @keyframes cardIn {
      from { opacity: 0; transform: translateY(10px) scale(0.98); }
      to { opacity: 1; transform: none; }
    }
    .gate-host { display: flex; align-items: center; gap: 12px; color: var(--text-muted); font-size: 13.5px; }
    .gate-host b { color: var(--text); }
    .gate-title {
      font-size: clamp(24px, 6vw, 30px);
      font-weight: 800;
      letter-spacing: -0.03em;
      line-height: 1.12;
      overflow-wrap: anywhere;
    }
    .now-card {
      display: flex;
      align-items: center;
      gap: 14px;
      padding: 12px;
      border-radius: 14px;
      background: var(--surface-2);
      min-width: 0;
    }
    .now-card img, .now-card .now-empty {
      width: 64px;
      height: 64px;
      border-radius: 10px;
      object-fit: cover;
      flex-shrink: 0;
      background: var(--surface-3);
      display: grid;
      place-items: center;
      color: var(--text-dim);
    }
    .now-card .now-empty svg { width: 26px; height: 26px; }
    .now-meta { min-width: 0; display: flex; flex-direction: column; gap: 2px; }
    .kicker { font-size: 10.5px; font-weight: 800; letter-spacing: 0.08em; color: var(--accent); text-transform: uppercase; }
    .now-title { font-weight: 700; font-size: 15px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
    .now-sub { font-size: 12.5px; color: var(--text-muted); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
    .people-line { display: flex; align-items: center; gap: 10px; color: var(--text-muted); font-size: 13px; }
    .stack { display: flex; }
    .stack .avatar { box-shadow: 0 0 0 2px var(--surface); }
    .stack .avatar + .avatar { margin-left: -8px; }
    .gate-form { display: flex; flex-direction: column; gap: 10px; }
    .field-label { font-size: 12px; font-weight: 600; color: var(--text-muted); }
    .text-input {
      width: 100%;
      background: var(--bg);
      border: 1px solid var(--border-bright);
      border-radius: 12px;
      padding: 12px 14px;
      font-size: 15px;
      outline: none;
      transition: border-color 0.14s;
    }
    .text-input:focus { border-color: var(--accent); }
    .member-chip {
      display: flex;
      align-items: center;
      gap: 10px;
      padding: 10px 12px;
      border-radius: 12px;
      background: var(--surface-2);
      font-size: 13.5px;
    }
    .member-chip span { color: var(--text-muted); }
    .form-error {
      background: rgba(244, 63, 94, 0.12);
      color: #fda4af;
      padding: 9px 12px;
      border-radius: 10px;
      font-size: 13px;
    }
    html[data-theme="light"] .form-error { color: #be123c; background: rgba(225, 29, 72, 0.08); }
    .gate-fine { text-align: center; font-size: 12.5px; color: var(--text-dim); }
    .host-note {
      font-size: 12.5px;
      color: var(--text-muted);
      text-align: center;
      padding-top: 2px;
    }
    .host-note a { color: var(--accent); font-weight: 600; }

    .notice-card { text-align: center; align-items: center; }
    .notice-icon {
      width: 64px;
      height: 64px;
      border-radius: 50%;
      display: grid;
      place-items: center;
      background: var(--surface-2);
      color: var(--text-muted);
    }
    .notice-icon svg { width: 30px; height: 30px; }
    .notice-card h1 { font-size: 24px; font-weight: 800; letter-spacing: -0.02em; }
    .notice-card p { color: var(--text-muted); font-size: 14px; max-width: 340px; }
    .notice-actions { display: flex; gap: 10px; flex-wrap: wrap; justify-content: center; }

    /* ---------- Room ---------- */
    .room {
      flex: 1;
      display: grid;
      grid-template-columns: minmax(0, 1fr) 380px;
      gap: 20px;
      align-items: stretch;
    }
    .stage {
      background: var(--surface);
      border-radius: 20px;
      padding: 22px 24px 24px;
      display: flex;
      flex-direction: column;
      gap: 18px;
      min-width: 0;
    }
    .stage-head { display: flex; align-items: center; justify-content: space-between; gap: 12px; min-width: 0; }
    .host-chip { display: flex; align-items: center; gap: 11px; min-width: 0; }
    .host-label { font-size: 11px; color: var(--text-dim); font-weight: 600; }
    .host-name { font-size: 14px; font-weight: 700; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
    .room-title {
      font-size: 13px;
      color: var(--text-muted);
      font-weight: 600;
      white-space: nowrap;
      overflow: hidden;
      text-overflow: ellipsis;
      max-width: 50%;
    }
    .art-wrap {
      position: relative;
      width: min(100%, 400px);
      aspect-ratio: 1 / 1;
      margin: 4px auto 0;
    }
    .art, .art-empty {
      width: 100%;
      height: 100%;
      border-radius: 14px;
      object-fit: cover;
      display: block;
      background: var(--surface-2);
      box-shadow: var(--shadow);
    }
    .art-empty { display: grid; place-items: center; color: var(--text-dim); text-align: center; padding: 24px; }
    .art-empty svg { width: 54px; height: 54px; margin: 0 auto 10px; display: block; opacity: 0.8; }
    .art-empty p { font-size: 13.5px; color: var(--text-muted); max-width: 240px; }
    .float-layer { position: absolute; inset: 0; pointer-events: none; overflow: hidden; border-radius: 14px; }
    .float-emoji {
      position: absolute;
      bottom: 8%;
      display: flex;
      flex-direction: column;
      align-items: center;
      gap: 2px;
      animation: floatUp 2.6s cubic-bezier(0.2, 0.7, 0.3, 1) forwards;
      will-change: transform, opacity;
    }
    .float-emoji .fe-icon { font-size: 34px; line-height: 1; filter: drop-shadow(0 4px 10px rgba(0, 0, 0, 0.35)); }
    .float-emoji .fe-name {
      font-size: 10.5px;
      font-weight: 700;
      color: #fff;
      background: rgba(0, 0, 0, 0.55);
      padding: 1px 7px;
      border-radius: 999px;
      max-width: 110px;
      white-space: nowrap;
      overflow: hidden;
      text-overflow: ellipsis;
    }
    @keyframes floatUp {
      0% { transform: translate(0, 0) scale(0.6); opacity: 0; }
      12% { transform: translate(var(--dx1), -18px) scale(1.08); opacity: 1; }
      70% { opacity: 1; }
      100% { transform: translate(var(--dx2), -260px) scale(0.92); opacity: 0; }
    }
    .sync-badge {
      position: absolute;
      top: 12px;
      left: 12px;
      display: inline-flex;
      align-items: center;
      gap: 6px;
      padding: 4px 10px;
      border-radius: 999px;
      background: rgba(0, 0, 0, 0.62);
      color: #fff;
      font-size: 11px;
      font-weight: 700;
      letter-spacing: 0.02em;
      backdrop-filter: blur(6px);
      -webkit-backdrop-filter: blur(6px);
    }
    .sync-badge i { width: 7px; height: 7px; border-radius: 50%; background: #1ed760; display: block; }
    .sync-badge.warn i { background: #fbbf24; }
    .sync-badge.idle i { background: #a0a0ab; }

    /* Positioned so they paint above the cover art's drop shadow (the
       art wrapper is position: relative and would otherwise cover them). */
    .meta { position: relative; text-align: center; min-width: 0; }
    .progress, .controls, .reactions { position: relative; }
    .t-title {
      font-size: clamp(20px, 3.2vw, 28px);
      font-weight: 800;
      letter-spacing: -0.03em;
      line-height: 1.15;
      overflow-wrap: anywhere;
    }
    .t-sub { margin-top: 4px; font-size: 14.5px; color: var(--text-muted); overflow-wrap: anywhere; }
    .badges { margin-top: 10px; display: flex; justify-content: center; gap: 6px; flex-wrap: wrap; }
    .q-badge {
      font-size: 10.5px;
      font-weight: 800;
      letter-spacing: 0.04em;
      padding: 3px 9px;
      border-radius: 999px;
      background: rgba(56, 189, 248, 0.14);
      color: var(--accent-blue);
    }
    .q-badge.lossless { background: rgba(245, 158, 11, 0.16); color: #f59e0b; }

    .progress { display: flex; flex-direction: column; gap: 8px; }
    .bar { height: 5px; border-radius: 999px; background: var(--surface-3); overflow: hidden; }
    .bar-fill { height: 100%; width: 0%; background: var(--text); border-radius: inherit; transition: width 0.25s linear; }
    .bar.live .bar-fill { background: var(--accent); }
    .times { display: flex; align-items: center; justify-content: space-between; gap: 10px; font-size: 12px; color: var(--text-muted); font-variant-numeric: tabular-nums; }
    .t-status { flex: 1; text-align: center; font-weight: 600; color: var(--text-muted); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
    .t-status.accent { color: var(--accent); }

    .controls { display: grid; grid-template-columns: 1fr auto 1fr; align-items: center; gap: 14px; }
    .controls .play-btn { grid-column: 2; grid-row: 1; }
    .controls .vol { grid-column: 3; grid-row: 1; justify-self: start; }
    .controls .invite-btn { grid-row: 1; }
    .play-btn {
      width: 56px;
      height: 56px;
      border-radius: 50%;
      background: var(--text);
      color: var(--bg);
      display: inline-grid;
      place-items: center;
      transition: transform 0.1s;
      flex-shrink: 0;
    }
    .play-btn:hover { transform: scale(1.04); }
    .play-btn:disabled { opacity: 0.35; cursor: default; transform: none; }
    .play-btn svg { width: 24px; height: 24px; }
    .vol { display: flex; align-items: center; gap: 4px; }
    .vol input { width: 110px; }

    .reactions { display: flex; justify-content: center; gap: 8px; flex-wrap: wrap; }
    .react-btn {
      width: 46px;
      height: 46px;
      border-radius: 999px;
      background: var(--surface-2);
      font-size: 22px;
      line-height: 1;
      display: inline-grid;
      place-items: center;
      transition: transform 0.12s, background 0.12s;
    }
    .react-btn:hover { background: var(--surface-3); transform: translateY(-2px); }
    .react-btn:active { transform: scale(0.9); }

    .side {
      background: var(--surface);
      border-radius: 20px;
      display: flex;
      flex-direction: column;
      /* Size containment: the panel takes the player card's height (grid
         stretch) instead of growing with the chat, which scrolls inside. */
      contain: size;
      min-height: 440px;
      overflow: hidden;
      min-width: 0;
    }
    .people-box { padding: 18px 18px 12px; }
    .people-head { display: flex; align-items: center; justify-content: space-between; margin-bottom: 10px; }
    .people-head h3 { font-size: 14px; font-weight: 800; }
    .count-chip { font-size: 12px; font-weight: 700; color: var(--accent); background: var(--accent-soft); padding: 2px 9px; border-radius: 999px; }
    .people { display: flex; flex-wrap: wrap; gap: 6px; max-height: 92px; overflow-y: auto; }
    .person {
      display: inline-flex;
      align-items: center;
      gap: 7px;
      padding: 3px 10px 3px 3px;
      border-radius: 999px;
      background: var(--surface-2);
      font-size: 12.5px;
      font-weight: 600;
      max-width: 100%;
    }
    .person > span:last-child { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; min-width: 0; }
    .person.you { box-shadow: inset 0 0 0 1.5px var(--accent); }
    .tabs { display: flex; gap: 4px; padding: 0 14px; border-bottom: 1px solid var(--border); }
    .tab {
      position: relative;
      padding: 11px 12px;
      font-size: 13px;
      font-weight: 700;
      color: var(--text-muted);
      display: inline-flex;
      align-items: center;
      gap: 6px;
    }
    .tab:hover { color: var(--text); }
    .tab.active { color: var(--text); }
    .tab.active::after {
      content: '';
      position: absolute;
      left: 12px;
      right: 12px;
      bottom: -1px;
      height: 2px;
      border-radius: 2px;
      background: var(--accent);
    }
    .tab-dot { min-width: 17px; height: 17px; border-radius: 999px; background: var(--accent); color: var(--accent-ink); font-size: 10.5px; display: inline-grid; place-items: center; padding: 0 5px; }
    .pane { flex: 1; min-height: 0; display: flex; flex-direction: column; }
    .pane-scroll { flex: 1; min-height: 0; overflow-y: auto; padding: 12px 14px; }

    .chat-msg { display: flex; gap: 10px; padding: 4px 4px; }
    .chat-msg.cont { padding-top: 0; }
    .chat-msg.cont .avatar { visibility: hidden; height: 0; }
    .chat-body { min-width: 0; flex: 1; }
    .chat-meta { display: flex; align-items: baseline; gap: 6px; font-size: 12px; }
    .chat-name { font-weight: 700; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; max-width: 70%; }
    .chat-time { color: var(--text-dim); font-size: 11px; }
    .host-tag { font-size: 9.5px; font-weight: 800; letter-spacing: 0.05em; color: var(--accent-ink); background: var(--accent); padding: 1px 6px; border-radius: 999px; }
    .chat-text { font-size: 13.5px; overflow-wrap: anywhere; white-space: pre-wrap; }
    .chat-sys { text-align: center; font-size: 12px; color: var(--text-dim); padding: 6px 0; }
    .chat-empty { text-align: center; color: var(--text-muted); font-size: 13px; padding: 40px 16px; }
    .chat-form { display: flex; gap: 8px; padding: 12px 14px 14px; border-top: 1px solid var(--border); }
    .chat-form .text-input { padding: 10px 14px; font-size: 14px; border-radius: 999px; }
    .send-btn { width: 42px; height: 42px; border-radius: 50%; background: var(--accent); color: var(--accent-ink); display: inline-grid; place-items: center; flex-shrink: 0; }
    .send-btn:disabled { opacity: 0.45; cursor: not-allowed; }
    .send-btn svg { width: 18px; height: 18px; }
    .chat-off { padding: 14px; border-top: 1px solid var(--border); text-align: center; font-size: 12.5px; color: var(--text-muted); }

    .next-row { display: flex; align-items: center; gap: 11px; padding: 7px 6px; border-radius: 10px; }
    .next-row img { width: 42px; height: 42px; border-radius: 8px; object-fit: cover; background: var(--surface-2); flex-shrink: 0; }
    .next-meta { min-width: 0; flex: 1; }
    .next-title { font-weight: 600; font-size: 13.5px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
    .next-sub { font-size: 12px; color: var(--text-muted); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
    .next-dur { font-size: 12px; color: var(--text-dim); font-variant-numeric: tabular-nums; }

    .lyrics { display: flex; flex-direction: column; gap: 6px; padding-bottom: 40%; }
    .lyric { font-size: 17px; font-weight: 700; color: var(--text-dim); line-height: 1.35; transition: color 0.2s; }
    .lyric.active { color: var(--text); }
    .lyric.plain { font-size: 14.5px; font-weight: 500; color: var(--text-muted); }

    .tap-resume {
      position: fixed;
      inset: 0;
      z-index: 50;
      display: grid;
      place-items: center;
      background: rgba(0, 0, 0, 0.55);
      backdrop-filter: blur(4px);
      -webkit-backdrop-filter: blur(4px);
    }
    .toast {
      position: fixed;
      left: 50%;
      bottom: 24px;
      transform: translateX(-50%);
      z-index: 60;
      background: var(--text);
      color: var(--bg);
      padding: 10px 18px;
      border-radius: 999px;
      font-size: 13px;
      font-weight: 700;
      max-width: calc(100vw - 32px);
      text-align: center;
    }

    @media (max-width: 960px) {
      .room { grid-template-columns: minmax(0, 1fr); }
      .side { height: 72vh; height: 72dvh; min-height: 380px; }
    }
    @media (max-width: 560px) {
      .shell { padding: 0 16px 20px; }
      .topbar { height: 56px; }
      .brand span { display: none; }
      .gate-card, .notice-card { padding: 22px 18px; border-radius: 18px; }
      .stage { padding: 16px 16px 20px; border-radius: 18px; gap: 16px; }
      .side { border-radius: 18px; }
      .art-wrap { width: min(100%, 320px); }
      .room-title { display: none; }
      .vol input { width: 72px; }
      .react-btn { width: 42px; height: 42px; font-size: 20px; }
    }
    /* Phone-only pieces stay out of the desktop/tablet layout. */
    .art-zone { display: flex; justify-content: center; }
    .sheet-head, .sheet-backdrop, .chat-ticker, .dock, .invite-btn { display: none; }

    /* ---------- Phone: one-screen player + bottom dock + sheet ---------- */
    @media (max-width: 760px) {
      body.in-room { overflow: hidden; overscroll-behavior: none; }
      .shell { padding: 0 16px; }
      .topbar { height: 56px; padding-top: env(safe-area-inset-top); }
      /* Join screen fills the phone: big cover up top, form at thumb height. */
      .gate { display: flex; padding: 4px 0 calc(16px + env(safe-area-inset-bottom)); }
      .gate-card {
        max-width: none;
        flex: 1;
        min-height: calc(100dvh - 56px - 20px - env(safe-area-inset-top) - env(safe-area-inset-bottom));
        gap: 16px;
        padding: 20px 18px;
      }
      .gate-title { text-align: center; }
      .gate-host { justify-content: center; text-align: left; }
      .now-card {
        flex-direction: column;
        text-align: center;
        background: transparent;
        padding: 0;
        gap: 12px;
      }
      .now-card img, .now-card .now-empty {
        width: min(64vw, 28dvh, 260px);
        height: auto;
        aspect-ratio: 1 / 1;
        border-radius: 14px;
        box-shadow: var(--shadow);
      }
      .now-card .now-empty svg { width: 40px; height: 40px; }
      .now-meta { align-items: center; max-width: 100%; }
      .people-line { justify-content: center; }
      .gate-form { margin-top: auto; }
      .gate-fine { margin-top: -4px; }

      .room {
        display: flex;
        flex-direction: column;
        height: calc(100dvh - 56px - env(safe-area-inset-top));
        padding-bottom: calc(74px + env(safe-area-inset-bottom));
        gap: 0;
      }
      .stage {
        flex: 1;
        min-height: 0;
        background: transparent;
        border-radius: 0;
        padding: 2px 0 14px;
        gap: 14px;
      }
      .stage-head { gap: 10px; }
      .room-title { display: block; max-width: 45%; font-size: 12px; }
      /* The art area takes whatever height is left; JS (fitArt) sizes the
         square cover to the smaller of its width and height. */
      .art-zone {
        flex: 1 1 0;
        min-height: 140px;
        align-items: center;
      }
      .art-wrap { width: min(100%, 260px); max-width: 100%; margin: 0; }
      .art, .art-empty { border-radius: 12px; }
      .meta { text-align: left; }
      .t-title { font-size: 22px; }
      .t-sub { font-size: 14px; margin-top: 2px; }
      .badges { justify-content: flex-start; margin-top: 8px; }
      .progress { gap: 6px; }
      .controls { gap: 8px; }
      .controls .vol { grid-column: 1; justify-self: start; }
      .controls .vol input { display: none; }
      .invite-btn { display: inline-flex; grid-column: 3; justify-self: end; width: 44px; height: 44px; }
      .controls .icon-btn { width: 44px; height: 44px; }
      .play-btn { width: 60px; height: 60px; }
      .reactions { gap: 6px; flex-wrap: nowrap; justify-content: space-between; }
      .react-btn { width: auto; flex: 1; max-width: 52px; height: 42px; font-size: 20px; }

      .side {
        position: fixed;
        left: 0;
        right: 0;
        bottom: 0;
        z-index: 40;
        height: 86vh;
        height: 86dvh;
        min-height: 0;
        contain: none;
        border-radius: 22px 22px 0 0;
        padding-bottom: env(safe-area-inset-bottom);
        transform: translateY(105%);
        transition: transform 0.32s cubic-bezier(0.2, 0.8, 0.2, 1);
        box-shadow: 0 -16px 48px rgba(0, 0, 0, 0.45);
        visibility: hidden;
      }
      .side.open { transform: none; visibility: visible; }
      .side.dragging { transition: none; }
      .sheet-head {
        display: flex;
        align-items: center;
        justify-content: center;
        position: relative;
        height: 30px;
        flex-shrink: 0;
        touch-action: none;
      }
      .sheet-grip { width: 40px; height: 5px; border-radius: 999px; background: var(--border-bright); }
      .sheet-head .icon-btn { position: absolute; right: 8px; top: 2px; width: 34px; height: 34px; }
      .people-box { padding: 4px 16px 10px; }
      .sheet-backdrop {
        display: block;
        position: fixed;
        inset: 0;
        z-index: 39;
        background: rgba(0, 0, 0, 0.5);
        opacity: 0;
        pointer-events: none;
        transition: opacity 0.25s;
      }
      .sheet-backdrop.open { opacity: 1; pointer-events: auto; }

      .dock {
        display: flex;
        align-items: center;
        gap: 4px;
        position: fixed;
        left: 0;
        right: 0;
        bottom: 0;
        z-index: 30;
        padding: 8px 12px calc(8px + env(safe-area-inset-bottom));
        background: var(--surface);
        border-top: 1px solid var(--border);
      }
      .dock-btn {
        flex: 1;
        display: flex;
        flex-direction: column;
        align-items: center;
        gap: 3px;
        padding: 6px 0;
        border-radius: 12px;
        font-size: 11px;
        font-weight: 700;
        color: var(--text-muted);
      }
      .dock-btn:active { background: var(--surface-2); }
      .dock-icon { position: relative; display: inline-flex; }
      .dock-icon svg { width: 22px; height: 22px; }
      .dock-dot {
        position: absolute;
        top: -5px;
        right: -10px;
        min-width: 17px;
        height: 17px;
        padding: 0 4px;
        border-radius: 999px;
        background: var(--accent);
        color: var(--accent-ink);
        font-size: 10px;
        font-weight: 800;
        display: grid;
        place-items: center;
        box-shadow: 0 0 0 2px var(--surface);
      }
      .dock-people {
        display: flex;
        align-items: center;
        gap: 6px;
        padding: 6px 10px 6px 6px;
        border-radius: 999px;
        background: var(--surface-2);
        flex-shrink: 0;
      }
      .dock-people .stack .avatar { box-shadow: 0 0 0 2px var(--surface-2); }
      .dock-count { font-size: 13px; font-weight: 800; }

      .chat-ticker {
        display: flex;
        align-items: center;
        gap: 10px;
        position: fixed;
        left: 16px;
        right: 16px;
        bottom: calc(82px + env(safe-area-inset-bottom));
        z-index: 31;
        padding: 9px 14px 9px 9px;
        border-radius: 16px;
        background: var(--surface-3);
        color: var(--text);
        text-align: left;
        box-shadow: 0 10px 30px rgba(0, 0, 0, 0.35);
        animation: tickerIn 0.25s ease;
      }
      .chat-ticker b { font-size: 12.5px; display: block; }
      .chat-ticker span.tk-text {
        font-size: 13px;
        color: var(--text-muted);
        display: block;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
      }
      .chat-ticker .tk-body { min-width: 0; flex: 1; }
      .toast { bottom: calc(90px + env(safe-area-inset-bottom)); }
    }
    /* Short phones (SE-class): tighten the join screen so it fits without scrolling. */
    @media (max-width: 760px) and (max-height: 700px) {
      .gate-card { gap: 10px; padding: 14px 16px; }
      .gate-title { font-size: 22px; }
      .gate-host .avatar.lg { width: 34px; height: 34px; font-size: 14px; }
      .now-card { gap: 8px; }
      .now-card img, .now-card .now-empty { width: min(64vw, 22dvh, 260px); }
      .gate-form { gap: 8px; }
      .text-input { padding: 10px 14px; }
      .btn-lg { padding: 11px 22px; }
    }
    @media (max-width: 760px) and (max-height: 640px) {
      .stage-head { display: none; }
      .badges { display: none; }
      .reactions .react-btn { height: 38px; }
    }
    @keyframes tickerIn {
      from { opacity: 0; transform: translateY(8px); }
      to { opacity: 1; transform: none; }
    }

    @media (prefers-reduced-motion: reduce) {
      .live-dot, .gate-card, .notice-card { animation: none; }
      .side { transition: none; }
      .float-emoji { animation-duration: 1.2s; }
    }
  </style>
</head>
<body>
  <div id="bg-tint" class="bg-tint"></div>
  <div class="shell">
    <header class="topbar">
      <a class="brand" href="/" aria-label="Beatlab home">
        <svg viewBox="0 0 32 32" fill="none" aria-hidden="true"><circle cx="16" cy="16" r="15" fill="#1ed760"/><path d="M12 10.5v11l9-5.5-9-5.5z" fill="#08120c"/></svg>
        <span>Beatlab</span>
      </a>
      <div class="top-right">
        <span id="live-pill" class="live-pill hidden"><span class="live-dot"></span><span id="live-text">LIVE</span></span>
        <a id="get-app" class="btn-secondary btn-sm" href="/">Get Beatlab</a>
      </div>
    </header>

    <main id="gate" class="gate hidden">
      <div class="gate-card">
        <div class="gate-host"><span id="gate-host-avatar" class="avatar-slot"></span><div id="gate-invite"></div></div>
        <h1 id="gate-title" class="gate-title"></h1>
        <div id="gate-now"></div>
        <div id="gate-people" class="people-line"></div>
        <form id="gate-form" class="gate-form" novalidate>
          <div id="gate-name-wrap" class="gate-form">
            <label class="field-label" for="gate-name">Your name</label>
            <input id="gate-name" class="text-input" type="text" maxlength="24" autocomplete="nickname" placeholder="What should friends call you?" />
          </div>
          <div id="gate-member" class="member-chip hidden"></div>
          <div id="gate-error" class="form-error hidden" role="alert"></div>
          <button id="gate-join" class="btn-primary btn-lg" type="submit">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 18v-6a9 9 0 0 1 18 0v6"/><path d="M21 19a2 2 0 0 1-2 2h-1a2 2 0 0 1-2-2v-3a2 2 0 0 1 2-2h3zM3 19a2 2 0 0 0 2 2h1a2 2 0 0 0 2-2v-3a2 2 0 0 0-2-2H3z"/></svg>
            <span>Join &amp; Listen</span>
          </button>
        </form>
        <p id="gate-fine" class="gate-fine"></p>
        <p id="host-note" class="host-note hidden">You're hosting this lobby. <a href="/#lobby">Manage it in Beatlab</a></p>
      </div>
    </main>

    <main id="notice" class="notice hidden">
      <div class="notice-card">
        <div id="notice-icon" class="notice-icon"></div>
        <h1 id="notice-title"></h1>
        <p id="notice-text"></p>
        <div class="notice-actions">
          <a class="btn-primary" href="/">Try Beatlab free</a>
          <button id="notice-retry" class="btn-secondary hidden" type="button">Try again</button>
        </div>
      </div>
    </main>

    <main id="room" class="room hidden">
      <section class="stage" aria-label="Now playing">
        <div class="stage-head">
          <div class="host-chip">
            <span id="host-avatar" class="avatar-slot"></span>
            <div style="min-width:0"><div class="host-label">Hosted by</div><div id="host-name" class="host-name"></div></div>
          </div>
          <div id="room-title" class="room-title"></div>
        </div>
        <div class="art-zone">
        <div class="art-wrap">
          <img id="art" class="art hidden" alt="" />
          <div id="art-empty" class="art-empty">
            <div>
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M9 18V5l12-2v13"/><circle cx="6" cy="18" r="3"/><circle cx="18" cy="16" r="3"/></svg>
              <p id="art-empty-text">Waiting for the host to play something</p>
            </div>
          </div>
          <div id="float-layer" class="float-layer"></div>
          <span id="sync-badge" class="sync-badge idle"><i></i><span id="sync-text">Connecting</span></span>
        </div>
        </div>
        <div class="meta">
          <h2 id="t-title" class="t-title">Nothing playing yet</h2>
          <div id="t-sub" class="t-sub"></div>
          <div class="badges"><span id="t-quality" class="q-badge hidden"></span></div>
        </div>
        <div class="progress">
          <div id="bar" class="bar"><div id="bar-fill" class="bar-fill"></div></div>
          <div class="times"><span id="t-elapsed">0:00</span><span id="t-status" class="t-status"></span><span id="t-duration">0:00</span></div>
        </div>
        <div class="controls">
          <button id="btn-listen" class="play-btn" type="button" title="Pause for me" aria-label="Pause for me">
            <svg id="listen-icon" viewBox="0 0 24 24" fill="currentColor"></svg>
          </button>
          <div class="vol">
            <button id="btn-mute" class="icon-btn" type="button" title="Mute" aria-label="Mute">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.1" stroke-linecap="round" stroke-linejoin="round"><g id="vol-icon"></g></svg>
            </button>
            <input id="vol" type="range" min="0" max="1" step="0.01" value="0.9" aria-label="Volume" />
          </div>
          <button id="btn-invite" class="icon-btn invite-btn" type="button" title="Invite a friend" aria-label="Invite a friend">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.1" stroke-linecap="round" stroke-linejoin="round"><path d="M12 15V3"/><path d="m7 8 5-5 5 5"/><path d="M5 13v6a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2v-6"/></svg>
          </button>
        </div>
        <div id="reactions" class="reactions" aria-label="Send a reaction"></div>
      </section>

      <aside id="side" class="side" aria-label="Lobby">
        <div id="sheet-head" class="sheet-head">
          <span class="sheet-grip" aria-hidden="true"></span>
          <button id="sheet-close" class="icon-btn" type="button" title="Close" aria-label="Close">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><path d="m6 9 6 6 6-6"/></svg>
          </button>
        </div>
        <div class="people-box">
          <div class="people-head"><h3>Listening now</h3><span id="count-chip" class="count-chip">0</span></div>
          <div id="people" class="people"></div>
        </div>
        <div class="tabs" role="tablist">
          <button class="tab active" role="tab" data-tab="chat" type="button">Chat <span id="chat-dot" class="tab-dot hidden"></span></button>
          <button class="tab" role="tab" data-tab="next" type="button">Up Next</button>
          <button class="tab" role="tab" data-tab="lyrics" type="button">Lyrics</button>
        </div>
        <div id="pane-chat" class="pane">
          <div id="chat-list" class="pane-scroll"></div>
          <form id="chat-form" class="chat-form">
            <input id="chat-input" class="text-input" type="text" maxlength="300" autocomplete="off" placeholder="Say something…" aria-label="Chat message" />
            <button id="chat-send" class="send-btn" type="submit" title="Send" aria-label="Send">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><path d="M5 12h14M13 6l6 6-6 6"/></svg>
            </button>
          </form>
          <div id="chat-off" class="chat-off hidden">The host turned chat off. Reactions still work.</div>
        </div>
        <div id="pane-next" class="pane hidden"><div id="next-list" class="pane-scroll"></div></div>
        <div id="pane-lyrics" class="pane hidden"><div id="lyrics-box" class="pane-scroll"></div></div>
      </aside>

      <!-- Phone only: backdrop for the sheet, live chat ticker and bottom dock -->
      <div id="sheet-backdrop" class="sheet-backdrop"></div>
      <button id="chat-ticker" class="chat-ticker hidden" type="button" aria-live="polite"></button>
      <nav id="dock" class="dock" aria-label="Lobby panels">
        <button class="dock-btn" type="button" data-open="chat">
          <span class="dock-icon"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 12a8 8 0 0 1-11.6 7.1L4 20.5l1.4-4.6A8 8 0 1 1 21 12z"/></svg><span id="dock-chat-dot" class="dock-dot hidden"></span></span>
          <span>Chat</span>
        </button>
        <button class="dock-btn" type="button" data-open="next">
          <span class="dock-icon"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="M8 6h13M8 12h13M8 18h13"/><circle cx="3.5" cy="6" r="1" fill="currentColor"/><circle cx="3.5" cy="12" r="1" fill="currentColor"/><circle cx="3.5" cy="18" r="1" fill="currentColor"/></svg></span>
          <span>Up Next</span>
        </button>
        <button class="dock-btn" type="button" data-open="lyrics">
          <span class="dock-icon"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M9 18V5l12-2v13"/><circle cx="6" cy="18" r="3"/><circle cx="18" cy="16" r="3"/></svg></span>
          <span>Lyrics</span>
        </button>
        <button class="dock-people" type="button" data-open="chat" aria-label="Who's listening">
          <span id="dock-stack" class="stack"></span>
          <span id="dock-count" class="dock-count">0</span>
        </button>
      </nav>
    </main>
  </div>

  <div id="tap-resume" class="tap-resume hidden">
    <button id="tap-resume-btn" class="btn-primary btn-lg" type="button">
      <svg viewBox="0 0 24 24" fill="currentColor"><path d="M8 4.5v15l12-7.5z"/></svg>
      <span>Tap to start audio</span>
    </button>
  </div>
  <div id="toast" class="toast hidden" role="status" aria-live="polite"></div>
  <audio id="audio" preload="auto" playsinline></audio>

  <script id="lobby-boot" type="application/json">__BOOT_JSON__</script>
  <script>
  (() => {
    'use strict';
    const BOOT = JSON.parse(document.getElementById('lobby-boot').textContent || '{}');
    const CODE = BOOT.code || '';
    const $ = (id) => document.getElementById(id);
    const audio = $('audio');

    const ICON = {
      pause: '<path d="M6.5 4.5h4v15h-4zm7 0h4v15h-4z"/>',
      play: '<path d="M8 4.5v15l12-7.5z"/>',
      volOn: '<polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5"/><path d="M15.5 8.5a5 5 0 0 1 0 7"/><path d="M19 5a10 10 0 0 1 0 14"/>',
      volOff: '<polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5"/><line x1="22" y1="9" x2="16" y2="15"/><line x1="16" y1="9" x2="22" y2="15"/>',
      ended: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M9 18V5l12-2v13"/><circle cx="6" cy="18" r="3"/><circle cx="18" cy="16" r="3"/></svg>',
      missing: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/><path d="M8.5 11h5"/></svg>',
      blocked: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="9"/><path d="m5.7 5.7 12.6 12.6"/></svg>',
      lock: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="4.5" y="11" width="15" height="10" rx="2"/><path d="M8 11V7.5a4 4 0 0 1 8 0V11"/></svg>',
      note: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M9 18V5l12-2v13"/><circle cx="6" cy="18" r="3"/><circle cx="18" cy="16" r="3"/></svg>',
    };

    const store = {
      get(k) { try { return localStorage.getItem(k); } catch { return null; } },
      set(k, v) { try { localStorage.setItem(k, v); } catch {} },
    };
    function newId() {
      if (window.crypto && typeof crypto.randomUUID === 'function') return crypto.randomUUID().replace(/-/g, '');
      let s = '';
      for (let i = 0; i < 32; i++) s += Math.floor(Math.random() * 16).toString(16);
      return s;
    }
    let lid = store.get('beatlab_lobby_lid') || '';
    if (!/^[A-Za-z0-9_-]{16,64}$/.test(lid)) {
      lid = newId();
      store.set('beatlab_lobby_lid', lid);
    }

    const S = {
      status: BOOT.status,
      state: BOOT.state || null,
      pid: '',
      joined: false,
      user: null,
      es: null,
      chat: [],
      clockOffset: 0,
      loadedId: '',
      base: 0,
      needSeek: false,
      checkOffset: false,
      settleUntil: 0,
      compatTried: '',
      drift: 0,
      localPaused: false,
      blocked: false,
      tab: 'chat',
      unread: 0,
      lyrics: null,
      lyricsFor: '',
      lyricIdx: -1,
      rejoinTimer: null,
      rejoinDelay: 2000,
      loops: [],
      volume: (() => { const v = parseFloat(store.get('beatlab_vol') || '0.9'); return Number.isFinite(v) ? Math.min(1, Math.max(0, v)) : 0.9; })(),
      muted: false,
      lastReactAt: 0,
      selfPauseUntil: 0,
      forceSyncTimer: null,
      sheetOpen: false,
      tickerTimer: null,
    };
    const MQ_PHONE = window.matchMedia('(max-width: 760px)');
    const isPhone = () => MQ_PHONE.matches;
    // Phones: make the square cover as big as the space left on screen allows.
    function fitArt() {
      const zone = document.querySelector('.art-zone');
      const wrap = document.querySelector('.art-wrap');
      if (!zone || !wrap) return;
      if (!isPhone()) { wrap.style.width = ''; return; }
      const size = Math.floor(Math.min(zone.clientWidth, zone.clientHeight));
      wrap.style.width = size > 0 ? `${size}px` : '';
    }
    // Chat counts as "seen" when its tab is showing (and, on phones, the sheet is up).
    const chatVisible = () => S.tab === 'chat' && (!isPhone() || S.sheetOpen);

    // ---------- Utilities ----------
    function esc(s) {
      return String(s ?? '').replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;').replace(/'/g, '&#39;');
    }
    function fmt(sec) {
      if (!Number.isFinite(sec) || sec < 0) sec = 0;
      const s = Math.floor(sec);
      return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
    }
    function clockTime(ms) {
      try { return new Date(ms).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' }); } catch { return ''; }
    }
    function toast(msg) {
      const el = $('toast');
      el.textContent = msg;
      el.classList.remove('hidden');
      clearTimeout(el._t);
      el._t = setTimeout(() => el.classList.add('hidden'), 2600);
    }
    function parse(s) { try { return JSON.parse(s); } catch { return null; } }
    function hostName() {
      const h = S.state && S.state.host;
      return (h && (h.displayName || h.username)) || 'The host';
    }
    function avatarHtml(p, cls = '') {
      const color = (p && p.color) || '#1ed760';
      const name = (p && (p.name || p.displayName || p.username)) || '?';
      const inner = p && p.avatarUrl ? `<img src="${esc(p.avatarUrl)}" alt="" />` : esc((p && p.avatarLetter) || name.trim().charAt(0) || '?');
      return `<span class="avatar ${cls}" style="background:${esc(color)}">${inner}</span>`;
    }
    function hostAvatar(cls) {
      const h = (S.state && S.state.host) || {};
      return avatarHtml({ name: h.displayName || h.username, avatarUrl: h.avatarUrl, avatarLetter: h.avatarLetter, color: h.avatarColor || '#1ed760' }, cls);
    }
    function curTrack() { return (S.state && S.state.playback && S.state.playback.track) || null; }

    // ---------- Server clock ----------
    function serverNow() { return Date.now() + S.clockOffset; }
    async function syncClock(samples = 4) {
      let best = Infinity;
      let offset = S.clockOffset;
      for (let i = 0; i < samples; i++) {
        const t0 = Date.now();
        try {
          const r = await fetch('/api/lobby/clock', { cache: 'no-store' });
          const d = await r.json();
          const t1 = Date.now();
          if (t1 - t0 < best && Number.isFinite(d.now)) {
            best = t1 - t0;
            offset = d.now - (t0 + t1) / 2;
          }
        } catch {}
      }
      if (best < Infinity) S.clockOffset = offset;
    }

    // ---------- Position math ----------
    function expectedSec() {
      const p = S.state && S.state.playback;
      if (!p || !p.track) return 0;
      let ms = p.positionMs || 0;
      if (p.playing) ms += Math.max(0, serverNow() - (p.updatedAt || 0));
      let sec = ms / 1000;
      if (p.track.duration > 0) sec = Math.min(sec, p.track.duration);
      return Math.max(0, sec);
    }
    function currentSec() { return S.base + (audio.currentTime || 0); }
    function shouldPlay() {
      const p = S.state && S.state.playback;
      return !!(S.joined && p && p.track && p.playing && !S.localPaused);
    }

    // ---------- Audio ----------
    const SILENT = (() => {
      const n = 1600;
      const buf = new Uint8Array(44 + n);
      const v = new DataView(buf.buffer);
      const w = (o, s) => { for (let i = 0; i < s.length; i++) buf[o + i] = s.charCodeAt(i); };
      w(0, 'RIFF'); v.setUint32(4, 36 + n, true); w(8, 'WAVE'); w(12, 'fmt ');
      v.setUint32(16, 16, true); v.setUint16(20, 1, true); v.setUint16(22, 1, true);
      v.setUint32(24, 8000, true); v.setUint32(28, 8000, true); v.setUint16(32, 1, true); v.setUint16(34, 8, true);
      w(36, 'data'); v.setUint32(40, n, true); buf.fill(128, 44);
      let s = '';
      for (let i = 0; i < buf.length; i++) s += String.fromCharCode(buf[i]);
      return `data:audio/wav;base64,${btoa(s)}`;
    })();

    function canPlay(fmtName) {
      const f = String(fmtName || '').trim().toLowerCase();
      if (!f) return true;
      const probes = {
        mp3: ['audio/mpeg'], flac: ['audio/flac', 'audio/x-flac'], wav: ['audio/wav', 'audio/x-wav'],
        m4a: ['audio/mp4; codecs="mp4a.40.2"', 'audio/mp4'], mp4: ['audio/mp4; codecs="mp4a.40.2"', 'audio/mp4'],
        aac: ['audio/aac', 'audio/mp4; codecs="mp4a.40.2"'], ogg: ['audio/ogg; codecs="vorbis"', 'audio/ogg'],
        opus: ['audio/ogg; codecs="opus"', 'audio/webm; codecs="opus"'], webm: ['audio/webm; codecs="opus"', 'audio/webm'],
      };
      const list = probes[f];
      return list ? list.some((m) => audio.canPlayType(m) !== '') : false;
    }
    function streamUrl(t, offsetSec, compat) {
      let u = `/api/lobby/${encodeURIComponent(CODE)}/stream/${encodeURIComponent(t.id)}?lid=${encodeURIComponent(lid)}`;
      if (compat) u += '&compat=1';
      if (offsetSec > 0.5) u += `&t=${offsetSec.toFixed(2)}`;
      return u;
    }
    const NORM_TARGET_LUFS = -10.5;
    const NORM_MAX_CEILING_LIN = Math.pow(10, (-0.3 + 3.5) / 20);
    const NORM_DEFAULT_GAIN = 0.75;
    const NORM = {
      ctx: null, source: null, gainNode: null, limiterNode: null, ready: false, failed: false, connecting: false,
      trackId: '', loudness: null, metaRetryDoneFor: '', reqToken: 0, retryTimer: null, cache: new Map(),
    };
    function computeNormGain(vol, muted, loudness) {
      if (muted || vol <= 0) return 0;
      if (loudness && Number.isFinite(loudness.lufs)) {
        const gainDb = Math.max(-18, Math.min(12, NORM_TARGET_LUFS - loudness.lufs));
        const trackGain = Math.pow(10, gainDb / 20);
        const tpDb = Number.isFinite(loudness.truePeakDb) ? loudness.truePeakDb : 0;
        const tpLin = Math.max(0.01, Math.min(4.0, Math.pow(10, tpDb / 20)));
        return Math.min(vol * trackGain, NORM_MAX_CEILING_LIN / tpLin);
      }
      return Math.min(vol * NORM_DEFAULT_GAIN, 1.0);
    }
    function ensureNormGraph() {
      if (NORM.ready) {
        if (NORM.ctx && NORM.ctx.state !== 'running') NORM.ctx.resume().catch(() => {});
        return;
      }
      if (NORM.failed || NORM.connecting) return;
      const AC = window.AudioContext || window.webkitAudioContext;
      if (!AC) { NORM.failed = true; return; }
      if (navigator.audioSession) { try { navigator.audioSession.type = 'playback'; } catch {} }
      try { if (!NORM.ctx) NORM.ctx = new AC(); } catch { NORM.failed = true; return; }
      const ctx = NORM.ctx;
      const wire = () => {
        if (NORM.ready) return;
        try {
          NORM.source = ctx.createMediaElementSource(audio);
          NORM.gainNode = ctx.createGain();
          NORM.source.connect(NORM.gainNode);
          if (typeof ctx.createDynamicsCompressor === 'function') {
            const lim = ctx.createDynamicsCompressor();
            lim.threshold.value = -1.0;
            lim.knee.value = 1.0;
            lim.ratio.value = 20.0;
            lim.attack.value = 0.001;
            lim.release.value = 0.05;
            NORM.gainNode.connect(lim);
            lim.connect(ctx.destination);
            NORM.limiterNode = lim;
          } else {
            NORM.gainNode.connect(ctx.destination);
          }
          NORM.ready = true;
          NORM.connecting = false;
          ctx.addEventListener('statechange', () => {
            if (ctx.state !== 'running' && !audio.paused && !audio.ended) ctx.resume().catch(() => {});
          });
          applyVolume(0);
        } catch {
          NORM.failed = true;
          NORM.connecting = false;
        }
      };
      if (ctx.state === 'running') { wire(); return; }
      NORM.connecting = true;
      ctx.resume().then(() => {
        if (ctx.state === 'running' && !NORM.ready) wire();
        else NORM.connecting = false;
      }).catch(() => { NORM.connecting = false; });
    }
    document.addEventListener('pointerdown', ensureNormGraph, { capture: true, once: true });
    document.addEventListener('keydown', ensureNormGraph, { capture: true, once: true });
    function applyVolume(rampSec = 0) {
      const muted = S.muted || S.volume === 0;
      const target = computeNormGain(S.volume, muted, NORM.loudness);
      audio.muted = muted;
      if (NORM.ready && NORM.gainNode && NORM.ctx) {
        audio.volume = 1;
        const p = NORM.gainNode.gain;
        const now = NORM.ctx.currentTime;
        try {
          p.cancelScheduledValues(now);
          if (rampSec > 0.05) {
            p.setValueAtTime(p.value, now);
            p.linearRampToValueAtTime(target, now + rampSec);
          } else if (rampSec > 0) {
            p.setTargetAtTime(target, now, 0.012);
          } else {
            p.setValueAtTime(target, now);
          }
        } catch { p.value = target; }
      } else {
        audio.volume = Math.max(0, Math.min(1, target));
      }
      $('vol-icon').innerHTML = audio.muted ? ICON.volOff : ICON.volOn;
      $('vol').value = String(S.volume);
    }
    function canApplyPrePlayLobbyGain() {
      return audio.paused || audio.readyState < 3 || (audio.currentTime || 0) < 0.35;
    }
    function fetchLobbyLoudnessPrePlay(t, attempt = 0) {
      if (!t || !t.id || NORM.trackId !== t.id || NORM.loudness) return;
      const tok = NORM.reqToken;
      fetch(`/api/lobby/${encodeURIComponent(CODE)}/loudness/${encodeURIComponent(t.id)}?lid=${encodeURIComponent(lid)}`)
        .then((r) => (r.ok ? r.json() : null))
        .then((d) => {
          if (!d || tok !== NORM.reqToken || NORM.trackId !== t.id) return;
          if (Number.isFinite(Number(d.lufs))) {
            const norm = { lufs: Number(d.lufs), truePeakDb: Number.isFinite(Number(d.truePeakDb)) ? Number(d.truePeakDb) : 0 };
            NORM.cache.set(t.id, norm);
            NORM.loudness = norm;
            applyVolume(canApplyPrePlayLobbyGain() ? 0 : 0.6);
            return;
          }
          const retryDelays = [2500, 4500];
          if (d.pending && attempt < retryDelays.length) {
            NORM.retryTimer = setTimeout(() => {
              NORM.retryTimer = null;
              fetchLobbyLoudnessPrePlay(t, attempt + 1);
            }, retryDelays[attempt]);
          }
        })
        .catch(() => {});
    }
    function syncLobbyLoudness(t) {
      if (NORM.retryTimer) { clearTimeout(NORM.retryTimer); NORM.retryTimer = null; }
      NORM.reqToken++;
      NORM.metaRetryDoneFor = '';
      if (!t || !t.id) { NORM.trackId = ''; NORM.loudness = null; applyVolume(0); return; }
      NORM.trackId = t.id;
      const cached = NORM.cache.get(t.id);
      if (cached) { NORM.loudness = cached; applyVolume(0); return; }
      NORM.loudness = null;
      applyVolume(0);
      fetchLobbyLoudnessPrePlay(t);
    }
    function unlockAudio() {
      // iOS/Safari only let audio start inside a tap; play a blip of silence now
      // so the element stays unlocked once the real stream is ready.
      try {
        setSrc(SILENT);
        const p = audio.play();
        if (p && p.catch) p.catch(() => {});
      } catch {}
    }
    // Pauses we cause (host paused, track switch, reload) vs. ones the OS or
    // user causes (headphones unplugged, phone call, lock-screen button).
    function markSelfPause() { S.selfPauseUntil = Date.now() + 800; }
    function setSrc(url) { markSelfPause(); audio.src = url; }
    function pauseAudio() { if (!audio.paused) { markSelfPause(); audio.pause(); } }
    // Get audio going again if it should be playing: never restart a finished
    // track from 0 (the host will switch soon), and reopen a stream that died.
    function resumeIfNeeded() {
      if (!shouldPlay() || S.blocked || !S.loadedId) return;
      const t = curTrack();
      if (!t || t.id !== S.loadedId) return;
      const exp = expectedSec();
      if (audio.error) { reopenAt(exp); return; }
      if (audio.ended) {
        if (t.duration > 0 && exp < t.duration - 2) seekTo(exp);
        else return;
      }
      if (audio.paused) playAudio();
    }
    function reopenAt(sec) {
      const t = curTrack();
      if (!t) return;
      S.base = Math.max(0, sec);
      S.checkOffset = S.base > 0.5;
      S.needSeek = false;
      S.settleUntil = Date.now() + 2500;
      setSrc(streamUrl(t, S.base, S.compatTried === t.id || !canPlay(t.format)));
      playAudio();
    }
    function playAudio() {
      if (!shouldPlay()) return;
      ensureNormGraph();
      let p;
      try { p = audio.play(); } catch { return; }
      if (p && p.catch) {
        p.catch((e) => {
          if (e && e.name === 'NotAllowedError') {
            S.blocked = true;
            $('tap-resume').classList.remove('hidden');
          }
        });
      }
    }
    function loadTrack(t) {
      S.loadedId = t.id;
      S.compatTried = '';
      S.settleUntil = Date.now() + 2500;
      audio.playbackRate = 1;
      ensureNormGraph();
      syncLobbyLoudness(t);
      const compat = !canPlay(t.format);
      const exp = expectedSec();
      if (compat && exp > 2) {
        // Transcoded streams can't seek far ahead, so open them at the live point.
        S.base = exp + 0.4;
        S.needSeek = false;
        S.checkOffset = true;
        setSrc(streamUrl(t, S.base, true));
      } else {
        S.base = 0;
        S.needSeek = true;
        S.checkOffset = false;
        setSrc(streamUrl(t, 0, compat));
      }
      playAudio();
      if (S.tab === 'lyrics') loadLyrics();
      updateMediaSession();
    }
    function canSeekTo(rel) {
      const r = audio.seekable;
      if (!r) return false;
      for (let i = 0; i < r.length; i++) {
        if (rel >= r.start(i) && rel <= r.end(i) - 0.25) return true;
      }
      return false;
    }
    function seekTo(sec) {
      const t = curTrack();
      if (!t || t.id !== S.loadedId) return;
      S.settleUntil = Date.now() + 2500;
      const rel = sec - S.base;
      if (rel >= 0 && canSeekTo(rel)) {
        try { audio.currentTime = rel; return; } catch {}
      }
      reopenAt(sec);
    }
    function syncAudio() {
      const t = curTrack();
      if (!S.joined) return;
      if (!t) {
        pauseAudio();
        updateMediaSession();
        return;
      }
      if (t.id !== S.loadedId) {
        loadTrack(t);
        return;
      }
      if (shouldPlay()) {
        resumeIfNeeded();
        correctDrift();
      } else {
        pauseAudio();
        audio.playbackRate = 1;
        if (!S.localPaused && Math.abs(currentSec() - expectedSec()) > 1) seekTo(expectedSec());
      }
    }
    // Ignore the post-seek settling window and correct immediately.
    function forceSync() {
      S.settleUntil = 0;
      correctDrift();
    }
    function correctDrift() {
      if (!shouldPlay() || audio.paused || audio.seeking || audio.readyState < 3) {
        if (!shouldPlay()) audio.playbackRate = 1;
        return;
      }
      if (Date.now() < S.settleUntil) return;
      const exp = expectedSec();
      const t = curTrack();
      if (t && t.duration > 0 && exp > t.duration - 1.5) return;
      const drift = currentSec() - exp;
      S.drift = drift;
      if (Math.abs(drift) > 1) {
        audio.playbackRate = 1;
        seekTo(exp + 0.2);
      } else if (Math.abs(drift) > 0.12) {
        // Small drift: speed up/slow down a touch instead of an audible jump.
        audio.playbackRate = Math.min(1.06, Math.max(0.94, 1 - drift * 0.1));
      } else {
        audio.playbackRate = 1;
      }
    }

    audio.preservesPitch = true;
    audio.addEventListener('loadedmetadata', () => {
      if (!S.loadedId) return;
      const t = curTrack();
      if (t && t.id === NORM.trackId && !NORM.loudness && NORM.metaRetryDoneFor !== t.id) {
        NORM.metaRetryDoneFor = t.id;
        fetchLobbyLoudnessPrePlay(t);
      }
      if (S.checkOffset && t && t.duration > 0 && Number.isFinite(audio.duration)) {
        S.checkOffset = false;
        // The server ignored ?t= (no transcoder available): we got the whole file.
        if (Math.abs(audio.duration - t.duration) < 2 && S.base > 5) {
          S.base = 0;
          S.needSeek = true;
        }
      }
      if (S.needSeek) {
        S.needSeek = false;
        const exp = expectedSec();
        if (exp > 1) seekTo(exp + 0.15);
      }
    });
    audio.addEventListener('playing', () => {
      S.blocked = false;
      $('tap-resume').classList.add('hidden');
      renderTransport();
      // Coming out of a stall or a resume: snap back to the live point right
      // away instead of waiting for the next correction tick.
      clearTimeout(S.forceSyncTimer);
      S.forceSyncTimer = setTimeout(forceSync, 350);
    });
    audio.addEventListener('pause', () => {
      // Paused by the OS/browser (headphones out, call, lock screen) while the
      // host is still playing: respect it instead of fighting it every second.
      if (S.joined && shouldPlay() && !audio.ended && !audio.error && Date.now() > S.selfPauseUntil) {
        S.localPaused = true;
      }
      renderTransport();
    });
    audio.addEventListener('waiting', renderTransport);
    audio.addEventListener('error', () => {
      if (!S.loadedId || !audio.error) return;
      const t = curTrack();
      if (!t || t.id !== S.loadedId) return;
      const code = audio.error.code;
      if ((code === 3 || code === 4) && S.compatTried !== t.id) {
        S.compatTried = t.id;
        reopenAt(expectedSec());
        return;
      }
      toast(`Couldn't play "${t.title}". It'll pick up on the next song.`);
    });
    audio.addEventListener('timeupdate', () => {
      if (S.tab === 'lyrics') syncLyrics();
    });

    // ---------- Rendering ----------
    function setTint(url) {
      const el = $('bg-tint');
      if (url) {
        el.style.backgroundImage = `url("${url.replace(/"/g, '%22')}")`;
        el.classList.add('on');
      } else {
        el.classList.remove('on');
      }
    }
    function renderLivePill() {
      const pill = $('live-pill');
      const st = S.state;
      if (!st || S.status !== 'live') { pill.classList.add('hidden'); return; }
      pill.classList.remove('hidden');
      const online = st.host && st.host.online;
      pill.classList.toggle('off', !online);
      const n = st.listenerCount || 0;
      $('live-text').textContent = online ? (n ? `LIVE · ${n}` : 'LIVE') : 'HOST AWAY';
    }
    function renderGate() {
      const st = S.state || {};
      const host = hostName();
      $('gate-host-avatar').innerHTML = hostAvatar('lg');
      $('gate-invite').innerHTML = `<b>${esc(host)}</b> invited you to listen together`;
      $('gate-title').textContent = st.name || 'Listening lobby';
      const t = curTrack();
      $('gate-now').innerHTML = t
        ? `<div class="now-card"><img src="${esc(t.coverUrl)}" alt="" /><div class="now-meta"><span class="kicker">${st.playback && st.playback.playing ? 'Now playing' : 'Paused'}</span><span class="now-title">${esc(t.title)}</span><span class="now-sub">${esc(t.artist)}${t.album ? ` · ${esc(t.album)}` : ''}</span></div></div>`
        : `<div class="now-card"><div class="now-empty">${ICON.note}</div><div class="now-meta"><span class="kicker">Starting soon</span><span class="now-title">Nothing playing yet</span><span class="now-sub">Join now and you'll hear the first song</span></div></div>`;
      const people = st.listeners || [];
      $('gate-people').innerHTML = people.length
        ? `<span class="stack">${people.slice(0, 5).map((p) => avatarHtml(p, 'sm')).join('')}</span><span>${people.length === 1 ? `${esc(people[0].name)} is listening` : `${people.length} people listening`}</span>`
        : '<span>Be the first to join</span>';
      $('gate-fine').textContent = `No account needed · Plays in sync with ${host}`;
      if (st.locked) showGateError('The host has locked this lobby. Ask them to unlock it.');
      setTint(t ? t.coverUrl : '');
    }
    function renderHeader() {
      const st = S.state || {};
      $('host-avatar').innerHTML = hostAvatar('');
      $('host-name').textContent = hostName();
      $('room-title').textContent = st.name || '';
      $('room-title').title = st.name || '';
    }
    function renderNow() {
      const t = curTrack();
      const art = $('art');
      if (t) {
        if (art.getAttribute('src') !== t.coverUrl) art.src = t.coverUrl;
        art.classList.remove('hidden');
        $('art-empty').classList.add('hidden');
        $('t-title').textContent = t.title;
        $('t-sub').textContent = [t.artist, t.album].filter(Boolean).join(' · ');
        const q = $('t-quality');
        q.textContent = t.qualityBadge || '';
        q.classList.toggle('hidden', !t.qualityBadge);
        q.classList.toggle('lossless', !!t.isLossless);
        $('t-duration').textContent = fmt(t.duration);
        document.title = `${t.title} · ${(S.state && S.state.name) || 'Listen Together'} — Beatlab`;
      } else {
        art.classList.add('hidden');
        $('art-empty').classList.remove('hidden');
        $('art-empty-text').textContent = `Waiting for ${hostName()} to play something`;
        $('t-title').textContent = 'Nothing playing yet';
        $('t-sub').textContent = `${hostName()} hasn't started a song`;
        $('t-quality').classList.add('hidden');
        $('t-duration').textContent = '0:00';
        document.title = `${(S.state && S.state.name) || 'Listen Together'} — Beatlab`;
      }
      setTint(t ? t.coverUrl : '');
    }
    function statusText() {
      const st = S.state;
      if (!st) return ['Connecting…', false];
      const host = hostName();
      if (!st.host.online) return [st.host.away ? `${host} stepped away` : `${host} went offline`, false];
      const p = st.playback;
      if (!p.track) return [`Waiting for ${host}`, false];
      if (p.buffering) return [`${host} is buffering…`, false];
      if (!p.playing) return [`Paused by ${host}`, false];
      if (S.localPaused) return ['Paused for you', false];
      if (S.blocked) return ['Tap to start audio', false];
      return ['Listening live', true];
    }
    function renderTransport() {
      const [text, live] = statusText();
      const status = $('t-status');
      status.textContent = text;
      status.classList.toggle('accent', live);
      $('bar').classList.toggle('live', live);
      // The big button controls *your* listening; when the host has stopped,
      // show a disabled play state instead of a misleading pause icon.
      const hostLive = !!(S.state && S.state.playback && S.state.playback.playing);
      const btn = $('btn-listen');
      let icon = ICON.pause;
      let label = 'Pause for me';
      if (S.localPaused) { icon = ICON.play; label = 'Rejoin live'; }
      else if (!hostLive) { icon = ICON.play; label = statusText()[0]; }
      $('listen-icon').innerHTML = icon;
      btn.disabled = !S.localPaused && !hostLive;
      btn.title = label;
      btn.setAttribute('aria-label', label);

      const badge = $('sync-badge');
      let cls = 'idle';
      let txt = 'Waiting';
      if (S.es && S.es.readyState !== 1) { cls = 'warn'; txt = 'Reconnecting'; }
      else if (shouldPlay()) {
        if (audio.paused || audio.readyState < 3) { cls = 'warn'; txt = 'Buffering'; }
        else if (Math.abs(S.drift) > 0.4) { cls = 'warn'; txt = 'Syncing'; }
        else { cls = ''; txt = 'In sync'; }
      } else if (S.state && S.state.playback && S.state.playback.track) {
        txt = S.localPaused ? 'Paused for you' : 'Paused';
      }
      badge.className = `sync-badge ${cls}`;
      $('sync-text').textContent = txt;
    }
    function renderProgress() {
      const t = curTrack();
      if (!t) {
        $('bar-fill').style.width = '0%';
        $('t-elapsed').textContent = '0:00';
        return;
      }
      const pos = expectedSec();
      $('t-elapsed').textContent = fmt(pos);
      const pct = t.duration > 0 ? Math.min(100, (pos / t.duration) * 100) : 0;
      $('bar-fill').style.width = `${pct.toFixed(2)}%`;
    }
    function renderPeople() {
      const people = (S.state && S.state.listeners) || [];
      $('count-chip').textContent = String(people.length);
      $('people').innerHTML = people.length
        ? people.map((p) => `<span class="person ${p.pid === S.pid ? 'you' : ''}" title="${esc(p.name)}${p.username ? ` (@${esc(p.username)})` : ''}">${avatarHtml(p, 'sm')}<span>${esc(p.name)}${p.pid === S.pid ? ' (you)' : ''}</span></span>`).join('')
        : '<span style="color:var(--text-muted);font-size:13px">Nobody else yet</span>';
      $('dock-stack').innerHTML = people.slice(0, 3).map((p) => avatarHtml(p, 'sm')).join('');
      $('dock-count').textContent = String(people.length);
    }
    function renderUpNext() {
      const list = (S.state && S.state.upNext) || [];
      $('next-list').innerHTML = list.length
        ? list.map((t) => `<div class="next-row"><img src="${esc(t.coverUrl)}" alt="" loading="lazy" /><div class="next-meta"><div class="next-title">${esc(t.title)}</div><div class="next-sub">${esc(t.artist)}</div></div><span class="next-dur">${fmt(t.duration)}</span></div>`).join('')
        : `<p class="chat-empty">${esc(hostName())}'s queue is a surprise. Stick around!</p>`;
    }
    function renderReactions() {
      const list = (S.state && S.state.reactions) || [];
      const box = $('reactions');
      if (box.dataset.ready === list.join('')) return;
      box.dataset.ready = list.join('');
      box.innerHTML = list.map((e) => `<button class="react-btn" type="button" data-emoji="${esc(e)}" aria-label="React ${esc(e)}">${esc(e)}</button>`).join('');
    }
    function renderChatAvailability() {
      const allow = !S.state || S.state.allowChat;
      $('chat-form').classList.toggle('hidden', !allow);
      $('chat-off').classList.toggle('hidden', allow);
    }
    function renderAll() {
      renderLivePill();
      if (!S.joined) { renderGate(); return; }
      renderHeader();
      renderNow();
      renderPeople();
      renderUpNext();
      renderReactions();
      renderChatAvailability();
      renderTransport();
      renderProgress();
    }

    // ---------- Chat & reactions ----------
    function chatRowHtml(m, prev) {
      if (m.kind === 'system') return `<div class="chat-sys">${esc(m.text)}</div>`;
      const cont = prev && prev.kind !== 'system' && prev.pid === m.pid && m.at - prev.at < 120000;
      return `<div class="chat-msg ${cont ? 'cont' : ''}">
        ${avatarHtml({ name: m.name, color: m.color, avatarUrl: m.avatarUrl }, 'sm')}
        <div class="chat-body">
          ${cont ? '' : `<div class="chat-meta"><span class="chat-name" style="color:${esc(m.color)}">${esc(m.name)}</span>${m.kind === 'host' ? '<span class="host-tag">HOST</span>' : ''}<span class="chat-time">${esc(clockTime(m.at))}</span></div>`}
          <div class="chat-text">${esc(m.text)}</div>
        </div>
      </div>`;
    }
    function renderChat() {
      const box = $('chat-list');
      box.innerHTML = S.chat.length
        ? S.chat.map((m, i) => chatRowHtml(m, S.chat[i - 1])).join('')
        : `<p class="chat-empty">Say hi to ${esc(hostName())} and the others 👋</p>`;
      box.scrollTop = box.scrollHeight;
    }
    function appendChat(m) {
      const box = $('chat-list');
      const nearBottom = box.scrollHeight - box.scrollTop - box.clientHeight < 80;
      const prev = S.chat[S.chat.length - 1];
      S.chat.push(m);
      if (S.chat.length > 120) S.chat.shift();
      if (S.chat.length === 1) box.innerHTML = '';
      box.insertAdjacentHTML('beforeend', chatRowHtml(m, prev));
      if (nearBottom || m.pid === S.pid) box.scrollTop = box.scrollHeight;
      if (!chatVisible() && m.kind !== 'system' && m.pid !== S.pid) {
        setUnread(S.unread + 1);
        if (isPhone()) showTicker(m);
      }
    }
    function floatReaction(emoji, name) {
      const layer = $('float-layer');
      if (!layer || layer.childElementCount > 24) return;
      const el = document.createElement('div');
      el.className = 'float-emoji';
      el.style.left = `${10 + Math.random() * 72}%`;
      el.style.setProperty('--dx1', `${(Math.random() * 30 - 15).toFixed(0)}px`);
      el.style.setProperty('--dx2', `${(Math.random() * 80 - 40).toFixed(0)}px`);
      el.innerHTML = `<span class="fe-icon">${esc(emoji)}</span>${name ? `<span class="fe-name">${esc(name)}</span>` : ''}`;
      layer.appendChild(el);
      setTimeout(() => el.remove(), 2700);
    }
    async function postJson(path, body) {
      const r = await fetch(path, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) });
      const d = await r.json().catch(() => ({}));
      if (!r.ok) {
        const e = new Error(d.error || `Request failed (${r.status})`);
        e.code = d.code || '';
        e.status = r.status;
        throw e;
      }
      return d;
    }
    async function sendChat(ev) {
      ev.preventDefault();
      const input = $('chat-input');
      const text = input.value.trim();
      if (!text) return;
      $('chat-send').disabled = true;
      try {
        await postJson(`/api/lobby/${encodeURIComponent(CODE)}/chat`, { lid, text });
        input.value = '';
      } catch (e) {
        if (e.code === 'not_joined') scheduleRejoin(0);
        toast(e.message);
      } finally {
        $('chat-send').disabled = false;
        input.focus();
      }
    }
    async function sendReaction(emoji) {
      const now = Date.now();
      if (now - S.lastReactAt < 400) return;
      S.lastReactAt = now;
      floatReaction(emoji, 'You');
      try {
        await postJson(`/api/lobby/${encodeURIComponent(CODE)}/react`, { lid, emoji });
      } catch (e) {
        if (e.code === 'not_joined') scheduleRejoin(0);
      }
    }

    // ---------- Lyrics ----------
    async function loadLyrics() {
      const t = curTrack();
      const box = $('lyrics-box');
      if (!t) { box.innerHTML = '<p class="chat-empty">Lyrics show up here once a song is playing.</p>'; S.lyrics = null; S.lyricsFor = ''; return; }
      if (S.lyricsFor === t.id) return;
      S.lyricsFor = t.id;
      S.lyrics = null;
      S.lyricIdx = -1;
      box.innerHTML = '<p class="chat-empty">Loading lyrics…</p>';
      let data = null;
      try {
        const r = await fetch(`/api/lobby/${encodeURIComponent(CODE)}/lyrics/${encodeURIComponent(t.id)}?lid=${encodeURIComponent(lid)}`);
        if (r.ok) data = await r.json();
      } catch {}
      if (S.lyricsFor !== t.id) return;
      const lines = (data && data.lines) || [];
      S.lyrics = data && data.synced ? lines : null;
      if (!lines.length && !(data && data.plain)) {
        box.innerHTML = '<p class="chat-empty">No lyrics for this one. Just vibe.</p>';
        return;
      }
      box.innerHTML = lines.length
        ? `<div class="lyrics">${lines.map((l, i) => `<div class="lyric ${data.synced ? '' : 'plain'}" data-i="${i}">${esc(l.text || '♪')}</div>`).join('')}</div>`
        : `<div class="lyrics"><div class="lyric plain" style="white-space:pre-wrap">${esc(data.plain)}</div></div>`;
      syncLyrics();
    }
    function syncLyrics() {
      if (!S.lyrics || !S.lyrics.length) return;
      const ms = (shouldPlay() && !audio.paused ? currentSec() : expectedSec()) * 1000;
      let idx = -1;
      for (let i = 0; i < S.lyrics.length; i++) {
        const st = S.lyrics[i].start_ms;
        if (st != null && st <= ms + 150) idx = i; else if (st != null && st > ms + 150) break;
      }
      if (idx === S.lyricIdx) return;
      S.lyricIdx = idx;
      const rows = $('lyrics-box').querySelectorAll('.lyric');
      rows.forEach((el, i) => el.classList.toggle('active', i === idx));
      if (idx >= 0 && rows[idx]) rows[idx].scrollIntoView({ behavior: 'smooth', block: 'center' });
    }
    function setUnread(n) {
      S.unread = n;
      const label = n > 9 ? '9+' : String(n);
      for (const id of ['chat-dot', 'dock-chat-dot']) {
        $(id).textContent = label;
        $(id).classList.toggle('hidden', !n);
      }
    }
    function showTicker(m) {
      const el = $('chat-ticker');
      el.innerHTML = `${avatarHtml({ name: m.name, color: m.color, avatarUrl: m.avatarUrl }, 'sm')}<span class="tk-body"><b style="color:${esc(m.color)}">${esc(m.name)}</b><span class="tk-text">${esc(m.text)}</span></span>`;
      el.classList.remove('hidden');
      el.style.animation = 'none';
      void el.offsetWidth;
      el.style.animation = '';
      clearTimeout(S.tickerTimer);
      S.tickerTimer = setTimeout(hideTicker, 4500);
    }
    function hideTicker() {
      clearTimeout(S.tickerTimer);
      $('chat-ticker').classList.add('hidden');
    }
    function openSheet(tab) {
      S.sheetOpen = true;
      $('side').classList.add('open');
      $('sheet-backdrop').classList.add('open');
      hideTicker();
      setTab(tab);
    }
    function closeSheet() {
      if (!S.sheetOpen) return;
      S.sheetOpen = false;
      $('side').classList.remove('open');
      $('side').style.transform = '';
      $('sheet-backdrop').classList.remove('open');
      if (document.activeElement && $('side').contains(document.activeElement)) document.activeElement.blur();
    }
    function inviteFriend() {
      const url = location.href.split('#')[0];
      const title = (S.state && S.state.name) || 'Listen Together';
      if (typeof navigator.share === 'function') {
        navigator.share({ title, text: `Listen along with ${hostName()} on Beatlab`, url }).catch(() => {});
        return;
      }
      if (navigator.clipboard && navigator.clipboard.writeText) {
        navigator.clipboard.writeText(url).then(() => toast('Invite link copied'), () => toast(url));
      } else {
        toast(url);
      }
    }
    function setTab(tab) {
      S.tab = tab;
      document.querySelectorAll('.tab').forEach((b) => {
        const on = b.dataset.tab === tab;
        b.classList.toggle('active', on);
        b.setAttribute('aria-selected', on ? 'true' : 'false');
      });
      $('pane-chat').classList.toggle('hidden', tab !== 'chat');
      $('pane-next').classList.toggle('hidden', tab !== 'next');
      $('pane-lyrics').classList.toggle('hidden', tab !== 'lyrics');
      if (tab === 'chat' && chatVisible()) {
        setUnread(0);
        const box = $('chat-list');
        box.scrollTop = box.scrollHeight;
      }
      if (tab === 'lyrics') loadLyrics();
    }

    // ---------- Media Session ----------
    function updateMediaSession() {
      if (!('mediaSession' in navigator)) return;
      const t = curTrack();
      try {
        navigator.mediaSession.metadata = t
          ? new MediaMetadata({
              title: t.title,
              artist: t.artist,
              album: `${(S.state && S.state.name) || 'Listen Together'} · Beatlab`,
              artwork: [{ src: new URL(t.coverUrl, location.origin).href, sizes: '512x512' }],
            })
          : null;
      } catch {}
    }
    if ('mediaSession' in navigator) {
      try {
        navigator.mediaSession.setActionHandler('play', () => setLocalPaused(false));
        navigator.mediaSession.setActionHandler('pause', () => setLocalPaused(true));
        ['seekbackward', 'seekforward', 'seekto', 'previoustrack', 'nexttrack'].forEach((a) => {
          try { navigator.mediaSession.setActionHandler(a, null); } catch {}
        });
      } catch {}
    }
    function setLocalPaused(paused) {
      S.localPaused = paused;
      if (paused) {
        pauseAudio();
      } else {
        S.blocked = false;
        $('tap-resume').classList.add('hidden');
        const t = curTrack();
        if (t && t.id !== S.loadedId) loadTrack(t);
        else if (shouldPlay()) { seekTo(expectedSec() + 0.2); playAudio(); }
      }
      renderTransport();
    }

    // ---------- Connection ----------
    function setConn() { renderTransport(); }
    function connect() {
      if (S.es) { try { S.es.close(); } catch {} }
      const es = new EventSource(`/api/lobby/${encodeURIComponent(CODE)}/events?lid=${encodeURIComponent(lid)}`);
      S.es = es;
      es.addEventListener('init', (e) => {
        const d = parse(e.data);
        if (!d) return;
        S.pid = d.pid;
        S.chat = d.chat || [];
        S.rejoinDelay = 2000;
        renderChat();
        applyState(d.state);
      });
      es.addEventListener('state', (e) => { const d = parse(e.data); if (d) applyState(d); });
      es.addEventListener('chat', (e) => { const m = parse(e.data); if (m) appendChat(m); });
      es.addEventListener('react', (e) => {
        const r = parse(e.data);
        if (r && r.pid !== S.pid) floatReaction(r.emoji, r.name);
      });
      es.addEventListener('kicked', () => { teardown(); showNotice('kicked'); });
      es.addEventListener('ended', () => { teardown(); showNotice('ended'); });
      es.onopen = setConn;
      es.onerror = () => {
        setConn();
        if (es.readyState === 2 && S.es === es) {
          S.es = null;
          scheduleRejoin();
        }
      };
    }
    function scheduleRejoin(delay) {
      if (!S.joined || S.rejoinTimer) return;
      const wait = typeof delay === 'number' ? delay : S.rejoinDelay;
      S.rejoinTimer = setTimeout(async () => {
        S.rejoinTimer = null;
        try {
          const d = await postJoin(store.get('beatlab_lobby_name') || '');
          S.pid = d.pid;
          S.chat = d.chat || [];
          renderChat();
          applyState(d.state);
          connect();
        } catch (e) {
          if (['ended', 'missing', 'kicked', 'locked'].includes(e.code)) {
            teardown();
            showNotice(e.code);
            return;
          }
          S.rejoinDelay = Math.min(20000, S.rejoinDelay * 2);
          scheduleRejoin();
        }
      }, wait);
    }
    function applyState(st) {
      if (!st) return;
      const prevTrack = curTrack();
      S.state = st;
      renderAll();
      if (S.joined) {
        syncAudio();
        const t = curTrack();
        if ((prevTrack && prevTrack.id) !== (t && t.id) && S.tab === 'lyrics') loadLyrics();
      }
    }
    function teardown() {
      S.joined = false;
      document.body.classList.remove('in-room');
      closeSheet();
      hideTicker();
      if (S.es) { try { S.es.close(); } catch {} S.es = null; }
      clearTimeout(S.rejoinTimer);
      S.rejoinTimer = null;
      S.loops.forEach((id) => clearInterval(id));
      S.loops = [];
      S.loadedId = '';
      try { audio.pause(); audio.removeAttribute('src'); audio.load(); } catch {}
      $('tap-resume').classList.add('hidden');
    }
    function startLoops() {
      S.loops.forEach((id) => clearInterval(id));
      S.loops = [
        setInterval(() => { renderProgress(); if (S.tab === 'lyrics') syncLyrics(); }, 250),
        setInterval(() => {
          if (!S.joined) return;
          resumeIfNeeded();
          correctDrift();
          renderTransport();
        }, 1000),
        setInterval(() => syncClock(3), 120000),
      ];
    }

    // ---------- Gate / join ----------
    function showGateError(msg) {
      const el = $('gate-error');
      el.textContent = msg;
      el.classList.toggle('hidden', !msg);
    }
    async function postJoin(name) {
      const headers = { 'Content-Type': 'application/json' };
      const tok = store.get('beatlab_token');
      if (tok && S.user) headers.Authorization = `Bearer ${tok}`;
      const r = await fetch(`/api/lobby/${encodeURIComponent(CODE)}/join`, { method: 'POST', headers, body: JSON.stringify({ lid, name }) });
      const d = await r.json().catch(() => ({}));
      if (!r.ok) {
        const e = new Error(d.error || 'Could not join the lobby.');
        e.code = d.code || '';
        throw e;
      }
      return d;
    }
    async function join(ev) {
      ev.preventDefault();
      const input = $('gate-name');
      const name = input.value.trim();
      if (!S.user && !name) {
        showGateError("Pick a name so friends know who's here.");
        input.focus();
        return;
      }
      if (!S.user) store.set('beatlab_lobby_name', name);
      showGateError('');
      unlockAudio();
      const btn = $('gate-join');
      btn.disabled = true;
      btn.querySelector('span').textContent = 'Joining…';
      try {
        const d = await postJoin(name);
        S.pid = d.pid;
        S.chat = d.chat || [];
        S.joined = true;
        S.state = d.state;
        $('gate').classList.add('hidden');
        $('room').classList.remove('hidden');
        document.body.classList.add('in-room');
        renderChat();
        renderAll();
        syncAudio();
        connect();
        startLoops();
      } catch (e) {
        if (['ended', 'missing', 'kicked'].includes(e.code)) {
          showNotice(e.code);
        } else {
          showGateError(e.code ? e.message : "Couldn't reach Beatlab. Check your connection and try again.");
        }
        try { audio.pause(); } catch {}
      } finally {
        btn.disabled = false;
        btn.querySelector('span').textContent = 'Join & Listen';
      }
    }
    function showNotice(kind) {
      $('gate').classList.add('hidden');
      $('room').classList.add('hidden');
      $('notice').classList.remove('hidden');
      $('live-pill').classList.add('hidden');
      setTint('');
      const host = hostName();
      const copy = {
        ended: [ICON.ended, 'This lobby has ended', S.state ? `Thanks for listening with ${host}. Start your own lobby on Beatlab and bring friends along.` : 'The host closed this lobby. Start your own on Beatlab and bring friends along.'],
        missing: [ICON.missing, 'Lobby not found', "This link doesn't exist or has expired. Ask your friend for a fresh invite."],
        kicked: [ICON.blocked, "You've left the lobby", 'The host removed you from this listening session.'],
        locked: [ICON.lock, 'Lobby locked', `${host} locked this lobby, so new listeners can't join right now.`],
      }[kind] || [ICON.missing, 'Something went wrong', 'Try opening the link again.'];
      $('notice-icon').innerHTML = copy[0];
      $('notice-title').textContent = copy[1];
      $('notice-text').textContent = copy[2];
      $('notice-retry').classList.toggle('hidden', kind !== 'locked');
      document.title = `${copy[1]} — Beatlab`;
    }
    async function detectMember() {
      const tok = store.get('beatlab_token');
      if (!tok) return;
      try {
        const r = await fetch('/api/web/me', { headers: { Authorization: `Bearer ${tok}` } });
        if (!r.ok) return;
        const d = await r.json();
        if (!d.user) return;
        S.user = d.user;
        $('get-app').textContent = 'Open Beatlab';
        if (S.joined) return;
        $('gate-name-wrap').classList.add('hidden');
        const chip = $('gate-member');
        chip.innerHTML = `${avatarHtml({ name: d.user.username, avatarUrl: d.user.avatarUrl, color: '#1ed760' }, 'sm')}<div><span>Joining as</span> <b>@${esc(d.user.username)}</b></div>`;
        chip.classList.remove('hidden');
        if (S.state && S.state.host && S.state.host.username === d.user.username) $('host-note').classList.remove('hidden');
      } catch {}
    }

    // ---------- Wiring ----------
    $('gate-form').addEventListener('submit', join);
    $('gate-name').value = store.get('beatlab_lobby_name') || '';
    $('chat-form').addEventListener('submit', sendChat);
    $('reactions').addEventListener('click', (e) => {
      const b = e.target.closest('[data-emoji]');
      if (b) sendReaction(b.dataset.emoji);
    });
    document.querySelectorAll('.tab').forEach((b) => b.addEventListener('click', () => setTab(b.dataset.tab)));
    document.querySelectorAll('[data-open]').forEach((b) => b.addEventListener('click', () => openSheet(b.dataset.open)));
    $('chat-ticker').addEventListener('click', () => openSheet('chat'));
    $('sheet-close').addEventListener('click', closeSheet);
    $('sheet-backdrop').addEventListener('click', closeSheet);
    $('btn-invite').addEventListener('click', inviteFriend);
    document.addEventListener('keydown', (e) => { if (e.key === 'Escape') closeSheet(); });
    MQ_PHONE.addEventListener('change', () => { if (!isPhone()) { closeSheet(); hideTicker(); } fitArt(); });
    if (typeof ResizeObserver === 'function') new ResizeObserver(fitArt).observe(document.querySelector('.art-zone'));
    window.addEventListener('resize', fitArt);
    // Swipe the sheet down by its handle to close it.
    (() => {
      const head = $('sheet-head');
      const side = $('side');
      let startY = null;
      let dy = 0;
      head.addEventListener('touchstart', (e) => { startY = e.touches[0].clientY; dy = 0; side.classList.add('dragging'); }, { passive: true });
      head.addEventListener('touchmove', (e) => {
        if (startY === null) return;
        dy = Math.max(0, e.touches[0].clientY - startY);
        side.style.transform = `translateY(${dy}px)`;
      }, { passive: true });
      const end = () => {
        if (startY === null) return;
        side.classList.remove('dragging');
        side.style.transform = '';
        if (dy > 90) closeSheet();
        startY = null;
      };
      head.addEventListener('touchend', end);
      head.addEventListener('touchcancel', end);
    })();
    $('btn-listen').addEventListener('click', () => setLocalPaused(!S.localPaused));
    $('tap-resume-btn').addEventListener('click', () => {
      S.blocked = false;
      $('tap-resume').classList.add('hidden');
      playAudio();
    });
    $('notice-retry').addEventListener('click', () => location.reload());
    $('btn-mute').addEventListener('click', () => {
      S.muted = !S.muted;
      if (!S.muted && S.volume === 0) S.volume = 0.6;
      applyVolume();
    });
    $('vol').addEventListener('input', (e) => {
      S.volume = Math.max(0, Math.min(1, parseFloat(e.target.value) || 0));
      S.muted = S.volume === 0;
      store.set('beatlab_vol', String(S.volume));
      applyVolume(0.015);
    });
    window.addEventListener('pagehide', () => {
      if (!S.joined || !navigator.sendBeacon) return;
      try {
        navigator.sendBeacon(`/api/lobby/${encodeURIComponent(CODE)}/leave`, new Blob([JSON.stringify({ lid })], { type: 'application/json' }));
      } catch {}
    });
    window.addEventListener('pageshow', (e) => {
      if (e.persisted && S.joined) connect();
    });
    document.addEventListener('visibilitychange', () => {
      if (document.visibilityState === 'visible' && S.joined) {
        syncClock(2).then(() => { syncAudio(); forceSync(); renderAll(); });
      }
    });

    applyVolume();
    if (S.status === 'live' && S.state) {
      $('gate').classList.remove('hidden');
      renderAll();
      syncClock(4);
      detectMember();
    } else {
      showNotice(S.status === 'ended' ? 'ended' : 'missing');
    }
  })();
  </script>
</body>
</html>
"##;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lobby_page_escapes_meta_and_boot_json() {
        let snap = json!({
            "name": "Late </script><script>alert(1)</script> Mix",
            "listenerCount": 2,
            "host": { "displayName": "Ana \"DJ\" <b>", "username": "ana" },
            "playback": { "track": { "title": "Song & Dance", "artist": "<Band>" } },
        });
        let html = render_lobby_html("https://music.example.com", "abc23456", "live", &snap);
        assert!(!html.contains("</script><script>alert(1)"));
        assert!(html.contains("Listen along with Ana &quot;DJ&quot; &lt;b&gt; on Beatlab"));
        assert!(html.contains("https://music.example.com/l/abc23456/cover"));
        assert!(html.contains("Now playing: Song &amp; Dance — &lt;Band&gt;"));
        // The boot payload is valid JSON once the browser unescapes nothing:
        let start = html.find("<script id=\"lobby-boot\" type=\"application/json\">").unwrap();
        let rest = &html[start..];
        let body = &rest[rest.find('>').unwrap() + 1..rest.find("</script>").unwrap()];
        let parsed: Value = serde_json::from_str(body).unwrap();
        assert_eq!(parsed["code"], "abc23456");
        assert_eq!(parsed["state"]["name"], snap["name"]);
    }

    #[test]
    fn ended_and_missing_pages_skip_live_state() {
        let html = render_lobby_html("https://x.test", "zzzz", "ended", &Value::Null);
        assert!(html.contains("This listening lobby has ended"));
        assert!(html.contains("\"status\":\"ended\""));
        let html = render_lobby_html("https://x.test", "", "missing", &Value::Null);
        assert!(html.contains("Listening lobby not found"));
    }
}
