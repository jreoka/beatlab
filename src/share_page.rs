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
    .btn-primary:disabled {{ opacity: 0.65; cursor: not-allowed; transform: none; }}
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
    .avatar-img {{
      width: 100%;
      height: 100%;
      object-fit: cover;
      display: block;
      border-radius: inherit;
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
    .auth-form input.auth-hp {{
      position: absolute;
      left: -9999px;
      top: -9999px;
      width: 1px;
      height: 1px;
      padding: 0;
      margin: 0;
      border: 0;
      opacity: 0;
      pointer-events: none;
      overflow: hidden;
    }}
    .auth-captcha {{
      position: relative;
      background: var(--bg);
      border: 1px solid var(--border);
      border-radius: 6px;
      overflow: hidden;
      transition: border-color 0.16s ease, background 0.16s ease;
    }}
    .auth-captcha:hover {{ border-color: var(--border-bright); }}
    .auth-captcha[data-state="interactive"] {{
      border-color: rgba(30, 215, 96, 0.5);
      background: var(--surface-2);
    }}
    .auth-captcha[data-state="verified"] {{ border-color: rgba(30, 215, 96, 0.42); }}
    .auth-captcha[data-state="error"] {{ border-color: rgba(244, 63, 94, 0.45); }}
    .auth-captcha-btn {{
      width: 100%;
      display: flex;
      align-items: center;
      gap: 12px;
      padding: 11px 12px;
      background: transparent;
      color: var(--text);
      text-align: left;
      cursor: pointer;
    }}
    .auth-captcha-btn:focus-visible {{
      outline: 2px solid var(--accent);
      outline-offset: -2px;
      border-radius: 6px;
    }}
    .auth-captcha-indicator {{
      position: relative;
      width: 22px;
      height: 22px;
      flex-shrink: 0;
      display: grid;
      place-items: center;
    }}
    .auth-captcha-box {{
      width: 20px;
      height: 20px;
      border-radius: 5px;
      border: 1.5px solid var(--border-bright);
      background: var(--surface);
      transition: border-color 0.15s ease, background 0.15s ease;
    }}
    .auth-captcha-btn:hover .auth-captcha-box,
    .auth-captcha[data-state="interactive"] .auth-captcha-box {{
      border-color: var(--accent);
    }}
    .auth-captcha-spinner,
    .auth-captcha-check,
    .auth-captcha-alert {{
      display: none;
    }}
    .auth-captcha[data-state="checking"] .auth-captcha-box,
    .auth-captcha[data-state="solving"] .auth-captcha-box,
    .auth-captcha[data-state="verified"] .auth-captcha-box,
    .auth-captcha[data-state="error"] .auth-captcha-box {{
      display: none;
    }}
    @keyframes saSpin {{ to {{ transform: rotate(360deg); }} }}
    .auth-captcha[data-state="checking"] .auth-captcha-spinner,
    .auth-captcha[data-state="solving"] .auth-captcha-spinner {{
      display: block;
      width: 20px;
      height: 20px;
      border-radius: 50%;
      border: 2px solid var(--border-bright);
      border-top-color: var(--accent);
      animation: saSpin 0.72s linear infinite;
    }}
    .auth-captcha[data-state="verified"] .auth-captcha-indicator {{
      border-radius: 50%;
      background: var(--accent);
      color: #04120a;
    }}
    .auth-captcha[data-state="verified"] .auth-captcha-check {{
      display: block;
      width: 14px;
      height: 14px;
    }}
    .auth-captcha[data-state="error"] .auth-captcha-alert {{
      display: block;
      width: 20px;
      height: 20px;
      color: #f43f5e;
    }}
    .auth-captcha-copy {{
      display: flex;
      flex-direction: column;
      gap: 1px;
      min-width: 0;
      flex: 1;
    }}
    .auth-captcha-label {{
      font-size: 13px;
      font-weight: 600;
      color: var(--text);
      line-height: 1.25;
    }}
    .auth-captcha-sub {{
      font-size: 11.5px;
      color: var(--text-dim);
      line-height: 1.25;
      white-space: nowrap;
      overflow: hidden;
      text-overflow: ellipsis;
    }}
    .auth-captcha[data-state="error"] .auth-captcha-sub {{ color: #fda4af; }}
    .auth-captcha-brand {{
      display: flex;
      flex-direction: column;
      align-items: center;
      gap: 2px;
      flex-shrink: 0;
      color: var(--text-dim);
      font-size: 9.5px;
      font-weight: 600;
      letter-spacing: 0.04em;
      text-transform: uppercase;
      padding-left: 8px;
      border-left: 1px solid var(--border);
    }}
    .auth-captcha-brand svg {{
      width: 15px;
      height: 15px;
      color: var(--text-muted);
    }}
    .auth-captcha[data-state="verified"] .auth-captcha-brand svg {{ color: var(--accent); }}
    .auth-captcha-progress {{
      position: absolute;
      left: 0;
      right: 0;
      bottom: 0;
      height: 2px;
      background: transparent;
      overflow: hidden;
      pointer-events: none;
    }}
    .auth-captcha-progress-fill {{
      height: 100%;
      width: 0%;
      background: var(--accent);
      transition: width 0.14s linear, opacity 0.2s ease;
      opacity: 0;
    }}
    .auth-captcha[data-state="checking"] .auth-captcha-progress-fill,
    .auth-captcha[data-state="solving"] .auth-captcha-progress-fill {{
      opacity: 1;
    }}
    @media (prefers-reduced-motion: reduce) {{
      .auth-captcha[data-state="checking"] .auth-captcha-spinner,
      .auth-captcha[data-state="solving"] .auth-captcha-spinner {{
        animation: none;
        border-color: var(--accent);
      }}
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
        <p>Create a free account in seconds for smart custom mixes, synced lyrics, and detailed listening stats.</p>
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
      <form class="auth-form" onsubmit="ShareApp.submitAuth(event)">
        <label>
          <span>Username</span>
          <input id="sa-user" type="text" autocomplete="username" placeholder="e.g. alex" required maxlength="64" />
        </label>
        <label>
          <span>Password</span>
          <input id="sa-pass" type="password" autocomplete="current-password" placeholder="Choose or enter password" required maxlength="256" />
        </label>
        <label id="sa-confirm-row" class="hidden">
          <span>Confirm password</span>
          <input id="sa-confirm" type="password" autocomplete="new-password" placeholder="Enter your password again" required maxlength="256" />
        </label>
        <input id="sa-hp" class="auth-hp" type="text" name="hp_contact_token" tabindex="-1" autocomplete="off" aria-hidden="true" />
        <div id="sa-captcha" class="auth-captcha" data-state="idle">
          <button id="sa-captcha-btn" type="button" class="auth-captcha-btn" role="checkbox" aria-checked="false" aria-describedby="sa-captcha-sub">
            <span class="auth-captcha-indicator" aria-hidden="true">
              <span class="auth-captcha-box"></span>
              <span class="auth-captcha-spinner"></span>
              <svg class="auth-captcha-check" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>
              <svg class="auth-captcha-alert" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="9"/><line x1="12" y1="8" x2="12" y2="12.5"/><circle cx="12" cy="16.2" r="0.8" fill="currentColor" stroke="none"/></svg>
            </span>
            <span class="auth-captcha-copy">
              <span id="sa-captcha-label" class="auth-captcha-label">Verify you&apos;re human</span>
              <span id="sa-captcha-sub" class="auth-captcha-sub">Browser &amp; proof-of-work check</span>
            </span>
            <span class="auth-captcha-brand" aria-hidden="true">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3 5 6v6c0 4.5 3 7.8 7 9 4-1.2 7-4.5 7-9V6l-7-3z"/><path d="m9.5 12 1.8 1.8 3.5-3.8"/></svg>
              <span>Shield</span>
            </span>
          </button>
          <div class="auth-captcha-progress" aria-hidden="true"><div id="sa-captcha-progress-fill" class="auth-captcha-progress-fill"></div></div>
        </div>
        <div id="sa-err" class="auth-err hidden"></div>
        <button id="sa-submit" type="submit" class="btn-primary" style="width:100%">Sign in</button>
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
      order: tracks.map((_, i) => i),
      orderPos: 0,
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
      authMode: 'signin',
    }};

    let masterVolume = 0.9;
    const NORM_TARGET_LUFS = -14.0;
    const NORM_CEILING_LIN = Math.pow(10, -0.5 / 20);
    const NORM_DEFAULT_GAIN = Math.pow(10, -3.0 / 20);
    const normState = {{
      ctx: null, source: null, gainNode: null, ready: false, failed: false, connecting: false,
      trackId: '', loudness: null, reqToken: 0, retryTimer: null,
    }};

    function computeNormGain(vol, loudness) {{
      if (vol <= 0) return 0;
      if (loudness && Number.isFinite(loudness.lufs)) {{
        const gainDb = Math.max(-18, Math.min(12, NORM_TARGET_LUFS - loudness.lufs));
        const trackGain = Math.pow(10, gainDb / 20);
        const tpDb = Number.isFinite(loudness.truePeakDb) ? loudness.truePeakDb : 0;
        const tpLin = Math.max(0.01, Math.min(4.0, Math.pow(10, tpDb / 20)));
        return Math.min(vol * trackGain, NORM_CEILING_LIN / tpLin);
      }}
      return Math.min(vol * NORM_DEFAULT_GAIN, NORM_CEILING_LIN);
    }}

    function ensureNormGraph() {{
      if (normState.ready) {{
        if (normState.ctx && normState.ctx.state !== 'running') normState.ctx.resume().catch(() => {{}});
        return;
      }}
      if (normState.failed || normState.connecting) return;
      const AC = window.AudioContext || window.webkitAudioContext;
      if (!AC) {{ normState.failed = true; return; }}
      if (navigator.audioSession) {{ try {{ navigator.audioSession.type = 'playback'; }} catch {{}} }}
      try {{ if (!normState.ctx) normState.ctx = new AC(); }} catch {{ normState.failed = true; return; }}
      const ctx = normState.ctx;
      const wire = () => {{
        if (normState.ready) return;
        try {{
          normState.source = ctx.createMediaElementSource(audio);
          normState.gainNode = ctx.createGain();
          normState.source.connect(normState.gainNode);
          normState.gainNode.connect(ctx.destination);
          normState.ready = true;
          normState.connecting = false;
          ctx.addEventListener('statechange', () => {{
            if (ctx.state !== 'running' && !audio.paused && !audio.ended) ctx.resume().catch(() => {{}});
          }});
          applyOutputVolume(0);
        }} catch {{
          normState.failed = true;
          normState.connecting = false;
        }}
      }};
      if (ctx.state === 'running') {{ wire(); return; }}
      normState.connecting = true;
      ctx.resume().then(() => {{
        if (ctx.state === 'running' && !normState.ready) wire();
        else normState.connecting = false;
      }}).catch(() => {{ normState.connecting = false; }});
    }}

    function applyOutputVolume(rampSec = 0) {{
      const v = Math.max(0, Math.min(1, Number(masterVolume) || 0));
      const target = computeNormGain(v, normState.loudness);
      audio.muted = v === 0;
      if (normState.ready && normState.gainNode && normState.ctx) {{
        audio.volume = 1;
        const p = normState.gainNode.gain;
        const now = normState.ctx.currentTime;
        try {{
          p.cancelScheduledValues(now);
          if (rampSec > 0.05) {{
            p.setValueAtTime(p.value, now);
            p.linearRampToValueAtTime(target, now + rampSec);
          }} else if (rampSec > 0) {{
            p.setTargetAtTime(target, now, 0.012);
          }} else {{
            p.setValueAtTime(target, now);
          }}
        }} catch {{ p.value = target; }}
      }} else {{
        audio.volume = Math.max(0, Math.min(1, target));
      }}
    }}

    function syncShareLoudness(t) {{
      if (normState.retryTimer) {{ clearTimeout(normState.retryTimer); normState.retryTimer = null; }}
      const tok = ++normState.reqToken;
      if (!t || !t.id) {{ normState.trackId = ''; normState.loudness = null; applyOutputVolume(0); return; }}
      normState.trackId = t.id;
      if (t.loudness && Number.isFinite(t.loudness.lufs)) {{
        normState.loudness = t.loudness;
        applyOutputVolume(0);
        return;
      }}
      normState.loudness = null;
      applyOutputVolume(0);
      const delays = [2500, 4000, 6000];
      let attempt = 0;
      const poll = () => {{
        if (tok !== normState.reqToken || normState.trackId !== t.id) return;
        fetch(`/api/public/share/${{encodeURIComponent(DATA.id)}}/loudness/${{encodeURIComponent(t.id)}}`)
          .then((r) => (r.ok ? r.json() : null))
          .then((d) => {{
            if (!d || tok !== normState.reqToken || normState.trackId !== t.id) return;
            if (Number.isFinite(Number(d.lufs))) {{
              t.loudness = {{ lufs: Number(d.lufs), truePeakDb: Number.isFinite(Number(d.truePeakDb)) ? Number(d.truePeakDb) : 0 }};
              normState.loudness = t.loudness;
              applyOutputVolume(!audio.paused && audio.currentTime > 0.25 ? 1.2 : 0);
              if (d.ready) return;
            }}
            if (d.pending && attempt < delays.length) {{
              normState.retryTimer = setTimeout(() => {{ normState.retryTimer = null; poll(); }}, delays[attempt++]);
            }}
          }})
          .catch(() => {{}});
      }};
      poll();
    }}

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

    function shuffleIndices(indices) {{
      const copy = indices.slice();
      for (let i = copy.length - 1; i > 0; i--) {{
        const j = Math.floor(Math.random() * (i + 1));
        [copy[i], copy[j]] = [copy[j], copy[i]];
      }}
      return copy;
    }}

    function rebuildPlayOrder(pinnedIdx) {{
      if (!tracks.length) {{
        state.order = [];
        state.orderPos = 0;
        return;
      }}
      if (state.shuffle && tracks.length > 1) {{
        if (typeof pinnedIdx === 'number' && pinnedIdx >= 0 && pinnedIdx < tracks.length) {{
          const rest = tracks.map((_, i) => i).filter((i) => i !== pinnedIdx);
          state.order = [pinnedIdx, ...shuffleIndices(rest)];
          state.orderPos = 0;
        }} else {{
          state.order = shuffleIndices(tracks.map((_, i) => i));
          state.orderPos = 0;
        }}
      }} else {{
        state.order = tracks.map((_, i) => i);
        state.orderPos = typeof pinnedIdx === 'number' && pinnedIdx >= 0 ? pinnedIdx : state.idx;
      }}
    }}

    function advanceShareTrack(step) {{
      if (tracks.length <= 1) return;
      if (state.order.length !== tracks.length) rebuildPlayOrder(state.idx);
      const nextPos = state.orderPos + step;
      if (nextPos >= 0 && nextPos < state.order.length) {{
        state.orderPos = nextPos;
        loadTrack(state.order[nextPos], true);
        return;
      }}
      if (state.repeat === 'all') {{
        if (step > 0) {{
          if (state.shuffle) {{
            const reshuffled = shuffleIndices(tracks.map((_, i) => i));
            if (reshuffled[0] === state.idx && reshuffled.length > 1) {{
              const swap = 1 + Math.floor(Math.random() * (reshuffled.length - 1));
              [reshuffled[0], reshuffled[swap]] = [reshuffled[swap], reshuffled[0]];
            }}
            state.order = reshuffled;
          }}
          state.orderPos = 0;
          loadTrack(state.order[0], true);
        }} else {{
          state.orderPos = state.order.length - 1;
          loadTrack(state.order[state.orderPos], true);
        }}
      }}
    }}

    function commitShareTrack(token, t, autoplay) {{
      if (token !== state.activeAudioToken || tracks[state.idx]?.id !== t.id) return;
      try {{
        if (!audio.paused) audio.pause();
      }} catch {{}}
      ensureNormGraph();
      syncShareLoudness(t);
      audio.src = buildUrl(t, false);
      if (state.lyricsOpen) loadLyrics(t);
      if (autoplay) {{
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
        <span class="user-chip"><span class="avatar-dot">${{
          state.user.avatarUrl
            ? `<img class="avatar-img" src="${{esc(state.user.avatarUrl)}}" alt="" />`
            : esc(init)
        }}</span><span>${{esc(state.user.username)}}</span></span>
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

    applyOutputVolume();
    audio.addEventListener('play', () => {{ ensureNormGraph(); updatePlayBtn(true); }});
    audio.addEventListener('pause', () => updatePlayBtn(false));
    audio.addEventListener('ended', () => {{
      if (state.loadCommitTimer) return;
      if (state.repeat === 'one') {{
        audio.currentTime = 0;
        audio.play().catch(() => {{}});
        return;
      }}
      if (tracks.length > 1) {{
        advanceShareTrack(1);
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
      masterVolume = parseFloat(e.target.value);
      applyOutputVolume(0.015);
    }});

    // Smart Proof-of-Work Captcha & Browser Integrity Check
    const POW_SHA256_CORE = `
      const K = new Int32Array([
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2
      ]);
      const W = new Int32Array(64);
      const BUF = new Uint8Array(192);
      function sha256FirstWordAscii(prefixBytes, prefixLen, nonceStr) {{
        const nLen = nonceStr.length;
        const len = prefixLen + nLen;
        const total = ((len + 9 + 63) >> 6) << 6;
        for (let i = 0; i < prefixLen; i++) BUF[i] = prefixBytes[i];
        for (let i = 0; i < nLen; i++) BUF[prefixLen + i] = nonceStr.charCodeAt(i) & 0xff;
        BUF[len] = 0x80;
        for (let i = len + 1; i < total - 4; i++) BUF[i] = 0;
        const bitLen = len * 8;
        BUF[total - 4] = (bitLen >>> 24) & 0xff;
        BUF[total - 3] = (bitLen >>> 16) & 0xff;
        BUF[total - 2] = (bitLen >>> 8) & 0xff;
        BUF[total - 1] = bitLen & 0xff;

        let h0 = 0x6a09e667 | 0, h1 = 0xbb67ae85 | 0, h2 = 0x3c6ef372 | 0, h3 = 0xa54ff53a | 0;
        let h4 = 0x510e527f | 0, h5 = 0x9b05688c | 0, h6 = 0x1f83d9ab | 0, h7 = 0x5be0cd19 | 0;

        for (let off = 0; off < total; off += 64) {{
          for (let i = 0; i < 16; i++) {{
            const j = off + (i << 2);
            W[i] = (BUF[j] << 24) | (BUF[j + 1] << 16) | (BUF[j + 2] << 8) | BUF[j + 3];
          }}
          for (let i = 16; i < 64; i++) {{
            const w15 = W[i - 15], w2 = W[i - 2];
            const s0 = ((w15 >>> 7) | (w15 << 25)) ^ ((w15 >>> 18) | (w15 << 14)) ^ (w15 >>> 3);
            const s1 = ((w2 >>> 17) | (w2 << 15)) ^ ((w2 >>> 19) | (w2 << 13)) ^ (w2 >>> 10);
            W[i] = (W[i - 16] + s0 + W[i - 7] + s1) | 0;
          }}
          let a = h0, b = h1, c = h2, d = h3, e = h4, f = h5, g = h6, h = h7;
          for (let i = 0; i < 64; i++) {{
            const S1 = ((e >>> 6) | (e << 26)) ^ ((e >>> 11) | (e << 21)) ^ ((e >>> 25) | (e << 7));
            const ch = (e & f) ^ (~e & g);
            const t1 = (h + S1 + ch + K[i] + W[i]) | 0;
            const S0 = ((a >>> 2) | (a << 30)) ^ ((a >>> 13) | (a << 19)) ^ ((a >>> 22) | (a << 10));
            const maj = (a & b) ^ (a & c) ^ (b & c);
            const t2 = (S0 + maj) | 0;
            h = g; g = f; f = e; e = (d + t1) | 0;
            d = c; c = b; b = a; a = (t1 + t2) | 0;
          }}
          h0 = (h0 + a) | 0; h1 = (h1 + b) | 0; h2 = (h2 + c) | 0; h3 = (h3 + d) | 0;
          h4 = (h4 + e) | 0; h5 = (h5 + f) | 0; h6 = (h6 + g) | 0; h7 = (h7 + h) | 0;
        }}
        return h0 >>> 0;
      }}
      function matchesDifficulty(h0, bits) {{
        if (bits <= 0) return true;
        if (bits >= 32) return h0 === 0;
        return (h0 >>> (32 - bits)) === 0;
      }}
    `;

    const captchaState = {{
      enabled: true,
      status: 'idle',
      action: 'signin',
      challenge: null,
      solution: null,
      interacted: false,
      runId: 0,
      worker: null,
      workerUrl: '',
      fallbackTimer: null,
      expiryTimer: null,
      autoStartTimer: null,
      inFlightPromise: null,
      workerSupported: typeof Worker === 'function' && typeof Blob === 'function',
    }};

    function computeBrowserProbe(probe) {{
      if (!probe) return 0;
      return ((Math.imul(probe.a | 0, probe.b | 0) ^ (probe.c | 0)) >>> 0);
    }}

    function detectAutomationGlobals() {{
      try {{
        const winKeys = [
          '__playwright',
          '__pw_manual',
          '__PW_inspect',
          '__nightmare',
          '_phantom',
          'callPhantom',
          'domAutomation',
          'domAutomationController',
          '__selenium_unwrapped',
          '__webdriver_evaluate',
          '__driver_evaluate',
        ];
        for (const k of winKeys) {{
          if (k in window && window[k]) return true;
        }}
        if (document) {{
          for (const k of Object.keys(document)) {{
            if (k.startsWith('$cdc_') || k.startsWith('$wdc_')) return true;
          }}
        }}
      }} catch {{}}
      return false;
    }}

    function checkCanvasRender() {{
      try {{
        const c = document.createElement('canvas');
        c.width = 16;
        c.height = 16;
        const ctx = c.getContext && c.getContext('2d');
        if (!ctx) return false;
        ctx.fillStyle = '#1ed760';
        ctx.fillRect(2, 2, 12, 12);
        const data = ctx.getImageData(4, 4, 1, 1).data;
        return data && data.length === 4 && data[3] > 0;
      }} catch {{
        return false;
      }}
    }}

    function collectBrowserSignals() {{
      const ua = String(navigator.userAgent || '');
      const headlessUa = /HeadlessChrome|PhantomJS|Playwright|Puppeteer|Selenium/i.test(ua);
      const card = document.querySelector('#auth-modal .modal-box');
      const rect = card && card.getBoundingClientRect ? card.getBoundingClientRect() : null;
      const layoutOk = !!(rect && rect.width > 100 && rect.height > 100);
      return {{
        webdriver: navigator.webdriver === true,
        automation: detectAutomationGlobals(),
        headlessUa,
        languages: Array.isArray(navigator.languages) ? navigator.languages.length : (navigator.language ? 1 : 0),
        screenW: (window.screen && window.screen.width) | 0,
        screenH: (window.screen && window.screen.height) | 0,
        innerW: window.innerWidth | 0,
        innerH: window.innerHeight | 0,
        cores: (navigator.hardwareConcurrency || 0) | 0,
        canvasOk: checkCanvasRender(),
        layoutOk,
        workerOk: captchaState.workerSupported,
        signalsPresent: true,
      }};
    }}

    function setCaptchaProgress(pct) {{
      const fill = $('sa-captcha-progress-fill');
      if (!fill) return;
      const clamped = Math.max(0, Math.min(100, Number(pct) || 0));
      fill.style.width = `${{clamped}}%`;
    }}

    function setCaptchaUi(status, label, sub, progressPct) {{
      captchaState.status = status;
      const root = $('sa-captcha');
      const btn = $('sa-captcha-btn');
      const lblEl = $('sa-captcha-label');
      const subEl = $('sa-captcha-sub');
      if (root) {{
        root.classList.toggle('hidden', !captchaState.enabled);
        root.dataset.state = status;
      }}
      if (btn) {{
        btn.setAttribute('aria-checked', status === 'verified' ? 'true' : 'false');
      }}
      if (lblEl && label !== undefined) lblEl.textContent = label;
      if (subEl && sub !== undefined) subEl.textContent = sub;
      if (progressPct !== undefined) setCaptchaProgress(progressPct);
    }}

    function cancelCaptchaWork() {{
      captchaState.runId += 1;
      captchaState.inFlightPromise = null;
      if (captchaState.autoStartTimer) {{
        clearTimeout(captchaState.autoStartTimer);
        captchaState.autoStartTimer = null;
      }}
      if (captchaState.expiryTimer) {{
        clearTimeout(captchaState.expiryTimer);
        captchaState.expiryTimer = null;
      }}
      if (captchaState.fallbackTimer) {{
        clearTimeout(captchaState.fallbackTimer);
        captchaState.fallbackTimer = null;
      }}
      if (captchaState.worker) {{
        try {{ captchaState.worker.terminate(); }} catch {{}}
        captchaState.worker = null;
      }}
      if (captchaState.workerUrl) {{
        try {{ URL.revokeObjectURL(captchaState.workerUrl); }} catch {{}}
        captchaState.workerUrl = '';
      }}
    }}

    function solvePowChallenge(challenge, probeAnswer, runId, onProgress) {{
      const prefix = `${{challenge.id}}:${{challenge.salt}}:${{probeAnswer}}:`;
      const difficulty = challenge.difficulty | 0;
      const expectedTries = Math.max(1, Math.pow(2, difficulty));
      const startedAt = performance.now ? performance.now() : Date.now();

      const runMainThreadFallback = () =>
        new Promise((resolve, reject) => {{
          const solverFn = new Function(
            'prefix',
            'difficulty',
            'startNonce',
            'batchSize',
            `${{POW_SHA256_CORE}}
            const prefixLen = prefix.length;
            const prefixBytes = new Uint8Array(prefixLen);
            for (let i = 0; i < prefixLen; i++) prefixBytes[i] = prefix.charCodeAt(i) & 0xff;
            for (let i = 0; i < batchSize; i++) {{
              const nonce = startNonce + i;
              if (matchesDifficulty(sha256FirstWordAscii(prefixBytes, prefixLen, String(nonce)), difficulty)) {{
                return nonce;
              }}
            }}
            return -1;`
          );
          let nonce = 0;
          const batchSize = 4096;
          const step = () => {{
            if (runId !== captchaState.runId) {{
              reject(new Error('cancelled'));
              return;
            }}
            const found = solverFn(prefix, difficulty, nonce, batchSize);
            if (found >= 0) {{
              const elapsed = Math.round((performance.now ? performance.now() : Date.now()) - startedAt);
              resolve({{ nonce: found, elapsedMs: Math.max(1, elapsed) }});
              return;
            }}
            nonce += batchSize;
            if (onProgress) onProgress(Math.min(92, Math.round((1 - Math.exp(-nonce / expectedTries)) * 100)));
            captchaState.fallbackTimer = setTimeout(step, 0);
          }};
          step();
        }});

      if (!captchaState.workerSupported) {{
        return runMainThreadFallback();
      }}

      return new Promise((resolve, reject) => {{
        try {{
          const workerSrc = `
            ${{POW_SHA256_CORE}}
            self.onmessage = function(e) {{
              const {{ prefix, difficulty }} = e.data;
              const prefixLen = prefix.length;
              const prefixBytes = new Uint8Array(prefixLen);
              for (let i = 0; i < prefixLen; i++) prefixBytes[i] = prefix.charCodeAt(i) & 0xff;
              const t0 = performance.now();
              let nonce = 0;
              while (true) {{
                for (let i = 0; i < 8192; i++) {{
                  const h0 = sha256FirstWordAscii(prefixBytes, prefixLen, String(nonce));
                  if (matchesDifficulty(h0, difficulty)) {{
                    self.postMessage({{ type: 'done', nonce, elapsedMs: Math.max(1, Math.round(performance.now() - t0)) }});
                    return;
                  }}
                  nonce++;
                }}
                self.postMessage({{ type: 'progress', tried: nonce }});
              }}
            }};
          `;
          const blob = new Blob([workerSrc], {{ type: 'application/javascript' }});
          const url = URL.createObjectURL(blob);
          const worker = new Worker(url);
          captchaState.worker = worker;
          captchaState.workerUrl = url;

          const cleanup = () => {{
            if (captchaState.worker === worker) {{
              captchaState.worker = null;
            }}
            try {{ worker.terminate(); }} catch {{}}
            if (captchaState.workerUrl === url) {{
              captchaState.workerUrl = '';
            }}
            try {{ URL.revokeObjectURL(url); }} catch {{}}
          }};

          worker.onmessage = (ev) => {{
            if (runId !== captchaState.runId) {{
              cleanup();
              reject(new Error('cancelled'));
              return;
            }}
            const msg = ev.data || {{}};
            if (msg.type === 'progress') {{
              if (onProgress) {{
                onProgress(Math.min(94, Math.round((1 - Math.exp(-msg.tried / expectedTries)) * 100)));
              }}
            }} else if (msg.type === 'done') {{
              cleanup();
              resolve({{ nonce: msg.nonce, elapsedMs: msg.elapsedMs }});
            }}
          }};

          worker.onerror = () => {{
            cleanup();
            captchaState.workerSupported = false;
            if (runId !== captchaState.runId) {{
              reject(new Error('cancelled'));
              return;
            }}
            runMainThreadFallback().then(resolve, reject);
          }};

          worker.postMessage({{ prefix, difficulty }});
        }} catch {{
          captchaState.workerSupported = false;
          runMainThreadFallback().then(resolve, reject);
        }}
      }});
    }}

    function captchaSolutionFresh(action) {{
      if (!captchaState.enabled) return true;
      if (captchaState.status !== 'verified' || !captchaState.solution || !captchaState.challenge) {{
        return false;
      }}
      if (captchaState.action !== action) return false;
      const nowSec = Math.floor(Date.now() / 1000);
      return nowSec < (captchaState.challenge.expiresAt || 0) - 10;
    }}

    async function solveCurrentChallenge(runId, userTriggered) {{
      const ch = captchaState.challenge;
      if (!ch) return null;
      if (userTriggered) captchaState.interacted = true;
      setCaptchaUi('solving', 'Verifying browser…', 'Solving proof-of-work…', 18);
      const probeAnswer = computeBrowserProbe(ch.probe);
      const solved = await solvePowChallenge(ch, probeAnswer, runId, (pct) => {{
        if (runId === captchaState.runId) {{
          setCaptchaProgress(18 + Math.round(pct * 0.78));
        }}
      }});
      if (runId !== captchaState.runId) return null;

      captchaState.solution = {{
        id: ch.id,
        nonce: solved.nonce,
        mac: ch.mac,
        probe: probeAnswer,
        interacted: captchaState.interacted,
        elapsedMs: solved.elapsedMs,
      }};
      setCaptchaUi('verified', 'Verified', `Browser check passed · ${{solved.elapsedMs}} ms`, 100);

      const ttlMs = Math.max(15000, ((ch.expiresAt || 0) - Math.floor(Date.now() / 1000) - 15) * 1000);
      captchaState.expiryTimer = setTimeout(() => {{
        if (runId !== captchaState.runId) return;
        captchaState.solution = null;
        captchaState.challenge = null;
        setCaptchaUi('idle', "Verify you're human", 'Verification expired — click to refresh', 0);
      }}, ttlMs);

      return captchaState.solution;
    }}

    function startCaptchaFlow(action, opts = {{}}) {{
      const userTriggered = Boolean(opts.userTriggered);
      cancelCaptchaWork();
      const runId = captchaState.runId;
      captchaState.action = action === 'signup' ? 'signup' : 'signin';
      captchaState.challenge = null;
      captchaState.solution = null;
      captchaState.interacted = userTriggered;

      const promise = (async () => {{
        setCaptchaUi('checking', 'Checking browser…', 'Inspecting browser environment', 10);
        try {{
          const signals = collectBrowserSignals();
          const resp = await fetch('/api/web/auth/captcha', {{
            method: 'POST',
            headers: {{ 'Content-Type': 'application/json' }},
            body: JSON.stringify({{ action: captchaState.action, signals }}),
          }});
          const ch = await resp.json().catch(() => ({{}}));
          if (!resp.ok) throw new Error(ch.error || `Request failed (${{resp.status}})`);
          if (runId !== captchaState.runId) return null;
          if (ch && ch.enabled === false) {{
            captchaState.enabled = false;
            setCaptchaUi('verified', 'Verified', '', 100);
            return null;
          }}
          captchaState.enabled = true;
          captchaState.challenge = ch;

          if (ch.interactive && !captchaState.interacted) {{
            setCaptchaUi(
              'interactive',
              "Click to verify you're human",
              'Additional browser verification required',
              0
            );
            return null;
          }}

          return await solveCurrentChallenge(runId, captchaState.interacted);
        }} catch (err) {{
          if (runId !== captchaState.runId) return null;
          const msg = err && err.message ? err.message : 'Could not reach verification service';
          setCaptchaUi('error', 'Verification failed', `${{msg}} — tap to retry`, 0);
          return null;
        }} finally {{
          if (captchaState.inFlightPromise === promise) {{
            captchaState.inFlightPromise = null;
          }}
        }}
      }})();

      captchaState.inFlightPromise = promise;
      return promise;
    }}

    function resetCaptchaAndStart(action, delayMs = 120) {{
      cancelCaptchaWork();
      captchaState.action = action === 'signup' ? 'signup' : 'signin';
      captchaState.challenge = null;
      captchaState.solution = null;
      captchaState.interacted = false;
      setCaptchaUi('idle', "Verify you're human", 'Browser & proof-of-work check', 0);
      const modal = $('auth-modal');
      if (!modal || modal.classList.contains('hidden')) return;
      const targetAction = captchaState.action;
      captchaState.autoStartTimer = setTimeout(() => {{
        captchaState.autoStartTimer = null;
        if ($('auth-modal') && !$('auth-modal').classList.contains('hidden') && state.authMode === targetAction) {{
          startCaptchaFlow(targetAction, {{ userTriggered: false }});
        }}
      }}, delayMs);
    }}

    async function onCaptchaButtonClick(e) {{
      const trusted = Boolean(e && e.isTrusted);
      const action = state.authMode === 'signup' ? 'signup' : 'signin';
      if (captchaSolutionFresh(action)) return;
      if (captchaState.status === 'checking' || captchaState.status === 'solving') {{
        if (trusted) captchaState.interacted = true;
        return;
      }}
      if (captchaState.status === 'interactive' && captchaState.challenge) {{
        const nowSec = Math.floor(Date.now() / 1000);
        if (nowSec < (captchaState.challenge.expiresAt || 0) - 15) {{
          const runId = captchaState.runId;
          const promise = (async () => {{
            try {{
              return await solveCurrentChallenge(runId, trusted);
            }} catch {{
              if (runId !== captchaState.runId) return null;
              setCaptchaUi('error', 'Verification failed', 'Tap to try again', 0);
              return null;
            }} finally {{
              if (captchaState.inFlightPromise === promise) {{
                captchaState.inFlightPromise = null;
              }}
            }}
          }})();
          captchaState.inFlightPromise = promise;
          await promise;
          return;
        }}
      }}
      await startCaptchaFlow(action, {{ userTriggered: trusted }});
    }}

    async function ensureCaptchaReady(action, userTriggered) {{
      if (!captchaState.enabled) return null;
      if (captchaSolutionFresh(action)) return captchaState.solution;
      if (captchaState.status === 'interactive') {{
        throw new Error('Please click the verification checkbox above to continue.');
      }}
      if (captchaState.inFlightPromise) {{
        await captchaState.inFlightPromise;
        if (!captchaState.enabled) return null;
        if (captchaSolutionFresh(action)) return captchaState.solution;
        if (captchaState.status === 'interactive') {{
          throw new Error('Please click the verification checkbox above to continue.');
        }}
      }}
      await startCaptchaFlow(action, {{ userTriggered }});
      if (!captchaState.enabled) return null;
      if (captchaSolutionFresh(action)) return captchaState.solution;
      if (captchaState.status === 'interactive') {{
        throw new Error('Please click the verification checkbox above to continue.');
      }}
      throw new Error('Human verification failed. Please click the verification box to try again.');
    }}

    const captchaBtn = $('sa-captcha-btn');
    if (captchaBtn) {{
      captchaBtn.addEventListener('click', (e) => {{
        const err = $('sa-err');
        if (err) err.classList.add('hidden');
        onCaptchaButtonClick(e);
      }});
    }}

    window.ShareApp = {{
      togglePlay: () => {{
        if (!tracks.length) return;
        if (!audio.getAttribute('src')) loadTrack(state.idx, true);
        else if (audio.paused) audio.play().catch(() => {{}});
        else audio.pause();
      }},
      playIndex: (i) => {{
        rebuildPlayOrder(i);
        loadTrack(i, true);
      }},
      playShuffled: () => {{
        if (!tracks.length) return;
        state.shuffle = true;
        $('btn-shuffle').classList.add('active');
        rebuildPlayOrder(null);
        loadTrack(state.order[0], true);
      }},
      prev: () => {{
        if (audio.currentTime > 4) {{ audio.currentTime = 0; return; }}
        advanceShareTrack(-1);
      }},
      next: () => {{
        advanceShareTrack(1);
      }},
      toggleShuffle: () => {{
        state.shuffle = !state.shuffle;
        $('btn-shuffle').classList.toggle('active', state.shuffle);
        rebuildPlayOrder(state.idx);
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
          if (audio.paused) {{ audio.play().catch(() => {{}}); }}
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
        state.authMode = mode === 'signup' ? 'signup' : 'signin';
        const signup = state.authMode === 'signup';
        $('auth-modal-title').textContent = signup ? 'Create Free Beatlab Account' : 'Sign In to Beatlab';
        $('sa-confirm-row').classList.toggle('hidden', !signup);
        $('sa-confirm').required = signup;
        if (!signup) $('sa-confirm').value = '';
        if ($('sa-hp')) $('sa-hp').value = '';
        $('sa-submit').disabled = false;
        $('sa-submit').textContent = signup ? 'Create account' : 'Sign in';
        $('sa-err').classList.add('hidden');
        $('auth-modal').classList.remove('hidden');
        resetCaptchaAndStart(state.authMode);
        setTimeout(() => $('sa-user')?.focus(), 50);
      }},
      closeAuth: () => {{
        cancelCaptchaWork();
        $('auth-modal').classList.add('hidden');
      }},
      submitAuth: async (e) => {{
        e.preventDefault();
        const err = $('sa-err');
        err.classList.add('hidden');
        const signup = state.authMode === 'signup';
        const action = signup ? 'signup' : 'signin';
        const username = $('sa-user').value.trim();
        const password = $('sa-pass').value;
        const confirmPassword = signup ? $('sa-confirm').value : '';
        if (signup && password !== confirmPassword) {{
          err.textContent = 'Passwords do not match';
          err.classList.remove('hidden');
          return;
        }}
        const submitBtn = $('sa-submit');
        const setSubmitting = (busy, label) => {{
          if (!submitBtn) return;
          submitBtn.disabled = busy;
          submitBtn.textContent = busy ? label : (signup ? 'Create account' : 'Sign in');
        }};
        let captchaSolution = null;
        try {{
          if (captchaState.enabled && !captchaSolutionFresh(action)) {{
            setSubmitting(true, 'Verifying…');
          }}
          captchaSolution = await ensureCaptchaReady(action, Boolean(e && e.isTrusted));
        }} catch (ex) {{
          setSubmitting(false);
          err.textContent = ex.message;
          err.classList.remove('hidden');
          if (captchaState.status === 'interactive' && $('sa-captcha-btn')) {{
            $('sa-captcha-btn').focus();
          }}
          return;
        }}
        const hpToken = ($('sa-hp') && $('sa-hp').value) || '';
        setSubmitting(true, signup ? 'Creating account…' : 'Signing in…');
        try {{
          const payload = signup
            ? {{ username, password, confirmPassword, captcha: captchaSolution, hpToken }}
            : {{ username, password, captcha: captchaSolution, hpToken }};
          const r = await fetch(signup ? '/api/web/auth/signup' : '/api/web/auth/signin', {{
            method: 'POST',
            headers: {{ 'Content-Type': 'application/json' }},
            body: JSON.stringify(payload),
          }});
          const res = await r.json().catch(() => ({{}}));
          if (!r.ok) throw new Error(res.error || 'Authentication failed');
          cancelCaptchaWork();
          if (res.mfaRequired) {{
            err.textContent = 'This account uses two-factor authentication. Please sign in from the main Beatlab app.';
            err.classList.remove('hidden');
            return;
          }}
          if (res.restoreRequired) {{
            err.textContent = 'This account is scheduled for deletion. Please sign in from the main Beatlab app to restore it.';
            err.classList.remove('hidden');
            return;
          }}
          state.token = res.token;
          state.user = res.user;
          localStorage.setItem('beatlab_token', res.token);
          $('auth-modal').classList.add('hidden');
          renderSignedInUI();
          toast(res.created ? `Welcome, ${{res.user.username}}! Account created.` : `Welcome back, ${{res.user.username}}!`);
        }} catch (ex) {{
          err.textContent = ex.message;
          err.classList.remove('hidden');
          resetCaptchaAndStart(action, 160);
        }} finally {{
          setSubmitting(false);
        }}
      }},
    }};

    if (tracks.length > 0) loadTrack(0, false);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn share_page_auth_modal_omits_subtitle_and_includes_captcha() {
        let share = ShareRecord {
            id: "testshare1".into(),
            kind: "track".into(),
            target_id: "loc:1".into(),
            title: "Sun".into(),
            subtitle: "Yorushika".into(),
            user_id: 1,
            creator_name: "cross".into(),
            track_ids: vec!["loc:1".into()],
            created: 1_700_000_000,
        };
        let track = Track {
            id: "loc:1".into(),
            source: "local".into(),
            title: "Sun".into(),
            artist: "Yorushika".into(),
            album: "Album".into(),
            duration: 266,
            format: "flac".into(),
            ..Default::default()
        };
        let html = render_share_html("https://beatlab.example", &share, &[track], false, "audio/flac");

        assert!(!html.contains("sa-desc"));
        assert!(!html.contains("Pick a username and password"));
        assert!(!html.contains("Sign in with your Beatlab username and password"));

        assert!(html.contains(r#"id="sa-captcha""#));
        assert!(html.contains(r#"id="sa-captcha-btn""#));
        assert!(html.contains(r#"id="sa-hp""#));
        assert!(html.contains("/api/web/auth/captcha"));
        assert!(html.contains("ensureCaptchaReady"));
        assert!(html.contains("captcha: captchaSolution, hpToken"));
    }
}
