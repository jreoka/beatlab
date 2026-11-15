use crate::models::{
    primary_artist, slug, AlbumSummary, ArtistSummary, Mood, Track, UserTrackStats,
};
use crate::neural::{event_target, extract_features_ctx, FeatureContext, NeuralCache, UserNet};
use crate::sources::SourceRegistry;
use crate::store::Store;
use chrono::Utc;
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
        let safe_val = if value.is_finite() { value } else { 0.0 };
        let _ = self.store.record_event(user_id, track_id, kind, safe_val);
        self.learn_signal(user_id, track_id, kind, safe_val);
    }

    /// Updates neural model and taste staleness for an already-persisted interaction event
    /// (avoids writing duplicate rows into `events` after `star_track`/`set_rating`).
    pub fn learn_signal(&self, user_id: i64, track_id: &str, kind: &str, value: f64) {
        let safe_val = if value.is_finite() { value } else { 0.0 };
        if user_id > 0
            && matches!(
                kind,
                "star" | "unstar" | "rate" | "playlist_add" | "playlist_remove" | "complete" | "skip"
            )
        {
            self.mark_taste_dirty(user_id);
        }
        if let Some((target, weight)) = event_target(kind, safe_val) {
            if let Ok(Some(track)) = self.store.get_track(track_id) {
                let prof = self.user_profile(user_id);
                let st = self
                    .store
                    .get_user_track_stats(user_id, track_id)
                    .unwrap_or_default();
                let fctx = build_feature_context(&track, &prof, &st);
                let feats = extract_features_ctx(&track, &fctx);
                self.neural.update_and_save(&self.store, user_id, |net| {
                    net.train_step(&feats, target, weight);
                });
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
}
