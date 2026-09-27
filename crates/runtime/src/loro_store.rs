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
//! # Bandwidth: incremental deltas after the first broadcast
//!
//! The first CRDT-mode write to a row ships the full snapshot (so
//! existing + freshly-subscribed clients agree on a baseline). Every
//! subsequent write computes an incremental delta from the last
//! broadcast's Loro VV and ships THAT. Loro's import is idempotent
//! so a client that just received the snapshot can also apply the
//! next delta cleanly (the ops already in their snapshot are no-ops
//! on import).
//!
//! Shape:
//!
//! | Workload                           | Snapshot/row | Delta/write   |
//! |------------------------------------|--------------|---------------|
//! | Chat message append                | ~200 B       | ~80 B         |
//! | Boring CRUD field change           | ~500 B       | ~120 B        |
//! | Whiteboard one-stroke add          | ~30 KB       | ~150 B        |
//! | Document one-char insert           | ~80 KB       | ~60 B         |
//!
//! `LoroStore::current_vv_bytes` + `LoroStore::update_since_bytes`
//! are the helpers the broadcast path uses; the router's
//! `broadcast_change_with_crdt` owns the per-row "last VV" map.

use std::sync::{Arc, Mutex};

use pylon_crdt::{
    apply_patch, apply_update as crdt_apply_update, encode_snapshot, encode_update_since,
    loro::{LoroDoc, VersionVector},
    project_doc_to_json, CrdtField,
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

/// Which snapshots a prune batch removes for one entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PruneScope {
    /// Snapshots whose row no longer exists in the entity table.
    Orphans,
    /// Every snapshot of the entity: it is no longer CRDT-backed, or no
    /// longer in the manifest.
    All,
}

/// Delete up to `batch` snapshots of `entity` in `scope`, with the rows'
/// server-side records; returns how many. Callers loop until it returns 0,
/// taking the write lock per batch.
pub fn prune_batch(
    conn: &Connection,
    entity: &str,
    scope: PruneScope,
    batch: usize,
) -> Result<usize, LoroStoreError> {
    let sql = match scope {
        PruneScope::Orphans => format!(
            "SELECT s.row_id FROM _pylon_crdt_snapshots s
             WHERE s.entity = ?1
               AND NOT EXISTS (SELECT 1 FROM {} t WHERE t.\"id\" = s.row_id)
             LIMIT ?2",
            quote_sqlite_ident(entity)
        ),
        PruneScope::All => {
            "SELECT row_id FROM _pylon_crdt_snapshots WHERE entity = ?1 LIMIT ?2".to_string()
        }
    };
    let err =
        |e: rusqlite::Error| LoroStoreError::Storage(format!("prune snapshots of {entity}: {e}"));
    let row_ids: Vec<String> = conn
        .prepare(&sql)
        .map_err(err)?
        .query_map(params![entity, batch as i64], |r| r.get(0))
        .map_err(err)?
        .collect::<Result<_, _>>()
        .map_err(err)?;
    let tx = conn.unchecked_transaction().map_err(err)?;
    for row_id in &row_ids {
        for table in std::iter::once("_pylon_crdt_snapshots").chain(ROW_SIDE_TABLES) {
            tx.prepare_cached(&format!(
                "DELETE FROM {table} WHERE entity = ?1 AND row_id = ?2"
            ))
            .and_then(|mut stmt| stmt.execute(params![entity, row_id]))
            .map_err(|e| LoroStoreError::Storage(format!("prune {table} of {entity}: {e}")))?;
        }
    }
    tx.commit().map_err(err)?;
    Ok(row_ids.len())
}

/// Entities that have at least one snapshot.
pub fn snapshot_entities(conn: &Connection) -> Result<Vec<String>, LoroStoreError> {
    let mut stmt = conn
        .prepare("SELECT DISTINCT entity FROM _pylon_crdt_snapshots")
        .map_err(|e| LoroStoreError::Storage(format!("list snapshot entities: {e}")))?;
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| LoroStoreError::Storage(format!("list snapshot entities: {e}")))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| LoroStoreError::Storage(format!("list snapshot entities: {e}")))
}

fn quote_sqlite_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Create the sidecar table. Safe to call repeatedly.
pub fn ensure_sidecar(conn: &Connection) -> Result<(), LoroStoreError> {
    conn.execute(CREATE_SIDECAR_SQL, [])
        .map(|_| ())
        .map_err(|e| LoroStoreError::Storage(format!("create sidecar: {e}")))?;
    // The peers of each row's synthetic ops (see `as_synthetic`), kept
    // beside the snapshot: server-side state, which a client's update
    // cannot change.
    conn.execute(
        "CREATE TABLE IF NOT EXISTS _pylon_crdt_synthetic (
            entity TEXT NOT NULL,
            row_id TEXT NOT NULL,
            peer INTEGER NOT NULL,
            PRIMARY KEY (entity, row_id, peer)
        )",
        [],
    )
    .map(|_| ())
    .map_err(|e| LoroStoreError::Storage(format!("create the synthetic peer table: {e}")))?;
    // Tables an earlier build of the container merge used; no release
    // shipped them.
    for table in ["_pylon_crdt_written", "_pylon_crdt_merged"] {
        conn.execute(&format!("DROP TABLE IF EXISTS {table}"), [])
            .map_err(|e| LoroStoreError::Storage(format!("drop {table}: {e}")))?;
    }
    // Per text, list, or tree field: the server's last whole write of it
    // (`pylon_crdt::merge::BaseWrite`, JSON).
    conn.execute(
        "CREATE TABLE IF NOT EXISTS _pylon_crdt_base (
            entity TEXT NOT NULL,
            row_id TEXT NOT NULL,
            field TEXT NOT NULL,
            base TEXT NOT NULL,
            PRIMARY KEY (entity, row_id, field)
        )",
        [],
    )
    .map(|_| ())
    .map_err(|e| LoroStoreError::Storage(format!("create the base write table: {e}")))?;
    // Per container merged into the one holding its field's key: its links
    // into that one (`pylon_crdt::merge::Links`, JSON).
    conn.execute(
        "CREATE TABLE IF NOT EXISTS _pylon_crdt_links (
            entity TEXT NOT NULL,
            row_id TEXT NOT NULL,
            source TEXT NOT NULL,
            links TEXT NOT NULL,
            PRIMARY KEY (entity, row_id, source)
        )",
        [],
    )
    .map(|_| ())
    .map_err(|e| LoroStoreError::Storage(format!("create the link table: {e}")))
}

/// The server-side tables kept per CRDT row beside its snapshot.
pub(crate) const ROW_SIDE_TABLES: [&str; 3] = [
    "_pylon_crdt_synthetic",
    "_pylon_crdt_base",
    "_pylon_crdt_links",
];

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

/// Lift a `DataError` into a `LoroStoreError::Storage` so closures
/// passed to `PostgresDataStore::with_client` can return
/// `LoroStoreError` directly. The `with_client` bound requires
/// `E: From<DataError>` so the lock-poisoning case can fan out into
/// the caller's error type.
impl From<pylon_http::DataError> for LoroStoreError {
    fn from(e: pylon_http::DataError) -> Self {
        LoroStoreError::Storage(format!("[{}] {}", e.code, e.message))
    }
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

/// A random peer id (Loro reserves `u64::MAX`).
fn fresh_peer() -> u64 {
    loop {
        let peer: u64 = rand::random();
        if peer != u64::MAX {
            return peer;
        }
    }
}

/// Run `write` as a synthetic write (a doc created or brought in line from
/// its row, or a merge after a client's update) under `peer`, then go back
/// to the doc's own peer. The caller records `peer` in
/// `_pylon_crdt_synthetic`.
fn as_synthetic(
    doc: &LoroDoc,
    peer: u64,
    write: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let own = doc.peer_id();
    doc.set_peer_id(peer)
        .map_err(|e| format!("set a synthetic peer: {e}"))?;
    let written = write();
    doc.commit();
    let reset = doc.set_peer_id(own);
    written?;
    reset.map_err(|e| format!("reset the peer: {e}"))
}

thread_local! {
    /// Rows whose cached doc this thread changed since its write
    /// transaction began. The cache holds the change before the commit, so
    /// a rollback evicts them ([`LoroStore::rolled_back`]).
    static CHANGED_IN_TX: std::cell::RefCell<Vec<(String, String)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Rows noted per write transaction; past this, a rollback clears the whole
/// cache instead.
const MAX_CHANGED_IN_TX: usize = 4096;

/// A write transaction began on this thread (see [`crate::begin_write`]).
pub(crate) fn write_tx_began() {
    CHANGED_IN_TX.with(|c| c.borrow_mut().clear());
}

fn note_changed(entity: &str, row_id: &str) {
    CHANGED_IN_TX.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() <= MAX_CHANGED_IN_TX {
            c.push((entity.to_string(), row_id.to_string()));
        }
    });
}

/// Server-side per-row LoroDoc cache + persistence layer.
///
/// One instance per Runtime. Holds a bounded cache of doc handles
/// (see [`crate::crdt_cache`]), each behind its own `Mutex` so
/// concurrent access to *different* rows doesn't contend.
pub struct LoroStore {
    /// Per-row cache. The cache lock is held only for lookup/insert;
    /// Loro work happens under the per-doc Mutex, so two requests
    /// targeting different rows never block each other.
    docs: crate::crdt_cache::DocCache,
    /// The peer of this process's synthetic ops: random per process (and
    /// per cache clear), so op ids stay unique after a database is
    /// restored from a backup, and one per process, so a row's version
    /// vector does not grow with every synthetic write.
    synthetic_peer: std::sync::atomic::AtomicU64,
}

impl Default for LoroStore {
    fn default() -> Self {
        Self::new()
    }
}

impl LoroStore {
    pub fn new() -> Self {
        Self {
            docs: Default::default(),
            synthetic_peer: std::sync::atomic::AtomicU64::new(fresh_peer()),
        }
    }

    fn synthetic_peer(&self) -> u64 {
        self.synthetic_peer
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Get the cached doc for a row, hydrating from the sidecar if absent.
    /// Returns a freshly-created doc if the row has no snapshot yet.
    fn get_or_hydrate(
        &self,
        conn: &Connection,
        entity: &str,
        row_id: &str,
    ) -> Result<Arc<Mutex<LoroDoc>>, LoroStoreError> {
        // Fast path: already cached.
        if let Some(doc) = self.docs.get(entity, row_id) {
            return Ok(doc);
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

        // Publish, but defer to whatever's already there if we lost the
        // race.
        Ok(self.docs.get_or_insert(entity, row_id, handle))
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
        let doc = handle.lock().unwrap();
        note_changed(entity, row_id);
        let peer = doc.peer_id();
        let start = doc.oplog_vv().get(&peer).copied().unwrap_or(0);
        apply_patch(&doc, fields, patch).map_err(LoroStoreError::Apply)?;
        let written: Vec<&str> = patch
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(_, v)| !v.is_null())
            .map(|(k, _)| k.as_str())
            .collect();
        self.record_bases(conn, entity, row_id, &doc, fields, &written, peer, start)?;
        self.persist_snapshot(conn, entity, row_id, &doc)?;
        Ok(project_doc_to_json(&doc, fields))
    }

    /// The peers of the row's synthetic ops.
    pub fn synthetic_peers(
        &self,
        conn: &Connection,
        entity: &str,
        row_id: &str,
    ) -> Result<std::collections::HashSet<u64>, LoroStoreError> {
        let mut stmt = conn
            .prepare_cached(
                "SELECT peer FROM _pylon_crdt_synthetic WHERE entity = ?1 AND row_id = ?2",
            )
            .map_err(|e| LoroStoreError::Storage(format!("read synthetic peers: {e}")))?;
        let peers = stmt
            .query_map(params![entity, row_id], |r| r.get::<_, i64>(0))
            .map_err(|e| LoroStoreError::Storage(format!("read synthetic peers: {e}")))?
            .filter_map(Result::ok)
            .map(|p| p as u64)
            .collect();
        Ok(peers)
    }

    fn record_synthetic(
        &self,
        conn: &Connection,
        entity: &str,
        row_id: &str,
        peer: u64,
    ) -> Result<(), LoroStoreError> {
        conn.execute(
            "INSERT OR IGNORE INTO _pylon_crdt_synthetic (entity, row_id, peer)
             VALUES (?1, ?2, ?3)",
            params![entity, row_id, peer as i64],
        )
        .map(|_| ())
        .map_err(|e| LoroStoreError::Storage(format!("record a synthetic peer: {e}")))
    }

    /// Record the server's whole write of `written` (text, list, and tree
    /// fields): `peer`'s ops from `start` to now.
    #[allow(clippy::too_many_arguments)]
    fn record_bases(
        &self,
        conn: &Connection,
        entity: &str,
        row_id: &str,
        doc: &LoroDoc,
        fields: &[CrdtField],
        written: &[&str],
        peer: u64,
        start: i32,
    ) -> Result<(), LoroStoreError> {
        let end = doc.oplog_vv().get(&peer).copied().unwrap_or(0);
        for f in fields.iter().filter(|f| written.contains(&f.name.as_str())) {
            let Some(base) = pylon_crdt::merge::base_write(doc, f, peer, start, end) else {
                continue;
            };
            let base = serde_json::to_string(&base)
                .map_err(|e| LoroStoreError::Storage(format!("encode a base write: {e}")))?;
            conn.prepare_cached(
                "INSERT INTO _pylon_crdt_base (entity, row_id, field, base)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (entity, row_id, field) DO UPDATE SET base = excluded.base",
            )
            .and_then(|mut stmt| stmt.execute(params![entity, row_id, f.name, base]))
            .map_err(|e| LoroStoreError::Storage(format!("record a base write: {e}")))?;
        }
        Ok(())
    }

    fn bases(
        &self,
        conn: &Connection,
        entity: &str,
        row_id: &str,
    ) -> Result<std::collections::HashMap<String, pylon_crdt::merge::BaseWrite>, LoroStoreError>
    {
        self.read_json_side(
            conn,
            "SELECT field, base FROM _pylon_crdt_base WHERE entity = ?1 AND row_id = ?2",
            entity,
            row_id,
        )
    }

    fn links(
        &self,
        conn: &Connection,
        entity: &str,
        row_id: &str,
    ) -> Result<std::collections::HashMap<String, pylon_crdt::merge::Links>, LoroStoreError> {
        self.read_json_side(
            conn,
            "SELECT source, links FROM _pylon_crdt_links WHERE entity = ?1 AND row_id = ?2",
            entity,
            row_id,
        )
    }

    /// A row's (key, JSON value) side records, decoded. A record that no
    /// longer decodes is skipped: the merge then starts over for it.
    fn read_json_side<T: serde::de::DeserializeOwned>(
        &self,
        conn: &Connection,
        sql: &str,
        entity: &str,
        row_id: &str,
    ) -> Result<std::collections::HashMap<String, T>, LoroStoreError> {
        let err = |e: rusqlite::Error| LoroStoreError::Storage(format!("read {sql}: {e}"));
        let mut stmt = conn.prepare_cached(sql).map_err(err)?;
        let rows = stmt
            .query_map(params![entity, row_id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(err)?;
        Ok(rows
            .into_iter()
            .filter_map(|(key, json)| Some((key, serde_json::from_str(&json).ok()?)))
            .collect())
    }

    fn drop_links(
        &self,
        conn: &Connection,
        entity: &str,
        row_id: &str,
        source: &str,
    ) -> Result<(), LoroStoreError> {
        conn.prepare_cached(
            "DELETE FROM _pylon_crdt_links WHERE entity = ?1 AND row_id = ?2 AND source = ?3",
        )
        .and_then(|mut stmt| stmt.execute(params![entity, row_id, source]))
        .map(|_| ())
        .map_err(|e| LoroStoreError::Storage(format!("drop links: {e}")))
    }

    fn record_links(
        &self,
        conn: &Connection,
        entity: &str,
        row_id: &str,
        source: &str,
        links: &pylon_crdt::merge::Links,
    ) -> Result<(), LoroStoreError> {
        let links = serde_json::to_string(links)
            .map_err(|e| LoroStoreError::Storage(format!("encode links: {e}")))?;
        conn.prepare_cached(
            "INSERT INTO _pylon_crdt_links (entity, row_id, source, links)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (entity, row_id, source) DO UPDATE SET links = excluded.links",
        )
        .and_then(|mut stmt| stmt.execute(params![entity, row_id, source, links]))
        .map(|_| ())
        .map_err(|e| LoroStoreError::Storage(format!("record links: {e}")))
    }

    /// Apply `values` as one synthetic write (a doc created or brought in
    /// line from its row: seed, reconcile, fill), one field at a time under
    /// one peer, and record it as the server's write of those fields.
    /// Returns the fields the doc could not take, with why.
    pub fn apply_seed_fields(
        &self,
        conn: &Connection,
        entity: &str,
        row_id: &str,
        fields: &[CrdtField],
        values: Vec<(String, Value)>,
    ) -> Result<Vec<(String, String)>, LoroStoreError> {
        if values.is_empty() {
            return Ok(Vec::new());
        }
        let handle = self.get_or_hydrate(conn, entity, row_id)?;
        let doc = handle.lock().unwrap();
        note_changed(entity, row_id);
        let tried = values.len();
        let mut failed = Vec::new();
        let mut written = Vec::new();
        let peer = self.synthetic_peer();
        let start = doc.oplog_vv().get(&peer).copied().unwrap_or(0);
        as_synthetic(&doc, peer, || {
            for (name, value) in values {
                let patch = serde_json::json!({ name.clone(): value });
                match apply_patch(&doc, fields, &patch) {
                    Ok(()) => written.push(name),
                    Err(e) => failed.push((name, e)),
                }
            }
            Ok(())
        })
        .map_err(LoroStoreError::Apply)?;
        if failed.len() < tried {
            self.record_synthetic(conn, entity, row_id, peer)?;
        }
        let written: Vec<&str> = written.iter().map(String::as_str).collect();
        self.record_bases(conn, entity, row_id, &doc, fields, &written, peer, start)?;
        self.persist_snapshot(conn, entity, row_id, &doc)?;
        Ok(failed)
    }

    /// Import a client's update into the row's doc and persist it. Then, as
    /// one synthetic write:
    ///
    /// - A register the update wrote whose winning op is synthetic takes
    ///   the update's value again: the client had not seen that op.
    /// - Per text, list, tree, or counter field, the other containers the
    ///   update changed, and the one the key moved from, are merged into
    ///   the one holding the key (`pylon_crdt::merge`).
    ///
    /// An update that adds no ops (a push sent again) changes nothing.
    /// Returns the projection before the import and the fields the update
    /// changed (deletes included), none when it added no ops.
    pub fn apply_client_update(
        &self,
        conn: &Connection,
        entity: &str,
        row_id: &str,
        fields: &[CrdtField],
        update: &[u8],
    ) -> Result<(Value, Option<Vec<String>>), LoroStoreError> {
        let peers = self.synthetic_peers(conn, entity, row_id)?;
        let handle = self.get_or_hydrate(conn, entity, row_id)?;
        let doc = handle.lock().unwrap();
        note_changed(entity, row_id);
        let before = project_doc_to_json(&doc, fields);
        let held_before = doc.oplog_vv();
        let prior = pylon_crdt::merge::holders(&doc, fields);
        crdt_apply_update(&doc, update).map_err(LoroStoreError::Decode)?;
        if doc.oplog_vv() == held_before {
            return Ok((before, None));
        }
        let imported = pylon_crdt::merge::read_import(&doc, fields, &held_before);
        let projected = project_doc_to_json(&doc, fields);
        let map = pylon_crdt::root_map(&doc);
        let null = Value::Null;
        let registers: Vec<(&String, &Value)> = imported
            .registers
            .iter()
            .filter(|(name, wanted)| {
                !crate::same_json_value(projected.get(name.as_str()).unwrap_or(&null), wanted)
                    && map
                        .get_last_editor(name)
                        .is_some_and(|p| peers.contains(&p))
            })
            .collect();
        let merging = pylon_crdt::merge::has_sources(&doc, fields, &imported, &prior);
        let (bases, mut links) = if merging {
            (
                self.bases(conn, entity, row_id)?,
                self.links(conn, entity, row_id)?,
            )
        } else {
            Default::default()
        };
        let held = doc.oplog_vv();
        let mut linked = Vec::new();
        let peer = self.synthetic_peer();
        as_synthetic(&doc, peer, || {
            // A value the field cannot take (a client can write any) is
            // left as Loro merged it.
            for (name, value) in &registers {
                if let Err(e) = apply_patch(&doc, fields, &serde_json::json!({ *name: value })) {
                    tracing::warn!(entity, row_id, field = %name, "apply a client's value again: {e}");
                }
            }
            if merging {
                linked = pylon_crdt::merge::merge_after_import(
                    &doc, fields, &imported, &prior, &bases, &mut links,
                )?;
            }
            Ok(())
        })
        .map_err(LoroStoreError::Apply)?;
        if doc.oplog_vv() != held {
            self.record_synthetic(conn, entity, row_id, peer)?;
        }
        for source in linked {
            match links.get(&source) {
                Some(l) => self.record_links(conn, entity, row_id, &source, l)?,
                None => self.drop_links(conn, entity, row_id, &source)?,
            }
        }
        self.persist_snapshot(conn, entity, row_id, &doc)?;
        Ok((before, Some(imported.touched)))
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
            note_changed(entity, row_id);
            crdt_apply_update(&doc, update).map_err(LoroStoreError::Decode)?;
            self.persist_snapshot(conn, entity, row_id, &doc)?;
            project_doc_to_json(&doc, fields)
        };
        Ok(projected)
    }

    /// Which of `fields` the row's doc holds (a key in its root map).
    pub fn held_fields(
        &self,
        conn: &Connection,
        entity: &str,
        row_id: &str,
        fields: &[CrdtField],
    ) -> Result<Vec<String>, LoroStoreError> {
        let handle = self.get_or_hydrate(conn, entity, row_id)?;
        let doc = handle.lock().unwrap();
        let map = pylon_crdt::root_map(&doc);
        Ok(fields
            .iter()
            .filter(|f| map.get(&f.name).is_some())
            .map(|f| f.name.clone())
            .collect())
    }

    /// Whether the row has a stored snapshot (a doc cached for a read of a
    /// row with none does not count).
    pub fn has_snapshot(
        &self,
        conn: &Connection,
        entity: &str,
        row_id: &str,
    ) -> Result<bool, LoroStoreError> {
        conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM _pylon_crdt_snapshots WHERE entity = ?1 AND row_id = ?2)",
            params![entity, row_id],
            |r| r.get(0),
        )
        .map_err(|e| LoroStoreError::Storage(format!("read snapshot: {e}")))
    }

    /// The row's doc as JSON, for `fields`.
    pub fn project(
        &self,
        conn: &Connection,
        entity: &str,
        row_id: &str,
        fields: &[CrdtField],
    ) -> Result<Value, LoroStoreError> {
        let handle = self.get_or_hydrate(conn, entity, row_id)?;
        let doc = handle.lock().unwrap();
        Ok(project_doc_to_json(&doc, fields))
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

    /// Current Loro version vector for a row, as the encoded bytes the
    /// WS broadcast path remembers between writes. Returns `None` when
    /// the row has no LoroDoc yet (no writes have happened).
    pub fn current_vv_bytes(
        &self,
        conn: &Connection,
        entity: &str,
        row_id: &str,
    ) -> Result<Option<Vec<u8>>, LoroStoreError> {
        let handle = self.get_or_hydrate(conn, entity, row_id)?;
        let doc = handle.lock().unwrap();
        Ok(Some(doc.oplog_vv().encode()))
    }

    /// Bytes-shaped wrapper around `update_since` for the trait method
    /// in pylon-http. `since` is the opaque bytes the broadcaster
    /// previously stashed via `current_vv_bytes`; we decode + diff
    /// here so the trait surface stays plain Vec<u8>.
    pub fn update_since_bytes(
        &self,
        conn: &Connection,
        entity: &str,
        row_id: &str,
        since: &[u8],
    ) -> Result<Option<Vec<u8>>, LoroStoreError> {
        let parsed = VersionVector::decode(since)
            .map_err(|e| LoroStoreError::Decode(format!("decode VV for {entity}/{row_id}: {e}")))?;
        let handle = self.get_or_hydrate(conn, entity, row_id)?;
        let doc = handle.lock().unwrap();
        Ok(Some(encode_update_since(&doc, &parsed)))
    }

    /// This thread's write transaction rolled back: drop the docs it
    /// changed from the cache, so the next read loads the committed
    /// snapshot.
    pub fn rolled_back(&self) {
        let changed = CHANGED_IN_TX.with(|c| std::mem::take(&mut *c.borrow_mut()));
        if changed.len() > MAX_CHANGED_IN_TX {
            self.clear_cache();
            return;
        }
        for (entity, row_id) in changed {
            self.evict(&entity, &row_id);
        }
    }

    /// Drop a row's doc from the in-memory cache. Doesn't touch the
    /// sidecar; the next read will re-hydrate from disk.
    pub fn evict(&self, entity: &str, row_id: &str) {
        self.docs.remove(entity, row_id);
    }

    /// Delete a row's snapshot from the sidecar and drop its cached doc.
    /// Call on the same connection (and transaction) as the entity-row
    /// DELETE. Without this every deleted CRDT row left its snapshot
    /// behind: an app whose crons replace rollup rows hourly had 1.37M
    /// orphaned snapshots, 95% of its database (Stack0 Analytics,
    /// 2026-09). Evicting before commit is safe either way: a rollback
    /// leaves the sidecar row, which the next read re-hydrates.
    pub fn delete_row(
        &self,
        conn: &Connection,
        entity: &str,
        row_id: &str,
    ) -> Result<(), LoroStoreError> {
        conn.execute(
            "DELETE FROM _pylon_crdt_snapshots WHERE entity = ?1 AND row_id = ?2",
            params![entity, row_id],
        )
        .map_err(|e| LoroStoreError::Storage(format!("delete snapshot: {e}")))?;
        for table in ROW_SIDE_TABLES {
            conn.execute(
                &format!("DELETE FROM {table} WHERE entity = ?1 AND row_id = ?2"),
                params![entity, row_id],
            )
            .map_err(|e| LoroStoreError::Storage(format!("delete from {table}: {e}")))?;
        }
        self.evict(entity, row_id);
        Ok(())
    }

    /// Drop every cached doc. Tests only (`reset_for_tests`).
    pub fn clear_cache(&self) {
        self.docs.clear();
        self.synthetic_peer
            .store(fresh_peer(), std::sync::atomic::Ordering::Relaxed);
    }

    /// Number of rows currently held in memory. Diagnostic.
    pub fn cached_rows(&self) -> usize {
        self.docs.len()
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
    fn sidecar_drops_the_earlier_merge_tables() {
        let conn = open_test_db();
        conn.execute_batch(
            "CREATE TABLE _pylon_crdt_written (x TEXT);
             CREATE TABLE _pylon_crdt_merged (x TEXT);",
        )
        .unwrap();
        ensure_sidecar(&conn).unwrap();
        let left: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE name IN ('_pylon_crdt_written', '_pylon_crdt_merged')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(left, 0);
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
        assert!(
            !snap.is_empty(),
            "snapshot should be non-empty after writes"
        );
        assert_eq!(store.cached_rows(), 1, "snapshot() rehydrated the cache");
    }

    #[test]
    fn current_vv_bytes_round_trips_through_update_since() {
        // Validates the delta wire path used by the WS broadcast
        // route: snapshot a row, capture its VV, write again, ask
        // for the delta from that captured VV. Applying the delta
        // to a peer with the snapshot must yield the same projected
        // state as importing a fresh snapshot directly.
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
        // Snapshot + VV at this point.
        let snap_v1 = store.snapshot(&conn, "Note", "n1").unwrap();
        let vv_v1 = store
            .current_vv_bytes(&conn, "Note", "n1")
            .unwrap()
            .unwrap();

        // Second write advances state.
        store
            .apply_patch(
                &conn,
                "Note",
                "n1",
                &fields(),
                &serde_json::json!({"title": "v2", "qty": 9}),
            )
            .unwrap();

        // Compute delta from the v1 VV.
        let delta = store
            .update_since_bytes(&conn, "Note", "n1", &vv_v1)
            .unwrap()
            .unwrap();
        // Delta is meaningfully smaller than the full snapshot
        // — that's the entire point of this batch.
        assert!(
            delta.len() < snap_v1.len(),
            "delta {} bytes should be smaller than snapshot {} bytes",
            delta.len(),
            snap_v1.len()
        );

        // Simulate a peer: open a fresh store, import the v1
        // snapshot, then apply the delta. Final projection must
        // match the server's v2 state.
        let peer_conn = open_test_db();
        let peer = LoroStore::new();
        peer.apply_remote_update(&peer_conn, "Note", "n1", &fields(), &snap_v1)
            .unwrap();
        let after_delta = peer
            .apply_remote_update(&peer_conn, "Note", "n1", &fields(), &delta)
            .unwrap();
        assert_eq!(after_delta["title"], "v2");
        assert_eq!(after_delta["qty"], 9.0);
    }

    #[test]
    fn update_since_bytes_rejects_garbage_vv() {
        // The trait surface accepts opaque bytes from the
        // broadcast layer; malformed bytes must produce a typed
        // error, not a panic. The caller falls back to a snapshot
        // broadcast on this error path.
        let conn = open_test_db();
        let store = LoroStore::new();
        store
            .apply_patch(
                &conn,
                "Note",
                "n1",
                &fields(),
                &serde_json::json!({"title": "x"}),
            )
            .unwrap();
        let err = store
            .update_since_bytes(&conn, "Note", "n1", b"not a real VV")
            .unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("decode VV"), "got {msg}");
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

        let delta = store.update_since(&conn, "Note", "n1", &early_vv).unwrap();
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
