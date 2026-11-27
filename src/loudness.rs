//! ITU-R BS.1770-4 / EBU R128 integrated loudness analysis and peak-safe
//! constant-gain normalization helpers.
//!
//! Normalization uses a single fixed scalar gain per track (never dynamic range
//! compression or real-time AGC) capped by true-peak headroom so every track's
//! internal dynamics, transients, and quiet-to-loud contrast stay untouched.

use lofty::file::TaggedFileExt;
use lofty::probe::Probe;
use lofty::tag::ItemKey;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

/// Loudness target (LUFS). `-7.0 LUFS` matches YouTube Music's full, punchy
/// max-volume output so modern masters (`-6` to `-8 LUFS`) play around unity
/// gain while ultra-hot masters are tamed and older/dynamic tracks lift cleanly
/// to match.
#[cfg(test)]
pub const TARGET_LUFS: f64 = -7.0;
/// True-peak reference ceiling (dBTP) before the transparent safety peak limiter.
#[cfg(test)]
pub const TRUE_PEAK_CEILING_DB: f64 = -0.3;
/// Maximum allowed peak overshoot (dB) into the transparent lookahead peak
/// limiter + soft ceiling so older/dynamic 0-dBFS-normalized tracks (`-14` to
/// `-20 LUFS`) can still be lifted all the way to match modern tracks cleanly.
#[cfg(test)]
pub const MAX_PEAK_CATCH_DB: f64 = 13.5;
/// Minimum/maximum allowed gain adjustment (dB) so corrupt/near-silent files
/// can never produce extreme attenuation or noise-floor amplification.
#[cfg(test)]
pub const MIN_GAIN_DB: f64 = -18.0;
#[cfg(test)]
pub const MAX_GAIN_DB: f64 = 16.0;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackLoudness {
    /// Whole-track gated integrated loudness in LUFS (ITU-R BS.1770-4).
    pub lufs: f64,
    /// Oversampled true peak in dBTP (0.0 dBTP = digital full scale).
    pub true_peak_db: f64,
    /// True when measured from the actual audio (EBU R128 / embedded RG tags).
    #[serde(skip_serializing)]
    pub exact: bool,
}

impl TrackLoudness {
    pub fn is_valid(&self) -> bool {
        // Real musical audio always has strictly negative integrated loudness
        // (even a full-scale 0 dBFS sine wave is -3.01 LUFS); rejecting >= -1.0
        // prevents bogus 0.0 LUFS readings from failed/empty analyzer runs from
        // ever poisoning the DB and attenuating playback by 7+ dB.
        self.lufs.is_finite()
            && self.lufs > -60.0
            && self.lufs <= -1.0
            && self.true_peak_db.is_finite()
            && self.true_peak_db > -60.0
            && self.true_peak_db <= 12.0
    }
}

/// Computes the peak-safe linear output gain for a track at a given user volume
/// (`0.0..=1.0`). Uses one fixed scalar multiplier across the whole track so
/// internal dynamic range stays intact; when boosting quiet tracks, allows at
/// most `MAX_PEAK_CATCH_DB` into the transparent `-1 dBFS` peak catcher so rare
/// millisecond transients never hard-clip.
#[cfg(test)]
pub fn compute_output_gain(lufs: f64, true_peak_db: f64, user_volume: f64) -> f64 {
    let vol = user_volume.clamp(0.0, 1.0);
    if vol <= 0.0 || !lufs.is_finite() {
        return 0.0;
    }
    let gain_db = (TARGET_LUFS - lufs).clamp(MIN_GAIN_DB, MAX_GAIN_DB);
    let track_gain = 10.0_f64.powf(gain_db / 20.0);
    let tp_db = if true_peak_db.is_finite() {
        true_peak_db
    } else {
        0.0
    };
    let tp_lin = 10.0_f64.powf(tp_db / 20.0).clamp(0.01, 4.0);
    let ceiling_lin = 10.0_f64.powf((TRUE_PEAK_CEILING_DB + MAX_PEAK_CATCH_DB) / 20.0);
    let max_safe_total = ceiling_lin / tp_lin;
    (vol * track_gain).min(max_safe_total)
}

/// Measures a local or cached audio file's integrated loudness and true peak.
///
/// Prefers exact ITU-R BS.1770-4 measurement via `ffmpeg` (`ebur128=peak=true`)
/// so every format and source shares an identical loudness reference. Falls
/// back to embedded ReplayGain / R128 / iTunNORM tags when `ffmpeg` is
/// unavailable or fails.
pub async fn measure_file_loudness(
    path: &Path,
    ffmpeg_available: bool,
) -> Option<(TrackLoudness, &'static str)> {
    if ffmpeg_available {
        if let Some(m) = measure_with_ffmpeg(path).await {
            return Some((m, "ebur128"));
        }
    }
    let p = path.to_path_buf();
    let tag_res = tokio::task::spawn_blocking(move || read_tag_loudness(&p))
        .await
        .ok()
        .flatten();
    tag_res.map(|m| (m, "tag"))
}

async fn measure_with_ffmpeg(path: &Path) -> Option<TrackLoudness> {
    // ITU-R BS.1770-4 is defined at 48 kHz. Resampling explicitly before ebur128
    // avoids FFmpeg 5.1 (Debian Bookworm) filter-graph failures on 88.2/96/192 kHz
    // Hi-Res FLACs and speeds up Hi-Res loudness scanning by 2-4x.
    if let Some(m) =
        run_ffmpeg_ebur128(path, "aresample=48000,ebur128=peak=true:framelog=quiet").await
    {
        return Some(m);
    }
    // Fallback to sample-peak mode if swresample 4x true-peak oversampling fails.
    run_ffmpeg_ebur128(path, "aresample=48000,ebur128=peak=sample:framelog=quiet").await
}

async fn run_ffmpeg_ebur128(path: &Path, filter: &str) -> Option<TrackLoudness> {
    let mut cmd = Command::new("ffmpeg");
    cmd.kill_on_drop(true)
        .args([
            "-hide_banner",
            "-nostats",
            "-i",
            &path.to_string_lossy(),
            "-vn",
            "-sn",
            "-dn",
            "-af",
            filter,
            "-f",
            "null",
            "-",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    let output = tokio::time::timeout(Duration::from_secs(30), cmd.output())
        .await
        .ok()?
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    parse_ebur128_summary(&stderr)
}

/// Parses FFmpeg's `ebur128=peak=true` summary block from stderr.
pub fn parse_ebur128_summary(stderr: &str) -> Option<TrackLoudness> {
    // Take the final `Summary:` section in case multiple streams or retries printed one.
    let summary_idx = stderr.rfind("Summary:")?;
    let summary = &stderr[summary_idx..];

    let mut section = "";
    let mut integrated_lufs: Option<f64> = None;
    let mut true_peak_db: Option<f64> = None;

    for raw_line in summary.lines() {
        let line = raw_line.trim();
        if line.starts_with("Integrated loudness:") {
            section = "integrated";
            continue;
        }
        if line.starts_with("Loudness range:") {
            section = "lra";
            continue;
        }
        if line.starts_with("True peak:") || line.starts_with("Sample peak:") {
            section = "peak";
            continue;
        }

        if section == "integrated" {
            if let Some(rest) = line.strip_prefix("I:") {
                if let Some(v) = parse_leading_f64(rest) {
                    integrated_lufs = Some(v);
                }
            }
        } else if section == "peak" {
            if let Some(rest) = line.strip_prefix("Peak:") {
                if let Some(v) = parse_leading_f64(rest) {
                    true_peak_db = Some(v);
                }
            }
        }
    }

    let lufs = integrated_lufs?;
    let peak = true_peak_db?;
    let m = TrackLoudness {
        lufs: (lufs * 100.0).round() / 100.0,
        true_peak_db: (peak * 100.0).round() / 100.0,
        exact: true,
    };
    m.is_valid().then_some(m)
}

fn parse_leading_f64(s: &str) -> Option<f64> {
    let tok = s.split_whitespace().next()?;
    let cleaned = tok.trim_end_matches(|c: char| !c.is_ascii_digit() && c != '.' && c != '-' && c != '+');
    let v: f64 = cleaned.parse().ok()?;
    v.is_finite().then_some(v)
}

/// Reads embedded ReplayGain, Opus R128, or Apple Sound Check (`iTunNORM`) tags.
pub fn read_tag_loudness(path: &Path) -> Option<TrackLoudness> {
    let tagged = Probe::open(path).ok()?.read().ok()?;
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag())?;

    let mut rg_gain_db: Option<f64> = None;
    let mut rg_peak_lin: Option<f64> = None;
    let mut r128_gain_q78: Option<f64> = None;
    let mut itun_norm: Option<String> = None;

    for item in tag.items() {
        let Some(val) = item.value().text() else {
            continue;
        };
        let val = val.trim();
        if val.is_empty() {
            continue;
        }
        match item.key() {
            ItemKey::ReplayGainTrackGain => {
                if let Some(db) = parse_db_value(val) {
                    rg_gain_db = Some(db);
                }
            }
            ItemKey::ReplayGainTrackPeak => {
                if let Some(pk) = parse_positive_f64(val) {
                    rg_peak_lin = Some(pk);
                }
            }
            ItemKey::Unknown(name) => {
                let upper = name.trim().to_ascii_uppercase();
                match upper.as_str() {
                    "REPLAYGAIN_TRACK_GAIN" if rg_gain_db.is_none() => {
                        rg_gain_db = parse_db_value(val);
                    }
                    "REPLAYGAIN_TRACK_PEAK" if rg_peak_lin.is_none() => {
                        rg_peak_lin = parse_positive_f64(val);
                    }
                    "R128_TRACK_GAIN" => {
                        r128_gain_q78 = val.parse::<f64>().ok().filter(|v| v.is_finite());
                    }
                    "ITUNNORM" | "----:COM.APPLE.ITUNES:ITUNNORM" => {
                        itun_norm = Some(val.to_string());
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    // 1. Opus R128_TRACK_GAIN (Q7.8 dB relative to -23 LUFS).
    if let Some(q78) = r128_gain_q78 {
        let gain_db = q78 / 256.0;
        let lufs = -23.0 - gain_db;
        let true_peak_db = rg_peak_lin
            .map(|p| 20.0 * p.log10())
            .unwrap_or(0.0);
        let m = TrackLoudness {
            lufs: (lufs * 100.0).round() / 100.0,
            true_peak_db: (true_peak_db * 100.0).round() / 100.0,
            exact: true,
        };
        if m.is_valid() {
            return Some(m);
        }
    }

    // 2. ReplayGain track gain (dB relative to -18 LUFS).
    if let Some(gain_db) = rg_gain_db {
        let lufs = -18.0 - gain_db;
        let true_peak_db = rg_peak_lin
            .map(|p| 20.0 * p.log10())
            .unwrap_or(0.0);
        let m = TrackLoudness {
            lufs: (lufs * 100.0).round() / 100.0,
            true_peak_db: (true_peak_db * 100.0).round() / 100.0,
            exact: true,
        };
        if m.is_valid() {
            return Some(m);
        }
    }

    // 3. Apple Sound Check (`iTunNORM`), referenced to -16 LUFS.
    if let Some(raw) = itun_norm {
        if let Some(m) = parse_itunnorm(&raw) {
            return Some(m);
        }
    }

    None
}

fn parse_db_value(raw: &str) -> Option<f64> {
    let s = raw.trim();
    let num = s.split_whitespace().next().unwrap_or(s);
    let num = num
        .trim_end_matches("dB")
        .trim_end_matches("db")
        .trim_end_matches("DB")
        .trim();
    let v: f64 = num.parse().ok()?;
    (v.is_finite() && (-40.0..=40.0).contains(&v)).then_some(v)
}

fn parse_positive_f64(raw: &str) -> Option<f64> {
    let v: f64 = raw.trim().parse().ok()?;
    (v.is_finite() && v > 0.0001 && v <= 4.0).then_some(v)
}

/// Parses Apple's 10-field hex `iTunNORM` Sound Check string into LUFS + peak.
pub fn parse_itunnorm(raw: &str) -> Option<TrackLoudness> {
    let vals: Vec<u32> = raw
        .split_whitespace()
        .filter_map(|tok| u32::from_str_radix(tok.trim(), 16).ok())
        .collect();
    if vals.len() < 8 {
        return None;
    }
    let power = vals[0].max(vals[1]);
    if power == 0 {
        return None;
    }
    // Sound Check power is in 1/1000ths relative to -16 LUFS reference (`1000` => 0 dB).
    let gain_db = -10.0 * (power as f64 / 1000.0).log10();
    let lufs = -16.0 - gain_db;

    // Fields 6 & 7 hold left/right peak amplitude on a 0..32768 scale.
    let peak_raw = vals[6].max(vals[7]);
    let true_peak_db = if peak_raw > 0 {
        20.0 * (peak_raw as f64 / 32768.0).log10()
    } else {
        0.0
    };

    let m = TrackLoudness {
        lufs: (lufs * 100.0).round() / 100.0,
        true_peak_db: (true_peak_db * 100.0).round() / 100.0,
        exact: true,
    };
    m.is_valid().then_some(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ffmpeg_ebur128_summary_block() {
        let sample = r#"
[Parsed_ebur128_0 @ 0x55] Summary:
  Integrated loudness:
    I:         -9.4 LUFS
    Threshold: -19.5 LUFS
  Loudness range:
    LRA:        6.2 LU
    Threshold: -29.5 LUFS
    LRA low:   -13.1 LUFS
    LRA high:  -6.9 LUFS
  True peak:
    Peak:      -0.2 dBFS
"#;
        let parsed = parse_ebur128_summary(sample).expect("summary should parse");
        assert!((parsed.lufs - (-9.4)).abs() < 0.001);
        assert!((parsed.true_peak_db - (-0.2)).abs() < 0.001);
        assert!(parsed.exact);
    }

    #[test]
    fn rejects_silent_ebur128_summary() {
        let sample = r#"
[Parsed_ebur128_0 @ 0x55] Summary:
  Integrated loudness:
    I:         -70.0 LUFS
  True peak:
    Peak:      -inf dBFS
"#;
        assert!(parse_ebur128_summary(sample).is_none());
    }

    #[test]
    fn rejects_zero_lufs_and_inf_peak_summary() {
        let bogus_zero = r#"
[Parsed_ebur128_0 @ 0x55] Summary:
  Integrated loudness:
    I:          0.0 LUFS
  True peak:
    Peak:      -inf dBFS
"#;
        assert!(parse_ebur128_summary(bogus_zero).is_none());
        let zero_with_peak = r#"
[Parsed_ebur128_0 @ 0x55] Summary:
  Integrated loudness:
    I:          0.0 LUFS
  True peak:
    Peak:       0.0 dBFS
"#;
        assert!(parse_ebur128_summary(zero_with_peak).is_none());
        assert!(!TrackLoudness {
            lufs: 0.0,
            true_peak_db: 0.0,
            exact: true,
        }
        .is_valid());
    }

    #[test]
    fn parses_itunnorm_sound_check() {
        // 00000FA0 = 4000 -> -10*log10(4) = -6.02 dB gain -> lufs = -16 - (-6.02) = -9.98 LUFS
        // Peak 00007FFF = 32767 -> ~0.0 dBFS
        let raw = " 00000FA0 00000FA0 00001F40 00001F40 00000000 00000000 00007FFF 00007FFF 00000000 00000000";
        let m = parse_itunnorm(raw).expect("iTunNORM should parse");
        assert!((m.lufs - (-9.98)).abs() < 0.05);
        assert!(m.true_peak_db.abs() < 0.05);
    }

    #[test]
    fn output_gain_matches_loudness_and_never_clips_peaks() {
        // Ultra-loud modern master (-5.0 LUFS, 0 dBTP) at 100% volume -> attenuated by -2 dB (~0.794)
        let loud = compute_output_gain(-5.0, 0.0, 1.0);
        assert!((loud - 10.0_f64.powf(-2.0 / 20.0)).abs() < 0.005);

        // Reference modern master (-7.0 LUFS, 0 dBTP) at 100% volume -> unity (1.0)
        let ref_track = compute_output_gain(-7.0, 0.0, 1.0);
        assert!((ref_track - 1.0).abs() < 0.001);

        // Older 80s dynamic track (-17.0 LUFS, 0 dBTP) at 100% volume -> lifted full +10 dB (~3.162)
        let eighties = compute_output_gain(-17.0, 0.0, 1.0);
        assert!((eighties - 10.0_f64.powf(10.0 / 20.0)).abs() < 0.01);

        // Very quiet dynamic track (-20.0 LUFS, -0.5 dBTP) at 90% volume -> lifted full +13 dB
        let quiet_90 = compute_output_gain(-20.0, -0.5, 0.9);
        assert!((quiet_90 - (0.9 * 10.0_f64.powf(13.0 / 20.0))).abs() < 0.01);

        // Extremely quiet track (-26.0 LUFS, 0 dBTP) at 100% volume -> capped by safe peak-catch budget
        let ultra_quiet = compute_output_gain(-26.0, 0.0, 1.0);
        let max_lin = 10.0_f64.powf((TRUE_PEAK_CEILING_DB + MAX_PEAK_CATCH_DB) / 20.0);
        assert!(ultra_quiet <= max_lin + 1e-6);
        assert!((ultra_quiet - max_lin).abs() < 0.001);

        // Muted / zero volume always produces silence.
        assert_eq!(compute_output_gain(-20.0, -6.0, 0.0), 0.0);
    }

    #[tokio::test]
    async fn ffmpeg_measures_real_wav_loudness_difference() {
        let ffmpeg_ok = std::process::Command::new("ffmpeg")
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ffmpeg_ok {
            return;
        }
        let dir = std::env::temp_dir().join(format!("beatlab_lufs_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let loud_wav = dir.join("loud.wav");
        let quiet_wav = dir.join("quiet.wav");

        for (path, vol) in [(&loud_wav, "-6dB"), (&quiet_wav, "-18dB")] {
            let ok = Command::new("ffmpeg")
                .args([
                    "-y",
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    &format!("sine=frequency=1000:duration=2:sample_rate=48000,volume={vol}"),
                    &path.to_string_lossy(),
                ])
                .status()
                .await
                .map(|s| s.success())
                .unwrap_or(false);
            assert!(ok, "ffmpeg failed generating test wav");
        }

        let (m_loud, src_loud) = measure_file_loudness(&loud_wav, true)
            .await
            .expect("loud wav should measure");
        let (m_quiet, src_quiet) = measure_file_loudness(&quiet_wav, true)
            .await
            .expect("quiet wav should measure");
        assert_eq!(src_loud, "ebur128");
        assert_eq!(src_quiet, "ebur128");
        // Difference between -6dB and -18dB tones must be ~12 LUFS.
        let diff = m_loud.lufs - m_quiet.lufs;
        assert!(
            (diff - 12.0).abs() < 0.6,
            "expected ~12 LUFS difference, got {} (loud={}, quiet={})",
            diff,
            m_loud.lufs,
            m_quiet.lufs
        );

        let hires_flac = dir.join("hires96k.flac");
        let ok_hires = Command::new("ffmpeg")
            .args([
                "-y",
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=1000:duration=2:sample_rate=96000,volume=-6dB",
                "-sample_fmt",
                "s32",
                "-c:a",
                "flac",
                &hires_flac.to_string_lossy(),
            ])
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false);
        assert!(ok_hires, "ffmpeg failed generating 96kHz test flac");
        let (m_hires, src_hires) = measure_file_loudness(&hires_flac, true)
            .await
            .expect("96kHz hires flac should measure");
        assert_eq!(src_hires, "ebur128");
        assert!(
            (m_hires.lufs - m_loud.lufs).abs() < 0.6,
            "96kHz FLAC loudness should match 48kHz WAV (hires={}, wav={})",
            m_hires.lufs,
            m_loud.lufs
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
