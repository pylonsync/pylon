//! Signed file-download URLs — `ctx.files.signedUrl(fileId, {ttlSecs})`.
//!
//! `GET /api/files/<id>` is owner-or-unscoped-admin only, which makes
//! cross-user reads impossible even when the APP's own authorization says
//! they should happen (an organizer reviewing a speaker's upload). The
//! supported escape hatch: a server function mints a short-lived HMAC-signed
//! URL — `/api/files/<id>?sig=<hex>&exp=<unix>` — and the GET handler honors
//! a valid, unexpired signature as an alternative to the owner check. The
//! app wraps the mint in a membership-gated function, so authorization stays
//! app-policy-driven and the endpoint stays closed to enumeration (no
//! signature → the owner check applies exactly as before).
//!
//! Signature: HMAC-SHA256 over `"file:{asset_id}:{exp}"`, hex-encoded.
//! Storage-backend agnostic — verification happens before the backend
//! dispatch, so local streams and CDN backends 302 exactly as they do for
//! owner reads.

use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

pub const DEFAULT_TTL_SECS: u64 = 300;
/// Upper bound on requested TTLs (24h). A signed URL is a bearer
/// capability; unbounded lifetimes turn a leaked link into a permanent
/// grant.
pub const MAX_TTL_SECS: u64 = 86_400;

/// The signing secret, resolved once per process:
///  1. `PYLON_JWT_SECRET` — reuse the deployment's existing HMAC secret.
///  2. `PYLON_ENCRYPTION_KEY` — present on deployments with encrypted
///     fields / connections.
///  3. A random secret persisted at `.pylon/file-url-secret` (created
///     0600 on first use) so URLs survive restarts — the same cwd-rooted
///     `.pylon/` dir the ISR cache uses, which lives on the app volume in
///     cloud deployments.
///  4. Last resort (unwritable filesystem): a random per-process secret —
///     URLs die on restart, but signing still works.
pub fn signing_secret() -> &'static [u8] {
    static CELL: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    CELL.get_or_init(|| {
        for var in ["PYLON_JWT_SECRET", "PYLON_ENCRYPTION_KEY"] {
            if let Ok(v) = std::env::var(var) {
                if !v.is_empty() {
                    return v.into_bytes();
                }
            }
        }
        let path = std::path::Path::new(".pylon").join("file-url-secret");
        if let Ok(existing) = std::fs::read(&path) {
            if existing.len() >= 32 {
                return existing;
            }
        }
        let fresh: Vec<u8> = {
            use ring::rand::SecureRandom;
            let mut buf = vec![0u8; 32];
            // Fill errors only on catastrophic RNG failure; fall back to a
            // time-derived value rather than a fixed one.
            if ring::rand::SystemRandom::new().fill(&mut buf).is_err() {
                buf = format!("{:?}", std::time::SystemTime::now()).into_bytes();
            }
            buf
        };
        if std::fs::create_dir_all(".pylon").is_ok() && std::fs::write(&path, &fresh).is_ok() {
            let _ = pylon_kernel::secret_file::restrict_to_owner(&path);
        }
        fresh
    })
}

fn mac_hex(secret: &[u8], asset_id: &str, exp: u64) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(format!("file:{asset_id}:{exp}").as_bytes());
    mac.finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Mint the signed path for an asset. `ttl_secs` is clamped to
/// [1, MAX_TTL_SECS]; `None` → DEFAULT_TTL_SECS.
pub fn signed_path_with(secret: &[u8], asset_id: &str, ttl_secs: Option<u64>, now: u64) -> String {
    let ttl = ttl_secs.unwrap_or(DEFAULT_TTL_SECS).clamp(1, MAX_TTL_SECS);
    let exp = now.saturating_add(ttl);
    let sig = mac_hex(secret, asset_id, exp);
    format!("/api/files/{asset_id}?sig={sig}&exp={exp}")
}

pub fn signed_path(asset_id: &str, ttl_secs: Option<u64>) -> String {
    signed_path_with(signing_secret(), asset_id, ttl_secs, unix_now())
}

/// Verify a presented signature. Constant-time comparison; an expired,
/// malformed, or mismatched signature is simply "not verified" — callers
/// fall through to the ordinary owner check rather than erroring, so a
/// stale link from an owner still serves.
pub fn verify_with(secret: &[u8], asset_id: &str, exp_raw: &str, sig_hex: &str, now: u64) -> bool {
    let Ok(exp) = exp_raw.parse::<u64>() else {
        return false;
    };
    if exp <= now {
        return false;
    }
    let expected = mac_hex(secret, asset_id, exp);
    // Constant-time equality — comparing the freshly computed MAC against
    // the presented hex with early-exit would leak prefix length.
    let a = expected.as_bytes();
    let b = sig_hex.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

pub fn verify(asset_id: &str, exp_raw: &str, sig_hex: &str) -> bool {
    verify_with(signing_secret(), asset_id, exp_raw, sig_hex, unix_now())
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Extract `sig` and `exp` from a raw query string (the part after `?`).
pub fn sig_params(query: &str) -> Option<(String, String)> {
    let mut sig = None;
    let mut exp = None;
    for pair in query.split('&') {
        if let Some(v) = pair.strip_prefix("sig=") {
            sig = Some(v.to_string());
        } else if let Some(v) = pair.strip_prefix("exp=") {
            exp = Some(v.to_string());
        }
    }
    match (sig, exp) {
        (Some(s), Some(e)) => Some((s, e)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &[u8] = b"test-secret-please-ignore";

    #[test]
    fn round_trip_verifies_until_expiry() {
        let path = signed_path_with(SECRET, "1700000000-slides.pdf", Some(300), 1_000);
        let query = path.split('?').nth(1).unwrap();
        let (sig, exp) = sig_params(query).unwrap();
        assert_eq!(exp, "1300");
        assert!(verify_with(
            SECRET,
            "1700000000-slides.pdf",
            &exp,
            &sig,
            1_299
        ));
        assert!(
            !verify_with(SECRET, "1700000000-slides.pdf", &exp, &sig, 1_300),
            "exactly-at-expiry must be rejected"
        );
        assert!(!verify_with(
            SECRET,
            "1700000000-slides.pdf",
            &exp,
            &sig,
            2_000
        ));
    }

    #[test]
    fn tampering_with_any_component_fails() {
        let path = signed_path_with(SECRET, "a.png", Some(60), 1_000);
        let (sig, exp) = sig_params(path.split('?').nth(1).unwrap()).unwrap();
        // Different asset — the leaked-signature-reuse attack.
        assert!(!verify_with(SECRET, "b.png", &exp, &sig, 1_001));
        // Forged longer expiry with the old signature.
        assert!(!verify_with(SECRET, "a.png", "9999999", &sig, 1_001));
        // Wrong secret (e.g. another deployment's URL replayed here).
        assert!(!verify_with(b"other-secret", "a.png", &exp, &sig, 1_001));
        // Truncated / padded signature.
        assert!(!verify_with(SECRET, "a.png", &exp, &sig[..10], 1_001));
        assert!(!verify_with(
            SECRET,
            "a.png",
            &exp,
            &format!("{sig}00"),
            1_001
        ));
        // Garbage expiry.
        assert!(!verify_with(SECRET, "a.png", "not-a-number", &sig, 1_001));
    }

    #[test]
    fn ttl_is_clamped_to_the_max() {
        let path = signed_path_with(SECRET, "a", Some(10 * MAX_TTL_SECS), 0);
        let (_, exp) = sig_params(path.split('?').nth(1).unwrap()).unwrap();
        assert_eq!(exp.parse::<u64>().unwrap(), MAX_TTL_SECS);
    }

    #[test]
    fn sig_params_requires_both() {
        assert!(sig_params("sig=abc").is_none());
        assert!(sig_params("exp=12").is_none());
        assert!(sig_params("sig=abc&exp=12").is_some());
        assert!(sig_params("exp=12&sig=abc&other=1").is_some());
    }
}
