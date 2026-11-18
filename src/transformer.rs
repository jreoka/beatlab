use crate::models::{primary_artist, slug, Mood, Track};
use crate::store::{NeuralTrainingCorpus, Store};
use chrono::Utc;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::sync::RwLock;
use tracing::{info, warn};

pub const D_MODEL: usize = 48;
pub const N_HEADS: usize = 4;
pub const D_HEAD: usize = D_MODEL / N_HEADS; // 12 (6 rotary pairs per head)
pub const D_FF: usize = 96;
pub const N_LAYERS: usize = 2;
pub const MAX_SEQ_LEN: usize = 32;
pub const N_ACTION_TYPES: usize = 8;

const TRACK_ID_OFFSET: usize = 0;
const N_TRACK_BUCKETS: usize = 2048;
const SONG_KEY_OFFSET: usize = TRACK_ID_OFFSET + N_TRACK_BUCKETS;
const N_SONG_KEY_BUCKETS: usize = 1024;
const ARTIST_OFFSET: usize = SONG_KEY_OFFSET + N_SONG_KEY_BUCKETS;
const N_ARTIST_BUCKETS: usize = 1024;
const TEXT_OFFSET: usize = ARTIST_OFFSET + N_ARTIST_BUCKETS;
const N_TEXT_BUCKETS: usize = 1024;
const GENRE_OFFSET: usize = TEXT_OFFSET + N_TEXT_BUCKETS;
const N_GENRE_BUCKETS: usize = 256;
const MOOD_OFFSET: usize = GENRE_OFFSET + N_GENRE_BUCKETS;
const N_MOOD_BUCKETS: usize = 16;
const ERA_OFFSET: usize = MOOD_OFFSET + N_MOOD_BUCKETS;
const N_ERA_BUCKETS: usize = 16;
pub const N_FEAT_ROWS: usize = ERA_OFFSET + N_ERA_BUCKETS; // 5408

const ROPE_BASE: f32 = 10000.0;
const XSA_EPS: f32 = 1e-6;
const RMS_EPS: f32 = 1e-5;
const CHECKPOINT_KV_KEY: &str = "transformer:checkpoint_v2";
/// v1 checkpoints were trained on raw play/complete event pairs (label leakage) and are discarded.
const LEGACY_CHECKPOINT_KV_KEY: &str = "transformer:checkpoint_v1";

pub const ACTION_STAR: usize = 0;
pub const ACTION_PLAYLIST: usize = 1;
pub const ACTION_COMPLETE: usize = 2;
pub const ACTION_PLAY: usize = 3;
pub const ACTION_SKIP: usize = 4;
pub const ACTION_DISLIKE: usize = 5;
pub const ACTION_SEED: usize = 6;
pub const ACTION_QUERY: usize = 7;

#[inline]
fn fnv1a_hash(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

#[inline]
fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e3779b97f4a7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
    x ^ (x >> 31)
}

#[inline]
fn deterministic_init(seed: u64, scale: f32) -> f32 {
    let u1 = ((splitmix64(seed) >> 11) as f64 + 0.5) / ((1u64 << 53) as f64);
    let u2 = ((splitmix64(seed ^ 0xa5a5a5a5a5a5a5a5) >> 11) as f64 + 0.5) / ((1u64 << 53) as f64);
    let z = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();
    (z as f32) * scale
}

#[inline]
fn gelu_forward(x: f32) -> f32 {
    let k = 0.7978845608_f32; // sqrt(2 / pi)
    let inner = k * (x + 0.044715 * x * x * x);
    0.5 * x * (1.0 + inner.tanh())
}

#[inline]
fn gelu_backward(x: f32) -> f32 {
    let k = 0.7978845608_f32;
    let x2 = x * x;
    let inner = k * (x + 0.044715 * x2 * x);
    let t = inner.tanh();
    let sech2 = 1.0 - t * t;
    let dinner = k * (1.0 + 3.0 * 0.044715 * x2);
    0.5 * (1.0 + t) + 0.5 * x * sech2 * dinner
}

/// Applies Rotary Position Embedding (RoPE) on a single head slice `[D_HEAD]`.
/// When `inverse == false`, applies `R_pos`; when `inverse == true`, applies `R_pos^T`.
#[inline]
fn rope_rotate_head(vec: &mut [f32], pos: usize, inverse: bool) {
    let pairs = D_HEAD / 2;
    for p in 0..pairs {
        let freq = ROPE_BASE.powf(-(2.0 * p as f32) / (D_HEAD as f32));
        let angle = (pos as f32) * freq;
        let (s_raw, c) = angle.sin_cos();
        let s = if inverse { -s_raw } else { s_raw };
        let i0 = 2 * p;
        let i1 = i0 + 1;
        let x0 = vec[i0];
        let x1 = vec[i1];
        vec[i0] = x0 * c - x1 * s;
        vec[i1] = x0 * s + x1 * c;
    }
}

/// Exclusive Self-Attention (XSA, arXiv:2603.09078):
/// removes the component of attention output `y` aligned with self-value `v`:
///   `z = y - ((y · v) / (||v||^2 + eps)) * v`
#[inline]
fn xsa_project_head(y: &[f32], v: &[f32], z_out: &mut [f32]) -> (f32, f32, f32) {
    let mut dot = 0.0f32;
    let mut v_norm2 = XSA_EPS;
    for d in 0..D_HEAD {
        dot += y[d] * v[d];
        v_norm2 += v[d] * v[d];
    }
    let lambda = dot / v_norm2;
    for d in 0..D_HEAD {
        z_out[d] = y[d] - lambda * v[d];
    }
    (dot, v_norm2, lambda)
}

#[inline]
fn xsa_project_head_backward(
    g_z: &[f32],
    y: &[f32],
    v: &[f32],
    v_norm2: f32,
    lambda: f32,
    g_y: &mut [f32],
    g_v_xsa: &mut [f32],
) {
    let mut gv = 0.0f32;
    for d in 0..D_HEAD {
        gv += g_z[d] * v[d];
    }
    let inv_n2 = 1.0 / v_norm2;
    let gv_inv = gv * inv_n2;
    for d in 0..D_HEAD {
        g_y[d] = g_z[d] - gv_inv * v[d];
        g_v_xsa[d] = -lambda * g_z[d] - gv_inv * y[d] + (2.0 * lambda * gv_inv) * v[d];
    }
}

#[inline]
fn rmsnorm_forward(x: &[f32], gamma: &[f32; D_MODEL], out: &mut [f32]) -> f32 {
    let mut ss = 0.0f32;
    for d in 0..D_MODEL {
        ss += x[d] * x[d];
    }
    let rms = (ss / (D_MODEL as f32) + RMS_EPS).sqrt();
    let inv = 1.0 / rms;
    for d in 0..D_MODEL {
        out[d] = (x[d] * inv) * gamma[d];
    }
    rms
}

#[inline]
fn rmsnorm_backward(
    g_out: &[f32],
    x: &[f32],
    rms: f32,
    gamma: &[f32; D_MODEL],
    g_x: &mut [f32],
    g_gamma: &mut [f32; D_MODEL],
) {
    let inv = 1.0 / rms;
    let mut dot_gu = 0.0f32;
    for d in 0..D_MODEL {
        let u = x[d] * inv;
        g_gamma[d] += g_out[d] * u;
        dot_gu += (g_out[d] * gamma[d]) * u;
    }
    let mean_gu = dot_gu / (D_MODEL as f32);
    for d in 0..D_MODEL {
        let u = x[d] * inv;
        g_x[d] += (g_out[d] * gamma[d] - u * mean_gu) * inv;
    }
}

#[inline]
fn l2_normalize(v: &[f32; D_MODEL]) -> ([f32; D_MODEL], f32) {
    let mut ss = 1e-12f32;
    for &x in v {
        ss += x * x;
    }
    let norm = ss.sqrt();
    let inv = 1.0 / norm;
    let mut out = [0.0f32; D_MODEL];
    for d in 0..D_MODEL {
        out[d] = v[d] * inv;
    }
    (out, norm)
}

#[inline]
fn l2_normalize_backward(g_unit: &[f32; D_MODEL], unit: &[f32; D_MODEL], norm: f32) -> [f32; D_MODEL] {
    let mut dot = 0.0f32;
    for d in 0..D_MODEL {
        dot += g_unit[d] * unit[d];
    }
    let inv = 1.0 / norm;
    let mut g_in = [0.0f32; D_MODEL];
    for d in 0..D_MODEL {
        g_in[d] = (g_unit[d] - unit[d] * dot) * inv;
    }
    g_in
}

/// Serializes `Vec<f32>` as base64 little-endian bytes: lossless and ~3x smaller than JSON numbers.
mod f32_b64 {
    use base64::Engine;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &[f32], s: S) -> Result<S::Ok, S::Error> {
        let mut bytes = Vec::with_capacity(v.len() * 4);
        for x in v {
            bytes.extend_from_slice(&x.to_le_bytes());
        }
        s.serialize_str(&base64::engine::general_purpose::STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<f32>, D::Error> {
        let raw = String::deserialize(d)?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(raw.as_bytes())
            .map_err(serde::de::Error::custom)?;
        if bytes.len() % 4 != 0 {
            return Err(serde::de::Error::custom("f32 payload length not a multiple of 4"));
        }
        Ok(bytes
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect())
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct AdamArray {
    #[serde(with = "f32_b64")]
    w: Vec<f32>,
    #[serde(with = "f32_b64")]
    m: Vec<f32>,
    #[serde(with = "f32_b64")]
    v: Vec<f32>,
}

impl AdamArray {
    fn shape_ok(&self, len: usize) -> bool {
        self.w.len() == len && self.m.len() == len && self.v.len() == len
    }
}

impl AdamArray {
    fn new_zeros(len: usize) -> Self {
        Self {
            w: vec![0.0; len],
            m: vec![0.0; len],
            v: vec![0.0; len],
        }
    }

    fn new_ones(len: usize) -> Self {
        Self {
            w: vec![1.0; len],
            m: vec![0.0; len],
            v: vec![0.0; len],
        }
    }

    fn new_init(len: usize, seed_base: u64, scale: f32) -> Self {
        let w = (0..len)
            .map(|i| deterministic_init(seed_base ^ (i as u64).wrapping_mul(0x9e3779b185ebca87), scale))
            .collect();
        Self {
            w,
            m: vec![0.0; len],
            v: vec![0.0; len],
        }
    }

    fn step_adamw(&mut self, grad: &[f32], lr: f32, wd: f32, step: u32) {
        let b1 = 0.9f32;
        let b2 = 0.999f32;
        let eps = 1e-8f32;
        let bc1 = 1.0 - b1.powi(step as i32);
        let bc2 = 1.0 - b2.powi(step as i32);
        for i in 0..self.w.len() {
            let g = grad[i];
            self.m[i] = b1 * self.m[i] + (1.0 - b1) * g;
            self.v[i] = b2 * self.v[i] + (1.0 - b2) * g * g;
            let m_hat = self.m[i] / bc1;
            let v_hat = self.v[i] / bc2;
            self.w[i] -= lr * (m_hat / (v_hat.sqrt() + eps) + wd * self.w[i]);
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct TransformerBlock {
    rms1_g: AdamArray,
    wq: AdamArray,
    wk: AdamArray,
    wv: AdamArray,
    wo: AdamArray,
    rms2_g: AdamArray,
    w1: AdamArray,
    b1: AdamArray,
    w_gate: AdamArray,
    b_gate: AdamArray,
    w2: AdamArray,
    b2: AdamArray,
}

impl TransformerBlock {
    fn new(layer_idx: usize) -> Self {
        let base = 0x42454154_0000_0000u64 ^ ((layer_idx as u64) << 20);
        let qkv_scale = (1.0 / (D_MODEL as f32)).sqrt() * 0.65;
        let out_scale = (1.0 / (D_MODEL as f32)).sqrt() * 0.45;
        let ffn_scale = (1.0 / (D_FF as f32)).sqrt() * 0.5;
        Self {
            rms1_g: AdamArray::new_ones(D_MODEL),
            wq: AdamArray::new_init(D_MODEL * D_MODEL, base ^ 1, qkv_scale),
            wk: AdamArray::new_init(D_MODEL * D_MODEL, base ^ 2, qkv_scale),
            wv: AdamArray::new_init(D_MODEL * D_MODEL, base ^ 3, qkv_scale),
            wo: AdamArray::new_init(D_MODEL * D_MODEL, base ^ 4, out_scale),
            rms2_g: AdamArray::new_ones(D_MODEL),
            w1: AdamArray::new_init(D_MODEL * D_FF, base ^ 5, qkv_scale),
            b1: AdamArray::new_zeros(D_FF),
            w_gate: AdamArray::new_init(D_MODEL * D_FF, base ^ 7, qkv_scale),
            b_gate: AdamArray::new_zeros(D_FF),
            w2: AdamArray::new_init(D_FF * D_MODEL, base ^ 6, ffn_scale),
            b2: AdamArray::new_zeros(D_MODEL),
        }
    }
}

struct BlockGrad {
    rms1_g: [f32; D_MODEL],
    wq: Vec<f32>,
    wk: Vec<f32>,
    wv: Vec<f32>,
    wo: Vec<f32>,
    rms2_g: [f32; D_MODEL],
    w1: Vec<f32>,
    b1: Vec<f32>,
    w_gate: Vec<f32>,
    b_gate: Vec<f32>,
    w2: Vec<f32>,
    b2: Vec<f32>,
}

impl BlockGrad {
    fn new() -> Self {
        Self {
            rms1_g: [0.0; D_MODEL],
            wq: vec![0.0; D_MODEL * D_MODEL],
            wk: vec![0.0; D_MODEL * D_MODEL],
            wv: vec![0.0; D_MODEL * D_MODEL],
            wo: vec![0.0; D_MODEL * D_MODEL],
            rms2_g: [0.0; D_MODEL],
            w1: vec![0.0; D_MODEL * D_FF],
            b1: vec![0.0; D_FF],
            w_gate: vec![0.0; D_MODEL * D_FF],
            b_gate: vec![0.0; D_FF],
            w2: vec![0.0; D_FF * D_MODEL],
            b2: vec![0.0; D_MODEL],
        }
    }

    fn sq_norm(&self) -> f32 {
        let mut s = 0.0f32;
        for &v in &self.rms1_g { s += v * v; }
        for &v in &self.wq { s += v * v; }
        for &v in &self.wk { s += v * v; }
        for &v in &self.wv { s += v * v; }
        for &v in &self.wo { s += v * v; }
        for &v in &self.rms2_g { s += v * v; }
        for &v in &self.w1 { s += v * v; }
        for &v in &self.b1 { s += v * v; }
        for &v in &self.w_gate { s += v * v; }
        for &v in &self.b_gate { s += v * v; }
        for &v in &self.w2 { s += v * v; }
        for &v in &self.b2 { s += v * v; }
        s
    }

    fn scale(&mut self, factor: f32) {
        for v in &mut self.rms1_g { *v *= factor; }
        for v in &mut self.wq { *v *= factor; }
        for v in &mut self.wk { *v *= factor; }
        for v in &mut self.wv { *v *= factor; }
        for v in &mut self.wo { *v *= factor; }
        for v in &mut self.rms2_g { *v *= factor; }
        for v in &mut self.w1 { *v *= factor; }
        for v in &mut self.b1 { *v *= factor; }
        for v in &mut self.w_gate { *v *= factor; }
        for v in &mut self.b_gate { *v *= factor; }
        for v in &mut self.w2 { *v *= factor; }
        for v in &mut self.b2 { *v *= factor; }
    }
}

struct BlockCache {
    x_in: Vec<f32>,    // [L * D_MODEL]
    rms1: Vec<f32>,    // [L]
    x_norm1: Vec<f32>, // [L * D_MODEL]
    q_rot: Vec<f32>,   // [L * D_MODEL]
    k_rot: Vec<f32>,   // [L * D_MODEL]
    v_raw: Vec<f32>,   // [L * D_MODEL] (unrotated V for XSA)
    v_rot: Vec<f32>,   // [L * D_MODEL]
    attn_probs: Vec<f32>, // [N_HEADS * L * L]
    y_unrot: Vec<f32>, // [L * D_MODEL] (after inverse RoPE R_i^T, before XSA)
    xsa_norm2: Vec<f32>, // [L * N_HEADS]
    xsa_lambda: Vec<f32>, // [L * N_HEADS]
    z_xsa: Vec<f32>,   // [L * D_MODEL] (XSA output before Wo)
    h1: Vec<f32>,      // [L * D_MODEL]
    rms2: Vec<f32>,    // [L]
    x_norm2: Vec<f32>, // [L * D_MODEL]
    ffn_val: Vec<f32>, // [L * D_FF] (x_norm2 * W1 + b1)
    ffn_gate_pre: Vec<f32>, // [L * D_FF] (x_norm2 * W_gate + b_gate)
    ffn_gate_act: Vec<f32>, // [L * D_FF] (GELU(ffn_gate_pre))
    ffn_hidden: Vec<f32>,   // [L * D_FF] (ffn_gate_act * ffn_val)
}

fn matmul_vec_add(inp: &[f32], w: &[f32], in_dim: usize, out_dim: usize, out: &mut [f32]) {
    for j in 0..out_dim {
        out[j] = 0.0;
    }
    for i in 0..in_dim {
        let a = inp[i];
        let row = &w[i * out_dim..(i + 1) * out_dim];
        for j in 0..out_dim {
            out[j] += a * row[j];
        }
    }
}

fn matmul_backward(
    g_out: &[f32],
    inp: &[f32],
    w: &[f32],
    in_dim: usize,
    out_dim: usize,
    g_inp: &mut [f32],
    g_w: &mut [f32],
) {
    for i in 0..in_dim {
        let a = inp[i];
        let w_row = &w[i * out_dim..(i + 1) * out_dim];
        let gw_row = &mut g_w[i * out_dim..(i + 1) * out_dim];
        let mut gi = 0.0f32;
        for j in 0..out_dim {
            let go = g_out[j];
            gi += go * w_row[j];
            gw_row[j] += a * go;
        }
        g_inp[i] += gi;
    }
}

impl TransformerBlock {
    fn forward(&self, x: &[f32], seq_len: usize) -> (Vec<f32>, BlockCache) {
        let mut rms1 = vec![0.0f32; seq_len];
        let mut x_norm1 = vec![0.0f32; seq_len * D_MODEL];
        let mut q_rot = vec![0.0f32; seq_len * D_MODEL];
        let mut k_rot = vec![0.0f32; seq_len * D_MODEL];
        let mut v_raw = vec![0.0f32; seq_len * D_MODEL];
        let mut v_rot = vec![0.0f32; seq_len * D_MODEL];

        let rms1_g: [f32; D_MODEL] = self.rms1_g.w[..D_MODEL].try_into().unwrap();
        let rms2_g: [f32; D_MODEL] = self.rms2_g.w[..D_MODEL].try_into().unwrap();

        for t in 0..seq_len {
            let x_t = &x[t * D_MODEL..(t + 1) * D_MODEL];
            let xn_t = &mut x_norm1[t * D_MODEL..(t + 1) * D_MODEL];
            rms1[t] = rmsnorm_forward(x_t, &rms1_g, xn_t);

            let q_t = &mut q_rot[t * D_MODEL..(t + 1) * D_MODEL];
            let k_t = &mut k_rot[t * D_MODEL..(t + 1) * D_MODEL];
            let v_t = &mut v_raw[t * D_MODEL..(t + 1) * D_MODEL];
            matmul_vec_add(xn_t, &self.wq.w, D_MODEL, D_MODEL, q_t);
            matmul_vec_add(xn_t, &self.wk.w, D_MODEL, D_MODEL, k_t);
            matmul_vec_add(xn_t, &self.wv.w, D_MODEL, D_MODEL, v_t);

            v_rot[t * D_MODEL..(t + 1) * D_MODEL].copy_from_slice(v_t);
            let vr_t = &mut v_rot[t * D_MODEL..(t + 1) * D_MODEL];

            // RoPE-QKVO: apply RoPE forward rotation R_t to Q, K, and V per head.
            for h in 0..N_HEADS {
                let hs = h * D_HEAD..(h + 1) * D_HEAD;
                rope_rotate_head(&mut q_t[hs.clone()], t, false);
                rope_rotate_head(&mut k_t[hs.clone()], t, false);
                rope_rotate_head(&mut vr_t[hs], t, false);
            }
        }

        let mut attn_probs = vec![0.0f32; N_HEADS * seq_len * seq_len];
        let mut y_unrot = vec![0.0f32; seq_len * D_MODEL];
        let mut xsa_norm2 = vec![0.0f32; seq_len * N_HEADS];
        let mut xsa_lambda = vec![0.0f32; seq_len * N_HEADS];
        let mut z_xsa = vec![0.0f32; seq_len * D_MODEL];

        let attn_scale = 1.0 / (D_HEAD as f32).sqrt();
        let mut logits = vec![0.0f32; seq_len];
        let mut y_rot_head = [0.0f32; D_HEAD];

        for h in 0..N_HEADS {
            let h_off = h * D_HEAD;
            for i in 0..seq_len {
                let qi = &q_rot[i * D_MODEL + h_off..i * D_MODEL + h_off + D_HEAD];
                let mut max_l = f32::NEG_INFINITY;
                for j in 0..seq_len {
                    let kj = &k_rot[j * D_MODEL + h_off..j * D_MODEL + h_off + D_HEAD];
                    let mut dot = 0.0f32;
                    for d in 0..D_HEAD {
                        dot += qi[d] * kj[d];
                    }
                    let l = dot * attn_scale;
                    logits[j] = l;
                    if l > max_l {
                        max_l = l;
                    }
                }
                let mut sum_e = 0.0f32;
                let p_base = (h * seq_len + i) * seq_len;
                for j in 0..seq_len {
                    let e = (logits[j] - max_l).exp();
                    attn_probs[p_base + j] = e;
                    sum_e += e;
                }
                let inv_e = 1.0 / sum_e.max(1e-12);
                y_rot_head.fill(0.0);
                for j in 0..seq_len {
                    let p = attn_probs[p_base + j] * inv_e;
                    attn_probs[p_base + j] = p;
                    let vj = &v_rot[j * D_MODEL + h_off..j * D_MODEL + h_off + D_HEAD];
                    for d in 0..D_HEAD {
                        y_rot_head[d] += p * vj[d];
                    }
                }

                // RoPE-QKVO: inverse rotation R_i^T on head output before O projection.
                rope_rotate_head(&mut y_rot_head, i, true);
                let y_dst = &mut y_unrot[i * D_MODEL + h_off..i * D_MODEL + h_off + D_HEAD];
                y_dst.copy_from_slice(&y_rot_head);

                // Exclusive Self-Attention (XSA): remove self-value projection.
                let v_self = &v_raw[i * D_MODEL + h_off..i * D_MODEL + h_off + D_HEAD];
                let z_dst = &mut z_xsa[i * D_MODEL + h_off..i * D_MODEL + h_off + D_HEAD];
                let (_, n2, lam) = xsa_project_head(y_dst, v_self, z_dst);
                xsa_norm2[i * N_HEADS + h] = n2;
                xsa_lambda[i * N_HEADS + h] = lam;
            }
        }

        let mut h1 = vec![0.0f32; seq_len * D_MODEL];
        let mut rms2 = vec![0.0f32; seq_len];
        let mut x_norm2 = vec![0.0f32; seq_len * D_MODEL];
        let mut ffn_val = vec![0.0f32; seq_len * D_FF];
        let mut ffn_gate_pre = vec![0.0f32; seq_len * D_FF];
        let mut ffn_gate_act = vec![0.0f32; seq_len * D_FF];
        let mut ffn_hidden = vec![0.0f32; seq_len * D_FF];
        let mut h2 = vec![0.0f32; seq_len * D_MODEL];
        let mut tmp_proj = [0.0f32; D_MODEL];

        for t in 0..seq_len {
            let z_t = &z_xsa[t * D_MODEL..(t + 1) * D_MODEL];
            matmul_vec_add(z_t, &self.wo.w, D_MODEL, D_MODEL, &mut tmp_proj);
            let h1_t = &mut h1[t * D_MODEL..(t + 1) * D_MODEL];
            let x_t = &x[t * D_MODEL..(t + 1) * D_MODEL];
            for d in 0..D_MODEL {
                h1_t[d] = x_t[d] + tmp_proj[d];
            }

            let xn2_t = &mut x_norm2[t * D_MODEL..(t + 1) * D_MODEL];
            rms2[t] = rmsnorm_forward(h1_t, &rms2_g, xn2_t);

            // GEGLU FFN: (GELU(x * W_gate + b_gate) * (x * W1 + b1)) * W2 + b2
            let val_t = &mut ffn_val[t * D_FF..(t + 1) * D_FF];
            let gpre_t = &mut ffn_gate_pre[t * D_FF..(t + 1) * D_FF];
            let gact_t = &mut ffn_gate_act[t * D_FF..(t + 1) * D_FF];
            let hid_t = &mut ffn_hidden[t * D_FF..(t + 1) * D_FF];
            matmul_vec_add(xn2_t, &self.w1.w, D_MODEL, D_FF, val_t);
            matmul_vec_add(xn2_t, &self.w_gate.w, D_MODEL, D_FF, gpre_t);
            for j in 0..D_FF {
                val_t[j] += self.b1.w[j];
                gpre_t[j] += self.b_gate.w[j];
                gact_t[j] = gelu_forward(gpre_t[j]);
                hid_t[j] = gact_t[j] * val_t[j];
            }

            matmul_vec_add(hid_t, &self.w2.w, D_FF, D_MODEL, &mut tmp_proj);
            let h2_t = &mut h2[t * D_MODEL..(t + 1) * D_MODEL];
            for d in 0..D_MODEL {
                h2_t[d] = h1_t[d] + tmp_proj[d] + self.b2.w[d];
            }
        }

        let cache = BlockCache {
            x_in: x.to_vec(),
            rms1,
            x_norm1,
            q_rot,
            k_rot,
            v_raw,
            v_rot,
            attn_probs,
            y_unrot,
            xsa_norm2,
            xsa_lambda,
            z_xsa,
            h1,
            rms2,
            x_norm2,
            ffn_val,
            ffn_gate_pre,
            ffn_gate_act,
            ffn_hidden,
        };
        (h2, cache)
    }

    fn backward(&self, g_out: &[f32], cache: &BlockCache, seq_len: usize, grad: &mut BlockGrad) -> Vec<f32> {
        let rms1_g: [f32; D_MODEL] = self.rms1_g.w[..D_MODEL].try_into().unwrap();
        let rms2_g: [f32; D_MODEL] = self.rms2_g.w[..D_MODEL].try_into().unwrap();

        let mut g_h1 = g_out.to_vec();
        let mut g_z_xsa = vec![0.0f32; seq_len * D_MODEL];

        let mut g_hid = [0.0f32; D_FF];
        let mut g_val = [0.0f32; D_FF];
        let mut g_gate_pre = [0.0f32; D_FF];
        let mut g_xn2 = [0.0f32; D_MODEL];

        for t in 0..seq_len {
            let go_t = &g_out[t * D_MODEL..(t + 1) * D_MODEL];
            for d in 0..D_MODEL {
                grad.b2[d] += go_t[d];
            }
            let hid_t = &cache.ffn_hidden[t * D_FF..(t + 1) * D_FF];
            g_hid.fill(0.0);
            matmul_backward(go_t, hid_t, &self.w2.w, D_FF, D_MODEL, &mut g_hid, &mut grad.w2);

            let val_t = &cache.ffn_val[t * D_FF..(t + 1) * D_FF];
            let gpre_t = &cache.ffn_gate_pre[t * D_FF..(t + 1) * D_FF];
            let gact_t = &cache.ffn_gate_act[t * D_FF..(t + 1) * D_FF];
            for j in 0..D_FF {
                let gh = g_hid[j];
                let gv = gh * gact_t[j];
                let gg = gh * val_t[j] * gelu_backward(gpre_t[j]);
                g_val[j] = gv;
                g_gate_pre[j] = gg;
                grad.b1[j] += gv;
                grad.b_gate[j] += gg;
            }

            let xn2_t = &cache.x_norm2[t * D_MODEL..(t + 1) * D_MODEL];
            g_xn2.fill(0.0);
            matmul_backward(&g_val, xn2_t, &self.w1.w, D_MODEL, D_FF, &mut g_xn2, &mut grad.w1);
            matmul_backward(
                &g_gate_pre,
                xn2_t,
                &self.w_gate.w,
                D_MODEL,
                D_FF,
                &mut g_xn2,
                &mut grad.w_gate,
            );

            let h1_t = &cache.h1[t * D_MODEL..(t + 1) * D_MODEL];
            let gh1_t = &mut g_h1[t * D_MODEL..(t + 1) * D_MODEL];
            rmsnorm_backward(&g_xn2, h1_t, cache.rms2[t], &rms2_g, gh1_t, &mut grad.rms2_g);

            let z_t = &cache.z_xsa[t * D_MODEL..(t + 1) * D_MODEL];
            let gz_t = &mut g_z_xsa[t * D_MODEL..(t + 1) * D_MODEL];
            matmul_backward(gh1_t, z_t, &self.wo.w, D_MODEL, D_MODEL, gz_t, &mut grad.wo);
        }

        let mut g_x = g_h1; // residual from h1 = x + wo(z_xsa)
        let mut g_q_rot = vec![0.0f32; seq_len * D_MODEL];
        let mut g_k_rot = vec![0.0f32; seq_len * D_MODEL];
        let mut g_v_rot = vec![0.0f32; seq_len * D_MODEL];
        let mut g_v_xsa_total = vec![0.0f32; seq_len * D_MODEL];

        let attn_scale = 1.0 / (D_HEAD as f32).sqrt();
        let mut g_y_unrot = [0.0f32; D_HEAD];
        let mut g_v_xsa_head = [0.0f32; D_HEAD];
        let mut g_probs = vec![0.0f32; seq_len];
        let mut g_logits = vec![0.0f32; seq_len];

        for h in 0..N_HEADS {
            let h_off = h * D_HEAD;
            for i in 0..seq_len {
                let gz_head = &g_z_xsa[i * D_MODEL + h_off..i * D_MODEL + h_off + D_HEAD];
                let y_head = &cache.y_unrot[i * D_MODEL + h_off..i * D_MODEL + h_off + D_HEAD];
                let v_self = &cache.v_raw[i * D_MODEL + h_off..i * D_MODEL + h_off + D_HEAD];

                xsa_project_head_backward(
                    gz_head,
                    y_head,
                    v_self,
                    cache.xsa_norm2[i * N_HEADS + h],
                    cache.xsa_lambda[i * N_HEADS + h],
                    &mut g_y_unrot,
                    &mut g_v_xsa_head,
                );
                let gv_xsa_dst = &mut g_v_xsa_total[i * D_MODEL + h_off..i * D_MODEL + h_off + D_HEAD];
                for d in 0..D_HEAD {
                    gv_xsa_dst[d] += g_v_xsa_head[d];
                }

                // Backward of inverse rotation R_i^T is forward rotation R_i
                rope_rotate_head(&mut g_y_unrot, i, false);

                let p_base = (h * seq_len + i) * seq_len;
                let mut dot_gp = 0.0f32;
                for j in 0..seq_len {
                    let p = cache.attn_probs[p_base + j];
                    let vj = &cache.v_rot[j * D_MODEL + h_off..j * D_MODEL + h_off + D_HEAD];
                    let gvj = &mut g_v_rot[j * D_MODEL + h_off..j * D_MODEL + h_off + D_HEAD];
                    let mut gp = 0.0f32;
                    for d in 0..D_HEAD {
                        gp += g_y_unrot[d] * vj[d];
                        gvj[d] += p * g_y_unrot[d];
                    }
                    g_probs[j] = gp;
                    dot_gp += p * gp;
                }

                let qi = &cache.q_rot[i * D_MODEL + h_off..i * D_MODEL + h_off + D_HEAD];
                let gqi = &mut g_q_rot[i * D_MODEL + h_off..i * D_MODEL + h_off + D_HEAD];
                for j in 0..seq_len {
                    let p = cache.attn_probs[p_base + j];
                    g_logits[j] = p * (g_probs[j] - dot_gp) * attn_scale;
                }
                for j in 0..seq_len {
                    let gl = g_logits[j];
                    let kj = &cache.k_rot[j * D_MODEL + h_off..j * D_MODEL + h_off + D_HEAD];
                    let gkj = &mut g_k_rot[j * D_MODEL + h_off..j * D_MODEL + h_off + D_HEAD];
                    for d in 0..D_HEAD {
                        gqi[d] += gl * kj[d];
                        gkj[d] += gl * qi[d];
                    }
                }
            }
        }

        let mut g_xn1 = [0.0f32; D_MODEL];
        for t in 0..seq_len {
            let gq_t = &mut g_q_rot[t * D_MODEL..(t + 1) * D_MODEL];
            let gk_t = &mut g_k_rot[t * D_MODEL..(t + 1) * D_MODEL];
            let gv_t = &mut g_v_rot[t * D_MODEL..(t + 1) * D_MODEL];
            let gv_xsa = &g_v_xsa_total[t * D_MODEL..(t + 1) * D_MODEL];

            for h in 0..N_HEADS {
                let hs = h * D_HEAD..(h + 1) * D_HEAD;
                rope_rotate_head(&mut gq_t[hs.clone()], t, true);
                rope_rotate_head(&mut gk_t[hs.clone()], t, true);
                rope_rotate_head(&mut gv_t[hs], t, true);
            }
            for d in 0..D_MODEL {
                gv_t[d] += gv_xsa[d];
            }

            let xn1_t = &cache.x_norm1[t * D_MODEL..(t + 1) * D_MODEL];
            g_xn1.fill(0.0);
            matmul_backward(gq_t, xn1_t, &self.wq.w, D_MODEL, D_MODEL, &mut g_xn1, &mut grad.wq);
            matmul_backward(gk_t, xn1_t, &self.wk.w, D_MODEL, D_MODEL, &mut g_xn1, &mut grad.wk);
            matmul_backward(gv_t, xn1_t, &self.wv.w, D_MODEL, D_MODEL, &mut g_xn1, &mut grad.wv);

            let x_t = &cache.x_in[t * D_MODEL..(t + 1) * D_MODEL];
            let gx_t = &mut g_x[t * D_MODEL..(t + 1) * D_MODEL];
            rmsnorm_backward(&g_xn1, x_t, cache.rms1[t], &rms1_g, gx_t, &mut grad.rms1_g);
        }

        g_x
    }

    fn apply_adamw(&mut self, grad: &BlockGrad, lr: f32, wd: f32, step: u32) {
        self.rms1_g.step_adamw(&grad.rms1_g, lr, 0.0, step);
        self.wq.step_adamw(&grad.wq, lr, wd, step);
        self.wk.step_adamw(&grad.wk, lr, wd, step);
        self.wv.step_adamw(&grad.wv, lr, wd, step);
        self.wo.step_adamw(&grad.wo, lr, wd, step);
        self.rms2_g.step_adamw(&grad.rms2_g, lr, 0.0, step);
        self.w1.step_adamw(&grad.w1, lr, wd, step);
        self.b1.step_adamw(&grad.b1, lr, 0.0, step);
        self.w_gate.step_adamw(&grad.w_gate, lr, wd, step);
        self.b_gate.step_adamw(&grad.b_gate, lr, 0.0, step);
        self.w2.step_adamw(&grad.w2, lr, wd, step);
        self.b2.step_adamw(&grad.b2, lr, 0.0, step);
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct SparseAdamRow {
    #[serde(with = "f32_b64")]
    w: Vec<f32>,
    #[serde(with = "f32_b64")]
    m: Vec<f32>,
    #[serde(with = "f32_b64")]
    v: Vec<f32>,
    step: u32,
}

impl SparseAdamRow {
    fn from_seed(seed: u64, scale: f32) -> Self {
        let mut w = vec![0.0f32; D_MODEL];
        for d in 0..D_MODEL {
            w[d] = deterministic_init(seed ^ (d as u64).wrapping_mul(0x517cc1b727220a95), scale);
        }
        Self {
            w,
            m: vec![0.0; D_MODEL],
            v: vec![0.0; D_MODEL],
            step: 0,
        }
    }

    fn step_adamw(&mut self, grad: &[f32; D_MODEL], lr: f32, wd: f32) {
        self.step = self.step.saturating_add(1);
        let b1 = 0.9f32;
        let b2 = 0.999f32;
        let eps = 1e-8f32;
        let bc1 = 1.0 - b1.powi(self.step as i32);
        let bc2 = 1.0 - b2.powi(self.step as i32);
        for d in 0..D_MODEL {
            let g = grad[d];
            self.m[d] = b1 * self.m[d] + (1.0 - b1) * g;
            self.v[d] = b2 * self.v[d] + (1.0 - b2) * g * g;
            let m_hat = self.m[d] / bc1;
            let v_hat = self.v[d] / bc2;
            self.w[d] -= lr * (m_hat / (v_hat.sqrt() + eps) + wd * self.w[d]);
        }
    }
}

/// One context token: sparse feature rows, optional continuous scalars, and the action type.
type SeqItem = (Vec<(usize, f32)>, Option<[f32; 4]>, usize);

/// Minimum number of held-out listens before the evaluation is trusted to gate the blend weight.
const MIN_EVAL_ITEMS: usize = 6;
/// Sampled negatives per training example (in-batch negatives are added on top).
const N_SAMPLED_NEGATIVES: usize = 8;
/// Random catalog distractors each held-out target is ranked against.
const EVAL_POOL_NEGATIVES: usize = 99;
/// Below this many history tokens, the user's stars are prepended as a taste prefix.
const PROFILE_SHORT_HISTORY: usize = 12;
const STAR_PREFIX_MAX: usize = 8;
/// History length used when a seed/query context follows the history (mixes, radio, playlists).
const CONTEXT_HISTORY_CAP: usize = 16;
const MAX_SEED_TOKENS: usize = 12;
const CORPUS_EVENTS_PER_USER: usize = 300;
const CORPUS_CATALOG_LIMIT: usize = 3600;
/// Events on the same track this close together belong to one listen (play → complete/skip).
const LISTEN_MERGE_WINDOW_SECS: i64 = 3 * 3600;

/// Held-out ranking quality of the model versus two simple baselines.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct EvalReport {
    /// Number of held-out listens evaluated.
    pub n: usize,
    pub model_mrr: f32,
    pub model_recall10: f32,
    /// Baseline: rank by global popularity (Deezer rank / quality).
    pub popularity_mrr: f32,
    /// Baseline: rank by artist/genre overlap with the user's context (content heuristic).
    pub content_mrr: f32,
    pub content_recall10: f32,
    /// `model_mrr / max(popularity_mrr, content_mrr)`.
    pub lift: f32,
    /// Smoothed 0.1..1.0 trust score derived from `lift`; scales the blend weight.
    pub quality: f32,
    pub evaluated_at: i64,
}

impl EvalReport {
    fn trusted(&self) -> bool {
        self.n >= MIN_EVAL_ITEMS
    }

    fn compute_lift(&mut self) {
        let base = self.popularity_mrr.max(self.content_mrr).max(1e-6);
        self.lift = self.model_mrr / base;
    }

    /// Folds this run's lift into the smoothed quality score carried over from `prev`.
    fn with_quality_from(mut self, prev: &EvalReport) -> Self {
        if self.trusted() {
            let q = ((self.lift - 0.85) / 0.65).clamp(0.1, 1.0);
            self.quality = if prev.trusted() { 0.5 * prev.quality + 0.5 * q } else { q };
        } else {
            self.quality = prev.quality;
        }
        self
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct TransformerNet {
    #[serde(with = "f32_b64")]
    feature_weights: Vec<f32>,
    #[serde(with = "f32_b64")]
    feature_m: Vec<f32>,
    #[serde(with = "f32_b64")]
    feature_v: Vec<f32>,
    feature_steps: Vec<u32>,
    user_embs: HashMap<i64, SparseAdamRow>,
    action_emb: AdamArray,
    scalar_proj: AdamArray,
    blocks: Vec<TransformerBlock>,
    final_rms_g: AdamArray,
    ctx_proj_w: AdamArray,
    track_proj_w: AdamArray,
    logit_scale: f32,
    logit_scale_m: f32,
    logit_scale_v: f32,
    pub step_count: u64,
    pub avg_loss: f32,
    pub last_trained_at: i64,
    #[serde(default)]
    pub eval: EvalReport,
    #[serde(default)]
    data_fingerprint: String,
    #[serde(default)]
    steps_on_fingerprint: u64,
    #[serde(default)]
    step_budget: u64,
    #[serde(default)]
    rejected_runs: u64,
}

impl TransformerNet {
    pub fn new() -> Self {
        let emb_scale = 0.10f32;
        let mut feature_weights = vec![0.0f32; N_FEAT_ROWS * D_MODEL];
        for r in 0..N_FEAT_ROWS {
            for d in 0..D_MODEL {
                let seed = 0x7472616e73_u64
                    ^ ((r as u64).wrapping_mul(0x9e3779b97f4a7c15))
                    ^ ((d as u64).wrapping_mul(0xbf58476d1ce4e5b9));
                feature_weights[r * D_MODEL + d] = deterministic_init(seed, emb_scale);
            }
        }
        let proj_scale = (1.0 / (D_MODEL as f32)).sqrt() * 0.6;
        let mut blocks = Vec::with_capacity(N_LAYERS);
        for i in 0..N_LAYERS {
            blocks.push(TransformerBlock::new(i));
        }
        Self {
            feature_weights,
            feature_m: vec![0.0; N_FEAT_ROWS * D_MODEL],
            feature_v: vec![0.0; N_FEAT_ROWS * D_MODEL],
            feature_steps: vec![0; N_FEAT_ROWS],
            user_embs: HashMap::new(),
            action_emb: AdamArray::new_init(N_ACTION_TYPES * D_MODEL, 0x1111_2222, emb_scale),
            scalar_proj: AdamArray::new_init(4 * D_MODEL, 0x3333_4444, 0.08),
            blocks,
            final_rms_g: AdamArray::new_ones(D_MODEL),
            ctx_proj_w: AdamArray::new_init(D_MODEL * D_MODEL, 0x5555_6666, proj_scale),
            track_proj_w: AdamArray::new_init(D_MODEL * D_MODEL, 0x7777_8888, proj_scale),
            logit_scale: 8.0,
            logit_scale_m: 0.0,
            logit_scale_v: 0.0,
            step_count: 0,
            avg_loss: 0.0,
            last_trained_at: 0,
            eval: EvalReport::default(),
            data_fingerprint: String::new(),
            steps_on_fingerprint: 0,
            step_budget: 0,
            rejected_runs: 0,
        }
    }

    fn shapes_ok(&self) -> bool {
        let dd = D_MODEL * D_MODEL;
        self.feature_weights.len() == N_FEAT_ROWS * D_MODEL
            && self.feature_m.len() == N_FEAT_ROWS * D_MODEL
            && self.feature_v.len() == N_FEAT_ROWS * D_MODEL
            && self.feature_steps.len() == N_FEAT_ROWS
            && self.action_emb.shape_ok(N_ACTION_TYPES * D_MODEL)
            && self.scalar_proj.shape_ok(4 * D_MODEL)
            && self.final_rms_g.shape_ok(D_MODEL)
            && self.ctx_proj_w.shape_ok(dd)
            && self.track_proj_w.shape_ok(dd)
            && self.user_embs.values().all(|r| r.w.len() == D_MODEL && r.m.len() == D_MODEL && r.v.len() == D_MODEL)
            && self.blocks.len() == N_LAYERS
            && self.blocks.iter().all(|b| {
                b.rms1_g.shape_ok(D_MODEL)
                    && b.wq.shape_ok(dd)
                    && b.wk.shape_ok(dd)
                    && b.wv.shape_ok(dd)
                    && b.wo.shape_ok(dd)
                    && b.rms2_g.shape_ok(D_MODEL)
                    && b.w1.shape_ok(D_MODEL * D_FF)
                    && b.b1.shape_ok(D_FF)
                    && b.w_gate.shape_ok(D_MODEL * D_FF)
                    && b.b_gate.shape_ok(D_FF)
                    && b.w2.shape_ok(D_FF * D_MODEL)
                    && b.b2.shape_ok(D_MODEL)
            })
    }

    fn user_row_vec(&self, user_id: i64) -> [f32; D_MODEL] {
        if let Some(row) = self.user_embs.get(&user_id) {
            if row.w.len() == D_MODEL {
                return row.w[..D_MODEL].try_into().unwrap();
            }
        }
        let seed = 0x55534552_00000000u64 ^ (user_id as u64).wrapping_mul(0x9e3779b97f4a7c15);
        let mut w = [0.0f32; D_MODEL];
        for d in 0..D_MODEL {
            w[d] = deterministic_init(seed ^ (d as u64).wrapping_mul(0x517cc1b727220a95), 0.15);
        }
        w
    }

    fn get_or_insert_user_row(&mut self, user_id: i64) -> &mut SparseAdamRow {
        self.user_embs.entry(user_id).or_insert_with(|| {
            let seed = 0x55534552_00000000u64 ^ (user_id as u64).wrapping_mul(0x9e3779b97f4a7c15);
            SparseAdamRow::from_seed(seed, 0.15)
        })
    }

    /// Encodes any track into sparse shared feature indices + normalized continuous scalars.
    pub fn extract_track_features(t: &Track) -> (Vec<(usize, f32)>, [f32; 4]) {
        let mut feats = Vec::with_capacity(20);

        let id_b = (fnv1a_hash(t.id.as_bytes()) as usize) % N_TRACK_BUCKETS;
        feats.push((TRACK_ID_OFFSET + id_b, 0.55));

        let sk = t.song_key();
        let sk_b = (fnv1a_hash(sk.as_bytes()) as usize) % N_SONG_KEY_BUCKETS;
        feats.push((SONG_KEY_OFFSET + sk_b, 0.60));

        let pa = slug(&primary_artist(&t.artist));
        if !pa.is_empty() && pa != "unknown-artist" {
            let a_b = (fnv1a_hash(pa.as_bytes()) as usize) % N_ARTIST_BUCKETS;
            feats.push((ARTIST_OFFSET + a_b, 1.25));
        }
        // Also hash secondary/featured artists so collaborations bridge artist graphs.
        let artist_low = t.artist.to_lowercase();
        let mut sec_artists = 0usize;
        for part in artist_low
            .replace(" feat. ", "|")
            .replace(" feat ", "|")
            .replace(" ft. ", "|")
            .replace(" ft ", "|")
            .replace(" with ", "|")
            .replace(" & ", "|")
            .replace(" x ", "|")
            .split([',', '|', ';'])
        {
            let ps = slug(part.trim());
            if !ps.is_empty() && ps != pa && ps != "unknown-artist" {
                let a_b = (fnv1a_hash(ps.as_bytes()) as usize) % N_ARTIST_BUCKETS;
                feats.push((ARTIST_OFFSET + a_b, 0.70));
                sec_artists += 1;
                if sec_artists >= 3 {
                    break;
                }
            }
        }

        let mut genre_count = 0usize;
        let mut genre_sub_seen: HashSet<String> = HashSet::new();
        for g in t.genre.split([',', ';', '/']) {
            let gl = g.trim().to_lowercase();
            if gl.len() >= 2 {
                let g_b = (fnv1a_hash(gl.as_bytes()) as usize) % N_GENRE_BUCKETS;
                feats.push((GENRE_OFFSET + g_b, 1.15));
                // Emit sub-genre tokens (e.g. "indie rock" -> "indie", "rock") for cross-genre generalization.
                for sub in gl.split([' ', '-']) {
                    let s = sub.trim();
                    if s.len() >= 3 && s != gl && genre_sub_seen.insert(s.to_string()) {
                        let sg_b = (fnv1a_hash(s.as_bytes()) as usize) % N_GENRE_BUCKETS;
                        feats.push((GENRE_OFFSET + sg_b, 0.55));
                    }
                }
                genre_count += 1;
                if genre_count >= 4 {
                    break;
                }
            }
        }

        let mut mood_seen = 0u16;
        for m in t.mood_list() {
            let idx = mood_to_idx(m);
            if (mood_seen & (1 << idx)) == 0 {
                mood_seen |= 1 << idx;
                feats.push((MOOD_OFFSET + idx, 0.95));
            }
        }
        // Infer coarse mood signals from genre/title when explicit mood tags are absent.
        if mood_seen == 0 {
            let combined = format!("{} {}", t.genre, t.title).to_lowercase();
            let inferred: &[(&[&str], Mood)] = &[
                (&["chill", "lofi", "lo-fi", "ambient", "downtempo", "relax"], Mood::Chill),
                (&["workout", "metal", "punk", "hardcore", "energy", "trap"], Mood::Energy),
                (&["dance", "house", "disco", "edm", "club", "party", "techno"], Mood::Party),
                (&["sad", "ballad", "melanchol", "blues", "heartbreak"], Mood::Melancholy),
                (&["acoustic", "unplugged", "folk", "singer-songwriter"], Mood::Acoustic),
                (&["indie", "rock", "grunge", "shoegaze", "alternative"], Mood::RockIndie),
                (&["soul", "r&b", "rnb", "funk", "groove", "jazz"], Mood::Groove),
            ];
            for (kws, m) in inferred {
                if kws.iter().any(|kw| combined.contains(kw)) {
                    let idx = mood_to_idx(*m);
                    if (mood_seen & (1 << idx)) == 0 {
                        mood_seen |= 1 << idx;
                        feats.push((MOOD_OFFSET + idx, 0.55));
                    }
                }
            }
        }

        let era_bucket = if t.year >= 1940 && t.year <= 2035 {
            ((t.year - 1940) / 10).clamp(0, 14) as usize
        } else {
            15
        };
        feats.push((ERA_OFFSET + era_bucket, 0.70));

        let mut tok_count = 0usize;
        let text_src = format!("{} {}", t.title, t.album);
        for raw in text_src.split(|c: char| !c.is_alphanumeric()) {
            let low = raw.trim().to_lowercase();
            if low.len() >= 3 && !matches!(low.as_str(), "the" | "and" | "feat" | "with" | "remaster" | "version") {
                let tb = (fnv1a_hash(low.as_bytes()) as usize) % N_TEXT_BUCKETS;
                feats.push((TEXT_OFFSET + tb, 0.35));
                tok_count += 1;
                if tok_count >= 8 {
                    break;
                }
            }
        }

        let rank_norm = parse_meta_rank_norm(&t.meta);
        let qual_norm = (t.quality_score() as f32 / 1000.0).clamp(0.0, 1.2);
        let dur_norm = if t.duration > 0 {
            ((t.duration as f32).ln_1p() / 7.0).clamp(0.0, 1.2)
        } else {
            0.7
        };
        let year_norm = if t.year >= 1940 && t.year <= 2035 {
            (t.year as f32 - 1990.0) / 35.0
        } else {
            0.0
        };
        (feats, [rank_norm, qual_norm, dur_norm, year_norm])
    }

    pub fn extract_query_features(query: &str, moods: &[Mood], genres: &[String]) -> Vec<(usize, f32)> {
        let mut feats = Vec::with_capacity(12);
        let mut tok_count = 0usize;
        for raw in query.split(|c: char| !c.is_alphanumeric() && c != '-') {
            let low = raw.trim().to_lowercase();
            if low.len() >= 2 {
                let tb = (fnv1a_hash(low.as_bytes()) as usize) % N_TEXT_BUCKETS;
                feats.push((TEXT_OFFSET + tb, 0.70));
                let ab = (fnv1a_hash(low.as_bytes()) as usize) % N_ARTIST_BUCKETS;
                feats.push((ARTIST_OFFSET + ab, 0.45));
                tok_count += 1;
                if tok_count >= 8 {
                    break;
                }
            }
        }
        for &m in moods {
            feats.push((MOOD_OFFSET + mood_to_idx(m), 0.95));
        }
        for g in genres.iter().take(4) {
            let gl = g.trim().to_lowercase();
            if gl.len() >= 2 {
                let gb = (fnv1a_hash(gl.as_bytes()) as usize) % N_GENRE_BUCKETS;
                feats.push((GENRE_OFFSET + gb, 0.95));
            }
        }
        feats
    }

    fn sum_sparse_features(&self, feats: &[(usize, f32)], scalars: Option<[f32; 4]>) -> [f32; D_MODEL] {
        let mut out = [0.0f32; D_MODEL];
        for &(row, w) in feats {
            let base = row * D_MODEL;
            let row_slice = &self.feature_weights[base..base + D_MODEL];
            for d in 0..D_MODEL {
                out[d] += row_slice[d] * w;
            }
        }
        if let Some(sc) = scalars {
            for k in 0..4 {
                let s = sc[k];
                if s != 0.0 {
                    let row = &self.scalar_proj.w[k * D_MODEL..(k + 1) * D_MODEL];
                    for d in 0..D_MODEL {
                        out[d] += row[d] * s;
                    }
                }
            }
        }
        out
    }

    /// Track tower on pre-extracted features: returns `(unit, raw, norm)`.
    fn encode_feats_unit(&self, feats: &[(usize, f32)], scalars: [f32; 4]) -> ([f32; D_MODEL], [f32; D_MODEL], f32) {
        let raw = self.sum_sparse_features(feats, Some(scalars));
        let mut proj = [0.0f32; D_MODEL];
        matmul_vec_add(&raw, &self.track_proj_w.w, D_MODEL, D_MODEL, &mut proj);
        let (unit, norm) = l2_normalize(&proj);
        (unit, raw, norm)
    }

    pub fn encode_track_unit(&self, t: &Track) -> ([f32; D_MODEL], Vec<(usize, f32)>, [f32; 4], [f32; D_MODEL], f32) {
        let (feats, scalars) = Self::extract_track_features(t);
        let (unit, raw, norm) = self.encode_feats_unit(&feats, scalars);
        (unit, feats, scalars, raw, norm)
    }

    /// Runs the 2-layer RoPE-QKVO + XSA Transformer over a sequence of items ending in `[USER_u]`.
    fn forward_context(&self, user_id: i64, seq_items: &[SeqItem]) -> ContextForwardCache {
        let capped_start = seq_items.len().saturating_sub(MAX_SEQ_LEN - 1);
        let items = &seq_items[capped_start..];
        let seq_len = items.len() + 1;

        let mut x0 = vec![0.0f32; seq_len * D_MODEL];
        let mut token_feats = Vec::with_capacity(seq_len);
        let mut token_scalars = Vec::with_capacity(seq_len);
        let mut token_actions = Vec::with_capacity(seq_len);

        for (pos, (feats, scalars, action)) in items.iter().enumerate() {
            let mut v = self.sum_sparse_features(feats, *scalars);
            let act_idx = (*action).min(N_ACTION_TYPES - 1);
            let act_row = &self.action_emb.w[act_idx * D_MODEL..(act_idx + 1) * D_MODEL];
            for d in 0..D_MODEL {
                v[d] += act_row[d];
            }
            x0[pos * D_MODEL..(pos + 1) * D_MODEL].copy_from_slice(&v);
            token_feats.push(feats.clone());
            token_scalars.push(*scalars);
            token_actions.push(Some(act_idx));
        }

        // Last token is [USER_u] so RoPE relative offset from the user token to the
        // most recent interaction is always -1 regardless of sequence length.
        let u_vec = self.user_row_vec(user_id);
        let u_pos = seq_len - 1;
        x0[u_pos * D_MODEL..(u_pos + 1) * D_MODEL].copy_from_slice(&u_vec);
        token_feats.push(Vec::new());
        token_scalars.push(None);
        token_actions.push(None);

        let mut h = x0;
        let mut block_caches = Vec::with_capacity(self.blocks.len());
        for blk in &self.blocks {
            let (next_h, bc) = blk.forward(&h, seq_len);
            h = next_h;
            block_caches.push(bc);
        }

        let final_rms_g: [f32; D_MODEL] = self.final_rms_g.w[..D_MODEL].try_into().unwrap();
        let h_last = &h[u_pos * D_MODEL..(u_pos + 1) * D_MODEL];
        let mut h_norm = [0.0f32; D_MODEL];
        let final_rms = rmsnorm_forward(h_last, &final_rms_g, &mut h_norm);

        let mut ctx_raw = [0.0f32; D_MODEL];
        matmul_vec_add(&h_norm, &self.ctx_proj_w.w, D_MODEL, D_MODEL, &mut ctx_raw);
        let (ctx_unit, ctx_norm) = l2_normalize(&ctx_raw);

        ContextForwardCache {
            user_id,
            seq_len,
            token_feats,
            token_scalars,
            token_actions,
            block_caches,
            h_last: h_last.try_into().unwrap(),
            final_rms,
            h_norm,
            ctx_unit,
            ctx_norm,
        }
    }

    /// One AdamW step of sampled-softmax InfoNCE over a batch. Each example ranks its positive
    /// against its own sampled negatives plus every other example's positive (in-batch negatives),
    /// skipping in-batch items the user is already known to like.
    fn train_step(
        &mut self,
        batch: &[&TrainingExample],
        table: &CorpusTable,
        users: &HashMap<i64, UserPrep>,
        lr: f32,
    ) -> f32 {
        if batch.is_empty() {
            return 0.0;
        }
        let wd = 1e-4f32;
        let mut block_grads: Vec<BlockGrad> = (0..self.blocks.len()).map(|_| BlockGrad::new()).collect();
        let mut g_final_rms_g = [0.0f32; D_MODEL];
        let mut g_ctx_w = vec![0.0f32; D_MODEL * D_MODEL];
        let mut g_trk_w = vec![0.0f32; D_MODEL * D_MODEL];
        let mut g_action = vec![0.0f32; N_ACTION_TYPES * D_MODEL];
        let mut g_scalar = vec![0.0f32; 4 * D_MODEL];
        let mut g_logit_scale = 0.0f32;

        let mut g_feats: HashMap<usize, [f32; D_MODEL]> = HashMap::new();
        let mut g_users: HashMap<i64, [f32; D_MODEL]> = HashMap::new();

        // Encode every distinct candidate in the batch exactly once.
        let mut local: HashMap<usize, usize> = HashMap::new();
        let mut cand_ids: Vec<usize> = Vec::new();
        for ex in batch {
            for &i in std::iter::once(&ex.positive).chain(ex.negatives.iter()) {
                local.entry(i).or_insert_with(|| {
                    cand_ids.push(i);
                    cand_ids.len() - 1
                });
            }
        }
        let mut units = Vec::with_capacity(cand_ids.len());
        let mut raws = Vec::with_capacity(cand_ids.len());
        let mut norms = Vec::with_capacity(cand_ids.len());
        for &i in &cand_ids {
            let (u, r, n) = self.encode_feats_unit(&table.feats[i], table.scalars[i]);
            units.push(u);
            raws.push(r);
            norms.push(n);
        }
        let mut g_units = vec![[0.0f32; D_MODEL]; cand_ids.len()];

        let inv_batch = 1.0 / (batch.len() as f32);
        let mut total_loss = 0.0f32;

        for (bi, ex) in batch.iter().enumerate() {
            let ctx_cache = self.forward_context(ex.user_id, &ex.context);

            let pos_song = table.song[ex.positive];
            let seen = users.get(&ex.user_id).map(|p| &p.seen_songs);
            let mut used_songs: HashSet<u64> = HashSet::new();
            used_songs.insert(pos_song);
            let mut cl: Vec<usize> = Vec::with_capacity(1 + ex.negatives.len() + batch.len());
            cl.push(local[&ex.positive]);
            for &n in &ex.negatives {
                if used_songs.insert(table.song[n]) {
                    cl.push(local[&n]);
                }
            }
            for (bj, other) in batch.iter().enumerate() {
                if bj == bi {
                    continue;
                }
                let s = table.song[other.positive];
                if seen.map_or(false, |ss| ss.contains(&s)) {
                    continue;
                }
                if used_songs.insert(s) {
                    cl.push(local[&other.positive]);
                }
            }
            if cl.len() < 2 {
                continue;
            }

            let mut cosines = Vec::with_capacity(cl.len());
            let mut max_l = f32::NEG_INFINITY;
            for &c in &cl {
                let mut cos = 0.0f32;
                for d in 0..D_MODEL {
                    cos += ctx_cache.ctx_unit[d] * units[c][d];
                }
                cosines.push(cos);
                max_l = max_l.max(cos * self.logit_scale);
            }
            let mut probs: Vec<f32> = cosines.iter().map(|c| (c * self.logit_scale - max_l).exp()).collect();
            let sum_exp: f32 = probs.iter().sum::<f32>().max(1e-12);
            for p in &mut probs {
                *p /= sum_exp;
            }
            let ce_loss = -(probs[0].max(1e-12)).ln();
            let ex_w = ex.weight * inv_batch;
            total_loss += ce_loss * ex_w;

            let mut g_ctx_unit = [0.0f32; D_MODEL];
            for (k, &c) in cl.iter().enumerate() {
                let target = if k == 0 { 1.0f32 } else { 0.0f32 };
                let g_logit = (probs[k] - target) * ex_w;
                g_logit_scale += g_logit * cosines[k];
                let g_cos = g_logit * self.logit_scale;
                for d in 0..D_MODEL {
                    g_ctx_unit[d] += g_cos * units[c][d];
                    g_units[c][d] += g_cos * ctx_cache.ctx_unit[d];
                }
            }

            // Backprop through ctx_proj -> final_rms -> Transformer blocks -> sequence inputs
            let g_ctx_raw = l2_normalize_backward(&g_ctx_unit, &ctx_cache.ctx_unit, ctx_cache.ctx_norm);
            let mut g_h_norm = [0.0f32; D_MODEL];
            matmul_backward(
                &g_ctx_raw,
                &ctx_cache.h_norm,
                &self.ctx_proj_w.w,
                D_MODEL,
                D_MODEL,
                &mut g_h_norm,
                &mut g_ctx_w,
            );

            let final_rms_g: [f32; D_MODEL] = self.final_rms_g.w[..D_MODEL].try_into().unwrap();
            let mut g_h_seq = vec![0.0f32; ctx_cache.seq_len * D_MODEL];
            let u_pos = ctx_cache.seq_len - 1;
            rmsnorm_backward(
                &g_h_norm,
                &ctx_cache.h_last,
                ctx_cache.final_rms,
                &final_rms_g,
                &mut g_h_seq[u_pos * D_MODEL..(u_pos + 1) * D_MODEL],
                &mut g_final_rms_g,
            );

            let mut g_h = g_h_seq;
            for l in (0..self.blocks.len()).rev() {
                g_h = self.blocks[l].backward(&g_h, &ctx_cache.block_caches[l], ctx_cache.seq_len, &mut block_grads[l]);
            }

            for pos in 0..ctx_cache.seq_len {
                let gp: [f32; D_MODEL] = g_h[pos * D_MODEL..(pos + 1) * D_MODEL].try_into().unwrap();
                if let Some(act_idx) = ctx_cache.token_actions[pos] {
                    let ga = &mut g_action[act_idx * D_MODEL..(act_idx + 1) * D_MODEL];
                    for d in 0..D_MODEL {
                        ga[d] += gp[d];
                    }
                    accumulate_sparse_feature_grad(
                        &ctx_cache.token_feats[pos],
                        ctx_cache.token_scalars[pos],
                        &gp,
                        &mut g_feats,
                        &mut g_scalar,
                    );
                } else {
                    let entry = g_users.entry(ctx_cache.user_id).or_insert([0.0; D_MODEL]);
                    for d in 0..D_MODEL {
                        entry[d] += gp[d];
                    }
                }
            }
        }

        // Backprop the track tower once per distinct candidate.
        for (c, &ti) in cand_ids.iter().enumerate() {
            if g_units[c].iter().all(|&g| g == 0.0) {
                continue;
            }
            let g_proj = l2_normalize_backward(&g_units[c], &units[c], norms[c]);
            let mut g_raw = [0.0f32; D_MODEL];
            matmul_backward(&g_proj, &raws[c], &self.track_proj_w.w, D_MODEL, D_MODEL, &mut g_raw, &mut g_trk_w);
            accumulate_sparse_feature_grad(&table.feats[ti], Some(table.scalars[ti]), &g_raw, &mut g_feats, &mut g_scalar);
        }

        // Global gradient norm clipping (max_norm = 1.0)
        let mut sq_norm = 0.0f32;
        for bg in &block_grads {
            sq_norm += bg.sq_norm();
        }
        for &v in g_final_rms_g.iter().chain(&g_ctx_w).chain(&g_trk_w).chain(&g_action).chain(&g_scalar) {
            sq_norm += v * v;
        }
        sq_norm += g_logit_scale * g_logit_scale;
        for g in g_feats.values().chain(g_users.values()) {
            for &v in g {
                sq_norm += v * v;
            }
        }

        let grad_norm = sq_norm.sqrt();
        if grad_norm > 1.0 {
            let clip = 1.0 / grad_norm;
            for bg in &mut block_grads {
                bg.scale(clip);
            }
            for v in g_final_rms_g
                .iter_mut()
                .chain(g_ctx_w.iter_mut())
                .chain(g_trk_w.iter_mut())
                .chain(g_action.iter_mut())
                .chain(g_scalar.iter_mut())
            {
                *v *= clip;
            }
            g_logit_scale *= clip;
            for g in g_feats.values_mut().chain(g_users.values_mut()) {
                for v in g.iter_mut() {
                    *v *= clip;
                }
            }
        }

        self.step_count += 1;
        let step_u32 = self.step_count.min(u32::MAX as u64) as u32;

        for (l, blk) in self.blocks.iter_mut().enumerate() {
            blk.apply_adamw(&block_grads[l], lr, wd, step_u32);
        }
        self.final_rms_g.step_adamw(&g_final_rms_g, lr, 0.0, step_u32);
        self.ctx_proj_w.step_adamw(&g_ctx_w, lr, wd, step_u32);
        self.track_proj_w.step_adamw(&g_trk_w, lr, wd, step_u32);
        self.action_emb.step_adamw(&g_action, lr, wd, step_u32);
        self.scalar_proj.step_adamw(&g_scalar, lr, wd, step_u32);

        // Sparse AdamW for touched shared feature rows
        let b1 = 0.9f32;
        let b2 = 0.999f32;
        let eps = 1e-8f32;
        for (row, g_row) in g_feats {
            self.feature_steps[row] = self.feature_steps[row].saturating_add(1);
            let s = self.feature_steps[row] as i32;
            let bc1 = 1.0 - b1.powi(s);
            let bc2 = 1.0 - b2.powi(s);
            let base = row * D_MODEL;
            for d in 0..D_MODEL {
                let idx = base + d;
                let g = g_row[d];
                self.feature_m[idx] = b1 * self.feature_m[idx] + (1.0 - b1) * g;
                self.feature_v[idx] = b2 * self.feature_v[idx] + (1.0 - b2) * g * g;
                let mh = self.feature_m[idx] / bc1;
                let vh = self.feature_v[idx] / bc2;
                self.feature_weights[idx] -= lr * (mh / (vh.sqrt() + eps) + wd * self.feature_weights[idx]);
            }
        }

        // Sparse AdamW for touched per-user embeddings
        for (uid, g_u) in g_users {
            self.get_or_insert_user_row(uid).step_adamw(&g_u, lr, wd);
        }

        // Adam update for logit_scale
        self.logit_scale_m = b1 * self.logit_scale_m + (1.0 - b1) * g_logit_scale;
        self.logit_scale_v = b2 * self.logit_scale_v + (1.0 - b2) * g_logit_scale * g_logit_scale;
        let mh = self.logit_scale_m / (1.0 - b1.powi(step_u32 as i32));
        let vh = self.logit_scale_v / (1.0 - b2.powi(step_u32 as i32));
        self.logit_scale = (self.logit_scale - lr * (mh / (vh.sqrt() + eps))).clamp(3.0, 20.0);

        if self.step_count == 1 {
            self.avg_loss = total_loss;
        } else {
            self.avg_loss = 0.9 * self.avg_loss + 0.1 * total_loss;
        }
        self.last_trained_at = Utc::now().timestamp();
        total_loss
    }

    /// Linear warm-up over the first 40 steps, then inverse-sqrt decay with a floor.
    fn learning_rate(&self) -> f32 {
        let s = self.step_count as f32;
        let warm = ((s + 1.0) / 40.0).min(1.0);
        (0.004 * warm / (1.0 + s * 0.0004).sqrt()).max(0.0006)
    }

    fn to_checkpoint_str(&self) -> Option<String> {
        use base64::Engine;
        let json = serde_json::to_vec(self).ok()?;
        let mut enc = GzEncoder::new(Vec::new(), Compression::default());
        enc.write_all(&json).ok()?;
        let gz = enc.finish().ok()?;
        Some(base64::engine::general_purpose::STANDARD.encode(gz))
    }

    fn from_checkpoint_str(raw: &str) -> Option<Self> {
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD.decode(raw.trim()).ok()?;
        let mut dec = GzDecoder::new(&bytes[..]);
        let mut json = Vec::new();
        dec.read_to_end(&mut json).ok()?;
        let net: Self = serde_json::from_slice(&json).ok()?;
        net.shapes_ok().then_some(net)
    }
}

fn accumulate_sparse_feature_grad(
    feats: &[(usize, f32)],
    scalars: Option<[f32; 4]>,
    g_out: &[f32; D_MODEL],
    g_feats: &mut HashMap<usize, [f32; D_MODEL]>,
    g_scalar: &mut [f32],
) {
    for &(row, w) in feats {
        let entry = g_feats.entry(row).or_insert([0.0; D_MODEL]);
        for d in 0..D_MODEL {
            entry[d] += g_out[d] * w;
        }
    }
    if let Some(sc) = scalars {
        for k in 0..4 {
            let s = sc[k];
            if s != 0.0 {
                let gs = &mut g_scalar[k * D_MODEL..(k + 1) * D_MODEL];
                for d in 0..D_MODEL {
                    gs[d] += g_out[d] * s;
                }
            }
        }
    }
}

struct ContextForwardCache {
    user_id: i64,
    seq_len: usize,
    token_feats: Vec<Vec<(usize, f32)>>,
    token_scalars: Vec<Option<[f32; 4]>>,
    token_actions: Vec<Option<usize>>,
    block_caches: Vec<BlockCache>,
    h_last: [f32; D_MODEL],
    final_rms: f32,
    h_norm: [f32; D_MODEL],
    ctx_unit: [f32; D_MODEL],
    ctx_norm: f32,
}

// ---------------------------------------------------------------------------
// Listens: raw events collapsed into one outcome per actual listen
// ---------------------------------------------------------------------------

/// What happened during one listen. Ordered by how strongly it overrides earlier signals
/// for the same listen (a play that ends in a skip is a skip; a completed track that gets
/// starred is a save; anything followed by "not interested" is a rejection).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Outcome {
    /// Playback started but never resolved (still playing, tab closed, or a now-playing ping).
    Started,
    /// Picked from search results; explicit intent even if unresolved.
    Picked,
    Skipped,
    Completed,
    Saved,
    Rejected,
}

#[derive(Clone, Debug)]
struct Listen {
    track_id: String,
    outcome: Outcome,
    action: usize,
    at: i64,
}

fn classify_event(kind: &str, value: f64) -> Option<(Outcome, usize)> {
    match kind {
        "play" | "mix_play" => Some((Outcome::Started, ACTION_PLAY)),
        "search_pick" => Some((Outcome::Picked, ACTION_PLAY)),
        "skip" => Some((Outcome::Skipped, ACTION_SKIP)),
        "complete" => Some((Outcome::Completed, ACTION_COMPLETE)),
        "star" => Some((Outcome::Saved, ACTION_STAR)),
        "playlist_add" => Some((Outcome::Saved, ACTION_PLAYLIST)),
        "rate" if value >= 4.0 => Some((Outcome::Saved, ACTION_STAR)),
        "rate" if value > 0.0 && value <= 2.0 => Some((Outcome::Rejected, ACTION_DISLIKE)),
        "not_interested" | "unstar" | "playlist_remove" => Some((Outcome::Rejected, ACTION_DISLIKE)),
        _ => None,
    }
}

/// Collapses a chronological event stream into listens. The web player and Subsonic clients
/// emit `play` when a track starts and `complete`/`skip` when it ends; treating those as two
/// separate interactions taught the old model to "predict" the track it had just been shown,
/// and counted every skipped track as a positive play.
fn collapse_events<'a, I>(events: I) -> Vec<Listen>
where
    I: IntoIterator<Item = (&'a str, &'a str, f64, i64)>,
{
    let mut out: Vec<Listen> = Vec::new();
    for (tid, kind, value, at) in events {
        let Some((outcome, action)) = classify_event(kind, value) else {
            continue;
        };
        if let Some(last) = out.last_mut() {
            let same = last.track_id == tid && (at - last.at).abs() <= LISTEN_MERGE_WINDOW_SECS;
            let is_start = matches!(outcome, Outcome::Started | Outcome::Picked);
            let last_open = matches!(last.outcome, Outcome::Started | Outcome::Picked);
            // A new start after a resolved listen is a replay, not part of the same listen.
            if same && (!is_start || last_open) {
                if outcome > last.outcome {
                    last.outcome = outcome;
                    last.action = action;
                }
                last.at = at;
                continue;
            }
        }
        out.push(Listen {
            track_id: tid.to_string(),
            outcome,
            action,
            at,
        });
    }
    out
}

fn listen_target_weight(o: Outcome) -> Option<f32> {
    match o {
        Outcome::Saved => Some(1.3),
        Outcome::Completed => Some(1.0),
        Outcome::Picked => Some(0.6),
        _ => None,
    }
}

fn is_positive_outcome(o: Outcome) -> bool {
    listen_target_weight(o).is_some()
}

/// Shared sequence policy for training and inference: stars (older side) only when history is
/// short, then chronological history, capped to `cap` tokens.
fn assemble_profile_sequence(mut history: Vec<SeqItem>, stars: Vec<SeqItem>, cap: usize) -> Vec<SeqItem> {
    if history.len() > cap {
        history.drain(0..history.len() - cap);
    }
    let star_budget = if history.len() < PROFILE_SHORT_HISTORY {
        STAR_PREFIX_MAX.min(cap - history.len())
    } else {
        0
    };
    let mut seq: Vec<SeqItem> = stars.into_iter().take(star_budget).collect();
    seq.extend(history);
    seq
}

// ---------------------------------------------------------------------------
// Training corpus
// ---------------------------------------------------------------------------

/// Pre-extracted features for every track in the corpus, indexed densely.
struct CorpusTable {
    ids: Vec<String>,
    feats: Vec<Vec<(usize, f32)>>,
    scalars: Vec<[f32; 4]>,
    song: Vec<u64>,
    artist: Vec<u64>,
    genres: Vec<Vec<u64>>,
    index: HashMap<String, usize>,
}

impl CorpusTable {
    fn build(corpus: &NeuralTrainingCorpus) -> Self {
        let mut ids: Vec<&String> = corpus.tracks.keys().collect();
        ids.sort();
        let n = ids.len();
        let mut tbl = CorpusTable {
            ids: Vec::with_capacity(n),
            feats: Vec::with_capacity(n),
            scalars: Vec::with_capacity(n),
            song: Vec::with_capacity(n),
            artist: Vec::with_capacity(n),
            genres: Vec::with_capacity(n),
            index: HashMap::with_capacity(n),
        };
        for id in ids {
            let t = &corpus.tracks[id];
            let (f, s) = TransformerNet::extract_track_features(t);
            tbl.index.insert(id.clone(), tbl.ids.len());
            tbl.ids.push(id.clone());
            tbl.feats.push(f);
            tbl.scalars.push(s);
            tbl.song.push(fnv1a_hash(t.song_key().as_bytes()));
            let pa = slug(&primary_artist(&t.artist));
            tbl.artist.push(if pa.is_empty() || pa == "unknown-artist" {
                0
            } else {
                fnv1a_hash(pa.as_bytes())
            });
            tbl.genres.push(genre_hashes(&t.genre));
        }
        tbl
    }

    fn len(&self) -> usize {
        self.ids.len()
    }

    fn idx(&self, id: &str) -> Option<usize> {
        self.index.get(id).copied()
    }

    fn item(&self, i: usize, action: usize) -> SeqItem {
        (self.feats[i].clone(), Some(self.scalars[i]), action)
    }
}

fn genre_hashes(genre: &str) -> Vec<u64> {
    genre
        .split([',', ';', '/'])
        .map(|g| g.trim().to_lowercase())
        .filter(|g| g.len() >= 2)
        .take(4)
        .map(|g| fnv1a_hash(g.as_bytes()))
        .collect()
}

/// Everything known about one user's taste, in table indices.
#[derive(Default)]
struct UserPrep {
    listens: Vec<(usize, Listen)>,
    stars: Vec<usize>,
    high_rated: Vec<usize>,
    disliked: Vec<usize>,
    /// Stars, high ratings and manual playlist tracks.
    explicit_songs: HashSet<u64>,
    /// Explicit songs plus completed / saved / picked listens.
    pos_songs: HashSet<u64>,
    /// Positive songs plus unresolved plays: never used as negatives for this user.
    seen_songs: HashSet<u64>,
    pos_list: Vec<usize>,
}

fn prepare_users(corpus: &NeuralTrainingCorpus, table: &CorpusTable) -> HashMap<i64, UserPrep> {
    let mut playlist_tracks: HashMap<i64, Vec<usize>> = HashMap::new();
    for (uid, _, tids) in &corpus.playlists {
        let e = playlist_tracks.entry(*uid).or_default();
        e.extend(tids.iter().filter_map(|id| table.idx(id)));
    }

    let mut out = HashMap::new();
    for &uid in &corpus.user_ids {
        let mut p = UserPrep::default();
        if let Some(evs) = corpus.user_events.get(&uid) {
            let listens = collapse_events(evs.iter().map(|(tid, kind, v, at)| (tid.as_str(), kind.as_str(), *v, *at)));
            for l in listens {
                if let Some(i) = table.idx(&l.track_id) {
                    p.listens.push((i, l));
                }
            }
        }
        if let Some(stars) = corpus.user_stars.get(&uid) {
            p.stars = stars.iter().filter_map(|id| table.idx(id)).collect();
        }
        if let Some(ratings) = corpus.user_ratings.get(&uid) {
            for (id, r) in ratings {
                if let Some(i) = table.idx(id) {
                    if *r >= 4 {
                        p.high_rated.push(i);
                    } else if *r <= 2 {
                        p.disliked.push(i);
                    }
                }
            }
        }
        if let Some(d) = corpus.user_disliked.get(&uid) {
            p.disliked.extend(d.iter().filter_map(|id| table.idx(id)));
        }
        let pl = playlist_tracks.remove(&uid).unwrap_or_default();

        for &i in p.stars.iter().chain(&p.high_rated).chain(&pl) {
            p.explicit_songs.insert(table.song[i]);
        }
        p.pos_songs = p.explicit_songs.clone();
        let mut pos_list: Vec<usize> = p.stars.iter().chain(&p.high_rated).chain(&pl).copied().collect();
        for (i, l) in &p.listens {
            match l.outcome {
                o if is_positive_outcome(o) => {
                    p.pos_songs.insert(table.song[*i]);
                    pos_list.push(*i);
                }
                Outcome::Skipped | Outcome::Rejected => p.disliked.push(*i),
                _ => {}
            }
        }
        p.seen_songs = p.pos_songs.clone();
        for (i, l) in &p.listens {
            if l.outcome == Outcome::Started {
                p.seen_songs.insert(table.song[*i]);
            }
        }
        let mut seen = HashSet::new();
        p.pos_list = pos_list.into_iter().filter(|&i| seen.insert(table.song[i])).collect();
        let mut seen = HashSet::new();
        let pos_songs = &p.pos_songs;
        p.disliked.retain(|&i| !pos_songs.contains(&table.song[i]) && seen.insert(table.song[i]));
        out.insert(uid, p);
    }
    out
}

/// Profile context from the user's first `upto` listens (stars prefixed when history is short).
fn profile_sequence(p: &UserPrep, table: &CorpusTable, upto: usize, exclude_song: Option<u64>, cap: usize) -> Vec<SeqItem> {
    let upto = upto.min(p.listens.len());
    let start = upto.saturating_sub(cap);
    let window = &p.listens[start..upto];
    let history: Vec<SeqItem> = window.iter().map(|(i, l)| table.item(*i, l.action)).collect();
    let stars = if history.len() < PROFILE_SHORT_HISTORY {
        let hist_songs: HashSet<u64> = window.iter().map(|(i, _)| table.song[*i]).collect();
        p.stars
            .iter()
            .filter(|&&i| !hist_songs.contains(&table.song[i]) && Some(table.song[i]) != exclude_song)
            .take(STAR_PREFIX_MAX)
            .map(|&i| table.item(i, ACTION_STAR))
            .collect()
    } else {
        Vec::new()
    };
    assemble_profile_sequence(history, stars, cap)
}

#[derive(Clone)]
struct TrainingExample {
    user_id: i64,
    context: Vec<SeqItem>,
    positive: usize,
    negatives: Vec<usize>,
    weight: f32,
}

/// A held-out listen: the model must rank `target` above random catalog tracks given `context`.
struct EvalItem {
    user_id: i64,
    context: Vec<SeqItem>,
    target: usize,
}

struct TrainingData {
    examples: Vec<TrainingExample>,
    holdouts: Vec<EvalItem>,
}

struct NegSampler<'a> {
    table: &'a CorpusTable,
    users: &'a HashMap<i64, UserPrep>,
    donor_users: Vec<i64>,
    genre_index: HashMap<u64, Vec<usize>>,
}

impl<'a> NegSampler<'a> {
    fn new(table: &'a CorpusTable, users: &'a HashMap<i64, UserPrep>) -> Self {
        let mut donor_users: Vec<i64> = users.iter().filter(|(_, p)| !p.pos_list.is_empty()).map(|(u, _)| *u).collect();
        donor_users.sort();
        let mut genre_index: HashMap<u64, Vec<usize>> = HashMap::new();
        for (i, gs) in table.genres.iter().enumerate() {
            for &g in gs {
                genre_index.entry(g).or_default().push(i);
            }
        }
        Self {
            table,
            users,
            donor_users,
            genre_index,
        }
    }

    /// Mix of personalized (the user's skips/dislikes), collaborative (liked by a user with
    /// disjoint taste), hard (same genre, different artist) and random catalog negatives.
    fn sample(&self, uid: i64, pos: usize, rng: &mut StdRng) -> Vec<usize> {
        let t = self.table;
        let prep = self.users.get(&uid);
        let is_seen = |s: u64| prep.map_or(false, |p| p.seen_songs.contains(&s));
        let pos_artist = t.artist[pos];
        let mut out = Vec::with_capacity(N_SAMPLED_NEGATIVES);
        let mut used: HashSet<u64> = HashSet::new();
        used.insert(t.song[pos]);
        let mut try_push = |i: usize, out: &mut Vec<usize>| {
            if out.len() < N_SAMPLED_NEGATIVES && used.insert(t.song[i]) {
                out.push(i);
            }
        };

        if let Some(p) = prep {
            for _ in 0..3 {
                if out.len() >= 2 || p.disliked.is_empty() {
                    break;
                }
                try_push(p.disliked[rng.gen_range(0..p.disliked.len())], &mut out);
            }
        }

        let goal = out.len() + 2;
        if self.donor_users.len() > 1 {
            for _ in 0..6 {
                if out.len() >= goal {
                    break;
                }
                let other = self.donor_users[rng.gen_range(0..self.donor_users.len())];
                if other == uid {
                    continue;
                }
                let op = &self.users[&other];
                if prep.map_or(false, |p| !p.pos_songs.is_disjoint(&op.pos_songs)) {
                    continue;
                }
                let i = op.pos_list[rng.gen_range(0..op.pos_list.len())];
                if !is_seen(t.song[i]) {
                    try_push(i, &mut out);
                }
            }
        }

        let goal = out.len() + 2;
        if !t.genres[pos].is_empty() {
            for _ in 0..10 {
                if out.len() >= goal {
                    break;
                }
                let g = t.genres[pos][rng.gen_range(0..t.genres[pos].len())];
                if let Some(bucket) = self.genre_index.get(&g) {
                    let i = bucket[rng.gen_range(0..bucket.len())];
                    if t.artist[i] != pos_artist && !is_seen(t.song[i]) {
                        try_push(i, &mut out);
                    }
                }
            }
        }

        let mut tries = 0;
        while out.len() < N_SAMPLED_NEGATIVES && tries < 48 && t.len() > 1 {
            tries += 1;
            let i = rng.gen_range(0..t.len());
            if is_seen(t.song[i]) || (tries < 36 && pos_artist != 0 && t.artist[i] == pos_artist) {
                continue;
            }
            try_push(i, &mut out);
        }
        out
    }
}

/// Randomly drops ~10% of context tokens (never the last one) for robustness.
fn token_dropout(seq: Vec<SeqItem>, rng: &mut StdRng) -> Vec<SeqItem> {
    if seq.len() <= 6 {
        return seq;
    }
    let last = seq.len() - 1;
    seq.into_iter()
        .enumerate()
        .filter(|(k, _)| *k == last || !rng.gen_bool(0.1))
        .map(|(_, s)| s)
        .collect()
}

fn build_training_data(
    corpus: &NeuralTrainingCorpus,
    table: &CorpusTable,
    users: &HashMap<i64, UserPrep>,
    seed: u64,
) -> TrainingData {
    let mut examples = Vec::new();
    let mut holdouts = Vec::new();
    if table.len() < 4 {
        return TrainingData { examples, holdouts };
    }
    let mut rng = StdRng::seed_from_u64(seed);
    let sampler = NegSampler::new(table, users);
    let mut uids: Vec<i64> = users.keys().copied().collect();
    uids.sort();
    let mut holdout_songs: HashMap<i64, u64> = HashMap::new();

    // Task 1: next positive listen given the listens before it (mirrors For You / Discover).
    for &uid in &uids {
        let p = &users[&uid];
        let all_targets: Vec<usize> = (1..p.listens.len()).filter(|&i| is_positive_outcome(p.listens[i].1.outcome)).collect();
        let targets = &all_targets[all_targets.len().saturating_sub(80)..];

        // Hold out the most recent listen of a track the user discovered by listening (not
        // starred / rated / playlisted, not heard earlier) to measure real generalization.
        let holdout_at = if targets.len() >= 4 {
            targets.iter().rev().copied().find(|&i| {
                let s = table.song[p.listens[i].0];
                i >= 3 && !p.explicit_songs.contains(&s) && !p.listens[..i].iter().any(|(j, _)| table.song[*j] == s)
            })
        } else {
            None
        };
        if let Some(h) = holdout_at {
            let tgt = p.listens[h].0;
            holdout_songs.insert(uid, table.song[tgt]);
            holdouts.push(EvalItem {
                user_id: uid,
                context: profile_sequence(p, table, h, Some(table.song[tgt]), MAX_SEQ_LEN - 1),
                target: tgt,
            });
        }

        for &i in targets {
            if Some(i) == holdout_at {
                continue;
            }
            let (pos, ref l) = p.listens[i];
            let pos_song = table.song[pos];
            let mut w = listen_target_weight(l.outcome).unwrap_or(1.0);
            if p.listens[..i].iter().any(|(j, _)| table.song[*j] == pos_song) {
                // Repeat listens are real but trivially predictable; don't let them dominate.
                w *= 0.35;
            }
            // 25%: session/radio-seed variant, the previous positive listen becomes an explicit seed.
            let seed_k = if rng.gen_bool(0.25) {
                (0..i)
                    .rev()
                    .take(6)
                    .find(|&k| is_positive_outcome(p.listens[k].1.outcome) && table.song[p.listens[k].0] != pos_song)
            } else {
                None
            };
            let ctx = match seed_k {
                Some(k) => {
                    let mut seq = profile_sequence(p, table, k, Some(pos_song), CONTEXT_HISTORY_CAP);
                    seq.push(table.item(p.listens[k].0, ACTION_SEED));
                    seq
                }
                None => profile_sequence(p, table, i, Some(pos_song), MAX_SEQ_LEN - 1),
            };
            let ctx = token_dropout(ctx, &mut rng);
            let negs = sampler.sample(uid, pos, &mut rng);
            if !negs.is_empty() {
                examples.push(TrainingExample { user_id: uid, context: ctx, positive: pos, negatives: negs, weight: w });
            }
        }
    }

    // Task 2: manual playlist completion (mirrors Playlist DNA: history + seed tracks + name).
    for (uid, name, tids) in &corpus.playlists {
        let mut seen_s = HashSet::new();
        let pl: Vec<usize> = tids.iter().filter_map(|id| table.idx(id)).filter(|&i| seen_s.insert(table.song[i])).collect();
        if pl.len() < 2 {
            continue;
        }
        let q = TransformerNet::extract_query_features(name, &[], &[]);
        let mut holds: Vec<usize> = (0..pl.len()).collect();
        holds.shuffle(&mut rng);
        holds.truncate(12);
        for h in holds {
            let pos = pl[h];
            let pos_song = table.song[pos];
            let mut w = 1.1f32;
            let mut ctx = Vec::new();
            if let Some(p) = users.get(uid) {
                if rng.gen_bool(0.5) {
                    let start = p.listens.len().saturating_sub(CONTEXT_HISTORY_CAP);
                    if p.listens[start..].iter().any(|(j, _)| table.song[*j] == pos_song) {
                        w *= 0.5;
                    }
                    ctx = profile_sequence(p, table, p.listens.len(), Some(pos_song), CONTEXT_HISTORY_CAP);
                }
            }
            let mut others: Vec<usize> = (0..pl.len()).filter(|&k| k != h).collect();
            others.sort_by_key(|&k| (k as i64 - h as i64).abs());
            others.truncate(MAX_SEED_TOKENS);
            others.sort();
            for k in others {
                ctx.push(table.item(pl[k], ACTION_SEED));
            }
            if !q.is_empty() && rng.gen_bool(0.7) {
                ctx.push((q.clone(), None, ACTION_QUERY));
            }
            let negs = sampler.sample(*uid, pos, &mut rng);
            if !negs.is_empty() {
                examples.push(TrainingExample { user_id: *uid, context: ctx, positive: pos, negatives: negs, weight: w });
            }
        }
    }

    // Task 3: explicit favourites (stars / high ratings) predict each other; covers users with
    // little listening history, whose inference context is a star prefix.
    for &uid in &uids {
        let p = &users[&uid];
        let mut s = HashSet::new();
        let positives: Vec<usize> = p.stars.iter().chain(&p.high_rated).copied().filter(|&i| s.insert(table.song[i])).collect();
        if positives.len() < 2 {
            continue;
        }
        let mut order: Vec<usize> = (0..positives.len()).collect();
        order.shuffle(&mut rng);
        for &k in order.iter().take(8) {
            let pos = positives[k];
            let mut ctx = Vec::new();
            if !p.disliked.is_empty() && rng.gen_bool(0.6) {
                ctx.push(table.item(p.disliked[rng.gen_range(0..p.disliked.len())], ACTION_DISLIKE));
            }
            let mut others: Vec<usize> = positives.iter().copied().filter(|&i| i != pos).collect();
            others.shuffle(&mut rng);
            others.truncate(10);
            for i in others {
                ctx.push(table.item(i, ACTION_STAR));
            }
            let negs = sampler.sample(uid, pos, &mut rng);
            if !negs.is_empty() {
                examples.push(TrainingExample { user_id: uid, context: ctx, positive: pos, negatives: negs, weight: 1.2 });
            }
        }
    }

    // Task 4: mood / genre query alignment (mirrors Mood, Explore and Instant mixes, including
    // logged-out use through the shared user-0 embedding).
    for &uid in &uids {
        let p = &users[&uid];
        let hold = holdout_songs.get(&uid).copied();
        let mut cands: Vec<usize> = p.pos_list.iter().copied().filter(|&i| Some(table.song[i]) != hold).collect();
        cands.shuffle(&mut rng);
        cands.truncate(6);
        for pos in cands {
            let Some(t) = corpus.tracks.get(&table.ids[pos]) else {
                continue;
            };
            let moods = t.mood_list();
            let genres: Vec<String> = t
                .genre
                .split([',', ';', '/'])
                .map(|g| g.trim().to_string())
                .filter(|g| g.len() >= 2)
                .take(2)
                .collect();
            if moods.is_empty() && genres.is_empty() {
                continue;
            }
            let r: f32 = rng.gen();
            let (q_moods, q_genres): (Vec<Mood>, Vec<String>) = if r < 0.4 && !moods.is_empty() {
                (moods, Vec::new())
            } else if r < 0.75 && !genres.is_empty() {
                (Vec::new(), genres[..1].to_vec())
            } else {
                (moods, genres)
            };
            let qf = TransformerNet::extract_query_features("", &q_moods, &q_genres);
            if qf.is_empty() {
                continue;
            }
            let pos_song = table.song[pos];
            let anonymous = rng.gen_bool(0.25);
            let mut w = 0.7f32;
            let mut ctx = if anonymous {
                Vec::new()
            } else {
                let start = p.listens.len().saturating_sub(CONTEXT_HISTORY_CAP);
                if p.listens[start..].iter().any(|(j, _)| table.song[*j] == pos_song) {
                    w *= 0.5;
                }
                profile_sequence(p, table, p.listens.len(), Some(pos_song), CONTEXT_HISTORY_CAP)
            };
            ctx.push((qf, None, ACTION_QUERY));
            let ctx_user = if anonymous { 0 } else { uid };
            let negs = sampler.sample(uid, pos, &mut rng);
            if !negs.is_empty() {
                examples.push(TrainingExample { user_id: ctx_user, context: ctx, positive: pos, negatives: negs, weight: w });
            }
        }
    }

    // Balance training weights across users so high-volume listeners don't drown out quieter users.
    let mut per_user_counts: HashMap<i64, usize> = HashMap::new();
    for ex in &examples {
        *per_user_counts.entry(ex.user_id).or_insert(0) += 1;
    }
    for ex in &mut examples {
        let cnt = per_user_counts.get(&ex.user_id).copied().unwrap_or(1) as f32;
        ex.weight *= (12.0 / cnt).sqrt().clamp(0.35, 1.5);
    }

    TrainingData { examples, holdouts }
}

fn rank_of_first(scores: &[f32]) -> f32 {
    let t = scores[0];
    let mut r = 1.0f32;
    for &s in &scores[1..] {
        if s > t {
            r += 1.0;
        } else if s == t {
            r += 0.5;
        }
    }
    r
}

/// Ranks each held-out listen against random unseen catalog tracks and compares the model
/// with a popularity baseline and an artist/genre-overlap content baseline.
fn evaluate(net: &TransformerNet, table: &CorpusTable, users: &HashMap<i64, UserPrep>, items: &[EvalItem], seed: u64) -> EvalReport {
    let mut rep = EvalReport {
        evaluated_at: Utc::now().timestamp(),
        ..Default::default()
    };
    if items.is_empty() || table.len() < 30 {
        return rep;
    }
    let mut unit_cache: HashMap<usize, [f32; D_MODEL]> = HashMap::new();
    let (mut m_rr, mut m_hit, mut p_rr, mut c_rr, mut c_hit) = (0f64, 0f64, 0f64, 0f64, 0f64);
    let mut n = 0usize;
    for (k, item) in items.iter().enumerate() {
        let mut rng = StdRng::seed_from_u64(seed ^ (k as u64 + 1).wrapping_mul(0x9e3779b97f4a7c15));
        let seen = users.get(&item.user_id).map(|p| &p.seen_songs);
        let mut pool = vec![item.target];
        let mut used: HashSet<u64> = HashSet::from([table.song[item.target]]);
        let mut tries = 0;
        while pool.len() <= EVAL_POOL_NEGATIVES && tries < EVAL_POOL_NEGATIVES * 5 {
            tries += 1;
            let i = rng.gen_range(0..table.len());
            let s = table.song[i];
            if seen.map_or(false, |ss| ss.contains(&s)) {
                continue;
            }
            if used.insert(s) {
                pool.push(i);
            }
        }
        if pool.len() < 21 {
            continue;
        }

        let ctx = net.forward_context(item.user_id, &item.context).ctx_unit;
        let mut profile_rows: HashMap<usize, f32> = HashMap::new();
        for (feats, _, act) in &item.context {
            if *act == ACTION_SKIP || *act == ACTION_DISLIKE {
                continue;
            }
            for &(row, _) in feats {
                if (ARTIST_OFFSET..TEXT_OFFSET).contains(&row) {
                    *profile_rows.entry(row).or_insert(0.0) += 2.0;
                } else if (GENRE_OFFSET..MOOD_OFFSET).contains(&row) {
                    *profile_rows.entry(row).or_insert(0.0) += 1.0;
                }
            }
        }

        let mut model_s = Vec::with_capacity(pool.len());
        let mut pop_s = Vec::with_capacity(pool.len());
        let mut content_s = Vec::with_capacity(pool.len());
        for &i in &pool {
            let u = *unit_cache
                .entry(i)
                .or_insert_with(|| net.encode_feats_unit(&table.feats[i], table.scalars[i]).0);
            let mut cos = 0.0f32;
            for d in 0..D_MODEL {
                cos += ctx[d] * u[d];
            }
            model_s.push(cos);
            let pop = table.scalars[i][0] + 0.05 * table.scalars[i][1];
            pop_s.push(pop);
            let mut c = 0.0f32;
            for &(row, w) in &table.feats[i] {
                if let Some(&pw) = profile_rows.get(&row) {
                    c += pw * w;
                }
            }
            content_s.push(c + 0.01 * pop);
        }
        let mr = rank_of_first(&model_s);
        let pr = rank_of_first(&pop_s);
        let cr = rank_of_first(&content_s);
        m_rr += 1.0 / mr as f64;
        p_rr += 1.0 / pr as f64;
        c_rr += 1.0 / cr as f64;
        m_hit += (mr <= 10.0) as u8 as f64;
        c_hit += (cr <= 10.0) as u8 as f64;
        n += 1;
    }
    if n > 0 {
        let nf = n as f64;
        rep.n = n;
        rep.model_mrr = (m_rr / nf) as f32;
        rep.model_recall10 = (m_hit / nf) as f32;
        rep.popularity_mrr = (p_rr / nf) as f32;
        rep.content_mrr = (c_rr / nf) as f32;
        rep.content_recall10 = (c_hit / nf) as f32;
        rep.compute_lift();
    }
    rep
}

#[derive(Debug, Clone, Default)]
pub struct TrainEpochStats {
    pub steps_run: usize,
    pub total_steps: u64,
    pub avg_loss: f32,
    pub examples_used: usize,
    pub distilled_users: usize,
    /// True when nothing changed since the last run and the step budget was already spent.
    pub skipped: bool,
    /// True when the new weights scored worse on held-out listens and were discarded.
    pub rejected: bool,
    pub eval: EvalReport,
}

// ---------------------------------------------------------------------------
// Engine (inference + background training)
// ---------------------------------------------------------------------------

pub struct TransformerEngine {
    net: RwLock<TransformerNet>,
    user_vec_cache: RwLock<HashMap<i64, (i64, u64, [f32; D_MODEL])>>,
    unit_cache: RwLock<HashMap<u64, [f32; D_MODEL]>>,
    train_lock: std::sync::Mutex<()>,
}

fn track_cache_key(t: &Track, model_step: u64) -> u64 {
    let mut h = fnv1a_hash(t.id.as_bytes());
    for s in [&t.artist, &t.title, &t.album, &t.genre, &t.moods, &t.meta] {
        h = splitmix64(h ^ fnv1a_hash(s.as_bytes()));
    }
    splitmix64(h ^ (t.year as u64) ^ ((t.duration as u64) << 24) ^ model_step.wrapping_mul(0x9e3779b97f4a7c15))
}

impl TransformerEngine {
    pub fn load_or_new(store: &Store) -> Self {
        let net = match store.kv_get(CHECKPOINT_KV_KEY) {
            Some(raw) => TransformerNet::from_checkpoint_str(&raw).unwrap_or_else(|| {
                warn!("transformer checkpoint unreadable; starting fresh");
                TransformerNet::new()
            }),
            None => TransformerNet::new(),
        };
        // v1 weights learned a label-leakage shortcut; free the space and retrain from scratch.
        store.kv_delete(LEGACY_CHECKPOINT_KV_KEY);
        Self {
            net: RwLock::new(net),
            user_vec_cache: RwLock::new(HashMap::new()),
            unit_cache: RwLock::new(HashMap::new()),
            train_lock: std::sync::Mutex::new(()),
        }
    }

    pub fn step_count(&self) -> u64 {
        self.net.read().unwrap_or_else(|e| e.into_inner()).step_count
    }

    pub fn invalidate_user_cache(&self, user_id: i64) {
        if let Ok(mut cache) = self.user_vec_cache.write() {
            cache.remove(&user_id);
        }
    }

    pub fn status_snapshot(&self) -> serde_json::Value {
        let net = self.net.read().unwrap_or_else(|e| e.into_inner());
        let ev = &net.eval;
        let r3 = |x: f32| (x * 1000.0).round() / 1000.0;
        serde_json::json!({
            "architecture": "RoPE-QKVO + XSA + GEGLU Transformer",
            "paper": "arXiv:2603.09078",
            "dModel": D_MODEL,
            "layers": N_LAYERS,
            "heads": N_HEADS,
            "steps": net.step_count,
            "avgLoss": r3(net.avg_loss),
            "logitScale": r3(net.logit_scale),
            "usersTracked": net.user_embs.len(),
            "lastTrainedAt": net.last_trained_at,
            "stepsOnCurrentData": net.steps_on_fingerprint,
            "stepBudget": net.step_budget,
            "rejectedRuns": net.rejected_runs,
            "eval": {
                "heldOutListens": ev.n,
                "trusted": ev.trusted(),
                "modelMrr": r3(ev.model_mrr),
                "modelRecall10": r3(ev.model_recall10),
                "popularityMrr": r3(ev.popularity_mrr),
                "contentMrr": r3(ev.content_mrr),
                "contentRecall10": r3(ev.content_recall10),
                "lift": r3(ev.lift),
                "quality": r3(ev.quality),
                "evaluatedAt": ev.evaluated_at,
            },
            "blendMultiplier": {
                "newUser": (Self::multiplier_for(&net, 0) * 1000.0).round() / 1000.0,
                "establishedUser": (Self::multiplier_for(&net, 30) * 1000.0).round() / 1000.0,
            },
        })
    }

    fn track_units(&self, net: &TransformerNet, tracks: &[Track]) -> Vec<[f32; D_MODEL]> {
        let keys: Vec<u64> = tracks.iter().map(|t| track_cache_key(t, net.step_count)).collect();
        let mut out = vec![[0.0f32; D_MODEL]; tracks.len()];
        let mut missing = Vec::new();
        match self.unit_cache.read() {
            Ok(cache) => {
                for (i, k) in keys.iter().enumerate() {
                    match cache.get(k) {
                        Some(u) => out[i] = *u,
                        None => missing.push(i),
                    }
                }
            }
            Err(_) => missing.extend(0..tracks.len()),
        }
        if missing.is_empty() {
            return out;
        }
        let computed: Vec<(usize, [f32; D_MODEL])> =
            missing.iter().map(|&i| (i, net.encode_track_unit(&tracks[i]).0)).collect();
        if let Ok(mut cache) = self.unit_cache.write() {
            if cache.len() + computed.len() > 60_000 {
                cache.clear();
            }
            for &(i, u) in &computed {
                cache.insert(keys[i], u);
            }
        }
        for (i, u) in computed {
            out[i] = u;
        }
        out
    }

    /// Computes Transformer compatibility scores for `candidates` given:
    /// - `user_id`: differentiates per-user taste via `u_emb[user_id]` + attention over user history
    /// - `context_tracks`: optional active context (playlist tracks, seed track, recent radio items)
    /// - `query`: optional text/mood/genre intent (Instant Mix, Mood, Explore, Search)
    pub fn score_candidates(
        &self,
        store: &Store,
        user_id: i64,
        context_tracks: &[Track],
        query: Option<&str>,
        moods: &[Mood],
        genres: &[String],
        candidates: &[Track],
    ) -> Vec<f32> {
        if candidates.is_empty() {
            return Vec::new();
        }
        let net = self.net.read().unwrap_or_else(|e| e.into_inner());
        if net.step_count == 0 {
            return vec![0.0; candidates.len()];
        }
        let confidence = (net.step_count as f32 / 40.0).clamp(0.05, 1.0);
        let ctx_vec = self.compute_context_vector_locked(&net, store, user_id, context_tracks, query, moods, genres);
        self.track_units(&net, candidates)
            .iter()
            .map(|u| {
                let mut cos = 0.0f32;
                for d in 0..D_MODEL {
                    cos += ctx_vec[d] * u[d];
                }
                let s = cos * net.logit_scale * confidence;
                if s.is_finite() {
                    s
                } else {
                    0.0
                }
            })
            .collect()
    }

    fn multiplier_for(net: &TransformerNet, history_count: usize) -> f64 {
        if net.step_count == 0 {
            return 0.0;
        }
        let model_factor = if net.eval.trusted() {
            // Earned trust: 0.36 when it loses to the baselines, up to 1.35 when it clearly wins.
            0.25 + 1.10 * net.eval.quality as f64
        } else {
            // Not yet measurable (too few held-out listens): stay modest.
            0.35 + 0.30 * (net.step_count as f64 / 120.0).clamp(0.0, 1.0)
        };
        let user_factor = 0.75 + 0.45 * ((history_count as f64) / 30.0).clamp(0.0, 1.0);
        (model_factor * user_factor).clamp(0.05, 1.60)
    }

    /// Blend multiplier for the Transformer's scores, scaled by its measured held-out quality
    /// (versus popularity and content baselines) and by how much history the user has.
    pub fn adaptive_multiplier(&self, history_count: usize) -> f64 {
        let net = self.net.read().unwrap_or_else(|e| e.into_inner());
        Self::multiplier_for(&net, history_count)
    }

    /// Scores `(artist_name, genres)` pairs against the user's Transformer context vector.
    pub fn score_artist_candidates(&self, store: &Store, user_id: i64, artists: &[(String, String)]) -> Vec<f32> {
        if artists.is_empty() {
            return Vec::new();
        }
        let net = self.net.read().unwrap_or_else(|e| e.into_inner());
        if net.step_count == 0 {
            return vec![0.0; artists.len()];
        }
        let confidence = (net.step_count as f32 / 40.0).clamp(0.05, 1.0);
        let ctx_vec = self.compute_context_vector_locked(&net, store, user_id, &[], None, &[], &[]);
        let mut scores = Vec::with_capacity(artists.len());
        for (name, genres) in artists {
            let mut synth = Track::default();
            synth.artist = name.clone();
            synth.genre = genres.clone();
            let (t_unit, _, _, _, _) = net.encode_track_unit(&synth);
            let mut cos = 0.0f32;
            for d in 0..D_MODEL {
                cos += ctx_vec[d] * t_unit[d];
            }
            let s = cos * net.logit_scale * confidence;
            scores.push(if s.is_finite() { s } else { 0.0 });
        }
        scores
    }

    fn compute_context_vector_locked(
        &self,
        net: &TransformerNet,
        store: &Store,
        user_id: i64,
        context_tracks: &[Track],
        query: Option<&str>,
        moods: &[Mood],
        genres: &[String],
    ) -> [f32; D_MODEL] {
        let has_query = query.map_or(false, |q| !q.trim().is_empty()) || !moods.is_empty() || !genres.is_empty();
        let has_extra_ctx = !context_tracks.is_empty() || has_query;

        let now = Utc::now().timestamp();
        if !has_extra_ctx && user_id > 0 {
            if let Ok(cache) = self.user_vec_cache.read() {
                if let Some(&(ts, step, vec)) = cache.get(&user_id) {
                    if now - ts < 120 && step == net.step_count {
                        return vec;
                    }
                }
            }
        }

        let mut seq_items: Vec<SeqItem> = Vec::new();
        if user_id > 0 {
            let hist_cap = if has_extra_ctx { CONTEXT_HISTORY_CAP } else { MAX_SEQ_LEN - 1 };
            let raw = store.load_user_neural_history(user_id, hist_cap * 3 + 8).unwrap_or_default();
            let by_id: HashMap<&str, &Track> = raw.iter().map(|(t, _, _, _)| (t.id.as_str(), t)).collect();
            let listens = collapse_events(raw.iter().map(|(t, k, v, at)| (t.id.as_str(), k.as_str(), *v, *at)));
            let start = listens.len().saturating_sub(hist_cap);
            let mut hist_songs = HashSet::new();
            let history: Vec<SeqItem> = listens[start..]
                .iter()
                .filter_map(|l| {
                    by_id.get(l.track_id.as_str()).map(|t| {
                        hist_songs.insert(t.song_key());
                        let (f, s) = TransformerNet::extract_track_features(t);
                        (f, Some(s), l.action)
                    })
                })
                .collect();
            let stars: Vec<SeqItem> = if history.len() < PROFILE_SHORT_HISTORY {
                store
                    .starred_tracks(user_id)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|(t, _)| !hist_songs.contains(&t.song_key()))
                    .take(STAR_PREFIX_MAX)
                    .map(|(t, _)| {
                        let (f, s) = TransformerNet::extract_track_features(&t);
                        (f, Some(s), ACTION_STAR)
                    })
                    .collect()
            } else {
                Vec::new()
            };
            seq_items = assemble_profile_sequence(history, stars, hist_cap);
        }

        for t in context_tracks.iter().take(MAX_SEED_TOKENS) {
            let (f, s) = TransformerNet::extract_track_features(t);
            seq_items.push((f, Some(s), ACTION_SEED));
        }

        if has_query {
            let qf = TransformerNet::extract_query_features(query.unwrap_or(""), moods, genres);
            if !qf.is_empty() {
                seq_items.push((qf, None, ACTION_QUERY));
            }
        }

        let out = net.forward_context(user_id.max(0), &seq_items).ctx_unit;
        if !has_extra_ctx && user_id > 0 {
            if let Ok(mut cache) = self.user_vec_cache.write() {
                cache.insert(user_id, (now, net.step_count, out));
            }
        }
        out
    }

    /// Background training across all users' listens, ratings, skips/dislikes and manual
    /// playlists. Only trains when the underlying data has changed (bounded step budget per
    /// data state, so the model doesn't overfit the same examples forever), evaluates on
    /// held-out listens before and after, keeps the new weights only if they did not get worse,
    /// then distills personalized `neural_track` / `neural_artist` links and checkpoints.
    pub fn train_background_epoch(
        &self,
        store: &Store,
        max_steps: usize,
        batch_size: usize,
        distill_users: bool,
    ) -> TrainEpochStats {
        let _train_guard = self.train_lock.lock().unwrap_or_else(|e| e.into_inner());
        let fingerprint = store.transformer_data_fingerprint().unwrap_or_default();
        {
            let net = self.net.read().unwrap_or_else(|e| e.into_inner());
            if net.step_count > 0
                && !fingerprint.is_empty()
                && net.data_fingerprint == fingerprint
                && net.steps_on_fingerprint >= net.step_budget
            {
                return TrainEpochStats {
                    total_steps: net.step_count,
                    avg_loss: net.avg_loss,
                    skipped: true,
                    eval: net.eval.clone(),
                    ..Default::default()
                };
            }
        }

        let corpus = match store.load_transformer_corpus(CORPUS_EVENTS_PER_USER, CORPUS_CATALOG_LIMIT) {
            Ok(c) => c,
            Err(_) => return TrainEpochStats::default(),
        };
        let table = CorpusTable::build(&corpus);
        let users = prepare_users(&corpus, &table);

        let mut net = self.net.read().unwrap_or_else(|e| e.into_inner()).clone();
        let data_seed = 0x54524149u64 ^ fnv1a_hash(fingerprint.as_bytes()) ^ net.step_count.wrapping_mul(0x9e3779b97f4a7c15);
        let data = build_training_data(&corpus, &table, &users, data_seed);
        let bs = batch_size.clamp(4, 32);

        if net.data_fingerprint != fingerprint || fingerprint.is_empty() {
            net.data_fingerprint = fingerprint.clone();
            net.steps_on_fingerprint = 0;
            // ~4 passes over the examples per data state, bounded.
            net.step_budget = ((data.examples.len() * 4 + bs - 1) / bs).clamp(24, 600) as u64;
        }
        if data.examples.is_empty() {
            net.step_budget = 0;
            *self.net.write().unwrap_or_else(|e| e.into_inner()) = net;
            return TrainEpochStats::default();
        }

        let remaining = net.step_budget.saturating_sub(net.steps_on_fingerprint) as usize;
        let steps_target = max_steps.min(remaining);
        // Fixed eval seed per data state so before/after numbers are comparable.
        let eval_seed = fnv1a_hash(fingerprint.as_bytes()) ^ 0x4556414c;
        let can_compare = net.step_count > 0 && data.holdouts.len() >= MIN_EVAL_ITEMS;
        let before = can_compare.then(|| evaluate(&net, &table, &users, &data.holdouts, eval_seed));
        let prev_net = can_compare.then(|| net.clone());

        let mut rng = StdRng::seed_from_u64(data_seed ^ 0x5348554646);
        let mut order: Vec<usize> = (0..data.examples.len()).collect();
        let mut steps_run = 0usize;
        'outer: while steps_run < steps_target {
            order.shuffle(&mut rng);
            for chunk in order.chunks(bs) {
                if steps_run >= steps_target {
                    break 'outer;
                }
                let batch: Vec<&TrainingExample> = chunk.iter().map(|&i| &data.examples[i]).collect();
                let lr = net.learning_rate();
                net.train_step(&batch, &table, &users, lr);
                steps_run += 1;
            }
        }
        net.steps_on_fingerprint += steps_run as u64;

        let after = evaluate(&net, &table, &users, &data.holdouts, eval_seed);
        let rejected = match (&before, &prev_net) {
            (Some(b), Some(_)) => after.trusted() && after.model_mrr + 0.015 < b.model_mrr,
            _ => false,
        };
        if rejected {
            let mut restored = prev_net.unwrap();
            restored.data_fingerprint = net.data_fingerprint.clone();
            restored.steps_on_fingerprint = net.steps_on_fingerprint;
            restored.step_budget = net.step_budget;
            restored.rejected_runs = net.rejected_runs + 1;
            let prev_eval = restored.eval.clone();
            restored.eval = before.clone().unwrap().with_quality_from(&prev_eval);
            info!(
                before_mrr = restored.eval.model_mrr,
                after_mrr = after.model_mrr,
                "transformer update made held-out ranking worse; keeping previous weights"
            );
            net = restored;
        } else {
            let prev_eval = net.eval.clone();
            net.eval = after.with_quality_from(&prev_eval);
        }

        let checkpoint = net.to_checkpoint_str();
        let distilled_users = if distill_users && steps_run > 0 && !rejected {
            distill_corpus_users(&net, store, &corpus, &table, &users)
        } else {
            0
        };

        let stats = TrainEpochStats {
            steps_run,
            total_steps: net.step_count,
            avg_loss: net.avg_loss,
            examples_used: data.examples.len(),
            distilled_users,
            skipped: false,
            rejected,
            eval: net.eval.clone(),
        };

        *self.net.write().unwrap_or_else(|e| e.into_inner()) = net;
        if let Ok(mut cache) = self.user_vec_cache.write() {
            cache.clear();
        }
        if let Ok(mut cache) = self.unit_cache.write() {
            cache.clear();
        }
        if let Some(cp) = checkpoint {
            store.kv_set(CHECKPOINT_KV_KEY, &cp);
        }
        stats
    }
}

fn distill_corpus_users(
    net: &TransformerNet,
    store: &Store,
    corpus: &NeuralTrainingCorpus,
    table: &CorpusTable,
    users: &HashMap<i64, UserPrep>,
) -> usize {
    if net.step_count < 10 || corpus.catalog_ids.is_empty() {
        return 0;
    }
    let catalog: Vec<(usize, [f32; D_MODEL])> = corpus
        .catalog_ids
        .iter()
        .filter_map(|id| table.idx(id))
        .map(|i| (i, net.encode_feats_unit(&table.feats[i], table.scalars[i]).0))
        .collect();
    if catalog.is_empty() {
        return 0;
    }
    // Distilled links feed other taste signals, so scale them by how much the model has earned.
    let trust = if net.eval.trusted() { 0.4 + 0.6 * net.eval.quality as f64 } else { 0.7 };

    let mut uids: Vec<i64> = users.keys().copied().collect();
    uids.sort();
    let mut distilled = 0usize;
    for uid in uids {
        let p = &users[&uid];
        let seq = profile_sequence(p, table, p.listens.len(), None, MAX_SEQ_LEN - 1);
        if seq.len() < 2 {
            continue;
        }
        let ctx = net.forward_context(uid, &seq).ctx_unit;
        let disliked: HashSet<u64> = p.disliked.iter().map(|&i| table.song[i]).collect();

        let mut scored: Vec<(f64, usize)> = Vec::new();
        for (i, u) in &catalog {
            if disliked.contains(&table.song[*i]) {
                continue;
            }
            let mut cos = 0.0f32;
            for d in 0..D_MODEL {
                cos += ctx[d] * u[d];
            }
            if cos > 0.15 {
                scored.push((((cos as f64) * 3.0).clamp(0.1, 3.5) * trust, *i));
            }
        }
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        let mut track_links = Vec::with_capacity(60);
        let mut artist_scores: HashMap<String, f64> = HashMap::new();
        for (w, i) in scored.into_iter().take(60) {
            track_links.push((table.ids[i].clone(), w));
            if let Some(t) = corpus.tracks.get(&table.ids[i]) {
                let ak = slug(&primary_artist(&t.artist));
                if !ak.is_empty() && ak != "unknown-artist" {
                    artist_scores
                        .entry(ak)
                        .and_modify(|cur| *cur = (*cur).max(w * 0.85))
                        .or_insert(w * 0.85);
                }
            }
        }
        let mut artist_links: Vec<(String, f64)> = artist_scores.into_iter().collect();
        artist_links.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        artist_links.truncate(48);

        if !track_links.is_empty() {
            let _ = store.replace_neural_taste_links(uid, &track_links, &artist_links);
            distilled += 1;
        }
    }
    distilled
}

#[inline]
fn mood_to_idx(m: Mood) -> usize {
    match m {
        Mood::Chill => 0,
        Mood::Energy => 1,
        Mood::LateNight => 2,
        Mood::Upbeat => 3,
        Mood::Melancholy => 4,
        Mood::Party => 5,
        Mood::Acoustic => 6,
        Mood::Groove => 7,
        Mood::RockIndie => 8,
    }
}

fn parse_meta_rank_norm(meta: &str) -> f32 {
    for part in meta.split(';') {
        if let Some(rest) = part.trim().strip_prefix("rank=") {
            if let Ok(r) = rest.trim().parse::<f32>() {
                return (r.max(0.0).ln_1p() / 14.0).clamp(0.0, 1.0);
            }
        }
    }
    0.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    fn mk_track(id: &str, title: &str, artist: &str, genre: &str, moods: &str, year: i32) -> Track {
        Track {
            id: id.to_string(),
            source: "deezer".to_string(),
            title: title.to_string(),
            artist: artist.to_string(),
            album: format!("{} Album", title),
            genre: genre.to_string(),
            moods: moods.to_string(),
            year,
            track_no: 1,
            disc_no: 1,
            duration: 210,
            bitrate: 320,
            sample_rate: 44100,
            bit_depth: 16,
            format: "mp3".to_string(),
            size: 0,
            stream_url: String::new(),
            cover_url: String::new(),
            meta: "rank=500000".to_string(),
            deezer_id: String::new(),
            created: 1000,
        }
    }

    #[test]
    fn rope_qkvo_and_xsa_orthogonality_hold() {
        let blk = TransformerBlock::new(0);
        let seq_len = 5;
        let x: Vec<f32> = (0..seq_len * D_MODEL)
            .map(|i| deterministic_init(i as u64 + 7, 0.5))
            .collect();
        let (_out, cache) = blk.forward(&x, seq_len);

        // XSA (arXiv:2603.09078): every head's output z_xsa must be orthogonal to self-value v_raw.
        for t in 0..seq_len {
            for h in 0..N_HEADS {
                let off = t * D_MODEL + h * D_HEAD;
                let z = &cache.z_xsa[off..off + D_HEAD];
                let v = &cache.v_raw[off..off + D_HEAD];
                let mut dot = 0.0f32;
                for d in 0..D_HEAD {
                    dot += z[d] * v[d];
                }
                assert!(dot.abs() < 1e-4, "XSA output not orthogonal to self-value at t={}, h={}: dot={}", t, h, dot);
            }
        }
    }

    #[test]
    fn rope_qkvo_xsa_transformer_gradients_match_finite_differences() {
        let mut net = TransformerNet::new();
        let t1 = mk_track("t1", "Midnight City", "M83", "Synthwave, Electronic", "late_night", 2011);
        let t2 = mk_track("t2", "Nightcall", "Kavinsky", "Synthwave", "late_night", 2010);
        let pos = mk_track("t3", "Resonance", "Home", "Synthwave", "chill,late_night", 2014);
        let neg1 = mk_track("n1", "Heavy Metal Anthem", "Iron Thunder", "Metal", "energy", 1995);
        let neg2 = mk_track("n2", "Acoustic Barn", "Folk Band", "Country", "acoustic", 2005);

        let f1 = TransformerNet::extract_track_features(&t1);
        let f2 = TransformerNet::extract_track_features(&t2);
        let user_id = 42i64;
        let context: Vec<SeqItem> = vec![(f1.0, Some(f1.1), ACTION_STAR), (f2.0, Some(f2.1), ACTION_COMPLETE)];
        let cand_tracks = [pos, neg1, neg2];

        let eval_loss = |n: &TransformerNet| -> f32 {
            let ctx = n.forward_context(user_id, &context);
            let logits: Vec<f32> = cand_tracks
                .iter()
                .map(|t| {
                    let c = n.encode_track_unit(t).0;
                    let mut cos = 0.0f32;
                    for d in 0..D_MODEL {
                        cos += ctx.ctx_unit[d] * c[d];
                    }
                    cos * n.logit_scale
                })
                .collect();
            let max_l = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            let sum_e: f32 = logits.iter().map(|l| (l - max_l).exp()).sum();
            -(logits[0] - max_l - sum_e.ln())
        };

        let ctx_cache = net.forward_context(user_id, &context);
        let cands: Vec<[f32; D_MODEL]> = cand_tracks.iter().map(|t| net.encode_track_unit(t).0).collect();
        let logits: Vec<f32> = cands
            .iter()
            .map(|c| {
                let mut cos = 0.0f32;
                for d in 0..D_MODEL {
                    cos += ctx_cache.ctx_unit[d] * c[d];
                }
                cos * net.logit_scale
            })
            .collect();
        let max_l = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let exps: Vec<f32> = logits.iter().map(|l| (l - max_l).exp()).collect();
        let sum_e: f32 = exps.iter().sum();
        let probs: Vec<f32> = exps.iter().map(|e| e / sum_e).collect();

        let mut g_ctx_unit = [0.0f32; D_MODEL];
        for c_idx in 0..3 {
            let target = if c_idx == 0 { 1.0 } else { 0.0 };
            let g_cos = (probs[c_idx] - target) * net.logit_scale;
            for d in 0..D_MODEL {
                g_ctx_unit[d] += g_cos * cands[c_idx][d];
            }
        }
        let g_ctx_raw = l2_normalize_backward(&g_ctx_unit, &ctx_cache.ctx_unit, ctx_cache.ctx_norm);
        let mut g_ctx_w = vec![0.0f32; D_MODEL * D_MODEL];
        let mut g_h_norm = [0.0f32; D_MODEL];
        matmul_backward(&g_ctx_raw, &ctx_cache.h_norm, &net.ctx_proj_w.w, D_MODEL, D_MODEL, &mut g_h_norm, &mut g_ctx_w);

        let final_rms_g: [f32; D_MODEL] = net.final_rms_g.w[..D_MODEL].try_into().unwrap();
        let mut g_h = vec![0.0f32; ctx_cache.seq_len * D_MODEL];
        let u_pos_idx = ctx_cache.seq_len - 1;
        let mut g_fr = [0.0f32; D_MODEL];
        rmsnorm_backward(&g_h_norm, &ctx_cache.h_last, ctx_cache.final_rms, &final_rms_g, &mut g_h[u_pos_idx * D_MODEL..], &mut g_fr);

        let mut bgs = vec![BlockGrad::new(), BlockGrad::new()];
        for l in (0..N_LAYERS).rev() {
            g_h = net.blocks[l].backward(&g_h, &ctx_cache.block_caches[l], ctx_cache.seq_len, &mut bgs[l]);
        }

        let _ = net.get_or_insert_user_row(42);
        let eps = 1e-3f32;
        let mut check = |get: &dyn Fn(&mut TransformerNet) -> &mut f32, an: f32, label: &str| {
            let orig = *get(&mut net);
            *get(&mut net) = orig + eps;
            let lp = eval_loss(&net);
            *get(&mut net) = orig - eps;
            let lm = eval_loss(&net);
            *get(&mut net) = orig;
            let fd = (lp - lm) / (2.0 * eps);
            assert!((fd - an).abs() < 2e-2, "{} grad mismatch: fd={} analytical={}", label, fd, an);
        };
        for idx in [0usize, 17, 131] {
            check(&|n| &mut n.blocks[0].wv.w[idx], bgs[0].wv[idx], "Wv");
        }
        for idx in [3usize, 49] {
            check(&|n| &mut n.blocks[0].wq.w[idx], bgs[0].wq[idx], "Wq");
        }
        for idx in [5usize, 77] {
            check(&|n| &mut n.blocks[0].w_gate.w[idx], bgs[0].w_gate[idx], "W_gate");
        }
        for d in [0usize, 11, 35] {
            let an = g_h[u_pos_idx * D_MODEL + d];
            check(&|n| &mut n.user_embs.get_mut(&42).unwrap().w[d], an, "UserEmb");
        }
    }

    #[test]
    fn collapse_merges_play_with_its_outcome_and_separates_replays() {
        let evs = [
            ("a", "play", 1.0, 100),
            ("a", "complete", 1.0, 300),
            ("b", "play", 1.0, 310),
            ("b", "skip", 1.0, 320),
            ("a", "play", 1.0, 330),     // replay of a: new listen
            ("a", "complete", 1.0, 500),
            ("a", "star", 1.0, 510),     // starred during/after that listen: upgrade
            ("c", "search_pick", 1.0, 600),
            ("c", "play", 1.0, 601),     // duplicate start ping merges
            ("d", "rate", 2.0, 700),     // low rating = rejection
            ("e", "rate", 3.0, 710),     // neutral rating ignored
            ("f", "play", 1.0, 800),     // unresolved
        ];
        let ls = collapse_events(evs.iter().map(|(t, k, v, at)| (*t, *k, *v, *at)));
        let got: Vec<(&str, Outcome)> = ls.iter().map(|l| (l.track_id.as_str(), l.outcome)).collect();
        assert_eq!(
            got,
            vec![
                ("a", Outcome::Completed),
                ("b", Outcome::Skipped),
                ("a", Outcome::Saved),
                ("c", Outcome::Picked),
                ("d", Outcome::Rejected),
                ("f", Outcome::Started),
            ]
        );
        assert_eq!(ls[1].action, ACTION_SKIP);
        assert_eq!(ls[2].action, ACTION_STAR);
    }

    fn genre_tracks() -> Vec<Track> {
        let mut out = Vec::new();
        let genres = [
            ("rock", "Rock, Alternative", "rock_indie,energy"),
            ("pop", "Synthpop, Dance Pop", "upbeat,party"),
            ("jazz", "Jazz, Bebop", "groove,chill"),
            ("metal", "Metal, Thrash", "energy"),
        ];
        for (g, genre, moods) in genres {
            for i in 0..12 {
                let mut t = mk_track(
                    &format!("{}{}", g, i),
                    &format!("{} Song {}", g, i),
                    &format!("{} Artist {}", g, i % 4),
                    genre,
                    moods,
                    1990 + i,
                );
                t.meta = format!("rank={}", 100_000 + (i as i64 * 37_000) % 400_000);
                out.push(t);
            }
        }
        out
    }

    fn listen(store: &Store, uid: i64, tid: &str, done: bool) {
        store.record_event(uid, tid, "play", 1.0).unwrap();
        store.record_event(uid, tid, if done { "complete" } else { "skip" }, 1.0).unwrap();
    }

    #[test]
    fn training_never_uses_the_just_played_track_or_skips_as_positives() {
        let store = Store::open_memory().unwrap();
        let u = store.create_user("listener", "pw123").unwrap();
        let tracks = genre_tracks();
        store.upsert_tracks(&tracks).unwrap();
        for i in 0..10 {
            listen(&store, u.id, &format!("rock{}", i), true);
            listen(&store, u.id, &format!("pop{}", i), false);
        }
        let corpus = store.load_transformer_corpus(CORPUS_EVENTS_PER_USER, CORPUS_CATALOG_LIMIT).unwrap();
        let table = CorpusTable::build(&corpus);
        let users = prepare_users(&corpus, &table);
        let p = &users[&u.id];
        assert_eq!(p.listens.len(), 20, "each play+outcome pair must collapse into one listen");

        let data = build_training_data(&corpus, &table, &users, 7);
        assert!(!data.examples.is_empty());
        for ex in &data.examples {
            let id = &table.ids[ex.positive];
            assert!(!id.starts_with("pop"), "skipped track {} used as a positive", id);
            if let Some((last_feats, _, act)) = ex.context.last() {
                if *act != ACTION_QUERY {
                    assert!(
                        *last_feats != table.feats[ex.positive],
                        "positive {} is the last context token (label leakage)",
                        id
                    );
                }
            }
        }
        assert_eq!(data.holdouts.len(), 1);
        assert!(table.ids[data.holdouts[0].target].starts_with("rock"));
    }

    #[test]
    fn transformer_learns_cross_user_patterns_and_differentiates_users() {
        let store = Store::open_memory().unwrap();
        let u_rock = store.create_user("rockfan", "pw123").unwrap();
        let u_pop = store.create_user("popfan", "pw123").unwrap();
        let u_curator = store.create_user("curator", "pw123").unwrap();

        let tracks = vec![
            mk_track("r1", "Everlong", "Foo Fighters", "Rock, Grunge", "rock_indie,energy", 1997),
            mk_track("r2", "Smells Like Teen Spirit", "Nirvana", "Grunge, Alternative", "rock_indie,energy", 1991),
            mk_track("r3", "When You Sleep", "My Bloody Valentine", "Shoegaze, Alternative", "rock_indie", 1991),
            mk_track("r4", "Alison", "Slowdive", "Shoegaze, Dream Pop", "chill,rock_indie", 1993),
            mk_track("p1", "Blinding Lights", "The Weeknd", "Synthpop, Pop", "upbeat,party", 2019),
            mk_track("p2", "Levitating", "Dua Lipa", "Dance Pop, Disco", "upbeat,party", 2020),
            mk_track("p3", "Run Away With Me", "Carly Rae Jepsen", "Synthpop, Dance Pop", "upbeat", 2015),
            mk_track("p4", "Midnight City", "M83", "Synthpop, Electronic", "upbeat,late_night", 2011),
            mk_track("j1", "So What", "Miles Davis", "Jazz", "groove,chill", 1959),
            mk_track("j2", "Take Five", "Dave Brubeck", "Jazz", "groove,focus", 1959),
            mk_track("c1", "Clair de Lune", "Claude Debussy", "Classical, Piano", "focus", 1905),
            mk_track("c2", "Gymnopedie No 1", "Erik Satie", "Classical, Ambient", "focus,chill", 1888),
        ];
        store.upsert_tracks(&tracks).unwrap();

        store.star_track(u_rock.id, "r1").unwrap();
        store.star_track(u_rock.id, "r2").unwrap();
        listen(&store, u_rock.id, "r1", true);
        listen(&store, u_rock.id, "r2", true);
        listen(&store, u_rock.id, "p1", false);
        listen(&store, u_rock.id, "p2", false);
        store.set_rating(u_rock.id, "p1", 1).unwrap();

        store.star_track(u_pop.id, "p1").unwrap();
        store.star_track(u_pop.id, "p2").unwrap();
        store.star_track(u_pop.id, "p4").unwrap();
        listen(&store, u_pop.id, "p1", true);
        listen(&store, u_pop.id, "p2", true);
        listen(&store, u_pop.id, "p4", true);
        listen(&store, u_pop.id, "r1", false);
        listen(&store, u_pop.id, "r2", false);
        store.set_rating(u_pop.id, "r1", 1).unwrap();

        // Another user curates a playlist bridging Nirvana (r2) with unseen shoegaze (r3, r4).
        store
            .create_playlist(u_curator.id, "90s Alt & Shoegaze", "", false, &["r2".into(), "r3".into(), "r4".into()])
            .unwrap();

        let engine = TransformerEngine::load_or_new(&store);
        let stats = engine.train_background_epoch(&store, 60, 6, true);
        assert!(stats.steps_run >= 10, "steps_run={}", stats.steps_run);
        assert!(stats.distilled_users >= 2);

        let test_cands = vec![tracks[2].clone(), tracks[6].clone()]; // r3, p3
        let rock_scores = engine.score_candidates(&store, u_rock.id, &[], None, &[], &[], &test_cands);
        let pop_scores = engine.score_candidates(&store, u_pop.id, &[], None, &[], &[], &test_cands);
        assert!(rock_scores[0] > rock_scores[1], "rock user: r3={} p3={}", rock_scores[0], rock_scores[1]);
        assert!(pop_scores[1] > pop_scores[0], "pop user: p3={} r3={}", pop_scores[1], pop_scores[0]);

        // Cached scoring path returns identical results.
        let again = engine.score_candidates(&store, u_rock.id, &[], None, &[], &[], &test_cands);
        assert_eq!(rock_scores, again);

        // Checkpoint roundtrip preserves exact scoring and training state.
        let reloaded = TransformerEngine::load_or_new(&store);
        assert_eq!(reloaded.step_count(), engine.step_count());
        let rock_scores_reloaded = reloaded.score_candidates(&store, u_rock.id, &[], None, &[], &[], &test_cands);
        assert!((rock_scores[0] - rock_scores_reloaded[0]).abs() < 1e-5);

        let low_hist = engine.adaptive_multiplier(0);
        let high_hist = engine.adaptive_multiplier(40);
        assert!(low_hist > 0.0 && high_hist > low_hist);

        let neural_artists = store.load_neural_artist_links(u_rock.id, 10).unwrap();
        assert!(!neural_artists.is_empty());

        let art_scores = engine.score_artist_candidates(
            &store,
            u_rock.id,
            &[
                ("Slowdive".to_string(), "Shoegaze, Rock".to_string()),
                ("Dua Lipa".to_string(), "Dance Pop, Disco".to_string()),
            ],
        );
        assert!(art_scores[0] > art_scores[1], "Slowdive={} Dua Lipa={}", art_scores[0], art_scores[1]);
    }

    #[test]
    fn training_is_gated_on_data_changes_and_evaluated_against_baselines() {
        let store = Store::open_memory().unwrap();
        let tracks = genre_tracks();
        store.upsert_tracks(&tracks).unwrap();
        let prefs = ["rock", "pop", "jazz", "metal", "rock", "jazz", "pop", "metal"];
        let mut ids = Vec::new();
        for (k, g) in prefs.iter().enumerate() {
            let u = store.create_user(&format!("user{}", k), "pw123").unwrap();
            ids.push(u.id);
            for i in 0..9 {
                let n = (i * 5 + k * 3) % 12;
                listen(&store, u.id, &format!("{}{}", g, n), true);
                if i % 3 == 0 {
                    let other = prefs[(k + 1) % prefs.len()];
                    listen(&store, u.id, &format!("{}{}", other, n), false);
                }
            }
        }

        let engine = TransformerEngine::load_or_new(&store);
        let mut total = 0usize;
        let mut last = TrainEpochStats::default();
        for _ in 0..40 {
            last = engine.train_background_epoch(&store, 50, 16, false);
            if last.skipped {
                break;
            }
            total += last.steps_run;
        }
        assert!(last.skipped, "training must stop once the step budget for unchanged data is spent");
        assert!(total > 0);

        let ev = &last.eval;
        assert!(ev.n >= MIN_EVAL_ITEMS, "expected held-out listens, got {}", ev.n);
        assert!(
            ev.model_mrr > ev.popularity_mrr && ev.model_mrr >= ev.content_mrr,
            "model should beat both baselines on genre-separable users: {:?}",
            ev
        );
        assert!(ev.quality > 0.5 && engine.adaptive_multiplier(30) > 1.0, "{:?}", ev);

        // New data re-opens training.
        listen(&store, ids[0], "rock11", true);
        let resumed = engine.train_background_epoch(&store, 10, 16, false);
        assert!(!resumed.skipped && resumed.steps_run > 0);

        let snap = engine.status_snapshot();
        assert!(snap["eval"]["heldOutListens"].as_u64().unwrap() >= MIN_EVAL_ITEMS as u64);
    }

    #[test]
    fn checkpoint_roundtrip_and_shape_validation() {
        let net = TransformerNet::new();
        let cp = net.to_checkpoint_str().unwrap();
        let back = TransformerNet::from_checkpoint_str(&cp).expect("roundtrip");
        assert_eq!(back.feature_weights, net.feature_weights);
        assert!(TransformerNet::from_checkpoint_str("not a checkpoint").is_none());
    }

    #[test]
    fn extract_features_captures_subgenres_collabs_and_inferred_moods() {
        let store = Store::open_memory().unwrap();
        let engine = TransformerEngine::load_or_new(&store);
        assert_eq!(engine.adaptive_multiplier(10), 0.0);

        let collab = mk_track("x1", "Midnight Acoustic Ballad", "Daft Punk feat. Julian Casablancas", "Indie Rock", "", 2013);
        let (feats, _) = TransformerNet::extract_track_features(&collab);
        let count = |lo: usize, hi: usize| feats.iter().filter(|(idx, _)| *idx >= lo && *idx < hi).count();
        assert!(count(ARTIST_OFFSET, TEXT_OFFSET) >= 2, "expected primary + featured artist buckets");
        assert!(count(GENRE_OFFSET, MOOD_OFFSET) >= 3, "expected full + sub-genre buckets");
        assert!(count(MOOD_OFFSET, ERA_OFFSET) >= 1, "expected inferred mood buckets when moods is empty");
    }
}
