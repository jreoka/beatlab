use crate::models::Track;
use crate::sources::MusicSource;
use crate::taste::classify_moods_str;
use async_trait::async_trait;
use regex::Regex;
use sha1::{Digest, Sha1};
use std::sync::OnceLock;
use std::time::Duration;

/// BandcampSource queries Bandcamp's public search page for music-only tracks
/// with clean metadata, cover art, and genre tags. When `yt-dlp` is available,
/// Beatlab resolves the Bandcamp track page URL to its highest-quality audio stream.
pub struct BandcampSource {
    http: reqwest::Client,
    ytdlp_bin: String,
}

impl BandcampSource {
    pub fn new(ytdlp_bin: Option<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(12))
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
            .build()
            .unwrap_or_default();
        Self {
            http,
            ytdlp_bin: ytdlp_bin.unwrap_or_default(),
        }
    }
}

#[async_trait]
impl MusicSource for BandcampSource {
    fn name(&self) -> &'static str {
        "bandcamp"
    }

    fn available(&self) -> bool {
        !self.ytdlp_bin.is_empty()
    }

    fn status(&self) -> String {
        if !self.ytdlp_bin.is_empty() {
            "bandcamp — music-only discovery + yt-dlp stream resolve".to_string()
        } else {
            "bandcamp disabled (requires yt-dlp for stream resolution)".to_string()
        }
    }

    async fn search(&self, query: &str, limit: usize) -> Vec<Track> {
        if self.ytdlp_bin.is_empty() {
            return vec![];
        }
        let url = format!(
            "https://bandcamp.com/search?q={}&item_type=t",
            urlencoding::encode(query)
        );
        let html = match self.http.get(&url).send().await {
            Ok(r) if r.status().is_success() => match r.text().await {
                Ok(t) => t,
                Err(_) => return vec![],
            },
            _ => return vec![],
        };

        parse_bandcamp_search(&html, limit)
    }

    async fn resolve_stream(&self, stream_ref: &str) -> Option<String> {
        if self.ytdlp_bin.is_empty() {
            return None;
        }
        let page = stream_ref.strip_prefix("ytdlp:")?;
        crate::sources::ytdlp::resolve_ytdlp_url(&self.ytdlp_bin, page).await
    }
}

struct BcRegexes {
    item: Regex,
    title: Regex,
    sub: Regex,
    art: Regex,
    tags: Regex,
    tag_strip: Regex,
}

fn bc_regexes() -> &'static BcRegexes {
    static RE: OnceLock<BcRegexes> = OnceLock::new();
    RE.get_or_init(|| BcRegexes {
        item: Regex::new(r#"(?s)<li class="searchresult data-search"[^>]*>(.*?)</li>"#).unwrap(),
        title: Regex::new(r#"(?s)<div class="heading">\s*<a href="([^"]+)"[^>]*>(.*?)</a>"#)
            .unwrap(),
        sub: Regex::new(r#"(?s)<div class="subhead">(.*?)</div>"#).unwrap(),
        art: Regex::new(r#"(?s)<div class="art">\s*<img src="([^"]+)""#).unwrap(),
        tags: Regex::new(r#"(?s)tags:\s*(.*?)</div>"#).unwrap(),
        tag_strip: Regex::new(r#"<[^>]+>"#).unwrap(),
    })
}

fn parse_bandcamp_search(html: &str, limit: usize) -> Vec<Track> {
    let re = bc_regexes();
    let re_item = &re.item;
    let re_title = &re.title;
    let re_sub = &re.sub;
    let re_art = &re.art;
    let re_tags = &re.tags;
    let re_tag_strip = &re.tag_strip;

    let mut tracks = Vec::new();
    for cap in re_item.captures_iter(html) {
        if tracks.len() >= limit {
            break;
        }
        let block = &cap[1];
        let Some(tcap) = re_title.captures(block) else {
            continue;
        };
        let raw_url = tcap[1].split('?').next().unwrap_or(&tcap[1]).trim().to_string();
        let title = html_unescape(re_tag_strip.replace_all(&tcap[2], "").trim());
        if title.is_empty() || raw_url.is_empty() {
            continue;
        }

        let sub = re_sub
            .captures(block)
            .map(|c| html_unescape(re_tag_strip.replace_all(&c[1], "").trim()))
            .unwrap_or_default();

        // Subhead is typically "from <Album> by <Artist>" or "by <Artist>"
        let (album, artist) = parse_bandcamp_subhead(&sub);
        if artist.is_empty() {
            continue;
        }

        let cover_url = re_art
            .captures(block)
            .map(|c| c[1].trim().to_string())
            .unwrap_or_default();

        let genre = re_tags
            .captures(block)
            .map(|c| {
                re_tag_strip
                    .replace_all(&c[1], "")
                    .split(',')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_string()
            })
            .unwrap_or_default();

        let mut hasher = Sha1::new();
        hasher.update(raw_url.as_bytes());
        let id = format!("bandcamp:{}", hex::encode(&hasher.finalize()[..10]));
        let moods = classify_moods_str(&title, &album, &genre, "");

        tracks.push(Track {
            id,
            source: "bandcamp".to_string(),
            title,
            artist,
            album,
            genre,
            moods,
            year: 0,
            track_no: 0,
            duration: 0,
            bitrate: 256,
            sample_rate: 44100,
            bit_depth: 0,
            format: "mp3".to_string(),
            size: 0,
            stream_url: format!("ytdlp:{}", raw_url),
            cover_url,
            meta: raw_url,
            play_count: 0,
            last_played: 0,
            created: chrono::Utc::now().timestamp(),
        });
    }
    tracks
}

fn parse_bandcamp_subhead(sub: &str) -> (String, String) {
    let cleaned = sub.split_whitespace().collect::<Vec<_>>().join(" ");
    if let Some(rest) = cleaned.strip_prefix("from ") {
        if let Some((alb, art)) = rest.rsplit_once(" by ") {
            return (alb.trim().to_string(), art.trim().to_string());
        }
    }
    if let Some((_, art)) = cleaned.rsplit_once("by ") {
        return (String::new(), art.trim().to_string());
    }
    (String::new(), String::new())
}

fn html_unescape(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}
