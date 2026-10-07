//! Settings and brute-force guard shared by every one-time code Pylon
//! issues: email magic codes, email verification codes (which reuse
//! [`crate::MagicCodeStore`]), and phone/SMS codes.
//!
//! **Length.** `PYLON_AUTH_CODE_LENGTH` sets how many digits a code has,
//! from 4 to 8. The default is 6. Any other value is a startup error.
//!
//! **Guard.** Each code allows a few wrong guesses before it burns, and a
//! new code can be requested every minute or so. Without a limit across
//! codes, an attacker could make thousands of guesses a day against one
//! account, and a short code space falls quickly. So the stores count
//! wrong guesses per email or phone over a rolling 24 hours, in a durable
//! [`CodeFailureLedger`], and refuse new codes and verifies once the count
//! reaches [`CodePolicy::daily_failure_limit`]. The limit scales with the
//! code space (`10^length / 1000`, at least 10), which keeps an attacker's
//! daily chance of guessing a code near 0.1% at every length:
//!
//! | length | codes      | wrong guesses allowed per 24 h |
//! |--------|------------|--------------------------------|
//! | 4      | 10,000     | 10                             |
//! | 5      | 100,000    | 100                            |
//! | 6      | 1,000,000  | 1,000                          |
//! | 7      | 10,000,000 | 10,000                         |
//! | 8      | 100,000,000| 100,000                        |
//!
//! A correct code clears the identifier's failures, so a member who
//! mistypes a few times and then signs in starts fresh.

use std::collections::HashMap;
use std::sync::Mutex;

/// Env var that sets the code length.
pub const CODE_LENGTH_ENV: &str = "PYLON_AUTH_CODE_LENGTH";
/// Code length when [`CODE_LENGTH_ENV`] is unset.
pub const DEFAULT_CODE_LENGTH: u8 = 6;
/// Shortest allowed code.
pub const MIN_CODE_LENGTH: u8 = 4;
/// Longest allowed code.
pub const MAX_CODE_LENGTH: u8 = 8;
/// The window wrong guesses are counted over.
pub const FAILURE_WINDOW_SECS: u64 = 24 * 60 * 60;

/// How long codes are and how many wrong guesses an identifier gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodePolicy {
    length: u8,
}

impl Default for CodePolicy {
    fn default() -> Self {
        Self {
            length: DEFAULT_CODE_LENGTH,
        }
    }
}

impl CodePolicy {
    /// A policy with `length`-digit codes. Errors outside 4..=8.
    pub fn new(length: u8) -> Result<Self, String> {
        if !(MIN_CODE_LENGTH..=MAX_CODE_LENGTH).contains(&length) {
            return Err(format!(
                "{CODE_LENGTH_ENV} must be a whole number from {MIN_CODE_LENGTH} to {MAX_CODE_LENGTH}; got {length}"
            ));
        }
        Ok(Self { length })
    }

    /// Read [`CODE_LENGTH_ENV`]. Unset or empty means the default. Any
    /// value that is not a whole number from 4 to 8 is an error, so a
    /// typo stops the server instead of silently using another length.
    pub fn from_env() -> Result<Self, String> {
        match std::env::var(CODE_LENGTH_ENV) {
            Ok(raw) if !raw.trim().is_empty() => Self::parse(&raw),
            _ => Ok(Self::default()),
        }
    }

    /// Parse a configured length.
    pub fn parse(raw: &str) -> Result<Self, String> {
        let trimmed = raw.trim();
        let length: u8 = trimmed.parse().map_err(|_| {
            format!(
                "{CODE_LENGTH_ENV} must be a whole number from {MIN_CODE_LENGTH} to {MAX_CODE_LENGTH}; got {trimmed:?}"
            )
        })?;
        Self::new(length)
    }

    /// Digits per code.
    pub fn length(&self) -> u8 {
        self.length
    }

    /// A new code from a CSPRNG, zero-padded to the configured length.
    pub fn generate(&self) -> String {
        use rand::Rng;
        let space = 10u32.pow(self.length as u32);
        let n = rand::thread_rng().gen_range(0..space);
        format!("{:0width$}", n, width = self.length as usize)
    }

    /// Wrong guesses one identifier may make in [`FAILURE_WINDOW_SECS`]
    /// before new codes and verifies are refused. `10^length / 1000`,
    /// never below 10.
    pub fn daily_failure_limit(&self) -> u32 {
        (10u32.pow(self.length as u32) / 1000).max(10)
    }
}

/// Ledger key for an email identifier.
pub fn email_key(email: &str) -> String {
    format!("email:{email}")
}

/// Ledger key for a phone identifier (E.164).
pub fn phone_key(phone: &str) -> String {
    format!("phone:{phone}")
}

/// Durable record of wrong code guesses, per identifier. The runtime
/// ships SQLite and Postgres implementations so the count survives a
/// restart or deploy; an in-memory count would reset and hand an attacker
/// a fresh budget each time.
///
/// Methods are infallible from the caller's side: a backend that cannot
/// write logs the error. Timestamps are Unix seconds.
pub trait CodeFailureLedger: Send + Sync {
    /// Record one wrong guess for `key` at `at`.
    fn record(&self, key: &str, at: u64);
    /// Wrong guesses for `key` at or after `since`.
    fn count_since(&self, key: &str, since: u64) -> u32;
    /// The earliest wrong guess for `key` at or after `since`, if any.
    fn oldest_since(&self, key: &str, since: u64) -> Option<u64>;
    /// Forget every wrong guess for `key` (after a correct code).
    fn clear(&self, key: &str);
    /// Drop records older than `before`. Called opportunistically.
    fn prune(&self, before: u64);
}

/// In-memory ledger, for tests and for stores built without a database.
#[derive(Default)]
pub struct InMemoryCodeFailureLedger {
    failures: Mutex<HashMap<String, Vec<u64>>>,
}

impl InMemoryCodeFailureLedger {
    pub fn new() -> Self {
        Self::default()
    }
}

impl CodeFailureLedger for InMemoryCodeFailureLedger {
    fn record(&self, key: &str, at: u64) {
        if let Ok(mut m) = self.failures.lock() {
            m.entry(key.to_string()).or_default().push(at);
        }
    }
    fn count_since(&self, key: &str, since: u64) -> u32 {
        self.failures
            .lock()
            .ok()
            .and_then(|m| {
                m.get(key)
                    .map(|v| v.iter().filter(|t| **t >= since).count() as u32)
            })
            .unwrap_or(0)
    }
    fn oldest_since(&self, key: &str, since: u64) -> Option<u64> {
        self.failures
            .lock()
            .ok()?
            .get(key)?
            .iter()
            .copied()
            .filter(|t| *t >= since)
            .min()
    }
    fn clear(&self, key: &str) {
        if let Ok(mut m) = self.failures.lock() {
            m.remove(key);
        }
    }
    fn prune(&self, before: u64) {
        if let Ok(mut m) = self.failures.lock() {
            for v in m.values_mut() {
                v.retain(|t| *t >= before);
            }
            m.retain(|_, v| !v.is_empty());
        }
    }
}

/// The guard both code stores apply: a policy plus a ledger.
#[derive(Clone)]
pub struct CodeGuard {
    pub policy: CodePolicy,
    pub ledger: std::sync::Arc<dyn CodeFailureLedger>,
}

impl Default for CodeGuard {
    fn default() -> Self {
        Self {
            policy: CodePolicy::default(),
            ledger: std::sync::Arc::new(InMemoryCodeFailureLedger::new()),
        }
    }
}

impl CodeGuard {
    pub fn new(policy: CodePolicy, ledger: std::sync::Arc<dyn CodeFailureLedger>) -> Self {
        Self { policy, ledger }
    }

    /// `Some(retry_after_secs)` when `key` has used its wrong-guess budget
    /// for the window ending at `now`; `None` when it may continue.
    pub fn locked_for(&self, key: &str, now: u64) -> Option<u64> {
        let since = now.saturating_sub(FAILURE_WINDOW_SECS);
        if self.ledger.count_since(key, since) < self.policy.daily_failure_limit() {
            return None;
        }
        // The lock lifts when the oldest counted failure leaves the window.
        let oldest = self.ledger.oldest_since(key, since).unwrap_or(now);
        Some((oldest + FAILURE_WINDOW_SECS).saturating_sub(now).max(1))
    }

    /// Record a wrong guess and drop records that have left the window.
    pub fn record_failure(&self, key: &str, now: u64) {
        self.ledger.record(key, now);
        self.ledger.prune(now.saturating_sub(FAILURE_WINDOW_SECS));
    }

    /// A correct code: start the identifier fresh.
    pub fn record_success(&self, key: &str) {
        self.ledger.clear(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn default_is_six() {
        assert_eq!(CodePolicy::default().length(), 6);
    }

    #[test]
    fn parse_accepts_four_to_eight() {
        for n in 4..=8u8 {
            assert_eq!(CodePolicy::parse(&n.to_string()).unwrap().length(), n);
        }
        assert_eq!(CodePolicy::parse(" 4 ").unwrap().length(), 4);
    }

    #[test]
    fn parse_rejects_out_of_range_and_junk() {
        for bad in ["3", "9", "0", "-4", "six", "4.5", "", "999"] {
            assert!(
                CodePolicy::parse(bad).is_err(),
                "{bad:?} should be rejected"
            );
        }
    }

    #[test]
    fn generated_codes_have_the_configured_length_and_are_digits() {
        for n in 4..=8u8 {
            let p = CodePolicy::new(n).unwrap();
            for _ in 0..200 {
                let c = p.generate();
                assert_eq!(c.len(), n as usize);
                assert!(c.bytes().all(|b| b.is_ascii_digit()));
            }
        }
    }

    #[test]
    fn small_values_are_zero_padded() {
        // 10_000 draws from a 4-digit space make a leading zero near-certain.
        let p = CodePolicy::new(4).unwrap();
        assert!((0..10_000)
            .map(|_| p.generate())
            .any(|c| c.starts_with('0') && c.len() == 4));
    }

    #[test]
    fn daily_limit_table() {
        let limits: Vec<u32> = (4..=8u8)
            .map(|n| CodePolicy::new(n).unwrap().daily_failure_limit())
            .collect();
        assert_eq!(limits, vec![10, 100, 1_000, 10_000, 100_000]);
    }

    #[test]
    fn guard_locks_at_the_limit_and_lifts_when_the_window_passes() {
        let guard = CodeGuard::new(
            CodePolicy::new(4).unwrap(),
            Arc::new(InMemoryCodeFailureLedger::new()),
        );
        let t0 = 1_000_000;
        for i in 0..9 {
            guard.record_failure("email:a@x.com", t0 + i);
        }
        assert_eq!(guard.locked_for("email:a@x.com", t0 + 10), None);
        guard.record_failure("email:a@x.com", t0 + 9);
        let retry = guard.locked_for("email:a@x.com", t0 + 10).expect("locked");
        assert_eq!(retry, FAILURE_WINDOW_SECS - 10);
        // Other identifiers are unaffected.
        assert_eq!(guard.locked_for("email:b@x.com", t0 + 10), None);
        // Once the oldest failure leaves the window, one more guess is allowed.
        assert_eq!(
            guard.locked_for("email:a@x.com", t0 + FAILURE_WINDOW_SECS + 1),
            None
        );
    }

    #[test]
    fn success_clears_failures() {
        let guard = CodeGuard::new(
            CodePolicy::new(4).unwrap(),
            Arc::new(InMemoryCodeFailureLedger::new()),
        );
        for i in 0..10 {
            guard.record_failure("phone:+15551234567", 100 + i);
        }
        assert!(guard.locked_for("phone:+15551234567", 200).is_some());
        guard.record_success("phone:+15551234567");
        assert_eq!(guard.locked_for("phone:+15551234567", 200), None);
    }

    #[test]
    fn prune_drops_old_records() {
        let ledger = InMemoryCodeFailureLedger::new();
        ledger.record("k", 10);
        ledger.record("k", 20);
        ledger.prune(15);
        assert_eq!(ledger.count_since("k", 0), 1);
        assert_eq!(ledger.oldest_since("k", 0), Some(20));
    }
}
