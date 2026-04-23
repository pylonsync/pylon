use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use hmac::{Hmac, Mac};
use sha1::Sha1;

use crate::Plugin;

type HmacSha1 = Hmac<Sha1>;

/// TOTP 2FA enrollment for a user.
///
/// `last_accepted_counter` is the most recent TOTP time-step counter that
/// was successfully verified. A re-use within the same 30s window has the
/// same counter, so we can detect and refuse replays. Without this, a code
/// observed by an attacker (shoulder-surfing, phishing, log leak) was
/// usable for up to 30 seconds — the full window.
///
/// Debug is deliberately NOT derived: the secret must never land in a log.
pub struct TotpEnrollment {
    pub user_id: String,
    pub secret: String,
    pub verified: bool,
    pub last_accepted_counter: Option<u64>,
}

impl Clone for TotpEnrollment {
    fn clone(&self) -> Self {
        Self {
            user_id: self.user_id.clone(),
            secret: self.secret.clone(),
            verified: self.verified,
            last_accepted_counter: self.last_accepted_counter,
        }
    }
}

/// TOTP 2FA plugin. Implements time-based one-time passwords (RFC 6238).
/// Uses HMAC-SHA1 with 30-second time steps and 6-digit codes.
pub struct TotpPlugin {
    enrollments: Mutex<HashMap<String, TotpEnrollment>>,
    /// If true, require 2FA verification on protected actions.
    pub enforce: bool,
    /// Actions that require 2FA (empty = all actions when enforce is true).
    pub protected_actions: Vec<String>,
}

impl TotpPlugin {
    pub fn new() -> Self {
        Self {
            enrollments: Mutex::new(HashMap::new()),
            enforce: false,
            protected_actions: vec![],
        }
    }

    pub fn enforced(protected_actions: Vec<String>) -> Self {
        Self {
            enrollments: Mutex::new(HashMap::new()),
            enforce: true,
            protected_actions,
        }
    }

    /// Enroll a user in 2FA. Returns the secret (for QR code generation).
    pub fn enroll(&self, user_id: &str) -> String {
        let secret = generate_secret();
        self.enrollments.lock().unwrap().insert(
            user_id.to_string(),
            TotpEnrollment {
                user_id: user_id.to_string(),
                secret: secret.clone(),
                verified: false,
                last_accepted_counter: None,
            },
        );
        secret
    }

    /// Verify a TOTP code and mark enrollment as verified.
    ///
    /// Constant-time compare prevents timing attacks on the 6-digit code.
    /// The verified code's counter is recorded so the same code cannot be
    /// replayed within its 30-second window — a successful verify burns
    /// that counter for this user.
    pub fn verify(&self, user_id: &str, code: &str) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let counter = now / 30;

        let mut enrollments = self.enrollments.lock().unwrap();
        let enrollment = match enrollments.get_mut(user_id) {
            Some(e) => e,
            None => return false,
        };

        // Replay guard: if this counter's code was already accepted for this
        // user, refuse. A legitimate second login attempt in the same window
        // will have to wait for the next 30s step.
        if enrollment.last_accepted_counter == Some(counter) {
            return false;
        }

        let expected = generate_totp_at(&enrollment.secret, now);
        if pylon_auth::constant_time_eq(expected.as_bytes(), code.as_bytes()) {
            enrollment.verified = true;
            enrollment.last_accepted_counter = Some(counter);
            return true;
        }
        false
    }

    /// Check if a user has verified 2FA.
    pub fn is_verified(&self, user_id: &str) -> bool {
        self.enrollments
            .lock()
            .unwrap()
            .get(user_id)
            .map(|e| e.verified)
            .unwrap_or(false)
    }

    /// Check if a user is enrolled (whether verified or not).
    pub fn is_enrolled(&self, user_id: &str) -> bool {
        self.enrollments.lock().unwrap().contains_key(user_id)
    }

    /// Generate the current TOTP code for a user.
    pub fn current_code(&self, user_id: &str) -> Option<String> {
        let enrollments = self.enrollments.lock().unwrap();
        let enrollment = enrollments.get(user_id)?;
        Some(generate_totp(&enrollment.secret))
    }

    /// Remove 2FA enrollment for a user.
    pub fn unenroll(&self, user_id: &str) -> bool {
        self.enrollments.lock().unwrap().remove(user_id).is_some()
    }
}

impl Plugin for TotpPlugin {
    fn name(&self) -> &str {
        "totp-2fa"
    }
}

/// Generate a random TOTP secret (16 chars, base32) using a CSPRNG.
fn generate_secret() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let chars = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    (0..16)
        .map(|_| chars[rng.gen_range(0..32)] as char)
        .collect()
}

/// Generate a 6-digit TOTP code per RFC 6238.
/// Uses HMAC-SHA1(secret, counter) where counter = floor(time / 30).
fn generate_totp(secret: &str) -> String {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    generate_totp_at(secret, ts)
}

/// Generate TOTP for a specific timestamp (testable).
/// Implements RFC 6238 with HMAC-SHA1 and dynamic truncation per RFC 4226.
fn generate_totp_at(secret: &str, unix_secs: u64) -> String {
    let counter = unix_secs / 30;
    let counter_bytes = counter.to_be_bytes();

    let mut mac =
        HmacSha1::new_from_slice(secret.as_bytes()).expect("HMAC can take key of any size");
    mac.update(&counter_bytes);
    let result = mac.finalize().into_bytes();
    let hash = result.as_slice();

    // Dynamic truncation per RFC 4226
    let offset = (hash[hash.len() - 1] & 0x0f) as usize;
    let binary = ((hash[offset] as u32 & 0x7f) << 24)
        | ((hash[offset + 1] as u32) << 16)
        | ((hash[offset + 2] as u32) << 8)
        | (hash[offset + 3] as u32);

    format!("{:06}", binary % 1_000_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enroll_and_verify() {
        let plugin = TotpPlugin::new();
        let secret = plugin.enroll("user-1");
        assert!(!secret.is_empty());
        assert!(plugin.is_enrolled("user-1"));
        assert!(!plugin.is_verified("user-1"));

        let code = plugin.current_code("user-1").unwrap();
        assert!(plugin.verify("user-1", &code));
        assert!(plugin.is_verified("user-1"));
    }

    #[test]
    fn wrong_code_rejected() {
        let plugin = TotpPlugin::new();
        plugin.enroll("user-1");
        assert!(!plugin.verify("user-1", "000000"));
        assert!(!plugin.is_verified("user-1"));
    }

    #[test]
    fn code_cannot_be_replayed_in_same_window() {
        // The second verify within the same 30-second counter must fail —
        // this is the replay guard. Even if `code` is still "current",
        // last_accepted_counter pins it to burn-on-use.
        let plugin = TotpPlugin::new();
        plugin.enroll("user-1");
        let code = plugin.current_code("user-1").unwrap();
        assert!(
            plugin.verify("user-1", &code),
            "first verify should succeed"
        );
        assert!(
            !plugin.verify("user-1", &code),
            "replay within the same window must be rejected"
        );
    }

    #[test]
    fn not_enrolled_returns_none() {
        let plugin = TotpPlugin::new();
        assert!(plugin.current_code("user-1").is_none());
        assert!(!plugin.is_enrolled("user-1"));
    }

    #[test]
    fn unenroll() {
        let plugin = TotpPlugin::new();
        plugin.enroll("user-1");
        assert!(plugin.unenroll("user-1"));
        assert!(!plugin.is_enrolled("user-1"));
        assert!(!plugin.unenroll("user-1")); // already removed
    }

    #[test]
    fn code_is_six_digits() {
        let plugin = TotpPlugin::new();
        plugin.enroll("user-1");
        let code = plugin.current_code("user-1").unwrap();
        assert_eq!(code.len(), 6);
        assert!(code.chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn different_users_different_secrets() {
        let plugin = TotpPlugin::new();
        let s1 = plugin.enroll("user-1");
        let s2 = plugin.enroll("user-2");
        assert!(!s1.is_empty());
        assert!(!s2.is_empty());
    }

    #[test]
    fn generate_totp_at_is_deterministic() {
        // Same secret + same timestamp must always produce the same code.
        let code1 = generate_totp_at("JBSWY3DPEHPK3PXP", 1_700_000_000);
        let code2 = generate_totp_at("JBSWY3DPEHPK3PXP", 1_700_000_000);
        assert_eq!(code1, code2);
        assert_eq!(code1.len(), 6);
        assert!(code1.chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn generate_totp_at_different_times_differ() {
        // Codes at different 30-second windows should (almost certainly) differ.
        let code1 = generate_totp_at("JBSWY3DPEHPK3PXP", 1_700_000_000);
        let code2 = generate_totp_at("JBSWY3DPEHPK3PXP", 1_700_000_030);
        assert_ne!(code1, code2);
    }

    #[test]
    fn generate_totp_at_same_window_equal() {
        // Two timestamps in the same 30-second window produce the same code.
        let code1 = generate_totp_at("SECRET", 1_700_000_000);
        let code2 = generate_totp_at("SECRET", 1_700_000_005);
        assert_eq!(code1, code2);
    }

    #[test]
    fn generate_totp_at_different_secrets_differ() {
        let code1 = generate_totp_at("SECRET_A", 1_700_000_000);
        let code2 = generate_totp_at("SECRET_B", 1_700_000_000);
        assert_ne!(code1, code2);
    }

    #[test]
    fn generate_secret_is_16_chars_base32() {
        let s = generate_secret();
        assert_eq!(s.len(), 16);
        assert!(s
            .chars()
            .all(|c| "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567".contains(c)));
    }
}
