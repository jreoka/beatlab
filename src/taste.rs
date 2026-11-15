use crate::models::{AutoMixSummary, Mood, Track, slug};
use crate::neural::{NeuralCache, UserNet, event_target, extract_features};
use crate::sources::SourceRegistry;
use crate::store::Store;
use chrono::Utc;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
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
    pub starred: HashSet<String>,
    pub ratings: HashMap<String, i32>,
    pub total_plays: usize,
}

impl TasteProfile {
    pub fn build(store: &Store, user_id: i64) -> Self {
        let mut p = Self::default();
        let events = store.user_events(user_id, 2000);
        let now = Utc::now().timestamp();

        let mut track_cache: HashMap<String, Track> = HashMap::new();
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
                    p.total_plays += 1;
                    2.2 * decay
                }
                "play" => {
                    *p.track_plays.entry(ev.track_id.clone()).or_default() += 1;
                    p.track_last_played
                        .entry(ev.track_id.clone())
                        .and_modify(|v| *v = (*v).max(ev.at))
                        .or_insert(ev.at);
                    p.total_plays += 1;
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
                    -1.0 * decay
                }
                "rate" => {
                    let r = ev.value as i32;
                    p.ratings.insert(ev.track_id.clone(), r);
                    ((r as f64) - 2.5) * 1.6 * decay
                }
                _ => 0.0,
            };

            let track = track_cache
                .entry(ev.track_id.clone())
                .or_insert_with(|| store.track(&ev.track_id).unwrap_or_default())
                .clone();

            if track.id.is_empty() {
                continue;
            }

            let a_key = track.artist.to_lowercase();
            if !a_key.is_empty() && a_key != "unknown artist" {
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
        let a_key = t.artist.to_lowercase();
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
}

pub struct TasteEngine {
    store: Store,
    registry: Arc<SourceRegistry>,
    warming: AtomicBool,
    neural: NeuralCache,
}

impl TasteEngine {
    pub fn new(store: Store, registry: Arc<SourceRegistry>) -> Self {
        Self {
            store,
            registry,
            warming: AtomicBool::new(false),
            neural: NeuralCache::new(),
        }
    }

    pub fn neural_stats(&self, user_id: i64) -> (u64, f64) {
        let net = self.neural.get_or_load(&self.store, user_id);
        (net.updates, net.avg_loss)
    }

    /// Trains the user's personal neural network online from a listening event
    /// and persists the updated weights to SQLite.
    pub fn learn_event(&self, user_id: i64, track: &Track, kind: &str, value: f64) {
        let Some((target, weight)) = event_target(kind, value) else {
            return;
        };
        let prof = self.profile(user_id);
        let a_aff = prof
            .artist_score
            .get(&track.artist.to_lowercase())
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
    }

    fn score_with_neural(&self, prof: &TasteProfile, net: &UserNet, t: &Track) -> f64 {
        let base = prof.score_track(t);
        if net.updates == 0 {
            return base;
        }
        let a_aff = prof
            .artist_score
            .get(&t.artist.to_lowercase())
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
    /// so both the Web Player and Subsonic/Navidrome clients see them.
    pub fn generate_auto_playlists(&self, user_id: i64) -> Vec<AutoMixSummary> {
        let prof = self.profile(user_id);
        let catalog = self.store.all_tracks(3000);
        let now = Utc::now().timestamp();
        let mut mixes = Vec::new();

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
            mixes.push(AutoMixSummary {
                id: format!("auto:{}:supermix", user_id),
                name: "My Supermix".into(),
                subtitle: "Your personalized all-in-one mix".into(),
                kind: "supermix".into(),
                tracks: supermix,
            });
        }

        // 2. Daily Mix 1, 2, 3 (by top genres or moods)
        let daily_clusters = self.build_daily_mixes(&prof, &catalog);
        for (idx, (title, subtitle, tracks)) in daily_clusters.into_iter().enumerate() {
            if tracks.is_empty() {
                continue;
            }
            let kind = format!("daily-mix-{}", idx + 1);
            self.save_auto_pl(user_id, &title, &subtitle, &kind, &tracks);
            mixes.push(AutoMixSummary {
                id: format!("auto:{}:{}", user_id, slug(&kind)),
                name: title,
                subtitle,
                kind,
                tracks,
            });
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
            mixes.push(AutoMixSummary {
                id: format!("auto:{}:discover", user_id),
                name: "Discover Weekly".into(),
                subtitle: "Unplayed gems matched to your taste".into(),
                kind: "discover".into(),
                tracks: discover,
            });
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
            mixes.push(AutoMixSummary {
                id: format!("auto:{}:on-repeat", user_id),
                name: "On Repeat".into(),
                subtitle: "Your heavy rotation".into(),
                kind: "on_repeat".into(),
                tracks: on_repeat,
            });
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
            mixes.push(AutoMixSummary {
                id: format!("auto:{}:forgotten-gems", user_id),
                name: "Forgotten Gems".into(),
                subtitle: "Favorites waiting for a replay".into(),
                kind: "forgotten_gems".into(),
                tracks: gems,
            });
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
            mixes.push(AutoMixSummary {
                id: format!("auto:{}:lossless", user_id),
                name: "Lossless Showcase".into(),
                subtitle: "Bit-perfect FLAC & Hi-Res tracks".into(),
                kind: "lossless".into(),
                tracks: lossless,
            });
        }

        mixes
    }

    fn save_auto_pl(&self, user_id: i64, name: &str, comment: &str, kind: &str, tracks: &[Track]) {
        let ids: Vec<String> = tracks.iter().map(|t| t.id.clone()).collect();
        let _ = self
            .store
            .create_playlist(user_id, name, comment, false, true, kind, &ids);
    }

    fn build_supermix(&self, prof: &TasteProfile, net: &UserNet, catalog: &[Track], limit: usize) -> Vec<Track> {
        let mut scored: Vec<(f64, &Track)> = catalog
            .iter()
            .filter(|t| !is_junk_track(t))
            .map(|t| (self.score_with_neural(prof, net, t), t))
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        mmr_diversify(
            scored.into_iter().map(|(_, t)| t.clone()).collect(),
            limit,
            2,
        )
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
        let mut unplayed: Vec<(f64, Track)> = catalog
            .iter()
            .filter(|t| {
                !is_junk_track(t)
                    && prof.track_plays.get(&t.id).copied().unwrap_or(0) == 0
                    && t.play_count == 0
            })
            .map(|t| (prof.score_track(t), t.clone()))
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
        played.sort_by(|a, b| b.0.cmp(&a.0));
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

    pub fn similar_songs(&self, seed: &Track, user_id: i64, limit: usize) -> Vec<Track> {
        let prof = self.profile(user_id);
        let catalog = self.store.all_tracks(2500);
        let seed_key = seed.song_key();
        let seed_aliases: HashSet<String> = seed.song_aliases().into_iter().collect();
        let seed_cjk = has_cjk(&seed.title) || has_cjk(&seed.artist);
        let seed_artist = seed.artist.to_lowercase();
        let seed_moods: HashSet<Mood> = seed.mood_list().into_iter().collect();

        let mut scored: Vec<(f64, Track)> = Vec::new();
        for t in catalog {
            if t.id == seed.id || is_junk_track(&t) {
                continue;
            }
            let tk = t.song_key();
            if tk == seed_key || seed_aliases.contains(&tk) {
                continue;
            }
            let t_cjk = has_cjk(&t.title) || has_cjk(&t.artist);
            if seed_cjk != t_cjk {
                continue;
            }

            let mut s = 0.0;
            let ta = t.artist.to_lowercase();
            if ta == seed_artist {
                s += 8.0;
            } else if let Some(co) = prof.cooccurrence.get(&seed_artist).and_then(|m| m.get(&ta)) {
                s += co * 3.5;
            }
            if !seed.genre.is_empty() && t.genre.eq_ignore_ascii_case(&seed.genre) {
                s += 5.0;
            }
            let t_moods: HashSet<Mood> = t.mood_list().into_iter().collect();
            let mood_overlap = seed_moods.intersection(&t_moods).count();
            s += (mood_overlap as f64) * 2.5;

            if seed.year > 1940 && t.year > 1940 && (seed.year - t.year).abs() <= 8 {
                s += 1.5;
            }

            s += prof.score_track(&t) * 0.35;
            if s > 0.0 {
                scored.push((s, t));
            }
        }

        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        mmr_diversify(scored.into_iter().map(|(_, t)| t).collect(), limit, 3)
    }

    pub async fn artist_radio(&self, artist: &str, user_id: i64, limit: usize) -> Vec<Track> {
        let mut tracks = self.store.tracks_by_artist(artist, limit);
        if tracks.len() < limit / 2 {
            let fetched = self.registry.search_all(artist, limit).await;
            tracks.extend(fetched);
        }
        let prof = self.profile(user_id);
        tracks.retain(|t| !is_junk_track(t));
        tracks.sort_by(|a, b| {
            prof.score_track(b)
                .partial_cmp(&prof.score_track(a))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        mmr_diversify(tracks, limit, 4)
    }

    pub async fn mood_radio(&self, mood: Mood, user_id: i64, limit: usize) -> Vec<Track> {
        let mut tracks = self.store.tracks_by_mood(mood.slug(), limit * 2);
        if tracks.len() < 8 {
            let query = match mood {
                Mood::Chill => "lofi chill downtempo ambient",
                Mood::Focus => "instrumental ambient study focus",
                Mood::Energy => "electronic rock upbeat workout",
                Mood::LateNight => "midnight jazz ambient electronic",
                Mood::Upbeat => "indie pop funk upbeat",
                Mood::Melancholy => "acoustic indie sad ballad",
                Mood::Party => "house dance electronic funk",
                Mood::Acoustic => "acoustic folk unplugged session",
                Mood::Groove => "soul funk jazz r&b",
                Mood::RockIndie => "indie alternative rock",
            };
            let fetched = self.registry.search_all(query, 15).await;
            tracks.extend(fetched);
        }
        let prof = self.profile(user_id);
        tracks.retain(|t| !is_junk_track(t));
        tracks.sort_by(|a, b| {
            prof.score_track(b)
                .partial_cmp(&prof.score_track(a))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        mmr_diversify(tracks, limit, 2)
    }

    /// Background catalog warmer so a fresh server immediately has diverse, high-quality music
    /// across moods and genres instead of an empty shelf.
    pub async fn warm_catalog(&self) {
        if self.warming.swap(true, Ordering::SeqCst) {
            return;
        }
        let existing = self.store.track_count();
        if existing < 30 {
            info!("warming fresh catalog in background...");
            let seeds = [
                "jazz lossless flac",
                "electronic ambient",
                "indie rock live",
                "classical piano",
                "funk soul groove",
                "acoustic folk session",
            ];
            for q in seeds {
                let _ = self.registry.search_all(q, 8).await;
                tokio::time::sleep(std::time::Duration::from_millis(400)).await;
            }
            info!(tracks = self.store.track_count(), "catalog warm complete");
        }
        self.warming.store(false, Ordering::SeqCst);
    }
}

/// Deduplicates by `song_key` and caps tracks per artist (`max_per_artist`) to keep mixes fresh.
pub fn mmr_diversify(tracks: Vec<Track>, limit: usize, max_per_artist: usize) -> Vec<Track> {
    let mut seen_keys = HashSet::new();
    let mut artist_counts: HashMap<String, usize> = HashMap::new();
    let mut out = Vec::with_capacity(limit);

    for t in tracks {
        if out.len() >= limit {
            break;
        }
        let key = t.song_key();
        if seen_keys.contains(&key) {
            continue;
        }
        let a_key = t.artist.to_lowercase();
        let cnt = artist_counts.get(&a_key).copied().unwrap_or(0);
        if cnt >= max_per_artist {
            continue;
        }
        seen_keys.insert(key);
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
    {
        push(Mood::Chill);
    }
    if hay.contains("ambient")
        || hay.contains("study")
        || hay.contains("focus")
        || hay.contains("instrumental")
        || hay.contains("classical")
        || hay.contains("piano")
    {
        push(Mood::Focus);
    }
    if hay.contains("metal")
        || hay.contains("punk")
        || hay.contains("workout")
        || hay.contains("hardcore")
        || hay.contains("drum and bass")
        || hay.contains("dnb")
    {
        push(Mood::Energy);
    }
    if hay.contains("night")
        || hay.contains("midnight")
        || hay.contains("nocturne")
        || hay.contains("dark")
        || hay.contains("ambient")
    {
        push(Mood::LateNight);
    }
    if hay.contains("pop")
        || hay.contains("happy")
        || hay.contains("sun")
        || hay.contains("summer")
        || hay.contains("bright")
    {
        push(Mood::Upbeat);
    }
    if hay.contains("sad")
        || hay.contains("blues")
        || hay.contains("melanchol")
        || hay.contains("lonely")
        || hay.contains("ballad")
    {
        push(Mood::Melancholy);
    }
    if hay.contains("house")
        || hay.contains("techno")
        || hay.contains("dance")
        || hay.contains("electronic")
        || hay.contains("disco")
        || hay.contains("club")
    {
        push(Mood::Party);
        push(Mood::Energy);
    }
    if hay.contains("acoustic")
        || hay.contains("folk")
        || hay.contains("unplugged")
        || hay.contains("singer")
    {
        push(Mood::Acoustic);
    }
    if hay.contains("jazz")
        || hay.contains("soul")
        || hay.contains("funk")
        || hay.contains("r&b")
        || hay.contains("groove")
    {
        push(Mood::Groove);
        push(Mood::Chill);
    }
    if hay.contains("rock") || hay.contains("indie") || hay.contains("alternative") {
        push(Mood::RockIndie);
    }

    if moods.is_empty() {
        moods.push(Mood::Chill);
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

fn has_cjk(s: &str) -> bool {
    s.chars().any(|c| {
        ('\u{3000}'..='\u{30FF}').contains(&c)
            || ('\u{4E00}'..='\u{9FFF}').contains(&c)
            || ('\u{FF00}'..='\u{FFEF}').contains(&c)
    })
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
        store.upsert_track(&t1).unwrap();
        store.upsert_track(&t2).unwrap();
        store.star_track(user.id, &t1.id);

        let reg = Arc::new(SourceRegistry::new(store.clone(), vec![]));
        let te = TasteEngine::new(store.clone(), reg);
        let mixes = te.generate_auto_playlists(user.id);
        assert!(mixes.iter().any(|m| m.kind == "supermix"));
        assert!(mixes.iter().any(|m| m.kind == "lossless"));
        let pls = store.list_playlists(user.id);
        assert!(pls.iter().any(|p| p.is_auto && p.name == "My Supermix"));
    }
}
