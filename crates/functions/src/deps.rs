//! Per-handler dep tracking for reactive queries.
//!
//! When a `query()` handler runs under a reactive subscription, the
//! runner records which entities (and optionally which row ids) the
//! handler read from `ctx.db.*`. The runtime's reactive registry
//! stores this dep set per subscription; whenever a future change
//! event touches any entity in the set, the handler is scheduled for
//! re-run and the new result is pushed to the subscribed client.
//!
//! The recorder is thread-local + stack-based so nested calls
//! (action → runQuery → handler) push/pop their own scope cleanly. A
//! reactive re-runner activates a recorder around the handler call;
//! all `execute_db_op` invocations inside that call write through it.
//! Non-reactive callers (HTTP query, mutation, action) see a no-op
//! and pay nothing.

use std::cell::RefCell;
use std::collections::HashSet;

/// Dep set collected during one handler invocation.
///
/// `entities` is the coarse signal — when a mutation touches this
/// entity the subscription is dirty. `rows` is the precise signal —
/// when set is non-empty the registry can suppress re-runs for
/// changes outside the read row ids. Both are populated; the registry
/// chooses which granularity to consult.
///
/// `row_cap_blown` is sticky: once a handler exceeds [`ROW_DEP_CAP`]
/// distinct row reads, we fall back to entity-only matching for the
/// life of this dep set. Without the sticky flag, subsequent
/// `record_read` calls would clear-and-refill `rows` in a loop,
/// producing a non-deterministic snapshot that could miss matches
/// the registry should have flagged.
#[derive(Debug, Clone, Default)]
pub struct DepSet {
    pub entities: HashSet<String>,
    pub rows: HashSet<(String, String)>,
    row_cap_blown: bool,
}

impl DepSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_read(&mut self, entity: &str, row_id: Option<&str>) {
        self.entities.insert(entity.to_string());
        if self.row_cap_blown {
            return;
        }
        if let Some(id) = row_id {
            // Cap row-level deps per sub. A query that reads 10k rows
            // doesn't need precise per-row tracking — the entity-level
            // dep already covers it, and storing 10k tuples per sub
            // bloats memory + slows match lookups. Once we exceed the
            // cap, drop to entity-only matching permanently for this
            // sub via the sticky `row_cap_blown` flag.
            if self.rows.len() >= ROW_DEP_CAP {
                self.rows.clear();
                self.row_cap_blown = true;
            } else {
                self.rows.insert((entity.to_string(), id.to_string()));
            }
        }
    }

    /// Returns true if `entities` is non-empty and `rows` is empty —
    /// the "match on entity only" mode set when the row cap was hit
    /// OR when the handler only read via list/query (no row ids).
    pub fn entity_only(&self) -> bool {
        !self.entities.is_empty() && self.rows.is_empty()
    }
}

/// Hard cap on per-subscription row-id deps. Beyond this we drop to
/// entity-level matching for the subscription. Picked to be generous
/// (most reactive handlers touch <100 rows) but bounded so a runaway
/// query can't OOM the registry.
const ROW_DEP_CAP: usize = 256;

// Thread-local recorder stack. `push` activates a recorder for the
// current call; `pop` retrieves it. The stack supports nested
// reactive re-runs (action → runQuery from inside a reactive handler)
// without one inner call's reads leaking to an outer one's dep set.
thread_local! {
    static RECORDER_STACK: RefCell<Vec<DepSet>> = const { RefCell::new(Vec::new()) };
}

/// Push a fresh recorder onto the stack. Returns a guard that pops
/// on drop, ensuring stack discipline even if the handler panics.
#[must_use]
pub fn enter() -> DepRecorderGuard {
    RECORDER_STACK.with(|cell| cell.borrow_mut().push(DepSet::new()));
    DepRecorderGuard { _priv: () }
}

/// Record a read against the top recorder on the stack, if any.
/// No-op when no recorder is active — the cost is a single
/// thread-local borrow + is_empty check.
pub fn record_read(entity: &str, row_id: Option<&str>) {
    RECORDER_STACK.with(|cell| {
        if let Some(top) = cell.borrow_mut().last_mut() {
            top.record_read(entity, row_id);
        }
    });
}

/// Drop guard for [`enter`]. Pops the top recorder on drop and
/// returns its accumulated dep set via [`take`]. The guard is
/// `#[must_use]` so callers can't accidentally forget to fetch the
/// deps before the scope ends.
pub struct DepRecorderGuard {
    // Private field so callers can't construct one without going
    // through `enter`.
    _priv: (),
}

impl DepRecorderGuard {
    /// Pop the active recorder and return its dep set. Consumes the
    /// guard — the recorder is gone after this call. If somehow the
    /// stack was empty (shouldn't happen given `enter` pushes), an
    /// empty DepSet is returned and the caller logs.
    pub fn take(self) -> DepSet {
        // Forget self so Drop doesn't double-pop. The actual pop
        // happens here.
        let result = RECORDER_STACK.with(|cell| cell.borrow_mut().pop().unwrap_or_default());
        std::mem::forget(self);
        result
    }
}

impl Drop for DepRecorderGuard {
    fn drop(&mut self) {
        // Either the caller forgot to call `take` (drop path) OR a
        // panic unwound through the scope. Either way pop so the
        // stack stays balanced; the dep set is discarded.
        RECORDER_STACK.with(|cell| {
            let _ = cell.borrow_mut().pop();
        });
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_active_recorder_is_a_noop() {
        // Recording without an active scope must not panic and must
        // not record anything (there's nowhere to record TO).
        record_read("X", Some("1"));
        // Nothing to assert — just verifying no panic.
    }

    #[test]
    fn enter_then_take_captures_reads() {
        let guard = enter();
        record_read("Recording", Some("r_1"));
        record_read("Recording", Some("r_2"));
        record_read("Org", None);
        let deps = guard.take();
        assert!(deps.entities.contains("Recording"));
        assert!(deps.entities.contains("Org"));
        assert_eq!(deps.rows.len(), 2);
        assert!(deps.rows.contains(&("Recording".into(), "r_1".into())));
    }

    #[test]
    fn drop_without_take_is_balanced() {
        {
            let _guard = enter();
            record_read("X", Some("1"));
            // _guard drops here without take(); deps are discarded.
        }
        // Stack is now empty; subsequent record_read is a no-op.
        record_read("X", Some("1"));
        // Verify by re-entering and confirming the previous data is gone.
        let g = enter();
        record_read("Y", None);
        let deps = g.take();
        assert!(!deps.entities.contains("X"));
        assert!(deps.entities.contains("Y"));
    }

    #[test]
    fn nested_recorders_dont_cross_talk() {
        let outer = enter();
        record_read("Outer", Some("o1"));
        {
            let inner = enter();
            record_read("Inner", Some("i1"));
            let inner_deps = inner.take();
            assert!(inner_deps.entities.contains("Inner"));
            assert!(!inner_deps.entities.contains("Outer"));
        }
        // After the inner scope exits, reads land in the outer
        // recorder again — proves the stack pop restored the outer.
        record_read("OuterAgain", None);
        let outer_deps = outer.take();
        assert!(outer_deps.entities.contains("Outer"));
        assert!(outer_deps.entities.contains("OuterAgain"));
        assert!(!outer_deps.entities.contains("Inner"));
    }

    #[test]
    fn row_cap_drops_to_entity_only() {
        let g = enter();
        for i in 0..(ROW_DEP_CAP + 10) {
            record_read("Big", Some(&format!("r{i}")));
        }
        let deps = g.take();
        assert!(deps.entities.contains("Big"));
        // Once the cap blew, rows was cleared as a sentinel for
        // entity-only matching.
        assert!(deps.entity_only());
        assert_eq!(deps.rows.len(), 0);
    }
}
