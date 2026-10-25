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
    doc_from_snapshot, doc_from_stored, merge_client_update, patch_doc, plan_persist, seed_doc,
    LoroStoreError, Peers, Persist, SideRecords, SideTable, StoredHead, ROW_SIDE_TABLES,
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
ALTER TABLE _pylon_crdt_snapshots ADD COLUMN IF NOT EXISTS log_count bigint NOT NULL DEFAULT 0;
ALTER TABLE _pylon_crdt_snapshots ADD COLUMN IF NOT EXISTS log_bytes bigint NOT NULL DEFAULT 0;
ALTER TABLE _pylon_crdt_snapshots ADD COLUMN IF NOT EXISTS vv bytea;
CREATE TABLE IF NOT EXISTS _pylon_crdt_updates (
    entity text NOT NULL,
    row_id text NOT NULL,
    seq    bigint NOT NULL,
    delta  bytea NOT NULL,
    PRIMARY KEY (entity, row_id, seq)
);
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

/// Take the row's CRDT advisory lock for the rest of the transaction (see
/// [`pylon_storage::pg_tx_store::lock_crdt_row`]).
fn lock_doc<C: PgConn>(conn: &mut C, entity: &str, row_id: &str) -> Result<(), LoroStoreError> {
    pylon_storage::pg_tx_store::lock_crdt_row(conn, entity, row_id)
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
                "[crdt-pg] the Postgres server restarted or changed (a failover or a \
                 restore): new CRDT peers for this process"
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
    ) -> Result<(LoroDoc, Option<StoredHead>), LoroStoreError> {
        lock_doc(conn, entity, row_id)?;
        self.hydrate_locked(conn, entity, row_id)
    }

    /// [`Self::hydrate_for_write`] for a caller that holds the row's lock
    /// already. The snapshot and its update log are read in one statement,
    /// so they match.
    fn hydrate_locked<C: PgConn>(
        &self,
        conn: &mut C,
        entity: &str,
        row_id: &str,
    ) -> Result<(LoroDoc, Option<StoredHead>), LoroStoreError> {
        let row = conn
            .query_opt(
                "SELECT s.snapshot, s.log_count, s.log_bytes, s.vv, \
                        extract(epoch FROM pg_postmaster_start_time())::float8, \
                        CASE WHEN s.snapshot IS NOT NULL THEN \
                            (SELECT array_agg(u.delta ORDER BY u.seq) FROM _pylon_crdt_updates u \
                             WHERE u.entity = $1 AND u.row_id = $2) END \
                 FROM (SELECT 1) AS one LEFT JOIN _pylon_crdt_snapshots s \
                   ON s.entity = $1 AND s.row_id = $2",
                &[&entity, &row_id],
            )
            .map_err(|e| pg_err("read pg snapshot", e))?
            .ok_or_else(|| LoroStoreError::Storage("read pg snapshot: no row".into()))?;
        let snapshot: Option<Vec<u8>> = row.get(0);
        let started: f64 = row.get(4);
        self.note_server_start(started);
        let head = snapshot.as_ref().map(|snap| {
            StoredHead::from_columns(snap.len() as i64, row.get(1), row.get(2), row.get(3))
        });
        let log: Vec<Vec<u8>> = row.get::<_, Option<Vec<Vec<u8>>>>(5).unwrap_or_default();
        let doc = doc_from_stored(snapshot.as_deref(), &log, &self.peers)?;
        Ok((doc, head))
    }
}

/// A row's update log, in order.
fn read_log_pg<C: PgConn>(
    conn: &mut C,
    entity: &str,
    row_id: &str,
) -> Result<Vec<Vec<u8>>, LoroStoreError> {
    Ok(conn
        .query(
            "SELECT delta FROM _pylon_crdt_updates WHERE entity = $1 AND row_id = $2 ORDER BY seq",
            &[&entity, &row_id],
        )
        .map_err(|e| pg_err("read the pg update log", e))?
        .into_iter()
        .map(|r| r.get::<_, Vec<u8>>(0))
        .collect())
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
    ) -> Result<(Arc<Mutex<LoroDoc>>, bool), LoroStoreError> {
        // One round trip: the stored version, the snapshot only when it is
        // not the cached one's, and the server's start time. A process
        // other than this one may have written the row since it cached it.
        let token = self.docs.token(entity, row_id);
        let cached = self.docs.get_stamped(entity, row_id);
        let known: Option<i64> = cached.as_ref().and_then(|(_, stamp)| *stamp);
        let read = |conn: &mut C, known: Option<i64>| {
            conn.query_opt(
                "SELECT s.stamp, CASE WHEN s.stamp IS DISTINCT FROM $3::bigint \
                                      THEN s.snapshot END, \
                        extract(epoch FROM pg_postmaster_start_time())::float8, \
                        CASE WHEN s.stamp IS DISTINCT FROM $3::bigint THEN \
                            (SELECT array_agg(u.delta ORDER BY u.seq) FROM _pylon_crdt_updates u \
                             WHERE u.entity = $1 AND u.row_id = $2) END \
                 FROM (SELECT 1) AS one LEFT JOIN LATERAL ( \
                     SELECT (extract(epoch FROM updated_at) * 1000000)::bigint AS stamp, \
                            snapshot \
                     FROM _pylon_crdt_snapshots WHERE entity = $1 AND row_id = $2 \
                 ) AS s ON true",
                &[&entity, &row_id, &known],
            )
            .map_err(|e| pg_err("read pg snapshot", e))?
            .ok_or_else(|| LoroStoreError::Storage("read pg snapshot: no row".into()))
        };
        let mut row = read(conn, known)?;
        let mut stamp: Option<i64> = row.get(0);
        let mut token = token;
        if self.note_server_start(row.get(2)) {
            // A new server: the cache was cleared, and the snapshot is read
            // whole.
            token = self.docs.token(entity, row_id);
            row = read(conn, None)?;
            stamp = row.get(0);
        } else if let Some((handle, cached_stamp)) = cached {
            if cached_stamp == stamp {
                return Ok((handle, stamp.is_some()));
            }
        }
        let snapshot: Option<Vec<u8>> = row.get(1);
        // Read in the same statement as the snapshot, so the two match.
        let log: Vec<Vec<u8>> = row.get::<_, Option<Vec<Vec<u8>>>>(3).unwrap_or_default();
        let doc = doc_from_stored(snapshot.as_deref(), &log, &self.peers)?;
        let handle = Arc::new(Mutex::new(doc));
        Ok((
            self.docs.publish(entity, row_id, handle, stamp, token),
            stamp.is_some(),
        ))
    }

    /// The row's cache state, to pass to [`PgLoroStore::mark_complete`].
    pub(crate) fn cache_token(&self, entity: &str, row_id: &str) -> crate::crdt_cache::Token {
        self.docs.token(entity, row_id)
    }

    /// Whether the row's cached doc is known to hold every field its row
    /// has a value for.
    pub(crate) fn is_complete(&self, entity: &str, row_id: &str) -> bool {
        self.docs.is_complete(entity, row_id)
    }

    /// Record that the row's cached doc holds every field its row has a
    /// value for, when nothing replaced it since `token`.
    pub(crate) fn mark_complete(
        &self,
        entity: &str,
        row_id: &str,
        token: crate::crdt_cache::Token,
    ) {
        self.docs.mark_complete(entity, row_id, token);
    }

    /// Store the row's doc after an apply, under the row's lock: the ops
    /// it added, appended to the update log, or a new snapshot when the log
    /// is long (see [`plan_persist`]). `head` is what was stored when the
    /// doc was read under the same lock.
    fn persist_snapshot<C: PgConn>(
        conn: &mut C,
        entity: &str,
        row_id: &str,
        doc: &LoroDoc,
        head: Option<&StoredHead>,
    ) -> Result<(), LoroStoreError> {
        let vv = doc.oplog_vv().encode();
        match plan_persist(head, doc) {
            Persist::Unchanged => Ok(()),
            Persist::Snapshot => {
                let snap = encode_snapshot(doc);
                conn.execute(
                    "WITH cleared AS ( \
                         DELETE FROM _pylon_crdt_updates WHERE entity = $1 AND row_id = $2 \
                     ) \
                     INSERT INTO _pylon_crdt_snapshots \
                        (entity, row_id, snapshot, updated_at, log_count, log_bytes, vv) \
                     VALUES ($1, $2, $3, clock_timestamp(), 0, 0, $4) \
                     ON CONFLICT (entity, row_id) DO UPDATE \
                     SET snapshot = EXCLUDED.snapshot, updated_at = EXCLUDED.updated_at, \
                         log_count = 0, log_bytes = 0, vv = EXCLUDED.vv",
                    &[&entity, &row_id, &snap, &vv],
                )
                .map(|_| ())
                .map_err(|e| pg_err("persist pg snapshot", e))
            }
            Persist::Append(delta) => {
                let head = head.expect("an append has a stored head");
                let seq = head.log_count + 1;
                let len = delta.len() as i64;
                conn.execute(
                    "WITH appended AS ( \
                         INSERT INTO _pylon_crdt_updates (entity, row_id, seq, delta) \
                         VALUES ($1, $2, $3, $4) \
                     ) \
                     UPDATE _pylon_crdt_snapshots \
                     SET log_count = log_count + 1, log_bytes = log_bytes + $5, vv = $6, \
                         updated_at = clock_timestamp() \
                     WHERE entity = $1 AND row_id = $2",
                    &[&entity, &row_id, &seq, &delta, &len, &vv],
                )
                .map(|_| ())
                .map_err(|e| pg_err("append to the pg update log", e))
            }
        }
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
        let (doc, head) = self.hydrate_for_write(conn, entity, row_id)?;
        patch_doc(&mut PgSide(conn), &doc, entity, row_id, fields, patch)?;
        Self::persist_snapshot(conn, entity, row_id, &doc, head.as_ref())?;
        Ok(project_doc_to_json(&doc, fields))
    }

    /// A server write by a caller that holds the row's CRDT lock and has
    /// read the row under its row lock (`row`, as the doc holds it): the
    /// doc takes the row's values it lacks, as [`Self::seed_missing_fields`]
    /// would, then `patch`. One read of the doc and one store. Returns the
    /// projected JSON.
    pub fn apply_patch_locked<C: PgConn>(
        &self,
        conn: &mut C,
        entity: &str,
        row_id: &str,
        fields: &[CrdtField],
        row: &Value,
        patch: &Value,
    ) -> Result<Value, LoroStoreError> {
        let (doc, head) = self.hydrate_locked(conn, entity, row_id)?;
        let held = crate::loro_store::held_keys(&doc, fields);
        if head.is_none() || held.len() < fields.len() {
            let values = crate::missing_values(fields, &held, row);
            let failed = seed_doc(
                &mut PgSide(conn),
                &doc,
                &self.peers,
                entity,
                row_id,
                fields,
                values,
            )?;
            for (field, why) in failed {
                tracing::warn!(entity, row_id, field = %field, "seed a CRDT doc from its row: {why}");
            }
        }
        patch_doc(&mut PgSide(conn), &doc, entity, row_id, fields, patch)?;
        Self::persist_snapshot(conn, entity, row_id, &doc, head.as_ref())?;
        Ok(project_doc_to_json(&doc, fields))
    }

    /// [`Self::apply_patch_locked`] for many rows of one entity, with one
    /// read of their docs and at most two statements to store them. The
    /// caller holds every row's CRDT lock. Per item: the row id, the row
    /// as the doc holds it (`None` for a row being inserted: nothing to
    /// seed from), and the patch. Ids must be distinct. Returns the
    /// projected JSON per item, in order.
    pub fn apply_patches_locked<C: PgConn>(
        &self,
        conn: &mut C,
        entity: &str,
        fields: &[CrdtField],
        items: &[(&str, Option<&Value>, &Value)],
    ) -> Result<Vec<Value>, LoroStoreError> {
        let ids: Vec<&str> = items.iter().map(|(id, _, _)| *id).collect();
        let mut stored = self.hydrate_many_locked(conn, entity, &ids)?;
        let mut docs = Vec::with_capacity(items.len());
        let mut projected = Vec::with_capacity(items.len());
        for (id, row, patch) in items {
            let (doc, head) = stored
                .remove(*id)
                .ok_or_else(|| LoroStoreError::Storage(format!("read pg snapshot {id}: no row")))?;
            if let Some(row) = row {
                let held = crate::loro_store::held_keys(&doc, fields);
                if head.is_none() || held.len() < fields.len() {
                    let values = crate::missing_values(fields, &held, row);
                    let failed = seed_doc(
                        &mut PgSide(conn),
                        &doc,
                        &self.peers,
                        entity,
                        id,
                        fields,
                        values,
                    )?;
                    for (field, why) in failed {
                        tracing::warn!(entity, row_id = %id, field = %field, "seed a CRDT doc from its row: {why}");
                    }
                }
            }
            patch_doc(&mut PgSide(conn), &doc, entity, id, fields, patch)?;
            projected.push(project_doc_to_json(&doc, fields));
            docs.push((*id, doc, head));
        }
        Self::persist_many(conn, entity, &docs)?;
        Ok(projected)
    }

    /// [`Self::hydrate_locked`] for many rows, in one statement.
    fn hydrate_many_locked<C: PgConn>(
        &self,
        conn: &mut C,
        entity: &str,
        ids: &[&str],
    ) -> Result<std::collections::HashMap<String, (LoroDoc, Option<StoredHead>)>, LoroStoreError>
    {
        let rows = conn
            .query(
                "SELECT r.row_id, s.snapshot, s.log_count, s.log_bytes, s.vv, \
                        extract(epoch FROM pg_postmaster_start_time())::float8, \
                        CASE WHEN s.snapshot IS NOT NULL THEN \
                            (SELECT array_agg(u.delta ORDER BY u.seq) FROM _pylon_crdt_updates u \
                             WHERE u.entity = $1 AND u.row_id = r.row_id) END \
                 FROM unnest($2::text[]) AS r(row_id) LEFT JOIN _pylon_crdt_snapshots s \
                   ON s.entity = $1 AND s.row_id = r.row_id",
                &[&entity, &ids],
            )
            .map_err(|e| pg_err("read pg snapshots", e))?;
        if let Some(row) = rows.first() {
            self.note_server_start(row.get(5));
        }
        let mut docs = std::collections::HashMap::with_capacity(rows.len());
        for row in rows {
            let snapshot: Option<Vec<u8>> = row.get(1);
            let head = snapshot.as_ref().map(|snap| {
                StoredHead::from_columns(snap.len() as i64, row.get(2), row.get(3), row.get(4))
            });
            let log: Vec<Vec<u8>> = row.get::<_, Option<Vec<Vec<u8>>>>(6).unwrap_or_default();
            let doc = doc_from_stored(snapshot.as_deref(), &log, &self.peers)?;
            docs.insert(row.get::<_, String>(0), (doc, head));
        }
        Ok(docs)
    }

    /// [`Self::persist_snapshot`] for many rows of one entity: one
    /// statement for the rows whose ops append to their log, one for the
    /// rows that get a new snapshot. Ids must be distinct.
    fn persist_many<C: PgConn>(
        conn: &mut C,
        entity: &str,
        docs: &[(&str, LoroDoc, Option<StoredHead>)],
    ) -> Result<(), LoroStoreError> {
        let (mut append_ids, mut seqs, mut deltas, mut lens, mut append_vvs) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let (mut snapshot_ids, mut snapshots, mut snapshot_vvs) =
            (Vec::new(), Vec::new(), Vec::new());
        for (id, doc, head) in docs {
            let vv = doc.oplog_vv().encode();
            match plan_persist(head.as_ref(), doc) {
                Persist::Unchanged => {}
                Persist::Snapshot => {
                    snapshot_ids.push(*id);
                    snapshots.push(encode_snapshot(doc));
                    snapshot_vvs.push(vv);
                }
                Persist::Append(delta) => {
                    let head = head.as_ref().expect("an append has a stored head");
                    append_ids.push(*id);
                    seqs.push(head.log_count + 1);
                    lens.push(delta.len() as i64);
                    deltas.push(delta);
                    append_vvs.push(vv);
                }
            }
        }
        if !append_ids.is_empty() {
            conn.execute(
                "WITH appended AS ( \
                     INSERT INTO _pylon_crdt_updates (entity, row_id, seq, delta) \
                     SELECT $1, u.row_id, u.seq, u.delta \
                     FROM unnest($2::text[], $3::bigint[], $4::bytea[]) AS u(row_id, seq, delta) \
                 ) \
                 UPDATE _pylon_crdt_snapshots AS s \
                 SET log_count = s.log_count + 1, log_bytes = s.log_bytes + u.len, vv = u.vv, \
                     updated_at = clock_timestamp() \
                 FROM unnest($2::text[], $5::bigint[], $6::bytea[]) AS u(row_id, len, vv) \
                 WHERE s.entity = $1 AND s.row_id = u.row_id",
                &[&entity, &append_ids, &seqs, &deltas, &lens, &append_vvs],
            )
            .map_err(|e| pg_err("append to the pg update logs", e))?;
        }
        if !snapshot_ids.is_empty() {
            conn.execute(
                "WITH cleared AS ( \
                     DELETE FROM _pylon_crdt_updates AS d USING unnest($2::text[]) AS c(row_id) \
                     WHERE d.entity = $1 AND d.row_id = c.row_id \
                 ) \
                 INSERT INTO _pylon_crdt_snapshots \
                    (entity, row_id, snapshot, updated_at, log_count, log_bytes, vv) \
                 SELECT $1, u.row_id, u.snapshot, clock_timestamp(), 0, 0, u.vv \
                 FROM unnest($2::text[], $3::bytea[], $4::bytea[]) AS u(row_id, snapshot, vv) \
                 ON CONFLICT (entity, row_id) DO UPDATE \
                 SET snapshot = EXCLUDED.snapshot, updated_at = EXCLUDED.updated_at, \
                     log_count = 0, log_bytes = 0, vv = EXCLUDED.vv",
                &[&entity, &snapshot_ids, &snapshots, &snapshot_vvs],
            )
            .map_err(|e| pg_err("persist pg snapshots", e))?;
        }
        Ok(())
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
        let (doc, head) = self.hydrate_for_write(conn, entity, row_id)?;
        crdt_apply_update(&doc, update).map_err(LoroStoreError::Decode)?;
        Self::persist_snapshot(conn, entity, row_id, &doc, head.as_ref())?;
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
        let (doc, head) = self.hydrate_for_write(conn, entity, row_id)?;
        let failed = seed_doc(
            &mut PgSide(conn),
            &doc,
            &self.peers,
            entity,
            row_id,
            fields,
            values,
        )?;
        Self::persist_snapshot(conn, entity, row_id, &doc, head.as_ref())?;
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
        let (doc, head) = self.hydrate_for_write(conn, entity, row_id)?;
        let had_doc = head.is_some();
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
            Self::persist_snapshot(conn, entity, row_id, &doc, head.as_ref())?;
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
        let (doc, head) = self.hydrate_for_write(conn, entity, row_id)?;
        let had_doc = head.is_some();
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
                Self::persist_snapshot(conn, entity, row_id, &doc, None)?;
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
        let (handle, exists) = self.get_or_hydrate_read(conn, entity, row_id)?;
        if !exists {
            return Ok(None);
        }
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
        let (handle, _) = self.get_or_hydrate_read(conn, entity, row_id)?;
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
        let (handle, _) = self.get_or_hydrate_read(conn, entity, row_id)?;
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
        let (handle, _) = self.get_or_hydrate_read(conn, entity, row_id)?;
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
        let (handle, _) = self.get_or_hydrate_read(conn, entity, row_id)?;
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
        // If the row doesn't exist, return an encoded empty doc so the
        // broadcast shape stays consistent with the SQLite path. If it
        // does, its stored state: the snapshot, with its update log
        // applied when there is one.
        if bytes.is_empty() {
            let doc = LoroDoc::new();
            return Ok(encode_snapshot(&doc));
        }
        let log = read_log_pg(conn, entity, row_id)?;
        if log.is_empty() {
            return Ok(bytes);
        }
        let doc = doc_from_stored(Some(&bytes), &log, &Peers::new())?;
        Ok(encode_snapshot(&doc))
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

impl PgCrdtHookImpl {
    fn entity(&self, entity: &str) -> Result<&pylon_kernel::ManifestEntity, pylon_http::DataError> {
        self.manifest
            .entities
            .iter()
            .find(|e| e.name == entity)
            .ok_or_else(|| pylon_http::DataError {
                code: "ENTITY_NOT_FOUND".into(),
                message: format!("Unknown entity: {entity}"),
            })
    }
}

impl PgCrdtHook for PgCrdtHookImpl {
    fn before_insert(
        &self,
        tx: &mut pylon_storage::pg_exec::CachedTransaction<'_>,
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
        tx: &mut pylon_storage::pg_exec::CachedTransaction<'_>,
        entity: &str,
        id: &str,
        row: &serde_json::Value,
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
        let row = crate::pg_row_as_doc_holds(ent, &crdt_fields, row.clone());
        let projected = self
            .crdt
            .apply_patch_locked(tx, entity, id, &crdt_fields, &row, data)
            .map_err(|e| pylon_http::DataError {
                code: "CRDT_APPLY_FAILED".into(),
                message: format!("crdt update {entity}/{id}: {e}"),
            })?;
        Ok(
            crate::crdt_container_corrections(&crdt_fields, data, &projected)
                .map(|c| crate::serialize_json_fields_for_storage(ent, &c).unwrap_or(c)),
        )
    }

    fn before_insert_many(
        &self,
        tx: &mut pylon_storage::pg_exec::CachedTransaction<'_>,
        entity: &str,
        rows: &[serde_json::Value],
    ) -> Result<Vec<serde_json::Value>, pylon_http::DataError> {
        let ent = self.entity(entity)?;
        let crdt_fields = crdt_fields_for(ent)?;
        // Each row with its id set, as `before_insert` does.
        let rows: Vec<serde_json::Value> = rows
            .iter()
            .map(|data| {
                let id = data
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(crate::generate_id);
                let mut row = data.clone();
                if let Some(obj) = row.as_object_mut() {
                    obj.insert("id".into(), serde_json::Value::String(id));
                }
                row
            })
            .collect();
        let ids: Vec<&str> = rows
            .iter()
            .map(|row| row["id"].as_str().expect("id set above"))
            .collect();
        let crdt_err = |e: LoroStoreError| pylon_http::DataError {
            code: "CRDT_APPLY_FAILED".into(),
            message: format!("crdt write {entity}: {e}"),
        };
        pylon_storage::pg_tx_store::lock_crdt_rows(tx, entity, &ids)
            .map_err(|e| crdt_err(pg_err("crdt advisory locks", e)))?;
        let items: Vec<(&str, Option<&serde_json::Value>, &serde_json::Value)> = ids
            .iter()
            .zip(&rows)
            .map(|(id, row)| (*id, None, row))
            .collect();
        self.crdt
            .apply_patches_locked(tx, entity, &crdt_fields, &items)
            .map_err(crdt_err)?;
        Ok(rows)
    }

    fn before_update_many(
        &self,
        tx: &mut pylon_storage::pg_exec::CachedTransaction<'_>,
        entity: &str,
        rows: &[pylon_storage::pg_tx_store::LockedUpdate<'_>],
    ) -> Result<Vec<Option<serde_json::Value>>, pylon_http::DataError> {
        let ent = self.entity(entity)?;
        let crdt_fields = crdt_fields_for(ent)?;
        let held: Vec<serde_json::Value> = rows
            .iter()
            .map(|row| crate::pg_row_as_doc_holds(ent, &crdt_fields, row.before.clone()))
            .collect();
        let items: Vec<(&str, Option<&serde_json::Value>, &serde_json::Value)> = rows
            .iter()
            .zip(&held)
            .map(|(row, held)| (row.id, Some(held), row.data))
            .collect();
        let projected = self
            .crdt
            .apply_patches_locked(tx, entity, &crdt_fields, &items)
            .map_err(|e| pylon_http::DataError {
                code: "CRDT_APPLY_FAILED".into(),
                message: format!("crdt update {entity}: {e}"),
            })?;
        Ok(rows
            .iter()
            .zip(&projected)
            .map(|(row, projected)| {
                crate::crdt_container_corrections(&crdt_fields, row.data, projected)
                    .map(|c| crate::serialize_json_fields_for_storage(ent, &c).unwrap_or(c))
            })
            .collect())
    }

    fn before_delete(
        &self,
        tx: &mut pylon_storage::pg_exec::CachedTransaction<'_>,
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

/// Use the runtime's field rules, including server-only vector exclusion.
fn crdt_fields_for(
    ent: &pylon_kernel::ManifestEntity,
) -> Result<Vec<pylon_crdt::CrdtField>, pylon_http::DataError> {
    crate::resolve_crdt_fields(ent).map_err(|error| pylon_http::DataError {
        code: error.code,
        message: error.message,
    })
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
