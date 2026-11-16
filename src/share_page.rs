use crate::models::{ShareRecord, Track};
use serde_json::{json, Value};

pub fn html_escape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

fn safe_json_for_script(val: &Value) -> String {
    val.to_string()
        .replace("</", "<\\/")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

fn fmt_dur(secs: i32) -> String {
    if secs <= 0 {
        return "0:00".to_string();
    }
    let m = secs / 60;
    let s = secs % 60;
    format!("{}:{:02}", m, s)
}

fn fmt_total_dur(secs: i32) -> String {
    if secs <= 0 {
        return "0 min".to_string();
    }
    let hrs = secs / 3600;
    let mins = (secs % 3600) / 60;
    if hrs > 0 {
        format!("{} hr {} min", hrs, mins)
    } else {
        format!("{} min", mins.max(1))
    }
}

fn share_kind_label(kind: &str) -> &'static str {
    match kind {
        "track" => "SHARED SONG",
        "mix" => "SHARED INSTANT MIX",
        "playlist" => "SHARED PLAYLIST",
        "album" => "SHARED ALBUM",
        "artist" => "SHARED ARTIST",
        _ => "SHARED ON BEATLAB",
    }
}

pub fn build_share_meta(share: &ShareRecord, tracks: &[Track]) -> (String, String) {
    let total_secs: i32 = tracks.iter().map(|t| t.duration.max(0)).sum();
    if share.kind == "track" {
        if let Some(t) = tracks.first() {
            let title = format!("{} — {}", t.title, t.artist);
            let mut parts: Vec<String> = Vec::new();
            if !t.album.trim().is_empty() {
                parts.push(t.album.trim().to_string());
            }
            if t.duration > 0 {
                parts.push(fmt_dur(t.duration));
            }
            parts.push(t.quality_badge());
            if t.year > 0 {
                parts.push(t.year.to_string());
            }
            if !t.genre.trim().is_empty() {
                parts.push(t.genre.trim().to_string());
            }
            parts.push("Listen on Beatlab".to_string());
            return (title, parts.join(" · "));
        }
    }

    let title = if !share.title.trim().is_empty() {
        share.title.trim().to_string()
    } else {
        "Beatlab Share".to_string()
    };

    let mut header_parts: Vec<String> = Vec::new();
    match share.kind.as_str() {
        "mix" => header_parts.push("Instant Mix".to_string()),
        "playlist" => header_parts.push("Playlist".to_string()),
        "album" => header_parts.push("Album".to_string()),
        "artist" => header_parts.push("Artist".to_string()),
        _ => {}
    }
    if !share.subtitle.trim().is_empty() {
        header_parts.push(share.subtitle.trim().to_string());
    }
    if !tracks.is_empty() {
        header_parts.push(format!(
            "{} track{}",
            tracks.len(),
            if tracks.len() == 1 { "" } else { "s" }
        ));
    }
    if total_secs > 0 {
        header_parts.push(fmt_total_dur(total_secs));
    }
    header_parts.push("Listen on Beatlab".to_string());

    let preview_lines: Vec<String> = tracks
        .iter()
        .take(4)
        .enumerate()
        .map(|(i, t)| format!("{}. {} — {}", i + 1, t.title, t.artist))
        .collect();
    let mut desc = header_parts.join(" · ");
    if !preview_lines.is_empty() {
        desc.push('\n');
        desc.push_str(&preview_lines.join("\n"));
        if tracks.len() > 4 {
            desc.push_str(&format!("\n+ {} more tracks", tracks.len() - 4));
        }
    }
    (title, desc)
}

pub fn render_share_html(
    origin: &str,
    share: &ShareRecord,
    tracks: &[Track],
    has_embed_video: bool,
    primary_audio_mime: &str,
) -> String {
    let (og_title, og_desc) = build_share_meta(share, tracks);
    let page_url = format!("{}/s/{}", origin, share.id);
    let cover_url = format!("{}/s/{}/cover", origin, share.id);
    let stream_url = format!("{}/s/{}/stream", origin, share.id);
    let embed_mp4_url = format!("{}/s/{}/embed.mp4", origin, share.id);
    let oembed_url = format!("{}/s/{}/oembed", origin, share.id);

    let use_video_embed = has_embed_video && share.kind == "track" && !tracks.is_empty();
    let og_type = if use_video_embed {
        "video.other"
    } else if share.kind == "track" {
        "music.song"
    } else if share.kind == "album" {
        "music.album"
    } else {
        "music.playlist"
    };

    let media_meta_tags = if use_video_embed {
        format!(
            r#"
  <meta property="og:video" content="{video}" />
  <meta property="og:video:secure_url" content="{video}" />
  <meta property="og:video:type" content="video/mp4" />
  <meta property="og:video:width" content="480" />
  <meta property="og:video:height" content="480" />
  <meta property="og:audio" content="{audio}" />
  <meta property="og:audio:secure_url" content="{audio}" />
  <meta property="og:audio:type" content="{audio_mime}" />
  <meta name="twitter:card" content="player" />
  <meta name="twitter:player" content="{video}" />
  <meta name="twitter:player:width" content="480" />
  <meta name="twitter:player:height" content="480" />"#,
            video = html_escape(&embed_mp4_url),
            audio = html_escape(&stream_url),
            audio_mime = html_escape(primary_audio_mime),
        )
    } else if !tracks.is_empty() {
        format!(
            r#"
  <meta property="og:audio" content="{audio}" />
  <meta property="og:audio:secure_url" content="{audio}" />
  <meta property="og:audio:type" content="{audio_mime}" />
  <meta name="twitter:card" content="summary" />"#,
            audio = html_escape(&stream_url),
            audio_mime = html_escape(primary_audio_mime),
        )
    } else {
        r#"  <meta name="twitter:card" content="summary" />"#.to_string()
    };

    let tracks_json: Vec<Value> = tracks
        .iter()
        .map(|t| {
            json!({
                "id": t.id,
                "title": t.title,
                "artist": t.artist,
                "album": t.album,
                "genre": t.genre,
                "year": t.year,
                "duration": t.duration,
                "format": t.format,
                "bitrate": t.bitrate,
                "sampleRate": t.sample_rate,
                "bitDepth": t.bit_depth,
                "isLossless": t.is_lossless(),
                "qualityBadge": t.quality_badge(),
                "coverUrl": format!("/api/web/cover/{}?v=2", urlencoding::encode(&t.id)),
                "streamUrl": format!("/s/{}/stream/{}", share.id, urlencoding::encode(&t.id)),
            })
        })
        .collect();

    let payload = json!({
        "id": share.id,
        "kind": share.kind,
        "kindLabel": share_kind_label(&share.kind),
        "targetId": share.target_id,
        "title": if share.title.trim().is_empty() { og_title.clone() } else { share.title.clone() },
        "subtitle": share.subtitle,
        "creatorName": share.creator_name,
        "coverUrl": format!("/s/{}/cover", share.id),
        "shareUrl": page_url,
        "tracks": tracks_json,
    });

    let safe_payload = safe_json_for_script(&payload);
    let kind_badge = share_kind_label(&share.kind);
    let initial_title = tracks
        .first()
        .filter(|_| share.kind == "track")
        .map(|t| t.title.as_str())
        .unwrap_or(if share.title.trim().is_empty() {
            "Shared on Beatlab"
        } else {
            share.title.as_str()
        });
    let initial_sub = if share.kind == "track" {
        tracks
            .first()
            .map(|t| {
                if t.album.trim().is_empty() {
                    t.artist.clone()
                } else {
                    format!("{} · {}", t.artist, t.album)
                }
            })
            .unwrap_or_default()
    } else if !share.subtitle.trim().is_empty() {
        share.subtitle.clone()
    } else {
        format!("{} tracks", tracks.len())
    };

    format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0, viewport-fit=cover" />
  <meta name="theme-color" content="#1ed760" />
  <title>{esc_page_title} — Beatlab</title>
  <link rel="icon" type="image/svg+xml" href="/web/favicon.svg" />
  <link rel="apple-touch-icon" sizes="180x180" href="/web/apple-touch-icon.png" />

  <meta property="og:site_name" content="Beatlab" />
  <meta property="og:type" content="{og_type}" />
  <meta property="og:title" content="{esc_og_title}" />
  <meta property="og:description" content="{esc_og_desc}" />
  <meta property="og:url" content="{esc_url}" />
  <meta property="og:image" content="{esc_cover}" />
  <meta property="og:image:secure_url" content="{esc_cover}" />
  <meta property="og:image:width" content="600" />
  <meta property="og:image:height" content="600" />
  <meta property="og:image:alt" content="{esc_og_title}" />
  <meta name="twitter:title" content="{esc_og_title}" />
  <meta name="twitter:description" content="{esc_og_desc}" />
  <meta name="twitter:image" content="{esc_cover}" />
  <link rel="alternate" type="application/json+oembed" href="{esc_oembed}" title="{esc_og_title}" />
{media_meta_tags}
  <style>
    :root {{
      --bg: #0c0e13;
      --surface: #12151c;
      --surface-2: #191d26;
      --surface-3: #222733;
      --border: #232834;
      --border-bright: #343b4c;
      --text: #f3f5f8;
      --text-muted: #98a2b3;
      --text-dim: #6b7485;
      --accent: #1ed760;
      --accent-hover: #2be66f;
      --accent-blue: #38bdf8;
      --font: system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    }}
    * {{ box-sizing: border-box; margin: 0; padding: 0; }}
    html, body {{
      min-height: 100%;
      background: var(--bg);
      color: var(--text);
      font-family: var(--font);
      font-size: 14px;
      line-height: 1.45;
      -webkit-font-smoothing: antialiased;
    }}
    a {{ color: inherit; text-decoration: none; }}
    button, input {{ font-family: inherit; color: inherit; }}
    button {{ cursor: pointer; border: none; background: none; }}
    .hidden {{ display: none !important; }}

    .shell {{
      position: relative;
      z-index: 1;
      max-width: 960px;
      margin: 0 auto;
      padding: 18px 18px 48px;
      display: flex;
      flex-direction: column;
      gap: 18px;
    }}

    .topbar {{
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 12px;
      flex-wrap: wrap;
      padding: 10px 14px;
      background: var(--surface);
      border: 1px solid var(--border);
      border-radius: 6px;
    }}
    .brand {{
      display: inline-flex;
      align-items: center;
      gap: 10px;
      font-weight: 800;
      font-size: 17px;
      letter-spacing: -0.02em;
    }}
    .brand svg {{ width: 28px; height: 28px; display: block; }}
    .topbar-right {{
      display: flex;
      align-items: center;
      gap: 8px;
      flex-wrap: wrap;
    }}
    .btn-primary {{
      background: var(--accent);
      color: #06120b;
      font-weight: 700;
      padding: 8px 16px;
      border-radius: 6px;
      display: inline-flex;
      align-items: center;
      justify-content: center;
      gap: 7px;
      font-size: 13.5px;
      transition: background 0.14s, transform 0.1s;
    }}
    .btn-primary:hover {{ background: var(--accent-hover); }}
    .btn-primary:active {{ transform: scale(0.98); }}
    .btn-secondary {{
      background: var(--surface-2);
      border: 1px solid var(--border-bright);
      color: var(--text);
      font-weight: 600;
      padding: 8px 14px;
      border-radius: 6px;
      display: inline-flex;
      align-items: center;
      justify-content: center;
      gap: 7px;
      font-size: 13px;
      transition: background 0.14s, border-color 0.14s;
    }}
    .btn-secondary:hover {{ background: var(--surface-3); }}
    .btn-secondary.active {{ border-color: var(--accent); color: var(--accent); }}
    .user-chip {{
      display: inline-flex;
      align-items: center;
      gap: 7px;
      padding: 4px 11px 4px 5px;
      border-radius: 6px;
      background: var(--surface-2);
      border: 1px solid var(--border);
      font-size: 13px;
      font-weight: 600;
    }}
    .avatar-dot {{
      width: 24px;
      height: 24px;
      border-radius: 4px;
      background: var(--accent);
      color: #06120b;
      font-weight: 800;
      font-size: 12px;
      display: grid;
      place-items: center;
    }}

    .player-card {{
      background: var(--surface);
      border: 1px solid var(--border);
      border-radius: 6px;
      padding: 22px;
      display: grid;
      grid-template-columns: 248px minmax(0, 1fr);
      gap: 22px;
      align-items: center;
    }}
    .art-box {{
      position: relative;
      width: 100%;
      aspect-ratio: 1 / 1;
      border-radius: 6px;
      overflow: hidden;
      border: 1px solid var(--border);
      background: var(--surface-2);
    }}
    .art-box img {{
      width: 100%;
      height: 100%;
      object-fit: cover;
      display: block;
    }}
    .art-live-pill {{
      position: absolute;
      left: 10px;
      bottom: 10px;
      background: var(--bg);
      border: 1px solid var(--border-bright);
      color: var(--accent);
      font-size: 10.5px;
      font-weight: 700;
      letter-spacing: 0.04em;
      padding: 3px 8px;
      border-radius: 4px;
    }}

    .meta-col {{
      display: flex;
      flex-direction: column;
      gap: 12px;
      min-width: 0;
    }}
    .kicker-row {{
      display: flex;
      align-items: center;
      gap: 8px;
      flex-wrap: wrap;
    }}
    .kicker {{
      font-size: 11px;
      font-weight: 800;
      letter-spacing: 0.08em;
      color: var(--accent);
      text-transform: uppercase;
    }}
    .shared-by {{
      font-size: 12px;
      color: var(--text-muted);
    }}
    .track-title {{
      font-size: clamp(22px, 3.6vw, 32px);
      font-weight: 800;
      letter-spacing: -0.03em;
      line-height: 1.12;
      overflow-wrap: anywhere;
    }}
    .track-sub {{
      font-size: 15px;
      color: var(--text-muted);
      overflow-wrap: anywhere;
    }}
    .badges-row {{
      display: flex;
      flex-wrap: wrap;
      gap: 6px;
      align-items: center;
    }}
    .badge {{
      font-size: 11px;
      font-weight: 700;
      padding: 3px 9px;
      border-radius: 6px;
      background: var(--surface-2);
      border: 1px solid var(--border);
      color: var(--text-muted);
    }}
    .badge.quality {{
      background: rgba(56, 189, 248, 0.14);
      border-color: rgba(56, 189, 248, 0.3);
      color: var(--accent-blue);
    }}
    .badge.quality.lossless {{
      background: rgba(245, 158, 11, 0.16);
      border-color: rgba(245, 158, 11, 0.35);
      color: #fbbf24;
    }}

    .transport-block {{
      display: flex;
      flex-direction: column;
      gap: 10px;
      margin-top: 4px;
      padding: 14px 16px;
      background: var(--surface-2);
      border: 1px solid var(--border);
      border-radius: 6px;
    }}
    .progress-row {{
      display: flex;
      align-items: center;
      gap: 10px;
    }}
    .time-lbl {{
      font-size: 11.5px;
      color: var(--text-muted);
      font-variant-numeric: tabular-nums;
      min-width: 36px;
    }}
    .time-lbl.right {{ text-align: right; }}
    input[type="range"] {{
      accent-color: var(--accent);
      cursor: pointer;
    }}
    .seek-slider {{
      flex: 1;
      height: 5px;
      min-width: 0;
    }}
    .controls-row {{
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 12px;
      flex-wrap: wrap;
    }}
    .controls-left {{
      display: flex;
      align-items: center;
      gap: 10px;
    }}
    .play-main {{
      width: 48px;
      height: 48px;
      border-radius: 50%;
      background: var(--accent);
      color: #06120b;
      display: inline-flex;
      align-items: center;
      justify-content: center;
      transition: transform 0.1s, background 0.14s;
      flex-shrink: 0;
    }}
    .play-main:hover {{ background: var(--accent-hover); transform: scale(1.04); }}
    .play-main svg {{ width: 22px; height: 22px; display: block; }}
    .icon-btn {{
      width: 36px;
      height: 36px;
      border-radius: 8px;
      display: inline-flex;
      align-items: center;
      justify-content: center;
      color: var(--text-muted);
      transition: background 0.14s, color 0.14s;
    }}
    .icon-btn:hover {{ background: var(--surface-2); color: var(--text); }}
    .icon-btn.active {{ color: var(--accent); }}
    .icon-btn svg {{ width: 19px; height: 19px; display: block; }}
    .vol-group {{
      display: flex;
      align-items: center;
      gap: 6px;
    }}
    .vol-slider {{ width: 84px; height: 4px; }}
    .visualizer {{
      width: 100%;
      height: 40px;
      display: block;
      margin-top: 2px;
    }}

    .actions-row {{
      display: flex;
      flex-wrap: wrap;
      gap: 8px;
      align-items: center;
    }}

    .card-panel {{
      background: var(--surface);
      border: 1px solid var(--border);
      border-radius: 6px;
      padding: 18px 20px;
    }}
    .panel-head {{
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 10px;
      flex-wrap: wrap;
      margin-bottom: 12px;
    }}
    .panel-head h2 {{ font-size: 17px; font-weight: 700; }}
    .panel-sub {{ font-size: 12.5px; color: var(--text-muted); }}

    .track-list {{
      display: flex;
      flex-direction: column;
      gap: 3px;
    }}
    .t-row {{
      display: grid;
      grid-template-columns: 32px minmax(0, 1fr) auto auto;
      align-items: center;
      gap: 12px;
      padding: 8px 10px;
      border-radius: 8px;
      border: 1px solid transparent;
      cursor: pointer;
      transition: background 0.13s, border-color 0.13s;
    }}
    .t-row:hover {{ background: rgba(255,255,255,0.04); }}
    .t-row.playing {{
      background: rgba(30, 215, 96, 0.1);
      border-color: rgba(30, 215, 96, 0.28);
    }}
    .t-row.playing .t-title {{ color: var(--accent); }}
    .t-idx {{
      text-align: center;
      font-size: 12.5px;
      color: var(--text-dim);
      font-variant-numeric: tabular-nums;
    }}
    .t-main {{
      display: flex;
      align-items: center;
      gap: 11px;
      min-width: 0;
    }}
    .t-main img {{
      width: 40px;
      height: 40px;
      border-radius: 6px;
      object-fit: cover;
      flex-shrink: 0;
      background: var(--surface-2);
    }}
    .t-meta {{ min-width: 0; flex: 1; }}
    .t-title {{
      font-weight: 600;
      font-size: 13.5px;
      white-space: nowrap;
      overflow: hidden;
      text-overflow: ellipsis;
    }}
    .t-sub {{
      font-size: 12px;
      color: var(--text-muted);
      white-space: nowrap;
      overflow: hidden;
      text-overflow: ellipsis;
    }}
    .t-dur {{
      font-size: 12px;
      color: var(--text-muted);
      font-variant-numeric: tabular-nums;
    }}

    .lyrics-box {{
      max-height: 290px;
      overflow-y: auto;
      display: flex;
      flex-direction: column;
      gap: 8px;
      padding-right: 4px;
    }}
    .lyric-line {{
      font-size: 14.5px;
      color: var(--text-dim);
      padding: 4px 8px;
      border-radius: 6px;
    }}
    .lyric-line.synced {{ cursor: pointer; }}
    .lyric-line.synced:hover {{ background: var(--surface-2); color: var(--text-muted); }}
    .lyric-line.active {{
      color: #fff;
      font-weight: 700;
      font-size: 15.5px;
      background: rgba(30, 215, 96, 0.14);
    }}

    .cta-banner {{
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 16px;
      flex-wrap: wrap;
      padding: 18px 22px;
      background: var(--surface);
      border: 1px solid var(--border-bright);
      border-left: 3px solid var(--accent);
      border-radius: 6px;
    }}
    .cta-banner h3 {{ font-size: 17px; font-weight: 800; margin-bottom: 3px; }}
    .cta-banner p {{ font-size: 13px; color: var(--text-muted); }}
    .cta-actions {{ display: flex; gap: 10px; flex-wrap: wrap; }}

    .modal-backdrop {{
      position: fixed;
      inset: 0;
      z-index: 200;
      background: rgba(0, 0, 0, 0.72);
      display: grid;
      place-items: center;
      padding: 16px;
    }}
    .modal-box {{
      width: 100%;
      max-width: 400px;
      background: var(--surface);
      border: 1px solid var(--border-bright);
      border-radius: 6px;
      padding: 24px;
      display: flex;
      flex-direction: column;
      gap: 14px;
    }}
    .modal-head {{
      display: flex;
      align-items: center;
      justify-content: space-between;
    }}
    .modal-head h3 {{ font-size: 18px; font-weight: 800; }}
    .auth-form {{ display: flex; flex-direction: column; gap: 12px; }}
    .auth-form label {{
      display: flex;
      flex-direction: column;
      gap: 5px;
      font-size: 12px;
      font-weight: 600;
      color: var(--text-muted);
    }}
    .auth-form input {{
      width: 100%;
      background: var(--bg);
      border: 1px solid var(--border);
      border-radius: 6px;
      padding: 10px 12px;
      font-size: 14px;
      outline: none;
    }}
    .auth-form input:focus {{ border-color: var(--accent); }}
    .auth-err {{
      background: rgba(244, 63, 94, 0.14);
      border: 1px solid rgba(244, 63, 94, 0.35);
      color: #fda4af;
      padding: 8px 10px;
      border-radius: 6px;
      font-size: 12.5px;
    }}
    .toast {{
      position: fixed;
      left: 50%;
      bottom: 24px;
      transform: translateX(-50%);
      z-index: 300;
      background: var(--text);
      border: 1px solid var(--border-bright);
      color: var(--bg);
      padding: 9px 18px;
      border-radius: 6px;
      font-size: 13px;
      font-weight: 700;
    }}

    @media (max-width: 720px) {{
      .player-card {{
        grid-template-columns: 1fr;
        padding: 18px;
        gap: 16px;
      }}
      .art-box {{
        max-width: 240px;
        margin: 0 auto;
      }}
      .vol-group {{ display: none; }}
      .t-row {{
        grid-template-columns: 24px minmax(0, 1fr) auto;
        gap: 8px;
        padding: 7px 6px;
      }}
      .t-row .badge.quality {{ display: none; }}
    }}
  </style>
</head>
<body>
  <div class="shell">
    <header class="topbar">
      <a class="brand" href="/">
        <svg viewBox="0 0 32 32" fill="none"><circle cx="16" cy="16" r="15" fill="#1ed760"/><path d="M12 10.5v11l9-5.5-9-5.5z" fill="#08120c"/></svg>
        <span>Beatlab</span>
      </a>
      <div id="topbar-auth" class="topbar-right">
        <button class="btn-secondary" onclick="ShareApp.openAuth('signin')">Sign In</button>
        <button class="btn-primary" onclick="ShareApp.openAuth('signup')">Sign Up Free</button>
      </div>
    </header>

    <section class="player-card">
      <div class="art-box">
        <img id="cover-img" src="/s/{esc_id}/cover" alt="Cover Art" />
        <span id="live-pill" class="art-live-pill hidden">PLAYING</span>
      </div>
      <div class="meta-col">
        <div class="kicker-row">
          <span class="kicker">{esc_kind_badge}</span>
          <span class="shared-by">· Shared by <strong>{esc_creator}</strong></span>
        </div>
        <h1 id="now-title" class="track-title">{esc_init_title}</h1>
        <div id="now-sub" class="track-sub">{esc_init_sub}</div>
        <div id="badges-row" class="badges-row"></div>

        <div class="transport-block">
          <div class="progress-row">
            <span id="elapsed-lbl" class="time-lbl">0:00</span>
            <input id="seek-slider" class="seek-slider" type="range" min="0" max="100" value="0" step="0.1" />
            <span id="duration-lbl" class="time-lbl right">0:00</span>
          </div>
          <div class="controls-row">
            <div class="controls-left">
              <button id="btn-shuffle" class="icon-btn hidden" title="Shuffle" onclick="ShareApp.toggleShuffle()">
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.1" stroke-linecap="round" stroke-linejoin="round"><polyline points="16 3 21 3 21 8"/><line x1="3" y1="20" x2="21" y2="3"/><polyline points="21 16 21 21 16 21"/><line x1="15" y1="15" x2="21" y2="21"/><line x1="3" y1="4" x2="9" y2="10"/></svg>
              </button>
              <button id="btn-prev" class="icon-btn hidden" title="Previous" onclick="ShareApp.prev()">
                <svg viewBox="0 0 24 24" fill="currentColor"><path d="M5 4.5h2.5v15H5zm4 7.5L19.5 4.5v15z"/></svg>
              </button>
              <button id="btn-play" class="play-main" title="Play / Pause" onclick="ShareApp.togglePlay()">
                <svg id="play-svg" viewBox="0 0 24 24" fill="currentColor"><path d="M8 4.5v15l12-7.5z"/></svg>
              </button>
              <button id="btn-next" class="icon-btn hidden" title="Next" onclick="ShareApp.next()">
                <svg viewBox="0 0 24 24" fill="currentColor"><path d="M4.5 4.5 15 12 4.5 19.5zm12 0H19v15h-2.5z"/></svg>
              </button>
              <button id="btn-repeat" class="icon-btn" title="Repeat" onclick="ShareApp.cycleRepeat()">
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.1" stroke-linecap="round" stroke-linejoin="round"><polyline points="17 2 21 6 17 10"/><path d="M3 12v-2a4 4 0 0 1 4-4h14"/><polyline points="7 22 3 18 7 14"/><path d="M21 12v2a4 4 0 0 1-4 4H3"/></svg>
              </button>
            </div>
            <div class="vol-group">
              <button class="icon-btn" title="Mute" onclick="ShareApp.toggleMute()">
                <svg id="vol-svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.1" stroke-linecap="round" stroke-linejoin="round"><polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5"/><path d="M15.5 8.5a5 5 0 0 1 0 7"/><path d="M19 5a10 10 0 0 1 0 14"/></svg>
              </button>
              <input id="vol-slider" class="vol-slider" type="range" min="0" max="1" step="0.01" value="0.9" />
            </div>
          </div>
          <canvas id="viz-canvas" class="visualizer" width="520" height="40"></canvas>
        </div>

        <div class="actions-row">
          <button class="btn-secondary" onclick="ShareApp.copyLink()">Copy Share Link</button>
          <button id="btn-lyrics" class="btn-secondary" onclick="ShareApp.toggleLyrics()">Lyrics</button>
          <a id="open-app-btn" class="btn-secondary" href="/">Open in Beatlab</a>
        </div>
      </div>
    </section>

    <section id="lyrics-card" class="card-panel hidden">
      <div class="panel-head">
        <div>
          <h2>Lyrics</h2>
          <p id="lyrics-sub" class="panel-sub">Synced lyrics</p>
        </div>
        <button class="btn-secondary" onclick="ShareApp.toggleLyrics(false)">Close</button>
      </div>
      <div id="lyrics-box" class="lyrics-box"></div>
    </section>

    <section id="tracks-card" class="card-panel hidden">
      <div class="panel-head">
        <div>
          <h2 id="tracks-heading">Tracklist</h2>
          <p id="tracks-sub" class="panel-sub"></p>
        </div>
        <div style="display:flex;gap:8px;flex-wrap:wrap">
          <button class="btn-primary" onclick="ShareApp.playIndex(0)">Play All</button>
          <button class="btn-secondary" onclick="ShareApp.playShuffled()">Shuffle</button>
        </div>
      </div>
      <div id="track-list" class="track-list"></div>
    </section>

    <section id="cta-banner" class="cta-banner">
      <div>
        <h3>Listen to everything on Beatlab</h3>
        <p>Create a free account in seconds for smart custom mixes, synced lyrics, detailed listening stats, and Subsonic/Navidrome apps.</p>
      </div>
      <div class="cta-actions">
        <button class="btn-primary" onclick="ShareApp.openAuth('signup')">Sign Up Free</button>
        <button class="btn-secondary" onclick="ShareApp.openAuth('signin')">Sign In</button>
      </div>
    </section>
  </div>

  <div id="auth-modal" class="modal-backdrop hidden" onclick="if(event.target===this)ShareApp.closeAuth()">
    <div class="modal-box">
      <div class="modal-head">
        <h3 id="auth-modal-title">Sign In or Sign Up</h3>
        <button class="icon-btn" onclick="ShareApp.closeAuth()">✕</button>
      </div>
      <p style="font-size:13px;color:var(--text-muted)">Enter any username &amp; password to sign in or automatically create a brand-new Beatlab account.</p>
      <form class="auth-form" onsubmit="ShareApp.submitAuth(event)">
        <label>
          <span>Username</span>
          <input id="sa-user" type="text" autocomplete="username" placeholder="e.g. alex" required maxlength="64" />
        </label>
        <label>
          <span>Password</span>
          <input id="sa-pass" type="password" autocomplete="current-password" placeholder="Choose or enter password" required maxlength="256" />
        </label>
        <div id="sa-err" class="auth-err hidden"></div>
        <button type="submit" class="btn-primary" style="width:100%">Continue to Beatlab</button>
      </form>
    </div>
  </div>

  <div id="toast" class="toast hidden"></div>
  <audio id="audio-el" preload="metadata"></audio>

  <script>
  (() => {{
    const DATA = {safe_payload};
    const tracks = DATA.tracks || [];
    const audio = document.getElementById('audio-el');
    const $ = (id) => document.getElementById(id);

    const state = {{
      idx: 0,
      shuffle: false,
      repeat: 'off',
      seeking: false,
      compatAttemptedId: '',
      lyricsOpen: false,
      lyricsCache: new Map(),
      activeLyrics: {{ synced: false, lines: [] }},
      activeLyricIdx: -1,
      activeAudioToken: 0,
      lastSwitchAt: 0,
      loadCommitTimer: null,
      lyricsAbortController: null,
      lyricsReqToken: 0,
      token: localStorage.getItem('beatlab_token') || '',
      user: null,
    }};

    let audioCtx = null;
    let analyser = null;
    let vizStarted = false;

    function esc(s) {{
      return String(s ?? '').replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;').replace(/"/g,'&quot;').replace(/'/g,'&#39;');
    }}
    function fmtTime(sec) {{
      if (!Number.isFinite(sec) || sec < 0) return '0:00';
      const s = Math.floor(sec);
      return `${{Math.floor(s / 60)}}:${{(s % 60) < 10 ? '0' : ''}}${{s % 60}}`;
    }}
    function fmtTotal(sec) {{
      if (!sec || sec <= 0) return '0 min';
      const h = Math.floor(sec / 3600);
      const m = Math.floor((sec % 3600) / 60);
      return h > 0 ? `${{h}} hr ${{m}} min` : `${{Math.max(1, m)}} min`;
    }}
    function toast(msg) {{
      const el = $('toast');
      el.textContent = msg;
      el.classList.remove('hidden');
      clearTimeout(el._t);
      el._t = setTimeout(() => el.classList.add('hidden'), 2400);
    }}
    function canBrowserPlayFormat(fmt) {{
      const f = String(fmt || '').trim().toLowerCase();
      if (!f) return true;
      const probes = {{
        mp3: ['audio/mpeg'],
        flac: ['audio/flac', 'audio/x-flac'],
        wav: ['audio/wav', 'audio/wave', 'audio/x-wav'],
        m4a: ['audio/mp4; codecs="mp4a.40.2"', 'audio/mp4'],
        mp4: ['audio/mp4; codecs="mp4a.40.2"', 'audio/mp4'],
        aac: ['audio/aac', 'audio/mp4; codecs="mp4a.40.2"'],
        ogg: ['audio/ogg; codecs="vorbis"', 'audio/ogg'],
        opus: ['audio/ogg; codecs="opus"', 'audio/webm; codecs="opus"', 'audio/opus'],
      }};
      const mimes = probes[f];
      return mimes ? mimes.some((m) => audio.canPlayType(m) !== '') : false;
    }}
    function buildUrl(t, forceCompat = false) {{
      const needCompat = forceCompat || !canBrowserPlayFormat(t.format);
      return needCompat ? `${{t.streamUrl}}?compat=1` : t.streamUrl;
    }}
    function ensureVisualizer() {{
      if (!vizStarted) {{
        vizStarted = true;
        drawViz();
      }}
      if (audioCtx) {{
        if (audioCtx.state === 'suspended') audioCtx.resume().catch(() => {{}});
        return;
      }}
      try {{
        const AC = window.AudioContext || window.webkitAudioContext;
        if (!AC) return;
        audioCtx = new AC();
        const src = audioCtx.createMediaElementSource(audio);
        analyser = audioCtx.createAnalyser();
        analyser.fftSize = 128;
        src.connect(analyser);
        analyser.connect(audioCtx.destination);
      }} catch {{}}
    }}
    function drawViz() {{
      const canvas = $('viz-canvas');
      if (canvas) {{
        const ctx = canvas.getContext('2d');
        const w = canvas.width;
        const h = canvas.height;
        ctx.clearRect(0, 0, w, h);
        const bars = 44;
        const bw = w / bars;
        if (analyser && !audio.paused) {{
          const data = new Uint8Array(analyser.frequencyBinCount);
          analyser.getByteFrequencyData(data);
          ctx.fillStyle = '#1ed760';
          for (let i = 0; i < bars; i++) {{
            const v = data[i] / 255;
            const bh = Math.max(3, v * h);
            ctx.fillRect(i * bw + 2, h - bh, Math.max(2, bw - 4), bh);
          }}
        }} else {{
          ctx.fillStyle = 'rgba(152,162,179,0.22)';
          for (let i = 0; i < bars; i++) {{
            const bh = 3 + ((Math.sin(i * 0.55) + 1) * 3.5);
            ctx.fillRect(i * bw + 2, h - bh, Math.max(2, bw - 4), bh);
          }}
        }}
      }}
      requestAnimationFrame(drawViz);
    }}

    function updateHeaderAndMeta(t) {{
      if (!t) return;
      const art = t.coverUrl || DATA.coverUrl;
      $('cover-img').src = art;
      $('now-title').textContent = DATA.kind === 'track' ? t.title : (tracks.length > 1 ? `${{t.title}}` : DATA.title);
      const subParts = [t.artist];
      if (t.album) subParts.push(t.album);
      if (DATA.kind !== 'track' && DATA.title) subParts.push(DATA.title);
      $('now-sub').textContent = subParts.filter(Boolean).join(' · ');

      const badges = [];
      if (t.qualityBadge) {{
        badges.push(`<span class="badge quality ${{t.isLossless ? 'lossless' : ''}}">${{esc(t.qualityBadge)}}</span>`);
      }}
      if (t.duration > 0) badges.push(`<span class="badge">${{fmtTime(t.duration)}}</span>`);
      if (t.year > 0) badges.push(`<span class="badge">${{t.year}}</span>`);
      if (t.genre) badges.push(`<span class="badge">${{esc(t.genre)}}</span>`);
      if (t.sampleRate > 0 && t.isLossless) {{
        badges.push(`<span class="badge">${{(t.sampleRate / 1000).toFixed(1).replace(/\.0$/, '')}} kHz</span>`);
      }}
      $('badges-row').innerHTML = badges.join('');
      $('duration-lbl').textContent = fmtTime(t.duration || 0);
      document.title = `${{t.title}} — ${{t.artist}} | Beatlab`;
      renderTracklist();
    }}

    function renderTracklist() {{
      const card = $('tracks-card');
      if (tracks.length <= 1) {{
        card.classList.add('hidden');
        return;
      }}
      card.classList.remove('hidden');
      $('btn-shuffle').classList.remove('hidden');
      $('btn-prev').classList.remove('hidden');
      $('btn-next').classList.remove('hidden');
      $('tracks-heading').textContent = DATA.title || 'Tracklist';
      const totalSec = tracks.reduce((a, b) => a + Math.max(0, b.duration || 0), 0);
      $('tracks-sub').textContent = `${{tracks.length}} songs · ${{fmtTotal(totalSec)}}`;
      $('track-list').innerHTML = tracks.map((t, i) => `
        <div class="t-row ${{i === state.idx ? 'playing' : ''}}" onclick="ShareApp.playIndex(${{i}})">
          <div class="t-idx">${{i === state.idx && !audio.paused ? '▶' : i + 1}}</div>
          <div class="t-main">
            <img src="${{esc(t.coverUrl)}}" loading="lazy" alt="" />
            <div class="t-meta">
              <div class="t-title">${{esc(t.title)}}</div>
              <div class="t-sub">${{esc(t.artist)}}${{t.album ? ` · ${{esc(t.album)}}` : ''}}</div>
            </div>
          </div>
          <span class="badge quality ${{t.isLossless ? 'lossless' : ''}}">${{esc(t.qualityBadge)}}</span>
          <span class="t-dur">${{fmtTime(t.duration)}}</span>
        </div>
      `).join('');
    }}

    function updatePlayBtn(playing) {{
      $('play-svg').innerHTML = playing
        ? '<path d="M6.5 4.5h4v15h-4zm7 0h4v15h-4z"/>'
        : '<path d="M8 4.5v15l12-7.5z"/>';
      $('live-pill').classList.toggle('hidden', !playing);
      renderTracklist();
    }}

    function commitShareTrack(token, t, autoplay) {{
      if (token !== state.activeAudioToken || tracks[state.idx]?.id !== t.id) return;
      try {{
        if (!audio.paused) audio.pause();
      }} catch {{}}
      audio.src = buildUrl(t, false);
      if (state.lyricsOpen) loadLyrics(t);
      if (autoplay) {{
        ensureVisualizer();
        audio.play().catch((e) => {{
          if (token !== state.activeAudioToken || (e && e.name === 'AbortError')) return;
          updatePlayBtn(false);
        }});
      }}
    }}

    function loadTrack(idx, autoplay = false) {{
      if (!tracks.length) return;
      if (state.loadCommitTimer) {{
        clearTimeout(state.loadCommitTimer);
        state.loadCommitTimer = null;
      }}
      if (state.lyricsAbortController) {{
        try {{ state.lyricsAbortController.abort(); }} catch {{}}
        state.lyricsAbortController = null;
      }}
      const now = typeof performance !== 'undefined' && performance.now ? performance.now() : Date.now();
      const isBurst = autoplay && state.lastSwitchAt > 0 && now - state.lastSwitchAt < 280;
      if (autoplay) state.lastSwitchAt = now;
      const token = ++state.activeAudioToken;
      state.idx = ((idx % tracks.length) + tracks.length) % tracks.length;
      const t = tracks[state.idx];
      state.compatAttemptedId = '';
      state.activeLyricIdx = -1;
      $('seek-slider').value = '0';
      $('elapsed-lbl').textContent = '0:00';
      updateHeaderAndMeta(t);
      if (autoplay) ensureVisualizer();
      if (!isBurst) {{
        commitShareTrack(token, t, autoplay);
        return;
      }}
      try {{
        if (!audio.paused) audio.pause();
      }} catch {{}}
      state.loadCommitTimer = setTimeout(() => {{
        state.loadCommitTimer = null;
        commitShareTrack(token, t, autoplay);
      }}, 220);
    }}

    async function loadLyrics(t) {{
      if (!t) return;
      if (state.lyricsAbortController) {{
        try {{ state.lyricsAbortController.abort(); }} catch {{}}
      }}
      const ctrl = typeof AbortController !== 'undefined' ? new AbortController() : null;
      state.lyricsAbortController = ctrl;
      const reqToken = ++state.lyricsReqToken;
      const box = $('lyrics-box');
      box.innerHTML = '<p style="color:var(--text-muted)">Loading lyrics...</p>';
      let data = state.lyricsCache.get(t.id);
      if (!data) {{
        try {{
          const r = await fetch(`/api/public/share/${{encodeURIComponent(DATA.id)}}/lyrics/${{encodeURIComponent(t.id)}}`, {{
            signal: ctrl ? ctrl.signal : undefined,
          }});
          data = r.ok ? await r.json() : {{ synced: false, lines: [] }};
          state.lyricsCache.set(t.id, data);
        }} catch (e) {{
          if (e && e.name === 'AbortError') return;
          data = {{ synced: false, lines: [] }};
        }} finally {{
          if (state.lyricsAbortController === ctrl) state.lyricsAbortController = null;
        }}
      }}
      if (reqToken !== state.lyricsReqToken || tracks[state.idx]?.id !== t.id) return;
      state.activeLyrics = data;
      const lines = data.lines || [];
      $('lyrics-sub').textContent = data.synced ? 'Synced lyrics · tap any line to seek' : `${{t.title}} — ${{t.artist}}`;
      box.innerHTML = lines.length
        ? lines.map((l, i) => `<div class="lyric-line ${{l.start_ms != null ? 'synced' : ''}}" data-li="${{i}}" ${{l.start_ms != null ? `onclick="ShareApp.seekMs(${{l.start_ms}})"` : ''}}>${{esc(l.text || '♪')}}</div>`).join('')
        : '<p style="color:var(--text-muted)">No lyrics available for this track.</p>';
    }}

    function syncLyrics(posMs) {{
      if (!state.lyricsOpen || !state.activeLyrics.synced || !state.activeLyrics.lines?.length) return;
      let active = -1;
      state.activeLyrics.lines.forEach((l, i) => {{
        if (l.start_ms != null && l.start_ms <= posMs + 150) active = i;
      }});
      if (active === state.activeLyricIdx) return;
      state.activeLyricIdx = active;
      $('lyrics-box').querySelectorAll('.lyric-line').forEach((el, i) => {{
        el.classList.toggle('active', i === active);
        if (i === active) el.scrollIntoView({{ behavior: 'smooth', block: 'center' }});
      }});
    }}

    async function checkAuth() {{
      if (!state.token) return;
      try {{
        const r = await fetch('/api/web/me', {{ headers: {{ Authorization: `Bearer ${{state.token}}` }} }});
        if (!r.ok) return;
        const d = await r.json();
        if (d.user) {{
          state.user = d.user;
          renderSignedInUI();
        }}
      }} catch {{}}
    }}

    function renderSignedInUI() {{
      if (!state.user) return;
      const init = (state.user.username[0] || 'B').toUpperCase();
      $('topbar-auth').innerHTML = `
        <span class="user-chip"><span class="avatar-dot">${{esc(init)}}</span><span>${{esc(state.user.username)}}</span></span>
        <a class="btn-primary" href="/">Open Full Player</a>
      `;
      $('cta-banner').classList.add('hidden');
      const cur = tracks[state.idx];
      if (cur && DATA.kind === 'track') {{
        $('open-app-btn').href = `/#search?q=${{encodeURIComponent(`${{cur.title}} ${{cur.artist}}`)}}`;
      }} else if (DATA.kind === 'album') {{
        $('open-app-btn').href = `/#album/${{encodeURIComponent(DATA.targetId)}}`;
      }} else if (DATA.kind === 'artist') {{
        $('open-app-btn').href = `/#artist/${{encodeURIComponent(DATA.targetId)}}`;
      }} else if (DATA.kind === 'playlist') {{
        $('open-app-btn').href = `/#playlist/${{encodeURIComponent(DATA.targetId)}}`;
      }}
    }}

    audio.volume = 0.9;
    audio.addEventListener('play', () => updatePlayBtn(true));
    audio.addEventListener('pause', () => updatePlayBtn(false));
    audio.addEventListener('ended', () => {{
      if (state.loadCommitTimer) return;
      if (state.repeat === 'one') {{
        audio.currentTime = 0;
        audio.play().catch(() => {{}});
        return;
      }}
      if (tracks.length > 1) {{
        if (state.shuffle) {{
          ShareApp.playIndex(Math.floor(Math.random() * tracks.length));
        }} else if (state.idx + 1 < tracks.length) {{
          ShareApp.playIndex(state.idx + 1);
        }} else if (state.repeat === 'all') {{
          ShareApp.playIndex(0);
        }}
      }}
    }});
    audio.addEventListener('error', () => {{
      if (state.loadCommitTimer || !audio.getAttribute('src')) return;
      const errToken = state.activeAudioToken;
      const t = tracks[state.idx];
      if (!t || state.compatAttemptedId === t.id) return;
      state.compatAttemptedId = t.id;
      audio.src = buildUrl(t, true);
      audio.play().catch((e) => {{
        if (errToken !== state.activeAudioToken || (e && e.name === 'AbortError')) return;
        updatePlayBtn(false);
      }});
    }});
    audio.addEventListener('timeupdate', () => {{
      if (state.seeking) return;
      const t = tracks[state.idx];
      const total = Number.isFinite(audio.duration) && audio.duration > 0 ? audio.duration : (t?.duration || 0);
      const pos = audio.currentTime || 0;
      $('elapsed-lbl').textContent = fmtTime(pos);
      if (total > 0) {{
        $('duration-lbl').textContent = fmtTime(total);
        $('seek-slider').value = String(Math.min(100, (pos / total) * 100));
      }}
      syncLyrics(pos * 1000);
    }});

    $('seek-slider').addEventListener('input', (e) => {{
      state.seeking = true;
      const t = tracks[state.idx];
      const total = Number.isFinite(audio.duration) && audio.duration > 0 ? audio.duration : (t?.duration || 0);
      if (total > 0) $('elapsed-lbl').textContent = fmtTime((parseFloat(e.target.value) / 100) * total);
    }});
    $('seek-slider').addEventListener('change', (e) => {{
      state.seeking = false;
      const t = tracks[state.idx];
      const total = Number.isFinite(audio.duration) && audio.duration > 0 ? audio.duration : (t?.duration || 0);
      if (total > 0) audio.currentTime = (parseFloat(e.target.value) / 100) * total;
    }});
    $('vol-slider').addEventListener('input', (e) => {{
      audio.volume = parseFloat(e.target.value);
      audio.muted = audio.volume === 0;
    }});

    window.ShareApp = {{
      togglePlay: () => {{
        if (!tracks.length) return;
        ensureVisualizer();
        if (!audio.getAttribute('src')) loadTrack(state.idx, true);
        else if (audio.paused) audio.play().catch(() => {{}});
        else audio.pause();
      }},
      playIndex: (i) => loadTrack(i, true),
      playShuffled: () => {{
        if (!tracks.length) return;
        state.shuffle = true;
        $('btn-shuffle').classList.add('active');
        loadTrack(Math.floor(Math.random() * tracks.length), true);
      }},
      prev: () => {{
        if (audio.currentTime > 4) {{ audio.currentTime = 0; return; }}
        loadTrack(state.idx - 1, true);
      }},
      next: () => {{
        if (state.shuffle && tracks.length > 1) {{
          loadTrack(Math.floor(Math.random() * tracks.length), true);
        }} else {{
          loadTrack(state.idx + 1, true);
        }}
      }},
      toggleShuffle: () => {{
        state.shuffle = !state.shuffle;
        $('btn-shuffle').classList.toggle('active', state.shuffle);
      }},
      cycleRepeat: () => {{
        const modes = ['off', 'all', 'one'];
        state.repeat = modes[(modes.indexOf(state.repeat) + 1) % modes.length];
        $('btn-repeat').classList.toggle('active', state.repeat !== 'off');
        toast(`Repeat: ${{state.repeat}}`);
      }},
      toggleMute: () => {{
        audio.muted = !audio.muted;
      }},
      toggleLyrics: (force) => {{
        state.lyricsOpen = typeof force === 'boolean' ? force : !state.lyricsOpen;
        $('lyrics-card').classList.toggle('hidden', !state.lyricsOpen);
        $('btn-lyrics').classList.toggle('active', state.lyricsOpen);
        if (state.lyricsOpen && tracks[state.idx]) loadLyrics(tracks[state.idx]);
      }},
      seekMs: (ms) => {{
        if (Number.isFinite(ms)) {{
          audio.currentTime = ms / 1000;
          if (audio.paused) {{ ensureVisualizer(); audio.play().catch(() => {{}}); }}
        }}
      }},
      copyLink: async () => {{
        try {{
          await navigator.clipboard.writeText(DATA.shareUrl || location.href);
          toast('Share link copied!');
        }} catch {{
          toast('Copy URL from address bar');
        }}
      }},
      openAuth: (mode) => {{
        $('auth-modal-title').textContent = mode === 'signup' ? 'Create Free Beatlab Account' : 'Sign In to Beatlab';
        $('sa-err').classList.add('hidden');
        $('auth-modal').classList.remove('hidden');
        setTimeout(() => $('sa-user')?.focus(), 50);
      }},
      closeAuth: () => $('auth-modal').classList.add('hidden'),
      submitAuth: async (e) => {{
        e.preventDefault();
        const err = $('sa-err');
        err.classList.add('hidden');
        try {{
          const r = await fetch('/api/web/auth/signin', {{
            method: 'POST',
            headers: {{ 'Content-Type': 'application/json' }},
            body: JSON.stringify({{ username: $('sa-user').value.trim(), password: $('sa-pass').value }}),
          }});
          const res = await r.json();
          if (!r.ok) throw new Error(res.error || 'Authentication failed');
          state.token = res.token;
          state.user = res.user;
          localStorage.setItem('beatlab_token', res.token);
          $('auth-modal').classList.add('hidden');
          renderSignedInUI();
          toast(res.created ? `Welcome, ${{res.user.username}}! Account created.` : `Welcome back, ${{res.user.username}}!`);
        }} catch (ex) {{
          err.textContent = ex.message;
          err.classList.remove('hidden');
        }}
      }},
    }};

    if (tracks.length > 0) loadTrack(0, false);
    drawViz();
    checkAuth();
  }})();
  </script>
</body>
</html>"##,
        esc_page_title = html_escape(&og_title),
        og_type = og_type,
        esc_og_title = html_escape(&og_title),
        esc_og_desc = html_escape(&og_desc),
        esc_url = html_escape(&page_url),
        esc_cover = html_escape(&cover_url),
        esc_oembed = html_escape(&oembed_url),
        media_meta_tags = media_meta_tags,
        safe_payload = safe_payload,
        esc_id = html_escape(&share.id),
        esc_kind_badge = html_escape(kind_badge),
        esc_creator = html_escape(&share.creator_name),
        esc_init_title = html_escape(initial_title),
        esc_init_sub = html_escape(&initial_sub),
    )
}

pub fn render_embed_html(origin: &str, share: &ShareRecord, tracks: &[Track]) -> String {
    let (title, _) = build_share_meta(share, tracks);
    let lead = tracks.first();
    let sub = lead
        .map(|t| {
            if share.kind == "track" {
                t.artist.clone()
            } else {
                format!("{} · {} tracks", share_kind_label(&share.kind), tracks.len())
            }
        })
        .unwrap_or_else(|| share.subtitle.clone());
    let stream_url = format!("/s/{}/stream", share.id);
    let cover_url = format!("/s/{}/cover", share.id);
    let open_url = format!("{}/s/{}", origin, share.id);
    let dur = lead.map(|t| t.duration).unwrap_or(0);

    format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0" />
  <title>{esc_title} — Beatlab</title>
  <style>
    * {{ box-sizing: border-box; margin: 0; padding: 0; }}
    html, body {{ height: 100%; background: #0c0e13; color: #f3f5f8; font-family: system-ui, -apple-system, sans-serif; overflow: hidden; }}
    .wrap {{ height: 100%; display: flex; align-items: center; gap: 14px; padding: 12px 14px; background: #12151c; border: 1px solid #232834; }}
    .art {{ width: 104px; height: 104px; border-radius: 6px; object-fit: cover; flex-shrink: 0; background: #191d26; border: 1px solid #232834; }}
    .body {{ flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 8px; }}
    .head {{ display: flex; justify-content: space-between; align-items: flex-start; gap: 8px; }}
    .title {{ font-size: 15px; font-weight: 800; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }}
    .sub {{ font-size: 12.5px; color: #98a2b3; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }}
    .open {{ font-size: 11px; font-weight: 700; color: #1ed760; text-decoration: none; flex-shrink: 0; padding: 3px 8px; border-radius: 4px; background: #191d26; border: 1px solid #343b4c; }}
    .ctrl {{ display: flex; align-items: center; gap: 10px; }}
    .play {{ width: 38px; height: 38px; border-radius: 50%; border: none; background: #1ed760; color: #06120b; display: grid; place-items: center; cursor: pointer; flex-shrink: 0; }}
    .play svg {{ width: 18px; height: 18px; display: block; }}
    .seek {{ flex: 1; accent-color: #1ed760; cursor: pointer; min-width: 0; }}
    .time {{ font-size: 11px; color: #98a2b3; font-variant-numeric: tabular-nums; flex-shrink: 0; }}
  </style>
</head>
<body>
  <div class="wrap">
    <img class="art" src="{esc_cover}" alt="" />
    <div class="body">
      <div class="head">
        <div style="min-width:0">
          <div class="title">{esc_title}</div>
          <div class="sub">{esc_sub}</div>
        </div>
        <a class="open" href="{esc_open}" target="_blank" rel="noopener">Beatlab ↗</a>
      </div>
      <div class="ctrl">
        <button id="ep" class="play" onclick="toggle()"><svg id="epi" viewBox="0 0 24 24" fill="currentColor"><path d="M8 4.5v15l12-7.5z"/></svg></button>
        <input id="es" class="seek" type="range" min="0" max="100" value="0" step="0.1" />
        <span id="et" class="time">0:00 / {esc_dur}</span>
      </div>
    </div>
  </div>
  <audio id="ea" preload="none" src="{esc_stream}"></audio>
  <script>
    const a = document.getElementById('ea'), p = document.getElementById('epi'), s = document.getElementById('es'), t = document.getElementById('et');
    const knownDur = {dur};
    let compat = false;
    const fmt = (v) => {{ if (!Number.isFinite(v) || v < 0) return '0:00'; const k = Math.floor(v); return Math.floor(k/60)+':'+(k%60<10?'0':'')+(k%60); }};
    function toggle() {{ if (a.paused) a.play().catch(()=>{{}}); else a.pause(); }}
    a.addEventListener('play', () => p.innerHTML = '<path d="M6.5 4.5h4v15h-4zm7 0h4v15h-4z"/>');
    a.addEventListener('pause', () => p.innerHTML = '<path d="M8 4.5v15l12-7.5z"/>');
    a.addEventListener('error', () => {{ if (!compat) {{ compat = true; a.src = '{esc_stream}?compat=1'; a.play().catch(()=>{{}}); }} }});
    a.addEventListener('timeupdate', () => {{
      const d = Number.isFinite(a.duration) && a.duration > 0 ? a.duration : knownDur;
      if (d > 0) {{ s.value = String(Math.min(100, (a.currentTime / d) * 100)); t.textContent = fmt(a.currentTime) + ' / ' + fmt(d); }}
    }});
    s.addEventListener('change', () => {{
      const d = Number.isFinite(a.duration) && a.duration > 0 ? a.duration : knownDur;
      if (d > 0) a.currentTime = (parseFloat(s.value) / 100) * d;
    }});
  </script>
</body>
</html>"##,
        esc_title = html_escape(&title),
        esc_sub = html_escape(&sub),
        esc_cover = html_escape(&cover_url),
        esc_open = html_escape(&open_url),
        esc_stream = html_escape(&stream_url),
        esc_dur = html_escape(&fmt_dur(dur)),
        dur = dur,
    )
}

pub fn render_share_not_found_html() -> String {
    r##"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0" />
  <meta name="theme-color" content="#0c0e13" />
  <title>Share Not Found — Beatlab</title>
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
    <h1>Share link not found</h1>
    <p>This Beatlab share link may have been removed or mistyped.</p>
    <a href="/">Go to Beatlab</a>
  </div>
</body>
</html>"##
        .to_string()
}
