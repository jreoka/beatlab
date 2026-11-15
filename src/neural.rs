use crate::models::{Mood, Track};
use crate::store::Store;
use chrono::{Local, Timelike};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const INPUT_DIM: usize = 32;
const H1_DIM: usize = 24;
const H2_DIM: usize = 12;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserNet {
    pub user_id: i64,
    pub updates: u64,
    pub avg_loss: f64,
    w1: Vec<f64>, // H1_DIM * INPUT_DIM
    b1: Vec<f64>, // H1_DIM
    w2: Vec<f64>, // H2_DIM * H1_DIM
    b2: Vec<f64>, // H2_DIM
    w3: Vec<f64>, // H2_DIM
    b3: f64,
}

impl UserNet {
    pub fn new(user_id: i64) -> Self {
        let mut rng = StdRng::seed_from_u64((user_id as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15).max(1));
        let s1 = (2.0 / INPUT_DIM as f64).sqrt();
        let s2 = (2.0 / H1_DIM as f64).sqrt();
        let s3 = (2.0 / H2_DIM as f64).sqrt();

        Self {
            user_id,
            updates: 0,
            avg_loss: 0.5,
            w1: (0..H1_DIM * INPUT_DIM)
                .map(|_| rng.gen_range(-s1..s1))
                .collect(),
            b1: vec![0.0; H1_DIM],
            w2: (0..H2_DIM * H1_DIM)
                .map(|_| rng.gen_range(-s2..s2))
                .collect(),
            b2: vec![0.0; H2_DIM],
            w3: (0..H2_DIM).map(|_| rng.gen_range(-s3..s3)).collect(),
            b3: 0.0,
        }
    }

    pub fn load_or_new(store: &Store, user_id: i64) -> Self {
        let key = format!("nn:user:{}", user_id);
        if let Some(raw) = store.kv_get(&key) {
            if let Ok(net) = serde_json::from_str::<UserNet>(&raw) {
                if net.w1.len() == H1_DIM * INPUT_DIM && net.w2.len() == H2_DIM * H1_DIM {
                    return net;
                }
            }
        }
        Self::new(user_id)
    }

    pub fn save(&self, store: &Store) {
        let key = format!("nn:user:{}", self.user_id);
        if let Ok(raw) = serde_json::to_string(self) {
            store.kv_set(&key, &raw);
        }
    }

    /// Forward pass returning predicted enjoyment probability in [0.0, 1.0].
    pub fn predict(&self, x: &[f64; INPUT_DIM]) -> f64 {
        let (_, _, out) = self.forward(x);
        out
    }

    fn forward(&self, x: &[f64; INPUT_DIM]) -> ([f64; H1_DIM], [f64; H2_DIM], f64) {
        let mut h1 = [0.0; H1_DIM];
        for i in 0..H1_DIM {
            let mut sum = self.b1[i];
            let row = i * INPUT_DIM;
            for j in 0..INPUT_DIM {
                sum += self.w1[row + j] * x[j];
            }
            h1[i] = leaky_relu(sum);
        }

        let mut h2 = [0.0; H2_DIM];
        for i in 0..H2_DIM {
            let mut sum = self.b2[i];
            let row = i * H1_DIM;
            for j in 0..H1_DIM {
                sum += self.w2[row + j] * h1[j];
            }
            h2[i] = leaky_relu(sum);
        }

        let mut z = self.b3;
        for i in 0..H2_DIM {
            z += self.w3[i] * h2[i];
        }
        (h1, h2, sigmoid(z))
    }

    /// Single online backpropagation step using Binary Cross-Entropy loss + L2 regularization.
    pub fn train_step(&mut self, x: &[f64; INPUT_DIM], target: f64, sample_weight: f64) {
        let target = target.clamp(0.01, 0.99);
        let (h1, h2, pred) = self.forward(x);

        let loss = -(target * pred.max(1e-7).ln() + (1.0 - target) * (1.0 - pred).max(1e-7).ln());
        self.avg_loss = if self.updates == 0 {
            loss
        } else {
            self.avg_loss * 0.95 + loss * 0.05
        };
        self.updates += 1;

        let lr = (0.045 / (1.0 + (self.updates as f64) * 0.0005)).max(0.008) * sample_weight;
        let l2 = 1e-4;

        // Output layer gradient (BCE + Sigmoid simplifies to pred - target)
        let dz3 = pred - target;

        let mut dh2 = [0.0; H2_DIM];
        for i in 0..H2_DIM {
            dh2[i] = dz3 * self.w3[i];
            self.w3[i] -= lr * (dz3 * h2[i] + l2 * self.w3[i]);
        }
        self.b3 -= lr * dz3;

        // Hidden layer 2 gradients
        let mut dz2 = [0.0; H2_DIM];
        for i in 0..H2_DIM {
            dz2[i] = dh2[i] * leaky_relu_grad(h2[i]);
        }
        let mut dh1 = [0.0; H1_DIM];
        for i in 0..H2_DIM {
            let row = i * H1_DIM;
            for j in 0..H1_DIM {
                dh1[j] += dz2[i] * self.w2[row + j];
                self.w2[row + j] -= lr * (dz2[i] * h1[j] + l2 * self.w2[row + j]);
            }
            self.b2[i] -= lr * dz2[i];
        }

        // Hidden layer 1 gradients
        for i in 0..H1_DIM {
            let dz1 = dh1[i] * leaky_relu_grad(h1[i]);
            let row = i * INPUT_DIM;
            for j in 0..INPUT_DIM {
                self.w1[row + j] -= lr * (dz1 * x[j] + l2 * self.w1[row + j]);
            }
            self.b1[i] -= lr * dz1;
        }
    }
}

/// Extracts a 32-dimensional normalized feature vector for `(track, user context)`.
pub fn extract_features(
    track: &Track,
    artist_affinity: f64,
    genre_affinity: f64,
    track_plays: i32,
    track_skips: i32,
) -> [f64; INPUT_DIM] {
    let mut x = [0.0; INPUT_DIM];

    // 0..10: Moods
    for m in track.mood_list() {
        let idx = match m {
            Mood::Chill => 0,
            Mood::Focus => 1,
            Mood::Energy => 2,
            Mood::LateNight => 3,
            Mood::Upbeat => 4,
            Mood::Melancholy => 5,
            Mood::Party => 6,
            Mood::Acoustic => 7,
            Mood::Groove => 8,
            Mood::RockIndie => 9,
        };
        x[idx] = 1.0;
    }

    // 10..18: Genre buckets
    let g = track.genre.to_lowercase();
    let g_idx = if g.contains("jazz") || g.contains("blues") {
        10
    } else if g.contains("rock") || g.contains("indie") || g.contains("metal") || g.contains("punk") {
        11
    } else if g.contains("electronic") || g.contains("ambient") || g.contains("house") || g.contains("techno") {
        12
    } else if g.contains("hip") || g.contains("rap") || g.contains("r&b") || g.contains("soul") || g.contains("funk") {
        13
    } else if g.contains("pop") || g.contains("latin") {
        14
    } else if g.contains("classical") || g.contains("soundtrack") {
        15
    } else if g.contains("folk") || g.contains("country") {
        16
    } else {
        17
    };
    x[g_idx] = 1.0;

    // 18..22: Audio Quality & Format
    x[18] = if track.is_lossless() { 1.0 } else { 0.0 };
    x[19] = (track.quality_score() as f64 / 1200.0).clamp(0.0, 1.0);
    x[20] = (track.duration as f64 / 600.0).clamp(0.0, 1.0);
    x[21] = if track.source == "local" { 1.0 } else { 0.0 };

    // 22..25: Era
    if track.year > 1900 && track.year < 1990 {
        x[22] = 1.0;
    } else if track.year >= 1990 && track.year < 2010 {
        x[23] = 1.0;
    } else {
        x[24] = 1.0;
    }

    // 25..27: Cyclical time-of-day (lets the net learn morning vs late-night preferences)
    let hour = Local::now().hour() as f64;
    let angle = 2.0 * std::f64::consts::PI * (hour / 24.0);
    x[25] = angle.sin();
    x[26] = angle.cos();

    // 27..32: User-Track & Affinity context
    x[27] = (artist_affinity / 5.0).tanh();
    x[28] = (genre_affinity / 5.0).tanh();
    x[29] = 1.0 / (1.0 + track_plays as f64); // novelty
    x[30] = (track_skips as f64) / ((track_plays + track_skips + 1) as f64); // skip ratio
    x[31] = 1.0; // bias-helper feature

    x
}

pub fn event_target(kind: &str, value: f64) -> Option<(f64, f64)> {
    match kind {
        "star" => Some((0.98, 1.4)),
        "complete" => Some((0.90, 1.1)),
        "play" => Some((0.65, 0.6)),
        "skip" => Some((0.05, 1.3)),
        "unstar" => Some((0.15, 1.0)),
        "rate" => Some((((value - 1.0) / 4.0).clamp(0.0, 1.0), 1.3)),
        _ => None,
    }
}

fn leaky_relu(v: f64) -> f64 {
    if v > 0.0 { v } else { 0.05 * v }
}

fn leaky_relu_grad(out: f64) -> f64 {
    if out > 0.0 { 1.0 } else { 0.05 }
}

fn sigmoid(z: f64) -> f64 {
    1.0 / (1.0 + (-z.clamp(-20.0, 20.0)).exp())
}

#[derive(Default)]
pub struct NeuralCache {
    nets: std::sync::RwLock<HashMap<i64, UserNet>>,
}

impl NeuralCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get_or_load(&self, store: &Store, user_id: i64) -> UserNet {
        {
            let guard = self.nets.read().unwrap();
            if let Some(n) = guard.get(&user_id) {
                return n.clone();
            }
        }
        let net = UserNet::load_or_new(store, user_id);
        self.nets.write().unwrap().insert(user_id, net.clone());
        net
    }

    pub fn update_and_save<F>(&self, store: &Store, user_id: i64, f: F) -> UserNet
    where
        F: FnOnce(&mut UserNet),
    {
        let mut net = self.get_or_load(store, user_id);
        f(&mut net);
        net.save(store);
        self.nets.write().unwrap().insert(user_id, net.clone());
        net
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neural_net_learns_user_preference_over_time() {
        let mut net = UserNet::new(1);
        let liked = Track {
            id: "local:jazz".into(),
            source: "local".into(),
            title: "Midnight Blue".into(),
            genre: "Jazz".into(),
            moods: "groove,late_night".into(),
            format: "flac".into(),
            bit_depth: 24,
            sample_rate: 96000,
            duration: 240,
            year: 1963,
            ..Default::default()
        };
        let disliked = Track {
            id: "youtube:metal".into(),
            source: "youtube".into(),
            title: "Loud Screaming".into(),
            genre: "Metal".into(),
            moods: "energy".into(),
            format: "opus".into(),
            bitrate: 128,
            duration: 180,
            year: 2020,
            ..Default::default()
        };

        let x_like = extract_features(&liked, 2.0, 2.0, 1, 0);
        let x_dislike = extract_features(&disliked, 0.0, 0.0, 0, 2);

        for _ in 0..40 {
            net.train_step(&x_like, 0.95, 1.2);
            net.train_step(&x_dislike, 0.05, 1.2);
        }

        let p_like = net.predict(&x_like);
        let p_dislike = net.predict(&x_dislike);
        assert!(p_like > 0.75, "expected high prob for liked track, got {}", p_like);
        assert!(p_dislike < 0.25, "expected low prob for disliked track, got {}", p_dislike);
        assert!(p_like > p_dislike);
    }
}
