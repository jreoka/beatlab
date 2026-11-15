use crate::models::{
    Mood, Track, core_song_title, is_generic_folder_name, primary_artist, slug,
};
use crate::neural::{NeuralCache, UserNet, event_target, extract_features};
use crate::sources::SourceRegistry;
use crate::store::Store;
use chrono::Utc;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use tracing::info;

#[derive(Debug, Clone, Default)]
pub struct TasteProfile {
    pub artist_score: HashMap<String, f64>,
    pub genre_score: HashMap<String, f64>,
    pub mood_score: HashMap<Mood, f64>,
    pub era_score: HashMap<i32, f64>,
    pub cooccurrence: HashMap<String, HashMap<String, f64>>,
    pub track_plays: HashMap<String, i32>,
    pub track_skips: HashMap<String, i32>,
    pub track_last_played: HashMap<String, i64>,
    pub song_last_played: HashMap<String, i64>,
    pub starred: HashSet<String>,
    pub ratings: HashMap<String, i32>,
}

impl TasteProfile {
    pub fn build(store: &Store, user_id: i64) -> Self {
        let mut p = Self::default();
        let events = store.user_events(user_id, 2000);
        let now = Utc::now().timestamp();

        let unique_ids: Vec<String> = events
            .iter()
            .map(|e| e.track_id.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let track_cache = store.tracks_by_ids(&unique_ids);
        let mut session_artists: Vec<(i64, String)> = Vec::new();

        for ev in &events {
            let days = ((now - ev.at).max(0) as f64) / 86400.0;
            let decay = 0.5_f64.powf(days / 90.0);

            let weight = match ev.kind.as_str() {
                "complete" => {
                    *p.track_plays.entry(ev.track_id.clone()).or_default() += 1;
                    p.track_last_played
                        .entry(ev.track_id.clone())
                        .and_modify(|v| *v = (*v).max(ev.at))
                        .or_insert(ev.at);
                    2.2 * decay
                }
                "play" => {
                    *p.track_plays.entry(ev.track_id.clone()).or_default() += 1;
                    p.track_last_played
                        .entry(ev.track_id.clone())
                        .and_modify(|v| *v = (*v).max(ev.at))
                        .or_insert(ev.at);
                    1.2 * decay
                }
                "skip" => {
                    *p.track_skips.entry(ev.track_id.clone()).or_default() += 1;
                    -1.5 * decay
                }
                "star" => {
                    p.starred.insert(ev.track_id.clone());
                    3.5 * decay
                }
                "unstar" => {
                    p.starred.remove(&ev.track_id);
                    -decay
                }
                "rate" => {
                    let r = ev.value as i32;
                    p.ratings.insert(ev.track_id.clone(), r);
                    ((r as f64) - 2.5) * 1.6 * decay
                }
                _ => 0.0,
            };

            let Some(track) = track_cache.get(&ev.track_id) else {
                continue;
            };

            if track.id.is_empty() {
                continue;
            }

            if matches!(ev.kind.as_str(), "complete" | "play") {
                p.song_last_played
                    .entry(track.song_key())
                    .and_modify(|v| *v = (*v).max(ev.at))
                    .or_insert(ev.at);
            }

            let pa = primary_artist(&track.artist);
            let a_key = pa.to_lowercase();
            if !a_key.is_empty() && a_key != "unknown artist" && !is_generic_folder_name(&pa) {
                *p.artist_score.entry(a_key.clone()).or_default() += weight;
                if weight > 0.0 {
                    session_artists.push((ev.at, a_key));
                }
            }

            if !track.genre.is_empty() {
                *p.genre_score.entry(track.genre.to_lowercase()).or_default() += weight;
            }

            for m in track.mood_list() {
                *p.mood_score.entry(m).or_default() += weight;
            }

            if track.year >= 1940 {
                let decade = (track.year / 10) * 10;
                *p.era_score.entry(decade).or_default() += weight * 0.6;
            }
        }

        // Build artist co-occurrence graph (artists played within 45 minutes of each other)
        for i in 0..session_artists.len() {
            for j in (i + 1)..session_artists.len().min(i + 12) {
                let (t1, ref a1) = session_artists[i];
                let (t2, ref a2) = session_artists[j];
                if a1 != a2 && (t1 - t2).abs() <= 2700 {
                    *p.cooccurrence
                        .entry(a1.clone())
                        .or_default()
                        .entry(a2.clone())
                        .or_default() += 1.0;
                    *p.cooccurrence
                        .entry(a2.clone())
                        .or_default()
                        .entry(a1.clone())
                        .or_default() += 1.0;
                }
            }
        }

        // Also include current stars from stars table
        for st in store.starred_tracks(user_id) {
            p.starred.insert(st.id);
        }

        p
    }

    pub fn score_track(&self, t: &Track) -> f64 {
        let mut score = 0.0;
        let a_key = primary_artist(&t.artist).to_lowercase();
        if let Some(s) = self.artist_score.get(&a_key) {
            score += s * 1.4;
        }
        if !t.genre.is_empty() {
            if let Some(s) = self.genre_score.get(&t.genre.to_lowercase()) {
                score += s * 1.1;
            }
        }
        for m in t.mood_list() {
            if let Some(s) = self.mood_score.get(&m) {
                score += s * 0.7;
            }
        }
        if t.year >= 1940 {
            let decade = (t.year / 10) * 10;
            if let Some(s) = self.era_score.get(&decade) {
                score += s * 0.4;
            }
        }
        if self.starred.contains(&t.id) {
            score += 4.0;
        }
        if let Some(&r) = self.ratings.get(&t.id) {
            score += (r as f64 - 2.5) * 2.0;
        }
        let skips = self.track_skips.get(&t.id).copied().unwrap_or(0);
        if skips > 0 {
            score -= (skips as f64) * 3.0;
        }
        // Quality preference bonus: lossless/high-quality sources get a natural boost
        score += (t.quality_score() as f64) / 400.0;
        score
    }

    /// Personal musical affinity ignoring global quality and generic decade bonuses,
    /// returning `(has_concrete_musical_link, affinity_score)`.
    fn musical_affinity(&self, t: &Track) -> (bool, f64) {
        let a_key = primary_artist(&t.artist).to_lowercase();
        let a_aff = self.artist_score.get(&a_key).copied().unwrap_or(0.0) * 1.4;
        let g_aff = if !t.genre.is_empty() {
            self.genre_score
                .get(&t.genre.to_lowercase())
                .copied()
                .unwrap_or(0.0)
                * 1.1
        } else {
            0.0
        };
        let m_aff: f64 = t
            .mood_list()
            .into_iter()
            .filter_map(|m| self.mood_score.get(&m).copied())
            .sum::<f64>()
            * 0.7;
        let star_aff = if self.starred.contains(&t.id) {
            4.0
        } else {
            0.0
        };
        let rate_aff = self
            .ratings
            .get(&t.id)
            .map(|&r| (r as f64 - 2.5) * 2.0)
            .unwrap_or(0.0);
        let skip_pen = (self.track_skips.get(&t.id).copied().unwrap_or(0) as f64) * 3.0;
        let has_link = a_aff > 0.0 || g_aff > 0.0 || m_aff > 0.0 || star_aff > 0.0 || rate_aff > 0.0;
        (has_link, a_aff + g_aff + m_aff + star_aff + rate_aff - skip_pen)
    }
}

pub struct TasteEngine {
    store: Store,
    registry: Arc<SourceRegistry>,
    warming: AtomicBool,
    neural: NeuralCache,
    auto_pl_last: Mutex<HashMap<i64, (i64, i64)>>,
    popular_ranks: Mutex<HashMap<String, usize>>,
    http: reqwest::Client,
}

impl TasteEngine {
    pub fn new(store: Store, registry: Arc<SourceRegistry>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(3))
            .user_agent("Beatlab/0.2")
            .build()
            .unwrap_or_default();
        Self {
            store,
            registry,
            warming: AtomicBool::new(false),
            neural: NeuralCache::new(),
            auto_pl_last: Mutex::new(HashMap::new()),
            popular_ranks: Mutex::new(HashMap::new()),
            http,
        }
    }

    /// Ensures auto-playlists exist for `user_id` without blocking every `getPlaylists` call.
    /// Generates synchronously if the user has no auto-playlists yet; otherwise refreshes in the
    /// background only when new listening events arrived (with a 30s debounce) or after 30 minutes.
    pub fn ensure_auto_playlists_fresh(self: &Arc<Self>, user_id: i64) {
        let now = Utc::now().timestamp();
        let latest_ev = self.store.latest_user_event_at(user_id);
        let last = self.auto_pl_last.lock().unwrap().get(&user_id).copied();
        if last.is_none() {
            let has_auto = self.store.list_playlists(user_id).iter().any(|p| p.is_auto);
            if !has_auto {
                self.auto_pl_last
                    .lock()
                    .unwrap()
                    .insert(user_id, (now, latest_ev));
                self.generate_auto_playlists(user_id);
                return;
            }
        }
        if let Some((last_gen, last_ev)) = last {
            if now - last_gen < 1800 && latest_ev == last_ev {
                return;
            }
            if now - last_gen < 30 {
                return;
            }
        }
        self.auto_pl_last
            .lock()
            .unwrap()
            .insert(user_id, (now, latest_ev));
        let me = Arc::clone(self);
        tokio::task::spawn_blocking(move || {
            me.generate_auto_playlists(user_id);
        });
    }

    /// Trains the user's personal neural network online from a listening event
    /// and persists the updated weights to SQLite. Also warms Deezer artist similarity
    /// in the background so personal mixes stay cohesive without blocking playback.
    pub fn learn_event(&self, user_id: i64, track: &Track, kind: &str, value: f64) {
        let Some((target, weight)) = event_target(kind, value) else {
            return;
        };
        let clean_artist = primary_artist(&track.artist);
        let prof = self.profile(user_id);
        let a_aff = prof
            .artist_score
            .get(&clean_artist.to_lowercase())
            .copied()
            .unwrap_or(0.0);
        let g_aff = prof
            .genre_score
            .get(&track.genre.to_lowercase())
            .copied()
            .unwrap_or(0.0);
        let plays = prof.track_plays.get(&track.id).copied().unwrap_or(0);
        let skips = prof.track_skips.get(&track.id).copied().unwrap_or(0);
        let feats = extract_features(track, a_aff, g_aff, plays, skips);
        self.neural.update_and_save(&self.store, user_id, |net| {
            net.train_step(&feats, target, weight);
        });

        if weight > 0.0
            && !clean_artist.is_empty()
            && !clean_artist.eq_ignore_ascii_case("unknown artist")
            && !is_generic_folder_name(&clean_artist)
        {
            let key = format!("sim:v3:{}", slug(&clean_artist));
            if self.store.kv_get(&key).is_none() {
                if let Ok(handle) = tokio::runtime::Handle::try_current() {
                    let store = self.store.clone();
                    let http = self.http.clone();
                    let title = track.title.clone();
                    handle.spawn(async move {
                        let _ = fetch_and_store_similarity(&store, &http, &clean_artist, &title)
                            .await;
                    });
                }
            }
        }
    }

    fn popular_boost(&self, prof: &TasteProfile, net: &UserNet, track_id: &str) -> f64 {
        let ranks = self.popular_ranks.lock().unwrap();
        let Some(&rank) = ranks.get(track_id) else {
            return 0.0;
        };
        let is_fresh = prof.artist_score.is_empty() && net.updates < 5;
        if is_fresh {
            20.0 - (rank as f64 * 0.4).min(12.0)
        } else {
            2.5 - (rank as f64 * 0.05).min(1.5)
        }
    }

    fn score_with_neural(&self, prof: &TasteProfile, net: &UserNet, t: &Track) -> f64 {
        let base = prof.score_track(t) + self.popular_boost(prof, net, &t.id);
        if net.updates == 0 {
            return base;
        }
        let a_aff = prof
            .artist_score
            .get(&primary_artist(&t.artist).to_lowercase())
            .copied()
            .unwrap_or(0.0);
        let g_aff = prof
            .genre_score
            .get(&t.genre.to_lowercase())
            .copied()
            .unwrap_or(0.0);
        let plays = prof.track_plays.get(&t.id).copied().unwrap_or(0);
        let skips = prof.track_skips.get(&t.id).copied().unwrap_or(0);
        let feats = extract_features(t, a_aff, g_aff, plays, skips);
        let pred = net.predict(&feats);
        // Smoothly ramp neural influence as the model gathers experience
        let confidence = (net.updates as f64 / 20.0).min(1.0);
        base + (pred - 0.5) * 10.0 * confidence
    }

    pub fn profile(&self, user_id: i64) -> TasteProfile {
        TasteProfile::build(&self.store, user_id)
    }

    /// Generates all personal mixes & auto playlists for a user and syncs them into SQLite
    /// so Subsonic/Navidrome clients see them.
    pub fn generate_auto_playlists(&self, user_id: i64) {
        let prof = self.profile(user_id);
        let catalog = self.store.all_tracks(3000);
        let now = Utc::now().timestamp();

        // 1. My Supermix
        let net = self.neural.get_or_load(&self.store, user_id);
        let supermix = self.build_supermix(&prof, &net, &catalog, 35);
        if !supermix.is_empty() {
            self.save_auto_pl(
                user_id,
                "My Supermix",
                "Personalized blend of your favorites, discoveries & high-res tracks",
                "supermix",
                &supermix,
            );
        }

        // 2. Daily Mix 1, 2, 3 (by top genres or moods)
        let daily_clusters = self.build_daily_mixes(&prof, &catalog);
        for (idx, (title, subtitle, tracks)) in daily_clusters.into_iter().enumerate() {
            if tracks.is_empty() {
                continue;
            }
            let kind = format!("daily-mix-{}", idx + 1);
            self.save_auto_pl(user_id, &title, &subtitle, &kind, &tracks);
        }

        // 3. Discover Weekly (unplayed tracks matching taste)
        let discover = self.build_discover_weekly(&prof, &catalog, 30);
        if !discover.is_empty() {
            self.save_auto_pl(
                user_id,
                "Discover Weekly",
                "Fresh tracks tailored to your taste that you haven't played yet",
                "discover",
                &discover,
            );
        }

        // 4. On Repeat
        let on_repeat = self.build_on_repeat(&prof, &catalog, 25);
        if !on_repeat.is_empty() {
            self.save_auto_pl(
                user_id,
                "On Repeat",
                "Songs you can't stop playing lately",
                "on-repeat",
                &on_repeat,
            );
        }

        // 5. Forgotten Gems
        let gems = self.build_forgotten_gems(&prof, &catalog, now, 25);
        if !gems.is_empty() {
            self.save_auto_pl(
                user_id,
                "Forgotten Gems",
                "Past favorites you haven't revisited recently",
                "forgotten-gems",
                &gems,
            );
        }

        // 6. Lossless Showcase
        let lossless = self.build_lossless_showcase(&prof, &catalog, 30);
        if !lossless.is_empty() {
            self.save_auto_pl(
                user_id,
                "Lossless Showcase",
                "Studio-grade FLAC, ALAC & WAV tracks in highest quality",
                "lossless",
                &lossless,
            );
        }
    }

    fn save_auto_pl(&self, user_id: i64, name: &str, comment: &str, kind: &str, tracks: &[Track]) {
        let ids: Vec<String> = tracks.iter().map(|t| t.id.clone()).collect();
        let _ = self
            .store
            .upsert_auto_playlist(user_id, name, comment, kind, &ids);
    }

    fn build_supermix(&self, prof: &TasteProfile, net: &UserNet, catalog: &[Track], limit: usize) -> Vec<Track> {
        let has_taste = prof.artist_score.values().any(|&v| v > 0.0)
            || prof.genre_score.values().any(|&v| v > 0.0)
            || !prof.starred.is_empty()
            || prof.ratings.values().any(|&r| r >= 3);

        let now = Utc::now().timestamp();
        let mut seed_weights: HashMap<String, f64> = prof
            .artist_score
            .iter()
            .filter(|(_, s)| **s > 0.0)
            .map(|(a, s)| (a.clone(), *s))
            .collect();

        if !prof.starred.is_empty() {
            for t in catalog {
                if prof.starred.contains(&t.id) {
                    let pa = primary_artist(&t.artist).to_lowercase();
                    if !pa.is_empty() && pa != "unknown artist" && !is_generic_folder_name(&pa) {
                        seed_weights.entry(pa).or_insert(2.5);
                    }
                }
            }
        }

        let mut top_seeds: Vec<(String, f64)> = seed_weights.into_iter().collect();
        top_seeds.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        top_seeds.truncate(8);

        let max_seed_w = top_seeds.first().map(|(_, w)| *w).unwrap_or(1.0).max(1.0);
        let mut related_boost: HashMap<String, f64> = HashMap::new();
        let mut radio_boost: HashMap<String, f64> = HashMap::new();
        for (seed_artist, w) in &top_seeds {
            if let Some(sim) = self.load_cached_similarity(seed_artist) {
                let scale = (*w / max_seed_w).clamp(0.35, 1.0);
                for (rank, rel) in sim.related_artists.iter().enumerate() {
                    let ra = primary_artist(rel).to_lowercase();
                    if !ra.is_empty() {
                        let b = (10.0 - (rank as f64 * 0.35).min(5.0)) * scale;
                        related_boost
                            .entry(ra)
                            .and_modify(|v| *v = (*v).max(b))
                            .or_insert(b);
                    }
                }
                for (ra, rt) in &sim.radio_tracks {
                    let rk = format!("{}|{}", slug(&primary_artist(ra)), core_song_title(rt));
                    let b = 14.0 * scale;
                    radio_boost
                        .entry(rk)
                        .and_modify(|v| *v = (*v).max(b))
                        .or_insert(b);
                }
            }
        }

        let mut scored: Vec<(f64, Track)> = Vec::new();
        for t in catalog {
            if is_junk_track(t)
                || (t.duration > 0 && t.duration < 45)
                || is_mix_variant_noise(&t.title)
            {
                continue;
            }
            let ta = primary_artist(&t.artist).to_lowercase();
            let sim_rel = related_boost.get(&ta).copied().unwrap_or(0.0);
            let sim_rad = radio_boost
                .get(&t.song_key())
                .copied()
                .or_else(|| {
                    t.song_aliases()
                        .iter()
                        .filter_map(|a| radio_boost.get(a).copied())
                        .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
                })
                .unwrap_or(0.0);
            let coocc = top_seeds
                .iter()
                .filter_map(|(sa, w)| {
                    let scale = (*w / max_seed_w).clamp(0.35, 1.0);
                    prof.cooccurrence
                        .get(sa)
                        .and_then(|m| m.get(&ta))
                        .map(|c| (c * 3.0 * scale).min(9.0))
                })
                .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
                .unwrap_or(0.0);

            if has_taste {
                let (has_link, mus_aff) = prof.musical_affinity(t);
                let total_aff = mus_aff + sim_rel + sim_rad + coocc;
                if !(has_link || sim_rel > 0.0 || sim_rad > 0.0 || coocc > 0.0) || total_aff <= 0.25 {
                    continue;
                }
            }

            let id_last = prof
                .track_last_played
                .get(&t.id)
                .copied()
                .unwrap_or(t.last_played);
            let song_last = prof
                .song_last_played
                .get(&t.song_key())
                .copied()
                .unwrap_or(0);
            let last = id_last.max(song_last);
            let cooldown = if last > 0 && now - last < 10800 {
                6.0
            } else {
                0.0
            };
            let s = self.score_with_neural(prof, net, t) + sim_rel + sim_rad + coocc - cooldown;
            scored.push((s, t.clone()));
        }
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        interleave_by_artist(scored.into_iter().map(|(_, t)| t).collect(), limit, 3)
    }

    fn build_daily_mixes(
        &self,
        prof: &TasteProfile,
        catalog: &[Track],
    ) -> Vec<(String, String, Vec<Track>)> {
        // Pick top 3 genres, or fall back to top 3 moods
        let mut top_genres: Vec<(String, f64)> = prof
            .genre_score
            .iter()
            .filter(|(_, s)| **s > 0.0)
            .map(|(g, s)| (g.clone(), *s))
            .collect();
        top_genres.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let mut out = Vec::new();
        for (i, (genre, _)) in top_genres.iter().take(3).enumerate() {
            let matching: Vec<Track> = catalog
                .iter()
                .filter(|t| !is_junk_track(t) && t.genre.eq_ignore_ascii_case(genre))
                .cloned()
                .collect();
            if matching.len() >= 3 {
                let diversified = mmr_diversify(matching, 25, 2);
                let title = format!("Daily Mix {}", i + 1);
                let sub = format!("{} & similar vibes", capitalize_words(genre));
                out.push((title, sub, diversified));
            }
        }

        // Fill remaining daily mixes with top moods if < 3
        if out.len() < 3 {
            let mut mood_order: Vec<Mood> = Mood::ALL.to_vec();
            mood_order.sort_by(|a, b| {
                let sa = prof.mood_score.get(a).copied().unwrap_or(0.0);
                let sb = prof.mood_score.get(b).copied().unwrap_or(0.0);
                sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
            });
            for mood in mood_order {
                if out.len() >= 3 {
                    break;
                }
                let matching: Vec<Track> = catalog
                    .iter()
                    .filter(|t| !is_junk_track(t) && t.mood_list().contains(&mood))
                    .cloned()
                    .collect();
                if !matching.is_empty() {
                    let idx = out.len() + 1;
                    let diversified = mmr_diversify(matching, 25, 2);
                    out.push((
                        format!("Daily Mix {}", idx),
                        mood.label().to_string(),
                        diversified,
                    ));
                }
            }
        }
        out
    }

    fn build_discover_weekly(
        &self,
        prof: &TasteProfile,
        catalog: &[Track],
        limit: usize,
    ) -> Vec<Track> {
        let empty_net = UserNet::new(0);
        let mut unplayed: Vec<(f64, Track)> = catalog
            .iter()
            .filter(|t| {
                !is_junk_track(t)
                    && prof.track_plays.get(&t.id).copied().unwrap_or(0) == 0
                    && t.play_count == 0
            })
            .map(|t| (prof.score_track(t) + self.popular_boost(prof, &empty_net, &t.id), t.clone()))
            .collect();
        unplayed.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        mmr_diversify(unplayed.into_iter().map(|(_, t)| t).collect(), limit, 2)
    }

    fn build_on_repeat(&self, prof: &TasteProfile, catalog: &[Track], limit: usize) -> Vec<Track> {
        let mut played: Vec<(i32, Track)> = catalog
            .iter()
            .filter_map(|t| {
                let c = prof.track_plays.get(&t.id).copied().unwrap_or(0) + t.play_count;
                if c > 0 {
                    Some((c, t.clone()))
                } else {
                    None
                }
            })
            .collect();
        played.sort_by_key(|a| std::cmp::Reverse(a.0));
        played.into_iter().take(limit).map(|(_, t)| t).collect()
    }

    fn build_forgotten_gems(
        &self,
        prof: &TasteProfile,
        catalog: &[Track],
        now: i64,
        limit: usize,
    ) -> Vec<Track> {
        let cutoff = now - 14 * 86400;
        let mut gems: Vec<(f64, Track)> = catalog
            .iter()
            .filter(|t| {
                let liked = prof.starred.contains(&t.id)
                    || prof.ratings.get(&t.id).copied().unwrap_or(0) >= 4
                    || prof.track_plays.get(&t.id).copied().unwrap_or(0) >= 2;
                let last = prof
                    .track_last_played
                    .get(&t.id)
                    .copied()
                    .unwrap_or(t.last_played);
                liked && (last == 0 || last < cutoff)
            })
            .map(|t| (prof.score_track(t), t.clone()))
            .collect();
        gems.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        gems.into_iter().take(limit).map(|(_, t)| t).collect()
    }

    fn build_lossless_showcase(
        &self,
        prof: &TasteProfile,
        catalog: &[Track],
        limit: usize,
    ) -> Vec<Track> {
        let mut lossless: Vec<(f64, Track)> = catalog
            .iter()
            .filter(|t| t.is_lossless() && !is_junk_track(t))
            .map(|t| (prof.score_track(t) + (t.quality_score() as f64) / 100.0, t.clone()))
            .collect();
        lossless.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        mmr_diversify(lossless.into_iter().map(|(_, t)| t).collect(), limit, 3)
    }

    pub fn recommend_for_user(&self, user_id: i64, limit: usize) -> Vec<Track> {
        let prof = self.profile(user_id);
        let net = self.neural.get_or_load(&self.store, user_id);
        let catalog = self.store.all_tracks(2500);
        let mut scored: Vec<(f64, Track)> = catalog
            .into_iter()
            .filter(|t| !is_junk_track(t))
            .map(|t| {
                let mut s = self.score_with_neural(&prof, &net, &t);
                // Slight novelty boost for unplayed tracks
                if prof.track_plays.get(&t.id).copied().unwrap_or(0) == 0 {
                    s += 1.2;
                }
                (s, t)
            })
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        mmr_diversify(scored.into_iter().map(|(_, t)| t).collect(), limit, 2)
    }

    #[cfg(test)]
    pub fn similar_songs(&self, seed: &Track, user_id: i64, limit: usize) -> Vec<Track> {
        let cached_sim = self.load_cached_similarity(&seed.artist);
        self.score_similar_local(seed, user_id, limit, cached_sim.as_ref())
    }

    /// Radio-grade similarity: combines Deezer's related artists & artist radio graph
    /// with local catalog metadata/co-occurrence, and dynamically discovers missing
    /// similar tracks from configured sources when the local library doesn't have enough.
    pub async fn similar_songs_async(
        self: &Arc<Self>,
        seed: &Track,
        user_id: i64,
        limit: usize,
    ) -> Vec<Track> {
        let sim_data = self.get_or_fetch_similarity(&seed.artist, &seed.title).await;
        if seed.genre.trim().is_empty() {
            if let Some(ref sim) = sim_data {
                if !sim.seed_genre.trim().is_empty() {
                    self.store.update_track_cover_and_genre(
                        &seed.id,
                        None,
                        Some(&sim.seed_genre),
                    );
                }
            }
        }

        let mut results = self.score_similar_local(seed, user_id, limit, sim_data.as_ref());

        // If local catalog doesn't have enough genuinely similar tracks, dynamically
        // search our sources for tracks from Deezer's artist radio so radio stays on-vibe.
        if results.len() < limit {
            if let Some(ref sim) = sim_data {
                let mut existing_keys: HashSet<String> = HashSet::new();
                let mut existing_titles: HashSet<String> = HashSet::new();
                mark_track_seen(seed, &mut existing_keys, &mut existing_titles);
                for t in &results {
                    mark_track_seen(t, &mut existing_keys, &mut existing_titles);
                }

                let mut to_fetch: Vec<(String, String)> = Vec::new();
                let mut per_artist: HashMap<String, usize> = HashMap::new();
                for (artist, title) in &sim.radio_tracks {
                    let clean_a = primary_artist(artist);
                    let ct = core_song_title(title);
                    let key = format!("{}|{}", slug(&clean_a), ct);
                    if existing_keys.contains(&key)
                        || (ct.chars().count() >= 3 && existing_titles.contains(&ct))
                    {
                        continue;
                    }
                    let a_low = clean_a.to_lowercase();
                    let cnt = per_artist.get(&a_low).copied().unwrap_or(0);
                    if cnt >= 2 {
                        continue;
                    }
                    per_artist.insert(a_low, cnt + 1);
                    existing_keys.insert(key);
                    if ct.chars().count() >= 3 {
                        existing_titles.insert(ct);
                    }
                    to_fetch.push((clean_a, title.clone()));
                    if to_fetch.len() >= (limit - results.len()).clamp(4, 12) {
                        break;
                    }
                }

                if !to_fetch.is_empty() {
                    let deadline = tokio::time::sleep(std::time::Duration::from_millis(3800));
                    let futs: Vec<_> = to_fetch
                        .iter()
                        .take(8)
                        .map(|(a, t)| {
                            let q = format!("{} {}", a, t);
                            let target_a = a.clone();
                            let target_t = t.clone();
                            let reg = Arc::clone(&self.registry);
                            async move { (target_a, target_t, reg.search_all(&q, 3).await) }
                        })
                        .collect();
                    let fetched = tokio::select! {
                        batches = futures::future::join_all(futs) => batches,
                        _ = deadline => vec![],
                    };
                    for (target_a, target_t, batch) in fetched {
                        let best = batch
                            .into_iter()
                            .filter(|t| {
                                !(t.duration > 0 && t.duration < 45)
                                    && !is_mix_variant_noise(&t.title)
                                    && !track_seen_check(t, &existing_keys, &existing_titles)
                            })
                            .filter_map(|t| {
                                let m = track_match_score(&t, &target_a, &target_t, 0)?;
                                Some((m * 1000 + t.quality_score(), t))
                            })
                            .max_by_key(|(s, _)| *s)
                            .map(|(_, t)| t);
                        if let Some(best) = best {
                            mark_track_seen(&best, &mut existing_keys, &mut existing_titles);
                            results.push(best);
                        }
                    }

                    // Background-warm any remaining radio tracks for seamless next-up
                    if to_fetch.len() > 8 {
                        let rest: Vec<(String, String)> = to_fetch.into_iter().skip(8).collect();
                        let reg = Arc::clone(&self.registry);
                        tokio::spawn(async move {
                            for (a, t) in rest {
                                let _ = reg.search_all(&format!("{} {}", a, t), 2).await;
                            }
                        });
                    }
                }
            }
        }

        // Re-score & interleave so the same artist doesn't play back-to-back
        interleave_by_artist(results, limit, 2)
    }

    fn score_similar_local(
        &self,
        seed: &Track,
        user_id: i64,
        limit: usize,
        sim_data: Option<&DeezerSimData>,
    ) -> Vec<Track> {
        let prof = self.profile(user_id);
        let catalog = self.store.all_tracks(3000);
        let now = Utc::now().timestamp();
        let mut seed_keys: HashSet<String> = HashSet::new();
        let mut seed_titles: HashSet<String> = HashSet::new();
        mark_track_seen(seed, &mut seed_keys, &mut seed_titles);

        let seed_artist_clean = primary_artist(&seed.artist);
        let seed_artist = if is_generic_folder_name(&seed_artist_clean) {
            String::new()
        } else {
            seed_artist_clean.to_lowercase()
        };
        let seed_moods: HashSet<Mood> = seed.mood_list().into_iter().collect();
        let seed_genre_low = seed.genre.trim().to_lowercase();
        let seed_genre_tokens: HashSet<&str> = seed_genre_low
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| w.len() >= 3)
            .collect();

        let related_ranks: HashMap<String, usize> = sim_data
            .map(|d| {
                d.related_artists
                    .iter()
                    .enumerate()
                    .map(|(i, a)| (primary_artist(a).to_lowercase(), i))
                    .collect()
            })
            .unwrap_or_default();

        let radio_keys: HashSet<String> = sim_data
            .map(|d| {
                d.radio_tracks
                    .iter()
                    .map(|(a, t)| format!("{}|{}", slug(&primary_artist(a)), core_song_title(t)))
                    .collect()
            })
            .unwrap_or_default();

        let sim_genre_low = sim_data
            .map(|d| d.seed_genre.trim().to_lowercase())
            .unwrap_or_default();

        let mut scored: Vec<(f64, Track)> = Vec::new();
        for t in catalog {
            if (!seed.id.is_empty() && t.id == seed.id)
                || is_junk_track(&t)
                || (t.duration > 0 && t.duration < 45)
                || is_mix_variant_noise(&t.title)
                || track_seen_check(&t, &seed_keys, &seed_titles)
            {
                continue;
            }

            let mut s = 0.0;
            let tk = t.song_key();
            let ta = primary_artist(&t.artist).to_lowercase();

            if radio_keys.contains(&tk) || t.song_aliases().iter().any(|a| radio_keys.contains(a)) {
                s += 16.0;
            }
            if let Some(&rank) = related_ranks.get(&ta) {
                s += 12.0 - (rank as f64 * 0.35).min(5.0);
            }
            if !seed_artist.is_empty() && ta == seed_artist {
                s += 7.5;
            } else if !seed_artist.is_empty() {
                if let Some(co) = prof.cooccurrence.get(&seed_artist).and_then(|m| m.get(&ta)) {
                    s += (co * 3.5).min(10.0);
                }
            }

            let tg_low = t.genre.trim().to_lowercase();
            if !tg_low.is_empty() {
                if !seed_genre_low.is_empty() && tg_low == seed_genre_low {
                    s += 6.5;
                } else if !sim_genre_low.is_empty() && tg_low == sim_genre_low {
                    s += 5.0;
                } else if !seed_genre_tokens.is_empty() {
                    let overlap = tg_low
                        .split(|c: char| !c.is_alphanumeric())
                        .filter(|w| w.len() >= 3 && seed_genre_tokens.contains(w))
                        .count();
                    if overlap > 0 {
                        s += 3.0;
                    }
                }
            }

            let t_moods: HashSet<Mood> = t.mood_list().into_iter().collect();
            let mood_overlap = seed_moods.intersection(&t_moods).count();
            s += (mood_overlap as f64) * 3.0;

            if seed.year > 1940 && t.year > 1940 {
                let diff = (seed.year - t.year).abs();
                if diff <= 6 {
                    s += 1.5;
                } else if diff <= 12 {
                    s += 0.5;
                }
            }

            if seed.duration > 30 && t.duration > 30 && (seed.duration - t.duration).abs() <= 45 {
                s += 0.8;
            }

            // Strict threshold: only keep genuinely similar tracks!
            if s < 3.0 {
                continue;
            }

            // Penalize recently played or skipped tracks so radio feels fresh
            let skips = prof.track_skips.get(&t.id).copied().unwrap_or(0);
            if skips > 0 {
                s -= (skips as f64) * 4.0;
            }
            let last = prof.track_last_played.get(&t.id).copied().unwrap_or(t.last_played);
            if last > 0 && now - last < 7200 {
                s -= 6.5;
            }

            // Tiny tiebreaker for quality & user taste among already-similar tracks
            s += (t.quality_score() as f64) / 1000.0 + prof.score_track(&t) * 0.12;
            scored.push((s, t));
        }

        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        interleave_by_artist(scored.into_iter().map(|(_, t)| t).collect(), limit, 2)
    }

    fn load_cached_similarity(&self, artist: &str) -> Option<DeezerSimData> {
        let clean = primary_artist(artist);
        if clean.is_empty()
            || clean.eq_ignore_ascii_case("unknown artist")
            || is_generic_folder_name(&clean)
        {
            return None;
        }
        let key = format!("sim:v3:{}", slug(&clean));
        let raw = self.store.kv_get(&key)?;
        let cached: CachedSimData = serde_json::from_str(&raw).ok()?;
        let age = Utc::now().timestamp() - cached.at;
        if age < 7 * 86400 {
            Some(cached.data)
        } else {
            None
        }
    }

    async fn get_or_fetch_similarity(&self, artist: &str, title: &str) -> Option<DeezerSimData> {
        let clean = primary_artist(artist);
        if clean.is_empty()
            || clean.eq_ignore_ascii_case("unknown artist")
            || is_generic_folder_name(&clean)
        {
            return None;
        }
        if let Some(cached) = self.load_cached_similarity(&clean) {
            return Some(cached);
        }
        fetch_and_store_similarity(&self.store, &self.http, &clean, title).await
    }

    /// Background catalog warmer so a fresh server immediately has currently popular music
    /// instead of an empty shelf.
    pub async fn warm_catalog(&self) {
        if self.store.track_count() < 30 {
            self.run_popular_warm("fresh catalog").await;
        }
    }

    /// Warms currently popular music once Soulseek connects so Soulseek-backed results
    /// are available right away on the home screen.
    pub async fn warm_popular_on_connect(&self) {
        if self.store.count_by_source("soulseek") == 0 || self.store.track_count() < 30 {
            self.run_popular_warm("soulseek connected").await;
        }
    }

    async fn run_popular_warm(&self, reason: &str) {
        if self.warming.swap(true, Ordering::SeqCst) {
            return;
        }
        info!(reason, "warming popular music catalog in background...");
        let seeds = fetch_popular_seeds().await;
        let mut verified = Vec::new();
        let mut ranks: HashMap<String, usize> = HashMap::new();
        let mut rank_idx = 0usize;

        for chunk in seeds.chunks(4) {
            let futs: Vec<_> = chunk
                .iter()
                .map(|seed| {
                    let q = format!("{} {}", seed.artist, seed.title);
                    let reg = Arc::clone(&self.registry);
                    async move { reg.search_all_raw(&q, 10).await }
                })
                .collect();
            let batches = futures::future::join_all(futs).await;
            for (seed, candidates) in chunk.iter().zip(batches) {
                let best = candidates
                    .into_iter()
                    .filter_map(|t| popular_match_score(&t, seed).map(|ms| (ms, t.quality_score(), t)))
                    .max_by_key(|(ms, qs, _)| (*ms, *qs));
                if let Some((_, _, mut track)) = best {
                    if !seed.album.is_empty()
                        && (track.album.is_empty() || track.album == "Unknown Album")
                    {
                        track.album = seed.album.clone();
                    }
                    if track.cover_url.is_empty() && !seed.cover_url.is_empty() {
                        track.cover_url = seed.cover_url.clone();
                    }
                    if track.duration == 0 && seed.duration > 0 {
                        track.duration = seed.duration;
                    }
                    if track.genre.is_empty() {
                        track.genre = "Pop".to_string();
                    }
                    ranks.insert(track.id.clone(), rank_idx);
                    verified.push(track);
                }
                rank_idx += 1;
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }

        if !verified.is_empty() {
            self.store.upsert_tracks(&verified);
            *self.popular_ranks.lock().unwrap() = ranks;
            // Force auto-playlists to refresh on next request with the newly warmed popular tracks.
            self.auto_pl_last.lock().unwrap().clear();
        }
        info!(
            warmed = verified.len(),
            tracks = self.store.track_count(),
            "popular catalog warm complete"
        );
        self.warming.store(false, Ordering::SeqCst);
    }
}

struct PopularSeed {
    artist: String,
    title: String,
    album: String,
    cover_url: String,
    duration: i32,
}

fn track_match_score(
    t: &Track,
    target_artist: &str,
    target_title: &str,
    target_duration: i32,
) -> Option<i32> {
    if is_junk_track(t) || (t.duration > 0 && t.duration < 45) {
        return None;
    }
    let title_low = t.title.to_lowercase();
    let seed_title_low = target_title.to_lowercase();
    const VARIANT_NOISE: &[&str] = &[
        "nightcore",
        "slowed",
        "reverb",
        "karaoke",
        "instrumental",
        "tribute",
        "type beat",
        "8d audio",
        "remix",
        "cover",
        "acoustic",
        "live",
    ];
    for kw in VARIANT_NOISE {
        if title_low.contains(kw) && !seed_title_low.contains(kw) {
            return None;
        }
    }
    if target_duration > 0 && t.duration > 0 && (t.duration - target_duration).abs() > 90 {
        return None;
    }

    let core_seed_title = core_song_title(target_title);
    let core_track_title = core_song_title(&t.title);
    if core_seed_title.is_empty() || core_track_title.is_empty() {
        return None;
    }

    let title_exact = core_track_title == core_seed_title;
    let seed_title_tokens: Vec<&str> = core_seed_title
        .split_whitespace()
        .filter(|w| w.len() >= 2)
        .collect();
    let title_contains = core_track_title.contains(&core_seed_title)
        || (!seed_title_tokens.is_empty()
            && seed_title_tokens.iter().all(|w| core_track_title.contains(w)));
    if !title_exact && !title_contains {
        return None;
    }

    let seed_artist_norm = slug(&primary_artist(target_artist));
    let track_artist_norm = slug(&primary_artist(&t.artist));
    let track_title_norm = slug(&t.title);
    let artist_tokens: Vec<String> = primary_artist(target_artist)
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 3)
        .map(|s| s.to_string())
        .collect();

    let artist_match = (!seed_artist_norm.is_empty()
        && (track_artist_norm.contains(&seed_artist_norm)
            || seed_artist_norm.contains(&track_artist_norm)
            || track_title_norm.contains(&seed_artist_norm)))
        || (!artist_tokens.is_empty()
            && artist_tokens
                .iter()
                .all(|tok| t.artist.to_lowercase().contains(tok) || title_low.contains(tok)));

    if !artist_match {
        return None;
    }

    let mut score = 0;
    if title_exact {
        score += 100;
    } else if title_contains {
        score += 50;
    }
    if track_artist_norm == seed_artist_norm {
        score += 80;
    } else {
        score += 40;
    }
    Some(score)
}

fn popular_match_score(t: &Track, seed: &PopularSeed) -> Option<i32> {
    track_match_score(t, &seed.artist, &seed.title, seed.duration)
}

async fn fetch_popular_seeds() -> Vec<PopularSeed> {
    let fallback: Vec<PopularSeed> = [
        ("Lady Gaga", "Die With A Smile"),
        ("Sabrina Carpenter", "Espresso"),
        ("Billie Eilish", "Birds of a Feather"),
        ("Kendrick Lamar", "Not Like Us"),
        ("Chappell Roan", "Good Luck Babe"),
        ("The Weeknd", "Blinding Lights"),
        ("SZA", "Kill Bill"),
        ("Taylor Swift", "Cruel Summer"),
        ("Dua Lipa", "Levitating"),
        ("Daft Punk", "Get Lucky"),
        ("Bruno Mars", "APT"),
        ("Charli xcx", "360"),
    ]
    .into_iter()
    .map(|(a, t)| PopularSeed {
        artist: a.to_string(),
        title: t.to_string(),
        album: String::new(),
        cover_url: String::new(),
        duration: 0,
    })
    .collect();

    let Ok(client) = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .user_agent("Beatlab/0.2")
        .build()
    else {
        return fallback;
    };

    let Ok(resp) = client
        .get("https://api.deezer.com/chart/0/tracks?limit=25")
        .send()
        .await
    else {
        return fallback;
    };

    let Ok(json) = resp.json::<serde_json::Value>().await else {
        return fallback;
    };

    let mut seeds = Vec::new();
    if let Some(items) = json.get("data").and_then(|d| d.as_array()) {
        for item in items.iter().take(25) {
            let title = item.get("title").and_then(|v| v.as_str()).unwrap_or("").trim();
            let artist = item
                .get("artist")
                .and_then(|a| a.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            if title.is_empty() || artist.is_empty() {
                continue;
            }
            let album = item
                .get("album")
                .and_then(|a| a.get("title"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            let cover_url = item
                .get("album")
                .and_then(|a| a.get("cover_xl").or_else(|| a.get("cover_big")))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            let duration = item.get("duration").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            seeds.push(PopularSeed {
                artist: artist.to_string(),
                title: title.to_string(),
                album,
                cover_url,
                duration,
            });
        }
    }

    if seeds.is_empty() {
        fallback
    } else {
        seeds
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct DeezerSimData {
    related_artists: Vec<String>,
    radio_tracks: Vec<(String, String)>,
    seed_genre: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct CachedSimData {
    at: i64,
    data: DeezerSimData,
}

async fn fetch_deezer_similarity(
    client: &reqwest::Client,
    artist: &str,
    title: &str,
) -> Option<DeezerSimData> {
    let q = if !title.trim().is_empty() {
        format!("artist:\"{}\" track:\"{}\"", artist.trim(), title.trim())
    } else {
        artist.trim().to_string()
    };
    let search_url = format!(
        "https://api.deezer.com/search?q={}&limit=3",
        urlencoding::encode(&q)
    );
    let mut artist_id: Option<u64> = None;
    let mut album_id: Option<u64> = None;

    if let Ok(resp) = client.get(&search_url).send().await {
        if let Ok(json) = resp.json::<serde_json::Value>().await {
            if let Some(first) = json.get("data").and_then(|d| d.as_array()).and_then(|a| a.first()) {
                artist_id = first.get("artist").and_then(|a| a.get("id")).and_then(|v| v.as_u64());
                album_id = first.get("album").and_then(|a| a.get("id")).and_then(|v| v.as_u64());
            }
        }
    }

    if artist_id.is_none() {
        let a_url = format!(
            "https://api.deezer.com/search/artist?q={}&limit=1",
            urlencoding::encode(artist.trim())
        );
        if let Ok(resp) = client.get(&a_url).send().await {
            if let Ok(json) = resp.json::<serde_json::Value>().await {
                artist_id = json
                    .get("data")
                    .and_then(|d| d.as_array())
                    .and_then(|a| a.first())
                    .and_then(|a| a.get("id"))
                    .and_then(|v| v.as_u64());
            }
        }
    }

    let aid = artist_id?;
    let radio_url = format!("https://api.deezer.com/artist/{}/radio?limit=35", aid);
    let rel_url = format!("https://api.deezer.com/artist/{}/related?limit=15", aid);
    let alb_url = album_id.map(|id| format!("https://api.deezer.com/album/{}", id));

    let (radio_res, rel_res, alb_res) = tokio::join!(
        client.get(&radio_url).send(),
        client.get(&rel_url).send(),
        async {
            if let Some(u) = alb_url {
                client.get(&u).send().await.ok()
            } else {
                None
            }
        }
    );

    let mut related_artists = Vec::new();
    if let Ok(resp) = rel_res {
        if let Ok(json) = resp.json::<serde_json::Value>().await {
            if let Some(arr) = json.get("data").and_then(|d| d.as_array()) {
                for item in arr {
                    if let Some(name) = item.get("name").and_then(|v| v.as_str()) {
                        let n = name.trim().to_string();
                        if !n.is_empty() {
                            related_artists.push(n);
                        }
                    }
                }
            }
        }
    }

    let mut radio_tracks = Vec::new();
    if let Ok(resp) = radio_res {
        if let Ok(json) = resp.json::<serde_json::Value>().await {
            if let Some(arr) = json.get("data").and_then(|d| d.as_array()) {
                for item in arr {
                    let t = item.get("title").and_then(|v| v.as_str()).unwrap_or("").trim();
                    let a = item
                        .get("artist")
                        .and_then(|ar| ar.get("name"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim();
                    if !t.is_empty() && !a.is_empty() {
                        radio_tracks.push((a.to_string(), t.to_string()));
                    }
                }
            }
        }
    }

    let mut seed_genre = String::new();
    if let Some(resp) = alb_res {
        if let Ok(json) = resp.json::<serde_json::Value>().await {
            if let Some(g) = json
                .get("genres")
                .and_then(|g| g.get("data"))
                .and_then(|d| d.as_array())
                .and_then(|a| a.first())
                .and_then(|g| g.get("name"))
                .and_then(|v| v.as_str())
            {
                seed_genre = g.trim().to_string();
            }
        }
    }

    if related_artists.is_empty() && radio_tracks.is_empty() {
        return None;
    }

    Some(DeezerSimData {
        related_artists,
        radio_tracks,
        seed_genre,
    })
}

async fn fetch_and_store_similarity(
    store: &Store,
    http: &reqwest::Client,
    clean_artist: &str,
    title: &str,
) -> Option<DeezerSimData> {
    let fetched = fetch_deezer_similarity(http, clean_artist, title).await?;
    let key = format!("sim:v3:{}", slug(clean_artist));
    let payload = CachedSimData {
        at: Utc::now().timestamp(),
        data: fetched.clone(),
    };
    if let Ok(json) = serde_json::to_string(&payload) {
        store.kv_set(&key, &json);
    }
    Some(fetched)
}

fn is_mix_variant_noise(title: &str) -> bool {
    let low = title.to_lowercase();
    const NOISE: &[&str] = &[
        "karaoke",
        "tribute",
        "type beat",
        "8d audio",
        "slowed",
        "reverb",
        "nightcore",
    ];
    NOISE.iter().any(|kw| low.contains(kw))
}

fn track_seen_check(
    t: &Track,
    seen_keys: &HashSet<String>,
    seen_titles: &HashSet<String>,
) -> bool {
    if seen_keys.contains(&t.song_key()) {
        return true;
    }
    for alias in t.song_aliases() {
        if seen_keys.contains(&alias) {
            return true;
        }
        if let Some((_, core)) = alias.split_once('|') {
            if core.chars().count() >= 3 && seen_titles.contains(core) {
                return true;
            }
        }
    }
    let ct = core_song_title(&t.title);
    ct.chars().count() >= 3 && seen_titles.contains(&ct)
}

fn mark_track_seen(
    t: &Track,
    seen_keys: &mut HashSet<String>,
    seen_titles: &mut HashSet<String>,
) {
    seen_keys.insert(t.song_key());
    for alias in t.song_aliases() {
        if let Some((_, core)) = alias.split_once('|') {
            if core.chars().count() >= 3 {
                seen_titles.insert(core.to_string());
            }
        }
        seen_keys.insert(alias);
    }
    let ct = core_song_title(&t.title);
    if ct.chars().count() >= 3 {
        seen_titles.insert(ct);
    }
}

/// Deduplicates by song identity/title, filters short clips & variant noise, caps tracks per primary artist, and avoids playing the same artist back-to-back.
pub fn interleave_by_artist(tracks: Vec<Track>, limit: usize, max_per_artist: usize) -> Vec<Track> {
    let candidates = mmr_diversify(tracks, limit * 2, max_per_artist);
    let mut remaining = candidates;
    let mut out: Vec<Track> = Vec::with_capacity(limit);
    let mut last_artist = String::new();

    while out.len() < limit && !remaining.is_empty() {
        let pick_idx = remaining
            .iter()
            .position(|t| primary_artist(&t.artist).to_lowercase() != last_artist)
            .unwrap_or(0);
        let chosen = remaining.remove(pick_idx);
        last_artist = primary_artist(&chosen.artist).to_lowercase();
        out.push(chosen);
    }
    out
}

/// Deduplicates by `song_key`, `song_aliases`, and `core_song_title`, filters short clips & variant noise, and caps tracks per primary artist.
pub fn mmr_diversify(tracks: Vec<Track>, limit: usize, max_per_artist: usize) -> Vec<Track> {
    let mut seen_keys = HashSet::new();
    let mut seen_titles = HashSet::new();
    let mut artist_counts: HashMap<String, usize> = HashMap::new();
    let mut out = Vec::with_capacity(limit);

    for t in tracks {
        if out.len() >= limit {
            break;
        }
        if is_junk_track(&t)
            || (t.duration > 0 && t.duration < 45)
            || is_mix_variant_noise(&t.title)
        {
            continue;
        }
        if track_seen_check(&t, &seen_keys, &seen_titles) {
            continue;
        }
        let a_key = primary_artist(&t.artist).to_lowercase();
        let cnt = artist_counts.get(&a_key).copied().unwrap_or(0);
        if cnt >= max_per_artist {
            continue;
        }
        mark_track_seen(&t, &mut seen_keys, &mut seen_titles);
        artist_counts.insert(a_key, cnt + 1);
        out.push(t);
    }
    out
}

pub fn classify_moods_str(title: &str, album: &str, genre: &str, extra: &str) -> String {
    let moods = classify_moods(title, album, genre, extra);
    moods
        .into_iter()
        .map(|m| m.slug())
        .collect::<Vec<_>>()
        .join(",")
}

pub fn classify_moods(title: &str, album: &str, genre: &str, extra: &str) -> Vec<Mood> {
    let hay = format!("{} {} {} {}", title, album, genre, extra).to_lowercase();
    let mut moods = Vec::new();

    let mut push = |m: Mood| {
        if !moods.contains(&m) {
            moods.push(m);
        }
    };

    if hay.contains("lofi")
        || hay.contains("lo-fi")
        || hay.contains("chill")
        || hay.contains("downtempo")
        || hay.contains("trip hop")
        || hay.contains("dream pop")
        || hay.contains("shoegaze")
    {
        push(Mood::Chill);
    }
    if hay.contains("ambient")
        || hay.contains("study")
        || hay.contains("focus")
        || hay.contains("instrumental")
        || hay.contains("classical")
        || hay.contains("piano")
        || hay.contains("soundtrack")
    {
        push(Mood::Focus);
    }
    if hay.contains("metal")
        || hay.contains("punk")
        || hay.contains("workout")
        || hay.contains("hardcore")
        || hay.contains("drum and bass")
        || hay.contains("dnb")
        || hay.contains("dubstep")
        || hay.contains("phonk")
    {
        push(Mood::Energy);
    }
    if hay.contains("night")
        || hay.contains("midnight")
        || hay.contains("nocturne")
        || hay.contains("dark")
        || hay.contains("ambient")
        || hay.contains("synthwave")
        || hay.contains("retrowave")
    {
        push(Mood::LateNight);
    }
    if hay.contains("pop")
        || hay.contains("happy")
        || hay.contains("sun")
        || hay.contains("summer")
        || hay.contains("bright")
        || hay.contains("latin")
        || hay.contains("afrobeat")
    {
        push(Mood::Upbeat);
    }
    if hay.contains("sad")
        || hay.contains("blues")
        || hay.contains("melanchol")
        || hay.contains("lonely")
        || hay.contains("ballad")
        || hay.contains("emo")
    {
        push(Mood::Melancholy);
    }
    if hay.contains("house")
        || hay.contains("techno")
        || hay.contains("dance")
        || hay.contains("electronic")
        || hay.contains("disco")
        || hay.contains("club")
        || hay.contains("edm")
        || hay.contains("trance")
    {
        push(Mood::Party);
        push(Mood::Energy);
    }
    if hay.contains("acoustic")
        || hay.contains("folk")
        || hay.contains("unplugged")
        || hay.contains("singer")
        || hay.contains("country")
    {
        push(Mood::Acoustic);
    }
    if hay.contains("jazz")
        || hay.contains("soul")
        || hay.contains("funk")
        || hay.contains("r&b")
        || hay.contains("rnb")
        || hay.contains("groove")
        || hay.contains("hip hop")
        || hay.contains("hip-hop")
        || hay.contains("rap")
        || hay.contains("trap")
    {
        push(Mood::Groove);
    }
    if hay.contains("rock")
        || hay.contains("indie")
        || hay.contains("alternative")
        || hay.contains("grunge")
        || hay.contains("post-punk")
    {
        push(Mood::RockIndie);
    }

    moods.truncate(3);
    moods
}

fn is_junk_track(t: &Track) -> bool {
    let low = t.title.to_lowercase();
    if low.contains("how to") {
        return true;
    }
    const JUNK: &[&str] = &[
        "reaction", "reactions", "reacts", "tutorial", "lesson", "lessons", "podcast",
        "interview", "interviews", "unboxing", "commentary", "explained",
    ];
    low.split(|c: char| !c.is_alphanumeric())
        .any(|w| JUNK.contains(&w))
}

fn capitalize_words(s: &str) -> String {
    s.split_whitespace()
        .map(|w| {
            let mut ch = w.chars();
            match ch.next() {
                Some(first) => first.to_uppercase().to_string() + ch.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::SourceRegistry;

    #[test]
    fn mood_classification_works() {
        let m = classify_moods("Midnight Jazz Trio", "Late Night Sessions", "Jazz", "");
        assert!(m.contains(&Mood::LateNight));
        assert!(m.contains(&Mood::Groove));
    }

    #[test]
    fn supermix_and_lossless_auto_playlists_work() {
        let store = Store::open_in_memory().unwrap();
        let user = store.create_user("alice", "pass", true).unwrap();
        let t1 = Track {
            id: "local:1".into(),
            source: "local".into(),
            title: "Blue in Green".into(),
            artist: "Miles Davis".into(),
            album: "Kind of Blue".into(),
            genre: "Jazz".into(),
            moods: "groove,chill".into(),
            format: "flac".into(),
            bit_depth: 24,
            sample_rate: 96000,
            stream_url: "file:/tmp/1.flac".into(),
            ..Default::default()
        };
        let t2 = Track {
            id: "youtube:2".into(),
            source: "youtube".into(),
            title: "Lofi Beat".into(),
            artist: "Chillhop".into(),
            album: "Study".into(),
            genre: "Electronic".into(),
            moods: "chill,focus".into(),
            format: "opus".into(),
            bitrate: 128,
            stream_url: "ytdlp:https://youtube.com/watch?v=abc".into(),
            ..Default::default()
        };
        store.upsert_tracks(&[t1.clone(), t2]);
        store.star_track(user.id, &t1.id);

        let reg = Arc::new(SourceRegistry::new(store.clone(), vec![]));
        let te = TasteEngine::new(store.clone(), reg);
        te.generate_auto_playlists(user.id);
        let pls = store.list_playlists(user.id);
        assert!(pls.iter().any(|p| p.is_auto && p.name == "My Supermix"));
        assert!(pls.iter().any(|p| p.is_auto && p.name == "Lossless Showcase"));
    }

    #[test]
    fn similar_songs_excludes_unrelated_tracks() {
        let store = Store::open_in_memory().unwrap();
        let user = store.create_user("alice", "pass", true).unwrap();
        let seed = Track {
            id: "s1".into(),
            source: "local".into(),
            title: "So What".into(),
            artist: "Miles Davis".into(),
            album: "Kind of Blue".into(),
            genre: "Jazz".into(),
            moods: "groove".into(),
            year: 1959,
            duration: 545,
            ..Default::default()
        };
        let sim_jazz = Track {
            id: "s2".into(),
            source: "local".into(),
            title: "A Love Supreme".into(),
            artist: "John Coltrane".into(),
            album: "A Love Supreme".into(),
            genre: "Jazz".into(),
            moods: "groove".into(),
            year: 1965,
            duration: 460,
            ..Default::default()
        };
        let unrelated_metal = Track {
            id: "s3".into(),
            source: "youtube".into(),
            title: "Master of Puppets".into(),
            artist: "Metallica".into(),
            album: "Master of Puppets".into(),
            genre: "Thrash Metal".into(),
            moods: "energy".into(),
            year: 1986,
            duration: 515,
            ..Default::default()
        };
        let unrelated_no_mood = Track {
            id: "s4".into(),
            source: "soulseek".into(),
            title: "Random Track".into(),
            artist: "Nobody".into(),
            album: "Misc".into(),
            genre: "".into(),
            moods: "".into(),
            year: 2024,
            duration: 180,
            ..Default::default()
        };
        store.upsert_tracks(&[seed.clone(), sim_jazz, unrelated_metal, unrelated_no_mood]);
        // Even if user starred the unrelated metal track, radio for Jazz shouldn't play it!
        store.star_track(user.id, "s3");

        let reg = Arc::new(SourceRegistry::new(store.clone(), vec![]));
        let te = TasteEngine::new(store, reg);
        let recs = te.similar_songs(&seed, user.id, 10);
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].id, "s2");
    }

    #[test]
    fn popular_match_filters_noise_and_boosts_fresh_accounts() {
        let seed = PopularSeed {
            artist: "Sabrina Carpenter".into(),
            title: "Espresso".into(),
            album: "Short n' Sweet".into(),
            cover_url: String::new(),
            duration: 175,
        };
        let good = Track {
            id: "pop1".into(),
            source: "soulseek".into(),
            title: "Espresso".into(),
            artist: "Sabrina Carpenter".into(),
            duration: 175,
            format: "flac".into(),
            ..Default::default()
        };
        let slowed_junk = Track {
            id: "junk1".into(),
            source: "youtube".into(),
            title: "Espresso (Slowed + Reverb)".into(),
            artist: "Sabrina Carpenter".into(),
            duration: 210,
            ..Default::default()
        };
        let unrelated_archive = Track {
            id: "junk2".into(),
            source: "archive".into(),
            title: "Morning Espresso Podcast Episode 12".into(),
            artist: "Random Podcaster".into(),
            duration: 1800,
            ..Default::default()
        };

        assert!(popular_match_score(&good, &seed).is_some());
        assert!(popular_match_score(&slowed_junk, &seed).is_none());
        assert!(popular_match_score(&unrelated_archive, &seed).is_none());

        let store = Store::open_in_memory().unwrap();
        let user = store.create_user("fresh", "pass", false).unwrap();
        let random_crap = Track {
            id: "rand1".into(),
            source: "archive".into(),
            title: "Obscure Bootleg 1974".into(),
            artist: "Unknown".into(),
            format: "flac".into(),
            bitrate: 1411,
            sample_rate: 96000,
            bit_depth: 24,
            ..Default::default()
        };
        let pop_mp3 = Track {
            id: "pop_mp3".into(),
            source: "youtube".into(),
            title: "Espresso".into(),
            artist: "Sabrina Carpenter".into(),
            format: "mp3".into(),
            bitrate: 192,
            ..Default::default()
        };
        store.upsert_tracks(&[random_crap, pop_mp3.clone()]);

        let reg = Arc::new(SourceRegistry::new(store.clone(), vec![]));
        let te = TasteEngine::new(store, reg);
        te.popular_ranks.lock().unwrap().insert(pop_mp3.id.clone(), 0);

        let recs = te.recommend_for_user(user.id, 2);
        assert_eq!(recs[0].id, "pop_mp3");
    }

    #[test]
    fn supermix_stays_cohesive_and_deduplicates_variants() {
        let store = Store::open_in_memory().unwrap();
        let user = store.create_user("fan", "pass", false).unwrap();

        let seed_track = Track {
            id: "r1".into(),
            source: "soulseek".into(),
            title: "ANIMA".into(),
            artist: "ReoNa".into(),
            album: "unknown".into(),
            genre: "Anime".into(),
            year: 2020,
            duration: 245,
            format: "flac".into(),
            ..Default::default()
        };
        let dup_upload = Track {
            id: "r1_yt".into(),
            source: "youtube".into(),
            title: "ANIMA [MV]".into(),
            artist: "ReoNa official YouTube channel".into(),
            genre: "Anime".into(),
            year: 2020,
            duration: 248,
            format: "m4a".into(),
            ..Default::default()
        };
        let rel_lisa = Track {
            id: "l1".into(),
            source: "soulseek".into(),
            title: "Crossing Field".into(),
            artist: "LiSA".into(),
            year: 2012,
            duration: 249,
            format: "mp3".into(),
            bitrate: 320,
            ..Default::default()
        };
        let short_clip = Track {
            id: "l_short".into(),
            source: "youtube".into(),
            title: "unlasting".into(),
            artist: "LiSA".into(),
            year: 2019,
            duration: 30,
            format: "m4a".into(),
            ..Default::default()
        };
        let slowed_junk = Track {
            id: "l_slowed".into(),
            source: "youtube".into(),
            title: "unlasting (Slowed + Reverb)".into(),
            artist: "LiSA".into(),
            year: 2019,
            duration: 230,
            format: "m4a".into(),
            ..Default::default()
        };
        let rel_aimer = Track {
            id: "a1".into(),
            source: "soulseek".into(),
            title: "Brave Shine".into(),
            artist: "Aimer".into(),
            year: 2015,
            duration: 233,
            format: "flac".into(),
            ..Default::default()
        };
        let unrelated_flac = Track {
            id: "unrel_flac".into(),
            source: "soulseek".into(),
            title: "Totally Unrelated Song".into(),
            artist: "Random Metal Band".into(),
            genre: "Metal".into(),
            year: 2024,
            duration: 310,
            format: "flac".into(),
            bit_depth: 24,
            sample_rate: 96000,
            bitrate: 1411,
            ..Default::default()
        };
        let unrelated_pop = Track {
            id: "unrel_pop".into(),
            source: "youtube".into(),
            title: "Top 40 Pop Hit".into(),
            artist: "Random Pop Star".into(),
            genre: "Pop".into(),
            year: 2024,
            duration: 195,
            format: "m4a".into(),
            ..Default::default()
        };

        store.upsert_tracks(&[
            seed_track.clone(),
            dup_upload,
            rel_lisa,
            short_clip,
            slowed_junk,
            rel_aimer,
            unrelated_flac,
            unrelated_pop.clone(),
        ]);
        store.record_event(user.id, &seed_track.id, "complete", 1.0);

        let sim_payload = CachedSimData {
            at: Utc::now().timestamp(),
            data: DeezerSimData {
                related_artists: vec!["LiSA".into(), "Aimer".into()],
                radio_tracks: vec![("LiSA".into(), "Crossing Field".into())],
                seed_genre: "Anime".into(),
            },
        };
        store.kv_set(
            "sim:v3:reona",
            &serde_json::to_string(&sim_payload).unwrap(),
        );

        let reg = Arc::new(SourceRegistry::new(store.clone(), vec![]));
        let te = TasteEngine::new(store.clone(), reg);
        te.popular_ranks.lock().unwrap().insert(unrelated_pop.id, 0);
        te.generate_auto_playlists(user.id);

        let pls = store.list_playlists(user.id);
        let supermix = pls
            .iter()
            .find(|p| p.kind == "supermix")
            .expect("supermix playlist created");
        let tracks = store.playlist_tracks(&supermix.id);
        let ids: Vec<&str> = tracks.iter().map(|t| t.id.as_str()).collect();

        assert!(ids.contains(&"r1"));
        assert!(ids.contains(&"l1"));
        assert!(ids.contains(&"a1"));
        assert!(!ids.contains(&"r1_yt"));
        assert!(!ids.contains(&"l_short"));
        assert!(!ids.contains(&"l_slowed"));
        assert!(!ids.contains(&"unrel_flac"));
        assert!(!ids.contains(&"unrel_pop"));
    }

    #[test]
    fn similar_songs_matches_cross_script_jpop() {
        let store = Store::open_in_memory().unwrap();
        let user = store.create_user("jpop", "pass", false).unwrap();

        let seed = Track {
            id: "u1".into(),
            source: "youtube".into(),
            title: "Harenohini".into(),
            artist: "Ushio Reira".into(),
            genre: "J-Pop".into(),
            duration: 276,
            ..Default::default()
        };
        let cjk_related = Track {
            id: "l_cjk".into(),
            source: "soulseek".into(),
            title: "紅蓮華".into(),
            artist: "LiSA".into(),
            duration: 237,
            format: "flac".into(),
            ..Default::default()
        };
        let unrelated = Track {
            id: "rock1".into(),
            source: "soulseek".into(),
            title: "Random Riff".into(),
            artist: "Some Western Rock Band".into(),
            genre: "Rock".into(),
            duration: 210,
            ..Default::default()
        };
        store.upsert_tracks(&[seed.clone(), cjk_related, unrelated]);

        let sim_payload = CachedSimData {
            at: Utc::now().timestamp(),
            data: DeezerSimData {
                related_artists: vec!["LiSA".into()],
                radio_tracks: vec![("LiSA".into(), "紅蓮華".into())],
                seed_genre: "J-Pop".into(),
            },
        };
        store.kv_set(
            "sim:v3:ushio-reira",
            &serde_json::to_string(&sim_payload).unwrap(),
        );

        let reg = Arc::new(SourceRegistry::new(store.clone(), vec![]));
        let te = TasteEngine::new(store, reg);
        let recs = te.similar_songs(&seed, user.id, 10);
        let ids: Vec<&str> = recs.iter().map(|t| t.id.as_str()).collect();
        assert!(ids.contains(&"l_cjk"));
        assert!(!ids.contains(&"rock1"));
    }
}
