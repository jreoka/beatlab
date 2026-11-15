use crate::models::{core_song_title, err_msg, primary_artist, slug, upgrade_cover_url, AppResult, Track};
use crate::sources::MusicSource;
use crate::taste::classify_moods_str;
use async_trait::async_trait;
use chrono::Utc;
use serde_json::Value;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;
use tracing::{debug, warn};

const INNERTUBE_SEARCH_URL: &str = "https://music.youtube.com/youtubei/v1/search?prettyPrint=false";
const INNERTUBE_NEXT_URL: &str = "https://music.youtube.com/youtubei/v1/next?prettyPrint=false";
const YTM_SONGS_PARAMS: &str = "EgWKAQIIAWoKEAMQBBAJEAoQBQ%3D%3D";

#[derive(Debug, Clone)]
pub struct ResolvedStream {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub ext: String,
    pub source_webpage_url: String,
}

#[derive(Debug, Clone)]
pub struct YtmCandidate {
    pub video_id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration: i32,
    pub cover_url: String,
}

#[derive(Clone)]
pub struct YtmSource {
    http: reqwest::Client,
    ytdlp_bin: String,
}

impl YtmSource {
    pub fn new(ytdlp_override: Option<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(12))
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36")
            .build()
            .unwrap_or_default();
        let ytdlp_bin = ytdlp_override
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(detect_ytdlp_bin);
        Self { http, ytdlp_bin }
    }

    pub fn ytdlp_bin(&self) -> &str {
        &self.ytdlp_bin
    }

    async fn innertube_post(&self, url: &str, body: Value) -> Option<Value> {
        let resp = match self
            .http
            .post(url)
            .header("Content-Type", "application/json")
            .header("Origin", "https://music.youtube.com")
            .header("Referer", "https://music.youtube.com/")
            .json(&body)
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => {
                debug!(status = %r.status(), url, "youtube music innertube non-success");
                return None;
            }
            Err(e) => {
                debug!(error = %e, url, "youtube music innertube error");
                return None;
            }
        };
        resp.json::<Value>().await.ok()
    }

    pub async fn search_innertube(
        &self,
        query: &str,
        songs_only: bool,
        limit: usize,
    ) -> Vec<YtmCandidate> {
        let q = query.trim();
        if q.is_empty() {
            return vec![];
        }
        let mut body = serde_json::json!({
            "context": innertube_client_context(),
            "query": q
        });
        if songs_only {
            body["params"] = Value::String(YTM_SONGS_PARAMS.to_string());
        }

        let Some(val) = self.innertube_post(INNERTUBE_SEARCH_URL, body).await else {
            return vec![];
        };
        let mut out = parse_innertube_results(&val);
        out.truncate(limit);
        out
    }

    /// Fetch official YouTube Music "Up Next" / Track Radio mix tracks seeded by `video_id`.
    pub async fn fetch_radio_tracks(&self, video_id: &str, limit: usize) -> Vec<Track> {
        let vid = video_id.trim().trim_start_matches("ytm:").trim();
        if vid.is_empty() {
            return vec![];
        }

        let mut cands = Vec::new();
        for playlist_prefix in ["RDAMVM", "RD"] {
            let body = serde_json::json!({
                "context": innertube_client_context(),
                "videoId": vid,
                "playlistId": format!("{}{}", playlist_prefix, vid),
                "isAudioOnly": true
            });
            if let Some(val) = self.innertube_post(INNERTUBE_NEXT_URL, body).await {
                cands = parse_innertube_radio_results(&val, vid);
                if !cands.is_empty() {
                    break;
                }
            }
        }

        let now = Utc::now().timestamp();
        cands
            .into_iter()
            .filter_map(|c| candidate_to_track(c, now))
            .take(limit)
            .collect()
    }

    /// Find the best YouTube Music video ID matching `(artist, title, duration)`.
    pub async fn find_best_video_for_track(
        &self,
        artist: &str,
        title: &str,
        target_duration: i32,
    ) -> Option<String> {
        let pa = primary_artist(artist);
        let clean_artist = if pa.is_empty() { artist.trim() } else { pa.as_str() };
        let clean_title = crate::models::strip_release_tags(title);
        let q = format!("{} {}", clean_artist, clean_title).trim().to_string();

        let mut cands = self.search_innertube(&q, true, 8).await;
        if cands.is_empty() {
            cands = self.search_innertube(&q, false, 8).await;
        }
        if !cands.is_empty() {
            cands.sort_by_key(|c| {
                std::cmp::Reverse(score_ytm_candidate(c, clean_artist, title, target_duration))
            });
            return Some(cands.into_iter().next()?.video_id);
        }

        // Last-resort yt-dlp flat search if Innertube HTTP is temporarily rate-limited.
        self.ytdlp_flat_search(&q, clean_artist, title, target_duration)
            .await
    }

    async fn ytdlp_flat_search(
        &self,
        query: &str,
        artist: &str,
        title: &str,
        target_duration: i32,
    ) -> Option<String> {
        let search_arg = format!("ytsearch4:{} audio", query);
        let mut cmd = Command::new(&self.ytdlp_bin);
        cmd.args([
            "-j",
            "--flat-playlist",
            "--no-warnings",
            "--no-check-certificates",
            &search_arg,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

        let out = tokio::time::timeout(Duration::from_secs(12), cmd.output())
            .await
            .ok()?
            .ok()?;
        let stdout = String::from_utf8_lossy(&out.stdout);
        let mut cands = Vec::new();
        for line in stdout.lines() {
            if let Ok(v) = serde_json::from_str::<Value>(line) {
                let vid = v.get("id").and_then(|s| s.as_str()).unwrap_or("");
                if vid.is_empty() {
                    continue;
                }
                cands.push(YtmCandidate {
                    video_id: vid.to_string(),
                    title: v
                        .get("title")
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string(),
                    artist: v
                        .get("uploader")
                        .or_else(|| v.get("channel"))
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string(),
                    album: String::new(),
                    duration: v.get("duration").and_then(|d| d.as_f64()).unwrap_or(0.0) as i32,
                    cover_url: String::new(),
                });
            }
        }
        cands.sort_by_key(|c| std::cmp::Reverse(score_ytm_candidate(c, artist, title, target_duration)));
        cands.into_iter().next().map(|c| c.video_id)
    }

    /// Resolve a playable HTTP audio URL + headers for a YouTube video ID or URL.
    pub async fn resolve_stream(&self, video_id_or_url: &str) -> AppResult<ResolvedStream> {
        let watch_url = if video_id_or_url.starts_with("http://")
            || video_id_or_url.starts_with("https://")
        {
            video_id_or_url.to_string()
        } else {
            let vid = video_id_or_url.trim_start_matches("ytm:");
            format!("https://www.youtube.com/watch?v={}", vid)
        };

        // Empirically verified on Windows + YouTube 2026: `android` client yields direct
        // progressive/audio URLs that return HTTP 206 Partial Content when fetched with
        // the returned `http_headers`. Try `android` first, then `ios`, then default.
        let attempts = [
            ("youtube:player_client=android", "bestaudio[protocol!=m3u8][protocol!=m3u8_native]/best[protocol!=m3u8][protocol!=m3u8_native]/best"),
            ("youtube:player_client=ios", "bestaudio[protocol!=m3u8][protocol!=m3u8_native]/best[protocol!=m3u8][protocol!=m3u8_native]/best"),
            ("", "bestaudio/best"),
        ];

        let mut last_err = String::new();
        for (extractor_args, fmt) in attempts {
            match self.run_ytdlp_json(&watch_url, extractor_args, fmt).await {
                Ok(res) => return Ok(res),
                Err(e) => {
                    warn!(client = extractor_args, error = %e, "yt-dlp resolution attempt failed");
                    last_err = e.to_string();
                }
            }
        }
        Err(err_msg(format!("yt-dlp resolution failed: {}", last_err)))
    }

    async fn run_ytdlp_json(
        &self,
        watch_url: &str,
        extractor_args: &str,
        format_sel: &str,
    ) -> AppResult<ResolvedStream> {
        let mut cmd = Command::new(&self.ytdlp_bin);
        cmd.arg("-j").arg("--no-playlist").arg("--no-warnings");
        if !extractor_args.is_empty() {
            cmd.arg("--extractor-args").arg(extractor_args);
        }
        cmd.arg("-f").arg(format_sel).arg(watch_url);
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        let out = tokio::time::timeout(Duration::from_secs(18), cmd.output())
            .await
            .map_err(|_| err_msg("yt-dlp timed out"))??;

        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            return Err(err_msg(if err.is_empty() {
                "yt-dlp returned non-zero status".to_string()
            } else {
                err
            }));
        }

        let line = String::from_utf8_lossy(&out.stdout);
        let first_json = line
            .lines()
            .find(|l| l.trim_start().starts_with('{'))
            .ok_or_else(|| err_msg("yt-dlp produced empty JSON"))?;
        let v: Value = serde_json::from_str(first_json)?;

        let url = v
            .get("url")
            .and_then(|u| u.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if url.is_empty() || (!url.starts_with("http://") && !url.starts_with("https://")) {
            return Err(err_msg("yt-dlp returned non-HTTP stream URL"));
        }

        let mut headers = Vec::new();
        if let Some(obj) = v.get("http_headers").and_then(|h| h.as_object()) {
            for (k, val) in obj {
                if let Some(s) = val.as_str() {
                    headers.push((k.clone(), s.to_string()));
                }
            }
        }
        let ext = v
            .get("ext")
            .and_then(|s| s.as_str())
            .unwrap_or("m4a")
            .to_lowercase();

        Ok(ResolvedStream {
            url,
            headers,
            ext,
            source_webpage_url: watch_url.to_string(),
        })
    }
}

#[async_trait]
impl MusicSource for YtmSource {
    fn name(&self) -> &'static str {
        "ytm"
    }

    fn available(&self) -> bool {
        true
    }

    fn status(&self) -> String {
        format!("youtube music — fast innertube search + {}", self.ytdlp_bin)
    }

    async fn search(&self, query: &str, limit: usize) -> Vec<Track> {
        let mut items = self.search_innertube(query, true, limit).await;
        if items.is_empty() {
            items = self.search_innertube(query, false, limit).await;
        }
        let now = Utc::now().timestamp();
        items
            .into_iter()
            .filter_map(|c| candidate_to_track(c, now))
            .collect()
    }
}

fn candidate_to_track(c: YtmCandidate, created: i64) -> Option<Track> {
    let dur = c.duration.max(0);
    if dur > 0 && (dur < 35 || dur > 1500) {
        // Skip tiny snippets and multi-hour mixes.
        return None;
    }
    let artist = if c.artist.is_empty() {
        "Unknown Artist".to_string()
    } else {
        c.artist
    };
    let album = if c.album.is_empty() {
        c.title.clone()
    } else {
        c.album
    };
    let moods = classify_moods_str(&c.title, &album, "", "");
    let cover_url = upgrade_cover_url(&c.cover_url);
    Some(Track {
        id: format!("ytm:{}", c.video_id),
        source: "ytm".to_string(),
        title: c.title,
        artist,
        album,
        genre: String::new(),
        moods,
        year: 0,
        track_no: 0,
        disc_no: 1,
        duration: dur,
        bitrate: 192,
        sample_rate: 44100,
        bit_depth: 0,
        format: "m4a".to_string(),
        size: 0,
        stream_url: format!("ytm:{}", c.video_id),
        cover_url,
        meta: format!("https://music.youtube.com/watch?v={}", c.video_id),
        deezer_id: String::new(),
        created,
    })
}

fn innertube_client_context() -> Value {
    serde_json::json!({
        "client": {
            "clientName": "WEB_REMIX",
            "clientVersion": "1.20241127.01.00",
            "hl": "en",
            "gl": "US"
        }
    })
}

fn detect_ytdlp_bin() -> String {
    if cfg!(windows) {
        for cand in ["yt-dlp.exe", "yt-dlp"] {
            if std::process::Command::new(cand)
                .arg("--version")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok()
            {
                return cand.to_string();
            }
        }
        "yt-dlp".to_string()
    } else {
        "yt-dlp".to_string()
    }
}

pub fn parse_innertube_results(root: &Value) -> Vec<YtmCandidate> {
    let mut raw_renderers = Vec::new();
    collect_objects_by_key(root, "musicResponsiveListItemRenderer", &mut raw_renderers);
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for item in raw_renderers {
        if let Some(c) = extract_candidate(item) {
            if seen.insert(c.video_id.clone()) {
                out.push(c);
            }
        }
    }
    out
}

fn parse_innertube_radio_results(root: &Value, seed_video_id: &str) -> Vec<YtmCandidate> {
    let mut raw_renderers = Vec::new();
    collect_objects_by_key(root, "playlistPanelVideoRenderer", &mut raw_renderers);
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for item in raw_renderers {
        if let Some(c) = extract_panel_candidate(item, seed_video_id) {
            if seen.insert(c.video_id.clone()) {
                out.push(c);
            }
        }
    }
    out
}

fn collect_objects_by_key<'a>(v: &'a Value, key: &str, out: &mut Vec<&'a Value>) {
    match v {
        Value::Object(map) => {
            if let Some(r) = map.get(key) {
                out.push(r);
            }
            for child in map.values() {
                collect_objects_by_key(child, key, out);
            }
        }
        Value::Array(arr) => {
            for child in arr {
                collect_objects_by_key(child, key, out);
            }
        }
        _ => {}
    }
}

fn best_thumbnail_url(thumbs: Option<&Value>, video_id: &str) -> String {
    let raw = thumbs
        .and_then(|t| t.as_array())
        .and_then(|arr| arr.last())
        .and_then(|t| t.get("url"))
        .and_then(|u| u.as_str())
        .unwrap_or("")
        .to_string();
    let candidate = if raw.is_empty() {
        format!("https://i.ytimg.com/vi/{}/hqdefault.jpg", video_id)
    } else {
        raw
    };
    upgrade_cover_url(&candidate)
}

fn extract_panel_candidate(item: &Value, seed_video_id: &str) -> Option<YtmCandidate> {
    let video_id = item
        .get("videoId")
        .or_else(|| item.pointer("/navigationEndpoint/watchEndpoint/videoId"))
        .and_then(|v| v.as_str())?
        .trim()
        .to_string();
    if video_id.is_empty() || video_id == seed_video_id {
        return None;
    }

    let title = item
        .pointer("/title/runs")
        .and_then(join_runs_text)
        .unwrap_or_default();
    if title.is_empty() {
        return None;
    }

    let subtitle_runs = item
        .pointer("/longBylineText/runs")
        .or_else(|| item.pointer("/shortBylineText/runs"))
        .and_then(|r| r.as_array());

    let mut artist = String::new();
    let mut album = String::new();
    let mut duration = item
        .pointer("/lengthText/runs")
        .and_then(join_runs_text)
        .and_then(|s| parse_mm_ss(&s))
        .unwrap_or(0);

    if let Some(runs) = subtitle_runs {
        for r in runs {
            let Some(part) = r.get("text").and_then(|t| t.as_str()).map(str::trim) else {
                continue;
            };
            if part.is_empty() || part == "•" || part == "·" || part == "&" || part == "," {
                continue;
            }
            let page_type = r
                .pointer("/navigationEndpoint/browseEndpoint/browseEndpointContextSupportedConfigs/browseEndpointContextMusicConfig/pageType")
                .and_then(|p| p.as_str())
                .unwrap_or("");
            if page_type == "MUSIC_PAGE_TYPE_ARTIST" {
                if artist.is_empty() {
                    artist = part.to_string();
                } else if !artist.contains(part) {
                    artist = format!("{}, {}", artist, part);
                }
                continue;
            }
            if page_type == "MUSIC_PAGE_TYPE_ALBUM" {
                if album.is_empty() {
                    album = part.to_string();
                }
                continue;
            }
            if let Some(d) = parse_mm_ss(part) {
                if duration == 0 {
                    duration = d;
                }
                continue;
            }
            let is_year = part.len() == 4 && part.chars().all(|c| c.is_ascii_digit());
            let low = part.to_lowercase();
            if is_year
                || low == "song"
                || low == "video"
                || low.ends_with("plays")
                || low.ends_with("views")
            {
                continue;
            }
            if artist.is_empty() {
                artist = part.to_string();
            } else if album.is_empty() && page_type.is_empty() {
                album = part.to_string();
            }
        }
    }

    let cover_url = best_thumbnail_url(item.pointer("/thumbnail/thumbnails"), &video_id);
    Some(YtmCandidate {
        video_id,
        title,
        artist,
        album,
        duration,
        cover_url,
    })
}

fn extract_candidate(item: &Value) -> Option<YtmCandidate> {
    let video_id = item
        .pointer("/playlistItemData/videoId")
        .or_else(|| {
            item.pointer("/overlay/musicItemThumbnailOverlayRenderer/content/musicPlayButtonRenderer/playNavigationEndpoint/watchEndpoint/videoId")
        })
        .and_then(|v| v.as_str())
        .or_else(|| find_watch_video_id(item))?
        .trim()
        .to_string();
    if video_id.is_empty() {
        return None;
    }

    let flex = item.get("flexColumns").and_then(|f| f.as_array())?;
    let title = flex
        .first()
        .and_then(|c| c.pointer("/musicResponsiveListItemFlexColumnRenderer/text/runs"))
        .and_then(join_runs_text)
        .unwrap_or_default();
    if title.is_empty() {
        return None;
    }

    let subtitle_runs = flex
        .get(1)
        .and_then(|c| c.pointer("/musicResponsiveListItemFlexColumnRenderer/text/runs"))
        .and_then(|r| r.as_array());

    let mut artist = String::new();
    let mut album = String::new();
    let mut duration = 0i32;

    if let Some(runs) = subtitle_runs {
        let parts: Vec<String> = runs
            .iter()
            .filter_map(|r| r.get("text").and_then(|t| t.as_str()))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && s != "•" && s != "·" && s != "&" && s != ",")
            .collect();

        for part in parts {
            let low = part.to_lowercase();
            if let Some(d) = parse_mm_ss(&part) {
                duration = d;
                continue;
            }
            if low == "song"
                || low == "video"
                || low == "single"
                || low == "ep"
                || low.ends_with("plays")
                || low.ends_with("views")
            {
                continue;
            }
            if artist.is_empty() {
                artist = part;
            } else if album.is_empty() {
                album = part;
            }
        }
    }

    let cover_url = best_thumbnail_url(
        item.pointer("/thumbnail/musicThumbnailRenderer/thumbnail/thumbnails"),
        &video_id,
    );

    Some(YtmCandidate {
        video_id,
        title,
        artist,
        album,
        duration,
        cover_url,
    })
}

fn find_watch_video_id(v: &Value) -> Option<&str> {
    match v {
        Value::Object(map) => {
            if let Some(vid) = map.get("watchEndpoint").and_then(|w| w.get("videoId")).and_then(|s| s.as_str()) {
                return Some(vid);
            }
            for child in map.values() {
                if let Some(found) = find_watch_video_id(child) {
                    return Some(found);
                }
            }
            None
        }
        Value::Array(arr) => {
            for child in arr {
                if let Some(found) = find_watch_video_id(child) {
                    return Some(found);
                }
            }
            None
        }
        _ => None,
    }
}

fn join_runs_text(v: &Value) -> Option<String> {
    let arr = v.as_array()?;
    let joined: String = arr
        .iter()
        .filter_map(|r| r.get("text").and_then(|t| t.as_str()))
        .collect::<Vec<_>>()
        .join("")
        .trim()
        .to_string();
    (!joined.is_empty()).then_some(joined)
}

fn parse_mm_ss(s: &str) -> Option<i32> {
    let parts: Vec<&str> = s.trim().split(':').collect();
    if parts.len() == 2 {
        let m = parts[0].parse::<i32>().ok()?;
        let sec = parts[1].parse::<i32>().ok()?;
        if (0..60).contains(&sec) && m >= 0 {
            return Some(m * 60 + sec);
        }
    } else if parts.len() == 3 {
        let h = parts[0].parse::<i32>().ok()?;
        let m = parts[1].parse::<i32>().ok()?;
        let sec = parts[2].parse::<i32>().ok()?;
        if (0..60).contains(&m) && (0..60).contains(&sec) && h >= 0 {
            return Some(h * 3600 + m * 60 + sec);
        }
    }
    None
}

fn score_ytm_candidate(
    c: &YtmCandidate,
    target_artist: &str,
    target_title: &str,
    target_duration: i32,
) -> i32 {
    let mut score = 100;
    let cand_title_core = core_song_title(&c.title);
    let want_title_core = core_song_title(target_title);
    let cand_title_low = c.title.to_lowercase();
    let cand_artist_slug = slug(&c.artist);
    let want_artist_slug = slug(target_artist);

    if cand_title_core == want_title_core && !want_title_core.is_empty() {
        score += 180;
    } else if !want_title_core.is_empty() && cand_title_core.contains(&want_title_core) {
        score += 90;
    }

    if !want_artist_slug.is_empty() && cand_artist_slug.contains(&want_artist_slug) {
        score += 120;
    } else if !want_artist_slug.is_empty() && cand_title_low.contains(&target_artist.to_lowercase()) {
        score += 60;
    }

    if target_duration > 0 && c.duration > 0 {
        let diff = (c.duration - target_duration).abs();
        if diff <= 6 {
            score += 120;
        } else if diff <= 15 {
            score += 65;
        } else if diff > 60 {
            score -= 100;
        }
    }

    let want_low = target_title.to_lowercase();
    for bad in [
        "karaoke",
        "instrumental",
        "cover",
        "slowed",
        "reverb",
        "nightcore",
        "8d audio",
        "reaction",
        "tutorial",
        "live",
    ] {
        if cand_title_low.contains(bad) && !want_low.contains(bad) {
            score -= 140;
        }
    }
    score
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn innertube_parser_extracts_song_fields() {
        let payload = serde_json::json!({
            "contents": {
                "tabbedSearchResultsRenderer": {
                    "tabs": [{
                        "tabRenderer": {
                            "content": {
                                "sectionListRenderer": {
                                    "contents": [{
                                        "musicShelfRenderer": {
                                            "contents": [{
                                                "musicResponsiveListItemRenderer": {
                                                    "playlistItemData": { "videoId": "FGBhQbmPwH8" },
                                                    "flexColumns": [
                                                        {
                                                            "musicResponsiveListItemFlexColumnRenderer": {
                                                                "text": { "runs": [{ "text": "One More Time" }] }
                                                            }
                                                        },
                                                        {
                                                            "musicResponsiveListItemFlexColumnRenderer": {
                                                                "text": {
                                                                    "runs": [
                                                                        { "text": "Daft Punk" },
                                                                        { "text": " • " },
                                                                        { "text": "Discovery" },
                                                                        { "text": " • " },
                                                                        { "text": "5:20" }
                                                                    ]
                                                                }
                                                            }
                                                        }
                                                    ]
                                                }
                                            }]
                                        }
                                    }]
                                }
                            }
                        }
                    }]
                }
            }
        });
        let items = parse_innertube_results(&payload);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].video_id, "FGBhQbmPwH8");
        assert_eq!(items[0].title, "One More Time");
        assert_eq!(items[0].artist, "Daft Punk");
        assert_eq!(items[0].album, "Discovery");
        assert_eq!(items[0].duration, 320);
    }

    #[test]
    fn scorer_prefers_studio_over_karaoke_and_matches_duration() {
        let studio = YtmCandidate {
            video_id: "good".into(),
            title: "One More Time".into(),
            artist: "Daft Punk".into(),
            album: "Discovery".into(),
            duration: 320,
            cover_url: String::new(),
        };
        let bad = YtmCandidate {
            video_id: "bad".into(),
            title: "One More Time (Karaoke Slowed + Reverb)".into(),
            artist: "Random Cover".into(),
            album: String::new(),
            duration: 210,
            cover_url: String::new(),
        };
        assert!(
            score_ytm_candidate(&studio, "Daft Punk", "One More Time", 320)
                > score_ytm_candidate(&bad, "Daft Punk", "One More Time", 320)
        );
    }

    #[test]
    fn radio_panel_parser_extracts_tracks_skips_seed_and_upgrades_thumbnails() {
        let payload = serde_json::json!({
            "contents": {
                "singleColumnMusicWatchNextResultsRenderer": {
                    "tabbedRenderer": {
                        "watchNextTabbedResultsRenderer": {
                            "tabs": [{
                                "tabRenderer": {
                                    "content": {
                                        "musicQueueRenderer": {
                                            "content": {
                                                "playlistPanelRenderer": {
                                                    "contents": [
                                                        {
                                                            "playlistPanelVideoRenderer": {
                                                                "videoId": "R-maTg10QKs",
                                                                "title": { "runs": [{ "text": "ソラゴト - Fake" }] },
                                                                "longBylineText": { "runs": [{ "text": "ASU" }, { "text": " • " }, { "text": "2023" }] },
                                                                "lengthText": { "runs": [{ "text": "3:15" }] }
                                                            }
                                                        },
                                                        {
                                                            "playlistPanelVideoRenderer": {
                                                                "videoId": "BRPQDDAUeMY",
                                                                "title": { "runs": [{ "text": "愛が灯る - The Flame of Love" }] },
                                                                "longBylineText": {
                                                                    "runs": [
                                                                        {
                                                                            "text": "Rokudenashi",
                                                                            "navigationEndpoint": {
                                                                                "browseEndpoint": {
                                                                                    "browseEndpointContextSupportedConfigs": {
                                                                                        "browseEndpointContextMusicConfig": { "pageType": "MUSIC_PAGE_TYPE_ARTIST" }
                                                                                    }
                                                                                }
                                                                            }
                                                                        },
                                                                        { "text": " • " },
                                                                        {
                                                                            "text": "The Flame of Love",
                                                                            "navigationEndpoint": {
                                                                                "browseEndpoint": {
                                                                                    "browseEndpointContextSupportedConfigs": {
                                                                                        "browseEndpointContextMusicConfig": { "pageType": "MUSIC_PAGE_TYPE_ALBUM" }
                                                                                    }
                                                                                }
                                                                            }
                                                                        },
                                                                        { "text": " • " },
                                                                        { "text": "2023" }
                                                                    ]
                                                                },
                                                                "thumbnail": {
                                                                    "thumbnails": [
                                                                        { "url": "https://yt3.googleusercontent.com/abc=w120-h120-l90-rj" }
                                                                    ]
                                                                },
                                                                "lengthText": { "runs": [{ "text": "3:22" }] }
                                                            }
                                                        }
                                                    ]
                                                }
                                            }
                                        }
                                    }
                                }
                            }]
                        }
                    }
                }
            }
        });
        let items = parse_innertube_radio_results(&payload, "R-maTg10QKs");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].video_id, "BRPQDDAUeMY");
        assert_eq!(items[0].artist, "Rokudenashi");
        assert_eq!(items[0].album, "The Flame of Love");
        assert_eq!(items[0].duration, 202);
        assert_eq!(
            items[0].cover_url,
            "https://yt3.googleusercontent.com/abc=w1080-h1080-l90-rj"
        );
    }
}
