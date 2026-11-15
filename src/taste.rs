use crate::models::{
    primary_artist, slug, AlbumSummary, ArtistSummary, Mood, Track, UserTrackStats,
};
use crate::neural::{event_target, extract_features_ctx, FeatureContext, NeuralCache, UserNet};
use crate::sources::{dedup_tracks_prefer_quality, SourceRegistry};
use crate::store::{search_match_score, Store};
use chrono::Utc;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use tracing::info;

pub fn split_genres(raw: &str) -> Vec<String> {
    raw.split([',', ';', '/'])
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

pub fn classify_moods_str(title: &str, album: &str, genre: &str, tags: &str) -> String {
    classify_moods(title, album, genre, tags)
        .into_iter()
        .map(|m| m.slug())
        .collect::<Vec<_>>()
        .join(",")
}

pub fn classify_moods(title: &str, album: &str, genre: &str, tags: &str) -> Vec<Mood> {
    let hay = format!("{} {} {} {}", title, album, genre, tags).to_lowercase();
    let mut moods = Vec::new();
    let rules: &[(Mood, &[&str])] = &[
        (
            Mood::Chill,
            &["chill", "lofi", "lo-fi", "downtempo", "dream", "soft", "relax", "bossa", "sunset"],
        ),
        (
            Mood::Focus,
            &["ambient", "instrumental", "study", "piano", "classical", "minimal", "score", "drone"],
        ),
        (
            Mood::Energy,
            &["rock", "metal", "punk", "drum", "bass", "hard", "workout", "trap", "phonk", "hype"],
        ),
        (
            Mood::LateNight,
            &["night", "midnight", "dark", "noir", "moody", "synthwave", "after", "moon"],
        ),
        (
            Mood::Upbeat,
            &["pop", "happy", "sun", "summer", "bright", "dance", "feel", "good", "anthem"],
        ),
        (
            Mood::Melancholy,
            &["sad", "blue", "rain", "acoustic", "lonely", "heartbreak", "ballad", "cry"],
        ),
        (
            Mood::Party,
            &["dance", "disco", "house", "club", "techno", "edm", "electro", "party", "rave"],
        ),
        (
            Mood::Acoustic,
            &["acoustic", "folk", "unplugged", "guitar", "singer", "country", "indie folk"],
        ),
        (
            Mood::Groove,
            &["funk", "soul", "jazz", "r&b", "rnb", "hip-hop", "hip hop", "groove", "neosoul"],
        ),
        (
            Mood::RockIndie,
            &["indie", "alt", "alternative", "shoegaze", "grunge", "britpop", "post-punk"],
        ),
    ];

    for (mood, keys) in rules {
        if keys.iter().any(|k| hay.contains(k)) {
            moods.push(*mood);
        }
    }
    if moods.is_empty() {
        if hay.contains("electronic") {
            moods.push(Mood::Party);
            moods.push(Mood::LateNight);
        } else {
            moods.push(Mood::Upbeat);
            moods.push(Mood::Chill);
        }
    }
    moods.truncate(3);
    moods
}

#[derive(Debug, Default, Clone)]
pub struct TasteProfile {
    pub artist_affinity: HashMap<String, f64>,
    pub genre_affinity: HashMap<String, f64>,
    pub mood_affinity: HashMap<Mood, f64>,
    pub playlist_artist_affinity: HashMap<String, f64>,
    pub playlist_genre_affinity: HashMap<String, f64>,
    pub playlist_mood_affinity: HashMap<Mood, f64>,
    pub playlist_track_counts: HashMap<String, usize>,
    pub manual_playlist_ids: HashSet<String>,
    pub manual_playlist_song_keys: HashSet<String>,
    pub manual_playlist_names: Vec<String>,
    pub followed_artists: HashSet<String>,
    pub saved_albums: HashSet<String>,
    pub starred: HashSet<String>,
    pub ratings: HashMap<String, i32>,
}

struct ScoringContext {
    prof: TasteProfile,
    stats: HashMap<String, UserTrackStats>,
    net: UserNet,
}

#[derive(Default)]
struct PlaylistCentroid {
    artists: HashMap<String, f64>,
    genres: HashMap<String, f64>,
    moods: HashMap<Mood, f64>,
    keywords: Vec<String>,
    mean_year: Option<f64>,
    mean_duration: Option<f64>,
    existing_ids: HashSet<String>,
    existing_keys: HashSet<String>,
}

#[derive(Clone, Copy)]
enum ScoreMode {
    Supermix,
    Discover,
    PlaylistDna,
}

#[derive(Debug, Clone)]
pub struct SearchMixResult {
    pub tracks: Vec<Track>,
    pub instant_mix: Option<InstantMix>,
    pub refinements: SearchRefinements,
    pub taste_match_ids: HashSet<String>,
    #[cfg_attr(not(test), allow(dead_code))]
    pub track_neural_scores: HashMap<String, f64>,
}

#[derive(Debug, Clone)]
pub struct InstantMix {
    pub title: String,
    pub subtitle: String,
    pub query: String,
    pub is_theme_mix: bool,
    pub tracks: Vec<Track>,
    pub total_duration: i32,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchRefinements {
    pub detected_intent: String,
    pub filter_chips: Vec<RefinementChip>,
    pub query_suggestions: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefinementChip {
    pub key: String,
    pub label: String,
    pub kind: String,
    pub count: usize,
}

#[derive(Debug, Clone)]
pub struct ThemeSpec {
    pub key: &'static str,
    pub label: &'static str,
    pub keywords: &'static [&'static str],
    pub search_boost: &'static str,
}

#[derive(Debug, Clone, Default)]
pub struct SearchIntent {
    pub raw: String,
    pub normalized: String,
    pub tokens: Vec<String>,
    pub decade_label: Option<&'static str>,
    pub year_range: Option<(i32, i32)>,
    pub wants_hits: bool,
    pub wants_classic: bool,
    pub moods: Vec<Mood>,
    pub genres: Vec<String>,
    pub themes: Vec<ThemeSpec>,
    pub is_mix_query: bool,
    pub is_theme_mix: bool,
}

impl SearchIntent {
    pub fn playlist_search_queries(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(dec) = self.decade_label {
            if self.genres.is_empty() && self.themes.is_empty() && self.moods.is_empty() {
                out.push(format!("{} hits", dec));
                out.push(dec.to_string());
            } else {
                out.push(self.normalized.clone());
                out.push(format!("{} hits", self.normalized));
            }
        } else if !self.themes.is_empty() {
            out.push(self.normalized.clone());
            let boost = self.themes[0].search_boost;
            if boost != self.normalized {
                out.push(boost.to_string());
            }
        } else if self.is_mix_query {
            out.push(self.normalized.clone());
            if !self.normalized.contains("hits") && !self.normalized.contains("mix") {
                out.push(format!("{} mix", self.normalized));
            }
        }
        out.truncate(2);
        out
    }

    pub fn expanded_track_queries(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(dec) = self.decade_label {
            if self.genres.is_empty() && self.themes.is_empty() {
                out.push(format!("{} greatest hits", dec));
            } else {
                out.push(format!("{} hits", self.normalized));
            }
        } else if !self.themes.is_empty() {
            out.push(format!("{} hits", self.themes[0].search_boost));
        }
        out.retain(|q| q != &self.normalized);
        out.truncate(1);
        out
    }
}

#[derive(Clone)]
pub struct TasteEngine {
    pub store: Store,
    pub sources: SourceRegistry,
    pub neural: std::sync::Arc<NeuralCache>,
}

impl TasteEngine {
    pub fn new(store: Store, sources: SourceRegistry) -> Self {
        Self {
            store,
            sources,
            neural: std::sync::Arc::new(NeuralCache::new()),
        }
    }

    pub fn mark_taste_dirty(&self, user_id: i64) {
        if user_id > 0 {
            self.store.kv_set(&format!("taste:dirty:{}", user_id), "1");
        }
    }

    /// Rebuilds auto-playlists if missing, marked dirty by recent user feedback/playlist edits,
    /// or older than `max_age_secs`.
    pub fn refresh_auto_playlists_if_stale(&self, user_id: i64, max_age_secs: i64) {
        if user_id <= 0 {
            return;
        }
        let dirty = self
            .store
            .kv_get(&format!("taste:dirty:{}", user_id))
            .map(|v| v == "1")
            .unwrap_or(false);
        let last_rebuilt = self
            .store
            .kv_get(&format!("taste:rebuilt_at:{}", user_id))
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(0);
        let now = Utc::now().timestamp();
        let has_auto = self
            .store
            .list_playlists(user_id, false)
            .map(|pls| pls.iter().any(|p| p.is_auto))
            .unwrap_or(false);

        if !has_auto || dirty || (now - last_rebuilt) >= max_age_secs {
            self.rebuild_auto_playlists(user_id);
        }
    }

    pub fn record_event(&self, user_id: i64, track_id: &str, kind: &str, value: f64) {
        self.record_event_with_query(user_id, track_id, kind, value, None);
    }

    pub fn record_event_with_query(
        &self,
        user_id: i64,
        track_id: &str,
        kind: &str,
        value: f64,
        query: Option<&str>,
    ) {
        let safe_val = if value.is_finite() { value } else { 0.0 };
        let _ = self.store.record_event(user_id, track_id, kind, safe_val);
        self.learn_signal_with_query(user_id, track_id, kind, safe_val, query);
    }

    /// Updates neural model and taste staleness for an already-persisted interaction event
    /// (avoids writing duplicate rows into `events` after `star_track`/`set_rating`).
    pub fn learn_signal(&self, user_id: i64, track_id: &str, kind: &str, value: f64) {
        self.learn_signal_with_query(user_id, track_id, kind, value, None);
    }

    pub fn learn_signal_with_query(
        &self,
        user_id: i64,
        track_id: &str,
        kind: &str,
        value: f64,
        query: Option<&str>,
    ) {
        let safe_val = if value.is_finite() { value } else { 0.0 };
        if user_id > 0
            && matches!(
                kind,
                "star"
                    | "unstar"
                    | "rate"
                    | "playlist_add"
                    | "playlist_remove"
                    | "complete"
                    | "skip"
                    | "search_pick"
                    | "mix_play"
            )
        {
            self.mark_taste_dirty(user_id);
        }
        if let Some((target, weight)) = event_target(kind, safe_val) {
            if let Ok(Some(track)) = self.store.get_track(track_id) {
                let prof = self.user_profile(user_id);
                let stats = if user_id > 0 {
                    self.store.track_stats_map(user_id).unwrap_or_default()
                } else {
                    HashMap::new()
                };
                let st = stats.get(track_id).copied().unwrap_or_default();
                let intent = query
                    .map(str::trim)
                    .filter(|q| !q.is_empty())
                    .map(parse_search_intent);
                let fctx = build_feature_context_with_query(&track, &prof, &st, intent.as_ref(), false);
                let feats = extract_features_ctx(&track, &fctx);

                // Ensure upgraded or cold-loaded nets bootstrap from existing history before online updates.
                if user_id > 0 && self.neural.get_or_load(&self.store, user_id).updates == 0 {
                    let bootstrap = self.collect_bootstrap_samples(user_id, &prof, &stats);
                    self.neural.update_and_save(&self.store, user_id, |net| {
                        if net.updates == 0 && !bootstrap.is_empty() {
                            net.train_batch(&bootstrap, 3);
                        }
                        net.train_step(&feats, target, weight);
                    });
                } else {
                    self.neural.update_and_save(&self.store, user_id, |net| {
                        net.train_step(&feats, target, weight);
                    });
                }
            }
        }
    }

    pub fn record_playlist_additions(&self, user_id: i64, track_ids: &[String]) {
        if user_id <= 0 || track_ids.is_empty() {
            return;
        }
        for tid in track_ids {
            self.record_event(user_id, tid, "playlist_add", 1.0);
        }
    }

    pub fn record_playlist_removals(&self, user_id: i64, track_ids: &[String]) {
        if user_id <= 0 || track_ids.is_empty() {
            return;
        }
        for tid in track_ids {
            self.record_event(user_id, tid, "playlist_remove", 1.0);
        }
    }

    pub fn record_album_save(&self, user_id: i64, album_id: &str, saved: bool) {
        if user_id <= 0 {
            return;
        }
        self.mark_taste_dirty(user_id);
        let tracks = self.store.album_tracks(album_id).unwrap_or_default();
        if tracks.is_empty() {
            return;
        }
        let prof = self.user_profile(user_id);
        let stats = self.store.track_stats_map(user_id).unwrap_or_default();
        let target = if saved { 0.90 } else { 0.25 };
        let weight = if saved { 1.1 } else { 0.7 };
        self.neural.update_and_save(&self.store, user_id, |net| {
            for t in tracks.iter().take(4) {
                let st = stats.get(&t.id).copied().unwrap_or_default();
                let fctx = build_feature_context(t, &prof, &st);
                let x = extract_features_ctx(t, &fctx);
                net.train_step(&x, target, weight);
            }
        });
    }

    pub fn record_artist_follow(&self, user_id: i64, artist_id: &str, followed: bool) {
        if user_id <= 0 {
            return;
        }
        self.mark_taste_dirty(user_id);
        let tracks = self.store.artist_tracks(artist_id).unwrap_or_default();
        if tracks.is_empty() {
            return;
        }
        let prof = self.user_profile(user_id);
        let stats = self.store.track_stats_map(user_id).unwrap_or_default();
        let target = if followed { 0.92 } else { 0.25 };
        let weight = if followed { 1.25 } else { 0.75 };
        self.neural.update_and_save(&self.store, user_id, |net| {
            for t in tracks.iter().take(4) {
                let st = stats.get(&t.id).copied().unwrap_or_default();
                let fctx = build_feature_context(t, &prof, &st);
                let x = extract_features_ctx(t, &fctx);
                net.train_step(&x, target, weight);
            }
        });
    }

    pub fn user_profile(&self, user_id: i64) -> TasteProfile {
        let mut prof = TasteProfile::default();
        if user_id <= 0 {
            return prof;
        }
        let starred = self.store.starred_ids(user_id).unwrap_or_default();
        let ratings = self.store.user_ratings(user_id).unwrap_or_default();
        let followed_artists = self.store.followed_artist_ids(user_id).unwrap_or_default();
        let saved_albums = self.store.saved_album_ids(user_id).unwrap_or_default();

        // 1. Recent events (with recency half-life + explicit playlist signals)
        let events = self.store.user_recent_events(user_id, 900).unwrap_or_default();
        let mut fetch_ids: HashSet<String> = events.iter().map(|e| e.track_id.clone()).collect();
        fetch_ids.extend(starred.iter().cloned());
        fetch_ids.extend(ratings.keys().cloned());
        let id_vec: Vec<String> = fetch_ids.into_iter().collect();
        let track_map: HashMap<String, Track> = self
            .store
            .get_tracks_by_ids(&id_vec)
            .unwrap_or_default()
            .into_iter()
            .map(|t| (t.id.clone(), t))
            .collect();

        let now = Utc::now().timestamp();
        for ev in &events {
            let Some(t) = track_map.get(&ev.track_id) else {
                continue;
            };
            let age_days = ((now - ev.at).max(0) as f64) / 86400.0;
            let decay = (-age_days / 21.0).exp().max(0.25);
            let base_w = match ev.kind.as_str() {
                "star" => 2.4,
                "playlist_add" => 2.1,
                "complete" => 1.5,
                "search_pick" => 1.35,
                "mix_play" => 1.05,
                "play" => 0.65,
                "rate" => (ev.value.clamp(1.0, 5.0) - 2.75) * 1.4,
                "skip" => -1.5,
                "unstar" => -1.6,
                "playlist_remove" => -1.0,
                _ => 0.15,
            };
            apply_track_affinity_delta(&mut prof, t, base_w * decay, false);
        }

        // 2. Persistent stars & explicit ratings anchor taste beyond the 900-event window
        for sid in &starred {
            if let Some(t) = track_map.get(sid) {
                apply_track_affinity_delta(&mut prof, t, 1.8, false);
            }
        }
        for (rid, &rating) in &ratings {
            if let Some(t) = track_map.get(rid) {
                let rw = match rating {
                    5 => 2.4,
                    4 => 1.1,
                    3 => 0.1,
                    2 => -1.4,
                    _ => -3.0,
                };
                apply_track_affinity_delta(&mut prof, t, rw, false);
            }
        }

        // 3. Curated manual playlists teach both playlist DNA and overall user taste
        let pl_entries = self
            .store
            .user_manual_playlist_tracks(user_id)
            .unwrap_or_default();
        if !pl_entries.is_empty() {
            let mut pl_lengths: HashMap<String, usize> = HashMap::new();
            let mut seen_names = HashSet::new();
            for (pid, pname, _) in &pl_entries {
                *pl_lengths.entry(pid.clone()).or_insert(0) += 1;
                if seen_names.insert(pid.clone()) {
                    prof.manual_playlist_names.push(pname.clone());
                }
            }
            for (pid, _, t) in &pl_entries {
                prof.manual_playlist_ids.insert(t.id.clone());
                prof.manual_playlist_song_keys.insert(t.song_key());
                *prof.playlist_track_counts.entry(t.id.clone()).or_insert(0) += 1;

                let n = pl_lengths.get(pid).copied().unwrap_or(1) as f64;
                // Tight curated playlists give stronger per-track signal than huge bulk dumps.
                let cur_w = (2.8 / n.sqrt().max(1.0)).clamp(0.45, 2.2);
                apply_track_affinity_delta(&mut prof, t, cur_w, true);
                apply_track_affinity_delta(&mut prof, t, cur_w * 0.65, false);
            }
        }

        // 4. Followed artists & saved albums boost taste directly
        for artist_id in &followed_artists {
            let norm = artist_id.strip_prefix("ar:").unwrap_or(artist_id);
            *prof.artist_affinity.entry(norm.to_string()).or_insert(0.0) += 2.8;
        }
        for alb_id in &saved_albums {
            for t in self.store.album_tracks(alb_id).unwrap_or_default().iter().take(6) {
                apply_track_affinity_delta(&mut prof, t, 0.55, false);
            }
        }

        prof.starred = starred;
        prof.ratings = ratings;
        prof.followed_artists = followed_artists;
        prof.saved_albums = saved_albums;
        prof
    }

    fn build_scoring_context(&self, user_id: i64) -> ScoringContext {
        let prof = self.user_profile(user_id);
        let stats = if user_id > 0 {
            self.store.track_stats_map(user_id).unwrap_or_default()
        } else {
            HashMap::new()
        };
        let mut net = if user_id > 0 {
            self.neural.get_or_load(&self.store, user_id)
        } else {
            UserNet::new(0)
        };

        // Warm-start / migrate neural net from persisted history when upgrading or cold-loaded.
        if user_id > 0 && net.updates < 4 {
            let samples = self.collect_bootstrap_samples(user_id, &prof, &stats);
            if !samples.is_empty() {
                net.train_batch(&samples, 3);
                net = self.neural.replace_and_save(&self.store, net);
            }
        }

        ScoringContext { prof, stats, net }
    }

    fn collect_bootstrap_samples(
        &self,
        user_id: i64,
        prof: &TasteProfile,
        stats: &HashMap<String, UserTrackStats>,
    ) -> Vec<([f64; crate::neural::INPUT_DIM], f64, f64)> {
        let mut seed_targets: HashMap<String, (f64, f64)> = HashMap::new();
        for ev in self.store.user_recent_events(user_id, 180).unwrap_or_default() {
            if let Some((t, w)) = event_target(&ev.kind, ev.value) {
                seed_targets.entry(ev.track_id).or_insert((t, w));
            }
        }
        for sid in &prof.starred {
            seed_targets.insert(sid.clone(), (0.98, 2.4));
        }
        for pid in &prof.manual_playlist_ids {
            seed_targets.entry(pid.clone()).or_insert((0.92, 1.9));
        }
        for (rid, &rating) in &prof.ratings {
            if let Some(pair) = event_target("rate", rating as f64) {
                seed_targets.insert(rid.clone(), pair);
            }
        }

        let ids: Vec<String> = seed_targets.keys().take(120).cloned().collect();
        let tracks = self.store.get_tracks_by_ids(&ids).unwrap_or_default();
        let mut samples = Vec::with_capacity(tracks.len());
        for t in tracks {
            if let Some(&(target, weight)) = seed_targets.get(&t.id) {
                let st = stats.get(&t.id).copied().unwrap_or_default();
                let fctx = build_feature_context(&t, prof, &st);
                samples.push((extract_features_ctx(&t, &fctx), target, weight));
            }
        }
        samples
    }

    pub fn for_you_tracks(&self, user_id: i64, limit: usize) -> Vec<Track> {
        let candidates = self
            .store
            .candidate_tracks_for_user(user_id, 1800)
            .unwrap_or_default();
        if candidates.is_empty() {
            return vec![];
        }
        let sctx = self.build_scoring_context(user_id);

        let mut scored: Vec<(f64, Track)> = candidates
            .into_iter()
            .filter(|t| !is_strongly_disliked(t, &sctx.prof, &sctx.stats))
            .map(|t| {
                let s = score_track_core(
                    user_id,
                    &t,
                    &sctx.prof,
                    &sctx.stats,
                    Some(&sctx.net),
                    ScoreMode::Supermix,
                );
                (s, t)
            })
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let ranked: Vec<Track> = scored.into_iter().map(|(_, t)| t).collect();
        dedup_and_interleave(ranked, limit, 2)
    }

    pub fn quick_picks(&self, user_id: i64, limit: usize) -> Vec<Track> {
        let pool = self.for_you_tracks(user_id, limit * 3);
        dedup_and_interleave(pool, limit, 1)
    }

    pub fn on_repeat_tracks(&self, user_id: i64, limit: usize) -> Vec<Track> {
        if user_id <= 0 {
            return vec![];
        }
        let prof = self.user_profile(user_id);
        let frequent = self
            .store
            .user_frequent_tracks(user_id, (limit * 4).max(40), 0)
            .unwrap_or_default();
        let now = Utc::now().timestamp();

        let mut scored: Vec<(f64, Track)> = frequent
            .into_iter()
            .filter(|(t, st)| {
                let bad_rating = prof.ratings.get(&t.id).copied().unwrap_or(3) <= 2;
                let net_positive = st.play_count > st.skip_count;
                !bad_rating && (net_positive || prof.starred.contains(&t.id))
            })
            .map(|(t, st)| {
                let age_days = ((now - st.last_played).max(0) as f64) / 86400.0;
                let recency = (-age_days / 30.0).exp().max(0.25);
                let replay_strength = (st.play_count as f64) * 1.5 - (st.skip_count as f64) * 1.6;
                let star_boost = if prof.starred.contains(&t.id) { 1.4 } else { 0.0 };
                let pl_boost = if prof.manual_playlist_ids.contains(&t.id) {
                    1.0
                } else {
                    0.0
                };
                ((replay_strength * recency) + star_boost + pl_boost, t)
            })
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let ranked: Vec<Track> = scored.into_iter().map(|(_, t)| t).collect();
        dedup_and_interleave(ranked, limit, 2)
    }

    pub fn forgotten_gems(&self, user_id: i64, limit: usize) -> Vec<Track> {
        let now = Utc::now().timestamp();
        let cutoff = now - 7 * 86400;
        let sctx = self.build_scoring_context(user_id);
        let starred = self.store.starred_tracks(user_id).unwrap_or_default();

        let mut pool: Vec<(f64, Track)> = Vec::new();
        let mut seen = HashSet::new();

        for (t, at) in starred {
            let lp = sctx.stats.get(&t.id).map(|s| s.last_played).unwrap_or(at);
            if lp < cutoff && !is_strongly_disliked(&t, &sctx.prof, &sctx.stats) && seen.insert(t.id.clone()) {
                pool.push((10.0, t));
            }
        }

        // Also surface older curated manual-playlist tracks & high-rated tracks not played recently.
        let candidates = self
            .store
            .candidate_tracks_for_user(user_id, 1800)
            .unwrap_or_default();
        for t in candidates {
            if !seen.insert(t.id.clone()) || is_strongly_disliked(&t, &sctx.prof, &sctx.stats) {
                continue;
            }
            let st = sctx.stats.get(&t.id).copied().unwrap_or_default();
            let is_curated = sctx.prof.manual_playlist_ids.contains(&t.id)
                || sctx.prof.ratings.get(&t.id).copied().unwrap_or(0) >= 4;
            let aged_play = st.last_played > 0 && st.last_played < cutoff && st.play_count > 0;
            let aged_curated = is_curated && (st.last_played == 0 || st.last_played < cutoff);
            if aged_play || aged_curated {
                let s = score_track_core(
                    user_id,
                    &t,
                    &sctx.prof,
                    &sctx.stats,
                    Some(&sctx.net),
                    ScoreMode::Supermix,
                );
                pool.push((s, t));
            }
        }

        pool.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let ranked: Vec<Track> = pool.into_iter().map(|(_, t)| t).collect();
        dedup_and_interleave(ranked, limit, 2)
    }

    pub fn discover_weekly(&self, user_id: i64, limit: usize) -> Vec<Track> {
        let candidates = self
            .store
            .candidate_tracks_for_user(user_id, 1800)
            .unwrap_or_default();
        if candidates.is_empty() {
            return vec![];
        }
        let sctx = self.build_scoring_context(user_id);

        let mut scored: Vec<(f64, Track)> = candidates
            .into_iter()
            .filter(|t| {
                let st = sctx.stats.get(&t.id).copied().unwrap_or_default();
                let heard = st.play_count > 0 || st.skip_count > 0;
                let known_fav = sctx.prof.starred.contains(&t.id)
                    || sctx.prof.manual_playlist_ids.contains(&t.id)
                    || sctx.prof.manual_playlist_song_keys.contains(&t.song_key())
                    || sctx.prof.ratings.contains_key(&t.id);
                !heard && !known_fav
            })
            .map(|t| {
                let s = score_track_core(
                    user_id,
                    &t,
                    &sctx.prof,
                    &sctx.stats,
                    Some(&sctx.net),
                    ScoreMode::Discover,
                );
                (s, t)
            })
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let ranked: Vec<Track> = scored.into_iter().map(|(_, t)| t).collect();
        dedup_and_interleave(ranked, limit, 1)
    }

    /// Synthesizes a mix built from the musical DNA across all of a user's curated playlists.
    pub fn playlist_dna_mix(&self, user_id: i64, limit: usize) -> Vec<Track> {
        if user_id <= 0 {
            return vec![];
        }
        let entries = self
            .store
            .user_manual_playlist_tracks(user_id)
            .unwrap_or_default();
        if entries.len() < 2 {
            return vec![];
        }
        let sctx = self.build_scoring_context(user_id);
        let pl_tracks: Vec<Track> = entries.iter().map(|(_, _, t)| t.clone()).collect();
        let mut centroid = build_playlist_centroid(&pl_tracks, &sctx.prof.manual_playlist_names.join(" "));
        centroid.existing_ids.clear();
        centroid.existing_keys.clear();

        let candidates = self
            .store
            .candidate_tracks_for_user(user_id, 1800)
            .unwrap_or_default();
        let mut scored: Vec<(f64, Track)> = candidates
            .into_iter()
            .filter(|t| !is_strongly_disliked(t, &sctx.prof, &sctx.stats))
            .map(|t| {
                let sim = score_against_centroid(&t, &centroid);
                let taste = score_track_core(
                    user_id,
                    &t,
                    &sctx.prof,
                    &sctx.stats,
                    Some(&sctx.net),
                    ScoreMode::PlaylistDna,
                );
                let fam_bonus = if sctx.prof.manual_playlist_ids.contains(&t.id) {
                    1.2
                } else {
                    0.5
                };
                (sim * 2.2 + taste * 0.7 + fam_bonus, t)
            })
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let ranked: Vec<Track> = scored.into_iter().map(|(_, t)| t).collect();
        dedup_and_interleave(ranked, limit, 2)
    }

    pub fn rebuild_auto_playlists(&self, user_id: i64) {
        if user_id <= 0 {
            return;
        }
        let specs: [(&str, &str, &str, &str, Vec<String>); 5] = [
            (
                "supermix",
                "My Supermix",
                "Personalized blend of your favorites, playlists & fresh discoveries",
                "supermix",
                self.for_you_tracks(user_id, 35)
                    .into_iter()
                    .map(|t| t.id)
                    .collect(),
            ),
            (
                "discover",
                "Discover Weekly",
                "Fresh tracks tailored to your taste & playlists",
                "discover",
                self.discover_weekly(user_id, 30)
                    .into_iter()
                    .map(|t| t.id)
                    .collect(),
            ),
            (
                "repeat",
                "On Repeat",
                "Songs you can't stop playing",
                "repeat",
                self.on_repeat_tracks(user_id, 25)
                    .into_iter()
                    .map(|t| t.id)
                    .collect(),
            ),
            (
                "gems",
                "Forgotten Favorites",
                "Loved & curated tracks you haven't spun lately",
                "gems",
                self.forgotten_gems(user_id, 25)
                    .into_iter()
                    .map(|t| t.id)
                    .collect(),
            ),
            (
                "playlist-dna",
                "Playlist DNA",
                "Synthesized from the styles inside your own playlists",
                "playlist_dna",
                self.playlist_dna_mix(user_id, 30)
                    .into_iter()
                    .map(|t| t.id)
                    .collect(),
            ),
        ];

        for (suffix, title, comment, kind, ids) in specs {
            let pid = format!("auto-u{}-{}", user_id, suffix);
            if !ids.is_empty() {
                let _ = self
                    .store
                    .upsert_auto_playlist(&pid, user_id, title, comment, kind, &ids);
            } else if suffix == "playlist-dna" {
                let _ = self.store.delete_playlist(&pid);
            }
        }

        self.store.kv_set(&format!("taste:dirty:{}", user_id), "0");
        self.store.kv_set(
            &format!("taste:rebuilt_at:{}", user_id),
            &Utc::now().timestamp().to_string(),
        );
    }

    /// Cold-start / periodic background warming using pure HTTP Deezer global + multi-genre charts.
    pub async fn warm_catalog(&self) {
        let dz = &self.sources.deezer;
        let (
            chart_tracks,
            chart_albums,
            chart_artists,
            pop_tracks,
            hiphop_tracks,
            rock_tracks,
            dance_tracks,
            rnb_tracks,
            alt_tracks,
        ) = tokio::join!(
            dz.fetch_chart_tracks(50),
            dz.fetch_chart_albums(24),
            dz.fetch_chart_artists(24),
            dz.fetch_genre_chart_tracks(132, "Pop", 20),
            dz.fetch_genre_chart_tracks(116, "Hip-Hop", 20),
            dz.fetch_genre_chart_tracks(152, "Rock", 20),
            dz.fetch_genre_chart_tracks(113, "Dance", 20),
            dz.fetch_genre_chart_tracks(165, "R&B", 20),
            dz.fetch_genre_chart_tracks(85, "Alternative", 20),
        );

        let mut all_tracks = chart_tracks;
        for batch in [
            pop_tracks,
            hiphop_tracks,
            rock_tracks,
            dance_tracks,
            rnb_tracks,
            alt_tracks,
        ] {
            all_tracks.extend(batch);
        }
        if !all_tracks.is_empty() {
            let _ = self.store.upsert_tracks(&all_tracks);
        }
        for alb in &chart_albums {
            let _ = self.store.upsert_album_meta(alb);
        }
        for art in &chart_artists {
            let _ = self.store.upsert_artist_meta(art);
        }

        if let Ok(uids) = self.store.all_user_ids() {
            for uid in uids {
                self.rebuild_auto_playlists(uid);
            }
        }
        info!(
            tracks = self.store.count_tracks().unwrap_or(0),
            "catalog warming complete"
        );
    }

    /// Ensures a full album is loaded into SQLite (hydrating from Deezer if it only has stubs).
    pub async fn get_or_hydrate_album(&self, album_id: &str) -> Option<(AlbumSummary, Vec<Track>)> {
        let existing_tracks = self.store.album_tracks(album_id).unwrap_or_default();
        let existing_meta = self.store.get_album_meta(album_id).ok().flatten();

        // Determine if we know a Deezer album ID to hydrate full tracklist from.
        let dz_album_id = existing_meta
            .as_ref()
            .map(|m| m.deezer_id.clone())
            .filter(|id| !id.is_empty())
            .or_else(|| {
                album_id
                    .strip_prefix("dz:")
                    .map(|s| s.to_string())
            })
            .or_else(|| {
                existing_tracks.iter().find_map(|t| {
                    serde_json::from_str::<serde_json::Value>(&t.meta)
                        .ok()
                        .and_then(|v| v.get("deezer_album_id").and_then(|x| x.as_u64()))
                        .filter(|&n| n > 0)
                        .map(|n| n.to_string())
                })
            });

        if existing_tracks.len() <= 2 {
            if let Some(ref dz_id) = dz_album_id {
                if let Some((meta, tracks)) = self.sources.deezer.fetch_album(dz_id).await {
                    let _ = self.store.upsert_album_meta(&meta);
                    let _ = self.store.upsert_tracks(&tracks);
                    let final_tracks = self.store.album_tracks(&meta.id).unwrap_or(tracks);
                    return Some((meta, final_tracks));
                }
            }
        }

        if let Some(summary) = self.store.album_summary_by_id(0, album_id).ok().flatten() {
            let tracks = if existing_tracks.is_empty() {
                self.store.album_tracks(&summary.id).unwrap_or_default()
            } else {
                existing_tracks
            };
            return Some((summary, tracks));
        }

        if let Some(dz_id) = dz_album_id {
            if let Some((meta, tracks)) = self.sources.deezer.fetch_album(&dz_id).await {
                let _ = self.store.upsert_album_meta(&meta);
                let _ = self.store.upsert_tracks(&tracks);
                return Some((meta, tracks));
            }
        }
        None
    }

    /// Loads full artist profile + top tracks + albums + related artists, hydrating live from Deezer when available.
    pub async fn get_or_hydrate_artist(
        &self,
        artist_id: &str,
        user_id: i64,
    ) -> Option<(ArtistSummary, Vec<Track>, Vec<AlbumSummary>, Vec<ArtistSummary>)> {
        let mut local_summary = self.store.artist_summary_by_id(artist_id).ok().flatten();
        let dz_artist_id = local_summary
            .as_ref()
            .map(|a| a.deezer_id.clone())
            .filter(|id| !id.is_empty())
            .or_else(|| artist_id.strip_prefix("dz:").map(|s| s.to_string()));

        let resolved_dz_id = match dz_artist_id {
            Some(id) => Some(id),
            None => {
                let name = local_summary
                    .as_ref()
                    .map(|a| a.name.clone())
                    .unwrap_or_else(|| {
                        artist_id
                            .trim_start_matches("ar:")
                            .replace('-', " ")
                    });
                self.sources
                    .deezer
                    .search_artists(&name, 3)
                    .await
                    .into_iter()
                    .find(|a| a.id == artist_id || slug(&a.name) == slug(&name))
                    .map(|a| a.deezer_id)
            }
        };

        if let Some(dz_id) = resolved_dz_id {
            if let Some((meta, top_tracks, dz_albums, related)) =
                self.sources.deezer.fetch_artist(&dz_id).await
            {
                let _ = self.store.upsert_artist_meta(&meta);
                let _ = self.store.upsert_tracks(&top_tracks);
                for a in &dz_albums {
                    let _ = self.store.upsert_album_meta(a);
                }
                for r in &related {
                    let _ = self.store.upsert_artist_meta(r);
                }
                let merged_tracks = merge_tracks_dedup(
                    self.store.artist_tracks(&meta.id).unwrap_or_default(),
                    top_tracks,
                );
                let merged_albums = merge_albums_dedup(
                    self.store.artist_albums(user_id, &meta.id).unwrap_or_default(),
                    dz_albums,
                );
                local_summary = Some(meta);
                return Some((
                    local_summary.unwrap(),
                    merged_tracks,
                    merged_albums,
                    related,
                ));
            }
        }

        let summary = local_summary?;
        let tracks = self.store.artist_tracks(&summary.id).unwrap_or_default();
        let albums = self.store.artist_albums(user_id, &summary.id).unwrap_or_default();
        Some((summary, tracks, albums, vec![]))
    }

    pub async fn similar_songs_async(&self, seed: &Track, count: usize) -> Vec<Track> {
        let mut out = Vec::new();
        let mut seen_keys: HashSet<String> = HashSet::new();
        seen_keys.insert(seed.song_key());

        let pa = primary_artist(&seed.artist);
        let pa_slug = slug(&pa);
        let is_ytm_seed = seed.source == "ytm" || seed.id.starts_with("ytm:");
        let target_fetch = (count * 2).max(24);

        // 1. For YouTube Music tracks, seed directly from YouTube Music's song radio mix.
        if is_ytm_seed {
            let vid = seed.id.trim_start_matches("ytm:");
            let radio = self.sources.ytm.fetch_radio_tracks(vid, target_fetch).await;
            self.push_radio_tracks(&mut seen_keys, &mut out, radio);
        }

        // 2. For Deezer/local/archive seeds (or when YTM radio yielded almost nothing),
        //    match an exact Deezer artist and pull artist radio.
        if (!is_ytm_seed || out.len() < 4) && !pa.is_empty() {
            let artists = self.sources.deezer.search_artists(&pa, 5).await;
            let matched = if !is_ytm_seed {
                artists
                    .iter()
                    .find(|a| slug(&a.name) == pa_slug)
                    .or_else(|| artists.first())
            } else {
                artists.iter().find(|a| slug(&a.name) == pa_slug)
            };
            if let Some(art) = matched {
                if !art.deezer_id.is_empty() {
                    let radio = self.sources.deezer.fetch_artist_radio(&art.deezer_id).await;
                    self.push_radio_tracks(&mut seen_keys, &mut out, radio);
                }
            }
        }

        // 3. If non-YTM seed had insufficient Deezer radio, resolve on YTM for track-level radio.
        if !is_ytm_seed && out.len() < 6 {
            if let Some(vid) = self
                .sources
                .ytm
                .find_best_video_for_track(&pa, &seed.title, seed.duration)
                .await
            {
                let radio = self.sources.ytm.fetch_radio_tracks(&vid, target_fetch).await;
                self.push_radio_tracks(&mut seen_keys, &mut out, radio);
            }
        }

        // 4. Supplement from local catalog matching artist, genre, or moods.
        if out.len() < count {
            self.supplement_radio_from_catalog(seed, count, &mut seen_keys, &mut out);
        }

        dedup_and_interleave(out, count, 3)
    }

    fn push_radio_tracks(
        &self,
        seen_keys: &mut HashSet<String>,
        out: &mut Vec<Track>,
        radio: Vec<Track>,
    ) {
        if radio.is_empty() {
            return;
        }
        let _ = self.store.upsert_tracks(&radio);
        for t in radio {
            if seen_keys.insert(t.song_key()) {
                out.push(t);
            }
        }
    }

    fn supplement_radio_from_catalog(
        &self,
        seed: &Track,
        count: usize,
        seen_keys: &mut HashSet<String>,
        out: &mut Vec<Track>,
    ) {
        let seed_moods: HashSet<Mood> = seed.mood_list().into_iter().collect();
        let seed_genre = seed.genre.to_lowercase();
        let pa_slug = slug(&primary_artist(&seed.artist));
        let require_affinity = !out.is_empty();
        let all = self.store.all_tracks(500, 0).unwrap_or_default();

        let mut scored: Vec<(i32, Track)> = all
            .into_iter()
            .filter(|t| !seen_keys.contains(&t.song_key()))
            .filter_map(|t| {
                let mut aff = 0i32;
                if !seed_genre.is_empty() && t.genre.to_lowercase() == seed_genre {
                    aff += 60;
                }
                let shared_m = t
                    .mood_list()
                    .iter()
                    .filter(|m| seed_moods.contains(m))
                    .count() as i32;
                aff += shared_m * 35;
                if !pa_slug.is_empty() && slug(&primary_artist(&t.artist)) == pa_slug {
                    aff += 25;
                }
                if require_affinity && aff == 0 {
                    return None;
                }
                Some((aff + t.quality_score() / 50, t))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0));
        for (_, t) in scored {
            if out.len() >= count * 2 {
                break;
            }
            if seen_keys.insert(t.song_key()) {
                out.push(t);
            }
        }
    }

    pub async fn recommend_for_playlist(
        &self,
        user_id: i64,
        playlist_id: &str,
        limit: usize,
    ) -> Vec<Track> {
        let Some(pl) = self.store.get_playlist(playlist_id).ok().flatten() else {
            return vec![];
        };
        let pl_tracks = self.store.playlist_tracks(playlist_id).unwrap_or_default();
        let query_hint = playlist_query_hint(&pl.name, &pl.comment);
        let centroid = build_playlist_centroid(&pl_tracks, &query_hint);
        let sctx = self.build_scoring_context(user_id);

        // Sample up to 3 diverse artists from the playlist to pull fresh related candidates.
        let mut seeds = Vec::new();
        let mut seen_artists = HashSet::new();
        for t in &pl_tracks {
            let ak = slug(&primary_artist(&t.artist));
            if seen_artists.insert(ak) {
                seeds.push(t.clone());
                if seeds.len() >= 3 {
                    break;
                }
            }
        }

        let live_futs = seeds.iter().map(|s| self.similar_songs_async(s, 8));
        let mut live_pool: Vec<Track> = futures::future::join_all(live_futs)
            .await
            .into_iter()
            .flatten()
            .collect();

        if pl_tracks.len() < 3 && !query_hint.is_empty() {
            let found = self.sources.search_all(&query_hint, 10).await;
            if !found.is_empty() {
                let _ = self.store.upsert_tracks(&found);
                live_pool.extend(found);
            }
        }

        let catalog = self
            .store
            .candidate_tracks_for_user(user_id, 1800)
            .unwrap_or_default();

        let mut dedup_candidates = Vec::new();
        let mut seen_ids = HashSet::new();
        for t in live_pool.into_iter().chain(catalog) {
            if seen_ids.insert(t.id.clone())
                && !centroid.existing_ids.contains(&t.id)
                && !centroid.existing_keys.contains(&t.song_key())
                && !is_strongly_disliked(&t, &sctx.prof, &sctx.stats)
            {
                dedup_candidates.push(t);
            }
        }

        let mut scored: Vec<(f64, Track)> = dedup_candidates
            .into_iter()
            .map(|t| {
                let c_sim = score_against_centroid(&t, &centroid);
                let u_taste = score_track_core(
                    user_id,
                    &t,
                    &sctx.prof,
                    &sctx.stats,
                    Some(&sctx.net),
                    ScoreMode::PlaylistDna,
                );
                let combined = if pl_tracks.is_empty() {
                    c_sim * 2.0 + u_taste * 0.9
                } else {
                    c_sim * 2.6 + u_taste * 0.65
                };
                (combined, t)
            })
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let ranked: Vec<Track> = scored.into_iter().map(|(_, t)| t).collect();
        dedup_and_interleave(ranked, limit, 2)
    }

    pub async fn playlist_radio_tracks(
        &self,
        user_id: i64,
        playlist_id: &str,
        exclude_ids: &HashSet<String>,
        count: usize,
    ) -> Vec<Track> {
        let recs = self
            .recommend_for_playlist(user_id, playlist_id, count * 2)
            .await;
        let pl_tracks = self.store.playlist_tracks(playlist_id).unwrap_or_default();
        let sctx = self.build_scoring_context(user_id);

        let mut familiar: Vec<Track> = pl_tracks
            .into_iter()
            .filter(|t| !exclude_ids.contains(&t.id) && !is_strongly_disliked(t, &sctx.prof, &sctx.stats))
            .collect();
        familiar.sort_by(|a, b| {
            let sa = score_track_core(user_id, a, &sctx.prof, &sctx.stats, Some(&sctx.net), ScoreMode::Supermix);
            let sb = score_track_core(user_id, b, &sctx.prof, &sctx.stats, Some(&sctx.net), ScoreMode::Supermix);
            sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
        });

        let fresh: Vec<Track> = recs
            .into_iter()
            .filter(|t| !exclude_ids.contains(&t.id))
            .collect();

        // Interleave ~3 fresh discoveries per 1 familiar playlist track so radio feels cohesive yet new.
        let mut combined = Vec::new();
        let mut fi = fresh.into_iter();
        let mut ki = familiar.into_iter();
        while combined.len() < count * 2 {
            let mut pushed = false;
            for _ in 0..3 {
                if let Some(t) = fi.next() {
                    combined.push(t);
                    pushed = true;
                }
            }
            if let Some(t) = ki.next() {
                combined.push(t);
                pushed = true;
            }
            if !pushed {
                break;
            }
        }
        if combined.is_empty() {
            return self.for_you_tracks(user_id, count);
        }
        dedup_and_interleave(combined, count, 2)
    }

    pub async fn radio_next_tracks_with_exclusions(
        &self,
        user_id: i64,
        seed: Option<&Track>,
        exclude_ids: &HashSet<String>,
        count: usize,
    ) -> Vec<Track> {
        if let Some(s) = seed {
            let sims = self.similar_songs_async(s, (count * 2).max(16)).await;
            let filtered: Vec<Track> = sims
                .into_iter()
                .filter(|t| !exclude_ids.contains(&t.id))
                .collect();
            if !filtered.is_empty() {
                if user_id <= 0 {
                    return dedup_and_interleave(filtered, count, 2);
                }
                let sctx = self.build_scoring_context(user_id);
                let total = filtered.len().max(1) as f64;
                let mut scored: Vec<(f64, Track)> = filtered
                    .into_iter()
                    .enumerate()
                    .filter(|(_, t)| !is_strongly_disliked(t, &sctx.prof, &sctx.stats))
                    .map(|(idx, t)| {
                        // Preserve strong similarity prior while tailoring order to user taste & skips.
                        let sim_prior = ((total - idx as f64) / total) * 4.5;
                        let user_fit = score_track_core(
                            user_id,
                            &t,
                            &sctx.prof,
                            &sctx.stats,
                            Some(&sctx.net),
                            ScoreMode::Supermix,
                        ) * 0.45;
                        (sim_prior + user_fit, t)
                    })
                    .collect();
                if !scored.is_empty() {
                    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
                    let ranked: Vec<Track> = scored.into_iter().map(|(_, t)| t).collect();
                    return dedup_and_interleave(ranked, count, 2);
                }
            }
        }
        let picks: Vec<Track> = self
            .for_you_tracks(user_id, count * 2)
            .into_iter()
            .filter(|t| !exclude_ids.contains(&t.id))
            .collect();
        dedup_and_interleave(picks, count, 2)
    }

    /// Pulls local & cached SQLite tracks matching a parsed search intent (e.g., 1980s year tags
    /// or seasonal/genre keywords) so offline libraries and cached gems fuel instant mixes.
    pub fn matching_catalog_tracks(&self, intent: &SearchIntent, limit: usize) -> Vec<Track> {
        if limit == 0 {
            return vec![];
        }
        let all = self.store.all_tracks(1500, 0).unwrap_or_default();
        let mut matched: Vec<(f64, Track)> = all
            .into_iter()
            .filter_map(|t| {
                let era = compute_query_era_match(&t, intent);
                let theme = compute_query_theme_match(&t, intent);
                let lex = search_match_score(&t, &intent.normalized) as f64;
                if let Some((min_y, max_y)) = intent.year_range {
                    let in_era = t.year > 1920 && t.year >= min_y && t.year <= max_y;
                    if !in_era && theme < 0.45 {
                        return None;
                    }
                } else if !intent.themes.is_empty() {
                    if theme < 0.35 && lex == 0.0 {
                        return None;
                    }
                } else if theme < 0.25 && lex == 0.0 {
                    return None;
                }
                let pop = parse_rank_bonus(&t.meta);
                let qual = (t.quality_score() as f64) / 500.0;
                let s = era.max(0.0) * 3.0 + theme * 3.5 + (lex / 200.0).min(2.0) + pop + qual;
                Some((s, t))
            })
            .collect();
        matched.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        matched.into_iter().take(limit).map(|(_, t)| t).collect()
    }

    /// Actively sorts and refines search results using `UserNet`, listening taste,
    /// lexical anchors, and query intent—and builds an ephemeral on-the-fly custom mix
    /// without persisting a playlist.
    pub fn search_and_refine(
        &self,
        user_id: i64,
        query: &str,
        raw_tracks: Vec<Track>,
        curated_batches: Vec<(String, Vec<Track>)>,
    ) -> SearchMixResult {
        let intent = parse_search_intent(query);
        let sctx = self.build_scoring_context(user_id);

        // Track provenance from curated playlists & intent-expanded catalog hits before dedup.
        let mut curated_prior_by_key: HashMap<String, f64> = HashMap::new();
        let mut combined_pool = raw_tracks;
        for (pl_title, batch) in curated_batches {
            let pl_low = pl_title.to_lowercase();
            let title_overlap = intent
                .tokens
                .iter()
                .filter(|tok| pl_low.contains(tok.as_str()))
                .count();
            let base_prior = if title_overlap > 0 { 2.4 } else { 1.6 };
            for t in batch {
                let k = t.song_key();
                let entry = curated_prior_by_key.entry(k).or_insert(0.0);
                if base_prior > *entry {
                    *entry = base_prior;
                }
                combined_pool.push(t);
            }
        }

        if intent.is_mix_query || combined_pool.len() < 12 {
            for t in self.matching_catalog_tracks(&intent, 65) {
                combined_pool.push(t);
            }
        }

        let deduped = dedup_tracks_prefer_quality(combined_pool);
        if deduped.is_empty() {
            let empty_matches = HashSet::new();
            return SearchMixResult {
                tracks: vec![],
                instant_mix: None,
                refinements: build_search_refinements(&intent, &[], &empty_matches, &sctx),
                taste_match_ids: empty_matches,
                track_neural_scores: HashMap::new(),
            };
        }

        struct Evaluated {
            track: Track,
            search_score: f64,
            mix_score: f64,
            era_match: f64,
            theme_match: f64,
            curated_prior: f64,
            is_junk: bool,
            disliked: bool,
        }

        let mut evals: Vec<Evaluated> = Vec::with_capacity(deduped.len());
        let mut taste_match_ids = HashSet::new();
        let mut track_neural_scores = HashMap::new();

        for t in deduped {
            let curated_prior = curated_prior_by_key
                .get(&t.song_key())
                .copied()
                .unwrap_or(0.0);
            let lex = search_match_score(&t, &intent.normalized) as f64;
            let era_match = compute_query_era_match(&t, &intent);
            let theme_match = compute_query_theme_match(&t, &intent);
            let is_junk = is_junk_track_for_query(&t, &intent.normalized);
            let disliked = is_strongly_disliked(&t, &sctx.prof, &sctx.stats);

            let st = sctx.stats.get(&t.id).copied().unwrap_or_default();
            let fctx = build_feature_context_with_query(
                &t,
                &sctx.prof,
                &st,
                Some(&intent),
                curated_prior > 0.0,
            );
            let x = extract_features_ctx(&t, &fctx);
            let net_prob = if user_id > 0 && sctx.net.updates > 0 {
                sctx.net.predict(&x)
            } else {
                0.5
            };
            let conf = if user_id > 0 && sctx.net.updates > 0 {
                sctx.net.confidence().max(0.25)
            } else {
                0.0
            };

            let neural_delta = (net_prob - 0.5) * 7.5 * conf;
            let taste_aff = fctx.artist_affinity * 0.85
                + fctx.genre_affinity * 0.65
                + fctx.mood_affinity * 0.45
                + fctx.playlist_artist_affinity * 0.5
                + fctx.playlist_genre_affinity * 0.4
                + if fctx.followed_artist { 1.4 } else { 0.0 }
                + if fctx.saved_album { 0.9 } else { 0.0 }
                + if sctx.prof.starred.contains(&t.id) { 1.8 } else { 0.0 };
            let pop_bonus = parse_rank_bonus(&t.meta);
            let quality_bonus = (t.quality_score() as f64) / 360.0;
            let skip_penalty = if disliked {
                12.0
            } else {
                (st.skip_count as f64) * 1.4
            };
            let junk_penalty = if is_junk { 14.0 } else { 0.0 };

            let is_taste_match = (user_id > 0 && sctx.net.updates >= 3 && net_prob >= 0.62)
                || fctx.artist_affinity >= 1.2
                || fctx.genre_affinity >= 1.2
                || fctx.followed_artist
                || sctx.prof.starred.contains(&t.id);
            if is_taste_match && !disliked && !is_junk {
                taste_match_ids.insert(t.id.clone());
            }
            track_neural_scores.insert(t.id.clone(), net_prob);

            let tl = t.title.to_lowercase();
            let al = primary_artist(&t.artist).to_lowercase();
            let is_exact_entity = !is_junk && (tl == intent.normalized || al == intent.normalized);

            // Pin exact song/artist matches at the top for entity searches while letting
            // UserNet & taste refine broad/ambiguous queries and tie-breaks.
            let lex_anchor = if intent.is_theme_mix {
                if is_exact_entity {
                    28.0
                } else if lex > 0.0 {
                    (lex / 110.0).min(4.0)
                } else {
                    0.0
                }
            } else if is_exact_entity {
                28.0 + (lex - 250.0).max(0.0) / 50.0
            } else if lex >= 450.0 {
                19.0 + (lex - 450.0) / 50.0
            } else if lex >= 280.0 {
                10.5 + (lex - 280.0) / 35.0
            } else if lex > 0.0 {
                lex / 55.0
            } else {
                0.0
            };

            let intent_fit = era_match * 4.5
                + theme_match * 4.5
                + curated_prior * 1.4
                + if intent.wants_hits { pop_bonus * 0.95 } else { pop_bonus * 0.65 };

            let search_score = lex_anchor
                + if intent.is_mix_query {
                    intent_fit
                } else {
                    era_match * 1.5 + theme_match * 1.5 + curated_prior * 0.6 + pop_bonus * 0.55
                }
                + neural_delta
                + taste_aff
                + quality_bonus
                - skip_penalty
                - junk_penalty;

            // Ephemeral instant-mix score focuses on theme/era coherence, hit quality, and UserNet fit.
            let mix_neural = (net_prob - 0.5) * 9.0 * conf;
            let mix_score = era_match * 5.2
                + theme_match * 5.5
                + curated_prior * 2.4
                + (if intent.wants_hits || intent.is_theme_mix {
                    pop_bonus * 1.25
                } else {
                    pop_bonus * 0.75
                })
                + (if !intent.is_theme_mix && lex > 0.0 {
                    (lex / 120.0).min(4.5)
                } else {
                    0.0
                })
                + mix_neural
                + taste_aff * 0.95
                + quality_bonus * 0.7
                - (st.skip_count as f64) * 1.8;

            evals.push(Evaluated {
                track: t,
                search_score: if search_score.is_finite() { search_score } else { 0.0 },
                mix_score: if mix_score.is_finite() { mix_score } else { 0.0 },
                era_match,
                theme_match,
                curated_prior,
                is_junk,
                disliked,
            });
        }

        // 1. Sort search results cleanly
        evals.sort_by(|a, b| {
            b.search_score
                .partial_cmp(&a.search_score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.track.quality_score().cmp(&a.track.quality_score()))
        });
        let ranked_tracks: Vec<Track> = evals.iter().map(|e| e.track.clone()).collect();

        // 2. Build ephemeral on-the-fly custom mix (never saved as a playlist)
        let mut mix_pool: Vec<(f64, Track)> = evals
            .iter()
            .filter(|e| {
                if e.is_junk || e.disliked {
                    return false;
                }
                if intent.year_range.is_some() && e.era_match <= -0.65 {
                    return false;
                }
                if !intent.themes.is_empty() && e.theme_match < 0.2 && e.curated_prior == 0.0 {
                    return false;
                }
                if intent.is_theme_mix
                    && e.curated_prior == 0.0
                    && e.era_match <= 0.0
                    && e.theme_match < 0.25
                {
                    return false;
                }
                true
            })
            .map(|e| (e.mix_score, e.track.clone()))
            .collect();

        // Relax only if strict filter yielded fewer than 2 tracks while query clearly wanted a non-themed mix.
        if mix_pool.len() < 2 && intent.is_mix_query && intent.themes.is_empty() {
            mix_pool = evals
                .iter()
                .filter(|e| !e.is_junk && !e.disliked && e.era_match > -0.8)
                .map(|e| (e.mix_score, e.track.clone()))
                .collect();
        }

        mix_pool.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let mix_ranked: Vec<Track> = mix_pool.into_iter().map(|(_, t)| t).collect();
        let mix_tracks = dedup_and_interleave(mix_ranked, 25, 2);

        let instant_mix = if (intent.is_theme_mix && mix_tracks.len() >= 2) || mix_tracks.len() >= 5 {
            let (title, subtitle) = build_instant_mix_title(&intent, &mix_tracks, &sctx.net);
            let total_duration = mix_tracks.iter().map(|t| t.duration.max(0)).sum();
            Some(InstantMix {
                title,
                subtitle,
                query: intent.raw.clone(),
                is_theme_mix: intent.is_theme_mix,
                tracks: mix_tracks,
                total_duration,
            })
        } else {
            None
        };

        let refinements =
            build_search_refinements(&intent, &ranked_tracks, &taste_match_ids, &sctx);

        SearchMixResult {
            tracks: ranked_tracks,
            instant_mix,
            refinements,
            taste_match_ids,
            track_neural_scores,
        }
    }
}

fn apply_track_affinity_delta(prof: &mut TasteProfile, t: &Track, w: f64, playlist_layer: bool) {
    let a_key = slug(&primary_artist(&t.artist));
    let target_artist = if playlist_layer {
        &mut prof.playlist_artist_affinity
    } else {
        &mut prof.artist_affinity
    };
    if !a_key.is_empty() {
        *target_artist.entry(a_key).or_insert(0.0) += w;
    }

    let target_genre = if playlist_layer {
        &mut prof.playlist_genre_affinity
    } else {
        &mut prof.genre_affinity
    };
    for gl in split_genres(&t.genre) {
        *target_genre.entry(gl).or_insert(0.0) += w;
    }

    let target_mood = if playlist_layer {
        &mut prof.playlist_mood_affinity
    } else {
        &mut prof.mood_affinity
    };
    for m in t.mood_list() {
        *target_mood.entry(m).or_insert(0.0) += w;
    }
}

fn build_feature_context(t: &Track, prof: &TasteProfile, st: &UserTrackStats) -> FeatureContext {
    build_feature_context_with_query(t, prof, st, None, false)
}

fn build_feature_context_with_query(
    t: &Track,
    prof: &TasteProfile,
    st: &UserTrackStats,
    intent: Option<&SearchIntent>,
    curated_match: bool,
) -> FeatureContext {
    let artist_slug = slug(&primary_artist(&t.artist));
    let artist_affinity = prof.artist_affinity.get(&artist_slug).copied().unwrap_or(0.0);
    let playlist_artist_affinity = prof
        .playlist_artist_affinity
        .get(&artist_slug)
        .copied()
        .unwrap_or(0.0);

    let genres = split_genres(&t.genre);
    let genre_affinity = genres
        .iter()
        .filter_map(|g| prof.genre_affinity.get(g))
        .fold(0.0f64, |acc, v| if v.abs() > acc.abs() { *v } else { acc });
    let playlist_genre_affinity = genres
        .iter()
        .filter_map(|g| prof.playlist_genre_affinity.get(g))
        .fold(0.0f64, |a, b| a.max(*b));

    let moods = t.mood_list();
    let mood_affinity = if moods.is_empty() {
        0.0
    } else {
        moods
            .iter()
            .filter_map(|m| prof.mood_affinity.get(m))
            .sum::<f64>()
            / (moods.len() as f64)
    };
    let playlist_mood_affinity = if moods.is_empty() {
        0.0
    } else {
        moods
            .iter()
            .filter_map(|m| prof.playlist_mood_affinity.get(m))
            .sum::<f64>()
            / (moods.len() as f64)
    };

    let followed_artist = prof.followed_artists.contains(&t.artist_id())
        || prof.followed_artists.contains(&format!("ar:{}", artist_slug))
        || prof.followed_artists.contains(&artist_slug);
    let saved_album = prof.saved_albums.contains(&t.album_id());
    let popularity_prior = (parse_rank_bonus(&t.meta) / 5.0).clamp(0.0, 1.0);

    let (query_active, query_lexical_fit, query_era_match, query_theme_match, query_popularity_intent) =
        if let Some(qi) = intent {
            let lex = search_match_score(t, &qi.normalized) as f64;
            let era = compute_query_era_match(t, qi);
            let theme = compute_query_theme_match(t, qi).max(if curated_match { 0.45 } else { 0.0 });
            let pop_intent = if qi.wants_hits || qi.wants_classic || qi.decade_label.is_some() {
                1.0
            } else {
                0.35
            };
            (true, (lex / 500.0).clamp(0.0, 1.0), era, theme, pop_intent)
        } else {
            (false, 0.0, 0.0, 0.0, 0.0)
        };

    FeatureContext {
        artist_affinity,
        genre_affinity,
        mood_affinity,
        playlist_artist_affinity,
        playlist_genre_affinity,
        playlist_mood_affinity,
        followed_artist,
        saved_album,
        track_plays: st.play_count,
        track_skips: st.skip_count,
        popularity_prior,
        hour_override: None,
        query_active,
        query_lexical_fit,
        query_era_match,
        query_theme_match,
        query_popularity_intent,
    }
}

pub fn parse_search_intent(raw: &str) -> SearchIntent {
    let trimmed = raw.trim();
    let normalized = trimmed.to_lowercase();
    let clean_norm = normalized.replace('\'', "");
    let tokens: Vec<String> = clean_norm
        .split(|c: char| !c.is_alphanumeric() && c != '-' && c != '&')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();

    let mut decade_label = None;
    let mut year_range = None;
    let mut wants_hits = false;
    let mut wants_classic = false;
    let mut explicit_mix_word = false;
    let mut moods: Vec<Mood> = Vec::new();
    let mut genres: Vec<String> = Vec::new();
    let mut themes: Vec<ThemeSpec> = Vec::new();
    let mut matched_token_flags = vec![false; tokens.len()];

    let theme_catalog: &[(&[&str], ThemeSpec)] = &[
        (
            &["christmas", "xmas", "holiday", "holidays", "santa", "noel", "yuletide"],
            ThemeSpec {
                key: "christmas",
                label: "Christmas",
                keywords: &[
                    "christmas", "xmas", "santa", "jingle", "silent night", "white christmas",
                    "last christmas", "let it snow", "winter wonderland", "rudolph", "frosty",
                    "feliz navidad", "mistletoe", "sleigh", "holly jolly", "noel", "carol",
                    "all i want for christmas", "rockin' around", "it's beginning to look",
                    "wonderful christmastime", "happy xmas", "o holy night",
                ],
                search_boost: "classic christmas",
            },
        ),
        (
            &["halloween", "spooky"],
            ThemeSpec {
                key: "halloween",
                label: "Halloween",
                keywords: &["halloween", "spooky", "ghost", "monster", "thriller", "witch", "zombie", "nightmare"],
                search_boost: "halloween hits",
            },
        ),
        (
            &["summer", "beach", "sunny", "tropical"],
            ThemeSpec {
                key: "summer",
                label: "Summer",
                keywords: &["summer", "sun", "sunshine", "beach", "island", "ocean", "california", "tropical", "heat"],
                search_boost: "summer hits",
            },
        ),
        (
            &["workout", "gym", "cardio", "fitness", "running"],
            ThemeSpec {
                key: "workout",
                label: "Workout",
                keywords: &["workout", "power", "stronger", "energy", "run", "hype", "pump", "beast"],
                search_boost: "workout hits",
            },
        ),
        (
            &["study", "studying", "focus", "coding", "homework"],
            ThemeSpec {
                key: "focus",
                label: "Focus & Study",
                keywords: &["ambient", "instrumental", "piano", "study", "lofi", "lo-fi", "chill", "focus"],
                search_boost: "lofi study beats",
            },
        ),
        (
            &["sleep", "bedtime", "rainy", "cozy"],
            ThemeSpec {
                key: "sleep",
                label: "Sleep & Cozy",
                keywords: &["sleep", "rain", "ambient", "soft", "dream", "acoustic", "calm", "lullaby"],
                search_boost: "chill sleep ambient",
            },
        ),
        (
            &["roadtrip", "driving", "cruise"],
            ThemeSpec {
                key: "roadtrip",
                label: "Road Trip",
                keywords: &["drive", "highway", "road", "ride", "cruise", "nightcall"],
                search_boost: "road trip hits",
            },
        ),
    ];

    let genre_map: &[(&str, &str)] = &[
        ("hip hop", "Hip-Hop"),
        ("hip-hop", "Hip-Hop"),
        ("hiphop", "Hip-Hop"),
        ("rap", "Rap"),
        ("r&b", "R&B"),
        ("rnb", "R&B"),
        ("new wave", "New Wave"),
        ("synthpop", "Synthpop"),
        ("synth pop", "Synthpop"),
        ("synthwave", "Synthwave"),
        ("lo-fi", "Lo-Fi"),
        ("lofi", "Lo-Fi"),
        ("rock", "Rock"),
        ("pop", "Pop"),
        ("jazz", "Jazz"),
        ("blues", "Blues"),
        ("soul", "Soul"),
        ("funk", "Funk"),
        ("disco", "Disco"),
        ("electronic", "Electronic"),
        ("edm", "EDM"),
        ("house", "House"),
        ("techno", "Techno"),
        ("trance", "Trance"),
        ("metal", "Metal"),
        ("punk", "Punk"),
        ("grunge", "Grunge"),
        ("indie", "Indie"),
        ("folk", "Folk"),
        ("country", "Country"),
        ("classical", "Classical"),
        ("reggae", "Reggae"),
        ("latin", "Latin"),
        ("ambient", "Ambient"),
        ("acoustic", "Acoustic"),
    ];

    let mood_tokens: &[(&str, Mood)] = &[
        ("chill", Mood::Chill),
        ("lofi", Mood::Chill),
        ("lo-fi", Mood::Chill),
        ("relax", Mood::Chill),
        ("focus", Mood::Focus),
        ("study", Mood::Focus),
        ("energy", Mood::Energy),
        ("workout", Mood::Energy),
        ("hype", Mood::Energy),
        ("night", Mood::LateNight),
        ("midnight", Mood::LateNight),
        ("upbeat", Mood::Upbeat),
        ("happy", Mood::Upbeat),
        ("sad", Mood::Melancholy),
        ("melancholy", Mood::Melancholy),
        ("party", Mood::Party),
        ("dance", Mood::Party),
        ("club", Mood::Party),
        ("acoustic", Mood::Acoustic),
        ("groove", Mood::Groove),
    ];

    let stopwords: HashSet<&str> = [
        "the", "of", "and", "for", "in", "to", "a", "an", "songs", "tracks", "music", "era",
        "from", "with", "all", "&",
    ]
    .into_iter()
    .collect();

    if clean_norm.contains("late night") {
        moods.push(Mood::LateNight);
    }
    if clean_norm.contains("feel good") {
        moods.push(Mood::Upbeat);
    }
    if clean_norm.contains("road trip") {
        themes.push(theme_catalog[6].1.clone());
    }

    for (idx, tok) in tokens.iter().enumerate() {
        if let Some((lbl, min_y, max_y)) = parse_decade_token(tok) {
            decade_label = Some(lbl);
            year_range = Some((min_y, max_y));
            matched_token_flags[idx] = true;
            continue;
        }
        if matches!(
            tok.as_str(),
            "classic" | "classics" | "oldies" | "retro" | "vintage" | "golden" | "traditional" | "throwback"
        ) {
            wants_classic = true;
            matched_token_flags[idx] = true;
            continue;
        }
        if matches!(
            tok.as_str(),
            "hits" | "hit" | "best" | "top" | "greatest" | "essential" | "essentials" | "anthems" | "popular"
        ) {
            wants_hits = true;
            matched_token_flags[idx] = true;
            continue;
        }
        if matches!(tok.as_str(), "playlist" | "mix" | "radio" | "vibes" | "vibe") {
            explicit_mix_word = true;
            matched_token_flags[idx] = true;
            continue;
        }
        if stopwords.contains(tok.as_str()) {
            matched_token_flags[idx] = true;
        }

        for (keys, spec) in theme_catalog {
            if keys.contains(&tok.as_str()) {
                if !themes.iter().any(|t| t.key == spec.key) {
                    themes.push(spec.clone());
                }
                matched_token_flags[idx] = true;
            }
        }
        for &(m_tok, m) in mood_tokens {
            if tok == m_tok {
                if !moods.contains(&m) {
                    moods.push(m);
                }
                matched_token_flags[idx] = true;
            }
        }
        for &(g_tok, g_lbl) in genre_map {
            if tok == g_tok {
                if !genres.iter().any(|g| g == g_lbl) {
                    genres.push(g_lbl.to_string());
                }
                matched_token_flags[idx] = true;
            }
        }
    }

    // Check two-word phrases like "hip hop", "new wave", "synth pop", "late night"
    for &(phrase, g_lbl) in genre_map {
        if phrase.contains(' ') && clean_norm.contains(phrase) {
            if !genres.iter().any(|g| g == g_lbl) {
                genres.push(g_lbl.to_string());
            }
            for (idx, tok) in tokens.iter().enumerate() {
                if phrase.split_whitespace().any(|p| p == tok) {
                    matched_token_flags[idx] = true;
                }
            }
        }
    }
    if clean_norm.contains("late night") || clean_norm.contains("feel good") || clean_norm.contains("road trip") {
        for (idx, tok) in tokens.iter().enumerate() {
            if matches!(tok.as_str(), "late" | "night" | "feel" | "good" | "road" | "trip") {
                matched_token_flags[idx] = true;
            }
        }
    }

    // Add default mood hints from themes when helpful
    for th in &themes {
        match th.key {
            "workout" if !moods.contains(&Mood::Energy) => moods.push(Mood::Energy),
            "focus" if !moods.contains(&Mood::Focus) => moods.push(Mood::Focus),
            "sleep" if !moods.contains(&Mood::Chill) => moods.push(Mood::Chill),
            _ => {}
        }
    }

    if decade_label.is_some() && !clean_norm.contains("deep") && !clean_norm.contains("obscure") {
        wants_hits = true;
    }

    let unknown_tokens = matched_token_flags.iter().filter(|&&m| !m).count();
    let has_cue = decade_label.is_some()
        || !themes.is_empty()
        || !genres.is_empty()
        || !moods.is_empty()
        || wants_classic
        || wants_hits
        || explicit_mix_word;
    let is_mix_query = has_cue && !tokens.is_empty() && (unknown_tokens == 0 || explicit_mix_word);
    let is_theme_mix = has_cue && !tokens.is_empty() && unknown_tokens == 0;
    if !is_mix_query {
        // Avoid single-word genre/mood/theme collisions inside artist/track names (e.g. "Daft Punk", "Pop Smoke").
        genres.clear();
        moods.clear();
        themes.clear();
    }

    SearchIntent {
        raw: trimmed.to_string(),
        normalized,
        tokens,
        decade_label,
        year_range,
        wants_hits,
        wants_classic,
        moods,
        genres,
        themes,
        is_mix_query,
        is_theme_mix,
    }
}

fn parse_decade_token(tok: &str) -> Option<(&'static str, i32, i32)> {
    match tok {
        "50s" | "1950s" | "fifties" => Some(("50s", 1950, 1959)),
        "60s" | "1960s" | "sixties" => Some(("60s", 1960, 1969)),
        "70s" | "1970s" | "seventies" => Some(("70s", 1970, 1979)),
        "80s" | "1980s" | "eighties" => Some(("80s", 1980, 1989)),
        "90s" | "1990s" | "nineties" => Some(("90s", 1990, 1999)),
        "00s" | "2000s" => Some(("2000s", 2000, 2009)),
        "10s" | "2010s" => Some(("2010s", 2010, 2019)),
        "20s" | "2020s" => Some(("2020s", 2020, 2029)),
        _ => None,
    }
}

fn compute_query_era_match(t: &Track, intent: &SearchIntent) -> f64 {
    if let Some((min_y, max_y)) = intent.year_range {
        if t.year > 1920 {
            if t.year >= min_y && t.year <= max_y {
                return 1.0;
            }
            let dist = if t.year < min_y {
                min_y - t.year
            } else {
                t.year - max_y
            };
            if dist <= 3 {
                return 0.35;
            }
            if dist <= 8 {
                return -0.45;
            }
            return -1.0;
        }
        return 0.0;
    }

    if intent.wants_classic && t.year > 1920 {
        if t.year <= 1985 {
            return 0.85;
        }
        if t.year <= 1996 {
            return 0.4;
        }
        if t.year >= 2015 {
            return -0.4;
        }
    }
    0.0
}

fn compute_query_theme_match(t: &Track, intent: &SearchIntent) -> f64 {
    let hay = format!("{} {} {} {}", t.title, t.album, t.genre, t.moods).to_lowercase();
    let mut score = 0.0f64;

    if !intent.themes.is_empty() {
        let mut hits = 0usize;
        for th in &intent.themes {
            hits += th.keywords.iter().filter(|kw| hay.contains(**kw)).count();
        }
        if hits > 0 {
            score += (0.55 + (hits as f64 - 1.0) * 0.15).min(0.95);
        }
    }

    if !intent.genres.is_empty() {
        let g_low = t.genre.to_lowercase();
        let g_hits = intent
            .genres
            .iter()
            .filter(|g| {
                let gl = g.to_lowercase();
                g_low.contains(&gl) || hay.contains(&gl)
            })
            .count();
        if g_hits > 0 {
            score += (0.45 * g_hits as f64).min(0.75);
        }
    }

    if !intent.moods.is_empty() {
        let tm = t.mood_list();
        let m_hits = intent.moods.iter().filter(|m| tm.contains(m)).count();
        if m_hits > 0 {
            score += (0.35 * m_hits as f64).min(0.6);
        }
    }

    score.clamp(0.0, 1.0)
}

fn is_junk_track_for_query(t: &Track, query_low: &str) -> bool {
    let hay = format!("{} {} {}", t.title, t.artist, t.album).to_lowercase();
    for junk in [
        "karaoke",
        "parody",
        "tribute",
        "made famous by",
        "ringtone",
        "8-bit",
        "8 bit",
        "lullaby rendition",
        "music box",
    ] {
        if hay.contains(junk) && !query_low.contains(junk) {
            return true;
        }
    }
    false
}

fn build_instant_mix_title(
    intent: &SearchIntent,
    tracks: &[Track],
    net: &UserNet,
) -> (String, String) {
    if intent.is_theme_mix {
        let mut parts: Vec<String> = Vec::new();
        if intent.wants_classic {
            parts.push("Classic".to_string());
        }
        if let Some(dec) = intent.decade_label {
            parts.push(dec.to_string());
        }
        for th in intent.themes.iter().take(1) {
            parts.push(th.label.to_string());
        }
        for g in intent.genres.iter().take(2) {
            if !parts.iter().any(|p| p.eq_ignore_ascii_case(g)) {
                parts.push(g.clone());
            }
        }
        if intent.themes.is_empty() && intent.genres.is_empty() {
            if let Some(m) = intent.moods.first() {
                parts.push(m.label().to_string());
            } else if intent.wants_hits || intent.decade_label.is_some() {
                parts.push("Hits".to_string());
            }
        } else if intent.genres.is_empty() && !intent.wants_classic && intent.wants_hits {
            parts.push("Hits".to_string());
        } else if intent.themes.len() == 1 && !intent.wants_classic && intent.genres.is_empty() {
            parts.push("Hits".to_string());
        }

        if parts.is_empty() {
            parts.push(title_case_query(&intent.raw));
        }
        let title = format!("{} Mix", parts.join(" "));

        // Describe dominant vibe & neural personalization in subtitle.
        let mut genre_counts: HashMap<String, usize> = HashMap::new();
        for t in tracks {
            for g in split_genres(&t.genre) {
                *genre_counts.entry(title_case_query(&g)).or_insert(0) += 1;
            }
        }
        let mut top_genres: Vec<(String, usize)> = genre_counts.into_iter().collect();
        top_genres.sort_by(|a, b| b.1.cmp(&a.1));
        let vibe_hint = if !top_genres.is_empty() {
            let names: Vec<String> = top_genres.into_iter().take(2).map(|(g, _)| g).collect();
            names.join(" & ")
        } else {
            "Curated Classics & Hits".to_string()
        };
        let subtitle = if net.updates >= 4 {
            format!("On-the-fly custom mix · {} · Tuned to your taste", vibe_hint)
        } else {
            format!("On-the-fly custom mix · {}", vibe_hint)
        };
        return (title, subtitle);
    }

    let anchor_artist = tracks
        .first()
        .map(|t| primary_artist(&t.artist))
        .filter(|a| !a.is_empty())
        .unwrap_or_else(|| title_case_query(&intent.raw));
    let title = format!("{} & Similar Mix", anchor_artist);
    let subtitle = if net.updates >= 4 {
        "Instant custom mix built from search results & your taste".to_string()
    } else {
        "Instant custom mix built from top search results".to_string()
    };
    (title, subtitle)
}

fn title_case_query(s: &str) -> String {
    s.split_whitespace()
        .map(|w| {
            let mut ch = w.chars();
            match ch.next() {
                Some(first) => first.to_uppercase().to_string() + &ch.as_str().to_lowercase(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn build_search_refinements(
    intent: &SearchIntent,
    tracks: &[Track],
    taste_match_ids: &HashSet<String>,
    sctx: &ScoringContext,
) -> SearchRefinements {
    let mut filter_chips = Vec::new();
    let taste_count = tracks
        .iter()
        .filter(|t| taste_match_ids.contains(&t.id))
        .count();
    if taste_count > 0 {
        filter_chips.push(RefinementChip {
            key: "taste".into(),
            label: "For Your Taste".into(),
            kind: "mode".into(),
            count: taste_count,
        });
    }
    let hits_count = tracks
        .iter()
        .filter(|t| parse_rank_bonus(&t.meta) >= 1.5)
        .count();
    if hits_count > 0 {
        filter_chips.push(RefinementChip {
            key: "hits".into(),
            label: "Popular Hits".into(),
            kind: "mode".into(),
            count: hits_count,
        });
    }
    let lossless_count = tracks
        .iter()
        .filter(|t| t.is_lossless() || t.source == "local")
        .count();
    if lossless_count > 0 {
        filter_chips.push(RefinementChip {
            key: "lossless".into(),
            label: "Lossless / Local".into(),
            kind: "mode".into(),
            count: lossless_count,
        });
    }

    let mut mood_counts: HashMap<Mood, usize> = HashMap::new();
    for t in tracks {
        for m in t.mood_list() {
            *mood_counts.entry(m).or_insert(0) += 1;
        }
    }
    let mut top_moods: Vec<(Mood, usize)> = mood_counts.into_iter().collect();
    top_moods.sort_by(|a, b| {
        let aff_a = sctx.prof.mood_affinity.get(&a.0).copied().unwrap_or(0.0);
        let aff_b = sctx.prof.mood_affinity.get(&b.0).copied().unwrap_or(0.0);
        (b.1 as f64 + aff_b * 2.0)
            .partial_cmp(&(a.1 as f64 + aff_a * 2.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for (m, c) in top_moods.into_iter().take(3) {
        if c >= 2 {
            filter_chips.push(RefinementChip {
                key: format!("mood:{}", m.slug()),
                label: m.label().to_string(),
                kind: "mood".into(),
                count: c,
            });
        }
    }

    // Suggest refined sub-queries ordered by user's genre affinities.
    let mut suggestions = Vec::new();
    if let Some(dec) = intent.decade_label {
        let mut sub_genres = vec!["Pop", "Rock", "Synthpop", "R&B", "Hip-Hop", "Dance"];
        sub_genres.sort_by(|a, b| {
            let fa = sctx.prof.genre_affinity.get(&a.to_lowercase()).copied().unwrap_or(0.0);
            let fb = sctx.prof.genre_affinity.get(&b.to_lowercase()).copied().unwrap_or(0.0);
            fb.partial_cmp(&fa).unwrap_or(std::cmp::Ordering::Equal)
        });
        for g in sub_genres.into_iter().take(4) {
            let cand = format!("{} {}", dec, g);
            if !cand.eq_ignore_ascii_case(&intent.raw) {
                suggestions.push(cand);
            }
        }
    } else if intent.themes.iter().any(|t| t.key == "christmas") {
        for cand in ["Classic Christmas", "Christmas Jazz", "Christmas Pop", "Upbeat Christmas"] {
            if !cand.eq_ignore_ascii_case(&intent.raw) {
                suggestions.push(cand.to_string());
            }
        }
    } else if !tracks.is_empty() && !intent.is_theme_mix {
        let mut g_counts: HashMap<String, usize> = HashMap::new();
        for t in tracks.iter().take(15) {
            for g in split_genres(&t.genre) {
                *g_counts.entry(title_case_query(&g)).or_insert(0) += 1;
            }
        }
        let mut sorted_g: Vec<(String, usize)> = g_counts.into_iter().collect();
        sorted_g.sort_by(|a, b| b.1.cmp(&a.1));
        for (g, _) in sorted_g.into_iter().take(2) {
            let cand = format!("{} {} mix", intent.raw, g);
            suggestions.push(cand);
        }
    }

    let detected_intent = if intent.is_theme_mix {
        if let Some(dec) = intent.decade_label {
            format!("{} Era Mix", dec)
        } else if let Some(th) = intent.themes.first() {
            format!("{} Theme Mix", th.label)
        } else {
            "Vibe Mix".to_string()
        }
    } else if intent.is_mix_query {
        "Blended Search".to_string()
    } else {
        "Neural Ranked".to_string()
    };

    SearchRefinements {
        detected_intent,
        filter_chips,
        query_suggestions: suggestions,
    }
}

fn is_strongly_disliked(t: &Track, prof: &TasteProfile, stats: &HashMap<String, UserTrackStats>) -> bool {
    if prof.ratings.get(&t.id).copied() == Some(1) {
        return true;
    }
    let st = stats.get(&t.id).copied().unwrap_or_default();
    st.skip_count >= st.play_count + 2 && !prof.starred.contains(&t.id) && !prof.manual_playlist_ids.contains(&t.id)
}

fn score_track_core(
    user_id: i64,
    t: &Track,
    prof: &TasteProfile,
    stats: &HashMap<String, UserTrackStats>,
    net: Option<&UserNet>,
    mode: ScoreMode,
) -> f64 {
    let st = stats.get(&t.id).copied().unwrap_or_default();
    let fctx = build_feature_context(t, prof, &st);

    let net_score = match net {
        Some(n) if user_id > 0 && n.updates > 0 => {
            let conf = n.confidence().max(if n.updates >= 1 { 0.2 } else { 0.0 });
            let x = extract_features_ctx(t, &fctx);
            (n.predict(&x) - 0.5) * 9.5 * conf
        }
        _ => 0.0,
    };

    let a_aff = fctx.artist_affinity;
    let g_aff = fctx.genre_affinity;
    let m_aff = fctx.mood_affinity;
    let pl_aff = fctx.playlist_artist_affinity * 0.8
        + fctx.playlist_genre_affinity * 0.7
        + fctx.playlist_mood_affinity * 0.5;

    let pop_bonus = parse_rank_bonus(&t.meta);
    let quality_bonus = (t.quality_score() as f64) / 400.0;
    let follow_bonus = if fctx.followed_artist { 1.6 } else { 0.0 };
    let album_bonus = if fctx.saved_album { 1.1 } else { 0.0 };

    let identity_bonus = match mode {
        ScoreMode::Supermix => {
            let star = if prof.starred.contains(&t.id) { 3.2 } else { 0.0 };
            let pl = (*prof.playlist_track_counts.get(&t.id).unwrap_or(&0) as f64).min(3.0) * 1.4;
            let rating = prof
                .ratings
                .get(&t.id)
                .map(|r| (*r as f64 - 2.5) * 1.5)
                .unwrap_or(0.0);
            star + pl + rating
        }
        ScoreMode::Discover | ScoreMode::PlaylistDna => 0.0,
    };

    let now = Utc::now().timestamp();
    let recency_penalty = match mode {
        ScoreMode::Supermix if st.last_played > 0 && (now - st.last_played) < 3600 * 6 => -4.5,
        _ => 0.0,
    };
    let skip_penalty = (st.skip_count as f64) * 1.8;

    let raw = net_score
        + pop_bonus
        + a_aff * 1.05
        + g_aff * 0.85
        + m_aff * 0.55
        + pl_aff * 0.45
        + follow_bonus
        + album_bonus
        + identity_bonus
        + quality_bonus
        + recency_penalty
        - skip_penalty;

    if raw.is_finite() {
        raw
    } else {
        0.0
    }
}

fn playlist_query_hint(name: &str, comment: &str) -> String {
    let stop: HashSet<&str> = [
        "my", "the", "a", "an", "playlist", "mix", "songs", "tracks", "music", "vibes", "new",
    ]
    .into_iter()
    .collect();
    let combined = format!("{} {}", name, comment);
    combined
        .split(|c: char| !c.is_alphanumeric() && c != '-' && c != '&')
        .map(str::trim)
        .filter(|w| w.len() >= 3 && !stop.contains(&w.to_lowercase().as_str()))
        .take(5)
        .collect::<Vec<_>>()
        .join(" ")
}

fn build_playlist_centroid(tracks: &[Track], query_hint: &str) -> PlaylistCentroid {
    let mut c = PlaylistCentroid::default();
    let mut year_sum = 0.0;
    let mut year_n = 0.0;
    let mut dur_sum = 0.0;
    let mut dur_n = 0.0;

    for t in tracks {
        c.existing_ids.insert(t.id.clone());
        c.existing_keys.insert(t.song_key());

        let ak = slug(&primary_artist(&t.artist));
        if !ak.is_empty() {
            *c.artists.entry(ak).or_insert(0.0) += 1.0;
        }
        for g in split_genres(&t.genre) {
            *c.genres.entry(g).or_insert(0.0) += 1.0;
        }
        for m in t.mood_list() {
            *c.moods.entry(m).or_insert(0.0) += 1.0;
        }
        if t.year > 1920 {
            year_sum += t.year as f64;
            year_n += 1.0;
        }
        if t.duration > 30 {
            dur_sum += t.duration as f64;
            dur_n += 1.0;
        }
    }

    let n = (tracks.len().max(1)) as f64;
    for v in c.artists.values_mut() {
        *v /= n;
    }
    for v in c.genres.values_mut() {
        *v /= n;
    }
    for v in c.moods.values_mut() {
        *v /= n;
    }
    if year_n > 0.0 {
        c.mean_year = Some(year_sum / year_n);
    }
    if dur_n > 0.0 {
        c.mean_duration = Some(dur_sum / dur_n);
    }
    c.keywords = query_hint
        .split_whitespace()
        .map(|w| w.to_lowercase())
        .filter(|w| w.len() >= 3)
        .collect();
    c
}

fn score_against_centroid(t: &Track, c: &PlaylistCentroid) -> f64 {
    let mut score = 0.0;

    let ak = slug(&primary_artist(&t.artist));
    if let Some(&aw) = c.artists.get(&ak) {
        score += 3.0 * aw.sqrt().min(1.0);
    }

    let genres = split_genres(&t.genre);
    if !genres.is_empty() && !c.genres.is_empty() {
        let mut g_match = 0.0;
        for g in &genres {
            if let Some(&w) = c.genres.get(g) {
                g_match += w;
            }
        }
        score += 2.6 * g_match.min(1.5);
    }

    let moods = t.mood_list();
    if !moods.is_empty() && !c.moods.is_empty() {
        let mut m_match = 0.0;
        for m in &moods {
            if let Some(&w) = c.moods.get(m) {
                m_match += w;
            }
        }
        score += 1.8 * m_match.min(1.5);
    }

    if let (Some(my), true) = (c.mean_year, t.year > 1920) {
        let diff = (t.year as f64 - my).abs();
        score += (1.0 - (diff / 25.0).min(1.0)) * 0.75;
    }

    if let (Some(md), true) = (c.mean_duration, t.duration > 30) {
        let diff = (t.duration as f64 - md).abs();
        score += (1.0 - (diff / 240.0).min(1.0)) * 0.35;
    }

    if !c.keywords.is_empty() {
        let hay = format!("{} {} {} {}", t.title, t.artist, t.genre, t.moods).to_lowercase();
        let hits = c.keywords.iter().filter(|kw| hay.contains(kw.as_str())).count();
        score += (hits as f64) * 1.4;
    }

    score
}

/// Deduplicates tracks by `song_key()`, limits tracks per primary artist to `max_per_artist`,
/// and interleaves artists using a lookahead cooldown window so identical artists never cluster back-to-back.
fn dedup_and_interleave(tracks: Vec<Track>, limit: usize, max_per_artist: usize) -> Vec<Track> {
    if limit == 0 || max_per_artist == 0 {
        return vec![];
    }

    let mut seen_keys = HashSet::new();
    let mut pool: Vec<(String, Track)> = Vec::with_capacity(tracks.len());
    for t in tracks {
        if seen_keys.insert(t.song_key()) {
            let ak = slug(&primary_artist(&t.artist));
            pool.push((ak, t));
        }
    }

    let cooldown = if limit <= 10 { 2 } else { 3 };
    let mut artist_counts: HashMap<String, usize> = HashMap::new();
    let mut recent_artists: Vec<String> = Vec::with_capacity(limit);
    let mut out = Vec::with_capacity(limit.min(pool.len()));

    while out.len() < limit && !pool.is_empty() {
        let window_start = recent_artists.len().saturating_sub(cooldown);
        let window = &recent_artists[window_start..];

        // Prefer top-ranked track respecting both max_per_artist and cooldown spacing.
        let best_spaced = pool
            .iter()
            .take(24)
            .position(|(ak, _)| {
                artist_counts.get(ak).copied().unwrap_or(0) < max_per_artist
                    && !window.contains(ak)
            });

        let chosen_idx = best_spaced.or_else(|| {
            pool.iter()
                .position(|(ak, _)| artist_counts.get(ak).copied().unwrap_or(0) < max_per_artist)
        });

        let Some(idx) = chosen_idx else {
            break;
        };

        let (ak, track) = pool.remove(idx);
        *artist_counts.entry(ak.clone()).or_insert(0) += 1;
        recent_artists.push(ak);
        out.push(track);
    }

    out
}

fn merge_tracks_dedup(primary: Vec<Track>, secondary: Vec<Track>) -> Vec<Track> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for t in primary.into_iter().chain(secondary) {
        if seen.insert(t.song_key()) {
            out.push(t);
        }
    }
    out
}

fn parse_rank_bonus(meta: &str) -> f64 {
    if meta.is_empty() {
        return 0.0;
    }
    let rank = serde_json::from_str::<serde_json::Value>(meta)
        .ok()
        .and_then(|v| v.get("rank").and_then(|r| r.as_u64()))
        .unwrap_or(0);
    if rank == 0 {
        0.0
    } else {
        ((rank as f64) / 200_000.0).clamp(0.2, 5.0)
    }
}

fn merge_albums_dedup(primary: Vec<AlbumSummary>, secondary: Vec<AlbumSummary>) -> Vec<AlbumSummary> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for a in primary.into_iter().chain(secondary) {
        if seen.insert(a.id.clone()) {
            out.push(a);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn cold_start_scoring_favors_popular_rank_over_random_untrained_noise() {
        let store = Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("cold", "pw").unwrap();
        let sources = SourceRegistry::new(
            crate::sources::deezer::DeezerSource::new(),
            crate::sources::ytm::YtmSource::new(None),
            crate::sources::archive::ArchiveSource::new(),
            Arc::new(crate::sources::local::LocalSource::new(store.clone(), vec![])),
            crate::sources::soulseek::SoulseekSource::new(
                store.clone(),
                std::env::temp_dir(),
            ),
        );
        let taste = TasteEngine::new(store.clone(), sources);

        let hit = Track {
            id: "dz:hit".into(),
            source: "deezer".into(),
            title: "Dracula".into(),
            artist: "Tame Impala".into(),
            meta: r#"{"rank":980000}"#.into(),
            bitrate: 256,
            ..Default::default()
        };
        let obscure = Track {
            id: "dz:obscure".into(),
            source: "deezer".into(),
            title: "Random Deep Cut".into(),
            artist: "Obscure Artist".into(),
            meta: "{}".into(),
            bitrate: 256,
            ..Default::default()
        };
        store.upsert_tracks(&[obscure.clone(), hit.clone()]).unwrap();
        let picks = taste.quick_picks(u.id, 5);
        assert_eq!(picks[0].id, "dz:hit");
    }

    #[test]
    fn radio_catalog_supplement_excludes_unrelated_noise_when_radio_already_present() {
        let store = Store::open_memory().unwrap();
        let sources = SourceRegistry::new(
            crate::sources::deezer::DeezerSource::new(),
            crate::sources::ytm::YtmSource::new(None),
            crate::sources::archive::ArchiveSource::new(),
            Arc::new(crate::sources::local::LocalSource::new(store.clone(), vec![])),
            crate::sources::soulseek::SoulseekSource::new(store.clone(), std::env::temp_dir()),
        );
        let taste = TasteEngine::new(store.clone(), sources);

        let seed = Track {
            id: "ytm:R-maTg10QKs".into(),
            source: "ytm".into(),
            title: "ソラゴト - Fake".into(),
            artist: "ASU".into(),
            moods: "late_night".into(),
            ..Default::default()
        };
        let radio_item = Track {
            id: "ytm:1nyZA4kqyYM".into(),
            source: "ytm".into(),
            title: "Spirits of the Sea".into(),
            artist: "ASU".into(),
            ..Default::default()
        };
        let unrelated = Track {
            id: "dz:unrelated".into(),
            source: "deezer".into(),
            title: "Fantasy".into(),
            artist: "Mariah Carey".into(),
            genre: "Pop".into(),
            bitrate: 1411,
            format: "flac".into(),
            ..Default::default()
        };
        let related = Track {
            id: "ytm:related".into(),
            source: "ytm".into(),
            title: "Bitter Etude".into(),
            artist: "ASU".into(),
            ..Default::default()
        };
        store
            .upsert_tracks(&[seed.clone(), radio_item.clone(), unrelated, related.clone()])
            .unwrap();

        let mut seen: HashSet<String> = HashSet::from([seed.song_key(), radio_item.song_key()]);
        let mut out = vec![radio_item];
        taste.supplement_radio_from_catalog(&seed, 10, &mut seen, &mut out);

        assert!(out.iter().any(|t| t.id == "ytm:related"));
        assert!(!out.iter().any(|t| t.id == "dz:unrelated"));
    }

    #[test]
    fn dedup_and_interleave_spaces_repeated_artists_apart() {
        let mk = |id: &str, artist: &str| Track {
            id: id.into(),
            title: id.into(),
            artist: artist.into(),
            ..Default::default()
        };
        let input = vec![
            mk("a1", "Alpha"),
            mk("a2", "Alpha"),
            mk("b1", "Beta"),
            mk("c1", "Gamma"),
            mk("a3", "Alpha"),
        ];
        let out = dedup_and_interleave(input, 4, 2);
        let artists: Vec<&str> = out.iter().map(|t| t.artist.as_str()).collect();
        assert_eq!(artists, vec!["Alpha", "Beta", "Gamma", "Alpha"]);
    }

    #[test]
    fn playlists_drive_learning_dna_mix_and_discover_exclusions() {
        let store = Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("curator", "pw").unwrap();
        let sources = SourceRegistry::new(
            crate::sources::deezer::DeezerSource::new(),
            crate::sources::ytm::YtmSource::new(None),
            crate::sources::archive::ArchiveSource::new(),
            Arc::new(crate::sources::local::LocalSource::new(store.clone(), vec![])),
            crate::sources::soulseek::SoulseekSource::new(store.clone(), std::env::temp_dir()),
        );
        let taste = TasteEngine::new(store.clone(), sources);

        let mk = |id: &str, title: &str, artist: &str, genre: &str, moods: &str| Track {
            id: id.into(),
            source: "deezer".into(),
            title: title.into(),
            artist: artist.into(),
            album: format!("{} Album", artist),
            genre: genre.into(),
            moods: moods.into(),
            year: 2022,
            bitrate: 320,
            ..Default::default()
        };

        let in_pl_1 = mk("s1", "Midnight Pulse", "Synth Hero", "Electronic", "late_night,energy");
        let in_pl_2 = mk("s2", "Neon Horizon", "Cyber Muse", "Electronic", "late_night");
        let fresh_match = mk("s3", "Starlight Grid", "Cyber Muse", "Electronic", "late_night");
        let hated = mk("s4", "Harsh Static", "Cyber Muse", "Electronic", "late_night");
        let unrelated = mk("s5", "Country Road", "Banjo Sam", "Country", "acoustic");
        store
            .upsert_tracks(&[
                in_pl_1.clone(),
                in_pl_2.clone(),
                fresh_match.clone(),
                hated.clone(),
                unrelated.clone(),
            ])
            .unwrap();

        let pl = store
            .create_playlist(u.id, "Late Night Synth", "", false, &[in_pl_1.id.clone(), in_pl_2.id.clone()])
            .unwrap();
        taste.record_playlist_additions(u.id, &[in_pl_1.id.clone(), in_pl_2.id.clone()]);
        store.set_rating(u.id, &hated.id, 1).unwrap();
        taste.record_event(u.id, &hated.id, "rate", 1.0);

        let prof = taste.user_profile(u.id);
        assert!(prof.playlist_artist_affinity.get("synth-hero").copied().unwrap_or(0.0) > 0.5);
        assert!(prof.playlist_genre_affinity.get("electronic").copied().unwrap_or(0.0) > 0.5);

        let discover = taste.discover_weekly(u.id, 10);
        let discover_ids: Vec<&str> = discover.iter().map(|t| t.id.as_str()).collect();
        // Discover must suggest matching fresh track, exclude tracks already in playlist, and skip 1-star tracks
        assert!(discover_ids.contains(&"s3"), "discover={:?}", discover_ids);
        assert!(!discover_ids.contains(&"s1"));
        assert!(!discover_ids.contains(&"s2"));

        taste.rebuild_auto_playlists(u.id);
        let dna = store.get_playlist(&format!("auto-u{}-playlist-dna", u.id)).unwrap();
        assert!(dna.is_some());
        assert!(store.get_playlist(&pl.id).unwrap().is_some());
        assert!(taste.for_you_tracks(u.id, 10).iter().all(|t| t.id != "s4"));
    }

    #[test]
    fn playlist_centroid_recommends_coherent_unadded_tracks() {
        let store = Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("jazzer", "pw").unwrap();
        let seed_a = Track {
            id: "j1".into(),
            source: "local".into(),
            title: "Blue Train".into(),
            artist: "John Coltrane".into(),
            genre: "Jazz".into(),
            moods: "chill,late_night".into(),
            year: 1958,
            duration: 420,
            bitrate: 1411,
            format: "flac".into(),
            ..Default::default()
        };
        let match_b = Track {
            id: "j2".into(),
            source: "local".into(),
            title: "So What".into(),
            artist: "Miles Davis".into(),
            genre: "Jazz".into(),
            moods: "chill,late_night".into(),
            year: 1959,
            duration: 540,
            bitrate: 1411,
            format: "flac".into(),
            ..Default::default()
        };
        let mismatch = Track {
            id: "m1".into(),
            source: "local".into(),
            title: "Sludge Pit".into(),
            artist: "Doom Hammer".into(),
            genre: "Metal".into(),
            moods: "energy".into(),
            year: 2021,
            duration: 180,
            bitrate: 1411,
            format: "flac".into(),
            ..Default::default()
        };
        store
            .upsert_tracks(&[seed_a.clone(), match_b.clone(), mismatch])
            .unwrap();

        let pl_tracks = vec![seed_a.clone()];
        let centroid = build_playlist_centroid(&pl_tracks, "Late Night Jazz");
        assert!(!centroid.existing_ids.contains(&match_b.id));
        let sim_match = score_against_centroid(&match_b, &centroid);
        let sim_mismatch = score_against_centroid(&Track {
            id: "m1".into(),
            genre: "Metal".into(),
            moods: "energy".into(),
            ..Default::default()
        }, &centroid);
        assert!(sim_match > sim_mismatch + 2.0, "match={} mismatch={}", sim_match, sim_mismatch);

        // Ensure store lookup & playlist creation work end to end
        let pl = store.create_playlist(u.id, "Late Night Jazz", "", false, &[seed_a.id]).unwrap();
        let stored_tracks = store.playlist_tracks(&pl.id).unwrap();
        assert_eq!(stored_tracks.len(), 1);
    }

    #[test]
    fn parse_search_intent_detects_eras_themes_and_entities() {
        let eighties = parse_search_intent("80s");
        assert_eq!(eighties.decade_label, Some("80s"));
        assert_eq!(eighties.year_range, Some((1980, 1989)));
        assert!(eighties.wants_hits);
        assert!(eighties.is_theme_mix);

        let xmas = parse_search_intent("classic christmas");
        assert!(xmas.wants_classic);
        assert!(xmas.themes.iter().any(|t| t.key == "christmas"));
        assert!(xmas.is_theme_mix);

        let pop_smoke = parse_search_intent("Pop Smoke");
        assert!(!pop_smoke.is_theme_mix);

        let daft = parse_search_intent("Daft Punk");
        assert!(!daft.is_mix_query);
        assert!(!daft.is_theme_mix);
    }

    #[test]
    fn instant_mix_builds_named_80s_and_christmas_mixes_without_saving_playlists() {
        let store = Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("mixuser", "pw").unwrap();
        let sources = SourceRegistry::new(
            crate::sources::deezer::DeezerSource::new(),
            crate::sources::ytm::YtmSource::new(None),
            crate::sources::archive::ArchiveSource::new(),
            Arc::new(crate::sources::local::LocalSource::new(store.clone(), vec![])),
            crate::sources::soulseek::SoulseekSource::new(store.clone(), std::env::temp_dir()),
        );
        let taste = TasteEngine::new(store.clone(), sources);

        let mk_track = |id: &str, title: &str, artist: &str, genre: &str, year: i32, rank: u64| Track {
            id: id.into(),
            source: "deezer".into(),
            title: title.into(),
            artist: artist.into(),
            album: format!("{} Album", title),
            genre: genre.into(),
            moods: "upbeat".into(),
            year,
            bitrate: 320,
            meta: format!(r#"{{"rank":{}}}"#, rank),
            ..Default::default()
        };

        let a = mk_track("t80_1", "Take On Me", "a-ha", "Synthpop", 1985, 950000);
        let b = mk_track("t80_2", "Billie Jean", "Michael Jackson", "Pop", 1982, 990000);
        let c = mk_track("t80_3", "Sweet Dreams", "Eurythmics", "Synthpop", 1983, 910000);
        let modern_trap = mk_track("t22", "80s Baby", "Modern Artist", "Pop", 2022, 120000);
        let karaoke = mk_track("t_kar", "Take On Me (Karaoke Version)", "Karaoke Pros", "Pop", 2018, 50000);

        store
            .upsert_tracks(&[a.clone(), b.clone(), c.clone(), modern_trap.clone(), karaoke.clone()])
            .unwrap();

        let res_80s = taste.search_and_refine(
            u.id,
            "80s",
            vec![modern_trap.clone(), karaoke.clone()],
            vec![("80s Hits".to_string(), vec![a.clone(), b.clone(), c.clone()])],
        );
        let mix = res_80s.instant_mix.expect("80s instant mix should be built");
        assert_eq!(mix.title, "80s Hits Mix");
        assert!(mix.is_theme_mix);
        let mix_ids: Vec<&str> = mix.tracks.iter().map(|t| t.id.as_str()).collect();
        assert!(mix_ids.contains(&"t80_1"));
        assert!(mix_ids.contains(&"t80_2"));
        assert!(!mix_ids.contains(&"t22"));
        assert!(!mix_ids.contains(&"t_kar"));

        // Must NOT save any playlist into SQLite!
        assert!(store.list_playlists(u.id, false).unwrap().is_empty());

        // Now test "classic christmas"
        let x1 = mk_track("x1", "White Christmas", "Bing Crosby", "Vocal", 1942, 940000);
        let x2 = mk_track("x2", "Jingle Bell Rock", "Bobby Helms", "Rock", 1957, 920000);
        let x3 = mk_track("x3", "Last Christmas", "Wham!", "Pop", 1984, 980000);
        store.upsert_tracks(&[x1.clone(), x2.clone(), x3.clone()]).unwrap();

        let res_xmas = taste.search_and_refine(
            u.id,
            "classic christmas",
            vec![x1, x2, x3, modern_trap],
            vec![],
        );
        let xmas_mix = res_xmas.instant_mix.expect("christmas mix should be built");
        assert_eq!(xmas_mix.title, "Classic Christmas Mix");
        assert_eq!(xmas_mix.tracks.len(), 3);
        assert!(store.list_playlists(u.id, false).unwrap().is_empty());
    }

    #[test]
    fn neural_network_reranks_search_results_while_pinning_exact_matches() {
        let store = Store::open_memory().unwrap();
        let (u, _) = store.authenticate_or_create_user("searchlearner", "pw").unwrap();
        let sources = SourceRegistry::new(
            crate::sources::deezer::DeezerSource::new(),
            crate::sources::ytm::YtmSource::new(None),
            crate::sources::archive::ArchiveSource::new(),
            Arc::new(crate::sources::local::LocalSource::new(store.clone(), vec![])),
            crate::sources::soulseek::SoulseekSource::new(store.clone(), std::env::temp_dir()),
        );
        let taste = TasteEngine::new(store.clone(), sources);

        let exact_track = Track {
            id: "exact".into(),
            source: "deezer".into(),
            title: "Midnight".into(),
            artist: "Coldplay".into(),
            genre: "Rock".into(),
            moods: "melancholy".into(),
            year: 2014,
            bitrate: 256,
            ..Default::default()
        };
        let synth_match = Track {
            id: "synth".into(),
            source: "deezer".into(),
            title: "Midnight City".into(),
            artist: "M83".into(),
            genre: "Electronic, Synthpop".into(),
            moods: "late_night,energy".into(),
            year: 2011,
            bitrate: 256,
            meta: r#"{"rank":500000}"#.into(),
            ..Default::default()
        };
        let jazz_match = Track {
            id: "jazz".into(),
            source: "deezer".into(),
            title: "Midnight Blue".into(),
            artist: "Kenny Burrell".into(),
            genre: "Jazz".into(),
            moods: "chill,late_night".into(),
            year: 1963,
            bitrate: 256,
            meta: r#"{"rank":500000}"#.into(),
            ..Default::default()
        };
        store
            .upsert_tracks(&[exact_track.clone(), synth_match.clone(), jazz_match.clone()])
            .unwrap();

        // Train user heavily toward Electronic/Synthpop search picks and away from Jazz
        for _ in 0..8 {
            taste.record_event_with_query(u.id, &synth_match.id, "search_pick", 1.0, Some("midnight"));
            taste.record_event_with_query(u.id, &jazz_match.id, "skip", 1.0, Some("midnight"));
        }

        let ranked = taste.search_and_refine(
            u.id,
            "midnight",
            vec![jazz_match.clone(), exact_track.clone(), synth_match.clone()],
            vec![],
        );
        let ids: Vec<&str> = ranked.tracks.iter().map(|t| t.id.as_str()).collect();
        // Exact title match "Midnight" stays pinned #1, and Synthpop beats skipped Jazz!
        assert_eq!(ids[0], "exact", "ids={:?}", ids);
        let pos_synth = ids.iter().position(|&id| id == "synth").unwrap();
        let pos_jazz = ids.iter().position(|&id| id == "jazz").unwrap();
        assert!(pos_synth < pos_jazz, "ids={:?}", ids);
        assert!(
            ranked.track_neural_scores["synth"] > ranked.track_neural_scores["jazz"],
            "neural scores={:?}",
            ranked.track_neural_scores
        );
    }
}
