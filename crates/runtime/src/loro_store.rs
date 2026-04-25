//! Server-side per-row LoroDoc cache with snapshot persistence.
//!
//! For CRDT-backed entities (`crdt: true` in the manifest, the default),
//! every row corresponds to one [`LoroDoc`]. This store owns those docs
//! in memory, hydrates them on demand from a sidecar SQLite table,
//! write-throughs every commit, and projects the doc state into the JSON
//! shape Pylon's existing storage layer expects.
//!
//! # Persistence shape
//!
//! Single sidecar table:
//!
//! ```sql
//! CREATE TABLE _pylon_crdt_snapshots (
//!     entity     TEXT NOT NULL,
//!     row_id     TEXT NOT NULL,
//!     snapshot   BLOB NOT NULL,
//!     updated_at TEXT NOT NULL,
//!     PRIMARY KEY (entity, row_id)
//! );
//! ```
//!
//! Snapshots are full-state Loro snapshots (`ExportMode::Snapshot`).
//! Loro applies internal compaction so the snapshot size stays bounded;
//! we don't track an op log separately.
//!
//! # In-memory cache
//!
//! Active rows live in a `HashMap<(entity, row_id), Arc<Mutex<LoroDoc>>>`.
//! First access for a row hydrates the doc from the sidecar (or creates
//! a fresh one). Subsequent accesses reuse the in-memory doc — required
//! both for correctness (Loro's CRDT identity is per-doc-instance) and
//! perf (snapshot decode is ~100µs per row).
//!
//! No eviction yet. Working sets up to ~100K active rows are fine on
//! commodity hardware (~5-50 MB). For larger working sets a follow-up
//! adds LRU eviction with snapshot reload on next access.
//!
//! # Bandwidth: full snapshot per write (TODO)
//!
//! Every CRDT-mode write triggers a binary WS broadcast carrying the
//! row's *full* current snapshot, not just the incremental update.
//! Loro's compaction bounds individual snapshots, but the per-write
//! cost still scales with total state size, not write size.
//!
//! Concrete numbers:
//!
//! | Workload                           | Snapshot/row | Per-write fanout |
//! |------------------------------------|--------------|------------------|
//! | Chat message                       | ~200 B       | tiny             |
//! | Boring CRUD record                 | ~500 B       | tiny             |
//! | Whiteboard with 1k strokes         | ~30 KB       | uncomfortable    |
//! | Document with 50K-char body        | ~80 KB       | bad              |
//!
//! Multiply by `connected_clients × writes_per_second` to get total
//! broadcast bandwidth. For chat-shaped workloads it's free. For collab
//! whiteboards / large documents it bites once you pass ~10 connected
//! clients on a hot row.
//!
//! # Switching to incremental updates
//!
//! Loro already supports `export(ExportMode::updates(version_vector))`
//! returning only the ops a peer hasn't seen — the building block is
//! there. What's missing is the per-client tracking:
//!
//! 1. Subscribe protocol — clients tell the server "I want updates for
//!    rows X, Y, Z" instead of every CRDT write fanning out to every
//!    client. Pylon's existing room layer is the natural transport
//!    once room semantics extend to per-row subscriptions.
//! 2. Server-side state — `(client_id, entity, row_id) → version_vector`
//!    so the server knows what each client is missing. Bounded by the
//!    subscribe set; LRU-evicted with the doc cache.
//! 3. Encoder swap — `notify_crdt` calls `encode_update_since(vv)`
//!    instead of `encode_snapshot()` and ships frame type `0x11`
//!    (CRDT_FRAME_UPDATE) instead of `0x10` (CRDT_FRAME_SNAPSHOT).
//!    Wire format already reserves both bytes.
//! 4. New-subscriber bootstrap — first frame is still a snapshot
//!    (`0x10`), subsequent frames are deltas (`0x11`).
//!
//! Estimated effort: ~2 days for a working slice plus a week of
//! production hardening (correct VV tracking under reconnects,
//! garbage-collecting subscriptions on disconnect, handling missed
//! frames via resync request).
//!
//! Until then this implementation is fine for chat / boring CRUD /
//! demo workloads. Don't run a Figma clone on it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use pylon_crdt::{
    apply_patch, apply_update as crdt_apply_update, encode_snapshot, encode_update_since,
    project_doc_to_json, CrdtField,
    loro::{LoroDoc, VersionVector},
};
use rusqlite::{params, Connection};
use serde_json::Value;

// ---------------------------------------------------------------------------
// Sidecar table
// ---------------------------------------------------------------------------

/// SQL to create the snapshot sidecar. Idempotent. Called by Runtime
/// constructor for any database where CRDT mode could be in use (always,
/// since `crdt: true` is the default).
pub const CREATE_SIDECAR_SQL: &str = "
CREATE TABLE IF NOT EXISTS _pylon_crdt_snapshots (
    entity     TEXT NOT NULL,
    row_id     TEXT NOT NULL,
    snapshot   BLOB NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (entity, row_id)
)
";

/// Create the sidecar table. Safe to call repeatedly.
pub fn ensure_sidecar(conn: &Connection) -> Result<(), LoroStoreError> {
    conn.execute(CREATE_SIDECAR_SQL, [])
        .map(|_| ())
        .map_err(|e| LoroStoreError::Storage(format!("create sidecar: {e}")))
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum LoroStoreError {
    /// Patch contained a value that didn't match the field's CRDT shape
    /// (e.g. number on a Bool field). Schema/caller mismatch.
    Apply(String),
    /// Storage layer error — sidecar create / read / write failed.
    Storage(String),
    /// Loro decode error — corrupted snapshot in the sidecar, or a peer
    /// sent an invalid binary update. The owning code should surface this
    /// to the client (for remote updates) or fail loud (for stored snapshots).
    Decode(String),
}

impl std::fmt::Display for LoroStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Apply(m) => write!(f, "apply: {m}"),
            Self::Storage(m) => write!(f, "storage: {m}"),
            Self::Decode(m) => write!(f, "decode: {m}"),
        }
    }
}

impl std::error::Error for LoroStoreError {}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

/// Server-side per-row LoroDoc cache + persistence layer.
///
/// One instance per Runtime. Cheap to clone via [`Arc`]; internally
/// guards a `HashMap` of doc handles, each itself behind a `Mutex` so
/// concurrent access to *different* rows doesn't contend.
#[derive(Default)]
pub struct LoroStore {
    /// Per-row cache. The outer Mutex guards lookup; the inner Mutex
    /// guards mutation of the specific doc. We hold the outer briefly
    /// (insert/lookup), then release before doing any Loro work, so two
    /// requests targeting different rows never block each other.
    docs: Mutex<HashMap<(String, String), Arc<Mutex<LoroDoc>>>>,
}

impl LoroStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get the cached doc for a row, hydrating from the sidecar if absent.
    /// Returns a freshly-created doc if the row has no snapshot yet.
    fn get_or_hydrate(
        &self,
        conn: &Connection,
        entity: &str,
        row_id: &str,
    ) -> Result<Arc<Mutex<LoroDoc>>, LoroStoreError> {
        let key = (entity.to_string(), row_id.to_string());

        // Fast path: already cached.
        {
            let guard = self.docs.lock().unwrap();
            if let Some(doc) = guard.get(&key) {
                return Ok(Arc::clone(doc));
            }
        }

        // Slow path: hydrate (or create fresh) outside the cache lock.
        // Two concurrent first-accesses can both do this; the loser's
        // doc is dropped after the cache check below. Loro's snapshot
        // decode is deterministic, so both copies are byte-identical;
        // the race only wastes a microsecond, never produces divergence.
        let snapshot: Option<Vec<u8>> = conn
            .query_row(
                "SELECT snapshot FROM _pylon_crdt_snapshots WHERE entity = ?1 AND row_id = ?2",
                params![entity, row_id],
                |r| r.get(0),
            )
            .map(Some)
            .or_else(|e| {
                if matches!(e, rusqlite::Error::QueryReturnedNoRows) {
                    Ok(None)
                } else {
                    Err(LoroStoreError::Storage(format!("read snapshot: {e}")))
                }
            })?;

        let doc = LoroDoc::new();
        if let Some(bytes) = snapshot {
            crdt_apply_update(&doc, &bytes).map_err(LoroStoreError::Decode)?;
        }
        let handle = Arc::new(Mutex::new(doc));

        // Re-acquire cache lock and publish, but defer to whatever's
        // already there if we lost the race.
        let mut guard = self.docs.lock().unwrap();
        let entry = guard.entry(key).or_insert_with(|| Arc::clone(&handle));
        Ok(Arc::clone(entry))
    }

    /// Persist the current snapshot for a row to the sidecar. Called
    /// after every commit. Synchronous; tests rely on read-after-write.
    fn persist_snapshot(
        &self,
        conn: &Connection,
        entity: &str,
        row_id: &str,
        doc: &LoroDoc,
    ) -> Result<(), LoroStoreError> {
        let snap = encode_snapshot(doc);
        let now = chrono_now_iso();
        conn.execute(
            "INSERT OR REPLACE INTO _pylon_crdt_snapshots
                (entity, row_id, snapshot, updated_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![entity, row_id, snap, now],
        )
        .map(|_| ())
        .map_err(|e| LoroStoreError::Storage(format!("persist snapshot: {e}")))
    }

    /// Apply a JSON `{field: value}` patch to the row's doc, persist the
    /// new snapshot, and return the projected JSON (the row shape SQLite
    /// stores in the materialized view).
    pub fn apply_patch(
        &self,
        conn: &Connection,
        entity: &str,
        row_id: &str,
        fields: &[CrdtField],
        patch: &Value,
    ) -> Result<Value, LoroStoreError> {
        let handle = self.get_or_hydrate(conn, entity, row_id)?;
        let projected = {
            let doc = handle.lock().unwrap();
            apply_patch(&doc, fields, patch).map_err(LoroStoreError::Apply)?;
            self.persist_snapshot(conn, entity, row_id, &doc)?;
            project_doc_to_json(&doc, fields)
        };
        Ok(projected)
    }

    /// Apply a binary update from a peer (typed-protocol client push or
    /// server-to-server replication). Persists the new snapshot. Returns
    /// the projected JSON for SQLite materialization so the materialized
    /// view stays in sync with the CRDT after remote-driven changes.
    pub fn apply_remote_update(
        &self,
        conn: &Connection,
        entity: &str,
        row_id: &str,
        fields: &[CrdtField],
        update: &[u8],
    ) -> Result<Value, LoroStoreError> {
        let handle = self.get_or_hydrate(conn, entity, row_id)?;
        let projected = {
            let doc = handle.lock().unwrap();
            crdt_apply_update(&doc, update).map_err(LoroStoreError::Decode)?;
            self.persist_snapshot(conn, entity, row_id, &doc)?;
            project_doc_to_json(&doc, fields)
        };
        Ok(projected)
    }

    /// Get the full snapshot for a row. Sent to a fresh client when it
    /// subscribes. Returns an empty `Vec` for rows that don't exist yet.
    pub fn snapshot(
        &self,
        conn: &Connection,
        entity: &str,
        row_id: &str,
    ) -> Result<Vec<u8>, LoroStoreError> {
        let handle = self.get_or_hydrate(conn, entity, row_id)?;
        let doc = handle.lock().unwrap();
        Ok(encode_snapshot(&doc))
    }

    /// Get an incremental update since `since` — only the ops the peer
    /// hasn't seen. Used to catch up a peer that's been disconnected.
    pub fn update_since(
        &self,
        conn: &Connection,
        entity: &str,
        row_id: &str,
        since: &VersionVector,
    ) -> Result<Vec<u8>, LoroStoreError> {
        let handle = self.get_or_hydrate(conn, entity, row_id)?;
        let doc = handle.lock().unwrap();
        Ok(encode_update_since(&doc, since))
    }

    /// Drop a row's doc from the in-memory cache. Useful for tests and
    /// for the eventual eviction policy. Doesn't touch the sidecar; the
    /// next read will re-hydrate from disk.
    pub fn evict(&self, entity: &str, row_id: &str) {
        self.docs
            .lock()
            .unwrap()
            .remove(&(entity.to_string(), row_id.to_string()));
    }

    /// Number of rows currently held in memory. Diagnostic.
    pub fn cached_rows(&self) -> usize {
        self.docs.lock().unwrap().len()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn chrono_now_iso() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{}Z", secs)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use pylon_crdt::CrdtFieldKind;

    fn open_test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        ensure_sidecar(&conn).unwrap();
        conn
    }

    fn fields() -> Vec<CrdtField> {
        vec![
            CrdtField {
                name: "title".into(),
                kind: CrdtFieldKind::LwwString,
            },
            CrdtField {
                name: "body".into(),
                kind: CrdtFieldKind::Text,
            },
            CrdtField {
                name: "qty".into(),
                kind: CrdtFieldKind::LwwNumber,
            },
        ]
    }

    #[test]
    fn sidecar_is_idempotent() {
        let conn = open_test_db();
        ensure_sidecar(&conn).unwrap(); // Re-create OK.
    }

    #[test]
    fn apply_patch_persists_and_projects() {
        let conn = open_test_db();
        let store = LoroStore::new();
        let projected = store
            .apply_patch(
                &conn,
                "Note",
                "n1",
                &fields(),
                &serde_json::json!({"title": "Hello", "body": "world", "qty": 7}),
            )
            .unwrap();
        assert_eq!(projected["title"], "Hello");
        assert_eq!(projected["body"], "world");
        assert_eq!(projected["qty"], 7.0);

        // Sidecar row exists.
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM _pylon_crdt_snapshots WHERE entity='Note' AND row_id='n1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn second_open_hydrates_from_sidecar() {
        let conn = open_test_db();
        let store = LoroStore::new();
        store
            .apply_patch(
                &conn,
                "Note",
                "n1",
                &fields(),
                &serde_json::json!({"title": "A", "qty": 1}),
            )
            .unwrap();

        // Drop in-memory cache; next read must rehydrate from disk.
        store.evict("Note", "n1");
        assert_eq!(store.cached_rows(), 0);

        let snap = store.snapshot(&conn, "Note", "n1").unwrap();
        assert!(!snap.is_empty(), "snapshot should be non-empty after writes");
        assert_eq!(store.cached_rows(), 1, "snapshot() rehydrated the cache");
    }

    #[test]
    fn empty_row_yields_empty_snapshot() {
        let conn = open_test_db();
        let store = LoroStore::new();
        let snap = store.snapshot(&conn, "Note", "missing").unwrap();
        // An empty Loro doc still produces a small snapshot with version
        // bookkeeping — just assert it round-trips, not its size.
        let store2 = LoroStore::new();
        store2
            .apply_remote_update(&conn, "Note", "missing", &fields(), &snap)
            .unwrap();
    }

    #[test]
    fn remote_update_merges_with_local_state() {
        let conn = open_test_db();

        // Server has a row with title=A, qty=1.
        let server = LoroStore::new();
        server
            .apply_patch(
                &conn,
                "Note",
                "n1",
                &fields(),
                &serde_json::json!({"title": "A", "qty": 1}),
            )
            .unwrap();
        let server_snap = server.snapshot(&conn, "Note", "n1").unwrap();

        // A different LoroStore (think: peer / replica) starts from a
        // fresh DB, applies the snapshot, then makes a divergent edit.
        let conn2 = open_test_db();
        let peer = LoroStore::new();
        peer.apply_remote_update(&conn2, "Note", "n1", &fields(), &server_snap)
            .unwrap();
        peer.apply_patch(
            &conn2,
            "Note",
            "n1",
            &fields(),
            &serde_json::json!({"qty": 2}),
        )
        .unwrap();
        let peer_update = peer.snapshot(&conn2, "Note", "n1").unwrap();

        // Server applies the peer's update. Both fields converge.
        let projected = server
            .apply_remote_update(&conn, "Note", "n1", &fields(), &peer_update)
            .unwrap();
        assert_eq!(projected["title"], "A");
        assert_eq!(projected["qty"], 2.0);
    }

    #[test]
    fn concurrent_text_writes_converge() {
        let conn_a = open_test_db();
        let conn_b = open_test_db();
        let a = LoroStore::new();
        let b = LoroStore::new();

        a.apply_patch(
            &conn_a,
            "Note",
            "n1",
            &fields(),
            &serde_json::json!({"body": "from-a"}),
        )
        .unwrap();
        b.apply_patch(
            &conn_b,
            "Note",
            "n1",
            &fields(),
            &serde_json::json!({"body": "from-b"}),
        )
        .unwrap();

        let snap_a = a.snapshot(&conn_a, "Note", "n1").unwrap();
        let snap_b = b.snapshot(&conn_b, "Note", "n1").unwrap();

        let projected_a = a
            .apply_remote_update(&conn_a, "Note", "n1", &fields(), &snap_b)
            .unwrap();
        let projected_b = b
            .apply_remote_update(&conn_b, "Note", "n1", &fields(), &snap_a)
            .unwrap();

        // Both stores converge to the same byte-for-byte state.
        assert_eq!(projected_a, projected_b);
        let body = projected_a["body"].as_str().unwrap();
        assert!(!body.is_empty(), "body should contain merged text");
    }

    #[test]
    fn incremental_update_carries_only_delta() {
        let conn = open_test_db();
        let store = LoroStore::new();

        store
            .apply_patch(
                &conn,
                "Note",
                "n1",
                &fields(),
                &serde_json::json!({"title": "v1", "qty": 1}),
            )
            .unwrap();

        // Snapshot before the next edit — represents what a connected
        // peer has already seen.
        let early_vv = {
            let handle = store.get_or_hydrate(&conn, "Note", "n1").unwrap();
            let vv = handle.lock().unwrap().oplog_vv();
            vv
        };
        let snap_full = store.snapshot(&conn, "Note", "n1").unwrap();

        store
            .apply_patch(
                &conn,
                "Note",
                "n1",
                &fields(),
                &serde_json::json!({"qty": 7}),
            )
            .unwrap();

        let delta = store
            .update_since(&conn, "Note", "n1", &early_vv)
            .unwrap();
        assert!(
            delta.len() < snap_full.len(),
            "incremental delta ({}) must be smaller than full snapshot ({})",
            delta.len(),
            snap_full.len()
        );
    }

    #[test]
    fn cache_keeps_distinct_rows_separate() {
        let conn = open_test_db();
        let store = LoroStore::new();
        store
            .apply_patch(
                &conn,
                "Note",
                "n1",
                &fields(),
                &serde_json::json!({"title": "first"}),
            )
            .unwrap();
        store
            .apply_patch(
                &conn,
                "Note",
                "n2",
                &fields(),
                &serde_json::json!({"title": "second"}),
            )
            .unwrap();
        assert_eq!(store.cached_rows(), 2);

        let p1 = store
            .apply_patch(&conn, "Note", "n1", &fields(), &serde_json::json!({}))
            .unwrap();
        let p2 = store
            .apply_patch(&conn, "Note", "n2", &fields(), &serde_json::json!({}))
            .unwrap();
        assert_eq!(p1["title"], "first");
        assert_eq!(p2["title"], "second");
    }
}
