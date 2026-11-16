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
const CHECKPOINT_KV_KEY: &str = "transformer:checkpoint_v1";

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

#[derive(Clone, Serialize, Deserialize)]
struct AdamArray {
    w: Vec<f32>,
    m: Vec<f32>,
    v: Vec<f32>,
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
    w: Vec<f32>,
    m: Vec<f32>,
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

#[derive(Clone, Serialize, Deserialize)]
pub struct TransformerNet {
    feature_weights: Vec<f32>,
    feature_m: Vec<f32>,
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
        }
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
                (&["focus", "instrumental", "classical", "study", "piano"], Mood::Focus),
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

    pub fn encode_track_unit(&self, t: &Track) -> ([f32; D_MODEL], Vec<(usize, f32)>, [f32; 4], [f32; D_MODEL], f32) {
        let (feats, scalars) = Self::extract_track_features(t);
        let raw = self.sum_sparse_features(&feats, Some(scalars));
        let mut proj = [0.0f32; D_MODEL];
        matmul_vec_add(&raw, &self.track_proj_w.w, D_MODEL, D_MODEL, &mut proj);
        let (unit, norm) = l2_normalize(&proj);
        (unit, feats, scalars, raw, norm)
    }

    /// Runs the 2-layer RoPE-QKVO + XSA Transformer over a sequence of items ending in `[USER_u]`.
    fn forward_context(
        &self,
        user_id: i64,
        seq_items: &[(Vec<(usize, f32)>, Option<[f32; 4]>, usize)],
    ) -> ContextForwardCache {
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

    fn train_step_examples(&mut self, examples: &[TrainingExample], lr: f32) -> f32 {
        if examples.is_empty() {
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

        let inv_batch = 1.0 / (examples.len() as f32);
        let mut total_loss = 0.0f32;

        for ex in examples {
            let ctx_cache = self.forward_context(ex.user_id, &ex.context);
            let n_cands = 1 + ex.negatives.len();
            let mut cand_units = Vec::with_capacity(n_cands);
            let mut cand_feats = Vec::with_capacity(n_cands);
            let mut cand_scalars = Vec::with_capacity(n_cands);
            let mut cand_raws = Vec::with_capacity(n_cands);
            let mut cand_norms = Vec::with_capacity(n_cands);

            let (u_pos, f_pos, s_pos, r_pos, n_pos) = self.encode_track_unit(&ex.positive);
            cand_units.push(u_pos);
            cand_feats.push(f_pos);
            cand_scalars.push(s_pos);
            cand_raws.push(r_pos);
            cand_norms.push(n_pos);

            for neg in &ex.negatives {
                let (u_n, f_n, s_n, r_n, n_n) = self.encode_track_unit(neg);
                cand_units.push(u_n);
                cand_feats.push(f_n);
                cand_scalars.push(s_n);
                cand_raws.push(r_n);
                cand_norms.push(n_n);
            }

            let mut logits = Vec::with_capacity(n_cands);
            let mut cosines = Vec::with_capacity(n_cands);
            let mut max_l = f32::NEG_INFINITY;
            for cu in &cand_units {
                let mut cos = 0.0f32;
                for d in 0..D_MODEL {
                    cos += ctx_cache.ctx_unit[d] * cu[d];
                }
                cosines.push(cos);
                let l = cos * self.logit_scale;
                logits.push(l);
                if l > max_l {
                    max_l = l;
                }
            }

            let mut sum_exp = 0.0f32;
            let mut probs = Vec::with_capacity(n_cands);
            for &l in &logits {
                let e = (l - max_l).exp();
                probs.push(e);
                sum_exp += e;
            }
            for p in &mut probs {
                *p /= sum_exp.max(1e-12);
            }
            let ce_loss = -(probs[0].max(1e-12)).ln();
            let ex_w = ex.weight * inv_batch;
            total_loss += ce_loss * ex_w;

            // Gradient of InfoNCE wrt ctx_unit and each candidate unit
            let mut g_ctx_unit = [0.0f32; D_MODEL];
            for c_idx in 0..n_cands {
                let target = if c_idx == 0 { 1.0f32 } else { 0.0f32 };
                let g_logit = (probs[c_idx] - target) * ex_w;
                g_logit_scale += g_logit * cosines[c_idx];

                let g_cos = g_logit * self.logit_scale;
                for d in 0..D_MODEL {
                    g_ctx_unit[d] += g_cos * cand_units[c_idx][d];
                }

                let mut g_cand_unit = [0.0f32; D_MODEL];
                for d in 0..D_MODEL {
                    g_cand_unit[d] = g_cos * ctx_cache.ctx_unit[d];
                }
                let g_cand_proj = l2_normalize_backward(&g_cand_unit, &cand_units[c_idx], cand_norms[c_idx]);
                let mut g_cand_raw = [0.0f32; D_MODEL];
                matmul_backward(
                    &g_cand_proj,
                    &cand_raws[c_idx],
                    &self.track_proj_w.w,
                    D_MODEL,
                    D_MODEL,
                    &mut g_cand_raw,
                    &mut g_trk_w,
                );
                accumulate_sparse_feature_grad(
                    &cand_feats[c_idx],
                    Some(cand_scalars[c_idx]),
                    &g_cand_raw,
                    &mut g_feats,
                    &mut g_scalar,
                );
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
                g_h = self.blocks[l].backward(
                    &g_h,
                    &ctx_cache.block_caches[l],
                    ctx_cache.seq_len,
                    &mut block_grads[l],
                );
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

        // Global gradient norm clipping (max_norm = 1.0)
        let mut sq_norm = 0.0f32;
        for bg in &block_grads { sq_norm += bg.sq_norm(); }
        for &v in &g_final_rms_g { sq_norm += v * v; }
        for &v in &g_ctx_w { sq_norm += v * v; }
        for &v in &g_trk_w { sq_norm += v * v; }
        for &v in &g_action { sq_norm += v * v; }
        for &v in &g_scalar { sq_norm += v * v; }
        sq_norm += g_logit_scale * g_logit_scale;
        for g in g_feats.values() {
            for &v in g { sq_norm += v * v; }
        }
        for g in g_users.values() {
            for &v in g { sq_norm += v * v; }
        }

        let grad_norm = sq_norm.sqrt();
        if grad_norm > 1.0 {
            let clip = 1.0 / grad_norm;
            for bg in &mut block_grads { bg.scale(clip); }
            for v in &mut g_final_rms_g { *v *= clip; }
            for v in &mut g_ctx_w { *v *= clip; }
            for v in &mut g_trk_w { *v *= clip; }
            for v in &mut g_action { *v *= clip; }
            for v in &mut g_scalar { *v *= clip; }
            g_logit_scale *= clip;
            for g in g_feats.values_mut() {
                for v in g.iter_mut() { *v *= clip; }
            }
            for g in g_users.values_mut() {
                for v in g.iter_mut() { *v *= clip; }
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
        self.logit_scale = (self.logit_scale - lr * (mh / (vh.sqrt() + eps))).clamp(2.0, 18.0);

        if self.step_count == 1 {
            self.avg_loss = total_loss;
        } else {
            self.avg_loss = 0.9 * self.avg_loss + 0.1 * total_loss;
        }
        self.last_trained_at = Utc::now().timestamp();
        total_loss
    }

    fn to_checkpoint_str(&self) -> Option<String> {
        let json = serde_json::to_vec(self).ok()?;
        let mut enc = GzEncoder::new(Vec::new(), Compression::fast());
        enc.write_all(&json).ok()?;
        let gz = enc.finish().ok()?;
        Some(hex::encode(gz))
    }

    fn from_checkpoint_str(raw: &str) -> Option<Self> {
        let bytes = hex::decode(raw.trim()).ok()?;
        let mut dec = GzDecoder::new(&bytes[..]);
        let mut json = Vec::new();
        dec.read_to_end(&mut json).ok()?;
        let net: Self = serde_json::from_slice(&json).ok()?;
        if net.feature_weights.len() != N_FEAT_ROWS * D_MODEL || net.blocks.len() != N_LAYERS {
            return None;
        }
        Some(net)
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

#[derive(Clone)]
struct TrainingExample {
    user_id: i64,
    context: Vec<(Vec<(usize, f32)>, Option<[f32; 4]>, usize)>,
    positive: Track,
    negatives: Vec<Track>,
    weight: f32,
}

#[derive(Debug, Clone, Default)]
pub struct TrainEpochStats {
    pub steps_run: usize,
    pub total_steps: u64,
    pub avg_loss: f32,
    pub examples_used: usize,
    pub distilled_users: usize,
}

pub struct TransformerEngine {
    net: RwLock<TransformerNet>,
    user_vec_cache: RwLock<HashMap<i64, (i64, [f32; D_MODEL])>>,
    train_lock: std::sync::Mutex<()>,
}

impl TransformerEngine {
    pub fn load_or_new(store: &Store) -> Self {
        let net = store
            .kv_get(CHECKPOINT_KV_KEY)
            .and_then(|raw| TransformerNet::from_checkpoint_str(&raw))
            .unwrap_or_else(TransformerNet::new);
        Self {
            net: RwLock::new(net),
            user_vec_cache: RwLock::new(HashMap::new()),
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
        serde_json::json!({
            "architecture": "RoPE-QKVO + XSA + GEGLU Transformer",
            "paper": "arXiv:2603.09078",
            "dModel": D_MODEL,
            "layers": N_LAYERS,
            "heads": N_HEADS,
            "steps": net.step_count,
            "avgLoss": (net.avg_loss * 1000.0).round() / 1000.0,
            "usersTracked": net.user_embs.len(),
            "lastTrainedAt": net.last_trained_at,
        })
    }

    /// Computes normalized Transformer compatibility scores for `candidates` given:
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
        let confidence = (net.step_count as f32 / 24.0).clamp(0.05, 1.0);
        let ctx_vec = self.compute_context_vector_locked(
            &net,
            store,
            user_id,
            context_tracks,
            query,
            moods,
            genres,
        );

        let mut scores = Vec::with_capacity(candidates.len());
        for t in candidates {
            let (t_unit, _, _, _, _) = net.encode_track_unit(t);
            let mut cos = 0.0f32;
            for d in 0..D_MODEL {
                cos += ctx_vec[d] * t_unit[d];
            }
            let s = cos * net.logit_scale * confidence;
            scores.push(if s.is_finite() { s } else { 0.0 });
        }
        scores
    }

    /// Confidence-adaptive multiplier in `[0.45, 1.60]` (or `0.0` if untrained) that scales
    /// the Transformer's influence up as both `step_count` and user `history_count` grow.
    pub fn adaptive_multiplier(&self, history_count: usize) -> f64 {
        let steps = self.step_count() as f64;
        if steps <= 0.0 {
            return 0.0;
        }
        let model_factor = 0.55 + 0.80 * (steps / 120.0).clamp(0.0, 1.0);
        let user_factor = 0.75 + 0.45 * ((history_count as f64) / 30.0).clamp(0.0, 1.0);
        (model_factor * user_factor).clamp(0.45, 1.60)
    }

    /// Scores `(artist_name, genres)` pairs against the user's Transformer context vector.
    pub fn score_artist_candidates(
        &self,
        store: &Store,
        user_id: i64,
        artists: &[(String, String)],
    ) -> Vec<f32> {
        if artists.is_empty() {
            return Vec::new();
        }
        let net = self.net.read().unwrap_or_else(|e| e.into_inner());
        if net.step_count == 0 {
            return vec![0.0; artists.len()];
        }
        let confidence = (net.step_count as f32 / 24.0).clamp(0.05, 1.0);
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
        let has_extra_ctx = !context_tracks.is_empty()
            || query.map_or(false, |q| !q.trim().is_empty())
            || !moods.is_empty()
            || !genres.is_empty();

        let now = Utc::now().timestamp();
        if !has_extra_ctx && user_id > 0 {
            if let Ok(cache) = self.user_vec_cache.read() {
                if let Some(&(ts, vec)) = cache.get(&user_id) {
                    if now - ts < 120 {
                        return vec;
                    }
                }
            }
        }

        let mut seq_items: Vec<(Vec<(usize, f32)>, Option<[f32; 4]>, usize)> = Vec::new();
        if user_id > 0 {
            let hist_cap = if has_extra_ctx { 16 } else { MAX_SEQ_LEN - 1 };
            let mut neg_items = Vec::new();
            let mut pos_items = Vec::new();
            let mut seen_ids = HashSet::new();
            if let Ok(hist) = store.load_user_neural_history(user_id, hist_cap) {
                for (t, kind, _) in hist {
                    seen_ids.insert(t.id.clone());
                    let act = event_kind_to_action(&kind);
                    let (f, s) = TransformerNet::extract_track_features(&t);
                    if act == ACTION_SKIP || act == ACTION_DISLIKE {
                        neg_items.push((f, Some(s), act));
                    } else {
                        pos_items.push((f, Some(s), act));
                    }
                }
            }
            if let Ok(stars) = store.starred_tracks(user_id) {
                for (t, _) in stars.into_iter().take(12) {
                    if seen_ids.insert(t.id.clone()) {
                        let (f, s) = TransformerNet::extract_track_features(&t);
                        pos_items.push((f, Some(s), ACTION_STAR));
                    }
                }
            }
            // Place negative feedback earlier in the sequence and positive feedback closest to [USER_u].
            for item in neg_items.into_iter().take(4) {
                seq_items.push(item);
            }
            for item in pos_items {
                seq_items.push(item);
            }
        }

        for t in context_tracks.iter().take(12) {
            let (f, s) = TransformerNet::extract_track_features(t);
            seq_items.push((f, Some(s), ACTION_SEED));
        }

        if query.map_or(false, |q| !q.trim().is_empty()) || !moods.is_empty() || !genres.is_empty() {
            let qf = TransformerNet::extract_query_features(query.unwrap_or(""), moods, genres);
            if !qf.is_empty() {
                seq_items.push((qf, None, ACTION_QUERY));
            }
        }

        let out = net.forward_context(user_id.max(0), &seq_items).ctx_unit;
        if !has_extra_ctx && user_id > 0 {
            if let Ok(mut cache) = self.user_vec_cache.write() {
                cache.insert(user_id, (now, out));
            }
        }
        out
    }

    /// Runs multi-user background training across all users' listening histories, ratings,
    /// skips/dislikes, and manual playlists, then distills personalized `neural_track` &
    /// `neural_artist` discoveries for every user (including inactive users) and checkpoints.
    pub fn train_background_epoch(
        &self,
        store: &Store,
        max_steps: usize,
        batch_size: usize,
        distill_users: bool,
    ) -> TrainEpochStats {
        let _train_guard = self.train_lock.lock().unwrap_or_else(|e| e.into_inner());
        let corpus = match store.load_transformer_corpus(100, 3600) {
            Ok(c) => c,
            Err(_) => return TrainEpochStats::default(),
        };
        let examples = build_training_examples(&corpus);
        if examples.is_empty() {
            return TrainEpochStats::default();
        }

        let mut rng = StdRng::seed_from_u64(
            0x54524149u64
                ^ (corpus.tracks.len() as u64).wrapping_mul(0x517cc1b727220a95)
                ^ self.step_count().wrapping_mul(0x9e3779b97f4a7c15),
        );
        let mut indices: Vec<usize> = (0..examples.len()).collect();
        indices.shuffle(&mut rng);

        let bs = batch_size.clamp(4, 32);
        let mut steps_run = 0usize;

        let mut net_clone = self.net.read().unwrap_or_else(|e| e.into_inner()).clone();
        let lr = (0.0045f32 / (1.0 + (net_clone.step_count as f32) * 0.0005)).max(0.0010);

        for _pass in 0..16 {
            if steps_run >= max_steps {
                break;
            }
            indices.shuffle(&mut rng);
            for chunk in indices.chunks(bs) {
                if steps_run >= max_steps {
                    break;
                }
                let batch: Vec<TrainingExample> =
                    chunk.iter().map(|&i| examples[i].clone()).collect();
                let _ = net_clone.train_step_examples(&batch, lr);
                steps_run += 1;
            }
        }
        let total_steps = net_clone.step_count;
        let last_loss = net_clone.avg_loss;

        let checkpoint = if steps_run > 0 {
            net_clone.to_checkpoint_str()
        } else {
            None
        };

        let distilled_users = if distill_users {
            distill_corpus_users(&net_clone, store, &corpus)
        } else {
            0
        };

        // Swap trained weights into live engine and invalidate cached user vectors.
        {
            let mut live = self.net.write().unwrap_or_else(|e| e.into_inner());
            *live = net_clone;
        }
        if let Ok(mut cache) = self.user_vec_cache.write() {
            cache.clear();
        }
        if let Some(cp) = checkpoint {
            store.kv_set(CHECKPOINT_KV_KEY, &cp);
        }

        TrainEpochStats {
            steps_run,
            total_steps,
            avg_loss: last_loss,
            examples_used: examples.len(),
            distilled_users,
        }
    }
}

fn distill_corpus_users(
    net: &TransformerNet,
    store: &Store,
    corpus: &NeuralTrainingCorpus,
) -> usize {
    if net.step_count < 10 || corpus.catalog_ids.is_empty() {
        return 0;
    }
    let mut catalog_encoded: Vec<(&Track, [f32; D_MODEL])> = Vec::with_capacity(corpus.catalog_ids.len());
    for tid in &corpus.catalog_ids {
        if let Some(t) = corpus.tracks.get(tid) {
            let (u, _, _, _, _) = net.encode_track_unit(t);
            catalog_encoded.push((t, u));
        }
    }
    if catalog_encoded.is_empty() {
        return 0;
    }

    let mut distilled = 0usize;
    for &uid in &corpus.user_ids {
        let seq_items = build_user_sequence_from_corpus(uid, corpus, MAX_SEQ_LEN - 1);
        if seq_items.len() < 2 {
            continue;
        }
        let ctx = net.forward_context(uid, &seq_items).ctx_unit;
        let disliked: HashSet<&str> = corpus
            .user_disliked
            .get(&uid)
            .map(|v| v.iter().map(String::as_str).collect())
            .unwrap_or_default();
        let disliked_keys: HashSet<String> = disliked
            .iter()
            .filter_map(|id| corpus.tracks.get(*id).map(Track::song_key))
            .collect();

        let mut scored_tracks: Vec<(f64, &Track)> = Vec::with_capacity(catalog_encoded.len());
        for (t, t_unit) in &catalog_encoded {
            if disliked.contains(t.id.as_str()) || disliked_keys.contains(&t.song_key()) {
                continue;
            }
            let mut cos = 0.0f32;
            for d in 0..D_MODEL {
                cos += ctx[d] * t_unit[d];
            }
            if cos > 0.22 {
                let w = ((cos as f64) * 3.0).clamp(0.1, 3.5);
                scored_tracks.push((w, *t));
            }
        }
        scored_tracks.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        let mut track_links = Vec::with_capacity(60);
        let mut artist_scores: HashMap<String, f64> = HashMap::new();
        for (w, t) in scored_tracks.into_iter().take(60) {
            track_links.push((t.id.clone(), w));
            let ak = slug(&primary_artist(&t.artist));
            if !ak.is_empty() && ak != "unknown-artist" {
                artist_scores
                    .entry(ak)
                    .and_modify(|cur| *cur = (*cur).max(w * 0.85))
                    .or_insert(w * 0.85);
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

fn build_user_sequence_from_corpus(
    uid: i64,
    corpus: &NeuralTrainingCorpus,
    max_items: usize,
) -> Vec<(Vec<(usize, f32)>, Option<[f32; 4]>, usize)> {
    let mut negs = Vec::new();
    let mut pos = Vec::new();
    let mut seen_ids = HashSet::new();
    if let Some(evs) = corpus.user_events.get(&uid) {
        for (tid, kind, _, _) in evs.iter().rev().take(max_items).rev() {
            if let Some(t) = corpus.tracks.get(tid) {
                seen_ids.insert(tid.as_str());
                let act = event_kind_to_action(kind);
                let (f, s) = TransformerNet::extract_track_features(t);
                if act == ACTION_SKIP || act == ACTION_DISLIKE {
                    negs.push((f, Some(s), act));
                } else {
                    pos.push((f, Some(s), act));
                }
            }
        }
    }
    if let Some(stars) = corpus.user_stars.get(&uid) {
        for tid in stars.iter().take(12) {
            if seen_ids.insert(tid.as_str()) {
                if let Some(t) = corpus.tracks.get(tid) {
                    let (f, s) = TransformerNet::extract_track_features(t);
                    pos.push((f, Some(s), ACTION_STAR));
                }
            }
        }
    }
    let mut seq: Vec<_> = negs.into_iter().take(4).chain(pos).collect();
    if seq.len() < 4 {
        for (pl_uid, _, tids) in &corpus.playlists {
            if *pl_uid != uid {
                continue;
            }
            for tid in tids {
                if seq.len() >= max_items {
                    break;
                }
                if seen_ids.insert(tid.as_str()) {
                    if let Some(t) = corpus.tracks.get(tid) {
                        let (f, s) = TransformerNet::extract_track_features(t);
                        seq.push((f, Some(s), ACTION_PLAYLIST));
                    }
                }
            }
        }
    }
    if seq.len() > max_items {
        let drop_n = seq.len() - max_items;
        seq.drain(0..drop_n);
    }
    seq
}

fn build_training_examples(corpus: &NeuralTrainingCorpus) -> Vec<TrainingExample> {
    if corpus.tracks.len() < 4 {
        return Vec::new();
    }
    let mut rng = StdRng::seed_from_u64(
        0x42454154 ^ (corpus.tracks.len() as u64) ^ ((corpus.user_ids.len() as u64) << 16),
    );
    let mut all_track_ids: Vec<&String> = corpus.tracks.keys().collect();
    all_track_ids.sort();
    let mut out = Vec::new();

    // Collect each user's positive set so cross-user negatives explicitly contrast users.
    let mut user_pos_sets: HashMap<i64, HashSet<String>> = HashMap::new();
    for &uid in &corpus.user_ids {
        let mut pset = HashSet::new();
        if let Some(stars) = corpus.user_stars.get(&uid) {
            for id in stars { pset.insert(id.clone()); }
        }
        if let Some(ratings) = corpus.user_ratings.get(&uid) {
            for (id, r) in ratings {
                if *r >= 4 { pset.insert(id.clone()); }
            }
        }
        if let Some(evs) = corpus.user_events.get(&uid) {
            for (id, kind, _, _) in evs {
                if is_positive_event_kind(kind) {
                    pset.insert(id.clone());
                }
            }
        }
        user_pos_sets.insert(uid, pset);
    }
    for (uid, _, tids) in &corpus.playlists {
        let entry = user_pos_sets.entry(*uid).or_default();
        for tid in tids {
            entry.insert(tid.clone());
        }
    }

    let sample_negatives = |uid: i64,
                            pos: &Track,
                            pos_artist: &str,
                            rng: &mut StdRng|
     -> Vec<Track> {
        let mut negs = Vec::with_capacity(4);
        let mut seen = HashSet::new();
        seen.insert(pos.id.clone());
        let user_pos = user_pos_sets.get(&uid);
        let pos_genres: HashSet<String> = pos
            .genre
            .split([',', ';', '/'])
            .map(|g| g.trim().to_lowercase())
            .filter(|g| g.len() >= 2)
            .collect();

        // 1. User's own skipped/disliked tracks (strong personalized negatives)
        if let Some(dislikes) = corpus.user_disliked.get(&uid) {
            for _ in 0..2 {
                if !dislikes.is_empty() {
                    let pick = &dislikes[rng.gen_range(0..dislikes.len())];
                    if seen.insert(pick.clone()) {
                        if let Some(t) = corpus.tracks.get(pick) {
                            negs.push(t.clone());
                        }
                    }
                }
            }
        }

        // 2. Cross-user negatives: tracks liked by OTHER users but not this user (teaches user differentiation)
        if corpus.user_ids.len() > 1 {
            for _ in 0..4 {
                let other_uid = corpus.user_ids[rng.gen_range(0..corpus.user_ids.len())];
                if other_uid == uid {
                    continue;
                }
                if let Some(other_pos) = user_pos_sets.get(&other_uid) {
                    let overlaps_taste = user_pos
                        .map_or(false, |up| other_pos.iter().any(|id| up.contains(id)));
                    if !overlaps_taste && !other_pos.is_empty() {
                        let skip_n = rng.gen_range(0..other_pos.len());
                        if let Some(cand_id) = other_pos.iter().nth(skip_n) {
                            if !user_pos.map_or(false, |s| s.contains(cand_id))
                                && seen.insert(cand_id.clone())
                            {
                                if let Some(t) = corpus.tracks.get(cand_id) {
                                    negs.push(t.clone());
                                    if negs.len() >= 4 {
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // 3. Random catalog negatives (preferring different artists and non-overlapping genres)
        let mut tries = 0usize;
        while negs.len() < 4 && tries < 28 {
            tries += 1;
            let pick = all_track_ids[rng.gen_range(0..all_track_ids.len())];
            if user_pos.map_or(false, |s| s.contains(pick.as_str())) {
                continue;
            }
            if let Some(t) = corpus.tracks.get(pick.as_str()) {
                if slug(&primary_artist(&t.artist)) == pos_artist && tries < 24 {
                    continue;
                }
                if tries < 20 && !pos_genres.is_empty() {
                    let shares_genre = t
                        .genre
                        .split([',', ';', '/'])
                        .map(|g| g.trim().to_lowercase())
                        .any(|g| pos_genres.contains(&g));
                    if shares_genre {
                        continue;
                    }
                }
                if seen.insert(t.id.clone()) {
                    negs.push(t.clone());
                }
            }
        }
        negs
    };

    // Task 1: Sequential next-interaction prediction across all users
    for &uid in &corpus.user_ids {
        let Some(evs) = corpus.user_events.get(&uid) else {
            continue;
        };
        let mut seq_tokens: Vec<(Vec<(usize, f32)>, Option<[f32; 4]>, usize, Track, String)> = Vec::new();
        for (tid, kind, _, _) in evs {
            if let Some(t) = corpus.tracks.get(tid) {
                let (f, s) = TransformerNet::extract_track_features(t);
                seq_tokens.push((f, Some(s), event_kind_to_action(kind), t.clone(), kind.clone()));
            }
        }
        if seq_tokens.len() < 2 {
            continue;
        }
        let step_stride = if seq_tokens.len() > 24 { 2 } else { 1 };
        for target_idx in (1..seq_tokens.len()).step_by(step_stride) {
            let target_kind = &seq_tokens[target_idx].4;
            if !is_positive_event_kind(target_kind) {
                continue;
            }
            let pos = &seq_tokens[target_idx].3;
            let start = target_idx.saturating_sub(MAX_SEQ_LEN - 1);
            let ctx: Vec<_> = seq_tokens[start..target_idx]
                .iter()
                .map(|(f, s, a, _, _)| (f.clone(), *s, *a))
                .collect();
            let pos_art = slug(&primary_artist(&pos.artist));
            let negs = sample_negatives(uid, pos, &pos_art, &mut rng);
            if negs.is_empty() {
                continue;
            }
            let w = match target_kind.as_str() {
                "star" | "playlist_add" => 1.35,
                "complete" => 1.15,
                _ => 0.95,
            };
            out.push(TrainingExample {
                user_id: uid,
                context: ctx,
                positive: pos.clone(),
                negatives: negs,
                weight: w,
            });
        }
    }

    // Task 2: Cross-user manual playlist coherence & query alignment
    for (uid, pl_name, tids) in &corpus.playlists {
        let pl_tracks: Vec<&Track> = tids.iter().filter_map(|id| corpus.tracks.get(id)).collect();
        if pl_tracks.len() < 2 {
            continue;
        }
        let pl_query_feats = TransformerNet::extract_query_features(pl_name, &[], &[]);
        for (hold_idx, &pos) in pl_tracks.iter().enumerate().take(12) {
            let mut ctx = Vec::new();
            for (i, &t) in pl_tracks.iter().enumerate() {
                if i != hold_idx && ctx.len() < 12 {
                    let (f, s) = TransformerNet::extract_track_features(t);
                    ctx.push((f, Some(s), ACTION_PLAYLIST));
                }
            }
            if !pl_query_feats.is_empty() {
                ctx.push((pl_query_feats.clone(), None, ACTION_QUERY));
            }
            let pos_art = slug(&primary_artist(&pos.artist));
            let negs = sample_negatives(*uid, pos, &pos_art, &mut rng);
            if negs.is_empty() {
                continue;
            }
            out.push(TrainingExample {
                user_id: *uid,
                context: ctx,
                positive: pos.clone(),
                negatives: negs,
                weight: 1.25,
            });
        }
    }

    // Task 3: Explicit stars / high ratings vs dislikes (captures users without dense event logs)
    for &uid in &corpus.user_ids {
        let mut positives: Vec<&Track> = Vec::new();
        if let Some(stars) = corpus.user_stars.get(&uid) {
            for tid in stars {
                if let Some(t) = corpus.tracks.get(tid) {
                    positives.push(t);
                }
            }
        }
        if let Some(ratings) = corpus.user_ratings.get(&uid) {
            for (tid, r) in ratings {
                if *r >= 4 {
                    if let Some(t) = corpus.tracks.get(tid) {
                        positives.push(t);
                    }
                }
            }
        }
        if positives.len() < 2 {
            continue;
        }
        for (i, &pos) in positives.iter().enumerate().take(8) {
            let mut ctx = Vec::new();
            if let Some(dislikes) = corpus.user_disliked.get(&uid) {
                if let Some(did) = dislikes.get(i % dislikes.len()) {
                    if let Some(dt) = corpus.tracks.get(did) {
                        let (f, s) = TransformerNet::extract_track_features(dt);
                        ctx.push((f, Some(s), ACTION_DISLIKE));
                    }
                }
            }
            for (j, &t) in positives.iter().enumerate() {
                if i != j && ctx.len() < 10 {
                    let (f, s) = TransformerNet::extract_track_features(t);
                    ctx.push((f, Some(s), ACTION_STAR));
                }
            }
            let pos_art = slug(&primary_artist(&pos.artist));
            let negs = sample_negatives(uid, pos, &pos_art, &mut rng);
            if negs.is_empty() {
                continue;
            }
            out.push(TrainingExample {
                user_id: uid,
                context: ctx,
                positive: pos.clone(),
                negatives: negs,
                weight: 1.3,
            });
        }
    }

    // Balance training weights across users so high-volume listeners don't drown out quieter users.
    let mut per_user_counts: HashMap<i64, usize> = HashMap::new();
    for ex in &out {
        *per_user_counts.entry(ex.user_id).or_insert(0) += 1;
    }
    for ex in &mut out {
        let cnt = per_user_counts.get(&ex.user_id).copied().unwrap_or(1) as f32;
        ex.weight *= (3.0 / cnt).sqrt().clamp(0.35, 1.5);
    }

    out
}

#[inline]
fn event_kind_to_action(kind: &str) -> usize {
    match kind {
        "star" | "rate" => ACTION_STAR,
        "playlist_add" => ACTION_PLAYLIST,
        "complete" => ACTION_COMPLETE,
        "play" | "search_pick" | "mix_play" => ACTION_PLAY,
        "skip" => ACTION_SKIP,
        "not_interested" | "unstar" | "playlist_remove" => ACTION_DISLIKE,
        _ => ACTION_PLAY,
    }
}

#[inline]
fn is_positive_event_kind(kind: &str) -> bool {
    matches!(
        kind,
        "star" | "playlist_add" | "complete" | "play" | "search_pick" | "mix_play"
    )
}

#[inline]
fn mood_to_idx(m: Mood) -> usize {
    match m {
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
                assert!(
                    dot.abs() < 1e-4,
                    "XSA output not orthogonal to self-value at t={}, h={}: dot={}",
                    t,
                    h,
                    dot
                );
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
        let ex = TrainingExample {
            user_id: 42,
            context: vec![(f1.0, Some(f1.1), ACTION_STAR), (f2.0, Some(f2.1), ACTION_COMPLETE)],
            positive: pos,
            negatives: vec![neg1, neg2],
            weight: 1.0,
        };

        let eval_loss = |n: &TransformerNet| -> f32 {
            let ctx = n.forward_context(ex.user_id, &ex.context);
            let mut cands = Vec::new();
            cands.push(n.encode_track_unit(&ex.positive).0);
            for neg in &ex.negatives {
                cands.push(n.encode_track_unit(neg).0);
            }
            let logits: Vec<f32> = cands
                .iter()
                .map(|c| {
                    let mut cos = 0.0f32;
                    for d in 0..D_MODEL { cos += ctx.ctx_unit[d] * c[d]; }
                    cos * n.logit_scale
                })
                .collect();
            let max_l = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            let sum_e: f32 = logits.iter().map(|l| (l - max_l).exp()).sum();
            -(logits[0] - max_l - sum_e.ln())
        };

        // Compute analytical gradients directly without clipping by running one unclipped backward
        let ctx_cache = net.forward_context(ex.user_id, &ex.context);
        let (u_pos, _, _, _, _) = net.encode_track_unit(&ex.positive);
        let (u_n1, _, _, _, _) = net.encode_track_unit(&ex.negatives[0]);
        let (u_n2, _, _, _, _) = net.encode_track_unit(&ex.negatives[1]);
        let cands = [u_pos, u_n1, u_n2];
        let logits: Vec<f32> = cands
            .iter()
            .map(|c| {
                let mut cos = 0.0f32;
                for d in 0..D_MODEL { cos += ctx_cache.ctx_unit[d] * c[d]; }
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

        // Finite-difference check on layer 0 Wq, Wv, Wo, W1, and user embedding!
        let eps = 1e-3f32;
        for idx in [0usize, 17, 131] {
            let orig = net.blocks[0].wv.w[idx];
            net.blocks[0].wv.w[idx] = orig + eps;
            let lp = eval_loss(&net);
            net.blocks[0].wv.w[idx] = orig - eps;
            let lm = eval_loss(&net);
            net.blocks[0].wv.w[idx] = orig;
            let fd = (lp - lm) / (2.0 * eps);
            let an = bgs[0].wv[idx];
            assert!(
                (fd - an).abs() < 2e-2,
                "Wv[{}] grad mismatch: fd={} analytical={}",
                idx,
                fd,
                an
            );
        }
        for idx in [3usize, 49] {
            let orig = net.blocks[0].wq.w[idx];
            net.blocks[0].wq.w[idx] = orig + eps;
            let lp = eval_loss(&net);
            net.blocks[0].wq.w[idx] = orig - eps;
            let lm = eval_loss(&net);
            net.blocks[0].wq.w[idx] = orig;
            let fd = (lp - lm) / (2.0 * eps);
            let an = bgs[0].wq[idx];
            assert!(
                (fd - an).abs() < 2e-2,
                "Wq[{}] grad mismatch: fd={} analytical={}",
                idx,
                fd,
                an
            );
        }
        for idx in [5usize, 77] {
            let orig = net.blocks[0].w_gate.w[idx];
            net.blocks[0].w_gate.w[idx] = orig + eps;
            let lp = eval_loss(&net);
            net.blocks[0].w_gate.w[idx] = orig - eps;
            let lm = eval_loss(&net);
            net.blocks[0].w_gate.w[idx] = orig;
            let fd = (lp - lm) / (2.0 * eps);
            let an = bgs[0].w_gate[idx];
            assert!(
                (fd - an).abs() < 2e-2,
                "W_gate[{}] grad mismatch: fd={} analytical={}",
                idx,
                fd,
                an
            );
        }
        // Check user embedding gradient backpropagated through both RoPE-QKVO + XSA layers
        let _ = net.get_or_insert_user_row(42);
        for d in [0usize, 11, 35] {
            let orig = net.user_embs.get(&42).unwrap().w[d];
            net.user_embs.get_mut(&42).unwrap().w[d] = orig + eps;
            let lp = eval_loss(&net);
            net.user_embs.get_mut(&42).unwrap().w[d] = orig - eps;
            let lm = eval_loss(&net);
            net.user_embs.get_mut(&42).unwrap().w[d] = orig;
            let fd = (lp - lm) / (2.0 * eps);
            let an = g_h[u_pos_idx * D_MODEL + d];
            assert!(
                (fd - an).abs() < 2e-2,
                "UserEmb[{}] grad mismatch: fd={} analytical={}",
                d,
                fd,
                an
            );
        }
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

        // User 1 loves rock & skips pop
        store.star_track(u_rock.id, "r1").unwrap();
        store.star_track(u_rock.id, "r2").unwrap();
        store.record_event(u_rock.id, "r1", "complete", 1.0).unwrap();
        store.record_event(u_rock.id, "r2", "complete", 1.0).unwrap();
        store.record_event(u_rock.id, "p1", "skip", 1.0).unwrap();
        store.record_event(u_rock.id, "p2", "skip", 1.0).unwrap();
        store.set_rating(u_rock.id, "p1", 1).unwrap();

        // User 2 loves synthpop & skips rock
        store.star_track(u_pop.id, "p1").unwrap();
        store.star_track(u_pop.id, "p2").unwrap();
        store.star_track(u_pop.id, "p4").unwrap();
        store.record_event(u_pop.id, "p1", "complete", 1.0).unwrap();
        store.record_event(u_pop.id, "p2", "complete", 1.0).unwrap();
        store.record_event(u_pop.id, "p4", "complete", 1.0).unwrap();
        store.record_event(u_pop.id, "r1", "skip", 1.0).unwrap();
        store.record_event(u_pop.id, "r2", "skip", 1.0).unwrap();
        store.set_rating(u_pop.id, "r1", 1).unwrap();

        // User 3 (another user!) curates a playlist bridging Nirvana (r2) with unseen shoegaze (r3, r4)
        store
            .create_playlist(
                u_curator.id,
                "90s Alt & Shoegaze",
                "",
                false,
                &["r2".into(), "r3".into(), "r4".into()],
            )
            .unwrap();

        let engine = TransformerEngine::load_or_new(&store);
        let stats = engine.train_background_epoch(&store, 28, 6, true);
        assert!(stats.steps_run >= 10);
        assert!(stats.distilled_users >= 2);

        // Score unseen tracks r3 (shoegaze bridged by User 3) vs p3 (synthpop) for User 1 vs User 2
        let test_cands = vec![tracks[2].clone(), tracks[6].clone()]; // r3, p3
        let rock_scores = engine.score_candidates(&store, u_rock.id, &[], None, &[], &[], &test_cands);
        let pop_scores = engine.score_candidates(&store, u_pop.id, &[], None, &[], &[], &test_cands);

        assert!(
            rock_scores[0] > rock_scores[1],
            "Rock user should prefer cross-user shoegaze track r3 ({}) over synthpop p3 ({})",
            rock_scores[0],
            rock_scores[1]
        );
        assert!(
            pop_scores[1] > pop_scores[0],
            "Pop user should prefer synthpop p3 ({}) over shoegaze r3 ({})",
            pop_scores[1],
            pop_scores[0]
        );

        // Verify checkpoint roundtrip preserves exact scoring
        let reloaded = TransformerEngine::load_or_new(&store);
        assert_eq!(reloaded.step_count(), engine.step_count());
        let rock_scores_reloaded =
            reloaded.score_candidates(&store, u_rock.id, &[], None, &[], &[], &test_cands);
        assert!((rock_scores[0] - rock_scores_reloaded[0]).abs() < 1e-4);

        // Verify adaptive_multiplier scales with history_count
        let low_hist = engine.adaptive_multiplier(0);
        let high_hist = engine.adaptive_multiplier(40);
        assert!(low_hist > 0.0 && high_hist > low_hist);

        // Verify distilled neural_artist links are queryable
        let neural_artists = store.load_neural_artist_links(u_rock.id, 10).unwrap();
        assert!(!neural_artists.is_empty());

        // Verify score_artist_candidates differentiates rock vs pop artists for u_rock
        let art_scores = engine.score_artist_candidates(
            &store,
            u_rock.id,
            &[
                ("Slowdive".to_string(), "Shoegaze, Rock".to_string()),
                ("Dua Lipa".to_string(), "Dance Pop, Disco".to_string()),
            ],
        );
        assert!(
            art_scores[0] > art_scores[1],
            "Rock user should score Slowdive ({}) higher than Dua Lipa ({})",
            art_scores[0],
            art_scores[1]
        );
    }

    #[test]
    fn extract_features_captures_subgenres_collabs_and_inferred_moods() {
        let store = Store::open_memory().unwrap();
        let engine = TransformerEngine::load_or_new(&store);
        assert_eq!(engine.adaptive_multiplier(10), 0.0);

        let collab = mk_track(
            "x1",
            "Midnight Acoustic Ballad",
            "Daft Punk feat. Julian Casablancas",
            "Indie Rock",
            "",
            2013,
        );
        let (feats, _) = TransformerNet::extract_track_features(&collab);
        let has_artist = feats
            .iter()
            .filter(|(idx, _)| *idx >= ARTIST_OFFSET && *idx < TEXT_OFFSET)
            .count();
        let has_genre = feats
            .iter()
            .filter(|(idx, _)| *idx >= GENRE_OFFSET && *idx < MOOD_OFFSET)
            .count();
        let has_mood = feats
            .iter()
            .filter(|(idx, _)| *idx >= MOOD_OFFSET && *idx < ERA_OFFSET)
            .count();
        assert!(has_artist >= 2, "expected primary + featured artist buckets");
        assert!(has_genre >= 3, "expected full + sub-genre buckets");
        assert!(has_mood >= 1, "expected inferred mood buckets when moods is empty");
    }
}
