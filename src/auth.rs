//! Authentication helpers shared by the web UI and store.
//!
//! Everything here is deliberately dependency-light except the WebAuthn
//! implementation: TOTP and recovery codes are built on the same SHA/HMAC
//! primitives already used elsewhere, so a self-hosted install never needs an
//! external identity provider.

use crate::models::{constant_time_eq, err_msg, AppResult};
use base64::Engine;
use chrono::Utc;
use hmac::{Hmac, Mac};
use qrcode::render::svg;
use qrcode::QrCode;
use rand::RngCore;
use sha1::Sha1;
use sha2::{Digest, Sha256};
use std::sync::OnceLock;
use webauthn_rs::prelude::*;

/// Seconds per TOTP step (RFC 6238 default).
pub const TOTP_PERIOD: i64 = 30;
/// Number of one-time recovery codes handed out when 2FA is set up.
pub const RECOVERY_CODE_COUNT: usize = 10;
/// How long an MFA check remains valid for step-up operations (10 minutes).
pub const MFA_WINDOW_SECS: i64 = 10 * 60;
/// Deleted accounts stay recoverable for seven days.
pub const RESTORE_WINDOW_SECS: i64 = 7 * 24 * 60 * 60;
/// Lifetime of a pending login / restore challenge.
pub const LOGIN_CHALLENGE_TTL: i64 = 5 * 60;
/// Lifetime of a WebAuthn registration / assertion challenge.
pub const WEBAUTHN_CHALLENGE_TTL: i64 = 5 * 60;

// ---------------------------------------------------------------------------
// Random tokens & hashing
// ---------------------------------------------------------------------------

pub fn random_bytes(len: usize) -> Vec<u8> {
    let mut buf = vec![0u8; len];
    rand::thread_rng().fill_bytes(&mut buf);
    buf
}

/// URL-safe, high-entropy token used for sessions, challenges and restore links.
pub fn random_token() -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random_bytes(32))
}

/// One-way hash for values that must be recoverable by comparison only
/// (recovery codes and restore tokens).
pub fn hash_token(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

// ---------------------------------------------------------------------------
// Password hashing (Argon2id)
// ---------------------------------------------------------------------------

const ARGON2_PREFIX: &str = "$argon2";

fn argon2_hasher() -> argon2::Argon2<'static> {
    use argon2::{Algorithm, Argon2, Params, Version};
    // OWASP's recommended Argon2id baseline (19 MiB, 2 passes). Tests use a
    // tiny cost so the suite stays fast; the format is identical.
    #[cfg(not(test))]
    let params = Params::new(19 * 1024, 2, 1, None);
    #[cfg(test)]
    let params = Params::new(256, 1, 1, None);
    Argon2::new(
        Algorithm::Argon2id,
        Version::V0x13,
        params.expect("valid Argon2 parameters"),
    )
}

/// Argon2 runs allowed at once. Each one takes ~19 MiB and a full core, so a
/// burst of sign-ins must queue rather than exhaust a small VM's memory.
const MAX_CONCURRENT_HASHES: usize = 2;

static HASH_SLOTS: OnceLock<(std::sync::Mutex<usize>, std::sync::Condvar)> = OnceLock::new();

/// Runs `f` while holding one of the limited hashing slots (blocking until a
/// slot frees up). Only call from blocking contexts (`spawn_blocking`, startup).
fn with_hash_slot<T>(f: impl FnOnce() -> T) -> T {
    let (lock, cvar) = HASH_SLOTS.get_or_init(|| (std::sync::Mutex::new(0), std::sync::Condvar::new()));
    {
        let mut busy = lock.lock().unwrap_or_else(|p| p.into_inner());
        while *busy >= MAX_CONCURRENT_HASHES {
            busy = cvar.wait(busy).unwrap_or_else(|p| p.into_inner());
        }
        *busy += 1;
    }
    struct Release;
    impl Drop for Release {
        fn drop(&mut self) {
            if let Some((lock, cvar)) = HASH_SLOTS.get() {
                let mut busy = lock.lock().unwrap_or_else(|p| p.into_inner());
                *busy = busy.saturating_sub(1);
                cvar.notify_one();
            }
        }
    }
    let _release = Release;
    f()
}

/// Hashes a password into a self-describing PHC string (`$argon2id$…`).
pub fn hash_password(password: &str) -> AppResult<String> {
    use argon2::password_hash::{rand_core::OsRng, PasswordHasher, SaltString};
    let salt = SaltString::generate(&mut OsRng);
    with_hash_slot(|| {
        argon2_hasher()
            .hash_password(password.as_bytes(), &salt)
            .map(|h| h.to_string())
            .map_err(|e| err_msg(format!("Could not hash password: {e}")))
    })
}

/// Burns the same Argon2 work as a real check when the username doesn't
/// exist, so response time doesn't reveal which accounts exist.
pub fn verify_password_dummy(password: &str) {
    static DUMMY: OnceLock<String> = OnceLock::new();
    let hash = DUMMY.get_or_init(|| hash_password("beatlab-timing-equaliser").unwrap_or_default());
    let _ = verify_password(hash, password);
}

pub fn is_password_hash(stored: &str) -> bool {
    stored.starts_with(ARGON2_PREFIX)
}

/// Checks a password against a stored Argon2 hash. Rows that predate hashing
/// (plaintext) are compared in constant time; startup migrates them anyway.
pub fn verify_password(stored: &str, password: &str) -> bool {
    use argon2::password_hash::{PasswordHash, PasswordVerifier};
    if !is_password_hash(stored) {
        return !stored.is_empty() && constant_time_eq(stored, password);
    }
    match PasswordHash::new(stored) {
        Ok(parsed) => with_hash_slot(|| {
            argon2_hasher()
                .verify_password(password.as_bytes(), &parsed)
                .is_ok()
        }),
        Err(_) => false,
    }
}

// ---------------------------------------------------------------------------
// Failed-attempt rate limiting
// ---------------------------------------------------------------------------

/// Window over which failed attempts are counted.
pub const RATE_WINDOW_SECS: i64 = 15 * 60;
/// Failed sign-ins allowed per account in a window before it is throttled.
pub const MAX_FAILURES_PER_ACCOUNT: u32 = 10;
/// Failed attempts allowed per client IP in a window (covers many accounts).
pub const MAX_FAILURES_PER_IP: u32 = 50;

static FAILURES: OnceLock<std::sync::Mutex<std::collections::HashMap<String, (u32, i64)>>> =
    OnceLock::new();

fn failures() -> std::sync::MutexGuard<'static, std::collections::HashMap<String, (u32, i64)>> {
    FAILURES
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

/// Rate-limit keys for one attempt: the account (when known) and the client IP
/// (when known). Empty parts are skipped.
pub fn rate_keys(scope: &str, account: &str, ip: &str) -> Vec<String> {
    let mut keys = Vec::with_capacity(2);
    let account = account.trim().to_lowercase();
    if !account.is_empty() {
        keys.push(format!("{scope}:acct:{account}"));
    }
    if !ip.trim().is_empty() {
        keys.push(format!("{scope}:ip:{}", ip.trim()));
    }
    keys
}

fn limit_for(key: &str) -> u32 {
    if key.contains(":ip:") {
        MAX_FAILURES_PER_IP
    } else {
        MAX_FAILURES_PER_ACCOUNT
    }
}

/// Seconds until another attempt is allowed, or `None` when not throttled.
pub fn throttled_for(keys: &[String]) -> Option<i64> {
    let now = Utc::now().timestamp();
    let map = failures();
    keys.iter()
        .filter_map(|k| {
            let (count, start) = *map.get(k)?;
            let remaining = start + RATE_WINDOW_SECS - now;
            (remaining > 0 && count >= limit_for(k)).then_some(remaining)
        })
        .max()
}

pub fn record_failure(keys: &[String]) {
    let now = Utc::now().timestamp();
    let mut map = failures();
    if map.len() > 50_000 {
        map.retain(|_, (_, start)| now - *start < RATE_WINDOW_SECS);
    }
    for key in keys {
        let entry = map.entry(key.clone()).or_insert((0, now));
        if now - entry.1 >= RATE_WINDOW_SECS {
            *entry = (0, now);
        }
        entry.0 = entry.0.saturating_add(1);
    }
}

/// A successful sign-in clears the account's counter (not the IP's).
pub fn record_success(keys: &[String]) {
    let mut map = failures();
    for key in keys.iter().filter(|k| !k.contains(":ip:")) {
        map.remove(key);
    }
}

/// Human-readable message for a throttled request.
pub fn throttle_message(retry_after: i64) -> String {
    let mins = (retry_after + 59) / 60;
    format!(
        "Too many failed attempts. Try again in {} minute{}.",
        mins.max(1),
        if mins == 1 { "" } else { "s" }
    )
}

// ---------------------------------------------------------------------------
// Base32 (RFC 4648) — used by every TOTP authenticator app
// ---------------------------------------------------------------------------

const BASE32_ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

pub fn base32_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity((data.len() * 8 + 4) / 5);
    let mut bits: u32 = 0;
    let mut count = 0u32;
    for &byte in data {
        bits = (bits << 8) | byte as u32;
        count += 8;
        while count >= 5 {
            count -= 5;
            out.push(BASE32_ALPHABET[((bits >> count) & 0x1f) as usize] as char);
        }
    }
    if count > 0 {
        out.push(BASE32_ALPHABET[((bits << (5 - count)) & 0x1f) as usize] as char);
    }
    out
}

pub fn base32_decode(input: &str) -> AppResult<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len() * 5 / 8);
    let mut bits: u32 = 0;
    let mut count = 0u32;
    for raw in input.bytes() {
        let ch = raw.to_ascii_uppercase();
        if ch == b'=' || ch == b' ' || ch == b'-' {
            continue;
        }
        let value = BASE32_ALPHABET
            .iter()
            .position(|&c| c == ch)
            .ok_or_else(|| err_msg("Invalid base32 secret"))? as u32;
        bits = (bits << 5) | value;
        count += 5;
        if count >= 8 {
            count -= 8;
            out.push((bits >> count) as u8);
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// TOTP (RFC 6238 — SHA-1, 6 digits, 30 second period)
// ---------------------------------------------------------------------------

pub fn generate_totp_secret() -> String {
    base32_encode(&random_bytes(20))
}

pub fn totp_code(secret: &str, step: i64) -> AppResult<String> {
    let key = base32_decode(secret)?;
    if key.is_empty() {
        return Err(err_msg("Invalid authenticator secret"));
    }
    let mut mac = Hmac::<Sha1>::new_from_slice(&key)
        .map_err(|_| err_msg("Could not initialise authenticator"))?;
    mac.update(&step.to_be_bytes());
    let digest = mac.finalize().into_bytes();
    let offset = (digest[19] & 0x0f) as usize;
    let value = ((digest[offset] as u32 & 0x7f) << 24)
        | ((digest[offset + 1] as u32) << 16)
        | ((digest[offset + 2] as u32) << 8)
        | (digest[offset + 3] as u32);
    Ok(format!("{:06}", value % 1_000_000))
}

/// Verify a 6-digit code within a ±1 step window. Returns the accepted step so
/// the caller can reject replays of the same code.
pub fn verify_totp(secret: &str, code: &str, last_step: i64) -> AppResult<Option<i64>> {
    let cleaned: String = code
        .trim()
        .chars()
        .filter(|c| c.is_ascii_digit())
        .collect();
    if cleaned.len() != 6 {
        return Ok(None);
    }
    let now_step = Utc::now().timestamp() / TOTP_PERIOD;
    for step in [now_step, now_step - 1, now_step + 1] {
        if step <= last_step {
            continue;
        }
        let expected = totp_code(secret, step)?;
        if constant_time_eq(&expected, &cleaned) {
            return Ok(Some(step));
        }
    }
    Ok(None)
}

/// `otpauth://` URI consumed by Google Authenticator, 1Password, Authy, ….
pub fn otpauth_uri(issuer: &str, username: &str, secret: &str) -> String {
    let raw_label = format!("{issuer}:{username}");
    let label = urlencoding::encode(&raw_label);
    let issuer_enc = urlencoding::encode(issuer);
    format!("otpauth://totp/{label}?secret={secret}&issuer={issuer_enc}&algorithm=SHA1&digits=6&period=30")
}

/// Renders a scannable QR code as inline SVG (no third-party CDN required).
pub fn qr_svg(text: &str) -> AppResult<String> {
    let code = QrCode::new(text.as_bytes()).map_err(|e| err_msg(format!("QR code: {e}")))?;
    let mut canvas = code.render::<svg::Color>();
    canvas
        .dark_color(svg::Color("#0f1117"))
        .light_color(svg::Color("#ffffff"))
        .min_dimensions(220, 220);
    Ok(canvas.build())
}

// ---------------------------------------------------------------------------
// Single-use recovery codes
// ---------------------------------------------------------------------------

const RECOVERY_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

pub fn generate_recovery_codes(count: usize) -> Vec<String> {
    (0..count)
        .map(|_| {
            let raw = random_bytes(16);
            let chars: String = raw
                .iter()
                .map(|b| RECOVERY_ALPHABET[*b as usize % RECOVERY_ALPHABET.len()] as char)
                .collect();
            format!(
                "{}-{}-{}-{}",
                &chars[0..4],
                &chars[4..8],
                &chars[8..12],
                &chars[12..16]
            )
        })
        .collect()
}

pub fn normalize_recovery_code(code: &str) -> String {
    code.trim()
        .to_ascii_uppercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect()
}

pub fn hash_recovery_code(code: &str) -> String {
    hash_token(&normalize_recovery_code(code))
}

// ---------------------------------------------------------------------------
// WebAuthn / passkeys
// ---------------------------------------------------------------------------

static WEBAUTHN: OnceLock<Option<Webauthn>> = OnceLock::new();

/// Configure the relying party from the public origin. Call once at startup.
/// Returns false (and leaves passkeys disabled) for unsupported hosts such as
/// bare IPs — WebAuthn requires a real domain and a secure context.
pub fn init_webauthn(origin_raw: &str) -> bool {
    let built = build_webauthn(origin_raw);
    let _ = WEBAUTHN.set(built);
    webauthn().is_some()
}

fn build_webauthn(origin_raw: &str) -> Option<Webauthn> {
    let mut raw = origin_raw.trim().to_string();
    if !raw.contains("://") {
        raw = format!("https://{raw}");
    }
    let origin = Url::parse(&raw).ok()?;
    let rp_id = origin.host_str()?.to_string();
    // WebauthnBuilder validates that rp_id is an effective domain of origin and
    // rejects IP literals, which is exactly what we want here.
    WebauthnBuilder::new(&rp_id, &origin)
        .ok()?
        .rp_name("Beatlab")
        .build()
        .ok()
}

pub fn webauthn() -> Option<&'static Webauthn> {
    WEBAUTHN.get().and_then(|value| value.as_ref())
}

fn credential_id_string(id: &CredentialID) -> String {
    serde_json::to_string(id)
        .unwrap_or_else(|_| "{}".to_string())
        .trim_matches('"')
        .to_string()
}

pub fn passkey_credential_id(passkey: &Passkey) -> String {
    credential_id_string(passkey.cred_id())
}

pub fn assertion_credential_id(result: &AuthenticationResult) -> String {
    credential_id_string(result.cred_id())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_hashes_verify_and_legacy_plaintext_still_works() {
        let hash = hash_password("hunter2").unwrap();
        assert!(hash.starts_with("$argon2id$"));
        assert_ne!(hash, hash_password("hunter2").unwrap(), "salted");
        assert!(verify_password(&hash, "hunter2"));
        assert!(!verify_password(&hash, "hunter3"));
        assert!(verify_password("legacy-plain", "legacy-plain"));
        assert!(!verify_password("legacy-plain", "nope"));
        assert!(!verify_password("", ""));
    }

    #[test]
    fn concurrent_hashing_is_bounded_and_never_deadlocks() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static RUNNING: AtomicUsize = AtomicUsize::new(0);
        static PEAK: AtomicUsize = AtomicUsize::new(0);
        let handles: Vec<_> = (0..8)
            .map(|i| {
                std::thread::spawn(move || {
                    with_hash_slot(|| {
                        let now = RUNNING.fetch_add(1, Ordering::SeqCst) + 1;
                        PEAK.fetch_max(now, Ordering::SeqCst);
                        std::thread::sleep(std::time::Duration::from_millis(20));
                        RUNNING.fetch_sub(1, Ordering::SeqCst);
                    });
                    let h = hash_password(&format!("pw{i}")).unwrap();
                    assert!(verify_password(&h, &format!("pw{i}")));
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert!(PEAK.load(Ordering::SeqCst) <= MAX_CONCURRENT_HASHES);
        verify_password_dummy("anything");
    }

    #[test]
    fn rate_limiter_throttles_after_limit_and_resets_on_success() {
        let keys = rate_keys("test-scope", "Limited-User", "");
        assert_eq!(keys, vec!["test-scope:acct:limited-user".to_string()]);
        for _ in 0..MAX_FAILURES_PER_ACCOUNT - 1 {
            record_failure(&keys);
        }
        assert!(throttled_for(&keys).is_none());
        record_failure(&keys);
        assert!(throttled_for(&keys).unwrap() > 0);
        record_success(&keys);
        assert!(throttled_for(&keys).is_none());
    }

    #[test]
    fn base32_roundtrip() {
        let raw = b"hello world secret";
        assert_eq!(base32_decode(&base32_encode(raw)).unwrap(), raw);
        assert_eq!(base32_encode(b"f"), "MY");
    }

    #[test]
    fn totp_codes_are_stable_and_windowed() {
        let secret = generate_totp_secret();
        let step = Utc::now().timestamp() / TOTP_PERIOD;
        let code = totp_code(&secret, step).unwrap();
        assert_eq!(code.len(), 6);
        assert_eq!(code, totp_code(&secret, step).unwrap());
        assert_eq!(verify_totp(&secret, &code, step - 1).unwrap(), Some(step));
        assert_eq!(verify_totp(&secret, &code, step).unwrap(), None);
        assert_eq!(verify_totp(&secret, "12345", 0).unwrap(), None);
    }

    #[test]
    fn recovery_codes_are_unique_and_normalised() {
        let codes = generate_recovery_codes(10);
        assert_eq!(codes.len(), 10);
        let mut seen = std::collections::HashSet::new();
        for code in &codes {
            assert_eq!(code.len(), 19);
            assert!(seen.insert(code));
            assert_eq!(hash_recovery_code(code), hash_recovery_code(&code.to_lowercase()));
        }
    }

    #[test]
    fn webauthn_accepts_localhost_and_rejects_ip_origins() {
        assert!(build_webauthn("http://localhost:4533").is_some());
        assert!(build_webauthn("https://music.example.com").is_some());
        assert!(build_webauthn("http://127.0.0.1:4533").is_none());
    }

    #[test]
    fn otpauth_uri_and_qr_are_generated() {
        let uri = otpauth_uri("Beatlab", "alice", "JBSWY3DPEHPK3PXP");
        assert!(uri.starts_with("otpauth://totp/Beatlab%3Aalice?"));
        let svg = qr_svg(&uri).unwrap();
        assert!(svg.contains("<svg"));
        assert!(svg.contains("</svg>"));
    }
}
