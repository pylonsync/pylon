//! TOTP (RFC 6238) — time-based one-time passwords for two-factor auth.
//!
//! Standard 6-digit, 30-second window, HMAC-SHA1 — the format every
//! authenticator app expects (Google Authenticator, 1Password, Authy,
//! Bitwarden, Apple Passwords, etc.). Verification accepts the
//! current window plus ±1 window of clock drift, matching the de
//! facto standard tolerance.
//!
//! Wire format:
//!   - **Secret**: 20 random bytes, base32-encoded (no padding) for
//!     the QR/provisioning URL. Authenticator apps consume base32
//!     uppercase alphanumeric — no `=` padding.
//!   - **Provisioning URL**: `otpauth://totp/<issuer>:<account>?secret=<base32>&issuer=<issuer>`
//!     — what you encode into a QR code or pass to the user's app
//!     via deep link.
//!
//! Storage shape — pylon stores ONE secret per user along with a
//! `verified: bool` flag. Enrollment is two-step: generate secret +
//! show QR, then user posts a code to confirm they scanned it. Only
//! after confirmation does TOTP gate subsequent logins.
//!
//! See `crates/router/src/routes/auth.rs` for the endpoints:
//!   - POST /api/auth/totp/enroll      → returns secret + URL (NOT verified yet)
//!   - POST /api/auth/totp/verify      → confirm enrollment with first code
//!   - POST /api/auth/totp/disable     → revoke (requires current code)
//!   - POST /api/auth/totp/challenge   → step 2 of login when 2FA enrolled

use hmac::{Hmac, Mac};
use sha1::Sha1;
use std::time::{SystemTime, UNIX_EPOCH};

type HmacSha1 = Hmac<Sha1>;

/// 30-second window per RFC 6238 — the universally implemented choice.
pub const TOTP_PERIOD_SECS: u64 = 30;

/// 6 digits per RFC 6238 — what every authenticator app shows.
pub const TOTP_DIGITS: u32 = 6;

/// Generate a fresh TOTP secret (20 random bytes — RFC 4226 §4
/// recommends ≥ 128 bits; 160 is the SHA-1 block size and the
/// industry default).
pub fn generate_secret() -> Vec<u8> {
    use rand::RngCore;
    let mut bytes = vec![0u8; 20];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes
}

/// Encode a secret into the base32 form authenticator apps expect.
/// RFC 4648 base32 alphabet (uppercase A-Z + 2-7), NO padding.
pub fn base32_encode(bytes: &[u8]) -> String {
    const ALPHA: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut out = String::with_capacity((bytes.len() * 8 + 4) / 5);
    let mut buf: u32 = 0;
    let mut bits: u8 = 0;
    for &b in bytes {
        buf = (buf << 8) | b as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            let idx = ((buf >> bits) & 0x1F) as usize;
            out.push(ALPHA[idx] as char);
        }
    }
    if bits > 0 {
        let idx = ((buf << (5 - bits)) & 0x1F) as usize;
        out.push(ALPHA[idx] as char);
    }
    out
}

/// Decode a base32 string back to bytes. Tolerates lowercase + `=`
/// padding so users can paste a secret in either form.
pub fn base32_decode(input: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(input.len() * 5 / 8);
    let mut buf: u32 = 0;
    let mut bits: u8 = 0;
    for ch in input.chars() {
        if ch == '=' || ch.is_whitespace() {
            continue;
        }
        let v = match ch.to_ascii_uppercase() {
            c @ 'A'..='Z' => (c as u32) - ('A' as u32),
            c @ '2'..='7' => (c as u32) - ('2' as u32) + 26,
            c => return Err(format!("base32: illegal char {c:?}")),
        };
        buf = (buf << 5) | v;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xFF) as u8);
        }
    }
    Ok(out)
}

/// Build the provisioning URL the authenticator app consumes.
/// `account` is typically the user's email; `issuer` is the app
/// name. Both are URL-encoded so spaces / special chars work.
///
/// Format: `otpauth://totp/<issuer>:<account>?secret=<base32>&issuer=<issuer>&algorithm=SHA1&digits=6&period=30`
pub fn provisioning_url(issuer: &str, account: &str, secret_b32: &str) -> String {
    let issuer_enc = url_encode(issuer);
    let account_enc = url_encode(account);
    format!(
        "otpauth://totp/{issuer_enc}:{account_enc}?secret={secret_b32}&issuer={issuer_enc}&algorithm=SHA1&digits=6&period=30"
    )
}

/// Compute the TOTP code for a given secret + Unix-epoch second.
/// Pure function — no clock access, so tests can pin the time.
pub fn compute_at(secret: &[u8], unix_seconds: u64) -> String {
    let counter = unix_seconds / TOTP_PERIOD_SECS;
    hotp(secret, counter, TOTP_DIGITS)
}

/// Compute the current TOTP code (uses system clock).
pub fn compute_now(secret: &[u8]) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    compute_at(secret, now)
}

/// Verify a code against the current window ± 1 step (90s of drift
/// tolerance total). Constant-time comparison so a wrong-byte-at-
/// position-N attacker can't time-side-channel the right code.
///
/// Returns `true` iff the code matches the current, previous, or
/// next window.
pub fn verify_now(secret: &[u8], code: &str) -> bool {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    verify_at(secret, code, now, 1)
}

/// Verify with explicit time + window-tolerance for tests / replay
/// detection. `window` is the number of ±steps to allow (typically 1).
pub fn verify_at(secret: &[u8], code: &str, unix_seconds: u64, window: i64) -> bool {
    let counter = (unix_seconds / TOTP_PERIOD_SECS) as i64;
    for delta in -window..=window {
        let c = (counter + delta).max(0) as u64;
        let expected = hotp(secret, c, TOTP_DIGITS);
        if crate::constant_time_eq(expected.as_bytes(), code.as_bytes()) {
            return true;
        }
    }
    false
}

/// HOTP (RFC 4226) — the building block TOTP wraps. Public so apps
/// that want raw HOTP (counter-based) can use it directly.
pub fn hotp(secret: &[u8], counter: u64, digits: u32) -> String {
    let mut mac = HmacSha1::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(&counter.to_be_bytes());
    let result = mac.finalize().into_bytes();
    // RFC 4226 §5.3 — dynamic truncation.
    let offset = (result[result.len() - 1] & 0x0f) as usize;
    let bin = ((result[offset] as u32 & 0x7f) << 24)
        | ((result[offset + 1] as u32) << 16)
        | ((result[offset + 2] as u32) << 8)
        | (result[offset + 3] as u32);
    let code = bin % 10u32.pow(digits);
    format!("{:0>width$}", code, width = digits as usize)
}

fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 4226 Appendix D test vector — secret = "12345678901234567890",
    /// counter sequence 0..10, expected codes are well-known.
    #[test]
    fn hotp_matches_rfc4226_vectors() {
        let secret = b"12345678901234567890";
        let expected = [
            "755224", "287082", "359152", "969429", "338314",
            "254676", "287922", "162583", "399871", "520489",
        ];
        for (i, want) in expected.iter().enumerate() {
            assert_eq!(hotp(secret, i as u64, 6), *want, "counter {i}");
        }
    }

    /// RFC 6238 Appendix B vectors — TOTP at fixed seconds.
    /// Secret = "12345678901234567890" (SHA-1 variant), digits = 8.
    #[test]
    fn totp_matches_rfc6238_vectors() {
        let secret = b"12345678901234567890";
        // (epoch_secs, expected_8_digit_code)
        for (t, want) in [(59u64, "94287082"), (1111111109, "07081804"), (1234567890, "89005924")] {
            assert_eq!(hotp(secret, t / 30, 8), want);
        }
    }

    #[test]
    fn base32_round_trip() {
        for raw in [
            &b""[..],
            &b"a"[..],
            &b"hello"[..],
            &b"\x00\xff\xa5\x5a\x12\x34\x56\x78\x9a\xbc"[..],
        ] {
            let enc = base32_encode(raw);
            // RFC 4648 base32 alphabet only.
            assert!(enc.chars().all(|c| c.is_ascii_uppercase() || ('2'..='7').contains(&c)));
            let dec = base32_decode(&enc).expect("decode");
            assert_eq!(dec, raw);
        }
    }

    #[test]
    fn base32_decode_tolerates_padding_and_lowercase() {
        let enc = base32_encode(b"hello world");
        let lower = enc.to_ascii_lowercase();
        let with_pad = format!("{enc}====");
        assert_eq!(base32_decode(&lower).unwrap(), b"hello world");
        assert_eq!(base32_decode(&with_pad).unwrap(), b"hello world");
    }

    #[test]
    fn verify_at_accepts_current_window() {
        let secret = generate_secret();
        let t = 1_700_000_000;
        let code = compute_at(&secret, t);
        assert!(verify_at(&secret, &code, t, 1));
    }

    #[test]
    fn verify_at_accepts_one_step_drift() {
        let secret = generate_secret();
        let t = 1_700_000_000;
        let code = compute_at(&secret, t);
        // Code from window N must validate at windows N-1 and N+1.
        assert!(verify_at(&secret, &code, t + 30, 1));
        assert!(verify_at(&secret, &code, t.saturating_sub(30), 1));
        // But NOT at window N+2 (60s drift).
        assert!(!verify_at(&secret, &code, t + 60, 1));
    }

    #[test]
    fn verify_at_rejects_wrong_code() {
        let secret = generate_secret();
        let t = 1_700_000_000;
        assert!(!verify_at(&secret, "000000", t, 1));
        assert!(!verify_at(&secret, "999999", t, 1));
        assert!(!verify_at(&secret, "", t, 1));
    }

    #[test]
    fn provisioning_url_encodes_special_chars() {
        let url = provisioning_url("My App", "user+tag@example.com", "JBSWY3DPEHPK3PXP");
        assert!(url.starts_with("otpauth://totp/My%20App:user%2Btag%40example.com?"));
        assert!(url.contains("secret=JBSWY3DPEHPK3PXP"));
        assert!(url.contains("issuer=My%20App"));
        assert!(url.contains("algorithm=SHA1"));
        assert!(url.contains("digits=6"));
        assert!(url.contains("period=30"));
    }
}
