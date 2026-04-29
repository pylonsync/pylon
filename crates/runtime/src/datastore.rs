//! Implements the platform-agnostic [`DataStore`] trait for [`Runtime`].
//!
//! This bridges the concrete SQLite-backed Runtime to the abstract trait
//! used by the router crate, enabling the same routing logic to run on
//! self-hosted servers and Cloudflare Workers alike.

use pylon_http::{DataError, DataStore};

use crate::Runtime;

// ---------------------------------------------------------------------------
// In-flight mutation schedule buffering
// ---------------------------------------------------------------------------

/// A scheduled function call captured during a mutation handler. Held in
/// the per-mutation pending list until the surrounding transaction
/// commits — at which point we drain and enqueue. On rollback the list
/// is dropped without enqueuing, so a failed mutation can't leave behind
/// scheduled side-effects (the docs claim this; before this buffer the
/// claim was false).
#[derive(Debug, Clone)]
pub(crate) struct PendingSchedule {
    pub fn_name: String,
    pub args: serde_json::Value,
    pub delay_ms: Option<u64>,
    pub run_at: Option<u64>,
}

thread_local! {
    /// Set by the mutation entry point (top-level + nested) for the
    /// duration of a TS handler call. The schedule hook checks this
    /// thread-local: when `Some`, scheduling buffers into the inner
    /// `Vec`; when `None`, the hook enqueues immediately (the
    /// historical, non-mutation behavior). The Bun stdio loop is
    /// single-threaded per call (the runner holds `io_lock` for the
    /// whole call duration), so a thread-local is the right scoping
    /// primitive — no cross-thread leakage.
    pub(crate) static MUTATION_SCHEDULE_BUFFER: std::cell::RefCell<Option<std::rc::Rc<std::cell::RefCell<Vec<PendingSchedule>>>>>
        = const { std::cell::RefCell::new(None) };
}

/// RAII guard that pushes a schedule buffer onto the thread-local for
/// the duration of a mutation handler call, then restores the previous
/// value (which is almost always `None`, but supports nested mutation
/// handlers stacking buffers correctly).
pub(crate) struct ScheduleBufferGuard {
    previous: Option<std::rc::Rc<std::cell::RefCell<Vec<PendingSchedule>>>>,
    current: std::rc::Rc<std::cell::RefCell<Vec<PendingSchedule>>>,
}

impl ScheduleBufferGuard {
    pub(crate) fn enter() -> Self {
        let current = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let previous = MUTATION_SCHEDULE_BUFFER.with(|cell| {
            let mut slot = cell.borrow_mut();
            let old = slot.take();
            *slot = Some(current.clone());
            old
        });
        Self { previous, current }
    }

    /// Drain the buffer captured during this guard's lifetime. Caller
    /// flushes after COMMIT succeeds; on rollback the buffer is just
    /// dropped along with the guard.
    pub(crate) fn take(&self) -> Vec<PendingSchedule> {
        std::mem::take(&mut *self.current.borrow_mut())
    }
}

impl Drop for ScheduleBufferGuard {
    fn drop(&mut self) {
        MUTATION_SCHEDULE_BUFFER.with(|cell| {
            *cell.borrow_mut() = self.previous.take();
        });
    }
}

// ---------------------------------------------------------------------------
// In-flight mutation depth marker (deadlock guard)
// ---------------------------------------------------------------------------

thread_local! {
    /// Counter of mutation-tx frames currently on the stack for this
    /// thread. Both backends acquire a single connection mutex per
    /// mutation (SQLite's write_conn, PG's `LivePostgresAdapter`).
    /// `std::sync::Mutex` is NOT re-entrant — a TS handler that calls
    /// `runMutation` from inside another mutation would block forever
    /// trying to re-acquire the connection lock it already holds.
    /// The nested-call hook checks this counter and rejects the call
    /// with `NESTED_MUTATION` instead of hanging.
    ///
    /// Counter (not bool) so future savepoint-based nesting could
    /// switch to a tx-reuse path without changing call sites.
    static MUTATION_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// RAII marker — incremented on entry to a mutation handler, decremented
/// on exit (including unwind). Used by the nested-call hook to detect
/// recursive mutations and reject them with a clear error rather than
/// deadlocking on the non-reentrant connection mutex.
pub(crate) struct MutationDepthGuard;

impl MutationDepthGuard {
    pub(crate) fn enter() -> Self {
        MUTATION_DEPTH.with(|d| d.set(d.get().saturating_add(1)));
        Self
    }
}

impl Drop for MutationDepthGuard {
    fn drop(&mut self) {
        MUTATION_DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
    }
}

/// True iff this thread is currently inside a mutation handler's tx.
pub(crate) fn in_mutation_tx() -> bool {
    MUTATION_DEPTH.with(|d| d.get() > 0)
}

// ---------------------------------------------------------------------------
// PG /api/transact CRDT-aware impl
// ---------------------------------------------------------------------------

impl Runtime {
    /// PG `/api/transact` implementation that runs each op through
    /// the typed `tx_*` helpers (so FTS shadow rows + the new id-
    /// reject + per-row locking all apply) AND adds CRDT projection
    /// + sidecar maintenance for `crdt: true` entities. Without this,
    /// batched admin writes desync from the CRDT layer.
    pub(crate) fn pg_transact_with_crdt(
        &self,
        pg: &crate::PgBackend,
        ops: &[serde_json::Value],
    ) -> Result<(bool, Vec<serde_json::Value>), DataError> {
        use pylon_storage::pg_tx_store::{tx_delete, tx_insert, tx_update};

        // Pre-validate every op shape — a malformed payload should
        // never open a tx and immediately roll back.
        enum Op<'a> {
            Insert {
                entity: &'a str,
                data: &'a serde_json::Value,
            },
            Update {
                entity: &'a str,
                id: &'a str,
                data: &'a serde_json::Value,
            },
            Delete {
                entity: &'a str,
                id: &'a str,
            },
        }
        let mut typed: Vec<Op<'_>> = Vec::with_capacity(ops.len());
        for op in ops {
            let op_type = op.get("op").and_then(|v| v.as_str()).unwrap_or("");
            let entity = op
                .get("entity")
                .and_then(|v| v.as_str())
                .ok_or_else(|| DataError {
                    code: "TX_INVALID_OP".into(),
                    message: "Each transact op must have an \"entity\" field".into(),
                })?;
            match op_type {
                "insert" => {
                    let data = op.get("data").ok_or_else(|| DataError {
                        code: "TX_INVALID_OP".into(),
                        message: "insert op requires \"data\"".into(),
                    })?;
                    typed.push(Op::Insert { entity, data });
                }
                "update" => {
                    let id = op
                        .get("id")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| DataError {
                            code: "TX_INVALID_OP".into(),
                            message: "update op requires \"id\"".into(),
                        })?;
                    let data = op.get("data").ok_or_else(|| DataError {
                        code: "TX_INVALID_OP".into(),
                        message: "update op requires \"data\"".into(),
                    })?;
                    typed.push(Op::Update { entity, id, data });
                }
                "delete" => {
                    let id = op
                        .get("id")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| DataError {
                            code: "TX_INVALID_OP".into(),
                            message: "delete op requires \"id\"".into(),
                        })?;
                    typed.push(Op::Delete { entity, id });
                }
                other => {
                    return Err(DataError {
                        code: "TX_INVALID_OP".into(),
                        message: format!("unknown op \"{other}\""),
                    });
                }
            }
        }

        // Track which CRDT rows we touched so we can refresh their
        // cache entries after commit (or evict on rollback).
        let mut crdt_touched: Vec<(String, String)> = Vec::new();

        let manifest = self.manifest.clone();
        let result = pg.store.with_transaction_raw(|tx| -> Result<Vec<serde_json::Value>, DataError> {
            let mut json_results: Vec<serde_json::Value> = Vec::with_capacity(typed.len());
            for op in &typed {
                let result = match op {
                    Op::Insert { entity, data } => {
                        let ent = manifest.entities.iter().find(|e| e.name == *entity);
                        let id = if ent.map(|e| e.crdt).unwrap_or(false) {
                            let crdt_fields = self.crdt_fields_for(ent.unwrap()).map_err(|e| {
                                DataError { code: e.code, message: e.message }
                            })?;
                            let id = crate::generate_id();
                            pg.crdt
                                .apply_patch(tx, entity, &id, &crdt_fields, data)
                                .map_err(|e| DataError {
                                    code: "CRDT_APPLY_FAILED".into(),
                                    message: format!("crdt write {entity}/{id}: {e}"),
                                })?;
                            let mut row = (*data).clone();
                            if let Some(obj) = row.as_object_mut() {
                                obj.insert("id".into(), serde_json::Value::String(id.clone()));
                            }
                            tx_insert(tx, &manifest, entity, &row)?;
                            crdt_touched.push((entity.to_string(), id.clone()));
                            id
                        } else {
                            tx_insert(tx, &manifest, entity, data)?
                        };
                        serde_json::json!({ "op": "insert", "id": id })
                    }
                    Op::Update { entity, id, data } => {
                        let ent = manifest.entities.iter().find(|e| e.name == *entity);
                        let updated = if ent.map(|e| e.crdt).unwrap_or(false) {
                            let crdt_fields = self.crdt_fields_for(ent.unwrap()).map_err(|e| {
                                DataError { code: e.code, message: e.message }
                            })?;
                            pg.crdt
                                .apply_patch(tx, entity, id, &crdt_fields, data)
                                .map_err(|e| DataError {
                                    code: "CRDT_APPLY_FAILED".into(),
                                    message: format!("crdt update {entity}/{id}: {e}"),
                                })?;
                            let updated = tx_update(tx, &manifest, entity, id, data)?;
                            if !updated {
                                return Err(DataError {
                                    code: "ENTITY_NOT_FOUND".into(),
                                    message: format!(
                                        "Update on {entity}/{id} found no row — refusing to commit \
                                         a CRDT snapshot that would orphan."
                                    ),
                                });
                            }
                            crdt_touched.push((entity.to_string(), id.to_string()));
                            updated
                        } else {
                            tx_update(tx, &manifest, entity, id, data)?
                        };
                        serde_json::json!({ "op": "update", "id": id, "updated": updated })
                    }
                    Op::Delete { entity, id } => {
                        let ent = manifest.entities.iter().find(|e| e.name == *entity);
                        let deleted = if ent.map(|e| e.crdt).unwrap_or(false) {
                            tx.execute(
                                "DELETE FROM _pylon_crdt_snapshots WHERE entity = $1 AND row_id = $2",
                                &[entity, id],
                            )
                            .map_err(|e| DataError {
                                code: "CRDT_SIDECAR_DELETE_FAILED".into(),
                                message: format!(
                                    "delete pg crdt snapshot {entity}/{id}: {e}"
                                ),
                            })?;
                            let deleted = tx_delete(tx, &manifest, entity, id)?;
                            crdt_touched.push((entity.to_string(), id.to_string()));
                            deleted
                        } else {
                            tx_delete(tx, &manifest, entity, id)?
                        };
                        serde_json::json!({ "op": "delete", "id": id, "deleted": deleted })
                    }
                };
                json_results.push(result);
            }
            // Refresh cache for CRDT rows we touched.
            for (entity, id) in &crdt_touched {
                pg.crdt.cache_after_commit(tx, entity, id);
            }
            Ok(json_results)
        });

        match result {
            Ok(json_results) => Ok((true, json_results)),
            Err(e) => {
                for (entity, id) in &crdt_touched {
                    pg.crdt.evict(entity, id);
                }
                Err(e)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// DataStore → Runtime bridge
// ---------------------------------------------------------------------------

impl DataStore for Runtime {
    fn manifest(&self) -> &pylon_kernel::AppManifest {
        Runtime::manifest(self)
    }

    fn insert(&self, entity: &str, data: &serde_json::Value) -> Result<String, DataError> {
        Runtime::insert(self, entity, data).map_err(into_data_error)
    }

    fn get_by_id(&self, entity: &str, id: &str) -> Result<Option<serde_json::Value>, DataError> {
        Runtime::get_by_id(self, entity, id).map_err(into_data_error)
    }

    fn list(&self, entity: &str) -> Result<Vec<serde_json::Value>, DataError> {
        Runtime::list(self, entity).map_err(into_data_error)
    }

    fn list_after(
        &self,
        entity: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>, DataError> {
        Runtime::list_after(self, entity, after, limit).map_err(into_data_error)
    }

    fn update(&self, entity: &str, id: &str, data: &serde_json::Value) -> Result<bool, DataError> {
        Runtime::update(self, entity, id, data).map_err(into_data_error)
    }

    fn delete(&self, entity: &str, id: &str) -> Result<bool, DataError> {
        Runtime::delete(self, entity, id).map_err(into_data_error)
    }

    fn lookup(
        &self,
        entity: &str,
        field: &str,
        value: &str,
    ) -> Result<Option<serde_json::Value>, DataError> {
        Runtime::lookup(self, entity, field, value).map_err(into_data_error)
    }

    fn link(
        &self,
        entity: &str,
        id: &str,
        relation: &str,
        target_id: &str,
    ) -> Result<bool, DataError> {
        Runtime::link(self, entity, id, relation, target_id).map_err(into_data_error)
    }

    fn unlink(&self, entity: &str, id: &str, relation: &str) -> Result<bool, DataError> {
        Runtime::unlink(self, entity, id, relation).map_err(into_data_error)
    }

    fn query_filtered(
        &self,
        entity: &str,
        filter: &serde_json::Value,
    ) -> Result<Vec<serde_json::Value>, DataError> {
        Runtime::query_filtered(self, entity, filter).map_err(into_data_error)
    }

    fn query_graph(&self, query: &serde_json::Value) -> Result<serde_json::Value, DataError> {
        Runtime::query_graph(self, query).map_err(into_data_error)
    }

    fn aggregate(
        &self,
        entity: &str,
        spec: &serde_json::Value,
    ) -> Result<serde_json::Value, DataError> {
        Runtime::aggregate(self, entity, spec).map_err(into_data_error)
    }

    fn transact(
        &self,
        ops: &[serde_json::Value],
    ) -> Result<(bool, Vec<serde_json::Value>), DataError> {
        // Postgres mode: delegate to the runtime-layer wrapper that
        // adds CRDT projection + sidecar maintenance for crdt:true
        // entities. The storage layer's transact (PostgresDataStore::
        // transact) only knows about FTS — codex flagged that
        // /api/transact would silently desync CRDT state.
        if let Some(pg) = self.pg_backend() {
            return self.pg_transact_with_crdt(pg, ops);
        }
        let conn = self.lock_conn_pub().map_err(into_data_error)?;
        let _ = conn.execute("BEGIN", []);
        let mut results: Vec<serde_json::Value> = Vec::new();
        let mut rollback = false;

        for op in ops {
            let op_type = op.get("op").and_then(|v| v.as_str()).unwrap_or("");
            let entity = op.get("entity").and_then(|v| v.as_str()).unwrap_or("");

            match op_type {
                "insert" => {
                    let data = op.get("data").cloned().unwrap_or(serde_json::json!({}));
                    match self.insert_with_conn(&conn, entity, &data) {
                        Ok(id) => {
                            results.push(serde_json::json!({"op": "insert", "id": id}));
                        }
                        Err(e) => {
                            results.push(serde_json::json!({"op": "insert", "error": e.message}));
                            rollback = true;
                            break;
                        }
                    }
                }
                "update" => {
                    let id = op.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    let data = op.get("data").cloned().unwrap_or(serde_json::json!({}));
                    match self.update_with_conn(&conn, entity, id, &data) {
                        Ok(_) => {
                            results.push(serde_json::json!({"op": "update", "id": id}));
                        }
                        Err(e) => {
                            results.push(serde_json::json!({"op": "update", "error": e.message}));
                            rollback = true;
                            break;
                        }
                    }
                }
                "delete" => {
                    let id = op.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    match self.delete_with_conn(&conn, entity, id) {
                        Ok(_) => {
                            results.push(serde_json::json!({"op": "delete", "id": id}));
                        }
                        Err(e) => {
                            results.push(serde_json::json!({"op": "delete", "error": e.message}));
                            rollback = true;
                            break;
                        }
                    }
                }
                _ => {
                    results.push(serde_json::json!({"op": op_type, "error": "unknown operation"}));
                }
            }
        }

        if rollback {
            let _ = conn.execute("ROLLBACK", []);
        } else {
            let _ = conn.execute("COMMIT", []);
        }

        Ok((!rollback, results))
    }

    /// Bridge the typed `SearchQuery` / `SearchResult` shapes to the
    /// trait's JSON-in / JSON-out contract. The router passes a JSON
    /// body; we deserialize, look up the entity's `SearchConfig`, run
    /// the planner, and re-serialize. Serialization round-tripping
    /// lets this method live on the DataStore trait without forcing
    /// pylon-http to depend on pylon-storage.
    fn search(
        &self,
        entity: &str,
        query: &serde_json::Value,
    ) -> Result<serde_json::Value, DataError> {
        let ent = self
            .manifest()
            .entities
            .iter()
            .find(|e| e.name == entity)
            .ok_or_else(|| DataError {
                code: "ENTITY_NOT_FOUND".into(),
                message: format!("Unknown entity: {entity}"),
            })?;
        let cfg = ent.search.as_ref().ok_or_else(|| DataError {
            code: "SEARCH_NOT_CONFIGURED".into(),
            message: format!("Entity {entity} has no `search:` config"),
        })?;
        let parsed: pylon_storage::search::SearchQuery = serde_json::from_value(query.clone())
            .map_err(|e| DataError {
                code: "INVALID_QUERY".into(),
                message: format!("search query body: {e}"),
            })?;

        // Postgres: dispatch to the PG-native FTS path (`tsvector` +
        // GIN index, maintained transactionally alongside CRUD by
        // PgTxStore). Same `SearchResult` shape as the SQLite path.
        if self.is_postgres() {
            let pg = self.pg_data_store().ok_or_else(|| DataError {
                code: "PG_DATASTORE_MISSING".into(),
                message: "is_postgres=true but pg_data_store() returned None".into(),
            })?;
            let result = pg.run_search(entity, cfg, &parsed).map_err(|e| DataError {
                code: e.code,
                message: e.message,
            })?;
            return serde_json::to_value(&result).map_err(|e| DataError {
                code: "SEARCH_SERIALIZE_FAILED".into(),
                message: e.to_string(),
            });
        }

        let conn = self.lock_conn_pub().map_err(into_data_error)?;
        let result =
            pylon_storage::search_query::run_search(&conn, entity, cfg, &parsed).map_err(|e| {
                DataError {
                    code: e.code,
                    message: e.message,
                }
            })?;
        serde_json::to_value(&result).map_err(|e| DataError {
            code: "SEARCH_SERIALIZE_FAILED".into(),
            message: e.to_string(),
        })
    }

    /// Return the binary CRDT snapshot for a row. `Ok(None)` for any
    /// entity with `crdt: false` (the LWW opt-out) — the router uses
    /// that to decide whether to ship a binary update over WebSocket
    /// after the write.
    fn crdt_snapshot(&self, entity: &str, row_id: &str) -> Result<Option<Vec<u8>>, DataError> {
        // Postgres: read from the PG `_pylon_crdt_snapshots` sidecar
        // via the same `PgLoroStore` that maintenance writes go
        // through. Same Ok(None) early-exit for non-CRDT entities so
        // the router skips the binary broadcast.
        if self.is_postgres() {
            let ent = self
                .manifest()
                .entities
                .iter()
                .find(|e| e.name == entity)
                .ok_or_else(|| DataError {
                    code: "ENTITY_NOT_FOUND".into(),
                    message: format!("Unknown entity: {entity}"),
                })?;
            if !ent.crdt {
                return Ok(None);
            }
            let pg_backend = match self.pg_backend() {
                Some(pg) => pg,
                None => return Ok(None),
            };
            // Single-read; with_client is fine here. PgLoroStore's
            // hydrate-on-miss + per-row Mutex ensures consistent
            // bytes even under concurrent applies on other threads.
            let snap = pg_backend.store.with_client(|client| -> Result<Vec<u8>, DataError> {
                pg_backend
                    .crdt
                    .snapshot(client, entity, row_id)
                    .map_err(|e| DataError {
                        code: "CRDT_SNAPSHOT_FAILED".into(),
                        message: format!("snapshot {entity}/{row_id}: {e}"),
                    })
            })?;
            return Ok(Some(snap));
        }
        let ent = self
            .manifest()
            .entities
            .iter()
            .find(|e| e.name == entity)
            .ok_or_else(|| DataError {
                code: "ENTITY_NOT_FOUND".into(),
                message: format!("Unknown entity: {entity}"),
            })?;
        if !ent.crdt {
            return Ok(None);
        }
        let conn = self.lock_conn_pub().map_err(into_data_error)?;
        let snap = self
            .crdt_store()
            .snapshot(&conn, entity, row_id)
            .map_err(|e| DataError {
                code: "CRDT_SNAPSHOT_FAILED".into(),
                message: format!("snapshot {entity}/{row_id}: {e}"),
            })?;
        Ok(Some(snap))
    }

    /// Client-pushed Loro update. Imports into the row's LoroDoc,
    /// re-projects the doc state into the materialized SQLite columns
    /// (so subsequent reads see the merged content), and returns the
    /// fresh full-row snapshot for the router to broadcast to other
    /// clients.
    ///
    /// Wrapped in a single SQLite transaction — same crash-safety
    /// shape as `Runtime::insert/update`. Either the LoroStore +
    /// SQLite columns both update or neither does.
    fn crdt_apply_update(
        &self,
        entity: &str,
        row_id: &str,
        update: &[u8],
    ) -> Result<Vec<u8>, DataError> {
        // Postgres: import the binary update into the row's PG-side
        // LoroDoc, persist the new snapshot in `_pylon_crdt_snapshots`,
        // re-project to the materialized PG row's columns, and return
        // the fresh full snapshot for the router to broadcast. Same
        // shape as the SQLite path below.
        if self.is_postgres() {
            let ent = self
                .manifest()
                .entities
                .iter()
                .find(|e| e.name == entity)
                .ok_or_else(|| DataError {
                    code: "ENTITY_NOT_FOUND".into(),
                    message: format!("Unknown entity: {entity}"),
                })?
                .clone();
            if !ent.crdt {
                return Err(DataError {
                    code: "NOT_SUPPORTED".into(),
                    message: format!(
                        "CRDT update sent for entity \"{entity}\" which has crdt: false"
                    ),
                });
            }
            let pg_backend = self.pg_backend().ok_or_else(|| DataError {
                code: "PG_BACKEND_MISSING".into(),
                message: "is_postgres=true but pg_backend() returned None".into(),
            })?;
            let crdt_fields = self.crdt_fields_for(&ent).map_err(into_data_error)?;

            // One transaction: apply the peer's update into the
            // LoroDoc + persist the new snapshot + reproject into the
            // materialized PG row + read back the fresh snapshot for
            // broadcast. Pre-fix this was three separate autocommits
            // — a failure between them desynced the layers, and the
            // broadcast snapshot might not reflect what actually
            // landed on disk.
            let result = pg_backend.store.with_transaction_raw(|tx| -> Result<Vec<u8>, DataError> {
                let projected = pg_backend
                    .crdt
                    .apply_remote_update(tx, entity, row_id, &crdt_fields, update)
                    .map_err(|e| {
                        // Distinguish decode errors (malformed client
                        // bytes — caller's fault, 400) from apply
                        // errors (schema mismatch, also caller's
                        // fault but a different shape). The CRDT
                        // route maps CRDT_DECODE_FAILED → 400, so
                        // unmapped errors land as 500 — codex
                        // flagged the asymmetry vs the SQLite path.
                        let code = match &e {
                            crate::loro_store::LoroStoreError::Decode(_) => "CRDT_DECODE_FAILED",
                            _ => "CRDT_APPLY_FAILED",
                        };
                        DataError {
                            code: code.into(),
                            message: format!("crdt apply update {entity}/{row_id}: {e}"),
                        }
                    })?;
                let updated = pylon_storage::pg_tx_store::tx_update(
                    tx,
                    self.manifest(),
                    entity,
                    row_id,
                    &projected,
                )?;
                if !updated {
                    // Same orphan guard as Runtime::update — refuse
                    // to commit a snapshot for a row that doesn't
                    // exist. Peer pushed an update for a row this
                    // replica's never seen.
                    return Err(DataError {
                        code: "ENTITY_NOT_FOUND".into(),
                        message: format!(
                            "Peer-pushed CRDT update targets {entity}/{row_id} which has \
                             no materialized row — refusing to commit an orphan snapshot."
                        ),
                    });
                }
                // Read the snapshot back from the tx, bypassing the
                // cache — a prior `crdt_snapshot()` call could have
                // populated the cache with bytes that predate this
                // peer update, and broadcasting them would silently
                // omit the just-applied change. Codex flagged this.
                let snap = crate::pg_loro_store::PgLoroStore::read_snapshot_via_conn(tx, entity, row_id)
                    .map_err(|e| DataError {
                        code: "CRDT_SNAPSHOT_FAILED".into(),
                        message: format!(
                            "post-update snapshot {entity}/{row_id}: {e}"
                        ),
                    })?;
                // Refresh the cache so the next reader on this
                // process skips the round-trip.
                pg_backend.crdt.cache_after_commit(tx, entity, row_id);
                Ok(snap)
            });
            if result.is_err() {
                // Same cache-coherency hygiene as Runtime::insert /
                // update — the in-memory doc absorbed the peer's
                // update before the tx rolled back, so evict to
                // force re-hydration from the persisted snapshot.
                pg_backend.crdt.evict(entity, row_id);
            }
            return result;
        }
        // Find the entity so we can build the projection field list +
        // confirm CRDT mode is on. Cheap manifest scan; counts are tiny.
        let ent = self
            .manifest()
            .entities
            .iter()
            .find(|e| e.name == entity)
            .ok_or_else(|| DataError {
                code: "ENTITY_NOT_FOUND".into(),
                message: format!("Unknown entity: {entity}"),
            })?
            .clone();
        if !ent.crdt {
            return Err(DataError {
                code: "NOT_SUPPORTED".into(),
                message: format!("Entity {entity} has crdt: false; client push requires CRDT mode"),
            });
        }
        let crdt_fields = self.crdt_fields_for(&ent).map_err(into_data_error)?;

        let conn = self.lock_conn_pub().map_err(into_data_error)?;
        crate::with_write_tx(&conn, || -> Result<Vec<u8>, crate::RuntimeError> {
            // Apply the update to the LoroDoc + persist the new snapshot
            // to the sidecar. Returns the projected JSON shape for the
            // post-merge state.
            let projected = self
                .crdt_store()
                .apply_remote_update(&conn, entity, row_id, &crdt_fields, update)
                .map_err(|e| crate::RuntimeError {
                    code: "CRDT_APPLY_FAILED".into(),
                    message: format!("apply_remote_update {entity}/{row_id}: {e}"),
                })?;

            // Re-project into the materialized SQLite row so SELECT
            // queries see the merged content. Build SET clauses from
            // the projection — every CRDT-managed field gets rewritten.
            let projection = projected.as_object().ok_or_else(|| crate::RuntimeError {
                code: "CRDT_PROJECTION_INVALID".into(),
                message: "projected row was not a JSON object".into(),
            })?;

            let mut set_clauses = Vec::with_capacity(projection.len());
            let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
            let mut idx = 1;
            for (key, val) in projection {
                if key == "id" {
                    continue;
                }
                set_clauses.push(format!("{} = ?{idx}", crate::quote_ident(key.as_str())));
                values.push(crate::json_to_sql(val));
                idx += 1;
            }
            if set_clauses.is_empty() {
                // No projected fields — happens when the doc has no
                // top-level keys yet (fresh row from a peer subscribing
                // before any writes). Skip the UPDATE; row may not exist
                // in SQLite. Subsequent inserts will materialize it.
            } else {
                values.push(Box::new(row_id.to_string()));
                let sql = format!(
                    "UPDATE {} SET {} WHERE \"id\" = ?{idx}",
                    crate::quote_ident(entity),
                    set_clauses.join(", ")
                );
                let params: Vec<&dyn rusqlite::types::ToSql> =
                    values.iter().map(|v| v.as_ref()).collect();
                conn.execute(&sql, params.as_slice())
                    .map_err(|e| crate::RuntimeError {
                        code: "UPDATE_FAILED".into(),
                        message: format!("post-merge UPDATE {entity}/{row_id}: {e}"),
                    })?;
            }

            // Return the new snapshot for the router to broadcast.
            let snap = self
                .crdt_store()
                .snapshot(&conn, entity, row_id)
                .map_err(|e| crate::RuntimeError {
                    code: "CRDT_SNAPSHOT_FAILED".into(),
                    message: format!("post-merge snapshot {entity}/{row_id}: {e}"),
                })?;
            Ok(snap)
        })
        .map_err(into_data_error)
    }
}

fn into_data_error(e: crate::RuntimeError) -> DataError {
    DataError {
        code: e.code,
        message: e.message,
    }
}

// ---------------------------------------------------------------------------
// ChangeNotifier for WsHub + SseHub
// ---------------------------------------------------------------------------

use crate::sse::SseHub;
use crate::ws::WsHub;
use std::sync::Arc;

/// Bridges WebSocket + SSE hubs to the router's [`ChangeNotifier`] trait.
pub struct WsSseNotifier {
    pub ws: Arc<WsHub>,
    pub sse: Arc<SseHub>,
}

impl pylon_router::ChangeNotifier for WsSseNotifier {
    fn notify(&self, event: &pylon_sync::ChangeEvent) {
        self.ws.broadcast(event);
        self.sse.broadcast(event);
    }

    fn notify_presence(&self, json: &str) {
        self.ws.broadcast_presence(json);
        self.sse.broadcast_message(json);
    }

    /// Encode a CRDT broadcast frame (1-byte type + length-prefixed
    /// entity + length-prefixed row_id + Loro snapshot bytes) and ship
    /// it to clients SUBSCRIBED to this row. SSE is text-only so it
    /// gets skipped — clients on the SSE transport stay on the JSON
    /// change-event path until a future SSE-friendly encoding (base64
    /// or hex-encoded chunks) lands.
    ///
    /// Filtering by subscription instead of broadcasting to every WS
    /// client matters once more than a handful of rows are in flight:
    /// a 50-channel app with 100 connected users would otherwise fan
    /// 100x for every keystroke in a single channel. Now each binary
    /// frame goes only to the (typically small) set of tabs that asked
    /// to mirror that specific row.
    ///
    /// If no clients are subscribed (empty list) the frame is dropped
    /// silently — the JSON change event from `notify` already told
    /// every connected client a write happened, so non-subscribed
    /// clients can re-fetch via the regular query path if they care.
    ///
    /// Authz: the policy check happens at SUBSCRIBE TIME (in
    /// `start_ws_server`'s SnapshotFetcher closure) — clients on the
    /// subscriber list have already passed `check_entity_read` for
    /// the row at that moment. We don't re-check on every broadcast
    /// because the broadcast hot path runs from the write thread
    /// without per-client auth context. A consequence: if a client is
    /// already subscribed and their permissions change mid-session
    /// (e.g. they're removed from a private channel), they'll keep
    /// receiving CRDT frames for that row until they disconnect.
    /// Future work: index subscribers by auth context so the broadcast
    /// can re-check, or invalidate subscriptions on policy changes.
    ///
    /// Frame-encode failure (entity / row_id over the 16-bit length
    /// header) gets logged and dropped — the row's regular JSON change
    /// event already shipped via `notify`, so clients still see the
    /// write happened, they just don't get the binary CRDT delta.
    fn notify_crdt(&self, entity: &str, row_id: &str, snapshot: &[u8]) {
        let subscribers = self.ws.subscriptions().subscribers(entity, row_id);
        if subscribers.is_empty() {
            return;
        }
        match pylon_router::encode_crdt_frame(
            pylon_router::CRDT_FRAME_SNAPSHOT,
            entity,
            row_id,
            snapshot,
        ) {
            Ok(frame) => self.ws.broadcast_binary_to(&subscribers, frame),
            Err(e) => {
                tracing::warn!("[crdt] dropping binary frame for {entity}/{row_id}: {e}");
            }
        }
    }
}

/// Serialize a value to JSON, falling back to `{}` on failure.
fn to_json<T: serde::Serialize>(val: T) -> serde_json::Value {
    serde_json::to_value(val).unwrap_or(serde_json::json!({}))
}

/// Serialize a value to JSON, falling back to `[]` on failure.
fn to_json_array<T: serde::Serialize>(val: T) -> serde_json::Value {
    serde_json::to_value(val).unwrap_or(serde_json::json!([]))
}

// ---------------------------------------------------------------------------
// Adapter: RoomManager → RoomOps
// ---------------------------------------------------------------------------

use crate::rooms::RoomManager;

impl pylon_router::RoomOps for RoomManager {
    fn join(
        &self,
        room: &str,
        user_id: &str,
        data: Option<serde_json::Value>,
    ) -> Result<(serde_json::Value, serde_json::Value), DataError> {
        RoomManager::join(self, room, user_id, data)
            .map(|(snapshot, join_event)| (to_json(&snapshot), to_json(&join_event)))
            .map_err(|e| DataError {
                code: e.code,
                message: e.message,
            })
    }

    fn leave(&self, room: &str, user_id: &str) -> Option<serde_json::Value> {
        RoomManager::leave(self, room, user_id).map(|event| to_json(&event))
    }

    fn set_presence(
        &self,
        room: &str,
        user_id: &str,
        data: serde_json::Value,
    ) -> Option<serde_json::Value> {
        RoomManager::set_presence(self, room, user_id, data).map(|event| to_json(&event))
    }

    fn broadcast(
        &self,
        room: &str,
        sender: Option<&str>,
        topic: &str,
        data: serde_json::Value,
    ) -> Option<serde_json::Value> {
        RoomManager::broadcast(self, room, sender, topic, data).map(|event| to_json(&event))
    }

    fn list_rooms(&self) -> Vec<String> {
        RoomManager::list_rooms(self)
    }

    fn room_size(&self, name: &str) -> usize {
        RoomManager::room_size(self, name)
    }

    fn members(&self, name: &str) -> Vec<serde_json::Value> {
        RoomManager::members(self, name)
            .into_iter()
            .map(|p| to_json(p))
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Adapter: CachePlugin → CacheOps (newtype wrapper for orphan rule)
// ---------------------------------------------------------------------------

use pylon_plugin::builtin::cache::CachePlugin;

/// Adapter that routes router-level CRUD hook calls into the PluginRegistry.
///
/// The router holds a `&dyn PluginHookOps`; this adapter wraps the runtime's
/// `Arc<PluginRegistry>` so registered plugins (audit_log, validation,
/// webhooks, timestamps, slugify, versioning, search) run on every
/// POST/PATCH/DELETE under `/api/entities/*`. Without this wiring, plugins
/// only saw the `on_request` hook and never got a chance to observe or
/// reject data-plane writes — a quiet correctness hole noted in the
/// pentest review.
pub struct PluginHooksAdapter(pub Arc<pylon_plugin::PluginRegistry>);

impl pylon_router::PluginHookOps for PluginHooksAdapter {
    fn before_insert(
        &self,
        entity: &str,
        data: &mut serde_json::Value,
        auth: &pylon_auth::AuthContext,
    ) -> Result<(), (u16, String, String)> {
        self.0
            .run_before_insert(entity, data, auth)
            .map_err(|e| (e.status, e.code, e.message))
    }
    fn after_insert(
        &self,
        entity: &str,
        id: &str,
        data: &serde_json::Value,
        auth: &pylon_auth::AuthContext,
    ) {
        self.0.run_after_insert(entity, id, data, auth);
    }
    fn before_update(
        &self,
        entity: &str,
        id: &str,
        data: &mut serde_json::Value,
        auth: &pylon_auth::AuthContext,
    ) -> Result<(), (u16, String, String)> {
        self.0
            .run_before_update(entity, id, data, auth)
            .map_err(|e| (e.status, e.code, e.message))
    }
    fn after_update(
        &self,
        entity: &str,
        id: &str,
        data: &serde_json::Value,
        auth: &pylon_auth::AuthContext,
    ) {
        self.0.run_after_update(entity, id, data, auth);
    }
    fn before_delete(
        &self,
        entity: &str,
        id: &str,
        auth: &pylon_auth::AuthContext,
    ) -> Result<(), (u16, String, String)> {
        self.0
            .run_before_delete(entity, id, auth)
            .map_err(|e| (e.status, e.code, e.message))
    }
    fn after_delete(&self, entity: &str, id: &str, auth: &pylon_auth::AuthContext) {
        self.0.run_after_delete(entity, id, auth);
    }
}

pub struct CacheAdapter(pub Arc<CachePlugin>);

impl pylon_router::CacheOps for CacheAdapter {
    fn handle_command(&self, body: &str) -> (u16, String) {
        crate::cache_handlers::handle_cache_command(&self.0, body)
    }

    fn handle_get(&self, key: &str) -> (u16, String) {
        crate::cache_handlers::handle_cache_get(&self.0, key)
    }

    fn handle_delete(&self, key: &str) -> (u16, String) {
        crate::cache_handlers::handle_cache_delete(&self.0, key)
    }
}

// ---------------------------------------------------------------------------
// Adapter: PubSubBroker → PubSubOps (newtype wrapper for orphan rule)
// ---------------------------------------------------------------------------

use crate::pubsub::PubSubBroker;

pub struct PubSubAdapter(pub Arc<PubSubBroker>);

impl pylon_router::PubSubOps for PubSubAdapter {
    fn handle_publish(&self, body: &str) -> (u16, String) {
        crate::cache_handlers::handle_pubsub_publish(&self.0, body)
    }

    fn handle_channels(&self) -> (u16, String) {
        crate::cache_handlers::handle_pubsub_channels(&self.0)
    }

    fn handle_history(&self, channel: &str, url: &str) -> (u16, String) {
        crate::cache_handlers::handle_pubsub_history(&self.0, channel, url)
    }
}

// ---------------------------------------------------------------------------
// Adapter: JobQueue → JobOps
// ---------------------------------------------------------------------------

use crate::jobs::{JobQueue, Priority};

impl pylon_router::JobOps for JobQueue {
    fn enqueue(
        &self,
        name: &str,
        payload: serde_json::Value,
        priority: &str,
        delay_secs: u64,
        max_retries: u32,
        queue: &str,
    ) -> String {
        let pri = Priority::from_str_loose(priority);
        JobQueue::enqueue_with_options(self, name, payload, pri, delay_secs, max_retries, queue)
    }

    fn stats(&self) -> serde_json::Value {
        to_json(JobQueue::stats(self))
    }

    fn dead_letters(&self) -> serde_json::Value {
        to_json_array(JobQueue::dead_letters(self))
    }

    fn retry_dead(&self, id: &str) -> bool {
        JobQueue::retry_dead(self, id)
    }

    fn list_jobs(
        &self,
        status: Option<&str>,
        queue: Option<&str>,
        limit: usize,
    ) -> serde_json::Value {
        to_json_array(JobQueue::list_jobs(self, status, queue, limit))
    }

    fn get_job(&self, id: &str) -> Option<serde_json::Value> {
        JobQueue::get_job(self, id).map(|j| to_json(j))
    }
}

// ---------------------------------------------------------------------------
// Adapter: Scheduler → SchedulerOps
// ---------------------------------------------------------------------------

use crate::scheduler::Scheduler;

impl pylon_router::SchedulerOps for Scheduler {
    fn list_tasks(&self) -> serde_json::Value {
        to_json_array(Scheduler::list_tasks(self))
    }

    fn trigger(&self, name: &str) -> bool {
        Scheduler::trigger(self, name)
    }
}

// ---------------------------------------------------------------------------
// Adapter: WorkflowEngine → WorkflowOps
// ---------------------------------------------------------------------------

use crate::workflows::WorkflowEngine;

impl pylon_router::WorkflowOps for WorkflowEngine {
    fn definitions(&self) -> serde_json::Value {
        to_json_array(WorkflowEngine::definitions(self))
    }

    fn start(&self, name: &str, input: serde_json::Value) -> Result<String, String> {
        WorkflowEngine::start(self, name, input)
    }

    fn list(&self, status_filter: Option<&str>) -> serde_json::Value {
        // Convert string filter to WorkflowStatus for the engine.
        let filter = status_filter.and_then(|s| match s {
            "pending" => Some(crate::workflows::WorkflowStatus::Pending),
            "running" => Some(crate::workflows::WorkflowStatus::Running),
            "sleeping" => Some(crate::workflows::WorkflowStatus::Sleeping),
            "waiting" => Some(crate::workflows::WorkflowStatus::WaitingForEvent),
            "completed" => Some(crate::workflows::WorkflowStatus::Completed),
            "failed" => Some(crate::workflows::WorkflowStatus::Failed),
            "cancelled" => Some(crate::workflows::WorkflowStatus::Cancelled),
            _ => None,
        });
        to_json_array(WorkflowEngine::list(self, filter.as_ref()))
    }

    fn get(&self, id: &str) -> Option<serde_json::Value> {
        WorkflowEngine::get(self, id).map(|inst| to_json(inst))
    }

    fn advance(&self, id: &str) -> Result<String, String> {
        WorkflowEngine::advance(self, id).map(|status| format!("{:?}", status))
    }

    fn send_event(&self, id: &str, event: &str, data: serde_json::Value) -> Result<(), String> {
        WorkflowEngine::send_event(self, id, event, data)
    }

    fn cancel(&self, id: &str) -> Result<(), String> {
        WorkflowEngine::cancel(self, id)
    }
}

// ---------------------------------------------------------------------------
// Adapter: FileStorage trait → FileOps
// ---------------------------------------------------------------------------

use pylon_storage::files::{FileStorage, LocalFileStorage, Stack0FileStorage};

/// Adapter that exposes a [`FileStorage`] backend through the router's [`FileOps`].
pub struct FileOpsAdapter {
    pub storage: Arc<dyn FileStorage>,
}

impl FileOpsAdapter {
    /// Create from environment variables.
    ///
    /// Selects backend via `PYLON_FILES_PROVIDER`:
    /// - `local` (default) — files saved under `PYLON_FILES_DIR` and served
    ///   via `PYLON_FILES_URL_PREFIX`.
    /// - `stack0` — uploads go to Stack0's CDN. Requires `PYLON_STACK0_API_KEY`.
    pub fn from_env() -> Self {
        let provider = std::env::var("PYLON_FILES_PROVIDER").unwrap_or_else(|_| "local".into());
        match provider.as_str() {
            "stack0" => match Stack0FileStorage::from_env() {
                Some(s) => Self {
                    storage: Arc::new(s),
                },
                None => {
                    tracing::warn!(
                        "PYLON_FILES_PROVIDER=stack0 but PYLON_STACK0_API_KEY is not set; falling back to local storage"
                    );
                    Self::local_from_env()
                }
            },
            _ => Self::local_from_env(),
        }
    }

    fn local_from_env() -> Self {
        let dir = std::env::var("PYLON_FILES_DIR").unwrap_or_else(|_| "uploads".into());
        let url_prefix =
            std::env::var("PYLON_FILES_URL_PREFIX").unwrap_or_else(|_| "/api/files".into());
        Self {
            storage: Arc::new(LocalFileStorage::new(&dir, &url_prefix)),
        }
    }
}

impl pylon_router::FileOps for FileOpsAdapter {
    fn upload(&self, _body: &str) -> (u16, String) {
        // The self-hosted server short-circuits /api/files/upload BEFORE the
        // request body is lossily coerced to a String, so binary uploads are
        // handled there. This fallback exists for non-self-hosted adapters
        // (e.g., Workers) and for defense in depth; it rejects string bodies
        // that wouldn't carry binary data correctly.
        (
            400,
            pylon_router::json_error(
                "UPLOAD_NEEDS_BINARY",
                "File uploads must use multipart/form-data or raw binary with X-Filename; this platform does not support string-body uploads",
            ),
        )
    }

    fn get_file(&self, id: &str) -> (u16, String) {
        match self.storage.get(id) {
            Ok(content) => (200, String::from_utf8_lossy(&content).into_owned()),
            Err(e) if e.code == "NOT_FOUND" => {
                (404, pylon_router::json_error("FILE_NOT_FOUND", &e.message))
            }
            Err(e) => (400, pylon_router::json_error(&e.code, &e.message)),
        }
    }
}

/// Backwards-compatible alias; old code refers to this name.
pub type LocalFileOps = FileOpsAdapter;

impl LocalFileOps {
    /// Default instance backed by the local `uploads/` directory.
    pub fn new_default() -> Self {
        Self::from_env()
    }
}

// ---------------------------------------------------------------------------
// Adapter: EmailTransport → EmailSender
// ---------------------------------------------------------------------------

use pylon_auth::email::{ConsoleTransport, EmailTransport, HttpEmailTransport};

/// Picks an email backend based on environment variables.
/// Falls back to `ConsoleTransport` (prints to stderr) when no provider is configured.
pub struct EmailAdapter {
    transport: Box<dyn EmailTransport>,
}

impl EmailAdapter {
    pub fn from_env() -> Self {
        if let Some(http) = HttpEmailTransport::from_env() {
            Self {
                transport: Box::new(http),
            }
        } else {
            Self {
                transport: Box::new(ConsoleTransport),
            }
        }
    }
}

impl pylon_router::EmailSender for EmailAdapter {
    fn send(&self, to: &str, subject: &str, body: &str) -> Result<(), String> {
        self.transport
            .send(to, subject, body)
            .map_err(|e| e.message)
    }
}

// ---------------------------------------------------------------------------
// Adapter: OpenAPI generator
// ---------------------------------------------------------------------------

pub struct RuntimeOpenApiGenerator<'a> {
    pub manifest: &'a pylon_kernel::AppManifest,
}

impl<'a> pylon_router::OpenApiGenerator for RuntimeOpenApiGenerator<'a> {
    fn generate(&self, base_url: &str) -> String {
        let spec = crate::openapi::generate_openapi(self.manifest, base_url);
        serde_json::to_string(&spec).unwrap_or_else(|_| "{}".into())
    }
}

// ---------------------------------------------------------------------------
// Adapter: DynShardRegistry → ShardOps
// ---------------------------------------------------------------------------

/// Wraps any `Arc<dyn DynShardRegistry>` so the router can dispatch shard
/// routes without knowing the concrete SimState type.
pub struct ShardOpsAdapter {
    pub registry: Arc<dyn pylon_realtime::DynShardRegistry>,
}

impl pylon_router::ShardOps for ShardOpsAdapter {
    fn get_shard(&self, id: &str) -> Option<Arc<dyn pylon_realtime::DynShard>> {
        self.registry.get(id)
    }

    fn list_shards(&self) -> Vec<String> {
        self.registry.ids()
    }

    fn shard_count(&self) -> usize {
        self.registry.len()
    }
}

#[cfg(test)]
mod find_runtime_tests {
    use super::*;

    #[test]
    fn env_override_takes_precedence() {
        let dir = std::env::temp_dir().join(format!("pylon_rt_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("custom_runtime.ts");
        std::fs::write(&path, "// test").unwrap();

        std::env::set_var("PYLON_FUNCTIONS_RUNTIME", path.to_str().unwrap());
        let found = find_functions_runtime();
        std::env::remove_var("PYLON_FUNCTIONS_RUNTIME");

        assert_eq!(found.as_deref(), path.to_str());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn returns_none_when_env_path_missing() {
        std::env::set_var(
            "PYLON_FUNCTIONS_RUNTIME",
            "/tmp/definitely-does-not-exist-42.ts",
        );
        // May still find something in CWD (dev path), so we only assert the env
        // path isn't what gets returned.
        let found = find_functions_runtime();
        std::env::remove_var("PYLON_FUNCTIONS_RUNTIME");
        assert_ne!(
            found.as_deref(),
            Some("/tmp/definitely-does-not-exist-42.ts")
        );
    }
}

// ---------------------------------------------------------------------------
// TxStore — DataStore backed by a held transaction connection
// ---------------------------------------------------------------------------

/// A `DataStore` that executes against a pre-held SQLite connection
/// for the duration of a single mutation handler.
///
/// # Safety contract
///
/// `rusqlite::Connection` is `Send` but not `Sync` (it uses `RefCell`s
/// internally for statement caching). The `DataStore` trait requires
/// `Send + Sync`, but `&'a Connection` is neither.
///
/// We hand-implement both via `unsafe impl` because:
///
/// 1. **Construction.** `TxStore::new` is only ever called by
///    `FnOpsImpl::call` for mutations, after acquiring the runtime's
///    write lock. The `&Connection` originates from a `MutexGuard`
///    that the constructing thread holds.
///
/// 2. **Lifetime.** The `'a` lifetime ties the `TxStore` to that guard.
///    The compiler enforces that the `TxStore` cannot outlive the held
///    lock; it must be dropped before the guard is.
///
/// 3. **Single-threaded use.** `FnRunner::call()` runs the handler
///    synchronously on the calling thread and never spawns threads
///    holding a reference to the `TxStore`. The `Send + Sync` bounds
///    on the `DataStore` trait are satisfied vacuously — no thread
///    other than the caller ever sees this `TxStore`.
///
/// 4. **No interior aliasing.** All `&Connection` calls go through
///    `Runtime::*_with_conn` methods which take `&Connection`, never
///    keeping the reference alive across an `await` point (this is
///    sync code, no awaits).
///
/// Future work: refactor `Runtime`'s `write_conn` to be
/// `Arc<Mutex<Connection>>` so TxStore can hold an `Arc<Mutex<...>>`,
/// eliminating the unsafe impl entirely.
pub struct TxStore<'a> {
    runtime: &'a Runtime,
    conn: &'a rusqlite::Connection,
    /// Pending change events to broadcast after the outer transaction
    /// commits. Buffered here rather than pushed to ChangeLog + notifier
    /// immediately so a rollback doesn't emit events for writes that
    /// didn't actually land.
    pending: std::cell::RefCell<Vec<pylon_sync::ChangeEvent>>,
}

impl<'a> TxStore<'a> {
    pub fn new(runtime: &'a Runtime, conn: &'a rusqlite::Connection) -> Self {
        Self {
            runtime,
            conn,
            pending: std::cell::RefCell::new(Vec::new()),
        }
    }

    /// Drain the pending-events buffer. Called after COMMIT succeeds;
    /// the caller is responsible for appending each event to the
    /// ChangeLog and broadcasting via the notifier. On rollback the
    /// caller just drops the buffer without calling this.
    pub fn take_pending(&self) -> Vec<pylon_sync::ChangeEvent> {
        std::mem::take(&mut *self.pending.borrow_mut())
    }

    fn record(
        &self,
        entity: &str,
        row_id: &str,
        kind: pylon_sync::ChangeKind,
        data: Option<&serde_json::Value>,
    ) {
        self.pending.borrow_mut().push(pylon_sync::ChangeEvent {
            seq: 0, // assigned by ChangeLog::append after commit
            entity: entity.to_string(),
            row_id: row_id.to_string(),
            kind,
            data: data.cloned(),
            timestamp: String::new(),
        });
    }
}

// SAFETY: see the contract on TxStore above.
unsafe impl<'a> Sync for TxStore<'a> {}
unsafe impl<'a> Send for TxStore<'a> {}

impl<'a> DataStore for TxStore<'a> {
    fn manifest(&self) -> &pylon_kernel::AppManifest {
        self.runtime.manifest()
    }

    fn insert(&self, entity: &str, data: &serde_json::Value) -> Result<String, DataError> {
        let id = self
            .runtime
            .insert_with_conn(self.conn, entity, data)
            .map_err(into_data_error)?;
        // Buffer the event. If the outer mutation rolls back, the buffer
        // is dropped instead of flushed, so sync subscribers never see a
        // row that doesn't exist.
        self.record(entity, &id, pylon_sync::ChangeKind::Insert, Some(data));
        Ok(id)
    }

    fn get_by_id(&self, entity: &str, id: &str) -> Result<Option<serde_json::Value>, DataError> {
        self.runtime
            .get_by_id_with_conn(self.conn, entity, id)
            .map_err(into_data_error)
    }

    fn list(&self, entity: &str) -> Result<Vec<serde_json::Value>, DataError> {
        self.runtime
            .list_with_conn(self.conn, entity)
            .map_err(into_data_error)
    }

    fn list_after(
        &self,
        entity: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>, DataError> {
        self.runtime
            .list_after_with_conn(self.conn, entity, after, limit)
            .map_err(into_data_error)
    }

    fn update(&self, entity: &str, id: &str, data: &serde_json::Value) -> Result<bool, DataError> {
        let updated = self
            .runtime
            .update_with_conn(self.conn, entity, id, data)
            .map_err(into_data_error)?;
        if updated {
            self.record(entity, id, pylon_sync::ChangeKind::Update, Some(data));
        }
        Ok(updated)
    }

    fn delete(&self, entity: &str, id: &str) -> Result<bool, DataError> {
        let deleted = self
            .runtime
            .delete_with_conn(self.conn, entity, id)
            .map_err(into_data_error)?;
        if deleted {
            self.record(entity, id, pylon_sync::ChangeKind::Delete, None);
        }
        Ok(deleted)
    }

    fn lookup(
        &self,
        entity: &str,
        field: &str,
        value: &str,
    ) -> Result<Option<serde_json::Value>, DataError> {
        self.runtime
            .lookup_with_conn(self.conn, entity, field, value)
            .map_err(into_data_error)
    }

    fn link(
        &self,
        entity: &str,
        id: &str,
        relation: &str,
        target_id: &str,
    ) -> Result<bool, DataError> {
        self.runtime
            .link_with_conn(self.conn, entity, id, relation, target_id)
            .map_err(into_data_error)
    }

    fn unlink(&self, entity: &str, id: &str, relation: &str) -> Result<bool, DataError> {
        self.runtime
            .unlink_with_conn(self.conn, entity, id, relation)
            .map_err(into_data_error)
    }

    fn query_filtered(
        &self,
        entity: &str,
        filter: &serde_json::Value,
    ) -> Result<Vec<serde_json::Value>, DataError> {
        self.runtime
            .query_filtered_with_conn(self.conn, entity, filter)
            .map_err(into_data_error)
    }

    fn query_graph(&self, query: &serde_json::Value) -> Result<serde_json::Value, DataError> {
        self.runtime
            .query_graph_with_conn(self.conn, query)
            .map_err(into_data_error)
    }

    fn aggregate(
        &self,
        entity: &str,
        spec: &serde_json::Value,
    ) -> Result<serde_json::Value, DataError> {
        // Aggregation inside a transaction uses the same runtime method.
        // The lookups do their own read-lock, which is fine since aggregate
        // is read-only.
        Runtime::aggregate(self.runtime, entity, spec).map_err(into_data_error)
    }

    fn transact(
        &self,
        _ops: &[serde_json::Value],
    ) -> Result<(bool, Vec<serde_json::Value>), DataError> {
        // Nested transactions aren't supported from within a mutation handler.
        // The mutation handler IS the transaction.
        Err(DataError {
            code: "NESTED_TRANSACTION".into(),
            message: "ctx.db.transact() is not allowed inside a mutation handler (the handler itself is transactional)".into(),
        })
    }

    fn search(
        &self,
        entity: &str,
        query: &serde_json::Value,
    ) -> Result<serde_json::Value, DataError> {
        // Search reads against the FTS shadow are read-only; route
        // through the runtime's main `search` impl which already
        // validates the entity + branches on backend. The held write
        // connection is fine for reads (SQLite serializes anyway).
        <Runtime as DataStore>::search(self.runtime, entity, query)
    }
}

// ---------------------------------------------------------------------------
// PG-transaction buffering wrapper
// ---------------------------------------------------------------------------

/// `DataStore` wrapper used by the Postgres mutation path. The Postgres
/// `PgTxStore` owns the transaction; this wrapper layers the same
/// "buffer change events, flush after COMMIT" guarantee that SQLite's
/// `TxStore` provides directly. The underlying `inner` ref lives only
/// for the duration of `PostgresDataStore::with_transaction`'s closure
/// — the lifetime tracks through.
struct PgBufferedTxStore<'a> {
    inner: &'a dyn DataStore,
    pending: std::sync::Mutex<Vec<pylon_sync::ChangeEvent>>,
}

impl<'a> PgBufferedTxStore<'a> {
    fn new(inner: &'a dyn DataStore) -> Self {
        Self {
            inner,
            pending: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn record(
        &self,
        entity: &str,
        row_id: &str,
        kind: pylon_sync::ChangeKind,
        data: Option<&serde_json::Value>,
    ) {
        if let Ok(mut p) = self.pending.lock() {
            p.push(pylon_sync::ChangeEvent {
                seq: 0,
                entity: entity.to_string(),
                row_id: row_id.to_string(),
                kind,
                data: data.cloned(),
                timestamp: String::new(),
            });
        }
    }

    fn take_pending(self) -> Vec<pylon_sync::ChangeEvent> {
        self.pending.into_inner().unwrap_or_default()
    }
}

impl<'a> DataStore for PgBufferedTxStore<'a> {
    fn manifest(&self) -> &pylon_kernel::AppManifest {
        self.inner.manifest()
    }

    fn insert(&self, entity: &str, data: &serde_json::Value) -> Result<String, DataError> {
        let id = self.inner.insert(entity, data)?;
        self.record(entity, &id, pylon_sync::ChangeKind::Insert, Some(data));
        Ok(id)
    }

    fn get_by_id(&self, entity: &str, id: &str) -> Result<Option<serde_json::Value>, DataError> {
        self.inner.get_by_id(entity, id)
    }

    fn list(&self, entity: &str) -> Result<Vec<serde_json::Value>, DataError> {
        self.inner.list(entity)
    }

    fn list_after(
        &self,
        entity: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>, DataError> {
        self.inner.list_after(entity, after, limit)
    }

    fn update(&self, entity: &str, id: &str, data: &serde_json::Value) -> Result<bool, DataError> {
        let updated = self.inner.update(entity, id, data)?;
        if updated {
            self.record(entity, id, pylon_sync::ChangeKind::Update, Some(data));
        }
        Ok(updated)
    }

    fn delete(&self, entity: &str, id: &str) -> Result<bool, DataError> {
        let deleted = self.inner.delete(entity, id)?;
        if deleted {
            self.record(entity, id, pylon_sync::ChangeKind::Delete, None);
        }
        Ok(deleted)
    }

    fn lookup(
        &self,
        entity: &str,
        field: &str,
        value: &str,
    ) -> Result<Option<serde_json::Value>, DataError> {
        self.inner.lookup(entity, field, value)
    }

    fn link(
        &self,
        entity: &str,
        id: &str,
        relation: &str,
        target_id: &str,
    ) -> Result<bool, DataError> {
        let linked = self.inner.link(entity, id, relation, target_id)?;
        if linked {
            // `link` is implemented as a typed `update` under the hood —
            // record an Update so subscribers see the FK-set the same way
            // they'd see any other column change.
            let data = serde_json::json!({ relation: target_id });
            self.record(entity, id, pylon_sync::ChangeKind::Update, Some(&data));
        }
        Ok(linked)
    }

    fn unlink(&self, entity: &str, id: &str, relation: &str) -> Result<bool, DataError> {
        let unlinked = self.inner.unlink(entity, id, relation)?;
        if unlinked {
            let data = serde_json::json!({ relation: serde_json::Value::Null });
            self.record(entity, id, pylon_sync::ChangeKind::Update, Some(&data));
        }
        Ok(unlinked)
    }

    fn query_filtered(
        &self,
        entity: &str,
        filter: &serde_json::Value,
    ) -> Result<Vec<serde_json::Value>, DataError> {
        self.inner.query_filtered(entity, filter)
    }

    fn query_graph(&self, query: &serde_json::Value) -> Result<serde_json::Value, DataError> {
        self.inner.query_graph(query)
    }

    fn aggregate(
        &self,
        entity: &str,
        spec: &serde_json::Value,
    ) -> Result<serde_json::Value, DataError> {
        self.inner.aggregate(entity, spec)
    }

    fn transact(
        &self,
        ops: &[serde_json::Value],
    ) -> Result<(bool, Vec<serde_json::Value>), DataError> {
        // Forward to the inner PgTxStore, which already returns
        // NESTED_TRANSACTION. Keeping the forward (instead of erroring
        // here) means the wrapper stays a faithful pass-through and any
        // future change to the inner policy applies uniformly.
        self.inner.transact(ops)
    }

    fn search(
        &self,
        entity: &str,
        query: &serde_json::Value,
    ) -> Result<serde_json::Value, DataError> {
        // Forward to inner — `PgTxStore::search` (default impl)
        // currently returns NOT_SUPPORTED for in-tx search, which is
        // the right answer: PG search uses a separate connection
        // pool today and would deadlock on the in-handler tx if we
        // tried to fan out from the same client.
        self.inner.search(entity, query)
    }
}

// ---------------------------------------------------------------------------
// Adapter: FnRunner → FnOps
// ---------------------------------------------------------------------------

use pylon_functions::protocol::{AuthInfo as FnAuth, FnType};
use pylon_functions::registry::{FnDef, FnRegistry};
use pylon_functions::runner::{FnCallError, FnRunner};
use pylon_functions::trace::FnTrace;

/// Adapter that implements [`FnOps`] by delegating to a [`FnRunner`].
///
/// Holds an `Arc<Runtime>` so function handlers get a [`DataStore`] to
/// operate against.
pub struct FnOpsImpl {
    pub runner: Arc<FnRunner>,
    pub registry: Arc<FnRegistry>,
    pub runtime: Arc<Runtime>,
    /// Per-function rate limiter, keyed on `"<fn_name>::<identity>"`.
    /// Limits are uniform; per-fn overrides can be added later via FnDef
    /// metadata once the TS define API surfaces them.
    pub fn_rate_limiter: Arc<crate::rate_limit::RateLimiter>,
    /// Sync change log for broadcasting `ctx.db.insert/update/delete` ops
    /// that happen inside a function handler. Without this, mutations via
    /// functions silently bypass sync — WS subscribers see nothing until
    /// they manually refetch. Flushed post-COMMIT so rollbacks don't emit
    /// phantom events.
    pub change_log: Arc<pylon_sync::ChangeLog>,
    /// Where to broadcast change events after a function mutation commits.
    pub notifier: Arc<dyn pylon_router::ChangeNotifier>,
    /// Job queue for flushing schedules buffered during a mutation
    /// handler. The schedule hook itself owns its own clone for the
    /// non-mutation enqueue path; this clone is what the
    /// post-COMMIT flush uses.
    pub job_queue: Arc<crate::jobs::JobQueue>,
}

impl FnOpsImpl {
    /// Drain a per-mutation schedule buffer and enqueue each entry on
    /// the job queue. Called only after COMMIT succeeds — a rolled-back
    /// handler's buffer is dropped without flushing.
    fn flush_pending_schedules(&self, pending: Vec<PendingSchedule>) {
        for sched in pending {
            let delay_secs = match (sched.delay_ms, sched.run_at) {
                (Some(ms), _) => ms / 1000,
                (None, Some(ts)) => {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64;
                    if ts > now { (ts - now) / 1000 } else { 0 }
                }
                _ => 0,
            };
            if let Err(e) = self.job_queue.try_enqueue_with_options(
                &sched.fn_name,
                sched.args,
                crate::jobs::Priority::Normal,
                delay_secs,
                3,
                "functions",
            ) {
                // Schedule was already acked OK to the TS handler — the
                // mutation has committed. Best we can do now is log
                // loudly so an operator notices the dropped enqueue.
                tracing::warn!(
                    "[functions] post-COMMIT enqueue failed for \"{}\": {e}",
                    sched.fn_name
                );
            }
        }
    }
}

impl pylon_router::FnOps for FnOpsImpl {
    fn get_fn(&self, name: &str) -> Option<FnDef> {
        self.registry.get(name)
    }

    fn list_fns(&self) -> Vec<FnDef> {
        self.registry.list()
    }

    fn call(
        &self,
        fn_name: &str,
        args: serde_json::Value,
        auth: FnAuth,
        on_stream: Option<Box<dyn FnMut(&str) + Send>>,
        request: Option<pylon_functions::protocol::RequestInfo>,
    ) -> Result<(serde_json::Value, FnTrace), FnCallError> {
        let def = self.registry.get(fn_name).ok_or_else(|| FnCallError {
            code: "FN_NOT_FOUND".into(),
            message: format!("Function \"{fn_name}\" is not registered"),
        })?;

        match def.fn_type {
            FnType::Mutation => {
                // Postgres backend: route through PostgresDataStore::with_transaction
                // so the TS handler runs against a held PG transaction. Reads
                // see the handler's own pending writes; an error rolls
                // everything back atomically. Change events are buffered in a
                // `PgBufferedTxStore` wrapper and flushed only after COMMIT —
                // mirrors the SQLite `TxStore` behavior.
                if self.runtime.is_postgres() {
                    let pg_backend = self.runtime.pg_backend().ok_or_else(|| FnCallError {
                        code: "PG_BACKEND_MISSING".into(),
                        message:
                            "Postgres backend reported is_postgres=true but pg_backend() returned None"
                                .into(),
                    })?;

                    // The closure has to own its own state (Box the stream
                    // callback so we can move it inside). Capture
                    // `request`/`auth`/`args` by move; they aren't needed
                    // again outside the closure.
                    let runner = self.runner.clone();
                    let fn_type = def.fn_type;
                    let fn_name_owned = fn_name.to_string();

                    // Push the schedule buffer onto the thread-local for
                    // the duration of the handler. Drain after COMMIT
                    // succeeds; on rollback the buffer is dropped.
                    let sched_guard = ScheduleBufferGuard::enter();
                    // Mark "we're in a mutation tx" so the nested-call
                    // hook rejects recursive mutation calls instead of
                    // deadlocking on the connection mutex.
                    let _depth_guard = MutationDepthGuard::enter();

                    // Install the CRDT hook so PgTxStore projects
                    // `ctx.db.X` writes on `crdt: true` entities through
                    // PgLoroStore + persists the snapshot in the same
                    // tx. Without this, codex-flagged: TS mutation
                    // writes on CRDT entities desync from the sidecar.
                    let crdt_hook: std::sync::Arc<
                        dyn pylon_storage::pg_tx_store::PgCrdtHook,
                    > = std::sync::Arc::new(crate::pg_loro_store::PgCrdtHookImpl {
                        crdt: std::sync::Arc::clone(&pg_backend.crdt),
                        manifest: std::sync::Arc::new(self.runtime.manifest().clone()),
                    });

                    let pg = &pg_backend.store;
                    let tx_result: Result<
                        (serde_json::Value, FnTrace, Vec<pylon_sync::ChangeEvent>),
                        FnCallError,
                    > = pg.with_transaction_crdt(crdt_hook, move |inner_store: &dyn DataStore| {
                        let buffered = PgBufferedTxStore::new(inner_store);
                        let (value, trace) = runner.call(
                            &buffered,
                            &fn_name_owned,
                            fn_type,
                            args,
                            auth,
                            on_stream,
                            request,
                        )?;
                        Ok((value, trace, buffered.take_pending()))
                    });

                    return match tx_result {
                        Ok((value, trace, pending)) => {
                            // Mirror the SQLite path: append to the change
                            // log first (so /api/sync/pull tail callers see
                            // it), then notify WS/SSE subscribers.
                            for ev in pending {
                                let seq = self.change_log.append(
                                    &ev.entity,
                                    &ev.row_id,
                                    ev.kind.clone(),
                                    ev.data.clone(),
                                );
                                let event = pylon_sync::ChangeEvent { seq, ..ev };
                                self.notifier.notify(&event);
                            }
                            // Flush scheduled jobs after the commit lands.
                            // On rollback the early `Err(e)` arm below
                            // skips this and the buffer is dropped.
                            self.flush_pending_schedules(sched_guard.take());
                            drop(sched_guard);
                            Ok((value, trace))
                        }
                        Err(e) => {
                            drop(sched_guard);
                            Err(e)
                        }
                    };
                }
                // Hold the write connection for the entire handler duration.
                // This keeps the BEGIN/COMMIT span free of interleaving from
                // other writers (who would otherwise become part of the
                // transaction because SQLite tracks it on the connection).
                //
                // Inside the handler, every `ctx.db` call routes through
                // TxStore, which uses this same held connection — so no
                // re-locking, no deadlock, no interleaving.
                let conn_guard = self.runtime.lock_conn_pub().map_err(|e| FnCallError {
                    code: e.code,
                    message: e.message,
                })?;

                if let Err(e) = conn_guard.execute("BEGIN", []) {
                    return Err(FnCallError {
                        code: "BEGIN_FAILED".into(),
                        message: format!("Failed to start transaction: {e}"),
                    });
                }

                // Same schedule buffering as the PG path — `runAfter`
                // calls inside this handler defer until COMMIT succeeds.
                let sched_guard = ScheduleBufferGuard::enter();
                // Same nested-mutation deadlock guard as the PG path.
                let _depth_guard = MutationDepthGuard::enter();

                let tx_store = TxStore::new(&self.runtime, &conn_guard);
                let result = self.runner.call(
                    &tx_store,
                    fn_name,
                    def.fn_type,
                    args,
                    auth,
                    on_stream,
                    request,
                );

                // Surface commit/rollback errors. A swallowed COMMIT failure
                // is the worst possible outcome: the caller sees success but
                // the data isn't durable. A swallowed ROLLBACK failure leaves
                // the connection in an unknown txn state for the next caller.
                let result = match result {
                    Ok(value) => match conn_guard.execute("COMMIT", []) {
                        Ok(_) => {
                            // Flush buffered change events NOW — after the
                            // commit durably lands but before we return
                            // success. Ordering matters: append to the log
                            // first (so /api/sync/pull callers that race
                            // with this broadcast see the row in the tail),
                            // then notify WS/SSE subscribers. `seq` on each
                            // pending event starts at 0; append assigns
                            // the real seq.
                            for ev in tx_store.take_pending() {
                                let seq = self.change_log.append(
                                    &ev.entity,
                                    &ev.row_id,
                                    ev.kind.clone(),
                                    ev.data.clone(),
                                );
                                let event = pylon_sync::ChangeEvent { seq, ..ev };
                                self.notifier.notify(&event);
                            }
                            // Same flush as the PG path — durable commit,
                            // then flush schedules. Drop the guard
                            // explicitly so the thread-local clears
                            // before the result returns.
                            self.flush_pending_schedules(sched_guard.take());
                            drop(sched_guard);
                            Ok(value)
                        }
                        Err(commit_err) => {
                            // Best-effort cleanup. If ROLLBACK also fails the
                            // connection is in a bad state — at minimum the
                            // operator sees both failures in the log.
                            if let Err(rollback_err) = conn_guard.execute("ROLLBACK", []) {
                                tracing::warn!(
                                    "[functions] ROLLBACK after COMMIT failure also failed: {rollback_err}"
                                );
                            }
                            Err(FnCallError {
                                code: "COMMIT_FAILED".into(),
                                message: format!(
                                    "Function \"{fn_name}\" succeeded but COMMIT failed: {commit_err}"
                                ),
                            })
                        }
                    },
                    Err(handler_err) => {
                        if let Err(rollback_err) = conn_guard.execute("ROLLBACK", []) {
                            // Don't shadow the handler error — log the
                            // rollback failure separately.
                            tracing::warn!(
                                "[functions] ROLLBACK after handler error failed: {rollback_err}"
                            );
                        }
                        Err(handler_err)
                    }
                };
                // conn_guard drops here, releasing the lock.
                result
            }
            _ => self.runner.call(
                &*self.runtime,
                fn_name,
                def.fn_type,
                args,
                auth,
                on_stream,
                request,
            ),
        }
    }

    fn recent_traces(&self, limit: usize) -> Vec<FnTrace> {
        self.runner.trace_log.recent(limit)
    }

    fn check_rate_limit(&self, fn_name: &str, identity: &str) -> Result<(), u64> {
        let key = format!("{fn_name}::{identity}");
        self.fn_rate_limiter.check(&key)
    }
}

/// Spawn the Bun function runtime if a `functions/` directory exists.
///
/// Returns `Some(FnOpsImpl)` if successful, `None` if no functions directory
/// or if Bun is not installed. Errors during startup print to stderr and
/// return `None` to keep the server running.
/// Resolve the path to the TypeScript function runtime script.
///
/// Searches in order:
/// 1. `$PYLON_FUNCTIONS_RUNTIME` environment variable (if set and file exists)
/// 2. `./node_modules/@pylon/functions/src/runtime.ts` (npm-installed)
/// 3. `./node_modules/@pylon/functions/dist/runtime.js` (built)
/// 4. `~/.pylon/runtime.ts` (user install)
/// 5. `packages/functions/src/runtime.ts` (dev monorepo)
///
/// Returns `None` if none exist.
pub fn find_functions_runtime() -> Option<String> {
    if let Ok(env_path) = std::env::var("PYLON_FUNCTIONS_RUNTIME") {
        if std::path::Path::new(&env_path).exists() {
            return Some(env_path);
        }
    }

    // Walk parent directories like Node.js resolution does, so running
    // `pylon dev` from an example sub-directory still finds the
    // hoisted workspace package at the repo root. Without this, bun/npm
    // workspace users see "TypeScript function runtime is not configured"
    // and think the server is broken when it's just a CWD issue.
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let relative_candidates = [
        "node_modules/@pylonsync/functions/src/runtime.ts",
        "node_modules/@pylonsync/functions/dist/runtime.js",
        // Legacy name — keeps upgrades from the pre-rename era working.
        "node_modules/@pylon/functions/src/runtime.ts",
        "node_modules/@pylon/functions/dist/runtime.js",
        // Monorepo dev: source tree at the workspace root.
        "packages/functions/src/runtime.ts",
    ];

    let mut dir: Option<&std::path::Path> = Some(cwd.as_path());
    while let Some(current) = dir {
        for rel in &relative_candidates {
            let candidate = current.join(rel);
            if candidate.exists() {
                return candidate.to_str().map(|s| s.to_string());
            }
        }
        dir = current.parent();
    }

    // Final fallback: user-wide install under ~/.pylon.
    let user_path = format!("{home}/.pylon/runtime.ts");
    if std::path::Path::new(&user_path).exists() {
        return Some(user_path);
    }
    None
}

pub fn try_spawn_functions(
    runtime: Arc<Runtime>,
    job_queue: Arc<crate::jobs::JobQueue>,
    fn_rate_limiter: Arc<crate::rate_limit::RateLimiter>,
    change_log: Arc<pylon_sync::ChangeLog>,
    notifier: Arc<dyn pylon_router::ChangeNotifier>,
) -> Option<Arc<FnOpsImpl>> {
    let fn_dir = std::env::var("PYLON_FUNCTIONS_DIR").unwrap_or_else(|_| "functions".into());
    if !std::path::Path::new(&fn_dir).exists() {
        return None;
    }

    let runtime_script = match find_functions_runtime() {
        Some(p) => p,
        None => {
            tracing::warn!(
                "[functions] No TypeScript runtime script found. TypeScript functions will be unavailable."
            );
            tracing::warn!(
                "[functions] Tried: $PYLON_FUNCTIONS_RUNTIME, node_modules/@pylon/functions/src/runtime.ts, ~/.pylon/runtime.ts, packages/functions/src/runtime.ts"
            );
            return None;
        }
    };

    let runner = Arc::new(FnRunner::new(1000));

    // start() now performs the handshake itself and returns the function
    // definitions, so there's no separate handshake step. On any failure the
    // child has already been killed.
    let defs = match runner.start("bun", &["run", &runtime_script, &fn_dir]) {
        Ok(defs) => defs,
        Err(e) => {
            tracing::warn!("[functions] Failed to start Bun runtime: {e}");
            tracing::warn!(
                "[functions] Install Bun from https://bun.sh — TypeScript functions will be unavailable."
            );
            return None;
        }
    };

    // Hold a separate handle on the job queue for registering function
    // job handlers below, since the schedule-hook closure consumes its
    // own copy.
    let job_queue_for_handlers = Arc::clone(&job_queue);

    // Wire scheduler requests from functions into the job queue. Use the
    // Result-returning variant so a persist failure surfaces as a TS-side
    // SCHEDULE_FAILED error instead of `{scheduled:true, id:""}`.
    //
    // Transaction-bound semantics: when this hook is invoked from inside
    // a mutation handler, the per-call `MUTATION_SCHEDULE_BUFFER`
    // thread-local is set. Schedules buffer there and drain post-COMMIT
    // (so a rolled-back mutation can't leave behind scheduled work).
    // Outside a mutation (queries, actions, top-level non-mutation
    // jobs), the buffer is `None` and we enqueue immediately — matching
    // the historical contract for those code paths.
    runner.set_schedule_hook(Box::new(move |fn_name, args, delay_ms, run_at| {
        // Check the thread-local first. If we're inside a mutation, the
        // buffer is `Some` and we defer.
        let buffered = MUTATION_SCHEDULE_BUFFER.with(|cell| {
            let slot = cell.borrow();
            slot.as_ref().map(|b| {
                b.borrow_mut().push(PendingSchedule {
                    fn_name: fn_name.to_string(),
                    args: args.clone(),
                    delay_ms,
                    run_at,
                });
            }).is_some()
        });
        if buffered {
            // No real job-id yet — the actual enqueue happens after
            // COMMIT. Returning a synthetic id keeps the TS contract
            // (`{scheduled:true,id:string}`) intact; a mutation that
            // rolls back will discard the buffer and the id was never
            // observable to anyone outside the handler anyway.
            return Ok(format!("pending:{fn_name}"));
        }

        let delay_secs = match (delay_ms, run_at) {
            (Some(ms), _) => ms / 1000,
            (None, Some(ts)) => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                if ts > now {
                    (ts - now) / 1000
                } else {
                    0
                }
            }
            _ => 0,
        };
        job_queue.try_enqueue_with_options(
            fn_name,
            args,
            crate::jobs::Priority::Normal,
            delay_secs,
            3,
            "functions",
        )
    }));

    let registry = Arc::new(FnRegistry::new());
    let count = defs.len();
    registry.replace_all(defs);
    tracing::warn!("[functions] Loaded {count} function(s) from {fn_dir}");

    let ops = Arc::new(FnOpsImpl {
        runner,
        registry,
        runtime,
        fn_rate_limiter,
        change_log,
        notifier,
        job_queue: Arc::clone(&job_queue_for_handlers),
    });

    install_nested_call_hook(&ops);
    register_function_job_handlers(&ops, &job_queue_for_handlers);
    spawn_runtime_supervisor(Arc::clone(&ops));
    Some(ops)
}

/// Bridge scheduled function calls (via `ctx.scheduler.runAfter` or
/// `runAt`) to the function runner. Without this, the schedule hook
/// enqueues a job whose `name` is the function name — but no handler
/// is registered for it, so the worker fails with "No handler
/// registered" and the scheduled callback never runs.
///
/// Registers one handler per loaded function. Each handler invokes
/// `FnOpsImpl::call` with a system auth context (no user_id, not
/// admin) so the called function runs with the same privileges a
/// trusted-server-side caller would have. The args come from the
/// job payload, which the schedule hook copies verbatim from the
/// `runAfter(ms, fn, args)` invocation.
fn register_function_job_handlers(ops: &Arc<FnOpsImpl>, job_queue: &Arc<crate::jobs::JobQueue>) {
    use pylon_router::FnOps as _;

    let fn_names: Vec<String> = ops.registry.list().into_iter().map(|d| d.name).collect();

    for name in fn_names {
        let weak = Arc::downgrade(ops);
        let fn_name = name.clone();
        job_queue.register(
            &name,
            Arc::new(move |job: &crate::jobs::Job| {
                let ops = match weak.upgrade() {
                    Some(o) => o,
                    None => {
                        return crate::jobs::JobResult::Failure(
                            "RUNTIME_GONE: function ops dropped".into(),
                        )
                    }
                };
                let auth = FnAuth {
                    user_id: None,
                    is_admin: false,
                    tenant_id: None,
                };
                match ops.call(&fn_name, job.payload.clone(), auth, None, None) {
                    Ok(_) => crate::jobs::JobResult::Success,
                    Err(e) => crate::jobs::JobResult::Retry(format!("{}: {}", e.code, e.message)),
                }
            }),
        );
    }
}

/// Route nested `RunFn` calls (action → query/mutation) through a
/// transactional wrapper so nested mutations get their own BEGIN/COMMIT.
///
/// Uses a `Weak<FnOpsImpl>` to avoid keeping the ops struct alive forever
/// through a cycle (hook stored on FnRunner ← held by FnOpsImpl). When the
/// ops struct is dropped the hook becomes a no-op error.
fn install_nested_call_hook(ops: &Arc<FnOpsImpl>) {
    use pylon_functions::protocol::{AuthInfo, FnType};

    let weak = Arc::downgrade(ops);
    ops.runner.set_nested_call_hook(Box::new(
        move |fn_name: &str,
              fn_type: FnType,
              args: serde_json::Value,
              auth: AuthInfo|
              -> Result<serde_json::Value, (String, String)> {
            let ops = match weak.upgrade() {
                Some(o) => o,
                None => {
                    return Err((
                        "RUNTIME_GONE".into(),
                        "pylon runtime is shutting down".into(),
                    ))
                }
            };

            match fn_type {
                FnType::Mutation => {
                    // Reject nested mutations: both backends acquire a
                    // single (non-reentrant) connection mutex per
                    // mutation, so a TS handler that calls runMutation
                    // from inside another mutation would block forever.
                    // Surface a clear NESTED_MUTATION error instead of
                    // hanging — callers should restructure to call the
                    // shared logic as a function (not a separate
                    // mutation) or call from an action.
                    if in_mutation_tx() {
                        return Err((
                            "NESTED_MUTATION".into(),
                            format!(
                                "ctx.runMutation(\"{fn_name}\") is not allowed from inside \
                                 another mutation handler — the mutation handler IS the \
                                 transaction, and the connection mutex is non-reentrant. \
                                 Restructure the shared logic into a regular function (not \
                                 a registered mutation), or call from an action handler."
                            ),
                        ));
                    }

                    // Postgres backend: route through the PG with_transaction
                    // closure, mirroring the top-level mutation path. Without
                    // this, action -> ctx.runMutation(...) errors with
                    // NOT_SQLITE_BACKEND on PG even though the top-level path
                    // works fine.
                    if ops.runtime.is_postgres() {
                        let pg_backend = ops.runtime.pg_backend().ok_or_else(|| {
                            (
                                "PG_BACKEND_MISSING".into(),
                                "Postgres backend reported is_postgres=true but pg_backend() returned None".into(),
                            )
                        })?;
                        let pg = &pg_backend.store;
                        let runner = ops.runner.clone();
                        let fn_name_owned = fn_name.to_string();
                        let sched_guard = ScheduleBufferGuard::enter();
                        let _depth_guard = MutationDepthGuard::enter();
                        // Same CRDT hook as the top-level mutation
                        // path so action -> runMutation on a crdt:true
                        // entity also maintains the sidecar.
                        let crdt_hook: std::sync::Arc<
                            dyn pylon_storage::pg_tx_store::PgCrdtHook,
                        > = std::sync::Arc::new(crate::pg_loro_store::PgCrdtHookImpl {
                            crdt: std::sync::Arc::clone(&pg_backend.crdt),
                            manifest: std::sync::Arc::new(ops.runtime.manifest().clone()),
                        });
                        let tx_result: Result<
                            (serde_json::Value, Vec<pylon_sync::ChangeEvent>),
                            FnCallError,
                        > = pg.with_transaction_crdt(crdt_hook, move |inner_store: &dyn DataStore| {
                            let buffered = PgBufferedTxStore::new(inner_store);
                            let (value, _trace) = runner.call_inner(
                                &buffered,
                                &fn_name_owned,
                                fn_type,
                                args,
                                auth,
                                None,
                                None,
                            )?;
                            Ok((value, buffered.take_pending()))
                        });
                        return match tx_result {
                            Ok((value, pending)) => {
                                for ev in pending {
                                    let seq = ops.change_log.append(
                                        &ev.entity,
                                        &ev.row_id,
                                        ev.kind.clone(),
                                        ev.data.clone(),
                                    );
                                    let event = pylon_sync::ChangeEvent { seq, ..ev };
                                    ops.notifier.notify(&event);
                                }
                                ops.flush_pending_schedules(sched_guard.take());
                                drop(sched_guard);
                                Ok(value)
                            }
                            Err(e) => {
                                drop(sched_guard);
                                Err((e.code, e.message))
                            }
                        };
                    }

                    // Wrap the nested mutation in its own write-conn + BEGIN
                    // + COMMIT, matching the top-level mutation contract.
                    let conn_guard = ops
                        .runtime
                        .lock_conn_pub()
                        .map_err(|e| (e.code, e.message))?;
                    if let Err(e) = conn_guard.execute("BEGIN", []) {
                        return Err(("BEGIN_FAILED".into(), e.to_string()));
                    }
                    let sched_guard = ScheduleBufferGuard::enter();
                    let _depth_guard = MutationDepthGuard::enter();
                    let tx_store = TxStore::new(&ops.runtime, &conn_guard);
                    // Re-enter protocol without acquiring io_lock — we're
                    // already inside the outer call_inner which holds it.
                    // Nested calls never get HTTP request metadata — that's
                    // only meaningful for the top-level webhook invocation.
                    let result = ops
                        .runner
                        .call_inner(&tx_store, fn_name, fn_type, args, auth, None, None);
                    match result {
                        Ok((value, _trace)) => {
                            if let Err(e) = conn_guard.execute("COMMIT", []) {
                                let _ = conn_guard.execute("ROLLBACK", []);
                                return Err(("COMMIT_FAILED".into(), e.to_string()));
                            }
                            // Flush change events after COMMIT so nested
                            // mutations (action → runMutation(...)) broadcast
                            // the same way top-level mutations do. Without
                            // this, every write an action emits is invisible
                            // to sync subscribers until the NEXT top-level
                            // mutation lands — streaming UIs stay empty.
                            for ev in tx_store.take_pending() {
                                let seq = ops.change_log.append(
                                    &ev.entity,
                                    &ev.row_id,
                                    ev.kind.clone(),
                                    ev.data.clone(),
                                );
                                let event = pylon_sync::ChangeEvent { seq, ..ev };
                                ops.notifier.notify(&event);
                            }
                            ops.flush_pending_schedules(sched_guard.take());
                            drop(sched_guard);
                            Ok(value)
                        }
                        Err(e) => {
                            let _ = conn_guard.execute("ROLLBACK", []);
                            drop(sched_guard);
                            Err((e.code, e.message))
                        }
                    }
                }
                _ => {
                    // Queries + actions: no transaction wrap needed. Just
                    // re-enter protocol via the same store (runtime).
                    // Nested: no HTTP request propagated (see above).
                    let result = ops.runner.call_inner(
                        &*ops.runtime,
                        fn_name,
                        fn_type,
                        args,
                        auth,
                        None,
                        None,
                    );
                    result.map(|(v, _)| v).map_err(|e| (e.code, e.message))
                }
            }
        },
    ));
}

/// Background watchdog that restarts the Bun runtime if it dies (crashed,
/// killed by the call timeout path, OOM, etc.). Exponential backoff: 1s, 2s,
/// 4s, ... capped at 30s. Resets to 1s after a successful respawn.
///
/// We don't try to "give up" — if Bun keeps crashing the supervisor keeps
/// trying with the capped delay. The operator sees repeated WARN logs and
/// can investigate. Better than silently leaving functions disabled forever.
fn spawn_runtime_supervisor(ops: Arc<FnOpsImpl>) {
    use std::time::Duration;

    std::thread::Builder::new()
        .name("pylon-fn-supervisor".into())
        .spawn(move || {
            let mut backoff = Duration::from_secs(1);
            let max_backoff = Duration::from_secs(30);
            loop {
                std::thread::sleep(Duration::from_secs(2));
                if ops.runner.is_alive() {
                    backoff = Duration::from_secs(1);
                    continue;
                }
                tracing::warn!(
                    "[functions] Bun runtime is not alive — respawning after {:?}",
                    backoff
                );
                std::thread::sleep(backoff);
                match ops.runner.respawn() {
                    Ok(defs) => {
                        let count = defs.len();
                        // Replace, not merge — deleted functions must stop
                        // being callable. register_all() alone leaves stale
                        // entries from the previous process generation.
                        ops.registry.replace_all(defs);
                        tracing::warn!("[functions] Respawned Bun runtime ({count} fn(s))");
                        backoff = Duration::from_secs(1);
                    }
                    Err(e) => {
                        tracing::warn!("[functions] Respawn failed: {e}");
                        // Persistent Bun-runtime failures are the kind of
                        // operator signal that belongs in error telemetry
                        // too. Include enough context to triage repeated
                        // events: current backoff (so operators can see
                        // how long failures have been compounding) and the
                        // component name.
                        let backoff_str = format!("{}", backoff.as_secs());
                        pylon_observability::report_error(&pylon_observability::ErrorEvent {
                            level: pylon_observability::ErrorLevel::Error,
                            code: "FN_RESPAWN_FAILED",
                            message: &e,
                            context: &[
                                ("component", "bun-runtime-supervisor"),
                                ("backoff_secs", &backoff_str),
                            ],
                        });
                        backoff = (backoff * 2).min(max_backoff);
                    }
                }
            }
        })
        .expect("failed to spawn function runtime supervisor");
}
