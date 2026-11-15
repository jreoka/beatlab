use crate::models::Track;
use crate::sources::MusicSource;
use crate::taste::classify_moods_str;
use async_trait::async_trait;
use regex::Regex;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::process::Command;
use tracing::warn;

#[derive(Clone, Copy)]
pub enum YtDlpMode {
    SoundCloud,
    YouTube,
}

pub struct YtDlpSource {
    bin: String,
    mode: YtDlpMode,
    cache: Arc<Mutex<HashMap<String, (String, Instant)>>>,
}

impl YtDlpSource {
    pub fn find_binary(override_path: Option<&str>) -> Option<String> {
        if let Some(p) = override_path {
            if !p.trim().is_empty() {
                return Some(p.trim().to_string());
            }
        }
        for candidate in ["yt-dlp", "yt-dlp.exe"] {
            if std::process::Command::new(candidate)
                .arg("--version")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
            {
                return Some(candidate.to_string());
            }
        }
        None
    }

    pub fn new(bin: Option<String>, mode: YtDlpMode) -> Self {
        Self {
            bin: bin.unwrap_or_default(),
            mode,
            cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

#[async_trait]
impl MusicSource for YtDlpSource {
    fn name(&self) -> &'static str {
        match self.mode {
            YtDlpMode::SoundCloud => "soundcloud",
            YtDlpMode::YouTube => "youtube",
        }
    }

    fn available(&self) -> bool {
        !self.bin.is_empty()
    }

    fn status(&self) -> String {
        if self.bin.is_empty() {
            "yt-dlp not found on PATH".to_string()
        } else {
            match self.mode {
                YtDlpMode::SoundCloud => format!("soundcloud via {} (music-first)", self.bin),
                YtDlpMode::YouTube => format!("youtube fallback via {} (bestaudio)", self.bin),
            }
        }
    }

    async fn search(&self, query: &str, limit: usize) -> Vec<Track> {
        if self.bin.is_empty() {
            return vec![];
        }
        let n = limit.clamp(1, 25);
        let search_arg = match self.mode {
            YtDlpMode::SoundCloud => format!("scsearch{}:{}", n, query),
            YtDlpMode::YouTube => format!("ytsearch{}:{}", n, query),
        };

        let out = match tokio::time::timeout(
            Duration::from_secs(20),
            Command::new(&self.bin)
                .args(["--flat-playlist", "-J", "--no-warnings", &search_arg])
                .output(),
        )
        .await
        {
            Ok(Ok(o)) if o.status.success() => o.stdout,
            Ok(Ok(o)) => {
                let err = String::from_utf8_lossy(&o.stderr);
                warn!(source = self.name(), err = %err.lines().next().unwrap_or(""), "yt-dlp search failed");
                return vec![];
            }
            _ => return vec![],
        };

        let parsed: YtSearchJson = match serde_json::from_slice(&out) {
            Ok(p) => p,
            Err(_) => return vec![],
        };

        let mut tracks = Vec::new();
        for e in parsed.entries {
            if e.id.is_empty() || e.duration > 1200.0 || (e.duration > 0.0 && e.duration < 45.0) {
                continue;
            }
            if e.title == "[Deleted video]" || e.title == "[Private video]" {
                continue;
            }
            let channel_raw = if !e.uploader.is_empty() {
                e.uploader
            } else {
                e.channel
            };
            let channel = crate::models::primary_artist(channel_raw.trim());
            let (artist, title) = parse_yt_title(&e.title, &channel);
            if artist.is_empty() || title.is_empty() || is_junk_title(&title) {
                continue;
            }
            // Treat YouTube/SoundCloud videos without album tags as singles so channel names
            // don't pollute the artist's "Appears on" discography.
            let album = title.clone();
            let page = if !e.webpage_url.is_empty() {
                e.webpage_url
            } else if matches!(self.mode, YtDlpMode::YouTube) {
                format!("https://www.youtube.com/watch?v={}", e.id)
            } else {
                e.url.clone()
            };
            if page.is_empty() {
                continue;
            }
            let cover = if !e.thumbnail.is_empty() {
                e.thumbnail
            } else {
                e.thumbnails
                    .into_iter()
                    .filter(|t| !t.url.is_empty())
                    .max_by_key(|t| t.width)
                    .map(|t| t.url)
                    .unwrap_or_default()
            };

            let (format, bitrate) = match self.mode {
                YtDlpMode::SoundCloud => ("mp3".to_string(), 256),
                YtDlpMode::YouTube => ("m4a".to_string(), 128),
            };
            let moods = classify_moods_str(&title, &album, "", "");

            tracks.push(Track {
                id: format!("{}:{}", self.name(), e.id),
                source: self.name().to_string(),
                title,
                artist,
                album,
                genre: String::new(),
                moods,
                year: 0,
                track_no: 0,
                duration: e.duration as i32,
                bitrate,
                sample_rate: 48000,
                bit_depth: 0,
                format,
                size: 0,
                stream_url: format!("ytdlp:{}", page),
                cover_url: cover,
                meta: page,
                play_count: 0,
                last_played: 0,
                created: chrono::Utc::now().timestamp(),
            });
        }
        tracks
    }

    async fn resolve_stream(&self, stream_ref: &str) -> Option<String> {
        if self.bin.is_empty() {
            return None;
        }
        let page = stream_ref.strip_prefix("ytdlp:")?;
        {
            let guard = self.cache.lock().unwrap();
            if let Some((u, at)) = guard.get(page) {
                if at.elapsed() < Duration::from_secs(25 * 60) {
                    return Some(u.clone());
                }
            }
        }
        let url = resolve_ytdlp_url(&self.bin, page).await?;
        let mut guard = self.cache.lock().unwrap();
        if guard.len() > 256 {
            guard.retain(|_, (_, at)| at.elapsed() < Duration::from_secs(25 * 60));
        }
        guard.insert(page.to_string(), (url.clone(), Instant::now()));
        Some(url)
    }
}

pub async fn resolve_ytdlp_url(bin: &str, page: &str) -> Option<String> {
    let out = tokio::time::timeout(
        Duration::from_secs(20),
        Command::new(bin)
            .args([
                "-g",
                "--no-playlist",
                "--user-agent",
                "Beatlab/0.2",
                "-f",
                "bestaudio[protocol!=m3u8][protocol!=m3u8_native][ext=m4a]/bestaudio[protocol!=m3u8][protocol!=m3u8_native][ext=mp3]/bestaudio[protocol!=m3u8][protocol!=m3u8_native]/bestaudio/best",
                "--no-warnings",
                page,
            ])
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    if !out.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    stdout
        .lines()
        .next()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn yt_junk_regex() -> &'static Regex {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)(\s*[\(\[][^\)\]]*(official|lyric|audio|video|mv|hd|4k|remaster|visualizer)[^\)\]]*[\)\]]|\s*[-–—]?\s*(official\s+)?music\s+video\s*[-–—]?)",
        )
        .unwrap()
    })
}

fn parse_yt_title(raw: &str, channel: &str) -> (String, String) {
    let norm = raw.replace(" – ", " - ").replace(" — ", " - ");
    let cleaned = yt_junk_regex().replace_all(norm.trim(), "");
    let mut title = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut artist = String::new();

    if let Some((a, t)) = title.split_once(" - ") {
        artist = crate::models::primary_artist(a);
        title = crate::models::strip_release_tags(t);
    } else if !channel.is_empty() {
        // Handle Japanese title formatting like `ReoNa『ANIMA』` when prefix matches cleaned channel
        let ch_low = channel.to_lowercase();
        for (open, close) in [('『', '』'), ('「', '」')] {
            if let Some(s) = title.find(open) {
                if let Some(e_rel) = title[s + open.len_utf8()..].find(close) {
                    let prefix = crate::models::primary_artist(&title[..s]);
                    let quoted = title[s + open.len_utf8()..s + open.len_utf8() + e_rel].trim();
                    let p_low = prefix.to_lowercase();
                    if !prefix.is_empty()
                        && !quoted.is_empty()
                        && (p_low == ch_low || ch_low.contains(&p_low) || p_low.contains(&ch_low))
                    {
                        artist = prefix;
                        title = quoted.to_string();
                        break;
                    }
                }
            }
        }
    }
    if artist.is_empty() {
        artist = crate::models::primary_artist(channel);
    }
    if title.is_empty() {
        title = raw.trim().to_string();
    }
    (artist, title)
}

fn is_junk_title(title: &str) -> bool {
    let low = title.to_lowercase();
    if low.contains("how to") {
        return true;
    }
    const BAD: &[&str] = &[
        "reaction", "reactions", "reacts", "tutorial", "lesson", "lessons", "podcast",
        "interview", "interviews", "unboxing", "review", "commentary", "explained",
    ];
    low.split(|c: char| !c.is_alphanumeric())
        .any(|w| BAD.contains(&w))
}

#[derive(Deserialize)]
struct YtSearchJson {
    #[serde(default)]
    entries: Vec<YtEntry>,
}

#[derive(Deserialize)]
struct YtEntry {
    #[serde(default)]
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    uploader: String,
    #[serde(default)]
    channel: String,
    #[serde(default)]
    duration: f64,
    #[serde(default)]
    thumbnail: String,
    #[serde(default)]
    thumbnails: Vec<YtThumb>,
    #[serde(default)]
    url: String,
    #[serde(default)]
    webpage_url: String,
}

#[derive(Deserialize)]
struct YtThumb {
    #[serde(default)]
    url: String,
    #[serde(default)]
    width: i32,
}

#[cfg(test)]
mod tests {
    use super::parse_yt_title;

    #[test]
    fn parse_yt_title_cleans_channel_and_japanese_brackets() {
        let ch = crate::models::primary_artist("ReoNa official YouTube channel");
        assert_eq!(ch, "ReoNa");
        let (a, t) = parse_yt_title("ReoNa『ANIMA』-Music Video-", &ch);
        assert_eq!(a, "ReoNa");
        assert_eq!(t, "ANIMA");
    }
}
