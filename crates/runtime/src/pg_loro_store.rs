//! Postgres-backed CRDT snapshot store.
//!
//! Mirrors `loro_store::LoroStore` (the SQLite path) but persists the
//! per-row Loro snapshots into a PG `_pylon_crdt_snapshots` table
//! instead of a SQLite sidecar. Cache shape, hydrate-on-miss, and
//! locking model are identical so the CRDT semantics don't drift
//! between backends.
//!
//! Every method is generic over `PgConn`, which is implemented for
//! both `postgres::Client` and `postgres::Transaction`. That's what
//! lets the runtime call `apply_patch` *inside* the same transaction
//! that writes the materialized entity row + maintains the FTS
//! shadow — sidecar write, entity write, and FTS write either all
//! commit or all roll back, so a crash mid-write can't desync the
//! CRDT snapshot from the materialized columns.

use std::sync::{Arc, Mutex};

use postgres::Client;
use pylon_crdt::{
    apply_update as crdt_apply_update, encode_snapshot, encode_update_since,
    loro::{LoroDoc, VersionVector},
    project_doc_to_json, CrdtField,
};
use pylon_storage::pg_exec::PgConn;
use serde_json::Value;

use crate::loro_store::{
    doc_from_snapshot, merge_client_update, patch_doc, seed_doc, LoroStoreError, Peers,
    SideRecords, SideTable, ROW_SIDE_TABLES,
};

/// SQL to create the PG sidecar table. Idempotent — called every time
/// the runtime opens a Postgres backend so a fresh database gets the
/// table without a manual migration step.
pub const CREATE_PG_SIDECAR_SQL: &str = "\
CREATE TABLE IF NOT EXISTS _pylon_crdt_snapshots (\
    entity     text NOT NULL,\
    row_id     text NOT NULL,\
    snapshot   bytea NOT NULL,\
    updated_at timestamptz NOT NULL DEFAULT now(),\
    PRIMARY KEY (entity, row_id)\
)";

/// The server-side records kept per CRDT row beside its snapshot (see
/// [`SideRecords`]); the same tables as the SQLite sidecar.
const CREATE_PG_SIDE_TABLES_SQL: &str = "
CREATE TABLE IF NOT EXISTS _pylon_crdt_synthetic (
    entity text NOT NULL,
    row_id text NOT NULL,
    peer   bigint NOT NULL,
    PRIMARY KEY (entity, row_id, peer)
);
CREATE TABLE IF NOT EXISTS _pylon_crdt_base (
    entity text NOT NULL,
    row_id text NOT NULL,
    field  text NOT NULL,
    base   text NOT NULL,
    PRIMARY KEY (entity, row_id, field)
);
CREATE TABLE IF NOT EXISTS _pylon_crdt_links (
    entity text NOT NULL,
    row_id text NOT NULL,
    source text NOT NULL,
    links  text NOT NULL,
    PRIMARY KEY (entity, row_id, source)
);";

pub fn ensure_sidecar(client: &mut Client) -> Result<(), LoroStoreError> {
    client
        .execute(CREATE_PG_SIDECAR_SQL, &[])
        .map(|_| ())
        .map_err(|e| pg_err("create pg sidecar", e))?;
    client
        .batch_execute(CREATE_PG_SIDE_TABLES_SQL)
        .map_err(|e| pg_err("create pg side tables", e))
}

/// A Postgres error with its server message and SQLSTATE (`postgres::Error`
/// displays only "db error").
pub(crate) fn pg_err(what: &str, e: postgres::Error) -> LoroStoreError {
    match e.as_db_error() {
        Some(db) => LoroStoreError::Storage(format!(
            "{what}: {} (SQLSTATE {})",
            db.message(),
            db.code().code()
        )),
        None => LoroStoreError::Storage(format!("{what}: {e}")),
    }
}

/// Take the row's CRDT advisory lock for the rest of the transaction.
/// Every CRDT write, and a delete, takes it before any row lock, so they
/// queue on it instead of locking the snapshot, side, and entity rows in
/// different orders.
fn lock_doc<C: PgConn>(conn: &mut C, entity: &str, row_id: &str) -> Result<(), LoroStoreError> {
    conn.execute(
        "SELECT pg_advisory_xact_lock($1::int, $2::int)",
        &[&pg_advisory_key(entity), &pg_advisory_key(row_id)],
    )
    .map(|_| ())
    .map_err(|e| pg_err("crdt advisory lock", e))
}

/// Delete a row's snapshot and side records, in the caller's transaction,
/// under the row's advisory lock (a write in flight finishes first).
pub fn delete_row_records<C: PgConn>(
    conn: &mut C,
    entity: &str,
    row_id: &str,
) -> Result<(), LoroStoreError> {
    lock_doc(conn, entity, row_id)?;
    for table in std::iter::once("_pylon_crdt_snapshots").chain(ROW_SIDE_TABLES) {
        conn.execute(
            &format!("DELETE FROM {table} WHERE entity = $1 AND row_id = $2"),
            &[&entity, &row_id],
        )
        .map_err(|e| pg_err(&format!("delete from {table}"), e))?;
    }
    Ok(())
}

/// Postgres counterpart of [`crate::loro_store::prune_batch`]: up to
/// `batch` snapshots of `entity` in `scope`, with the rows' side records,
/// in one transaction.
pub fn prune_batch(
    client: &mut Client,
    entity: &str,
    scope: crate::loro_store::PruneScope,
    batch: i64,
) -> Result<u64, LoroStoreError> {
    let sql = match scope {
        crate::loro_store::PruneScope::Orphans => format!(
            "SELECT s.row_id FROM _pylon_crdt_snapshots s
             WHERE s.entity = $1
               AND NOT EXISTS (SELECT 1 FROM \"{}\" t WHERE t.id = s.row_id)
             LIMIT $2",
            entity.replace('"', "\"\"")
        ),
        crate::loro_store::PruneScope::All => {
            "SELECT row_id FROM _pylon_crdt_snapshots WHERE entity = $1 LIMIT $2".to_string()
        }
    };
    let err = |e: postgres::Error| pg_err(&format!("prune pg snapshots of {entity}"), e);
    let mut tx = client.transaction().map_err(err)?;
    let row_ids: Vec<String> = tx
        .query(sql.as_str(), &[&entity, &batch])
        .map_err(err)?
        .into_iter()
        .map(|r| r.get(0))
        .collect();
    for row_id in &row_ids {
        delete_row_records(&mut tx, entity, row_id)?;
    }
    tx.commit().map_err(err)?;
    Ok(row_ids.len() as u64)
}

/// Entities that have at least one snapshot.
pub fn snapshot_entities(client: &mut Client) -> Result<Vec<String>, LoroStoreError> {
    client
        .query("SELECT DISTINCT entity FROM _pylon_crdt_snapshots", &[])
        .map(|rows| rows.into_iter().map(|r| r.get::<_, String>(0)).collect())
        .map_err(|e| pg_err("list pg snapshot entities", e))
}

/// The side records in a Postgres database, on the caller's connection
/// (normally the write's transaction).
pub(crate) struct PgSide<'a, C: PgConn>(pub &'a mut C);

impl<C: PgConn> SideRecords for PgSide<'_, C> {
    fn synthetic_peers(
        &mut self,
        entity: &str,
        row_id: &str,
    ) -> Result<std::collections::HashSet<u64>, LoroStoreError> {
        let rows = self
            .0
            .query(
                "SELECT peer FROM _pylon_crdt_synthetic WHERE entity = $1 AND row_id = $2",
                &[&entity, &row_id],
            )
            .map_err(|e| pg_err("read synthetic peers", e))?;
        Ok(rows
            .into_iter()
            .map(|r| r.get::<_, i64>(0) as u64)
            .collect())
    }

    fn record_synthetic(
        &mut self,
        entity: &str,
        row_id: &str,
        peer: u64,
    ) -> Result<(), LoroStoreError> {
        self.0
            .execute(
                "INSERT INTO _pylon_crdt_synthetic (entity, row_id, peer) VALUES ($1, $2, $3)
                 ON CONFLICT DO NOTHING",
                &[&entity, &row_id, &(peer as i64)],
            )
            .map(|_| ())
            .map_err(|e| pg_err("record a synthetic peer", e))
    }

    fn read_json(
        &mut self,
        table: SideTable,
        entity: &str,
        row_id: &str,
    ) -> Result<Vec<(String, String)>, LoroStoreError> {
        let (name, key, value) = table.columns();
        let rows = self
            .0
            .query(
                &format!("SELECT {key}, {value} FROM {name} WHERE entity = $1 AND row_id = $2"),
                &[&entity, &row_id],
            )
            .map_err(|e| pg_err(&format!("read {name}"), e))?;
        Ok(rows.into_iter().map(|r| (r.get(0), r.get(1))).collect())
    }

    fn write_json(
        &mut self,
        table: SideTable,
        entity: &str,
        row_id: &str,
        key: &str,
        json: &str,
    ) -> Result<(), LoroStoreError> {
        let (name, key_col, value_col) = table.columns();
        self.0
            .execute(
                &format!(
                    "INSERT INTO {name} (entity, row_id, {key_col}, {value_col})
                     VALUES ($1, $2, $3, $4)
                     ON CONFLICT (entity, row_id, {key_col}) DO UPDATE SET
                        {value_col} = EXCLUDED.{value_col}"
                ),
                &[&entity, &row_id, &key, &json],
            )
            .map(|_| ())
            .map_err(|e| pg_err(&format!("write {name}"), e))
    }

    fn delete_json(
        &mut self,
        table: SideTable,
        entity: &str,
        row_id: &str,
        key: &str,
    ) -> Result<(), LoroStoreError> {
        let (name, key_col, _) = table.columns();
        self.0
            .execute(
                &format!("DELETE FROM {name} WHERE entity = $1 AND row_id = $2 AND {key_col} = $3"),
                &[&entity, &row_id, &key],
            )
            .map(|_| ())
            .map_err(|e| pg_err(&format!("delete from {name}"), e))
    }
}

/// PG analogue of `LoroStore`. Lives on the Postgres-backed runtime;
/// holds the per-row LoroDoc cache (mutated only behind the inner
/// per-row Mutex) and persists snapshots to the PG sidecar table.
pub struct PgLoroStore {
    /// Bounded; see [`crate::crdt_cache`].
    docs: crate::crdt_cache::DocCache,
    peers: Peers,
    /// When the Postgres server this process last wrote through started
    /// (`pg_postmaster_start_time()`, epoch seconds). A failover or a
    /// point-in-time restore answers from another server instance, whose
    /// data can be behind what this process wrote: the peers are renewed,
    /// so no op id is used twice.
    server_started: Mutex<Option<f64>>,
}

impl Default for PgLoroStore {
    fn default() -> Self {
        Self::new()
    }
}

impl PgLoroStore {
    pub fn new() -> Self {
        Self {
            docs: Default::default(),
            peers: Peers::new(),
            server_started: Mutex::new(None),
        }
    }

    /// Record the start time of the Postgres server a write reads from.
    /// When it changed (a failover, restart, or restore), the peers are
    /// renewed and the cache cleared. Returns whether it changed.
    fn note_server_start(&self, started: f64) -> bool {
        let mut known = self.server_started.lock().unwrap();
        let changed = known.is_some_and(|k| k != started);
        if changed {
            tracing::warn!(
                "[crdt-pg] the Postgres server changed (failover, restart, or restore): \
                 new CRDT peers for this process"
            );
            self.peers.renew();
            self.docs.clear();
        }
        *known = Some(started);
        changed
    }

    /// Hydrate a doc for a CRDT write, taking a transaction-scoped
    /// advisory lock keyed on (entity, row_id). The lock auto-releases
    /// at COMMIT/ROLLBACK.
    ///
    /// Why advisory + not row-level: `SELECT ... FOR UPDATE` only
    /// locks rows that exist. The very first CRDT write to a row has
    /// no sidecar row to lock, so two replicas would both hydrate
    /// empty docs and race the UPSERT. The advisory lock is keyed on
    /// the (entity, row_id) hash and works whether the sidecar row
    /// exists or not. Codex flagged this.
    ///
    /// Bypass the in-memory cache on the write path — the cache is
    /// only safe to read across tx boundaries when every write goes
    /// through ONE process. For multi-replica it's a foot-gun.
    /// Re-decoding the snapshot per write is cheap (a few hundred µs
    /// for a typical row) compared to the round-trip we already pay.
    fn hydrate_for_write<C: PgConn>(
        &self,
        conn: &mut C,
        entity: &str,
        row_id: &str,
    ) -> Result<(LoroDoc, bool), LoroStoreError> {
        lock_doc(conn, entity, row_id)?;
        let row = conn
            .query_opt(
                "SELECT (SELECT snapshot FROM _pylon_crdt_snapshots \
                         WHERE entity = $1 AND row_id = $2), \
                        extract(epoch FROM pg_postmaster_start_time())::float8",
                &[&entity, &row_id],
            )
            .map_err(|e| pg_err("read pg snapshot", e))?
            .ok_or_else(|| LoroStoreError::Storage("read pg snapshot: no row".into()))?;
        let snapshot: Option<Vec<u8>> = row.get(0);
        let started: f64 = row.get(1);
        self.note_server_start(started);
        let doc = doc_from_snapshot(snapshot.as_deref(), &self.peers)?;
        Ok((doc, snapshot.is_some()))
    }
}

/// Hash a string into an i32 suitable for `pg_advisory_xact_lock`.
/// PG's two-key advisory lock form takes int4 args; we use SipHash
/// (the std hasher) and truncate to 32 bits. Collisions are
/// possible but the worst outcome is two unrelated rows blocking
/// each other briefly — never correctness loss.
fn pg_advisory_key(s: &str) -> i32 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut hasher);
    let h = hasher.finish();
    // Take the low 32 bits and reinterpret as i32 — PG accepts the
    // full int4 range. Using `as i32` would panic-truncate; `as u32
    // as i32` round-trips through the bit pattern.
    (h as u32) as i32
}

impl PgLoroStore {
    /// Read-only hydrate — no FOR UPDATE lock. Used by `snapshot()`
    /// and `update_since()` which don't mutate. Hits the in-memory
    /// cache on a hit so repeated reads of the same row don't pay
    /// the decode cost.
    fn get_or_hydrate_read<C: PgConn>(
        &self,
        conn: &mut C,
        entity: &str,
        row_id: &str,
    ) -> Result<Arc<Mutex<LoroDoc>>, LoroStoreError> {
        // Fast path: already cached.
        if let Some(doc) = self.docs.get(entity, row_id) {
            return Ok(doc);
        }

        let snapshot: Option<Vec<u8>> = conn
            .query_opt(
                "SELECT snapshot FROM _pylon_crdt_snapshots WHERE entity = $1 AND row_id = $2",
                &[&entity, &row_id],
            )
            .map_err(|e| pg_err("read pg snapshot", e))?
            .map(|r| r.get::<_, Vec<u8>>(0));

        let doc = doc_from_snapshot(snapshot.as_deref(), &self.peers)?;
        let handle = Arc::new(Mutex::new(doc));
        Ok(self.docs.get_or_insert(entity, row_id, handle))
    }

    /// Persist the current snapshot via UPSERT. Called after every
    /// apply.
    fn persist_snapshot<C: PgConn>(
        conn: &mut C,
        entity: &str,
        row_id: &str,
        doc: &LoroDoc,
    ) -> Result<(), LoroStoreError> {
        let snap = encode_snapshot(doc);
        conn.execute(
            "INSERT INTO _pylon_crdt_snapshots (entity, row_id, snapshot, updated_at) \
             VALUES ($1, $2, $3, now()) \
             ON CONFLICT (entity, row_id) DO UPDATE \
             SET snapshot = EXCLUDED.snapshot, updated_at = EXCLUDED.updated_at",
            &[&entity, &row_id, &snap],
        )
        .map(|_| ())
        .map_err(|e| pg_err("persist pg snapshot", e))
    }

    /// Apply a JSON patch, persist the new snapshot, return the
    /// projected JSON. Caller is responsible for materializing the
    /// projected JSON into the entity row — typically done in the
    /// same `with_transaction_raw` so both writes share BEGIN/COMMIT.
    ///
    /// Multi-replica safe: hydrates under the row's advisory lock, which
    /// serializes concurrent writes to the same row across processes.
    /// Bypasses the in-memory cache on the write path, so a stale cache
    /// from a different process can't shadow the snapshot just read; the
    /// caller evicts the row's cached doc after the transaction ends.
    pub fn apply_patch<C: PgConn>(
        &self,
        conn: &mut C,
        entity: &str,
        row_id: &str,
        fields: &[CrdtField],
        patch: &Value,
    ) -> Result<Value, LoroStoreError> {
        let (doc, _) = self.hydrate_for_write(conn, entity, row_id)?;
        patch_doc(&mut PgSide(conn), &doc, entity, row_id, fields, patch)?;
        Self::persist_snapshot(conn, entity, row_id, &doc)?;
        Ok(project_doc_to_json(&doc, fields))
    }

    /// Apply a binary update from a peer. Returns the projected JSON
    /// for re-materialization on the entity row. Same locking shape
    /// as `apply_patch`.
    pub fn apply_remote_update<C: PgConn>(
        &self,
        conn: &mut C,
        entity: &str,
        row_id: &str,
        fields: &[CrdtField],
        update: &[u8],
    ) -> Result<Value, LoroStoreError> {
        let (doc, _) = self.hydrate_for_write(conn, entity, row_id)?;
        crdt_apply_update(&doc, update).map_err(LoroStoreError::Decode)?;
        Self::persist_snapshot(conn, entity, row_id, &doc)?;
        let projected = project_doc_to_json(&doc, fields);
        Ok(projected)
    }

    /// Apply `values` to the row's doc as one synthetic write (see
    /// [`seed_doc`]) and persist it. Returns the fields the doc could not
    /// take, with why.
    pub fn apply_seed_fields<C: PgConn>(
        &self,
        conn: &mut C,
        entity: &str,
        row_id: &str,
        fields: &[CrdtField],
        values: Vec<(String, Value)>,
    ) -> Result<Vec<(String, String)>, LoroStoreError> {
        if values.is_empty() {
            return Ok(Vec::new());
        }
        let (doc, _) = self.hydrate_for_write(conn, entity, row_id)?;
        let failed = seed_doc(
            &mut PgSide(conn),
            &doc,
            &self.peers,
            entity,
            row_id,
            fields,
            values,
        )?;
        Self::persist_snapshot(conn, entity, row_id, &doc)?;
        Ok(failed)
    }

    /// Import a client's update into the row's doc (see
    /// [`merge_client_update`]) and persist it. Also returns whether the
    /// row had a doc before.
    pub fn apply_client_update<C: PgConn>(
        &self,
        conn: &mut C,
        entity: &str,
        row_id: &str,
        fields: &[CrdtField],
        update: &[u8],
    ) -> Result<(Value, Option<Vec<String>>, bool), LoroStoreError> {
        let (doc, had_doc) = self.hydrate_for_write(conn, entity, row_id)?;
        let (before, touched) = merge_client_update(
            &mut PgSide(conn),
            &doc,
            &self.peers,
            entity,
            row_id,
            fields,
            update,
        )?;
        if touched.is_some() {
            Self::persist_snapshot(conn, entity, row_id, &doc)?;
        }
        Ok((before, touched, had_doc))
    }

    /// Whether the row has a stored snapshot.
    pub fn has_snapshot<C: PgConn>(
        conn: &mut C,
        entity: &str,
        row_id: &str,
    ) -> Result<bool, LoroStoreError> {
        conn.query_opt(
            "SELECT 1 FROM _pylon_crdt_snapshots WHERE entity = $1 AND row_id = $2",
            &[&entity, &row_id],
        )
        .map(|r| r.is_some())
        .map_err(|e| pg_err("read pg snapshot", e))
    }

    /// The row's doc as stored, as JSON for `fields`.
    pub fn project_stored<C: PgConn>(
        &self,
        conn: &mut C,
        entity: &str,
        row_id: &str,
        fields: &[CrdtField],
    ) -> Result<Value, LoroStoreError> {
        let snapshot = Self::read_snapshot_via_conn(conn, entity, row_id)?;
        let doc = doc_from_snapshot(Some(&snapshot), &self.peers)?;
        Ok(project_doc_to_json(&doc, fields))
    }

    /// Bring a row's doc up to its values where the doc lacks them, under
    /// the row's advisory lock: a row with no doc (from before its entity
    /// was CRDT, or written around the runtime) gets one, stored even when
    /// empty; a doc without a field the row holds (left by an older build,
    /// which made a doc of only the fields a server update wrote) takes the
    /// row's value. Returns whether the doc changed. `row` reads the row as
    /// the doc holds it (see `crate::pg_row_for_doc`).
    pub fn seed_missing_fields<C: PgConn>(
        &self,
        conn: &mut C,
        entity: &str,
        row_id: &str,
        fields: &[CrdtField],
        row: impl FnOnce(&mut C) -> Result<Option<Value>, LoroStoreError>,
    ) -> Result<bool, LoroStoreError> {
        let (doc, had_doc) = self.hydrate_for_write(conn, entity, row_id)?;
        let held = crate::loro_store::held_keys(&doc, fields);
        if had_doc && held.len() == fields.len() {
            return Ok(false);
        }
        let Some(row) = row(conn)? else {
            return Ok(false);
        };
        let values = crate::missing_values(fields, &held, &row);
        if values.is_empty() {
            if !had_doc {
                Self::persist_snapshot(conn, entity, row_id, &doc)?;
                self.evict(entity, row_id);
                return Ok(true);
            }
            return Ok(false);
        }
        let failed = self.apply_seed_fields(conn, entity, row_id, fields, values)?;
        for (field, why) in failed {
            tracing::warn!(entity, row_id, field = %field, "seed a CRDT doc from its row: {why}");
        }
        self.evict(entity, row_id);
        Ok(true)
    }

    /// Which of `fields` the row's doc holds, from the read cache. None
    /// when the row has no stored snapshot.
    pub fn held_fields<C: PgConn>(
        &self,
        conn: &mut C,
        entity: &str,
        row_id: &str,
        fields: &[CrdtField],
    ) -> Result<Option<Vec<String>>, LoroStoreError> {
        if !Self::has_snapshot(conn, entity, row_id)? {
            return Ok(None);
        }
        let handle = self.get_or_hydrate_read(conn, entity, row_id)?;
        let doc = handle.lock().unwrap();
        Ok(Some(crate::loro_store::held_keys(&doc, fields)))
    }

    /// Full snapshot for the row. Returns the encoded LoroDoc bytes
    /// (empty doc if the row hasn't been written yet — same shape as
    /// the SQLite path). Read-only, no FOR UPDATE.
    pub fn snapshot<C: PgConn>(
        &self,
        conn: &mut C,
        entity: &str,
        row_id: &str,
    ) -> Result<Vec<u8>, LoroStoreError> {
        let handle = self.get_or_hydrate_read(conn, entity, row_id)?;
        let doc = handle.lock().unwrap();
        Ok(encode_snapshot(&doc))
    }

    /// Incremental update since `since` for catch-up. Same shape as
    /// the SQLite path's `update_since`.
    pub fn update_since<C: PgConn>(
        &self,
        conn: &mut C,
        entity: &str,
        row_id: &str,
        since: &VersionVector,
    ) -> Result<Vec<u8>, LoroStoreError> {
        let handle = self.get_or_hydrate_read(conn, entity, row_id)?;
        let doc = handle.lock().unwrap();
        Ok(encode_update_since(&doc, since))
    }

    /// Current Loro VV for a row, as the encoded bytes the WS
    /// broadcast path remembers between writes. SQLite-path parity
    /// — the broadcast layer doesn't care which backend it came
    /// from, it just stashes the bytes and feeds them back to
    /// `update_since_bytes` on the next write.
    pub fn current_vv_bytes<C: PgConn>(
        &self,
        conn: &mut C,
        entity: &str,
        row_id: &str,
    ) -> Result<Option<Vec<u8>>, LoroStoreError> {
        let handle = self.get_or_hydrate_read(conn, entity, row_id)?;
        let doc = handle.lock().unwrap();
        Ok(Some(doc.oplog_vv().encode()))
    }

    /// Bytes-shaped wrapper around `update_since` — decodes the
    /// supplied VV bytes, delegates to the in-memory doc, returns
    /// the delta. Malformed VV bytes return a typed Decode error;
    /// the broadcast layer falls back to a snapshot rather than
    /// crashing the write path.
    pub fn update_since_bytes<C: PgConn>(
        &self,
        conn: &mut C,
        entity: &str,
        row_id: &str,
        since: &[u8],
    ) -> Result<Option<Vec<u8>>, LoroStoreError> {
        let parsed = VersionVector::decode(since)
            .map_err(|e| LoroStoreError::Decode(format!("decode VV for {entity}/{row_id}: {e}")))?;
        let handle = self.get_or_hydrate_read(conn, entity, row_id)?;
        let doc = handle.lock().unwrap();
        Ok(Some(encode_update_since(&doc, &parsed)))
    }

    /// Read the snapshot bytes directly through the supplied
    /// connection, bypassing the in-memory cache. Used by the
    /// crdt_apply_update path: the cache may hold stale bytes from a
    /// prior read (snapshot() populates it), and we need the bytes
    /// that *just* committed to land in the broadcast.
    pub fn read_snapshot_via_conn<C: PgConn>(
        conn: &mut C,
        entity: &str,
        row_id: &str,
    ) -> Result<Vec<u8>, LoroStoreError> {
        let snap: Option<Vec<u8>> = conn
            .query_opt(
                "SELECT snapshot FROM _pylon_crdt_snapshots WHERE entity = $1 AND row_id = $2",
                &[&entity, &row_id],
            )
            .map_err(|e| pg_err("read pg snapshot", e))?
            .map(|r| r.get::<_, Vec<u8>>(0));
        let bytes = snap.unwrap_or_default();
        // If the row exists, return its bytes verbatim. If it
        // doesn't, return an encoded empty doc so the broadcast
        // shape stays consistent with the SQLite path.
        if bytes.is_empty() {
            let doc = LoroDoc::new();
            Ok(encode_snapshot(&doc))
        } else {
            Ok(bytes)
        }
    }

    /// Drop a row's cached doc. Next access re-hydrates from the PG
    /// sidecar.
    pub fn evict(&self, entity: &str, row_id: &str) {
        self.docs.remove(entity, row_id);
    }

    /// Diagnostic — number of rows cached in memory.
    pub fn cached_rows(&self) -> usize {
        self.docs.len()
    }
}

// ---------------------------------------------------------------------------
// PgCrdtHook impl — bridges pylon-storage's PgTxStore to PgLoroStore so
// TS-mutation `ctx.db.X` calls maintain the CRDT sidecar in the same tx.
// ---------------------------------------------------------------------------

use pylon_kernel::AppManifest;
use pylon_storage::pg_tx_store::PgCrdtHook;

/// Bridge struct that lets PgTxStore (in pylon-storage) call back
/// into the runtime's CRDT machinery without a direct dependency.
/// Lives only for the duration of a single mutation tx.
pub struct PgCrdtHookImpl {
    /// Reference to the runtime's PgLoroStore. `Arc` so the trait
    /// object can be cloned across the storage / runtime boundary.
    pub crdt: std::sync::Arc<PgLoroStore>,
    /// Shared with the runtime so we can resolve the field-shape
    /// for each CRDT entity (which Loro types each field uses).
    pub manifest: std::sync::Arc<AppManifest>,
}

impl PgCrdtHook for PgCrdtHookImpl {
    fn before_insert(
        &self,
        tx: &mut postgres::Transaction<'_>,
        entity: &str,
        data: &serde_json::Value,
    ) -> Result<Option<serde_json::Value>, pylon_http::DataError> {
        let ent = self
            .manifest
            .entities
            .iter()
            .find(|e| e.name == entity)
            .ok_or_else(|| pylon_http::DataError {
                code: "ENTITY_NOT_FOUND".into(),
                message: format!("Unknown entity: {entity}"),
            })?;
        let crdt_fields = crdt_fields_for(ent)?;

        // If the caller supplied an `id`, reuse it as the snapshot
        // key so the materialized row and the sidecar stay aligned.
        // Otherwise generate one and inject it back into the data
        // (build_insert_sql honors `data["id"]`).
        let id = data
            .get("id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(crate::generate_id);

        self.crdt
            .apply_patch(tx, entity, &id, &crdt_fields, data)
            .map_err(|e| pylon_http::DataError {
                code: "CRDT_APPLY_FAILED".into(),
                message: format!("crdt write {entity}/{id}: {e}"),
            })?;

        // Bake the id back into the row so PgTxStore's tx_insert
        // uses it instead of generating a fresh one.
        let mut row = data.clone();
        if let Some(obj) = row.as_object_mut() {
            obj.insert("id".into(), serde_json::Value::String(id.clone()));
        }
        Ok(Some(row))
    }

    fn before_update(
        &self,
        tx: &mut postgres::Transaction<'_>,
        entity: &str,
        id: &str,
        data: &serde_json::Value,
    ) -> Result<Option<serde_json::Value>, pylon_http::DataError> {
        let ent = self
            .manifest
            .entities
            .iter()
            .find(|e| e.name == entity)
            .ok_or_else(|| pylon_http::DataError {
                code: "ENTITY_NOT_FOUND".into(),
                message: format!("Unknown entity: {entity}"),
            })?;
        let crdt_fields = crdt_fields_for(ent)?;
        let projected =
            crate::pg_crdt_patch(&self.crdt, tx, ent, &crdt_fields, id, data).map_err(|e| {
                pylon_http::DataError {
                    code: "CRDT_APPLY_FAILED".into(),
                    message: format!("crdt update {entity}/{id}: {e}"),
                }
            })?;
        Ok(
            crate::crdt_container_corrections(&crdt_fields, data, &projected)
                .map(|c| crate::serialize_json_fields_for_storage(ent, &c).unwrap_or(c)),
        )
    }

    fn before_delete(
        &self,
        tx: &mut postgres::Transaction<'_>,
        entity: &str,
        id: &str,
    ) -> Result<(), pylon_http::DataError> {
        // Drop the sidecar row inside the same tx; runtime evicts
        // cache entry on commit via after_commit/on_rollback.
        delete_row_records(tx, entity, id).map_err(|e| pylon_http::DataError {
            code: "CRDT_SIDECAR_DELETE_FAILED".into(),
            message: format!("delete pg crdt snapshot {entity}/{id}: {e}"),
        })
    }

    fn after_commit(&self, entity: &str, id: &str) {
        // The next read hydrates from the committed snapshot.
        self.crdt.evict(entity, id);
    }

    fn on_rollback(&self, entity: &str, id: &str) {
        // Rolled-back tx: the in-memory doc may have been mutated
        // in place by apply_patch. Evict to force re-hydration from
        // the (unchanged) persisted snapshot.
        self.crdt.evict(entity, id);
    }
}

/// Resolve the CRDT field shape for an entity. Same logic as
/// `Runtime::crdt_fields_for` but without the runtime borrow — the
/// hook lives across the storage/runtime boundary and only needs
/// the manifest. Returns `Err` if any field's CRDT annotation is
/// invalid, matching `Runtime::crdt_fields_for`'s strict behavior:
/// silently dropping an invalid field would commit the SQL row
/// while omitting that field from the snapshot — exactly the
/// sidecar/row divergence we're trying to prevent. Codex flagged.
fn crdt_fields_for(
    ent: &pylon_kernel::ManifestEntity,
) -> Result<Vec<pylon_crdt::CrdtField>, pylon_http::DataError> {
    let mut out = Vec::with_capacity(ent.fields.len());
    for f in &ent.fields {
        if f.name == "id" {
            continue;
        }
        let kind =
            pylon_crdt::field_kind(&f.field_type, f.crdt).map_err(|e| pylon_http::DataError {
                code: "INVALID_CRDT_FIELD".into(),
                message: format!(
                    "{}.{}: {e} (declared type={}, crdt={:?})",
                    ent.name, f.name, f.field_type, f.crdt
                ),
            })?;
        out.push(pylon_crdt::CrdtField {
            name: f.name.clone(),
            kind,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A write through another Postgres server instance (a failover or a
    /// restore) takes new peers; the same server keeps them.
    #[test]
    fn a_new_server_start_renews_the_peers() {
        let store = PgLoroStore::new();
        let peers = |s: &PgLoroStore| (s.peers.server(), s.peers.synthetic());
        let first = peers(&store);
        assert!(!store.note_server_start(100.0));
        assert!(!store.note_server_start(100.0));
        assert_eq!(peers(&store), first);
        assert!(store.note_server_start(250.5));
        let renewed = peers(&store);
        assert_ne!(renewed.0, first.0);
        assert_ne!(renewed.1, first.1);
        assert!(!store.note_server_start(250.5));
        assert_eq!(peers(&store), renewed);
    }
}
