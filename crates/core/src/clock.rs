//! Clock abstraction for testable time-dependent logic.
//!
//! Code that needs the current time should accept a `&dyn Clock` instead of
//! calling `SystemTime::now()` directly. Production callers pass
//! `SystemClock`; tests pass `MockClock` and advance it explicitly.
//!
//! Both `now_unix_secs()` (wall clock) and `now_monotonic()` (monotonic) are
//! exposed because they have different uses:
//! - wall clock for timestamps stored in the database (session expiry,
//!   `createdAt`)
//! - monotonic for measuring elapsed time (rate-limiter windows, tick
//!   scheduling) — wall clock can jump backward via NTP

use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub trait Clock: Send + Sync {
    /// Current wall-clock time as Unix epoch seconds.
    fn now_unix_secs(&self) -> u64;

    /// Current wall-clock time as Unix epoch milliseconds.
    fn now_unix_millis(&self) -> u64 {
        self.now_unix_secs() * 1000
    }

    /// A monotonically increasing instant for measuring durations.
    /// Implementations may return synthetic instants that share a base.
    fn now_monotonic(&self) -> Instant;
}

/// Clock backed by `std::time` — the production default.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_unix_secs(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    fn now_unix_millis(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    fn now_monotonic(&self) -> Instant {
        Instant::now()
    }
}

/// Test clock with manual advancement.
///
/// Both wall and monotonic time advance together. Start at any Unix time and
/// step forward with `advance(duration)`.
pub struct MockClock {
    inner: Mutex<MockState>,
}

struct MockState {
    unix_millis: u64,
    base_instant: Instant,
    elapsed: Duration,
}

impl MockClock {
    pub fn new(start_unix_secs: u64) -> Self {
        Self {
            inner: Mutex::new(MockState {
                unix_millis: start_unix_secs * 1000,
                base_instant: Instant::now(),
                elapsed: Duration::ZERO,
            }),
        }
    }

    pub fn advance(&self, by: Duration) {
        let mut s = self.inner.lock().expect("MockClock poisoned");
        s.unix_millis += by.as_millis() as u64;
        s.elapsed += by;
    }
}

impl Clock for MockClock {
    fn now_unix_secs(&self) -> u64 {
        self.inner.lock().expect("MockClock poisoned").unix_millis / 1000
    }

    fn now_unix_millis(&self) -> u64 {
        self.inner.lock().expect("MockClock poisoned").unix_millis
    }

    fn now_monotonic(&self) -> Instant {
        let s = self.inner.lock().expect("MockClock poisoned");
        s.base_instant + s.elapsed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_clock_returns_nonzero() {
        let c = SystemClock;
        assert!(c.now_unix_secs() > 1_700_000_000);
    }

    #[test]
    fn mock_clock_starts_at_given_time() {
        let c = MockClock::new(1_000_000);
        assert_eq!(c.now_unix_secs(), 1_000_000);
        assert_eq!(c.now_unix_millis(), 1_000_000_000);
    }

    #[test]
    fn mock_clock_advances_wall_and_monotonic_together() {
        let c = MockClock::new(0);
        let m0 = c.now_monotonic();
        c.advance(Duration::from_secs(60));
        assert_eq!(c.now_unix_secs(), 60);
        let m1 = c.now_monotonic();
        assert_eq!(m1.duration_since(m0), Duration::from_secs(60));
    }

    #[test]
    fn dyn_clock_is_object_safe() {
        fn use_it(c: &dyn Clock) -> u64 {
            c.now_unix_secs()
        }
        assert_eq!(use_it(&MockClock::new(42)), 42);
    }
}
