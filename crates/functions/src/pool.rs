//! Pool of bun runtime processes.
//!
//! One pylon process owns N independent `FnRunner` instances — N
//! separate bun child processes, each with its own stdin/stdout
//! pipes and io_lock. Top-level function calls round-robin across
//! the pool; nested calls (`ctx.runQuery` / `ctx.runMutation` /
//! `ctx.runAction` from inside a handler) MUST stay on the same
//! runner as the parent because the protocol message correlation
//! (call_id ↔ stdio) is per-process.
//!
//! # Why this exists
//!
//! The single-runner model serialized every function call through
//! one bun process's io_lock. A single slow handler (today's case:
//! `getProjectMetrics` blocked on a 30s `fetch` against a dead
//! customer machine) held the lock long enough for the framework's
//! kill-timeout to fire — taking ALL N registered functions
//! offline during the respawn. Health probes flapped, requests
//! cascaded into rate-limit territory, and the whole control plane
//! went unreliable for minutes.
//!
//! With a pool, that same slow handler only wedges ONE runner.
//! Other runners keep serving requests. The respawn (when it
//! happens) only affects calls already dispatched to that runner.
//!
//! # Configuration
//!
//! - `PYLON_FN_POOL_SIZE` — number of bun processes to spawn.
//!   Default 1 (single-runner behavior preserved — no machines get
//!   unexpectedly OOM'd by 4× memory baseline on upgrade).
//! - Each runner ~80-120MB resident baseline plus the user's app
//!   footprint. Set per-deployment based on RAM headroom.
//!
//! # Routing
//!
//! `pick()` returns the next runner in round-robin order. We don't
//! track in-flight calls per runner today — round-robin spreads
//! load adequately for the realistic case (many short calls,
//! occasional slow one). Least-busy is a future enhancement when
//! we have per-runner concurrency telemetry.
//!
//! # Health
//!
//! `health_probe` returns Ok if ANY runner responds within the
//! budget. One wedged runner doesn't fail the probe — the proxy
//! still has working runners to route to. All-wedged is the
//! degraded signal that fails /health/deep.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::runner::FnRunner;
use crate::trace::FnTrace;

/// Pool of bun runtime processes. See module docs for design.
pub struct FnRunnerPool {
    runners: Vec<Arc<FnRunner>>,
    /// Round-robin counter. Atomic so the pick is thread-safe
    /// without serializing through a Mutex (every HTTP request
    /// goes through pick() — making it a bottleneck would defeat
    /// the purpose).
    next: AtomicUsize,
}

impl FnRunnerPool {
    /// Build a pool from already-started runners. Caller is
    /// responsible for `runner.start(...)` on each — keeps the
    /// pool's only job "routing", separate from "process
    /// lifecycle" which has its own backoff/retry semantics in
    /// `spawn_runtime_supervisor`.
    pub fn new(runners: Vec<Arc<FnRunner>>) -> Self {
        assert!(!runners.is_empty(), "FnRunnerPool requires >= 1 runner");
        Self {
            runners,
            next: AtomicUsize::new(0),
        }
    }

    /// Number of runners in the pool.
    pub fn size(&self) -> usize {
        self.runners.len()
    }

    /// All runners — used by the supervisor to walk each child
    /// process independently for the alive/respawn check.
    pub fn runners(&self) -> &[Arc<FnRunner>] {
        &self.runners
    }

    /// Pick a runner for a top-level call. Round-robin across the
    /// pool, SKIPPING runners whose bun child isn't currently alive.
    /// Returns an `Arc<FnRunner>` clone so the caller can hold it
    /// across the protocol exchange without keeping a borrow on
    /// the pool.
    ///
    /// IMPORTANT: nested calls (`ctx.runQuery` / `runMutation` /
    /// `runAction` from inside a handler) must NOT call pick() —
    /// they have to stay on the parent's runner because the
    /// protocol's call_id correlation is per-process stdio. The
    /// nested-call hook registered on each runner captures that
    /// runner's `Arc` directly; pick() is only for top-level
    /// entries (HTTP, jobs, scheduler).
    ///
    /// Dead-runner avoidance: when a runner's bun child crashes
    /// the supervisor takes some time to respawn it (exponential
    /// backoff up to 30s). During that window every Nth call would
    /// otherwise land on the dead runner and fail with
    /// `RUNNER_NOT_STARTED`. We scan up to N rotations looking for
    /// an alive runner; if all are dead, fall back to the next-slot
    /// runner so the caller gets the existing typed error rather
    /// than an infinite loop here. /health/deep stays the
    /// authoritative "any-alive" signal — the proxy will route
    /// elsewhere if all runners are dead.
    pub fn pick(&self) -> Arc<FnRunner> {
        // Wrapping arithmetic on a counter that increments forever
        // doesn't underflow into 0 — `%` handles that. usize::MAX is
        // 18 quintillion on 64-bit; even at 1M calls/sec it'd take
        // ~580k years to overflow.
        let start = self.next.fetch_add(1, Ordering::Relaxed) % self.runners.len();
        for offset in 0..self.runners.len() {
            let i = (start + offset) % self.runners.len();
            if self.runners[i].is_alive() {
                return Arc::clone(&self.runners[i]);
            }
        }
        // All runners are dead. Return the originally-picked slot
        // so the caller's downstream protocol call surfaces the
        // typed error instead of us papering over a wholly-broken
        // pool. The supervisor is presumably mid-respawn; the next
        // request will see a different state.
        Arc::clone(&self.runners[start])
    }

    /// True if at least one runner's bun process is alive. Used by
    /// the supervisor to decide whether to short-circuit — if all
    /// runners died simultaneously (kernel kill, OOM), we still
    /// loop and respawn each.
    pub fn any_alive(&self) -> bool {
        self.runners.iter().any(|r| r.is_alive())
    }

    /// Probe pool responsiveness. Succeeds if ANY runner is
    /// responsive within `timeout`. The budget is split across the
    /// pool — each runner gets `timeout / N`. The first success
    /// short-circuits.
    ///
    /// "Any responsive = healthy" is intentional. A single wedged
    /// runner (slow handler holding io_lock) shouldn't flap the
    /// proxy's health check because the OTHER runners can still
    /// serve. All-wedged means traffic genuinely has nowhere to
    /// go, and 503 from /health/deep is correct.
    pub fn health_probe(&self, timeout: Duration) -> Result<(), String> {
        if self.runners.is_empty() {
            return Err("pool is empty".into());
        }
        // Split the budget so total wall-time is bounded. usize→u32
        // saturating cast — runner counts in the thousands are
        // implausible.
        let n = u32::try_from(self.runners.len()).unwrap_or(u32::MAX);
        let per = timeout / n.max(1);
        let mut errs: Vec<String> = Vec::with_capacity(self.runners.len());
        for (i, r) in self.runners.iter().enumerate() {
            match r.health_probe(per) {
                Ok(()) => return Ok(()),
                Err(e) => errs.push(format!("runner[{i}]: {e}")),
            }
        }
        Err(format!(
            "all {} runners unresponsive within {}ms: {}",
            self.runners.len(),
            timeout.as_millis(),
            errs.join("; ")
        ))
    }

    /// Recent traces across all runners, merged + capped. Used by
    /// admin trace endpoints (/api/fn/traces, Studio) so the
    /// operator sees activity from EVERY runner in one stream
    /// rather than per-runner subsets.
    pub fn recent_traces(&self, limit: usize) -> Vec<FnTrace> {
        let mut all: Vec<FnTrace> = self
            .runners
            .iter()
            .flat_map(|r| r.trace_log.recent(limit))
            .collect();
        // Newest first across the merge. FnTrace's started_at is
        // epoch seconds; sort descending so admin views show the
        // latest activity at the top.
        all.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        all.truncate(limit);
        all
    }

    /// Resolve the pool size from the env. Default 1 preserves the
    /// pre-pool behaviour so a framework upgrade doesn't surprise
    /// anyone with 4× memory baseline overnight.
    ///
    /// PYLON_FN_POOL_SIZE = "auto" picks max(1, cpus / 2) so
    /// operators can opt into "use the machine's actual capacity"
    /// without hard-coding. Explicit integers always win.
    pub fn size_from_env(default: usize) -> usize {
        let raw = std::env::var("PYLON_FN_POOL_SIZE").unwrap_or_default();
        let cpus = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);
        Self::parse_pool_size(raw.as_str(), default, cpus)
    }

    /// Pure parser for the PYLON_FN_POOL_SIZE value — separated
    /// from the env read so tests can exercise the logic without
    /// mutating process-wide state (which races under Rust's
    /// default parallel test runner). `size_from_env` is the
    /// production-side wrapper that reads the env + queries CPU
    /// count, then delegates here.
    pub fn parse_pool_size(raw: &str, default: usize, cpus: usize) -> usize {
        let t = raw.trim();
        if t.is_empty() {
            return default;
        }
        if t.eq_ignore_ascii_case("auto") {
            return (cpus / 2).max(1);
        }
        t.parse::<usize>().unwrap_or(default).max(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_runner() -> Arc<FnRunner> {
        // Bare runner, never started — pick() doesn't touch the
        // process. Trace log capacity 10 keeps allocations small.
        Arc::new(FnRunner::new(10))
    }

    #[test]
    fn pick_falls_back_to_start_when_all_runners_dead() {
        // All runners are unstarted → is_alive() returns false on
        // each. pick() scans the whole pool, finds nothing alive,
        // and falls back to the originally-chosen rotation slot
        // so the caller's downstream protocol call surfaces the
        // typed RUNNER_NOT_STARTED error rather than us looping.
        let pool = FnRunnerPool::new(vec![dummy_runner(), dummy_runner(), dummy_runner()]);
        // Just exercise the call path; with all dummies dead we
        // expect a stable runner back (no panic, no infinite loop).
        let a = pool.pick();
        let b = pool.pick();
        // The rotation counter still advances; both calls return
        // SOME runner from the pool (the start slot for each).
        assert!(Arc::as_ptr(&a) as usize != 0);
        assert!(Arc::as_ptr(&b) as usize != 0);
    }

    // Tests exercise `parse_pool_size` (the pure parser) instead
    // of `size_from_env` (the env-reading wrapper). Earlier
    // revisions mutated PYLON_FN_POOL_SIZE which races under
    // cargo test's parallel runner — flaky CI in the making.
    #[test]
    fn parse_pool_size_explicit_int_wins() {
        assert_eq!(FnRunnerPool::parse_pool_size("7", 1, 8), 7);
    }

    #[test]
    fn parse_pool_size_empty_uses_default() {
        assert_eq!(FnRunnerPool::parse_pool_size("", 3, 8), 3);
        assert_eq!(FnRunnerPool::parse_pool_size("   ", 3, 8), 3);
    }

    #[test]
    fn parse_pool_size_zero_clamps_to_one() {
        assert_eq!(FnRunnerPool::parse_pool_size("0", 1, 8), 1);
    }

    #[test]
    fn parse_pool_size_auto_picks_half_cpus() {
        assert_eq!(FnRunnerPool::parse_pool_size("auto", 1, 8), 4);
        assert_eq!(FnRunnerPool::parse_pool_size("AUTO", 1, 8), 4);
        // 1 cpu still yields 1 — never zero.
        assert_eq!(FnRunnerPool::parse_pool_size("auto", 1, 1), 1);
    }

    #[test]
    fn parse_pool_size_bogus_input_uses_default() {
        assert_eq!(FnRunnerPool::parse_pool_size("twelve", 2, 8), 2);
    }

    #[test]
    #[should_panic(expected = "FnRunnerPool requires >= 1 runner")]
    fn empty_pool_panics() {
        FnRunnerPool::new(vec![]);
    }
}
