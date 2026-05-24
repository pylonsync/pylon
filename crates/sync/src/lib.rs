use std::sync::Mutex;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Change events — the append-only log entries
// ---------------------------------------------------------------------------

/// A change event in the sync log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeEvent {
    /// Monotonically increasing sequence number.
    pub seq: u64,
    /// The entity that was changed.
    pub entity: String,
    /// The row ID that was changed.
    pub row_id: String,
    /// The type of change.
    pub kind: ChangeKind,
    /// The data after the change (None for deletes).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    /// Pre-update row snapshot for `Update` events whose visibility
    /// transitions across the read-policy boundary (e.g.
    /// `allowRead: "auth.userId == data.userId"` + an ownership
    /// change).
    ///
    /// Invariant: when set on an Update, the broadcast filter runs a
    /// two-stage check per subscriber — first against `data` (post),
    /// then against `prev_data` (pre) if the post check denied. A
    /// subscriber whose policy allowed them to see the row PRE-update
    /// but denies them POST-update receives a synthesized Delete at
    /// this seq instead of being silently dropped — closing the
    /// stale-row ghost where a row that "moved away" from a viewer
    /// stayed in their local replica indefinitely.
    ///
    /// Default `None` — `Insert` and `Delete` events don't need it,
    /// and `Update` events that don't change visibility don't need
    /// to ship the extra bytes either.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prev_data: Option<serde_json::Value>,
    /// Timestamp of the change.
    pub timestamp: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeKind {
    Insert,
    Update,
    Delete,
}

/// Typed, shape-correct change record. Use `ChangeLog::record` instead
/// of `ChangeLog::append` whenever possible — the typed variants make
/// invariants checkable at the call site:
///
/// - `Insert { row }` requires the full post-insert row (not the
///   caller's partial input). The change-event broadcast filter relies
///   on per-row policy evaluation, and a partial row trips false denies.
/// - `Update { row }` requires the full post-update row. Same reason.
/// - `Delete { snapshot }` requires the pre-delete row (or explicit
///   `None` for cases where the snapshot was unrecoverable — e.g. the
///   row was already gone, or this is a tenant-scoped purge). The WS
///   broadcast filter needs the snapshot to authorize delivery to
///   subscribers whose policy depends on row data.
#[derive(Debug, Clone)]
pub enum ChangeRecord {
    Insert {
        row: serde_json::Value,
    },
    Update {
        row: serde_json::Value,
        /// Pre-update row snapshot. Drives the dual-check on the WS
        /// broadcast and pull filter — see `ChangeEvent::prev_data`.
        /// `None` is permitted but means subscribers whose visibility
        /// was revoked by this update won't get a tombstone and may
        /// keep a stale local row until the next reconcile.
        prev: Option<serde_json::Value>,
    },
    Delete {
        /// The pre-delete row snapshot. `None` is permitted but
        /// documented at every call site that uses it — without the
        /// snapshot, row-scoped read policies fall back to entity-level
        /// only and a private row's deletion may leak to clients that
        /// shouldn't have seen the row in the first place.
        snapshot: Option<serde_json::Value>,
    },
}

impl ChangeRecord {
    pub fn kind(&self) -> ChangeKind {
        match self {
            ChangeRecord::Insert { .. } => ChangeKind::Insert,
            ChangeRecord::Update { .. } => ChangeKind::Update,
            ChangeRecord::Delete { .. } => ChangeKind::Delete,
        }
    }

    pub fn data(&self) -> Option<&serde_json::Value> {
        match self {
            ChangeRecord::Insert { row } | ChangeRecord::Update { row, .. } => Some(row),
            ChangeRecord::Delete { snapshot } => snapshot.as_ref(),
        }
    }

    pub fn into_parts(self) -> (Option<serde_json::Value>, Option<serde_json::Value>) {
        match self {
            ChangeRecord::Insert { row } => (Some(row), None),
            ChangeRecord::Update { row, prev } => (Some(row), prev),
            ChangeRecord::Delete { snapshot } => (snapshot, None),
        }
    }
}

// ---------------------------------------------------------------------------
// Sync cursor — tracks client position in the log
// ---------------------------------------------------------------------------

/// A sync cursor representing a client's position in the change log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncCursor {
    /// The last sequence number the client has seen.
    pub last_seq: u64,
}

impl SyncCursor {
    pub fn beginning() -> Self {
        Self { last_seq: 0 }
    }
}

// ---------------------------------------------------------------------------
// Pull response — what the server sends to a pulling client
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullResponse {
    /// Changes since the client's cursor.
    pub changes: Vec<ChangeEvent>,
    /// The new cursor position after these changes.
    pub cursor: SyncCursor,
    /// Whether there are more changes to pull.
    pub has_more: bool,
}

/// Error returned by [`ChangeLog::pull`].
#[derive(Debug, Clone)]
pub enum PullError {
    /// The caller's cursor has fallen off the back of the retention window.
    /// The client should do a full re-sync from entity-list state rather than
    /// trusting the delta stream — events between `cursor.last_seq` and
    /// `oldest_seq` were evicted and cannot be replayed.
    ResyncRequired { oldest_seq: u64, cursor: SyncCursor },
}

// ---------------------------------------------------------------------------
// Push request — what a client sends to push changes
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushRequest {
    /// The changes the client wants to push.
    pub changes: Vec<ClientChange>,
    /// Stable identifier for this client across reconnects. Lets the server
    /// correlate retries (even without op_id) and attach per-client
    /// diagnostics / rate limits. Clients that don't supply one get a
    /// synthesized `"anon"` bucket for those features. Legacy clients
    /// without this field keep working — the router ignores it when
    /// absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientChange {
    pub entity: String,
    pub row_id: String,
    pub kind: ChangeKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    /// Client-minted idempotency key. The server remembers recently-seen
    /// op_ids and short-circuits replays with the previous result instead
    /// of re-applying the change. When absent, no dedup is performed (legacy
    /// clients stay functional but lose idempotency on retry).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub op_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Change log — in-memory append-only log
// ---------------------------------------------------------------------------

/// An in-memory change log with bounded retention.
///
/// Older events are evicted when the log exceeds `capacity`. The sequence
/// counter still increments monotonically; clients pulling with an old
/// cursor will see only what remains in memory (or should issue a full
/// re-sync if their cursor falls off the back).
/// Closure called by `ChangeLog::append` to mint a new sequence number.
/// Wired by the runtime to Postgres' `nextval('pylon_change_seq')` in
/// cluster mode — that gives globally-monotonic seqs across every
/// instance writing to the same database. SQLite / single-instance
/// deployments leave this `None` and the in-memory atomic counter
/// generates seqs locally.
pub type SeqProvider = std::sync::Arc<dyn Fn() -> u64 + Send + Sync>;

pub struct ChangeLog {
    events: Mutex<std::collections::VecDeque<ChangeEvent>>,
    seq: Mutex<u64>,
    capacity: usize,
    /// External seq mint. When set, `append` calls this for every new
    /// event's seq AND tracks max-seen in `seq` so `current_seq()` /
    /// `append_peer()` interop correctly with the global counter.
    seq_provider: Option<SeqProvider>,
    /// Recently-claimed client op_ids and their applied seq, for push
    /// idempotency. Entries transition `Pending` → `Applied(seq)` on the
    /// final state of the write; on failure the entry is removed entirely
    /// so the client's retry can run for real. Bounded by `op_id_capacity`:
    /// the FIFO queue tracks insertion order so the oldest entries age
    /// out when the map grows past the limit.
    op_state: Mutex<std::collections::HashMap<String, OpEntry>>,
    op_order: Mutex<std::collections::VecDeque<String>>,
    op_id_capacity: usize,
}

/// State of a tracked op_id. Used by the push handler to disambiguate
/// "concurrent retry of an in-flight write" (Pending) from "retry after
/// a confirmed apply" (Applied) — pre-fix the two looked identical
/// because we stored a flat seen-set with no per-entry result.
#[derive(Debug, Clone)]
pub enum OpEntry {
    /// Claim succeeded; the writer is currently mutating the store. A
    /// concurrent push carrying the same op_id should NOT re-apply (it
    /// would race the first writer) but also can't be told a seq yet
    /// because the first hasn't committed.
    Pending,
    /// First write succeeded and was assigned this seq. Subsequent
    /// pushes carrying the same op_id get the cached seq back so the
    /// client can advance its cursor correctly instead of treating the
    /// dedupe as "applied at unknown seq".
    Applied { seq: u64 },
}

/// Outcome of `ChangeLog::claim_op_id_v2`. Caller branches:
/// - `Proceed` → run the write; on success call `complete_op_id`, on
///    failure call `forget_op_id`.
/// - `InFlight` → another caller is currently writing this op_id; return
///    `status="pending"` to the client so it can retry later when the
///    first writer has committed and the entry has transitioned to
///    `Applied`.
/// - `Replayed { seq }` → previously applied. Return success with the
///    cached seq.
#[derive(Debug, Clone)]
pub enum OpClaim {
    Proceed,
    InFlight,
    Replayed { seq: u64 },
}

impl ChangeLog {
    /// Create a new change log with the default capacity of 10,000 events.
    pub fn new() -> Self {
        Self::with_capacity(10_000)
    }

    /// Create a new change log with a specific capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            events: Mutex::new(std::collections::VecDeque::with_capacity(
                capacity.min(1024),
            )),
            seq: Mutex::new(0),
            capacity,
            seq_provider: None,
            op_state: Mutex::new(std::collections::HashMap::with_capacity(1024)),
            op_order: Mutex::new(std::collections::VecDeque::with_capacity(1024)),
            op_id_capacity: 10_000,
        }
    }

    /// Install an external seq provider — typically a closure that
    /// calls `nextval()` on a Postgres SEQUENCE so multi-instance
    /// deployments share a single monotonic counter. Returns self so
    /// callers can chain: `ChangeLog::with_capacity(N).with_seq(p)`.
    ///
    /// On install, seeds the internal `seq` mutex with the provider's
    /// current value (`SELECT last_value FROM pylon_change_seq` style)
    /// — the wiring closure should bump `seq` to that value before
    /// returning, or pass it through `with_initial_seq`. We don't
    /// query the provider here because that would couple the sync
    /// crate to whatever the provider's I/O strategy is.
    pub fn with_seq(mut self, provider: SeqProvider) -> Self {
        self.seq_provider = Some(provider);
        self
    }

    /// Seed the local seq counter — used in cluster mode after the
    /// provider is installed, so `current_seq()` returns the correct
    /// value before any `append` has fired. Idempotent (only bumps,
    /// never lowers).
    pub fn with_initial_seq(self, initial: u64) -> Self {
        {
            let mut seq = self.seq.lock().unwrap();
            if initial > *seq {
                *seq = initial;
            }
        }
        self
    }

    /// Returns true if this op_id was already applied to completion.
    /// Used by tests + observability surfaces. Push handlers should
    /// prefer `claim_op_id` which atomically transitions state.
    pub fn has_seen_op_id(&self, op_id: &str) -> bool {
        matches!(
            self.op_state.lock().unwrap().get(op_id),
            Some(OpEntry::Applied { .. })
        )
    }

    /// Atomic check-and-claim. Replaces the seen-set design that
    /// conflated "in flight" with "applied" — pre-fix, a concurrent
    /// retry that arrived while the first writer was mid-flush got
    /// deduped, and if the first writer then errored and called
    /// `forget_op_id`, the retry was lost forever. Now state is
    /// tristate:
    ///   - absent           → claimer wins, transitions to Pending
    ///   - present Pending  → another writer is mid-flight; return
    ///                        `InFlight` so the client knows to retry
    ///                        once that writer commits (or fails)
    ///   - present Applied  → previous write committed; return the
    ///                        cached seq so the client's optimistic
    ///                        row can adopt the canonical seq instead
    ///                        of waiting for the WS rebroadcast
    ///
    /// After `Proceed`, the caller MUST call either `complete_op_id`
    /// (on success, with the assigned seq) or `forget_op_id` (on
    /// failure). Forgetting clears the Pending entry so the client's
    /// retry has a clean state to claim. Codex P1.
    pub fn claim_op_id(&self, op_id: &str) -> OpClaim {
        let mut state = self.op_state.lock().unwrap();
        match state.get(op_id) {
            Some(OpEntry::Pending) => OpClaim::InFlight,
            Some(OpEntry::Applied { seq }) => OpClaim::Replayed { seq: *seq },
            None => {
                state.insert(op_id.to_string(), OpEntry::Pending);
                drop(state);
                let mut q = self.op_order.lock().unwrap();
                q.push_back(op_id.to_string());
                while q.len() > self.op_id_capacity {
                    if let Some(evicted) = q.pop_front() {
                        self.op_state.lock().unwrap().remove(&evicted);
                    }
                }
                OpClaim::Proceed
            }
        }
    }

    /// Mark a previously-claimed op_id as successfully applied at
    /// `seq`. Subsequent claims for the same op_id return
    /// `OpClaim::Replayed { seq }` so the client can advance its
    /// cursor / clear its optimistic ghost with the right seq.
    pub fn complete_op_id(&self, op_id: &str, seq: u64) {
        let mut state = self.op_state.lock().unwrap();
        // Always overwrite to Applied even if not Pending — a stale
        // pre-fix call site that called `remember_op_id` without first
        // claiming still ends up in the right terminal state.
        state.insert(op_id.to_string(), OpEntry::Applied { seq });
        if !self.op_order.lock().unwrap().iter().any(|s| s == op_id) {
            // Defensive: keep the FIFO + map in sync if a caller
            // somehow completed without claiming first.
            let mut q = self.op_order.lock().unwrap();
            q.push_back(op_id.to_string());
            while q.len() > self.op_id_capacity {
                if let Some(evicted) = q.pop_front() {
                    state.remove(&evicted);
                }
            }
        }
    }

    /// Roll back a `claim_op_id`. Called by push handlers when the
    /// downstream write failed — without this, the client's legitimate
    /// retry would be deduped away by the still-cached claim. Only
    /// removes Pending entries; Applied entries are immutable history.
    pub fn forget_op_id(&self, op_id: &str) {
        let mut state = self.op_state.lock().unwrap();
        if matches!(state.get(op_id), Some(OpEntry::Applied { .. })) {
            return;
        }
        state.remove(op_id);
        drop(state);
        let mut q = self.op_order.lock().unwrap();
        if let Some(pos) = q.iter().position(|s| s == op_id) {
            q.remove(pos);
        }
    }

    /// Current highest assigned sequence number. Reads the seq counter
    /// without consulting the events deque, so it's correct even when
    /// the oldest events have been evicted under capacity pressure.
    /// Returns 0 when no events have been appended.
    ///
    /// Used by the action HTTP handler to bracket "events generated
    /// during this action" (capture pre_seq, run action, capture
    /// post_seq) and emit a `X-Pylon-Change-Seq` header so the SDK
    /// can pull immediately if WS broadcast hasn't caught up yet.
    pub fn current_seq(&self) -> u64 {
        *self.seq.lock().unwrap()
    }

    /// Append a typed change record. Returns the full `ChangeEvent`
    /// (seq, entity, row_id, kind, data, timestamp) so callers can
    /// hand it straight to the broadcast notifier without re-
    /// assembling the fields and accidentally diverging from the log
    /// entry's shape.
    ///
    /// Prefer this over `append`: the typed variant catches shape
    /// errors (e.g. emitting an Insert without the post-insert row) at
    /// the call site instead of producing a partial event that the
    /// per-client policy filter then rejects, leaving subscribers
    /// silently desynced.
    pub fn record(&self, entity: &str, row_id: &str, change: ChangeRecord) -> ChangeEvent {
        let kind = change.kind();
        let (data, prev_data) = change.into_parts();
        self.append_with_prev(entity, row_id, kind, data, prev_data)
    }

    /// Append a change event. Returns the assigned sequence number.
    ///
    /// Prefer `record` or `append_with_prev` for new call sites —
    /// `append` is preserved for legacy code that doesn't have
    /// `prev_data` in scope.
    pub fn append(
        &self,
        entity: &str,
        row_id: &str,
        kind: ChangeKind,
        data: Option<serde_json::Value>,
    ) -> u64 {
        self.append_with_prev(entity, row_id, kind, data, None).seq
    }

    /// Append a change event with optional `prev_data`. Returns the
    /// full `ChangeEvent` so callers can hand it straight to the
    /// broadcaster.
    ///
    /// Lock order is `events` → `seq` (matches `pull()`'s order so
    /// concurrent append + pull don't deadlock).
    ///
    /// Invariant: `prev_data` is persisted in the retained log
    /// entry. /api/sync/pull's visibility-flip filter reads it to
    /// synthesize Delete tombstones for clients whose reconnect
    /// straddled the ownership transition. Pre-fix the field was
    /// hardcoded `None` in the stored event, so pull missed every
    /// tombstone.
    pub fn append_with_prev(
        &self,
        entity: &str,
        row_id: &str,
        kind: ChangeKind,
        data: Option<serde_json::Value>,
        prev_data: Option<serde_json::Value>,
    ) -> ChangeEvent {
        let mut events = self.events.lock().unwrap();
        let mut seq = self.seq.lock().unwrap();
        // External provider (typically Postgres SEQUENCE for cluster
        // mode) gives globally-monotonic seqs across instances.
        let new_seq = if let Some(provider) = self.seq_provider.as_ref() {
            let s = provider();
            if s > *seq {
                *seq = s;
            }
            s
        } else {
            *seq += 1;
            *seq
        };
        let event = ChangeEvent {
            seq: new_seq,
            entity: entity.to_string(),
            row_id: row_id.to_string(),
            kind,
            data,
            prev_data,
            timestamp: now_iso8601(),
        };
        let returned = event.clone();
        events.push_back(event);
        while events.len() > self.capacity {
            events.pop_front();
        }
        returned
    }

    /// Append a change event RECEIVED FROM A PEER over the cluster bus,
    /// preserving its original seq. The local seq counter advances to
    /// max(local_seq, event.seq) so `current_seq()` reflects the most
    /// recent globally-observed seq across all instances.
    ///
    /// Codex P1: pre-fix, peer events were rebroadcast to local WS/SSE
    /// clients without touching the local log. A client connected to
    /// machine B would advance its cursor to a seq machine A had
    /// assigned (say 1000) and then re-pulling from B (whose local
    /// counter was at 200) hit `cursor.last_seq > current_seq` and
    /// resync-required'd. Mirroring the peer's seq into the local log
    /// closes that gap: B's `current_seq` now reflects A's seq=1000
    /// once it's observed, and pulls return the correct tail.
    ///
    /// This is NOT a substitute for a truly globally-monotonic seq
    /// (Postgres SERIAL or similar) — concurrent appends on A and B
    /// can still produce duplicate seqs locally on each instance. But
    /// for the common case (cluster bus delivers in order, instances
    /// only write to their own log), it converges the visible seq
    /// space across nodes.
    pub fn append_peer(&self, event: ChangeEvent) {
        let mut events = self.events.lock().unwrap();
        let mut seq = self.seq.lock().unwrap();
        if event.seq > *seq {
            *seq = event.seq;
        }
        events.push_back(event);
        while events.len() > self.capacity {
            events.pop_front();
        }
    }

    /// Pull changes since a cursor, up to a limit.
    ///
    /// Returns `Err(PullError::ResyncRequired)` when the caller's cursor has
    /// fallen off the back of the retention window — i.e. the cursor's
    /// `last_seq` is lower than the oldest seq we still remember. Previously
    /// this case was silent: `pull` would return the surviving tail and
    /// advance the cursor, so the client converged to a state that skipped
    /// the evicted events entirely. That's a permanent correctness bug;
    /// clients should instead do a full re-sync from entity list state.
    pub fn pull(&self, cursor: &SyncCursor, limit: usize) -> Result<PullResponse, PullError> {
        let events = self.events.lock().unwrap();
        let current_seq = *self.seq.lock().unwrap();

        // Detect "cursor from a previous server lifetime": the caller's
        // cursor is ahead of the current seq counter. In-memory change logs
        // reset on process restart, so a client that persisted cursor=15
        // under the old server will silently tail-follow forever against
        // the new server (which starts at 0 and will never produce seqs
        // within (0, 15]). Force a resync so the client rehydrates from
        // the entity list endpoints.
        if cursor.last_seq > current_seq {
            return Err(PullError::ResyncRequired {
                oldest_seq: current_seq.saturating_add(1),
                cursor: cursor.clone(),
            });
        }

        // Detect "cursor too old": the caller's cursor is before the oldest
        // retained event by more than one seq. EXCEPT cursor=0 — a fresh
        // client gets whatever the log currently holds. The previous
        // policy 410'd cursor=0 whenever the seeded entity replay had
        // been evicted, which the React client handled by resetting
        // back to cursor=0 and re-pulling — an infinite loop. The
        // partial-tail risk the old comment warned about is real but
        // narrow: the runtime now also re-seeds entity rows on demand
        // (see `Runtime::seed_change_log`), so cursor=0 always gets a
        // current snapshot of state.
        if cursor.last_seq > 0 {
            if let Some(front) = events.front() {
                if cursor.last_seq + 1 < front.seq {
                    return Err(PullError::ResyncRequired {
                        oldest_seq: front.seq,
                        cursor: cursor.clone(),
                    });
                }
            }
        }

        let changes: Vec<ChangeEvent> = events
            .iter()
            .filter(|e| e.seq > cursor.last_seq)
            .take(limit)
            .cloned()
            .collect();

        let last_seq = changes.last().map(|e| e.seq).unwrap_or(cursor.last_seq);
        let has_more = events.iter().any(|e| e.seq > last_seq);

        Ok(PullResponse {
            changes,
            cursor: SyncCursor { last_seq },
            has_more,
        })
    }

    /// Get the total number of events in the log.
    pub fn len(&self) -> usize {
        self.events.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.lock().unwrap().is_empty()
    }
}

fn now_iso8601() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("{}Z", ts)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_log() {
        let log = ChangeLog::new();
        assert!(log.is_empty());
        assert_eq!(log.len(), 0);
    }

    #[test]
    fn append_and_pull() {
        let log = ChangeLog::new();
        log.append(
            "User",
            "u1",
            ChangeKind::Insert,
            Some(serde_json::json!({"name": "Alice"})),
        );
        log.append(
            "User",
            "u2",
            ChangeKind::Insert,
            Some(serde_json::json!({"name": "Bob"})),
        );

        assert_eq!(log.len(), 2);

        let resp = log.pull(&SyncCursor::beginning(), 100).unwrap();
        assert_eq!(resp.changes.len(), 2);
        assert_eq!(resp.cursor.last_seq, 2);
        assert!(!resp.has_more);
    }

    #[test]
    fn pull_with_cursor() {
        let log = ChangeLog::new();
        log.append("User", "u1", ChangeKind::Insert, None);
        log.append("User", "u2", ChangeKind::Insert, None);
        log.append("User", "u3", ChangeKind::Insert, None);

        // Pull from seq 1 — should get events 2 and 3.
        let resp = log.pull(&SyncCursor { last_seq: 1 }, 100).unwrap();
        assert_eq!(resp.changes.len(), 2);
        assert_eq!(resp.changes[0].seq, 2);
        assert_eq!(resp.changes[1].seq, 3);
    }

    #[test]
    fn pull_with_limit() {
        let log = ChangeLog::new();
        log.append("User", "u1", ChangeKind::Insert, None);
        log.append("User", "u2", ChangeKind::Insert, None);
        log.append("User", "u3", ChangeKind::Insert, None);

        let resp = log.pull(&SyncCursor::beginning(), 2).unwrap();
        assert_eq!(resp.changes.len(), 2);
        assert!(resp.has_more);
        assert_eq!(resp.cursor.last_seq, 2);

        // Continue pulling.
        let resp2 = log.pull(&resp.cursor, 2).unwrap();
        assert_eq!(resp2.changes.len(), 1);
        assert!(!resp2.has_more);
    }

    #[test]
    fn change_kinds() {
        let log = ChangeLog::new();
        log.append(
            "Todo",
            "t1",
            ChangeKind::Insert,
            Some(serde_json::json!({"title": "Test"})),
        );
        log.append(
            "Todo",
            "t1",
            ChangeKind::Update,
            Some(serde_json::json!({"title": "Updated"})),
        );
        log.append("Todo", "t1", ChangeKind::Delete, None);

        let resp = log.pull(&SyncCursor::beginning(), 100).unwrap();
        assert_eq!(resp.changes[0].kind, ChangeKind::Insert);
        assert_eq!(resp.changes[1].kind, ChangeKind::Update);
        assert_eq!(resp.changes[2].kind, ChangeKind::Delete);
        assert!(resp.changes[2].data.is_none());
    }

    #[test]
    fn sequence_numbers_are_monotonic() {
        let log = ChangeLog::new();
        let s1 = log.append("A", "1", ChangeKind::Insert, None);
        let s2 = log.append("B", "2", ChangeKind::Insert, None);
        let s3 = log.append("C", "3", ChangeKind::Insert, None);
        assert_eq!(s1, 1);
        assert_eq!(s2, 2);
        assert_eq!(s3, 3);
    }

    #[test]
    fn serialization_roundtrip() {
        let event = ChangeEvent {
            seq: 1,
            entity: "User".into(),
            row_id: "u1".into(),
            kind: ChangeKind::Insert,
            data: Some(serde_json::json!({"name": "Test"})),
            prev_data: None,
            timestamp: "2024-01-01T00:00:00Z".into(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: ChangeEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, parsed);
    }

    // -- Edge cases --

    #[test]
    fn pull_from_future_cursor_requires_resync() {
        // A cursor whose last_seq is greater than the log's current seq
        // counter is from a previous server lifetime (the in-memory log
        // reset on restart). The server must force resync — silently
        // returning an empty tail here used to wedge clients forever.
        let log = ChangeLog::new();
        log.append("User", "u1", ChangeKind::Insert, None);
        let err = log
            .pull(&SyncCursor { last_seq: 999 }, 100)
            .expect_err("future cursors must signal resync");
        match err {
            PullError::ResyncRequired { cursor, .. } => {
                assert_eq!(cursor.last_seq, 999);
            }
        }
    }

    #[test]
    fn pull_limit_zero_returns_empty() {
        let log = ChangeLog::new();
        log.append("User", "u1", ChangeKind::Insert, None);
        let resp = log.pull(&SyncCursor::beginning(), 0).unwrap();
        assert!(resp.changes.is_empty());
    }

    #[test]
    fn pull_with_evicted_cursor_requires_resync() {
        // Capacity 2 — we keep only the most recent 2. After seq 1..4 are
        // appended the oldest retained is seq 3.
        let log = ChangeLog::with_capacity(2);
        log.append("A", "1", ChangeKind::Insert, None);
        log.append("A", "2", ChangeKind::Insert, None);
        log.append("A", "3", ChangeKind::Insert, None);
        log.append("A", "4", ChangeKind::Insert, None);

        // Client knew up to seq 1 — seq 2 is unrecoverable, so RESYNC.
        let err = log.pull(&SyncCursor { last_seq: 1 }, 100).unwrap_err();
        match err {
            PullError::ResyncRequired { oldest_seq, .. } => {
                assert_eq!(oldest_seq, 3);
            }
        }
    }

    #[test]
    fn fresh_cursor_zero_never_resyncs() {
        // Regression: previously cursor=0 would 410 if the seeded entity
        // replay had been evicted, and the React client handled it by
        // resetting to cursor=0 and re-pulling — infinite loop. cursor=0
        // is "I just connected, give me what you have"; never 410.
        let log = ChangeLog::with_capacity(2);
        log.append("A", "1", ChangeKind::Insert, None);
        log.append("A", "2", ChangeKind::Insert, None);
        log.append("A", "3", ChangeKind::Insert, None);
        log.append("A", "4", ChangeKind::Insert, None);
        // Front is now seq 3 (1+2 evicted). Old behavior: 410 because
        // 0+1 < 3. New: succeed and return what we have.
        let resp = log
            .pull(&SyncCursor { last_seq: 0 }, 100)
            .expect("cursor=0 must never resync — no infinite loop");
        assert_eq!(resp.changes.len(), 2);
        assert_eq!(resp.changes[0].seq, 3);
    }

    #[test]
    fn pull_with_cursor_at_eviction_boundary_is_ok() {
        // Capacity 2 retains seq 2..3 after appending 1..3.
        let log = ChangeLog::with_capacity(2);
        log.append("A", "1", ChangeKind::Insert, None);
        log.append("A", "2", ChangeKind::Insert, None);
        log.append("A", "3", ChangeKind::Insert, None);
        // Client cursor=1, next event is seq 2 — exactly what we have.
        let resp = log.pull(&SyncCursor { last_seq: 1 }, 100).unwrap();
        assert_eq!(resp.changes.len(), 2);
    }

    #[test]
    fn delete_event_has_no_data() {
        let log = ChangeLog::new();
        log.append("User", "u1", ChangeKind::Delete, None);
        let resp = log.pull(&SyncCursor::beginning(), 100).unwrap();
        assert!(resp.changes[0].data.is_none());
    }

    #[test]
    fn concurrent_appends_get_unique_seqs() {
        let log = ChangeLog::new();
        let s1 = log.append("A", "1", ChangeKind::Insert, None);
        let s2 = log.append("A", "1", ChangeKind::Update, None);
        let s3 = log.append("A", "1", ChangeKind::Delete, None);
        assert!(s1 < s2);
        assert!(s2 < s3);
    }

    #[test]
    fn push_request_serialization() {
        let req = PushRequest {
            changes: vec![ClientChange {
                entity: "User".into(),
                row_id: "u1".into(),
                kind: ChangeKind::Insert,
                data: Some(serde_json::json!({"name": "Alice"})),
                op_id: None,
            }],
            client_id: Some("cl_123".into()),
        };
        let json = serde_json::to_string(&req).unwrap();
        let parsed: PushRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.changes.len(), 1);
        assert_eq!(parsed.changes[0].entity, "User");
        assert_eq!(parsed.client_id.as_deref(), Some("cl_123"));
    }

    #[test]
    fn push_request_accepts_missing_client_id() {
        // Legacy clients that don't send client_id must still parse.
        let json = r#"{"changes":[]}"#;
        let parsed: PushRequest = serde_json::from_str(json).unwrap();
        assert!(parsed.client_id.is_none());
    }

    // ---- op_id state machine ----

    // ---- typed change records ----

    #[test]
    fn record_insert_carries_full_row() {
        let log = ChangeLog::new();
        let event = log.record(
            "Todo",
            "t1",
            ChangeRecord::Insert {
                row: serde_json::json!({"id": "t1", "title": "Test"}),
            },
        );
        assert_eq!(event.seq, 1);
        assert_eq!(event.kind, ChangeKind::Insert);
        assert_eq!(event.data.as_ref().unwrap()["title"], "Test");
    }

    #[test]
    fn record_delete_carries_snapshot_for_policy_authz() {
        // Regression: tenant/owner-scoped delete events must include
        // the deleted row snapshot so WS subscribers' row-level read
        // policy can authorize delivery. A bare delete with None
        // forces entity-level policy and leaks "this row was here"
        // to clients that never saw the row.
        let log = ChangeLog::new();
        let event = log.record(
            "PrivateNote",
            "n1",
            ChangeRecord::Delete {
                snapshot: Some(serde_json::json!({"ownerId": "u1", "body": "secret"})),
            },
        );
        assert_eq!(event.kind, ChangeKind::Delete);
        assert!(event.data.is_some());
        assert_eq!(event.data.as_ref().unwrap()["ownerId"], "u1");
    }

    #[test]
    fn record_update_persists_prev_data_in_log() {
        // The retained event MUST carry prev_data so /api/sync/pull
        // can run the visibility-flip dual-check on reconnect /
        // missed-event recovery. Without persistence, pull saw an
        // Update without prev_data, denied it for the previous
        // owner, and never tombstoned the row.
        let log = ChangeLog::new();
        log.record(
            "Doc",
            "d1",
            ChangeRecord::Update {
                row: serde_json::json!({"id": "d1", "ownerId": "u_new"}),
                prev: Some(serde_json::json!({"id": "d1", "ownerId": "u_old"})),
            },
        );
        let resp = log.pull(&SyncCursor::beginning(), 100).unwrap();
        assert_eq!(resp.changes.len(), 1);
        let stored = &resp.changes[0];
        assert_eq!(stored.kind, ChangeKind::Update);
        assert_eq!(
            stored.prev_data.as_ref().unwrap()["ownerId"],
            "u_old",
            "prev_data must round-trip through the retained log",
        );
    }

    #[test]
    fn record_delete_with_none_snapshot_is_explicit() {
        // Permitted shape: a delete whose pre-snapshot was unrecoverable
        // (row was already gone, tenant purge, etc.). Compiles without
        // friction so call sites that genuinely can't supply a snapshot
        // can express that intent — and reviewers can flag any
        // gratuitous `snapshot: None` that should have captured the row.
        let log = ChangeLog::new();
        let event = log.record("Todo", "t1", ChangeRecord::Delete { snapshot: None });
        assert_eq!(event.kind, ChangeKind::Delete);
        assert!(event.data.is_none());
    }

    #[test]
    fn op_id_first_claim_proceeds() {
        let log = ChangeLog::new();
        match log.claim_op_id("op-1") {
            OpClaim::Proceed => {}
            other => panic!("expected Proceed, got {other:?}"),
        }
        // Still Pending until complete.
        assert!(!log.has_seen_op_id("op-1"));
    }

    #[test]
    fn op_id_concurrent_retry_during_pending_returns_inflight() {
        // Codex P1: pre-fix this returned `false` (deduped + claimed),
        // and if the first writer then errored, the retry was lost
        // because the claim was rolled back AFTER the retry had been
        // told "you're already done". InFlight lets the client try
        // again later.
        let log = ChangeLog::new();
        assert!(matches!(log.claim_op_id("op-1"), OpClaim::Proceed));
        match log.claim_op_id("op-1") {
            OpClaim::InFlight => {}
            other => panic!("expected InFlight while Pending, got {other:?}"),
        }
    }

    #[test]
    fn op_id_replay_after_apply_returns_cached_seq() {
        // The whole point of the redesign: a retry that arrives AFTER
        // the first write committed should learn the seq so the
        // client's optimistic ghost can adopt the canonical seq rather
        // than waiting for the WS rebroadcast.
        let log = ChangeLog::new();
        assert!(matches!(log.claim_op_id("op-1"), OpClaim::Proceed));
        log.complete_op_id("op-1", 42);
        match log.claim_op_id("op-1") {
            OpClaim::Replayed { seq: 42 } => {}
            other => panic!("expected Replayed{{42}}, got {other:?}"),
        }
        assert!(log.has_seen_op_id("op-1"));
    }

    #[test]
    fn op_id_forget_clears_pending_for_retry() {
        // Failure path: the writer errored, forget() runs, the next
        // claim from the client's retry must succeed (Proceed) — not
        // be silently swallowed as a stale claim.
        let log = ChangeLog::new();
        assert!(matches!(log.claim_op_id("op-1"), OpClaim::Proceed));
        log.forget_op_id("op-1");
        match log.claim_op_id("op-1") {
            OpClaim::Proceed => {}
            other => panic!("expected Proceed after forget, got {other:?}"),
        }
    }

    #[test]
    fn op_id_forget_does_not_clear_applied() {
        // Once an op_id is Applied it's immutable history — a stray
        // forget call (e.g. error after a successful broadcast) must
        // not let the client re-apply by retrying.
        let log = ChangeLog::new();
        assert!(matches!(log.claim_op_id("op-1"), OpClaim::Proceed));
        log.complete_op_id("op-1", 7);
        log.forget_op_id("op-1");
        match log.claim_op_id("op-1") {
            OpClaim::Replayed { seq: 7 } => {}
            other => panic!("Applied must survive forget, got {other:?}"),
        }
    }
}
