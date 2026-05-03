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

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use postgres::Client;
use pylon_crdt::{
    apply_patch, apply_update as crdt_apply_update, encode_snapshot, encode_update_since,
    loro::{LoroDoc, VersionVector},
    project_doc_to_json, CrdtField,
};
use pylon_storage::pg_exec::PgConn;
use serde_json::Value;

use crate::loro_store::LoroStoreError;

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

pub fn ensure_sidecar(client: &mut Client) -> Result<(), LoroStoreError> {
    client
        .execute(CREATE_PG_SIDECAR_SQL, &[])
        .map(|_| ())
        .map_err(|e| LoroStoreError::Storage(format!("create pg sidecar: {e}")))
}

/// PG analogue of `LoroStore`. Lives on the Postgres-backed runtime;
/// holds the per-row LoroDoc cache (mutated only behind the inner
/// per-row Mutex) and persists snapshots to the PG sidecar table.
#[derive(Default)]
pub struct PgLoroStore {
    docs: Mutex<HashMap<(String, String), Arc<Mutex<LoroDoc>>>>,
}

impl PgLoroStore {
    pub fn new() -> Self {
        Self::default()
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
        conn: &mut C,
        entity: &str,
        row_id: &str,
    ) -> Result<LoroDoc, LoroStoreError> {
        // pg_advisory_xact_lock(key1, key2) — two-key form fits the
        // (entity, row_id) tuple naturally. We hash each side into an
        // i32 so the same logical row maps to the same lock across
        // processes. Released automatically at tx end.
        let entity_key = pg_advisory_key(entity);
        let row_key = pg_advisory_key(row_id);
        conn.execute(
            "SELECT pg_advisory_xact_lock($1::int, $2::int)",
            &[&entity_key, &row_key],
        )
        .map_err(|e| LoroStoreError::Storage(format!("crdt advisory lock: {e}")))?;

        let snapshot: Option<Vec<u8>> = conn
            .query_opt(
                "SELECT snapshot FROM _pylon_crdt_snapshots \
                 WHERE entity = $1 AND row_id = $2",
                &[&entity, &row_id],
            )
            .map_err(|e| LoroStoreError::Storage(format!("read pg snapshot: {e}")))?
            .map(|r| r.get::<_, Vec<u8>>(0));

        let doc = LoroDoc::new();
        if let Some(bytes) = snapshot {
            crdt_apply_update(&doc, &bytes).map_err(LoroStoreError::Decode)?;
        }
        Ok(doc)
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
        let key = (entity.to_string(), row_id.to_string());

        // Fast path: already cached.
        {
            let guard = self.docs.lock().unwrap();
            if let Some(doc) = guard.get(&key) {
                return Ok(Arc::clone(doc));
            }
        }

        let snapshot: Option<Vec<u8>> = conn
            .query_opt(
                "SELECT snapshot FROM _pylon_crdt_snapshots WHERE entity = $1 AND row_id = $2",
                &[&entity, &row_id],
            )
            .map_err(|e| LoroStoreError::Storage(format!("read pg snapshot: {e}")))?
            .map(|r| r.get::<_, Vec<u8>>(0));

        let doc = LoroDoc::new();
        if let Some(bytes) = snapshot {
            crdt_apply_update(&doc, &bytes).map_err(LoroStoreError::Decode)?;
        }
        let handle = Arc::new(Mutex::new(doc));

        let mut guard = self.docs.lock().unwrap();
        let entry = guard.entry(key).or_insert_with(|| Arc::clone(&handle));
        Ok(Arc::clone(entry))
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
        .map_err(|e| LoroStoreError::Storage(format!("persist pg snapshot: {e}")))
    }

    /// Apply a JSON patch, persist the new snapshot, return the
    /// projected JSON. Caller is responsible for materializing the
    /// projected JSON into the entity row — typically done in the
    /// same `with_transaction_raw` so both writes share BEGIN/COMMIT.
    ///
    /// Multi-replica safe: hydrates with `SELECT ... FOR UPDATE`,
    /// which serializes concurrent updates to the same row across
    /// processes. Bypasses the in-memory cache on the write path —
    /// the cache only updates after commit (see `cache_after_commit`),
    /// so a stale cache from a different process can't shadow the
    /// row-locked snapshot we just read.
    pub fn apply_patch<C: PgConn>(
        &self,
        conn: &mut C,
        entity: &str,
        row_id: &str,
        fields: &[CrdtField],
        patch: &Value,
    ) -> Result<Value, LoroStoreError> {
        let doc = Self::hydrate_for_write(conn, entity, row_id)?;
        apply_patch(&doc, fields, patch).map_err(LoroStoreError::Apply)?;
        Self::persist_snapshot(conn, entity, row_id, &doc)?;
        let projected = project_doc_to_json(&doc, fields);
        // Cache update happens through `cache_after_commit` from the
        // runtime layer once the surrounding tx commits. If the tx
        // rolls back, no cache write happens — so the next read
        // hydrates from the (unchanged) sidecar.
        Ok(projected)
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
        let doc = Self::hydrate_for_write(conn, entity, row_id)?;
        crdt_apply_update(&doc, update).map_err(LoroStoreError::Decode)?;
        Self::persist_snapshot(conn, entity, row_id, &doc)?;
        let projected = project_doc_to_json(&doc, fields);
        Ok(projected)
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
            .map_err(|e| LoroStoreError::Storage(format!("read pg snapshot: {e}")))?
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

    /// Refresh the in-memory cache entry for a row from the
    /// just-committed sidecar bytes. Called by the runtime layer
    /// after `with_transaction_raw` commits the CRDT write — this
    /// way the cache only ever reflects what's on disk, and a
    /// rolled-back tx leaves no cache poison.
    ///
    /// On any read error we evict instead of caching stale state.
    pub fn cache_after_commit<C: PgConn>(&self, conn: &mut C, entity: &str, row_id: &str) {
        let snap_result = conn.query_opt(
            "SELECT snapshot FROM _pylon_crdt_snapshots WHERE entity = $1 AND row_id = $2",
            &[&entity, &row_id],
        );
        let bytes = match snap_result {
            Ok(Some(row)) => row.get::<_, Vec<u8>>(0),
            _ => {
                self.evict(entity, row_id);
                return;
            }
        };
        let doc = LoroDoc::new();
        if crdt_apply_update(&doc, &bytes).is_err() {
            self.evict(entity, row_id);
            return;
        }
        let handle = Arc::new(Mutex::new(doc));
        let mut guard = self.docs.lock().unwrap();
        guard.insert((entity.to_string(), row_id.to_string()), handle);
    }

    /// Drop a row's cached doc. Next access re-hydrates from the PG
    /// sidecar.
    pub fn evict(&self, entity: &str, row_id: &str) {
        self.docs
            .lock()
            .unwrap()
            .remove(&(entity.to_string(), row_id.to_string()));
    }

    /// Diagnostic — number of rows cached in memory.
    pub fn cached_rows(&self) -> usize {
        self.docs.lock().unwrap().len()
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
    ) -> Result<(), pylon_http::DataError> {
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
        self.crdt
            .apply_patch(tx, entity, id, &crdt_fields, data)
            .map(|_| ())
            .map_err(|e| pylon_http::DataError {
                code: "CRDT_APPLY_FAILED".into(),
                message: format!("crdt update {entity}/{id}: {e}"),
            })
    }

    fn before_delete(
        &self,
        tx: &mut postgres::Transaction<'_>,
        entity: &str,
        id: &str,
    ) -> Result<(), pylon_http::DataError> {
        // Drop the sidecar row inside the same tx; runtime evicts
        // cache entry on commit via after_commit/on_rollback.
        tx.execute(
            "DELETE FROM _pylon_crdt_snapshots WHERE entity = $1 AND row_id = $2",
            &[&entity, &id],
        )
        .map(|_| ())
        .map_err(|e| pylon_http::DataError {
            code: "CRDT_SIDECAR_DELETE_FAILED".into(),
            message: format!("delete pg crdt snapshot {entity}/{id}: {e}"),
        })
    }

    fn after_commit(&self, entity: &str, id: &str) {
        // Refresh cache via a fresh client connection. Can't pass
        // the tx in here since it's already committed and dropped.
        // The cache_after_commit method on PgLoroStore expects a
        // PgConn — we don't have one here. Simplest: evict so the
        // next read re-hydrates from the persisted snapshot. This
        // is correct (just one extra round-trip for the next read);
        // the alternative would require the runtime to hand us a
        // fresh client which is more plumbing for marginal benefit.
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
