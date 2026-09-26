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

/// Delete up to `batch` snapshots of `entity` in `scope`; returns how many.
/// Callers loop until it returns 0, taking the write lock per batch.
pub fn prune_batch(
    conn: &Connection,
    entity: &str,
    scope: PruneScope,
    batch: usize,
) -> Result<usize, LoroStoreError> {
    let sql = match scope {
        PruneScope::Orphans => format!(
            "DELETE FROM _pylon_crdt_snapshots WHERE rowid IN (
                SELECT s.rowid FROM _pylon_crdt_snapshots s
                WHERE s.entity = ?1
                  AND NOT EXISTS (SELECT 1 FROM {} t WHERE t.\"id\" = s.row_id)
                LIMIT ?2)",
            quote_sqlite_ident(entity)
        ),
        PruneScope::All => "DELETE FROM _pylon_crdt_snapshots WHERE rowid IN (
                SELECT rowid FROM _pylon_crdt_snapshots WHERE entity = ?1 LIMIT ?2)"
            .to_string(),
    };
    let pruned = conn
        .execute(&sql, params![entity, batch as i64])
        .map_err(|e| LoroStoreError::Storage(format!("prune snapshots of {entity}: {e}")))?;
    // The rows' server-side records go with their snapshots.
    for table in ROW_SIDE_TABLES {
        conn.execute(
            &format!(
                "DELETE FROM {table} WHERE entity = ?1 AND NOT EXISTS (
                    SELECT 1 FROM _pylon_crdt_snapshots s
                    WHERE s.entity = {table}.entity AND s.row_id = {table}.row_id)"
            ),
            params![entity],
        )
        .map_err(|e| LoroStoreError::Storage(format!("prune {table} of {entity}: {e}")))?;
    }
    Ok(pruned)
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
    // Per text, list, or tree field: the value the server last wrote into
    // the container holding the key, and whose value it was (a seed: "",
    // or a client's container). A client's value replaces the container's
    // only while it still holds exactly that write.
    conn.execute(
        "CREATE TABLE IF NOT EXISTS _pylon_crdt_written (
            entity TEXT NOT NULL,
            row_id TEXT NOT NULL,
            field TEXT NOT NULL,
            container TEXT NOT NULL,
            value TEXT NOT NULL,
            source TEXT NOT NULL,
            PRIMARY KEY (entity, row_id, field)
        )",
        [],
    )
    .map(|_| ())
    .map_err(|e| LoroStoreError::Storage(format!("create the written-value table: {e}")))?;
    // Per client container that lost its key: its content already merged
    // into the one holding the key, so a later push adds only what is new.
    conn.execute(
        "CREATE TABLE IF NOT EXISTS _pylon_crdt_merged (
            entity TEXT NOT NULL,
            row_id TEXT NOT NULL,
            source TEXT NOT NULL,
            value TEXT NOT NULL,
            PRIMARY KEY (entity, row_id, source)
        )",
        [],
    )
    .map(|_| ())
    .map_err(|e| LoroStoreError::Storage(format!("create the merged-value table: {e}")))
}

/// The server-side tables kept per CRDT row beside its snapshot.
const ROW_SIDE_TABLES: [&str; 3] = [
    "_pylon_crdt_synthetic",
    "_pylon_crdt_written",
    "_pylon_crdt_merged",
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
/// its row, or a client's value applied again): under a fresh random peer,
/// so op ids stay unique even after a database is restored from a backup,
/// then back to another fresh peer for the server's own writes. Returns
/// the peer, for the caller to record in `_pylon_crdt_synthetic`.
fn as_synthetic(doc: &LoroDoc, write: impl FnOnce() -> Result<(), String>) -> Result<u64, String> {
    let peer = fresh_peer();
    doc.set_peer_id(peer)
        .map_err(|e| format!("set a synthetic peer: {e}"))?;
    let written = write();
    doc.commit();
    let reset = doc.set_peer_id(fresh_peer());
    written?;
    reset.map_err(|e| format!("reset the peer: {e}"))?;
    Ok(peer)
}

/// The start of an update, limited to ops the doc holds: an update of
/// several changes lists its own earlier changes among its dependencies,
/// which the doc does not have until it imports them.
fn update_base(doc: &LoroDoc, update: &[u8]) -> Option<pylon_crdt::loro::Frontiers> {
    let meta = LoroDoc::decode_import_blob_meta(update, false).ok()?;
    let held = doc.oplog_vv();
    Some(
        meta.start_frontiers
            .iter()
            .filter(|id| held.get(&id.peer).is_some_and(|&n| id.counter < n))
            .collect(),
    )
}

/// Whether `doc` holds synthetic ops (by `peers`) the update's author had
/// not seen: only then can the update have lost to one.
fn unseen_synthetic_ops(
    doc: &LoroDoc,
    peers: &std::collections::HashSet<u64>,
    update: &[u8],
) -> bool {
    if peers.is_empty() {
        return false;
    }
    let Some(base) = update_base(doc, update) else {
        return true;
    };
    let Some(seen) = doc.frontiers_to_vv(&base) else {
        return true;
    };
    let held = doc.oplog_vv();
    peers
        .iter()
        .any(|p| held.get(p).copied().unwrap_or(0) > seen.get(p).copied().unwrap_or(0))
}

/// A counter's value, 0 for one the doc does not hold (Loro asserts the
/// container exists).
fn counter_value(doc: &LoroDoc, id: &pylon_crdt::loro::ContainerID) -> f64 {
    if doc.has_container(id) {
        doc.get_counter(id.clone()).get_value()
    } else {
        0.0
    }
}

/// What a client's update set, read from a copy of the doc at the
/// update's start: the fields it changed, their values after it, and the
/// container it left under each key (for a counter, where its increments
/// are).
struct ClientIntent {
    touched: Vec<String>,
    after: Value,
    containers: std::collections::HashMap<String, pylon_crdt::loro::ContainerID>,
}

fn field_containers(
    doc: &LoroDoc,
    fields: &[CrdtField],
) -> std::collections::HashMap<String, pylon_crdt::loro::ContainerID> {
    use pylon_crdt::loro::{ContainerTrait, ValueOrContainer};
    let map = pylon_crdt::root_map(doc);
    fields
        .iter()
        .filter_map(|f| match map.get(&f.name) {
            Some(ValueOrContainer::Container(c)) => Some((f.name.clone(), c.id())),
            _ => None,
        })
        .collect()
}

fn client_intent(doc: &LoroDoc, fields: &[CrdtField], update: &[u8]) -> Option<ClientIntent> {
    let base = update_base(doc, update)?;
    let scratch = doc.fork_at(&base).ok()?;
    let before = project_doc_to_json(&scratch, fields);
    let before_containers = field_containers(&scratch, fields);
    let editors = |doc: &LoroDoc| -> Vec<Option<u64>> {
        let map = pylon_crdt::root_map(doc);
        fields
            .iter()
            .map(|f| map.get_last_editor(&f.name))
            .collect()
    };
    let before_editors = editors(&scratch);
    let status = scratch.import(update).ok()?;
    // Ops the copy could not place (the update depends on others): what
    // it set cannot be read.
    if status.pending.is_some() {
        return None;
    }
    let after = project_doc_to_json(&scratch, fields);
    let containers = field_containers(&scratch, fields);
    let after_editors = editors(&scratch);
    let null = Value::Null;
    // A field the update wrote, deleted (the value may not change: a key
    // an empty doc never held), or moved to a new container.
    let touched = fields
        .iter()
        .enumerate()
        .filter(|(i, f)| {
            before.get(&f.name).unwrap_or(&null) != after.get(&f.name).unwrap_or(&null)
                || before_containers.get(&f.name) != containers.get(&f.name)
                || before_editors[*i] != after_editors[*i]
        })
        .map(|(_, f)| f.name.clone())
        .collect();
    Some(ClientIntent {
        touched,
        after,
        containers,
    })
}

/// What `steps_after_import` returns: the steps, the values the server
/// writes (field, value, source), and the client values merged (source,
/// value).
type AfterImport = (
    Vec<Step>,
    Vec<(String, Value, String)>,
    Vec<(String, Value)>,
);

/// A value the server last wrote into a text, list, or tree container.
struct Written {
    container: String,
    value: Value,
    source: String,
}

/// What to do after a client's update was imported.
enum Step {
    /// Patch a field (a register's value, a counter's increment, or a
    /// text, list, or tree replaced whole).
    Patch(String, Value),
    /// Append to the text or list holding a field's key.
    Append(String, Value),
}

/// The part of `now` past its common start with `was`: the text or items
/// a client container gained since it was last merged.
fn gained(was: &Value, now: &Value) -> Option<Value> {
    match (was, now) {
        (Value::String(a), Value::String(b)) => {
            let common = a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count();
            let rest: String = b.chars().skip(common).collect();
            (!rest.is_empty()).then_some(Value::String(rest))
        }
        (Value::Array(a), Value::Array(b)) => {
            let common = a.iter().zip(b).take_while(|(x, y)| x == y).count();
            (common < b.len()).then(|| Value::Array(b[common..].to_vec()))
        }
        (Value::Null, now) if !now.is_null() => gained(
            &match now {
                Value::String(_) => Value::String(String::new()),
                _ => Value::Array(Vec::new()),
            },
            now,
        ),
        _ => None,
    }
}

/// After a client's update was imported: what to apply so the client's
/// values that lost to synthetic ops still count.
///
/// - A register whose winning op is synthetic takes the client's value.
/// - A text, list, or tree the client made under a key another container
///   holds: while that container still holds exactly what the server last
///   wrote there (from a seed, or from this same client container), the
///   client's value replaces it. Otherwise clients have edited it, and the
///   client's text or items it gained since its last merge are appended (a
///   tree is left as it is).
/// - A counter: `counted` is what this update added, per field; when those
///   increments are in a counter not holding the key, they are added to
///   the one that does. A counter the client made that took the key from
///   another adds that one's total.
///
/// Returns the steps, and the written and merged values to record.
#[allow(clippy::too_many_arguments)]
fn steps_after_import(
    doc: &LoroDoc,
    fields: &[CrdtField],
    intent: &ClientIntent,
    prior: &std::collections::HashMap<String, pylon_crdt::loro::ContainerID>,
    counted: &std::collections::HashMap<String, f64>,
    peers: &std::collections::HashSet<u64>,
    written: &std::collections::HashMap<String, Written>,
    merged_before: &std::collections::HashMap<String, Value>,
) -> AfterImport {
    use pylon_crdt::CrdtFieldKind as K;
    let map = pylon_crdt::root_map(doc);
    let projected = project_doc_to_json(doc, fields);
    let now = field_containers(doc, fields);
    let null = Value::Null;
    let (mut steps, mut writes, mut merges) = (Vec::new(), Vec::new(), Vec::new());
    for f in fields.iter().filter(|f| intent.touched.contains(&f.name)) {
        let wanted = intent.after.get(&f.name).unwrap_or(&null);
        let client_container = intent.containers.get(&f.name);
        let winner = now.get(&f.name);
        match f.kind {
            K::LwwString | K::LwwNumber | K::LwwBool | K::LwwJson => {
                if projected.get(&f.name).unwrap_or(&null) != wanted
                    && map
                        .get_last_editor(&f.name)
                        .is_some_and(|p| peers.contains(&p))
                {
                    steps.push(Step::Patch(f.name.clone(), wanted.clone()));
                }
            }
            K::Text | K::List | K::MovableList | K::Tree => {
                let (Some(client), Some(winner)) = (client_container, winner) else {
                    continue;
                };
                if client == winner {
                    continue;
                }
                let source = client.to_string();
                let holds_our_write = written.get(&f.name).is_some_and(|w| {
                    w.container == winner.to_string()
                        && projected.get(&f.name).unwrap_or(&null) == &w.value
                        && (w.source.is_empty() || w.source == source)
                });
                if holds_our_write {
                    steps.push(Step::Patch(f.name.clone(), wanted.clone()));
                    writes.push((f.name.clone(), wanted.clone(), source.clone()));
                    merges.push((source, wanted.clone()));
                } else if f.kind != K::Tree {
                    let was = merged_before.get(&source).unwrap_or(&null);
                    if let Some(rest) = gained(was, wanted) {
                        steps.push(Step::Append(f.name.clone(), rest));
                        merges.push((source, wanted.clone()));
                    }
                }
            }
            K::Counter => {
                let mut add = 0.0;
                if let (Some(sum), Some(client)) = (counted.get(&f.name), client_container) {
                    if winner != Some(client) {
                        add += sum;
                    }
                }
                if let (Some(old), Some(winner)) = (prior.get(&f.name), winner) {
                    if old != winner && Some(winner) == client_container {
                        add += counter_value(doc, old);
                    }
                }
                if add != 0.0 {
                    steps.push(Step::Patch(f.name.clone(), serde_json::json!(add)));
                }
            }
        }
    }
    (steps, writes, merges)
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
#[derive(Default)]
pub struct LoroStore {
    /// Per-row cache. The cache lock is held only for lookup/insert;
    /// Loro work happens under the per-doc Mutex, so two requests
    /// targeting different rows never block each other.
    docs: crate::crdt_cache::DocCache,
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
        let projected = {
            let doc = handle.lock().unwrap();
            note_changed(entity, row_id);
            apply_patch(&doc, fields, patch).map_err(LoroStoreError::Apply)?;
            self.persist_snapshot(conn, entity, row_id, &doc)?;
            project_doc_to_json(&doc, fields)
        };
        Ok(projected)
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

    fn record_written(
        &self,
        conn: &Connection,
        entity: &str,
        row_id: &str,
        field: &str,
        written: &Written,
    ) -> Result<(), LoroStoreError> {
        conn.execute(
            "INSERT INTO _pylon_crdt_written (entity, row_id, field, container, value, source)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT (entity, row_id, field) DO UPDATE SET
                container = excluded.container, value = excluded.value,
                source = excluded.source",
            params![
                entity,
                row_id,
                field,
                written.container,
                written.value.to_string(),
                written.source
            ],
        )
        .map(|_| ())
        .map_err(|e| LoroStoreError::Storage(format!("record a written value: {e}")))
    }

    fn written_values(
        &self,
        conn: &Connection,
        entity: &str,
        row_id: &str,
    ) -> Result<std::collections::HashMap<String, Written>, LoroStoreError> {
        let mut stmt = conn
            .prepare_cached(
                "SELECT field, container, value, source FROM _pylon_crdt_written
                 WHERE entity = ?1 AND row_id = ?2",
            )
            .map_err(|e| LoroStoreError::Storage(format!("read written values: {e}")))?;
        let rows = stmt
            .query_map(params![entity, row_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })
            .map_err(|e| LoroStoreError::Storage(format!("read written values: {e}")))?;
        Ok(rows
            .filter_map(Result::ok)
            .filter_map(|(field, container, value, source)| {
                Some((
                    field,
                    Written {
                        container,
                        value: serde_json::from_str(&value).ok()?,
                        source,
                    },
                ))
            })
            .collect())
    }

    fn merged_values(
        &self,
        conn: &Connection,
        entity: &str,
        row_id: &str,
    ) -> Result<std::collections::HashMap<String, Value>, LoroStoreError> {
        let mut stmt = conn
            .prepare_cached(
                "SELECT source, value FROM _pylon_crdt_merged WHERE entity = ?1 AND row_id = ?2",
            )
            .map_err(|e| LoroStoreError::Storage(format!("read merged values: {e}")))?;
        let rows = stmt
            .query_map(params![entity, row_id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(|e| LoroStoreError::Storage(format!("read merged values: {e}")))?;
        Ok(rows
            .filter_map(Result::ok)
            .filter_map(|(source, value)| Some((source, serde_json::from_str(&value).ok()?)))
            .collect())
    }

    fn record_merged(
        &self,
        conn: &Connection,
        entity: &str,
        row_id: &str,
        source: &str,
        value: &Value,
    ) -> Result<(), LoroStoreError> {
        conn.execute(
            "INSERT INTO _pylon_crdt_merged (entity, row_id, source, value)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (entity, row_id, source) DO UPDATE SET value = excluded.value",
            params![entity, row_id, source, value.to_string()],
        )
        .map(|_| ())
        .map_err(|e| LoroStoreError::Storage(format!("record a merged value: {e}")))
    }

    /// Apply `values` as one synthetic write (a doc created or brought in
    /// line from its row: seed, reconcile, fill), one field at a time under
    /// one peer. A text, list, or tree it writes is recorded as the
    /// server's write (source ""). Returns the fields the doc could not
    /// take, with why.
    pub fn apply_seed_fields(
        &self,
        conn: &Connection,
        entity: &str,
        row_id: &str,
        fields: &[CrdtField],
        values: Vec<(String, Value)>,
    ) -> Result<Vec<(String, String)>, LoroStoreError> {
        use pylon_crdt::CrdtFieldKind as K;
        if values.is_empty() {
            return Ok(Vec::new());
        }
        let handle = self.get_or_hydrate(conn, entity, row_id)?;
        let doc = handle.lock().unwrap();
        note_changed(entity, row_id);
        let tried = values.len();
        let mut failed = Vec::new();
        let mut written = Vec::new();
        let peer = as_synthetic(&doc, || {
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
        let projected = project_doc_to_json(&doc, fields);
        let containers = field_containers(&doc, fields);
        for f in fields.iter().filter(|f| {
            matches!(f.kind, K::Text | K::List | K::MovableList | K::Tree)
                && written.contains(&f.name)
        }) {
            if let Some(container) = containers.get(&f.name) {
                let written = Written {
                    container: container.to_string(),
                    value: projected.get(&f.name).cloned().unwrap_or(Value::Null),
                    source: String::new(),
                };
                self.record_written(conn, entity, row_id, &f.name, &written)?;
            }
        }
        self.persist_snapshot(conn, entity, row_id, &doc)?;
        Ok(failed)
    }

    /// Import a client's update into the row's doc and persist it, keeping
    /// the client's values that lost to synthetic ops it had not seen (see
    /// `steps_after_import`). An update that adds no ops (a push sent
    /// again) changes nothing. Returns the projection before the import and
    /// the fields the update set (deletes included) when they could be read
    /// from it.
    pub fn apply_client_update(
        &self,
        conn: &Connection,
        entity: &str,
        row_id: &str,
        fields: &[CrdtField],
        update: &[u8],
        always_read_intent: bool,
    ) -> Result<(Value, Option<Vec<String>>), LoroStoreError> {
        let peers = self.synthetic_peers(conn, entity, row_id)?;
        let handle = self.get_or_hydrate(conn, entity, row_id)?;
        let doc = handle.lock().unwrap();
        note_changed(entity, row_id);
        let before = project_doc_to_json(&doc, fields);
        let held_before = doc.oplog_vv();
        let prior = field_containers(&doc, fields);
        let intent = if always_read_intent || unseen_synthetic_ops(&doc, &peers, update) {
            client_intent(&doc, fields, update)
        } else {
            None
        };
        // A counter's increments in this update, measured on the doc itself
        // (the ops it did not hold yet), in the container the update counts
        // in.
        let counter_before: std::collections::HashMap<String, f64> = intent
            .iter()
            .flat_map(|i| {
                fields
                    .iter()
                    .filter(|f| f.kind == pylon_crdt::CrdtFieldKind::Counter)
                    .filter_map(|f| {
                        let id = i.containers.get(&f.name)?;
                        Some((f.name.clone(), counter_value(&doc, id)))
                    })
            })
            .collect();
        crdt_apply_update(&doc, update).map_err(LoroStoreError::Decode)?;
        if doc.oplog_vv() == held_before {
            return Ok((before, None));
        }
        if let Some(intent) = &intent {
            let counted: std::collections::HashMap<String, f64> = counter_before
                .iter()
                .filter_map(|(name, was)| {
                    let id = intent.containers.get(name)?;
                    Some((name.clone(), counter_value(&doc, id) - was))
                })
                .collect();
            let written = self.written_values(conn, entity, row_id)?;
            let merged = self.merged_values(conn, entity, row_id)?;
            let (steps, writes, merges) = steps_after_import(
                &doc, fields, intent, &prior, &counted, &peers, &written, &merged,
            );
            if !steps.is_empty() {
                let peer = as_synthetic(&doc, || {
                    for step in &steps {
                        match step {
                            Step::Patch(name, value) => {
                                apply_patch(&doc, fields, &serde_json::json!({ name: value }))?
                            }
                            Step::Append(name, value) => {
                                let field = fields
                                    .iter()
                                    .find(|f| &f.name == name)
                                    .ok_or_else(|| format!("no field {name}"))?;
                                pylon_crdt::append_to_field(&doc, field, value)?
                            }
                        }
                    }
                    Ok(())
                })
                .map_err(LoroStoreError::Apply)?;
                self.record_synthetic(conn, entity, row_id, peer)?;
                let containers = field_containers(&doc, fields);
                for (field, value, source) in writes {
                    if let Some(container) = containers.get(&field) {
                        let written = Written {
                            container: container.to_string(),
                            value,
                            source,
                        };
                        self.record_written(conn, entity, row_id, &field, &written)?;
                    }
                }
                for (source, value) in merges {
                    self.record_merged(conn, entity, row_id, &source, &value)?;
                }
            }
        }
        self.persist_snapshot(conn, entity, row_id, &doc)?;
        Ok((before, intent.map(|i| i.touched)))
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
