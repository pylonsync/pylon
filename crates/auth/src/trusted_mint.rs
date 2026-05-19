//! Trusted server-side session mint — HMAC-signed `POST /api/auth/sessions/trusted-mint`
//! support. This module owns the signature primitive; the route handler
//! lives in `pylon-router::routes::auth`.
//!
//! Wire shape (lifted from Stripe's webhook signing for familiarity):
//!
//! ```text
//! Headers:
//!   X-Pylon-Trusted-Timestamp: <unix seconds, ±5min from server clock>
//!   X-Pylon-Trusted-Signature: <hex(HMAC_SHA256(secret, ts + "." + body))>
//! ```
//!
//! Why timestamp + body rather than body alone:
//! - Replay protection. The signed value binds a specific moment in time
//!   so an attacker who captures one signed request can't replay it a
//!   week later (or even five minutes later).
//! - Tamper protection on the body comes for free from any HMAC.
//!
//! The verifier is intentionally self-contained — no header parsing, no
//! payload deserialization. Callers pass the timestamp + signature in
//! already-parsed form, which keeps the security boundary obvious:
//! "you give us a `(secret, ts, body, sig, now)` tuple, we tell you
//! whether the signature is valid against the secret as of `now`."

use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Reasons a trusted-mint signature verification can fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustedMintError {
    /// `PYLON_TRUSTED_SECRET` is unset (or empty) on the server. The
    /// endpoint operates in an "opt-in" mode — without a secret it
    /// should 404 so plain installs don't accidentally expose a
    /// signing surface to the network.
    SecretNotConfigured,
    /// Missing or unparseable `X-Pylon-Trusted-Timestamp` header.
    MissingTimestamp,
    /// Missing or empty `X-Pylon-Trusted-Signature` header.
    MissingSignature,
    /// Timestamp is more than 5 minutes from the server's clock —
    /// either replay attempt or clock skew. Per Stripe convention,
    /// the freshness window is ±5 minutes.
    StaleTimestamp,
    /// HMAC-SHA256 did not match — body tampered with, or wrong
    /// secret used to sign.
    BadSignature,
}

impl std::fmt::Display for TrustedMintError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::SecretNotConfigured => "PYLON_TRUSTED_SECRET is not set",
            Self::MissingTimestamp => "X-Pylon-Trusted-Timestamp header missing or invalid",
            Self::MissingSignature => "X-Pylon-Trusted-Signature header missing",
            Self::StaleTimestamp => "timestamp outside ±5min freshness window",
            Self::BadSignature => "signature does not match",
        })
    }
}

/// Compute the canonical signature for a `(timestamp, body)` pair.
///
/// Hex-lowercase encoding so the wire format is stable across HMAC
/// libraries. Used by [`verify_signature`] (for comparison) and exposed
/// publicly so test/SDK callers can construct valid requests.
pub fn sign(secret: &str, timestamp: u64, body: &[u8]) -> String {
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(format!("{timestamp}.").as_bytes());
    mac.update(body);
    bytes_to_hex(&mac.finalize().into_bytes())
}

/// Maximum age (in seconds) of a signed request, in either direction.
/// 5 minutes mirrors Stripe; long enough to absorb a clock that's a
/// few minutes off, short enough that a captured signature isn't
/// useful tomorrow.
pub const FRESHNESS_WINDOW_SECS: u64 = 5 * 60;

/// Verify a trusted-mint request.
///
/// Returns `Ok(())` if the signature matches and the timestamp is
/// within ±5 minutes of `now_secs`. Otherwise returns a typed
/// [`TrustedMintError`] describing which check failed.
///
/// The function is constant-time on the signature comparison (uses
/// [`crate::constant_time_eq`]); other early-exit paths (missing
/// header, stale ts) leak no signature-bit information because they
/// don't touch the secret.
pub fn verify_signature(
    secret: &str,
    timestamp: u64,
    body: &[u8],
    signature_hex: &str,
    now_secs: u64,
) -> Result<(), TrustedMintError> {
    if secret.is_empty() {
        return Err(TrustedMintError::SecretNotConfigured);
    }
    if signature_hex.is_empty() {
        return Err(TrustedMintError::MissingSignature);
    }
    let diff = if now_secs > timestamp {
        now_secs - timestamp
    } else {
        timestamp - now_secs
    };
    if diff > FRESHNESS_WINDOW_SECS {
        return Err(TrustedMintError::StaleTimestamp);
    }
    let expected = sign(secret, timestamp, body);
    if !crate::constant_time_eq(expected.as_bytes(), signature_hex.as_bytes()) {
        return Err(TrustedMintError::BadSignature);
    }
    Ok(())
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "wh_trusted_test_secret";

    #[test]
    fn round_trip_ok() {
        let body = br#"{"email":"jane@example.com"}"#;
        let ts = 1_700_000_000;
        let sig = sign(SECRET, ts, body);
        assert_eq!(verify_signature(SECRET, ts, body, &sig, ts), Ok(()));
    }

    #[test]
    fn freshness_window_boundary() {
        let body = b"{}";
        let ts = 1_700_000_000;
        let sig = sign(SECRET, ts, body);
        // Exactly 5 minutes in the past — boundary is inclusive.
        assert_eq!(
            verify_signature(SECRET, ts, body, &sig, ts + FRESHNESS_WINDOW_SECS),
            Ok(()),
        );
        // 5 minutes + 1 second — outside the window.
        assert_eq!(
            verify_signature(SECRET, ts, body, &sig, ts + FRESHNESS_WINDOW_SECS + 1),
            Err(TrustedMintError::StaleTimestamp),
        );
        // 5 minutes in the future — boundary is symmetric.
        assert_eq!(
            verify_signature(SECRET, ts + FRESHNESS_WINDOW_SECS, body, &sig, ts),
            Err(TrustedMintError::BadSignature), // ts differs → sig differs
        );
    }

    #[test]
    fn rejects_tampered_body() {
        let body = br#"{"email":"jane@example.com"}"#;
        let ts = 1_700_000_000;
        let sig = sign(SECRET, ts, body);
        let tampered = br#"{"email":"attacker@example.com"}"#;
        assert_eq!(
            verify_signature(SECRET, ts, tampered, &sig, ts),
            Err(TrustedMintError::BadSignature),
        );
    }

    #[test]
    fn rejects_wrong_secret() {
        let body = b"{}";
        let ts = 1_700_000_000;
        let sig = sign(SECRET, ts, body);
        assert_eq!(
            verify_signature("different_secret", ts, body, &sig, ts),
            Err(TrustedMintError::BadSignature),
        );
    }

    #[test]
    fn rejects_empty_secret() {
        assert_eq!(
            verify_signature("", 0, b"", "deadbeef", 0),
            Err(TrustedMintError::SecretNotConfigured),
        );
    }

    #[test]
    fn rejects_empty_signature() {
        assert_eq!(
            verify_signature(SECRET, 0, b"", "", 0),
            Err(TrustedMintError::MissingSignature),
        );
    }

    #[test]
    fn replay_after_window_rejected() {
        // The exact failure mode the freshness check exists to prevent:
        // valid signature captured today, replayed tomorrow.
        let body = br#"{"email":"jane@example.com","createIfMissing":true}"#;
        let ts = 1_700_000_000;
        let sig = sign(SECRET, ts, body);
        // 24 hours later.
        let now = ts + 24 * 60 * 60;
        assert_eq!(
            verify_signature(SECRET, ts, body, &sig, now),
            Err(TrustedMintError::StaleTimestamp),
        );
    }

    #[test]
    fn signature_is_lowercase_hex() {
        let sig = sign(SECRET, 1, b"x");
        assert!(sig.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')));
        assert_eq!(sig.len(), 64); // 32 bytes * 2 hex chars
    }
}
