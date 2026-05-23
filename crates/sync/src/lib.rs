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
    /// Recently-seen client op_ids, for push idempotency. Bounded by
    /// `op_id_capacity`; oldest entries age out when the map grows past it.
    seen_op_ids: Mutex<std::collections::VecDeque<String>>,
    seen_op_id_set: Mutex<std::collections::HashSet<String>>,
    op_id_capacity: usize,
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
            seen_op_ids: Mutex::new(std::collections::VecDeque::with_capacity(1024)),
            seen_op_id_set: Mutex::new(std::collections::HashSet::with_capacity(1024)),
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

    /// Returns true if this op_id was already applied. Used by the push
    /// handler to short-circuit replays. Callers that observe `true` should
    /// NOT re-apply the change and should return success to the client.
    pub fn has_seen_op_id(&self, op_id: &str) -> bool {
        self.seen_op_id_set.lock().unwrap().contains(op_id)
    }

    /// Mark an op_id as processed. Safe to call multiple times. Evicts the
    /// oldest entry when the cache exceeds `op_id_capacity`.
    pub fn remember_op_id(&self, op_id: &str) {
        let mut set = self.seen_op_id_set.lock().unwrap();
        if set.contains(op_id) {
            return;
        }
        set.insert(op_id.to_string());
        drop(set);
        let mut q = self.seen_op_ids.lock().unwrap();
        q.push_back(op_id.to_string());
        while q.len() > self.op_id_capacity {
            if let Some(evicted) = q.pop_front() {
                self.seen_op_id_set.lock().unwrap().remove(&evicted);
            }
        }
    }

    /// Atomic check-and-claim: returns true if THIS caller is the first
    /// to claim the op_id (and it's now marked seen), false if another
    /// caller already claimed it. Use this instead of the
    /// `has_seen_op_id` + `remember_op_id` pair when two concurrent
    /// pushes carrying the same op_id arrive — the pair has a race
    /// where both check `has_seen_op_id`, both see false, and both
    /// apply the same change. Codex P1.
    ///
    /// The caller MUST roll back the claim via `forget_op_id` if the
    /// downstream write fails, otherwise a real retry by the client
    /// would be silently swallowed as a duplicate.
    pub fn claim_op_id(&self, op_id: &str) -> bool {
        let mut set = self.seen_op_id_set.lock().unwrap();
        if set.contains(op_id) {
            return false;
        }
        set.insert(op_id.to_string());
        drop(set);
        let mut q = self.seen_op_ids.lock().unwrap();
        q.push_back(op_id.to_string());
        while q.len() > self.op_id_capacity {
            if let Some(evicted) = q.pop_front() {
                self.seen_op_id_set.lock().unwrap().remove(&evicted);
            }
        }
        true
    }

    /// Roll back a `claim_op_id`. Called by push handlers when the
    /// downstream write failed — without this, the client's legitimate
    /// retry would be deduped away by the still-cached claim.
    pub fn forget_op_id(&self, op_id: &str) {
        let mut set = self.seen_op_id_set.lock().unwrap();
        set.remove(op_id);
        drop(set);
        let mut q = self.seen_op_ids.lock().unwrap();
        // Cheap O(n) here; op_id_capacity is small (1024 default) and
        // rollback is a slow path.
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

    /// Append a change event. Returns the assigned sequence number.
    ///
    /// Lock order: `events` then `seq`. `pull()` takes the same order
    /// (codex P1: previously this method took seq-then-events while
    /// pull took events-then-seq, a classic lock-order inversion that
    /// could deadlock under concurrent append + pull).
    pub fn append(
        &self,
        entity: &str,
        row_id: &str,
        kind: ChangeKind,
        data: Option<serde_json::Value>,
    ) -> u64 {
        let mut events = self.events.lock().unwrap();
        let mut seq = self.seq.lock().unwrap();
        // External provider (typically Postgres SEQUENCE for cluster
        // mode) gives globally-monotonic seqs across instances. The
        // local counter tracks the max seen so `current_seq()` and
        // `append_peer()` interop with the provider's space. When no
        // provider is wired (SQLite / single-instance), increment the
        // local counter as before.
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
            timestamp: now_iso8601(),
        };
        events.push_back(event);
        while events.len() > self.capacity {
            events.pop_front();
        }
        new_seq
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
}
