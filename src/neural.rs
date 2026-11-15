use crate::models::{Mood, Track};
use crate::store::Store;
use chrono::{Timelike, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::RwLock;

/// Rich 48-dimensional transferable track+context feature vector:
/// - `[0..10]`  mood one-hot / multi-hot indicators
/// - `[10..18]` broad genre family indicators
/// - `[18..22]` lossless flag, normalized bitrate, normalized duration, local-file flag
/// - `[22..28]` era buckets (<1970, 1970s, 1980s, 1990s, 2000s, 2010+)
/// - `[28..30]` cyclical time of day `(sin, cos)`
/// - `[30..33]` artist affinity, genre affinity, mood affinity (`tanh`-compressed)
/// - `[33..36]` curated playlist artist / genre / mood overlap affinities
/// - `[36..38]` followed-artist & saved-album indicators
/// - `[38..41]` track novelty, skip-risk ratio, popularity/rank prior
/// - `[41..46]` active search/mix query context (active flag, lexical fit, era match, theme match, hits synergy)
/// - `[46..48]` artist-mood synergy interaction & bias term
pub const INPUT_DIM: usize = 48;
const H1_DIM: usize = 32;
const H2_DIM: usize = 16;
const NET_VERSION: u32 = 3;
const REPLAY_CAP: usize = 96;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ReplaySample {
    x: Vec<f32>,
    target: f32,
    weight: f32,
}

#[derive(Debug, Clone, Default)]
pub struct FeatureContext {
    pub artist_affinity: f64,
    pub genre_affinity: f64,
    pub mood_affinity: f64,
    pub playlist_artist_affinity: f64,
    pub playlist_genre_affinity: f64,
    pub playlist_mood_affinity: f64,
    pub followed_artist: bool,
    pub saved_album: bool,
    pub track_plays: i32,
    pub track_skips: i32,
    pub popularity_prior: f64,
    pub hour_override: Option<u32>,
    pub query_active: bool,
    pub query_lexical_fit: f64,
    pub query_era_match: f64,
    pub query_theme_match: f64,
    pub query_popularity_intent: f64,
}

/// Per-user compact 3-layer neural preference model (`40 -> 32 -> 16 -> 1`)
/// trained online with momentum SGD, gradient clipping, L2 regularization, and
/// bounded experience replay to prevent catastrophic forgetting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserNet {
    #[serde(default = "default_net_version")]
    pub version: u32,
    pub user_id: i64,
    pub updates: u64,
    w1: Vec<f64>,
    b1: Vec<f64>,
    w2: Vec<f64>,
    b2: Vec<f64>,
    w3: Vec<f64>,
    b3: f64,
    #[serde(default)]
    v_w1: Vec<f64>,
    #[serde(default)]
    v_b1: Vec<f64>,
    #[serde(default)]
    v_w2: Vec<f64>,
    #[serde(default)]
    v_b2: Vec<f64>,
    #[serde(default)]
    v_w3: Vec<f64>,
    #[serde(default)]
    v_b3: f64,
    #[serde(default)]
    replay: Vec<ReplaySample>,
}

fn default_net_version() -> u32 {
    1
}

impl UserNet {
    pub fn new(user_id: i64) -> Self {
        let mut seed = (user_id as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(0xD1B5_4A32_D192_ED03);
        let mut init_vec = |len: usize, scale: f64| -> Vec<f64> {
            (0..len)
                .map(|_| {
                    seed = seed
                        .wrapping_mul(6364136223846793005)
                        .wrapping_add(1442695040888963407);
                    let u = (((seed >> 32) as u32) as f64) / (u32::MAX as f64);
                    (u * 2.0 - 1.0) * scale
                })
                .collect()
        };

        let mut net = Self {
            version: NET_VERSION,
            user_id,
            updates: 0,
            w1: init_vec(H1_DIM * INPUT_DIM, 0.22),
            b1: vec![0.0; H1_DIM],
            w2: init_vec(H2_DIM * H1_DIM, 0.25),
            b2: vec![0.0; H2_DIM],
            w3: init_vec(H2_DIM, 0.35),
            b3: 0.0,
            v_w1: vec![0.0; H1_DIM * INPUT_DIM],
            v_b1: vec![0.0; H1_DIM],
            v_w2: vec![0.0; H2_DIM * H1_DIM],
            v_b2: vec![0.0; H2_DIM],
            v_w3: vec![0.0; H2_DIM],
            v_b3: 0.0,
            replay: Vec::with_capacity(REPLAY_CAP),
        };
        net.ensure_buffers();
        net
    }

    pub fn is_compatible(&self) -> bool {
        self.version == NET_VERSION
            && self.w1.len() == H1_DIM * INPUT_DIM
            && self.b1.len() == H1_DIM
            && self.w2.len() == H2_DIM * H1_DIM
            && self.b2.len() == H2_DIM
            && self.w3.len() == H2_DIM
    }

    fn ensure_buffers(&mut self) {
        if self.v_w1.len() != H1_DIM * INPUT_DIM {
            self.v_w1 = vec![0.0; H1_DIM * INPUT_DIM];
        }
        if self.v_b1.len() != H1_DIM {
            self.v_b1 = vec![0.0; H1_DIM];
        }
        if self.v_w2.len() != H2_DIM * H1_DIM {
            self.v_w2 = vec![0.0; H2_DIM * H1_DIM];
        }
        if self.v_b2.len() != H2_DIM {
            self.v_b2 = vec![0.0; H2_DIM];
        }
        if self.v_w3.len() != H2_DIM {
            self.v_w3 = vec![0.0; H2_DIM];
        }
        if !self.v_b3.is_finite() {
            self.v_b3 = 0.0;
        }
    }

    pub fn load_or_new(store: &Store, user_id: i64) -> Self {
        let key = format!("nn:user:{}", user_id);
        if let Some(raw) = store.kv_get(&key) {
            if let Ok(mut net) = serde_json::from_str::<UserNet>(&raw) {
                if net.is_compatible() && net.weights_finite() {
                    net.ensure_buffers();
                    return net;
                }
            }
        }
        Self::new(user_id)
    }

    pub fn save(&self, store: &Store) {
        let key = format!("nn:user:{}", self.user_id);
        if let Ok(json) = serde_json::to_string(self) {
            store.kv_set(&key, &json);
        }
    }

    pub fn confidence(&self) -> f64 {
        (self.updates as f64 / 28.0).clamp(0.0, 1.0)
    }

    fn weights_finite(&self) -> bool {
        self.b3.is_finite()
            && self.w1.iter().all(|v| v.is_finite())
            && self.b1.iter().all(|v| v.is_finite())
            && self.w2.iter().all(|v| v.is_finite())
            && self.b2.iter().all(|v| v.is_finite())
            && self.w3.iter().all(|v| v.is_finite())
    }

    fn forward(&self, x: &[f64; INPUT_DIM]) -> ([f64; H1_DIM], [f64; H2_DIM], f64) {
        let mut h1 = [0.0; H1_DIM];
        for i in 0..H1_DIM {
            let row = i * INPUT_DIM;
            let mut sum = self.b1[i];
            for j in 0..INPUT_DIM {
                let xj = if x[j].is_finite() { x[j] } else { 0.0 };
                sum += self.w1[row + j] * xj;
            }
            h1[i] = leaky_relu(sum);
        }

        let mut h2 = [0.0; H2_DIM];
        for i in 0..H2_DIM {
            let row = i * H1_DIM;
            let mut sum = self.b2[i];
            for j in 0..H1_DIM {
                sum += self.w2[row + j] * h1[j];
            }
            h2[i] = leaky_relu(sum);
        }

        let mut out_logit = self.b3;
        for i in 0..H2_DIM {
            out_logit += self.w3[i] * h2[i];
        }
        let prob = sigmoid(out_logit.clamp(-16.0, 16.0));
        (h1, h2, prob)
    }

    pub fn predict(&self, x: &[f64; INPUT_DIM]) -> f64 {
        let (_, _, p) = self.forward(x);
        if p.is_finite() {
            p.clamp(0.0, 1.0)
        } else {
            0.5
        }
    }

    /// Performs one momentum-SGD gradient step without modifying the replay ring buffer.
    fn apply_gradient_step(&mut self, x: &[f64; INPUT_DIM], target: f64, lr: f64) {
        if !x.iter().all(|v| v.is_finite()) || !target.is_finite() || !lr.is_finite() {
            return;
        }
        self.ensure_buffers();
        let target = target.clamp(0.0, 1.0);
        let lr = lr.clamp(0.0005, 0.25);
        let momentum = 0.68;
        let l2 = 1e-4;

        let (h1, h2, pred) = self.forward(x);
        let d_out = (pred - target).clamp(-2.0, 2.0);

        let mut d_h2 = [0.0; H2_DIM];
        for i in 0..H2_DIM {
            d_h2[i] = d_out * self.w3[i] * d_leaky_relu(h2[i]);
        }

        let mut d_h1 = [0.0; H1_DIM];
        for j in 0..H1_DIM {
            let mut acc = 0.0;
            for i in 0..H2_DIM {
                acc += d_h2[i] * self.w2[i * H1_DIM + j];
            }
            d_h1[j] = acc * d_leaky_relu(h1[j]);
        }

        // Layer 3 update with momentum
        for i in 0..H2_DIM {
            let g = (d_out * h2[i] + l2 * self.w3[i]).clamp(-3.0, 3.0);
            self.v_w3[i] = momentum * self.v_w3[i] - lr * g;
            self.w3[i] = (self.w3[i] + self.v_w3[i]).clamp(-6.0, 6.0);
        }
        let g_b3 = d_out.clamp(-3.0, 3.0);
        self.v_b3 = momentum * self.v_b3 - lr * g_b3;
        self.b3 = (self.b3 + self.v_b3).clamp(-6.0, 6.0);

        // Layer 2 update
        for i in 0..H2_DIM {
            let dh = d_h2[i];
            let row = i * H1_DIM;
            for j in 0..H1_DIM {
                let g = (dh * h1[j] + l2 * self.w2[row + j]).clamp(-3.0, 3.0);
                self.v_w2[row + j] = momentum * self.v_w2[row + j] - lr * g;
                self.w2[row + j] = (self.w2[row + j] + self.v_w2[row + j]).clamp(-6.0, 6.0);
            }
            let gb = dh.clamp(-3.0, 3.0);
            self.v_b2[i] = momentum * self.v_b2[i] - lr * gb;
            self.b2[i] = (self.b2[i] + self.v_b2[i]).clamp(-6.0, 6.0);
        }

        // Layer 1 update
        for i in 0..H1_DIM {
            let dh = d_h1[i];
            let row = i * INPUT_DIM;
            for j in 0..INPUT_DIM {
                let g = (dh * x[j] + l2 * self.w1[row + j]).clamp(-3.0, 3.0);
                self.v_w1[row + j] = momentum * self.v_w1[row + j] - lr * g;
                self.w1[row + j] = (self.w1[row + j] + self.v_w1[row + j]).clamp(-6.0, 6.0);
            }
            let gb = dh.clamp(-3.0, 3.0);
            self.v_b1[i] = momentum * self.v_b1[i] - lr * gb;
            self.b1[i] = (self.b1[i] + self.v_b1[i]).clamp(-6.0, 6.0);
        }
    }

    pub fn train_step(&mut self, x: &[f64; INPUT_DIM], target: f64, sample_weight: f64) {
        if !x.iter().all(|v| v.is_finite()) || !target.is_finite() || !sample_weight.is_finite() {
            return;
        }
        let target = target.clamp(0.0, 1.0);
        let sw = sample_weight.clamp(0.1, 3.5);
        let lr = 0.045 * sw;
        self.apply_gradient_step(x, target, lr);

        // Interleave up to 3 strided historical samples from replay buffer to preserve older taste.
        if !self.replay.is_empty() {
            let n = self.replay.len();
            let picks = n.min(3);
            let stride = (n / picks).max(1);
            let offset = (self.updates as usize) % n;
            let replay_samples: Vec<ReplaySample> = (0..picks)
                .map(|k| self.replay[(offset + k * stride) % n].clone())
                .collect();
            for sample in replay_samples {
                if sample.x.len() == INPUT_DIM {
                    let mut arr = [0.0; INPUT_DIM];
                    for (dst, &src) in arr.iter_mut().zip(sample.x.iter()) {
                        *dst = src as f64;
                    }
                    self.apply_gradient_step(
                        &arr,
                        sample.target as f64,
                        0.02 * (sample.weight as f64).clamp(0.1, 3.0),
                    );
                }
            }
        }

        if self.replay.len() >= REPLAY_CAP {
            self.replay.remove(0);
        }
        self.replay.push(ReplaySample {
            x: x.iter().map(|&v| v as f32).collect(),
            target: target as f32,
            weight: sw as f32,
        });
        self.updates += 1;
    }

    /// Batch-trains on historical interactions (used when migrating or cold-bootstrapping).
    pub fn train_batch(&mut self, samples: &[([f64; INPUT_DIM], f64, f64)], epochs: usize) {
        if samples.is_empty() || epochs == 0 {
            return;
        }
        for _ in 0..epochs {
            for &(ref x, target, weight) in samples {
                if x.iter().all(|v| v.is_finite()) && target.is_finite() && weight.is_finite() {
                    self.apply_gradient_step(
                        x,
                        target.clamp(0.0, 1.0),
                        0.024 * weight.clamp(0.1, 3.0),
                    );
                }
            }
        }
        self.replay.clear();
        let start = samples.len().saturating_sub(REPLAY_CAP);
        for &(ref x, target, weight) in &samples[start..] {
            if x.iter().all(|v| v.is_finite()) && target.is_finite() && weight.is_finite() {
                self.replay.push(ReplaySample {
                    x: x.iter().map(|&v| v as f32).collect(),
                    target: target.clamp(0.0, 1.0) as f32,
                    weight: weight.clamp(0.1, 3.0) as f32,
                });
            }
        }
        self.updates += samples.len() as u64;
    }
}

fn leaky_relu(z: f64) -> f64 {
    if z >= 0.0 {
        z
    } else {
        0.05 * z
    }
}

fn d_leaky_relu(act: f64) -> f64 {
    if act >= 0.0 {
        1.0
    } else {
        0.05
    }
}

fn sigmoid(z: f64) -> f64 {
    1.0 / (1.0 + (-z).exp())
}

fn clean_finite(v: f64) -> f64 {
    if v.is_finite() {
        v
    } else {
        0.0
    }
}

/// Extracts a 40-element transferable feature vector from track attributes and user context.
pub fn extract_features_ctx(track: &Track, ctx: &FeatureContext) -> [f64; INPUT_DIM] {
    let mut f = [0.0; INPUT_DIM];

    // 0..10 moods
    let track_moods = track.mood_list();
    for (idx, m) in Mood::ALL.iter().enumerate() {
        if track_moods.contains(m) {
            f[idx] = 1.0;
        }
    }

    // 10..18 broad genre buckets
    let g = track.genre.to_lowercase();
    let genre_buckets: [&[&str]; 8] = [
        &["rock", "alt", "punk", "grunge", "new wave"],
        &["pop", "synthpop", "dance", "disco"],
        &["electronic", "house", "techno", "edm", "ambient", "trance", "dnb", "synthwave"],
        &["hip hop", "hip-hop", "rap", "trap", "r&b", "soul", "funk"],
        &["jazz", "blues", "bossanova", "swing", "vocal"],
        &["classical", "orchestra", "piano", "soundtrack", "score"],
        &["indie", "folk", "acoustic", "singer", "country"],
        &["metal", "hardcore", "industrial"],
    ];
    for (idx, keys) in genre_buckets.iter().enumerate() {
        if keys.iter().any(|k| g.contains(k)) {
            f[10 + idx] = 1.0;
        }
    }

    // 18..22 audio & format characteristics
    f[18] = if track.bitrate >= 500
        || matches!(track.format.as_str(), "flac" | "alac" | "wav")
    {
        1.0
    } else {
        0.0
    };
    f[19] = (track.bitrate.max(0) as f64 / 1411.0).clamp(0.0, 1.0);
    f[20] = (track.duration.max(0) as f64 / 600.0).clamp(0.0, 1.0);
    f[21] = if track.source == "local" { 1.0 } else { 0.0 };

    // 22..28 era buckets (<1970, 70s, 80s, 90s, 2000s, 2010+)
    if track.year > 0 {
        let idx = match track.year {
            ..=1969 => 22,
            1970..=1979 => 23,
            1980..=1989 => 24,
            1990..=1999 => 25,
            2000..=2009 => 26,
            _ => 27,
        };
        f[idx] = 1.0;
    }

    // 28..30 time of day cyclical features
    let hour = ctx
        .hour_override
        .unwrap_or_else(|| Utc::now().hour())
        .min(23) as f64;
    let angle = 2.0 * std::f64::consts::PI * (hour / 24.0);
    f[28] = angle.sin();
    f[29] = angle.cos();

    // 30..33 taste affinities
    let a_aff = (clean_finite(ctx.artist_affinity) / 3.0).tanh();
    let g_aff = (clean_finite(ctx.genre_affinity) / 3.0).tanh();
    let m_aff = (clean_finite(ctx.mood_affinity) / 2.5).tanh();
    f[30] = a_aff;
    f[31] = g_aff;
    f[32] = m_aff;

    // 33..36 playlist curation context affinities
    f[33] = (clean_finite(ctx.playlist_artist_affinity) / 2.5).tanh();
    f[34] = (clean_finite(ctx.playlist_genre_affinity) / 2.5).tanh();
    f[35] = (clean_finite(ctx.playlist_mood_affinity) / 2.0).tanh();

    // 36..38 explicit entity affinity flags
    f[36] = if ctx.followed_artist { 1.0 } else { 0.0 };
    f[37] = if ctx.saved_album { 1.0 } else { 0.0 };

    // 38..41 behavioral & popularity priors
    let plays = ctx.track_plays.max(0) as f64;
    let skips = ctx.track_skips.max(0) as f64;
    let pop = clean_finite(ctx.popularity_prior).clamp(0.0, 1.0);
    f[38] = (-plays * 0.4).exp(); // novelty
    f[39] = skips / (plays + skips + 1.5); // skip ratio
    f[40] = pop;

    // 41..46 active search/mix query context features
    f[41] = if ctx.query_active { 1.0 } else { 0.0 };
    f[42] = clean_finite(ctx.query_lexical_fit).clamp(0.0, 1.0);
    f[43] = clean_finite(ctx.query_era_match).clamp(-1.0, 1.0);
    f[44] = clean_finite(ctx.query_theme_match).clamp(0.0, 1.0);
    f[45] = (clean_finite(ctx.query_popularity_intent).clamp(0.0, 1.0) * pop).clamp(0.0, 1.0);

    // 46..48 interaction & bias
    f[46] = (a_aff * m_aff).clamp(-1.0, 1.0);
    f[47] = 1.0;

    f
}

/// Maps interaction events `(kind, value)` into `(target_probability, sample_weight)`.
pub fn event_target(kind: &str, value: f64) -> Option<(f64, f64)> {
    let safe_val = if value.is_finite() { value } else { 3.0 };
    match kind {
        "star" => Some((0.98, 2.5)),
        "playlist_add" => Some((0.93, 2.0)),
        "search_pick" => Some((0.91, 1.7)),
        "complete" => Some((0.90, 1.5)),
        "mix_play" => Some((0.86, 1.3)),
        "play" => Some((0.68, 0.7)),
        "skip" => Some((0.06, 1.8)),
        "unstar" => Some((0.12, 1.5)),
        "playlist_remove" => Some((0.20, 1.2)),
        "rate" => {
            let v = safe_val.clamp(1.0, 5.0);
            Some((((v - 1.0) / 4.0).clamp(0.0, 1.0), 2.2))
        }
        _ => None,
    }
}

/// Thread-safe in-memory cache of active user neural nets backed by SQLite KV persistence.
pub struct NeuralCache {
    nets: RwLock<HashMap<i64, UserNet>>,
}

impl Default for NeuralCache {
    fn default() -> Self {
        Self::new()
    }
}

impl NeuralCache {
    pub fn new() -> Self {
        Self {
            nets: RwLock::new(HashMap::new()),
        }
    }

    pub fn get_or_load(&self, store: &Store, user_id: i64) -> UserNet {
        {
            let guard = self.nets.read().unwrap_or_else(|e| e.into_inner());
            if let Some(net) = guard.get(&user_id) {
                return net.clone();
            }
        }
        let net = UserNet::load_or_new(store, user_id);
        self.nets
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(user_id, net.clone());
        net
    }

    pub fn update_and_save<F>(&self, store: &Store, user_id: i64, f: F) -> UserNet
    where
        F: FnOnce(&mut UserNet),
    {
        let mut net = self.get_or_load(store, user_id);
        f(&mut net);
        net.save(store);
        self.nets
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(user_id, net.clone());
        net
    }

    pub fn replace_and_save(&self, store: &Store, net: UserNet) -> UserNet {
        net.save(store);
        self.nets
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(net.user_id, net.clone());
        net
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_track(id: &str, artist: &str, genre: &str, moods: &str) -> Track {
        Track {
            id: id.into(),
            title: id.into(),
            artist: artist.into(),
            genre: genre.into(),
            moods: moods.into(),
            bitrate: 1411,
            format: "flac".into(),
            year: 2020,
            ..Default::default()
        }
    }

    #[test]
    fn online_training_separates_liked_and_skipped_tracks() {
        let mut net = UserNet::new(1);
        let liked = test_track("t1", "Daft Punk", "Electronic", "energy,party");
        let disliked = test_track("t2", "Slow Strings", "Classical", "melancholy");

        let x_like = extract_features_ctx(
            &liked,
            &FeatureContext {
                artist_affinity: 2.0,
                genre_affinity: 2.0,
                playlist_artist_affinity: 1.5,
                playlist_genre_affinity: 1.5,
                followed_artist: true,
                track_plays: 1,
                hour_override: Some(21),
                ..Default::default()
            },
        );
        let x_dislike = extract_features_ctx(
            &disliked,
            &FeatureContext {
                track_skips: 3,
                hour_override: Some(21),
                ..Default::default()
            },
        );

        for _ in 0..25 {
            net.train_step(&x_like, 0.95, 2.0);
            net.train_step(&x_dislike, 0.08, 1.8);
        }

        let p_like = net.predict(&x_like);
        let p_dislike = net.predict(&x_dislike);
        assert!(p_like > p_dislike + 0.25, "p_like={} p_dislike={}", p_like, p_dislike);
    }

    #[test]
    fn replay_buffer_prevents_catastrophic_forgetting() {
        let mut net = UserNet::new(7);
        let jazz = test_track("j1", "Miles", "Jazz", "chill,late_night");
        let edm = test_track("e1", "DJ Noise", "Electronic", "energy,party");

        let x_jazz = extract_features_ctx(
            &jazz,
            &FeatureContext {
                artist_affinity: 2.5,
                genre_affinity: 2.0,
                playlist_genre_affinity: 1.8,
                hour_override: Some(22),
                ..Default::default()
            },
        );
        let x_edm = extract_features_ctx(
            &edm,
            &FeatureContext {
                track_skips: 2,
                hour_override: Some(22),
                ..Default::default()
            },
        );

        for _ in 0..20 {
            net.train_step(&x_jazz, 0.96, 2.0);
        }
        // Burst of negative feedback on EDM tracks should not erase earlier Jazz preference.
        for _ in 0..12 {
            net.train_step(&x_edm, 0.06, 1.8);
        }

        let p_jazz = net.predict(&x_jazz);
        let p_edm = net.predict(&x_edm);
        assert!(p_jazz > 0.65 && p_jazz > p_edm + 0.25, "jazz={} edm={}", p_jazz, p_edm);
    }

    #[test]
    fn nan_or_inf_inputs_never_corrupt_weights() {
        let mut net = UserNet::new(9);
        let mut bad = [0.0; INPUT_DIM];
        bad[0] = f64::NAN;
        bad[3] = f64::INFINITY;
        net.train_step(&bad, f64::NAN, 1.0);
        assert!(net.weights_finite());

        let clean = [0.5; INPUT_DIM];
        let pred = net.predict(&clean);
        assert!(pred.is_finite() && (0.0..=1.0).contains(&pred));
    }

    #[test]
    fn playlist_events_map_to_strong_training_signals() {
        let (t_add, w_add) = event_target("playlist_add", 1.0).unwrap();
        let (t_rem, w_rem) = event_target("playlist_remove", 1.0).unwrap();
        let (t_pick, w_pick) = event_target("search_pick", 1.0).unwrap();
        let (t_mix, w_mix) = event_target("mix_play", 1.0).unwrap();
        assert!(t_add > 0.85 && w_add >= 1.5);
        assert!(t_rem < 0.3 && w_rem >= 1.0);
        assert!(t_pick > 0.85 && w_pick >= 1.5);
        assert!(t_mix > 0.8 && w_mix >= 1.0);
    }

    #[test]
    fn query_context_training_separates_on_era_and_search_picks() {
        let mut net = UserNet::new(11);
        let mut eighties_hit = test_track("e80", "a-ha", "Synthpop", "upbeat,party");
        eighties_hit.year = 1985;
        let mut modern_cover = test_track("m22", "Modern Remake", "Pop", "upbeat");
        modern_cover.year = 2022;

        let x_hit = extract_features_ctx(
            &eighties_hit,
            &FeatureContext {
                popularity_prior: 0.9,
                query_active: true,
                query_era_match: 1.0,
                query_theme_match: 0.8,
                query_popularity_intent: 1.0,
                hour_override: Some(20),
                ..Default::default()
            },
        );
        let x_miss = extract_features_ctx(
            &modern_cover,
            &FeatureContext {
                popularity_prior: 0.1,
                query_active: true,
                query_era_match: -1.0,
                query_theme_match: 0.1,
                query_popularity_intent: 1.0,
                track_skips: 2,
                hour_override: Some(20),
                ..Default::default()
            },
        );

        let (t_pick, w_pick) = event_target("search_pick", 1.0).unwrap();
        let (t_skip, w_skip) = event_target("skip", 1.0).unwrap();
        for _ in 0..16 {
            net.train_step(&x_hit, t_pick, w_pick);
            net.train_step(&x_miss, t_skip, w_skip);
        }

        assert!(net.predict(&x_hit) > net.predict(&x_miss) + 0.35);
    }

    #[test]
    fn incompatible_old_version_fails_compatibility_cleanly() {
        let mut old = UserNet::new(4);
        old.version = 2;
        assert!(!old.is_compatible());
    }
}
