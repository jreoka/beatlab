use crate::models::{
    primary_artist, slug, AlbumSummary, ArtistSummary, Mood, Track, UserTrackStats,
};
use crate::neural::{event_target, extract_features, NeuralCache};
use crate::sources::SourceRegistry;
use crate::store::Store;
use chrono::Utc;
use std::collections::{HashMap, HashSet};
use tracing::info;

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

#[derive(Default, Clone)]
pub struct TasteProfile {
    pub artist_affinity: HashMap<String, f64>,
    pub genre_affinity: HashMap<String, f64>,
    pub mood_affinity: HashMap<Mood, f64>,
    pub starred: HashSet<String>,
    pub ratings: HashMap<String, i32>,
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

    pub fn record_event(&self, user_id: i64, track_id: &str, kind: &str, value: f64) {
        let _ = self.store.record_event(user_id, track_id, kind, value);
        if let Some((target, weight)) = event_target(kind, value) {
            if let Ok(Some(track)) = self.store.get_track(track_id) {
                let prof = self.user_profile(user_id);
                let st = self
                    .store
                    .get_user_track_stats(user_id, track_id)
                    .unwrap_or_default();
                let a_aff = prof
                    .artist_affinity
                    .get(&slug(&primary_artist(&track.artist)))
                    .copied()
                    .unwrap_or(0.0);
                let g_aff = prof
                    .genre_affinity
                    .get(&track.genre.to_lowercase())
                    .copied()
                    .unwrap_or(0.0);
                let feats = extract_features(&track, a_aff, g_aff, st.play_count, st.skip_count);
                self.neural.update_and_save(&self.store, user_id, |net| {
                    net.train_step(&feats, target, weight);
                });
            }
        }
    }

    pub fn user_profile(&self, user_id: i64) -> TasteProfile {
        let mut prof = TasteProfile::default();
        if user_id <= 0 {
            return prof;
        }
        prof.starred = self.store.starred_ids(user_id).unwrap_or_default();
        prof.ratings = self.store.user_ratings(user_id).unwrap_or_default();
        let events = self.store.user_recent_events(user_id, 500).unwrap_or_default();
        let ids: Vec<String> = events
            .iter()
            .map(|e| e.track_id.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let track_map: HashMap<String, Track> = self
            .store
            .get_tracks_by_ids(&ids)
            .unwrap_or_default()
            .into_iter()
            .map(|t| (t.id.clone(), t))
            .collect();

        let now = Utc::now().timestamp();
        for ev in events {
            let Some(t) = track_map.get(&ev.track_id) else {
                continue;
            };
            let age_days = ((now - ev.at).max(0) as f64) / 86400.0;
            let decay = (-age_days / 21.0).exp();
            let base_w = match ev.kind.as_str() {
                "star" => 3.5,
                "complete" => 2.0,
                "play" => 1.0,
                "rate" => (ev.value - 2.5) * 1.5,
                "skip" => -1.8,
                "unstar" => -1.5,
                _ => 0.2,
            };
            let w = base_w * decay;

            let a_key = slug(&primary_artist(&t.artist));
            *prof.artist_affinity.entry(a_key).or_insert(0.0) += w;
            for g in t.genre.split(&[',', ';', '/'][..]) {
                let gl = g.trim().to_lowercase();
                if !gl.is_empty() {
                    *prof.genre_affinity.entry(gl).or_insert(0.0) += w;
                }
            }
            for m in t.mood_list() {
                *prof.mood_affinity.entry(m).or_insert(0.0) += w;
            }
        }
        prof
    }

    pub fn score_track(
        &self,
        user_id: i64,
        t: &Track,
        prof: &TasteProfile,
        stats: &HashMap<String, UserTrackStats>,
    ) -> f64 {
        let a_aff = prof
            .artist_affinity
            .get(&slug(&primary_artist(&t.artist)))
            .copied()
            .unwrap_or(0.0);
        let g_aff = t
            .genre
            .split(&[',', ';', '/'][..])
            .filter_map(|g| prof.genre_affinity.get(&g.trim().to_lowercase()))
            .fold(0.0f64, |a, b| a.max(*b));
        let m_aff = t
            .mood_list()
            .iter()
            .filter_map(|m| prof.mood_affinity.get(m))
            .sum::<f64>();

        let st = stats.get(&t.id).copied().unwrap_or_default();
        let net_score = if user_id > 0 {
            let net = self.neural.get_or_load(&self.store, user_id);
            // Ignore untrained random weights on cold-start users until real feedback accumulates.
            if net.updates == 0 {
                0.0
            } else {
                let conf = (net.updates as f64 / 5.0).clamp(0.0, 1.0);
                let x = extract_features(t, a_aff, g_aff, st.play_count, st.skip_count);
                (net.predict(&x) - 0.5) * 10.0 * conf
            }
        } else {
            0.0
        };

        let pop_bonus = parse_rank_bonus(&t.meta);
        let star_bonus = if prof.starred.contains(&t.id) { 3.5 } else { 0.0 };
        let rating_bonus = prof
            .ratings
            .get(&t.id)
            .map(|r| (*r as f64 - 2.5) * 1.4)
            .unwrap_or(0.0);
        let quality_bonus = (t.quality_score() as f64) / 400.0;

        let now = Utc::now().timestamp();
        let recency_penalty = if st.last_played > 0 && (now - st.last_played) < 3600 * 6 {
            -4.5
        } else {
            0.0
        };
        let skip_penalty = (st.skip_count as f64) * 1.8;

        net_score
            + pop_bonus
            + a_aff * 1.15
            + g_aff * 0.9
            + m_aff * 0.45
            + star_bonus
            + rating_bonus
            + quality_bonus
            + recency_penalty
            - skip_penalty
    }

    pub fn for_you_tracks(&self, user_id: i64, limit: usize) -> Vec<Track> {
        let candidates = self.store.all_tracks(800, 0).unwrap_or_default();
        if candidates.is_empty() {
            return vec![];
        }
        let prof = self.user_profile(user_id);
        let stats = if user_id > 0 {
            self.store.track_stats_map(user_id).unwrap_or_default()
        } else {
            HashMap::new()
        };

        let mut scored: Vec<(f64, Track)> = candidates
            .into_iter()
            .map(|t| {
                let s = self.score_track(user_id, &t, &prof, &stats);
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
        self.store
            .user_frequent_tracks(user_id, limit * 2, 0)
            .unwrap_or_default()
            .into_iter()
            .map(|(t, _)| t)
            .take(limit)
            .collect()
    }

    pub fn forgotten_gems(&self, user_id: i64, limit: usize) -> Vec<Track> {
        let now = Utc::now().timestamp();
        let cutoff = now - 7 * 86400;
        let stats = self.store.track_stats_map(user_id).unwrap_or_default();
        let starred = self.store.starred_tracks(user_id).unwrap_or_default();

        let mut pool = Vec::new();
        for (t, at) in starred {
            let lp = stats.get(&t.id).map(|s| s.last_played).unwrap_or(at);
            if lp < cutoff {
                pool.push(t);
            }
        }
        if pool.len() < limit {
            for t in self.for_you_tracks(user_id, limit * 2) {
                if !pool.iter().any(|p| p.id == t.id) {
                    let st = stats.get(&t.id).copied().unwrap_or_default();
                    if st.play_count > 0 && st.last_played < cutoff {
                        pool.push(t);
                    }
                }
            }
        }
        pool.truncate(limit);
        pool
    }

    pub fn discover_weekly(&self, user_id: i64, limit: usize) -> Vec<Track> {
        let stats = self.store.track_stats_map(user_id).unwrap_or_default();
        let unplayed: Vec<Track> = self
            .for_you_tracks(user_id, limit * 5)
            .into_iter()
            .filter(|t| stats.get(&t.id).map(|s| s.play_count).unwrap_or(0) == 0)
            .collect();
        // Skip the top couple tracks/artists from Supermix so Discover Weekly gets a distinct lead & cover.
        let mut diversified: Vec<Track> = unplayed.iter().skip(2).cloned().collect();
        diversified.extend(unplayed.into_iter().take(2));
        dedup_and_interleave(diversified, limit, 1)
    }

    pub fn rebuild_auto_playlists(&self, user_id: i64) {
        if user_id <= 0 {
            return;
        }
        let supermix: Vec<String> = self
            .for_you_tracks(user_id, 35)
            .into_iter()
            .map(|t| t.id)
            .collect();
        if !supermix.is_empty() {
            let _ = self.store.upsert_auto_playlist(
                &format!("auto-u{}-supermix", user_id),
                user_id,
                "My Supermix",
                "Personalized blend of your favorites & fresh discoveries",
                "supermix",
                &supermix,
            );
        }

        let discover: Vec<String> = self
            .discover_weekly(user_id, 30)
            .into_iter()
            .map(|t| t.id)
            .collect();
        if !discover.is_empty() {
            let _ = self.store.upsert_auto_playlist(
                &format!("auto-u{}-discover", user_id),
                user_id,
                "Discover Weekly",
                "Fresh tracks tailored to your taste profile",
                "discover",
                &discover,
            );
        }

        let repeat: Vec<String> = self
            .on_repeat_tracks(user_id, 25)
            .into_iter()
            .map(|t| t.id)
            .collect();
        if !repeat.is_empty() {
            let _ = self.store.upsert_auto_playlist(
                &format!("auto-u{}-repeat", user_id),
                user_id,
                "On Repeat",
                "Songs you can't stop playing",
                "repeat",
                &repeat,
            );
        }

        let gems: Vec<String> = self
            .forgotten_gems(user_id, 25)
            .into_iter()
            .map(|t| t.id)
            .collect();
        if !gems.is_empty() {
            let _ = self.store.upsert_auto_playlist(
                &format!("auto-u{}-gems", user_id),
                user_id,
                "Forgotten Favorites",
                "Loved tracks you haven't spun lately",
                "gems",
                &gems,
            );
        }
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

    pub async fn radio_next_tracks(
        &self,
        user_id: i64,
        seed: Option<&Track>,
        count: usize,
    ) -> Vec<Track> {
        if let Some(s) = seed {
            let sims = self.similar_songs_async(s, count).await;
            if !sims.is_empty() {
                return sims;
            }
        }
        self.for_you_tracks(user_id, count)
    }
}

fn dedup_and_interleave(tracks: Vec<Track>, limit: usize, max_per_artist: usize) -> Vec<Track> {
    let mut seen_keys = HashSet::new();
    let mut artist_counts: HashMap<String, usize> = HashMap::new();
    let mut out = Vec::with_capacity(limit.min(tracks.len()));

    for t in tracks {
        if out.len() >= limit {
            break;
        }
        if !seen_keys.insert(t.song_key()) {
            continue;
        }
        let ak = slug(&primary_artist(&t.artist));
        let c = artist_counts.entry(ak).or_insert(0);
        if *c >= max_per_artist {
            continue;
        }
        *c += 1;
        out.push(t);
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
}
