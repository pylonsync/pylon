//! In-process request-log ring buffer for the `/admin/logs/tail`
//! endpoint.
//!
//! The Tinybird shipper writes every request to an external
//! analytics store — great for long-term aggregates, terrible as
//! a live-tail backend (each query costs a Tinybird quota credit
//! and adds 50-300ms of network latency). The dashboard's logs
//! page polls every 2s, which on Tinybird's free tier exhausts
//! the daily quota in ~22 minutes.
//!
//! Solution: keep a small in-process ring of the most recent
//! request log entries, and serve the tail directly from it. The
//! ring is bounded (oldest entries drop off), so it's not a
//! durable log — Tinybird is still the source of truth for
//! anything more than `RING_CAPACITY` deep. But for "what just
//! happened on this machine?" — the dashboard's primary use
//! case — the ring is free, lock-cheap, and gives sub-second
//! freshness.
//!
//! Lives at module scope as a `OnceLock`, mirroring the
//! TinybirdLogger pattern. One ring per process; the dashboard
//! talks to one customer machine at a time (Pylon Cloud is
//! single-machine per project today).

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

use serde::Serialize;

/// One captured request line — same shape the Tinybird shipper
/// emits, minus the deployment_id/project_id/region fields the
/// dashboard already knows from its caller-side context. Keeping
/// the in-process shape lean lets the ring hold more entries for
/// the same memory budget.
#[derive(Debug, Clone, Serialize)]
pub struct RingEntry {
    /// ISO-8601 with millisecond precision, UTC. Same format the
    /// Tinybird shipper produces — the dashboard uses this as the
    /// poll cursor (`?since=`).
    pub timestamp: String,
    pub method: String,
    pub path: String,
    pub status: u16,
    pub cpu_ms: u32,
}

/// ~10 minutes at 100 rps. Tuned for the dashboard tail use case:
/// the page shows 1000 visible lines max, so a deeper ring would
/// be pure overhead. At a generous 200 bytes/entry the ring caps
/// memory at ~200 KB.
const RING_CAPACITY: usize = 1000;

/// Lock-guarded ring buffer. `record` runs once per request — the
/// contention is the same as the `request_buckets` Mutex already
/// held in `Metrics::record_request`, and we get the same
/// "dwarfed by everything else in dispatch" amortization.
pub struct RequestLogRing {
    inner: Mutex<VecDeque<RingEntry>>,
}

impl Default for RequestLogRing {
    fn default() -> Self {
        Self::new()
    }
}

impl RequestLogRing {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(VecDeque::with_capacity(RING_CAPACITY)),
        }
    }

    /// Append a new entry, dropping the oldest when at capacity.
    /// Best-effort: a poisoned mutex (panic in another thread holding
    /// the lock) silently drops the entry rather than propagating —
    /// the ring is observability infrastructure and must never take
    /// down the request hot path.
    pub fn push(&self, entry: RingEntry) {
        if let Ok(mut q) = self.inner.lock() {
            if q.len() == RING_CAPACITY {
                q.pop_front();
            }
            q.push_back(entry);
        }
    }

    /// Return all entries with `timestamp > since` (strict, mirrors
    /// the Tinybird pipe's semantics so the dashboard cursor logic
    /// doesn't need a branch for backend choice). `since` is a
    /// lexicographic compare on the ISO-8601 string — valid because
    /// ISO-8601 timestamps sort identically as strings and as
    /// real-time. Pass `None` for a full snapshot.
    ///
    /// Returns newest-first to match the existing `recent_logs`
    /// pipe order, so the dashboard's `[...rows].reverse()` and
    /// cursor-on-`rows[0]` logic still works unchanged.
    pub fn tail_since(&self, since: Option<&str>) -> Vec<RingEntry> {
        let q = match self.inner.lock() {
            Ok(g) => g,
            Err(_) => return Vec::new(),
        };
        let mut out: Vec<RingEntry> = match since {
            Some(s) => q
                .iter()
                .filter(|e| e.timestamp.as_str() > s)
                .cloned()
                .collect(),
            None => q.iter().cloned().collect(),
        };
        out.reverse();
        out
    }
}

// Process-global ring. Same lazy-init pattern as TINYBIRD_LOGGER so
// callers don't have to wire it through; one `init` call at server
// boot, then `record_request` reads via `log_ring()`.
static LOG_RING: OnceLock<RequestLogRing> = OnceLock::new();

pub fn init_log_ring() {
    LOG_RING.get_or_init(RequestLogRing::new);
}

pub fn log_ring() -> Option<&'static RequestLogRing> {
    LOG_RING.get()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(ts: &str, status: u16) -> RingEntry {
        RingEntry {
            timestamp: ts.into(),
            method: "GET".into(),
            path: "/x".into(),
            status,
            cpu_ms: 1,
        }
    }

    #[test]
    fn tail_since_returns_strictly_newer_entries_newest_first() {
        let ring = RequestLogRing::new();
        ring.push(entry("2026-05-16T10:00:00.000Z", 200));
        ring.push(entry("2026-05-16T10:00:01.000Z", 200));
        ring.push(entry("2026-05-16T10:00:02.000Z", 500));

        let out = ring.tail_since(Some("2026-05-16T10:00:00.500Z"));
        assert_eq!(out.len(), 2, "the t=0.000 entry must be excluded");
        // Newest-first ordering.
        assert_eq!(out[0].timestamp, "2026-05-16T10:00:02.000Z");
        assert_eq!(out[1].timestamp, "2026-05-16T10:00:01.000Z");
    }

    #[test]
    fn tail_since_none_returns_everything() {
        let ring = RequestLogRing::new();
        ring.push(entry("2026-05-16T10:00:00.000Z", 200));
        ring.push(entry("2026-05-16T10:00:01.000Z", 500));
        let out = ring.tail_since(None);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn tail_since_exact_match_excludes_the_cursor_entry() {
        // Strict > semantics — dashboard sets cursor = last seen
        // timestamp, and we must NOT re-ship that row on the next
        // poll. Mirrors the Tinybird pipe's `timestamp > {{since}}`.
        let ring = RequestLogRing::new();
        ring.push(entry("2026-05-16T10:00:00.000Z", 200));
        let out = ring.tail_since(Some("2026-05-16T10:00:00.000Z"));
        assert!(out.is_empty());
    }

    #[test]
    fn capacity_drops_oldest_first() {
        let ring = RequestLogRing::new();
        for i in 0..(RING_CAPACITY + 50) {
            ring.push(entry(&format!("2026-05-16T10:00:{:02}.000Z", i % 60), 200));
        }
        let all = ring.tail_since(None);
        assert_eq!(all.len(), RING_CAPACITY);
    }
}
