use crate::models::Track;
use crate::sources::MusicSource;
use crate::taste::classify_moods_str;
use async_trait::async_trait;
use serde::Deserialize;
use std::collections::HashMap;
use std::time::Duration;

#[derive(Clone)]
pub struct ArchiveSource {
    http: reqwest::Client,
}

impl ArchiveSource {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .user_agent("Beatlab/0.2 (Rust; high-quality-audio)")
            .build()
            .unwrap_or_default();
        Self { http }
    }

    async fn item_tracks(&self, identifier: &str) -> Vec<Track> {
        let url = format!("https://archive.org/metadata/{}", identifier);
        let resp = match self.http.get(&url).send().await {
            Ok(r) if r.status().is_success() => r,
            _ => return vec![],
        };
        let md: IaMetadata = match resp.json().await {
            Ok(m) => m,
            Err(_) => return vec![],
        };
        parse_item_tracks(identifier, &md)
    }
}

fn parse_item_tracks(identifier: &str, md: &IaMetadata) -> Vec<Track> {
    if is_junk_item(md) {
        return vec![];
    }

    let album = decode_ia_text(&md.metadata.title);
    let artist = extract_creator(&md.metadata.creator)
        .map(|s| decode_ia_text(&s))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Unknown Artist".to_string());
    let year = md
        .metadata
        .date
        .as_str()
        .get(..4)
        .and_then(|s| s.parse::<i32>().ok())
        .unwrap_or(0);
    let genre = canonical_genre(&md.metadata.subject);
    let cover_url = pick_cover(identifier, &md.files);

    // Group audio files by normalized stem so we pick FLAC/ALAC/WAV over MP3/OGG
    let mut best_by_stem: HashMap<String, (&IaFile, i32, &'static str)> = HashMap::new();
    for f in &md.files {
        let Some((prio, ext)) = audio_format_priority(&f.format, &f.name) else {
            continue;
        };
        let stem = file_stem(&f.name);
        match best_by_stem.get(&stem) {
            Some((_, existing_prio, _)) if *existing_prio >= prio => {}
            _ => {
                best_by_stem.insert(stem, (f, prio, ext));
            }
        }
    }

    let mut tracks = Vec::new();
    for (_, (f, _, ext)) in best_by_stem {
        let duration = parse_length(&f.length);
        if duration < 45 || duration > 1500 {
            // Skip unindexed 0s files, short clips < 45s, and full-concert single-file rips > 25 min
            continue;
        }
        let decoded_file_title = decode_ia_text(&f.title);
        let decoded_file_artist = decode_ia_text(&f.artist);
        let decoded_file_creator = decode_ia_text(&f.creator);
        let decoded_file_album = decode_ia_text(&f.album);

        let title = if !decoded_file_title.is_empty() {
            decoded_file_title.clone()
        } else {
            decode_ia_text(&file_stem_display(&f.name))
        };
        let track_artist = if !decoded_file_artist.is_empty() {
            decoded_file_artist.clone()
        } else if !decoded_file_creator.is_empty() {
            decoded_file_creator.clone()
        } else {
            artist.clone()
        };
        let track_album = if !decoded_file_album.is_empty() {
            decoded_file_album.clone()
        } else if !album.is_empty() {
            album.clone()
        } else {
            track_artist.clone()
        };

        let has_file_tags = !decoded_file_title.is_empty()
            || !decoded_file_artist.is_empty()
            || !decoded_file_creator.is_empty()
            || !decoded_file_album.is_empty();
        if title.is_empty()
            || (track_artist.eq_ignore_ascii_case("Unknown Artist") && !has_file_tags)
        {
            continue;
        }

        let size = f.size.trim().parse::<i64>().unwrap_or(0);
        let bitrate = if duration > 0 && size > 0 {
            ((size * 8) / (duration as i64 * 1000)) as i32
        } else if ext == "flac" || ext == "wav" || ext == "alac" {
            900
        } else {
            256
        };
        let bit_depth = if f.format.to_lowercase().contains("24bit") {
            24
        } else if ext == "flac" || ext == "wav" || ext == "alac" {
            16
        } else {
            0
        };
        let sample_rate = if ext == "flac" || ext == "wav" || ext == "alac" {
            44100
        } else {
            0
        };

        let encoded_name = urlencoding::encode(&f.name);
        let stream_url = format!("https://archive.org/download/{}/{}", identifier, encoded_name);
        let moods = classify_moods_str(&title, &track_album, &genre, "");

        tracks.push(Track {
            id: format!("archive:{}:{}", identifier, f.name),
            source: "archive".to_string(),
            title,
            artist: track_artist,
            album: track_album,
            genre: genre.clone(),
            moods,
            year,
            track_no: f.track.trim().parse::<i32>().unwrap_or(0),
            disc_no: 1,
            duration,
            bitrate,
            sample_rate,
            bit_depth,
            format: ext.to_string(),
            size,
            stream_url,
            cover_url: cover_url.clone(),
            meta: format!("https://archive.org/details/{}", identifier),
            deezer_id: String::new(),
            created: chrono::Utc::now().timestamp(),
        });
    }
    tracks.sort_by_key(|t| t.track_no);
    tracks
}

#[async_trait]
impl MusicSource for ArchiveSource {
    fn name(&self) -> &'static str {
        "archive"
    }

    fn available(&self) -> bool {
        true
    }

    fn status(&self) -> String {
        "archive.org — lossless-first (FLAC > ALAC > WAV > MP3), keyless".to_string()
    }

    async fn search(&self, query: &str, limit: usize) -> Vec<Track> {
        let trimmed = query.trim();
        if trimmed.is_empty() || (trimmed.is_ascii() && trimmed.len() < 3) {
            return vec![];
        }
        let q = format!(
            "({}) AND mediatype:audio AND -collection:librivoxaudio AND -collection:podcasts",
            trimmed.replace('"', "")
        );
        let url = format!(
            "https://archive.org/advancedsearch.php?q={}&fl[]=identifier,title,creator&rows=8&output=json",
            urlencoding::encode(&q)
        );
        let resp = match self.http.get(&url).send().await {
            Ok(r) if r.status().is_success() => r,
            _ => return vec![],
        };
        let parsed: IaSearchResponse = match resp.json().await {
            Ok(p) => p,
            Err(_) => return vec![],
        };

        let docs: Vec<_> = parsed.response.docs.into_iter().take(6).collect();
        let mut futs = Vec::new();
        for doc in &docs {
            futs.push(self.item_tracks(&doc.identifier));
        }
        let results = futures::future::join_all(futs).await;
        let mut out: Vec<Track> = results
            .into_iter()
            .flatten()
            .filter(|t| crate::store::search_match_score(t, query) > 0)
            .collect();
        out.sort_by_key(|a| {
            std::cmp::Reverse((
                crate::store::search_match_score(a, query),
                a.quality_score(),
            ))
        });
        out.truncate(limit);
        out
    }
}

fn audio_format_priority(format: &str, name: &str) -> Option<(i32, &'static str)> {
    let f = format.to_lowercase();
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    if f.contains("24bit flac") || (f == "flac" && ext == "flac") {
        if f.contains("24bit") {
            return Some((100, "flac"));
        }
        return Some((90, "flac"));
    }
    match (f.as_str(), ext.as_str()) {
        ("flac", _) | (_, "flac") => Some((90, "flac")),
        ("apple lossless", _) | (_, "alac") => Some((85, "alac")),
        ("wave", _) | ("wav", _) | (_, "wav") => Some((80, "wav")),
        ("vbr mp3", _) | ("mp3", _) | (_, "mp3") => Some((50, "mp3")),
        ("ogg vorbis", _) | (_, "ogg") => Some((45, "ogg")),
        ("aac", _) | (_, "m4a") => Some((45, "m4a")),
        _ => None,
    }
}

fn file_stem(name: &str) -> String {
    match name.rfind('.') {
        Some(i) => name[..i].to_lowercase(),
        None => name.to_lowercase(),
    }
}

fn file_stem_display(name: &str) -> String {
    let base = name.rsplit('/').next().unwrap_or(name);
    match base.rfind('.') {
        Some(i) => base[..i].to_string(),
        None => base.to_string(),
    }
}

fn parse_length(s: &str) -> i32 {
    let s = s.trim();
    if s.is_empty() {
        return 0;
    }
    if let Some((m, sec)) = s.split_once(':') {
        let mins = m.parse::<i32>().unwrap_or(0);
        let secs = sec.parse::<f64>().unwrap_or(0.0) as i32;
        return mins * 60 + secs;
    }
    s.parse::<f64>().unwrap_or(0.0) as i32
}

fn pick_cover(identifier: &str, files: &[IaFile]) -> String {
    let mut best: Option<(i32, i64, String)> = None;
    for f in files {
        let low = f.name.to_lowercase();
        if !(low.ends_with(".jpg") || low.ends_with(".jpeg") || low.ends_with(".png")) {
            continue;
        }
        if low.contains("spectrogram") || low.contains("waveform") || low.contains("__ia_thumb") {
            continue;
        }
        let mut score = 0;
        for kw in ["cover", "front", "folder", "album", "artwork"] {
            if low.contains(kw) {
                score += 10;
                break;
            }
        }
        let size = f.size.trim().parse::<i64>().unwrap_or(0);
        let url = format!(
            "https://archive.org/download/{}/{}",
            identifier,
            urlencoding::encode(&f.name)
        );
        match &best {
            Some((bs, bsz, _)) if *bs > score || (*bs == score && *bsz >= size) => {}
            _ => best = Some((score, size, url)),
        }
    }
    best.map(|b| b.2).unwrap_or_default()
}

fn is_junk_item(md: &IaMetadata) -> bool {
    let coll = serde_json::to_string(&md.metadata.collection)
        .unwrap_or_default()
        .to_lowercase();
    for bad in [
        "librivox",
        "podcasts",
        "audio_bookspoetry",
        "opensource_audio",
        "community_audio",
        "spokenword",
    ] {
        if coll.contains(bad) {
            return true;
        }
    }
    false
}

/// Decodes percent-encoded UTF-8 strings (including Archive.org titles where spaces were inserted
/// between `%` and hex digits, e.g. `% D 8% AA...`), while leaving normal `%` text untouched.
fn decode_ia_text(s: &str) -> String {
    let trimmed = s.trim();
    if !trimmed.contains('%') {
        return trimmed.to_string();
    }
    let step1 = decode_percent_runs(trimmed, false);
    if step1.contains('%') {
        decode_percent_runs(&step1, true)
    } else {
        step1
    }
}

fn decode_percent_runs(s: &str, allow_spaces: bool) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'%' {
            let mut cur = i;
            let mut decoded = Vec::new();
            let mut tokens = 0usize;

            while let Some((b, next)) = parse_percent_byte(bytes, cur, allow_spaces) {
                decoded.push(b);
                tokens += 1;
                cur = next;
                if allow_spaces {
                    let mut peek = cur;
                    while peek < bytes.len() && bytes[peek] == b' ' {
                        peek += 1;
                    }
                    if peek < bytes.len() && bytes[peek] == b'%' {
                        cur = peek;
                    } else {
                        break;
                    }
                }
            }

            let valid_utf8 = std::str::from_utf8(&decoded).ok();
            let no_ctrl = !decoded.iter().any(|&b| b < 0x20);
            let enough_signal = !allow_spaces || (tokens >= 2 && decoded.iter().any(|&b| b >= 0x80));
            if tokens > 0 && no_ctrl && enough_signal {
                if let Some(text) = valid_utf8 {
                    out.push_str(text);
                    i = cur;
                    continue;
                }
            }
        }

        let ch = s[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }

    out.trim().to_string()
}

fn parse_percent_byte(bytes: &[u8], pos: usize, allow_spaces: bool) -> Option<(u8, usize)> {
    if bytes.get(pos) != Some(&b'%') {
        return None;
    }
    let mut p = pos + 1;
    if allow_spaces {
        while bytes.get(p) == Some(&b' ') {
            p += 1;
        }
    }
    let h1 = hex_nibble(*bytes.get(p)?)?;
    p += 1;
    if allow_spaces {
        while bytes.get(p) == Some(&b' ') {
            p += 1;
        }
    }
    let h2 = hex_nibble(*bytes.get(p)?)?;
    Some(((h1 << 4) | h2, p + 1))
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn extract_creator(val: &serde_json::Value) -> Option<String> {
    match val {
        serde_json::Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        serde_json::Value::Array(arr) => arr
            .iter()
            .filter_map(|v| v.as_str())
            .map(|s| s.trim())
            .find(|s| !s.is_empty())
            .map(|s| s.to_string()),
        _ => None,
    }
}

fn canonical_genre(val: &serde_json::Value) -> String {
    let raw = match val {
        serde_json::Value::String(s) => s.to_lowercase(),
        serde_json::Value::Array(arr) => arr
            .iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>()
            .join(";")
            .to_lowercase(),
        _ => return String::new(),
    };
    let map = [
        ("jazz", "Jazz"),
        ("blues", "Blues"),
        ("rock", "Rock"),
        ("indie", "Indie"),
        ("pop", "Pop"),
        ("classical", "Classical"),
        ("electronic", "Electronic"),
        ("techno", "Electronic"),
        ("house", "Electronic"),
        ("ambient", "Ambient"),
        ("hip hop", "Hip-Hop"),
        ("hip-hop", "Hip-Hop"),
        ("rap", "Hip-Hop"),
        ("folk", "Folk"),
        ("country", "Country"),
        ("soul", "Soul"),
        ("funk", "Funk"),
        ("r&b", "R&B"),
        ("reggae", "Reggae"),
        ("metal", "Metal"),
        ("punk", "Punk"),
        ("latin", "Latin"),
        ("soundtrack", "Soundtrack"),
    ];
    for (k, v) in map {
        if raw.contains(k) {
            return v.to_string();
        }
    }
    String::new()
}

#[derive(Deserialize)]
struct IaSearchResponse {
    response: IaSearchInner,
}

#[derive(Deserialize)]
struct IaSearchInner {
    docs: Vec<IaSearchDoc>,
}

#[derive(Deserialize)]
struct IaSearchDoc {
    identifier: String,
}

#[derive(Deserialize)]
struct IaMetadata {
    metadata: IaMetaInner,
    #[serde(default)]
    files: Vec<IaFile>,
}

#[derive(Deserialize)]
struct IaMetaInner {
    #[serde(default)]
    title: String,
    #[serde(default)]
    creator: serde_json::Value,
    #[serde(default, deserialize_with = "de_flex_string")]
    date: String,
    #[serde(default)]
    subject: serde_json::Value,
    #[serde(default)]
    collection: serde_json::Value,
}

#[derive(Deserialize)]
struct IaFile {
    #[serde(default)]
    name: String,
    #[serde(default)]
    format: String,
    #[serde(default, deserialize_with = "de_flex_string")]
    length: String,
    #[serde(default, deserialize_with = "de_flex_string")]
    size: String,
    #[serde(default, deserialize_with = "de_flex_string")]
    track: String,
    #[serde(default, deserialize_with = "de_flex_string")]
    title: String,
    #[serde(default, deserialize_with = "de_flex_string")]
    artist: String,
    #[serde(default, deserialize_with = "de_flex_string")]
    album: String,
    #[serde(default, deserialize_with = "de_flex_string")]
    creator: String,
}

fn de_flex_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let val = serde_json::Value::deserialize(deserializer)?;
    Ok(match val {
        serde_json::Value::String(s) => s,
        serde_json::Value::Number(n) => n.to_string(),
        _ => String::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_file(name: &str, format: &str, length: &str) -> IaFile {
        IaFile {
            name: name.to_string(),
            format: format.to_string(),
            length: length.to_string(),
            size: "1234567".to_string(),
            track: "1".to_string(),
            title: String::new(),
            artist: String::new(),
            album: String::new(),
            creator: String::new(),
        }
    }

    #[test]
    fn decodes_percent_and_spaced_percent_utf8_without_corrupting_normal_text() {
        assert_eq!(
            decode_ia_text("%D8%AA%D8%B3%D8%AC%D9%8A%D9%84%D9%8A #48"),
            "تسجيلي #48"
        );
        assert_eq!(
            decode_ia_text("% D 8% AA% D 8% B 3% D 8% AC% D 9% 8 A% D 9% 84% D 9% 8 A% 20% 2348"),
            "تسجيلي #48"
        );
        assert_eq!(
            decode_ia_text("%E3%83%8F%E3%83%AC%E3%81%AE%E6%97%A5%E3%81%AB"),
            "ハレの日に"
        );
        assert_eq!(decode_ia_text("100% Pure & 50% 2024"), "100% Pure & 50% 2024");
    }

    #[test]
    fn rejects_zero_duration_and_unlabelled_unknown_artist_uploads() {
        let md = IaMetadata {
            metadata: IaMetaInner {
                title: "% D 8% AA% D 8% B 3% D 8% AC% D 9% 8 A% D 9% 84% D 9% 8 A% 20% 2348".into(),
                creator: serde_json::Value::Null,
                date: "2013-02-01".into(),
                subject: serde_json::Value::Null,
                collection: serde_json::json!(["audio"]),
            },
            files: vec![
                sample_file("%D8%AA%D8%B3%D8%AC%D9%8A%D9%84%D9%8A #48.wav", "WAVE", ""),
                sample_file("recording_with_length.wav", "WAVE", "120"),
            ],
        };
        assert!(parse_item_tracks("D8AAD8B3D8ACD98AD984D98A202348_201302", &md).is_empty());
    }

    #[test]
    fn accepts_valid_music_and_decodes_percent_encoded_metadata() {
        let mut f = sample_file("%E3%83%8F%E3%83%AC%E3%81%AE%E6%97%A5%E3%81%AB.flac", "Flac", "245");
        f.title = "%E3%83%8F%E3%83%AC%E3%81%AE%E6%97%A5%E3%81%AB".into();
        let md = IaMetadata {
            metadata: IaMetaInner {
                title: "Live%20Session".into(),
                creator: serde_json::json!("Reira%20Ushio"),
                date: "2025-01-01".into(),
                subject: serde_json::json!("indie"),
                collection: serde_json::json!(["netlabels"]),
            },
            files: vec![f],
        };
        let tracks = parse_item_tracks("valid-music-item", &md);
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].title, "ハレの日に");
        assert_eq!(tracks[0].album, "Live Session");
        assert_eq!(tracks[0].artist, "Reira Ushio");
        assert_eq!(tracks[0].duration, 245);
    }

    #[test]
    fn rejects_community_audio_and_spokenword_collections() {
        let mut f = sample_file("track.flac", "Flac", "180");
        f.title = "Track".into();
        let md = IaMetadata {
            metadata: IaMetaInner {
                title: "Album".into(),
                creator: serde_json::json!("Someone"),
                date: "2020".into(),
                subject: serde_json::Value::Null,
                collection: serde_json::json!(["community_audio"]),
            },
            files: vec![f],
        };
        assert!(parse_item_tracks("comm-1", &md).is_empty());
    }
}
