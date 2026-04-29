//! [`PgTxStore`] — a `DataStore` impl backed by a single in-progress
//! Postgres transaction. Powers TS-function `Mutation` handlers on
//! Postgres-backed deploys: every `ctx.db.X` call routes through the
//! same transaction, reads see the handler's own pending writes, and
//! a thrown error rolls everything back atomically.
//!
//! All the heavy lifting lives in PgConn-generic free functions
//! (`tx_insert`, `tx_update`, `tx_delete`, etc.) so the runtime layer
//! can call the same primitives inside its own held transaction —
//! that's how CRDT projection + entity write + FTS maintenance all
//! land in one BEGIN/COMMIT.
//!
//! Lifetime contract: the inner `postgres::Transaction<'_>` borrows
//! the `&mut Client` that the caller (`PostgresDataStore::with_transaction`)
//! locked via the inner mutex. The store cannot outlive that lock —
//! see `PostgresDataStore::with_transaction` for the closure pattern
//! that enforces this.

#![cfg(feature = "postgres-live")]

use std::sync::Mutex;

use pylon_http::{DataError, DataStore};
use pylon_kernel::AppManifest;

use crate::pg_exec::PgConn;
use crate::pg_search;
use crate::postgres::{
    aggregate_rows_to_json_pub, build_aggregate_sql_pub, build_insert_sql,
    build_query_filtered_sql_pub, build_update_sql, quote_ident_pub, JsonParam,
};

// ---------------------------------------------------------------------------
// PgConn-generic primitives — used by PgTxStore AND by the runtime
// layer's CRDT-augmented closures so both compose into one tx.
// ---------------------------------------------------------------------------

/// `as_pg_params` lifts `&[JsonParam]` into the slice-of-refs the
/// postgres driver wants. Free function so it's reachable from any
/// caller (PgTxStore, runtime-side helpers).
pub fn as_pg_params(values: &[JsonParam]) -> Vec<&(dyn postgres::types::ToSql + Sync)> {
    values
        .iter()
        .map(|v| v as &(dyn postgres::types::ToSql + Sync))
        .collect()
}

/// Insert one entity row + maintain its FTS shadow (if any) through
/// the supplied connection (Client or Transaction). Returns the
/// generated id (or the caller-supplied id from `data["id"]`).
pub fn tx_insert<C: PgConn>(
    conn: &mut C,
    manifest: &AppManifest,
    entity: &str,
    data: &serde_json::Value,
) -> Result<String, DataError> {
    let (sql, values) = build_insert_sql(entity, data).map_err(|e| DataError {
        code: e.code,
        message: e.message,
    })?;
    let id = match &values[0] {
        JsonParam::Text(s) => s.clone(),
        _ => {
            return Err(DataError {
                code: "PG_INTERNAL".into(),
                message: "build_insert_sql produced non-text id param".into(),
            });
        }
    };
    let params = as_pg_params(&values);
    conn.execute(&sql, &params).map_err(pg_err_to_data)?;
    if let Some(cfg) = search_config_for(manifest, entity) {
        // Build the FTS payload from the data the caller passed +
        // the resolved id, so text-field projection sees the
        // landed row's shape.
        let mut row_with_id = data.clone();
        if let Some(obj) = row_with_id.as_object_mut() {
            obj.insert("id".into(), serde_json::Value::String(id.clone()));
        }
        pg_search::apply_insert(conn, entity, &id, &row_with_id, &cfg)
            .map_err(search_err_to_data)?;
    }
    Ok(id)
}

/// Update one entity row + maintain its FTS shadow.
///
/// FTS maintenance needs the post-update row state to rebuild the
/// tsvector correctly. To avoid a lost-update race between concurrent
/// `tx_update` calls (codex-flagged: two replicas read the same
/// pre-update row, both compute their own merge, last write wins on
/// the FTS shadow but might not match the row's actual final state)
/// we lock the row via `SELECT ... FOR UPDATE` BEFORE the UPDATE.
/// Then any concurrent `tx_update` blocks until we commit, after
/// which it reads the new post-update state for its own merge.
///
/// The lock is no-op when the row doesn't exist (UPDATE finds 0 rows
/// and returns false anyway). When the row exists and search isn't
/// configured we still take the lock — cheap and keeps the update
/// path's locking shape uniform.
pub fn tx_update<C: PgConn>(
    conn: &mut C,
    manifest: &AppManifest,
    entity: &str,
    id: &str,
    data: &serde_json::Value,
) -> Result<bool, DataError> {
    let (sql, values) = build_update_sql(entity, id, data).map_err(|e| DataError {
        code: e.code,
        message: e.message,
    })?;
    let search_cfg = search_config_for(manifest, entity);

    // Take a row lock for the rest of the tx so the FTS rebuild and
    // the actual UPDATE see a consistent view. Skip when the row
    // doesn't exist — there's nothing to lock and the UPDATE below
    // will return Ok(false).
    let lock_sql = format!(
        "SELECT 1 FROM {} WHERE id = $1 FOR UPDATE",
        quote_ident_pub(entity)
    );
    let locked = conn
        .query_opt(&lock_sql, &[&id])
        .map_err(pg_err_to_data)?;
    if locked.is_none() {
        return Ok(false);
    }

    // Read the OLD row only after the lock so concurrent updates
    // can't slip in between read and UPDATE.
    let old_row = if search_cfg.is_some() {
        tx_get_by_id(conn, entity, id)?.unwrap_or(serde_json::Value::Null)
    } else {
        serde_json::Value::Null
    };
    let params = as_pg_params(&values);
    let n = conn.execute(&sql, &params).map_err(pg_err_to_data)?;
    if n > 0 {
        if let Some(cfg) = &search_cfg {
            pg_search::apply_update(conn, entity, id, &old_row, data, cfg)
                .map_err(search_err_to_data)?;
        }
    }
    Ok(n > 0)
}

/// Delete one entity row + clear its FTS shadow. The FK CASCADE on
/// `_fts_<entity>.entity_id` already covers shadow cleanup when the
/// entity row is dropped, but we call the explicit helper too so the
/// maintenance contract matches the SQLite path.
pub fn tx_delete<C: PgConn>(
    conn: &mut C,
    manifest: &AppManifest,
    entity: &str,
    id: &str,
) -> Result<bool, DataError> {
    let sql = format!(
        "DELETE FROM {} WHERE id = $1",
        quote_ident_pub(entity)
    );
    if let Some(cfg) = search_config_for(manifest, entity) {
        pg_search::apply_delete(conn, entity, id, &cfg).map_err(search_err_to_data)?;
    }
    let n = conn.execute(&sql, &[&id]).map_err(pg_err_to_data)?;
    Ok(n > 0)
}

/// Read one row by id. Free function so the runtime layer can read
/// pre-update state from inside its own held tx.
pub fn tx_get_by_id<C: PgConn>(
    conn: &mut C,
    entity: &str,
    id: &str,
) -> Result<Option<serde_json::Value>, DataError> {
    let sql = format!(
        "SELECT * FROM {} WHERE id = $1",
        quote_ident_pub(entity)
    );
    let row = conn.query_opt(&sql, &[&id]).map_err(pg_err_to_data)?;
    Ok(row.map(|r| crate::postgres::row_to_json_pub(&r)))
}

/// Resolve the entity's `search:` config from the manifest. `None`
/// means the entity isn't searchable — maintenance helpers
/// short-circuit on `None`.
pub fn search_config_for(manifest: &AppManifest, entity: &str) -> Option<crate::search::SearchConfig> {
    manifest
        .entities
        .iter()
        .find(|e| e.name == entity)
        .and_then(|e| e.search.clone())
        .filter(|c| !c.is_empty())
}

/// Convert a `postgres::Error` into a `DataError` while walking the
/// source chain so callers see the underlying SQLSTATE message instead
/// of `postgres::Error`'s deliberately short Display impl.
pub fn pg_err_to_data(e: postgres::Error) -> DataError {
    use std::error::Error;
    let mut detail = format!("{e}");
    let mut src: Option<&dyn Error> = e.source();
    while let Some(s) = src {
        detail.push_str(": ");
        detail.push_str(&format!("{s}"));
        src = s.source();
    }
    DataError {
        code: "PG_TX_QUERY_FAILED".into(),
        message: format!("Postgres query in transaction failed: {detail}"),
    }
}

/// Lift a search-maintenance `StorageError` into a `DataError` so the
/// closure boundary sees a uniform error type.
pub fn search_err_to_data(e: crate::StorageError) -> DataError {
    DataError {
        code: e.code,
        message: e.message,
    }
}

// ---------------------------------------------------------------------------
// CRDT hook — runtime-layer injection point so PgTxStore can project
// CRDT writes through PgLoroStore without depending on pylon-runtime.
// ---------------------------------------------------------------------------

/// Implemented by the runtime layer's PgLoroStore wrapper. Called by
/// PgTxStore inside its insert/update/delete (when running through
/// the `with_crdt` constructor) so a TS mutation handler's
/// `ctx.db.X` calls maintain the CRDT sidecar in the same tx.
///
/// `before_insert` may return a new `id` to use (when the runtime
/// generated one for the CRDT snapshot key) — tx_insert respects
/// `data["id"]` if present, so the hook injects there.
pub trait PgCrdtHook: Send + Sync {
    fn before_insert(
        &self,
        tx: &mut postgres::Transaction<'_>,
        entity: &str,
        data: &serde_json::Value,
    ) -> Result<Option<serde_json::Value>, DataError>;

    fn before_update(
        &self,
        tx: &mut postgres::Transaction<'_>,
        entity: &str,
        id: &str,
        data: &serde_json::Value,
    ) -> Result<(), DataError>;

    fn before_delete(
        &self,
        tx: &mut postgres::Transaction<'_>,
        entity: &str,
        id: &str,
    ) -> Result<(), DataError>;

    /// Called after the surrounding tx commits successfully — runtime
    /// uses this to refresh its in-memory cache.
    fn after_commit(&self, entity: &str, id: &str);

    /// Called when the surrounding tx rolls back — runtime uses this
    /// to evict any cache entries it touched in apply_patch.
    fn on_rollback(&self, entity: &str, id: &str);
}

// ---------------------------------------------------------------------------
// PgTxStore — DataStore impl wrapping a held postgres Transaction
// ---------------------------------------------------------------------------

pub struct PgTxStore<'a> {
    /// The active transaction. Wrapped in `Mutex` because the
    /// `DataStore` trait is `Send + Sync` (the trait can't ask for
    /// `&mut self` without breaking other backends), while postgres
    /// `Transaction::execute`/`query` need `&mut self`. Contention is
    /// nil — `PostgresDataStore::with_transaction` already holds the
    /// outer connection mutex for the closure's lifetime, so this
    /// inner lock is uncontended every call.
    tx: Mutex<Option<postgres::Transaction<'a>>>,
    manifest: &'a AppManifest,
    /// Optional runtime-supplied CRDT hook. When present, insert/
    /// update/delete on entities with `crdt: true` route through it
    /// FIRST so the CRDT sidecar + materialized row land in the same
    /// tx. Plain (non-CRDT) entities skip the hook.
    crdt_hook: Option<std::sync::Arc<dyn PgCrdtHook>>,
    /// Rows touched via the CRDT hook. After commit/rollback the
    /// `commit()`/drop runs `after_commit`/`on_rollback` for each so
    /// the runtime layer can keep its cache in sync.
    crdt_touched: Mutex<Vec<(String, String)>>,
}

impl<'a> PgTxStore<'a> {
    pub fn new(tx: postgres::Transaction<'a>, manifest: &'a AppManifest) -> Self {
        Self {
            tx: Mutex::new(Some(tx)),
            manifest,
            crdt_hook: None,
            crdt_touched: Mutex::new(Vec::new()),
        }
    }

    /// Construct a PgTxStore with a runtime-supplied CRDT hook.
    /// Used by FnOpsImpl::call's PG mutation path so a TS handler's
    /// `ctx.db.insert/update/delete` on `crdt: true` entities
    /// projects through the LoroDoc + persists the snapshot in the
    /// same transaction as the materialized row.
    pub fn with_crdt(
        tx: postgres::Transaction<'a>,
        manifest: &'a AppManifest,
        crdt_hook: std::sync::Arc<dyn PgCrdtHook>,
    ) -> Self {
        Self {
            tx: Mutex::new(Some(tx)),
            manifest,
            crdt_hook: Some(crdt_hook),
            crdt_touched: Mutex::new(Vec::new()),
        }
    }

    /// Returns true iff this store has a CRDT hook installed and the
    /// entity has `crdt: true` in the manifest.
    fn entity_is_crdt(&self, entity: &str) -> bool {
        self.crdt_hook.is_some()
            && self
                .manifest
                .entities
                .iter()
                .any(|e| e.name == entity && e.crdt)
    }

    fn record_crdt_touched(&self, entity: &str, id: &str) {
        if let Ok(mut g) = self.crdt_touched.lock() {
            g.push((entity.to_string(), id.to_string()));
        }
    }

    /// Commit the underlying transaction. Caller is `with_transaction`
    /// after the body returns `Ok`. After commit the inner transaction
    /// is consumed; subsequent `DataStore` calls will return an
    /// internal error (which shouldn't happen — the closure's
    /// dropped immediately).
    ///
    /// Fires `after_commit` on the CRDT hook for every (entity, id)
    /// that flowed through the hook so the runtime can refresh its
    /// in-memory cache. On commit failure (or if anyone calls
    /// `Drop` without `commit`), `on_rollback` runs from the Drop
    /// impl below.
    pub fn commit(self) -> Result<(), postgres::Error> {
        let touched: Vec<(String, String)> = self
            .crdt_touched
            .lock()
            .map(|mut g| std::mem::take(&mut *g))
            .unwrap_or_default();
        let hook = self.crdt_hook.clone();
        let result = {
            let mut guard = self.tx.lock().expect("PgTxStore mutex poisoned");
            if let Some(tx) = guard.take() {
                tx.commit()
            } else {
                Ok(())
            }
        };
        match (&result, hook) {
            (Ok(_), Some(h)) => {
                for (entity, id) in &touched {
                    h.after_commit(entity, id);
                }
            }
            (Err(_), Some(h)) => {
                for (entity, id) in &touched {
                    h.on_rollback(entity, id);
                }
            }
            _ => {}
        }
        result
    }
}

impl<'a> Drop for PgTxStore<'a> {
    fn drop(&mut self) {
        // If we still hold a tx (caller didn't commit), the
        // postgres::Transaction's own Drop runs ROLLBACK. We need to
        // mirror that on the CRDT cache so any apply_patch'd docs
        // get re-hydrated next time.
        let still_held = self
            .tx
            .lock()
            .map(|g| g.is_some())
            .unwrap_or(false);
        if !still_held {
            return;
        }
        if let Some(hook) = &self.crdt_hook {
            if let Ok(mut touched) = self.crdt_touched.lock() {
                for (entity, id) in touched.drain(..) {
                    hook.on_rollback(&entity, &id);
                }
            }
        }
    }
}

impl<'a> PgTxStore<'a> {

    /// Run `body` against the held transaction. Centralizes the
    /// "lock the mutex, check we still hold a tx" preamble.
    fn with_tx<F, T>(&self, body: F) -> Result<T, DataError>
    where
        F: FnOnce(&mut postgres::Transaction<'a>) -> Result<T, DataError>,
    {
        let mut guard = self.tx.lock().map_err(|_| DataError {
            code: "TX_LOCK_POISONED".into(),
            message: "PgTxStore mutex poisoned".into(),
        })?;
        let tx = guard.as_mut().ok_or_else(|| DataError {
            code: "TX_CONSUMED".into(),
            message: "PgTxStore used after commit/rollback".into(),
        })?;
        body(tx)
    }
}

impl<'a> DataStore for PgTxStore<'a> {
    fn manifest(&self) -> &AppManifest {
        self.manifest
    }

    fn insert(&self, entity: &str, data: &serde_json::Value) -> Result<String, DataError> {
        let manifest = self.manifest;
        if self.entity_is_crdt(entity) {
            let hook = self.crdt_hook.as_ref().expect("entity_is_crdt implies hook present").clone();
            let id = self.with_tx(|tx| -> Result<String, DataError> {
                let projected_data = hook.before_insert(tx, entity, data)?;
                let row = projected_data.as_ref().unwrap_or(data);
                tx_insert(tx, manifest, entity, row)
            })?;
            self.record_crdt_touched(entity, &id);
            Ok(id)
        } else {
            self.with_tx(|tx| tx_insert(tx, manifest, entity, data))
        }
    }

    fn get_by_id(&self, entity: &str, id: &str) -> Result<Option<serde_json::Value>, DataError> {
        self.with_tx(|tx| tx_get_by_id(tx, entity, id))
    }

    fn list(&self, entity: &str) -> Result<Vec<serde_json::Value>, DataError> {
        let sql = format!(
            "SELECT * FROM {} ORDER BY id",
            quote_ident_pub(entity)
        );
        self.with_tx(|tx| {
            let rows = tx.query(sql.as_str(), &[]).map_err(pg_err_to_data)?;
            Ok(rows.iter().map(crate::postgres::row_to_json_pub).collect())
        })
    }

    fn list_after(
        &self,
        entity: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>, DataError> {
        let capped: i64 = limit.min(10_000) as i64;
        let qi = quote_ident_pub(entity);
        match after {
            Some(cursor) => self.with_tx(|tx| {
                let rows = tx
                    .query(
                        &format!("SELECT * FROM {qi} WHERE id > $1 ORDER BY id ASC LIMIT $2"),
                        &[&cursor, &capped],
                    )
                    .map_err(pg_err_to_data)?;
                Ok(rows.iter().map(crate::postgres::row_to_json_pub).collect())
            }),
            None => self.with_tx(|tx| {
                let rows = tx
                    .query(
                        &format!("SELECT * FROM {qi} ORDER BY id ASC LIMIT $1"),
                        &[&capped],
                    )
                    .map_err(pg_err_to_data)?;
                Ok(rows.iter().map(crate::postgres::row_to_json_pub).collect())
            }),
        }
    }

    fn update(&self, entity: &str, id: &str, data: &serde_json::Value) -> Result<bool, DataError> {
        let manifest = self.manifest;
        if self.entity_is_crdt(entity) {
            let hook = self.crdt_hook.as_ref().expect("entity_is_crdt implies hook present").clone();
            let updated = self.with_tx(|tx| -> Result<bool, DataError> {
                hook.before_update(tx, entity, id, data)?;
                let updated = tx_update(tx, manifest, entity, id, data)?;
                if !updated {
                    // Same orphan guard as Runtime::update — refuse
                    // to commit a snapshot for a row that doesn't
                    // exist. Rolls back via Err propagation.
                    return Err(DataError {
                        code: "ENTITY_NOT_FOUND".into(),
                        message: format!(
                            "Update on {entity}/{id} found no row — refusing to commit a CRDT \
                             snapshot that would orphan."
                        ),
                    });
                }
                Ok(updated)
            })?;
            self.record_crdt_touched(entity, id);
            Ok(updated)
        } else {
            self.with_tx(|tx| tx_update(tx, manifest, entity, id, data))
        }
    }

    fn delete(&self, entity: &str, id: &str) -> Result<bool, DataError> {
        let manifest = self.manifest;
        if self.entity_is_crdt(entity) {
            let hook = self.crdt_hook.as_ref().expect("entity_is_crdt implies hook present").clone();
            let deleted = self.with_tx(|tx| -> Result<bool, DataError> {
                hook.before_delete(tx, entity, id)?;
                tx_delete(tx, manifest, entity, id)
            })?;
            self.record_crdt_touched(entity, id);
            Ok(deleted)
        } else {
            self.with_tx(|tx| tx_delete(tx, manifest, entity, id))
        }
    }

    fn lookup(
        &self,
        entity: &str,
        field: &str,
        value: &str,
    ) -> Result<Option<serde_json::Value>, DataError> {
        // Validate the field against the manifest BEFORE quoting — same
        // gate the non-tx path uses (PostgresDataStore::lookup).
        let ent = self
            .manifest
            .entities
            .iter()
            .find(|e| e.name == entity)
            .ok_or_else(|| DataError {
                code: "ENTITY_NOT_FOUND".into(),
                message: format!("Unknown entity: \"{entity}\""),
            })?;
        if field != "id" && !ent.fields.iter().any(|f| f.name == field) {
            return Err(DataError {
                code: "UNKNOWN_COLUMN".into(),
                message: format!("Unknown column \"{field}\" on entity \"{entity}\""),
            });
        }
        let sql = format!(
            "SELECT * FROM {} WHERE {} = $1 LIMIT 1",
            quote_ident_pub(entity),
            quote_ident_pub(field),
        );
        self.with_tx(|tx| {
            let row = tx.query_opt(sql.as_str(), &[&value]).map_err(pg_err_to_data)?;
            Ok(row.map(|r| crate::postgres::row_to_json_pub(&r)))
        })
    }

    fn link(
        &self,
        entity: &str,
        id: &str,
        relation: &str,
        target_id: &str,
    ) -> Result<bool, DataError> {
        let ent = self
            .manifest
            .entities
            .iter()
            .find(|e| e.name == entity)
            .ok_or_else(|| DataError {
                code: "ENTITY_NOT_FOUND".into(),
                message: format!("Unknown entity: \"{entity}\""),
            })?;
        let rel = ent
            .relations
            .iter()
            .find(|r| r.name == relation)
            .ok_or_else(|| DataError {
                code: "RELATION_NOT_FOUND".into(),
                message: format!("Relation \"{relation}\" not found"),
            })?;
        let data = serde_json::json!({ rel.field.clone(): target_id });
        self.update(entity, id, &data)
    }

    fn unlink(&self, entity: &str, id: &str, relation: &str) -> Result<bool, DataError> {
        let ent = self
            .manifest
            .entities
            .iter()
            .find(|e| e.name == entity)
            .ok_or_else(|| DataError {
                code: "ENTITY_NOT_FOUND".into(),
                message: format!("Unknown entity: \"{entity}\""),
            })?;
        let rel = ent
            .relations
            .iter()
            .find(|r| r.name == relation)
            .ok_or_else(|| DataError {
                code: "RELATION_NOT_FOUND".into(),
                message: format!("Relation \"{relation}\" not found"),
            })?;
        let data = serde_json::json!({ rel.field.clone(): serde_json::Value::Null });
        self.update(entity, id, &data)
    }

    fn query_filtered(
        &self,
        entity: &str,
        filter: &serde_json::Value,
    ) -> Result<Vec<serde_json::Value>, DataError> {
        let ent = self
            .manifest
            .entities
            .iter()
            .find(|e| e.name == entity)
            .ok_or_else(|| DataError {
                code: "ENTITY_NOT_FOUND".into(),
                message: format!("Unknown entity: \"{entity}\""),
            })?;
        let columns: Vec<String> = ent.fields.iter().map(|f| f.name.clone()).collect();
        let (sql, values) =
            build_query_filtered_sql_pub(entity, filter, &columns).map_err(|e| DataError {
                code: e.code,
                message: e.message,
            })?;
        self.with_tx(|tx| {
            let params = as_pg_params(&values);
            let rows = tx.query(sql.as_str(), &params).map_err(pg_err_to_data)?;
            Ok(rows.iter().map(crate::postgres::row_to_json_pub).collect())
        })
    }

    fn query_graph(&self, query: &serde_json::Value) -> Result<serde_json::Value, DataError> {
        let obj = query.as_object().ok_or_else(|| DataError {
            code: "INVALID_QUERY".into(),
            message: "Graph query must be a JSON object".into(),
        })?;
        let mut results = serde_json::Map::new();
        for (entity_name, opts) in obj {
            let ent = self
                .manifest
                .entities
                .iter()
                .find(|e| e.name == *entity_name)
                .ok_or_else(|| DataError {
                    code: "ENTITY_NOT_FOUND".into(),
                    message: format!("Unknown entity: \"{entity_name}\""),
                })?;
            let filter = opts.get("where").cloned().unwrap_or(serde_json::json!({}));
            let rows = self.query_filtered(entity_name, &filter)?;
            let rows = if let Some(include) = opts.get("include").and_then(|v| v.as_object()) {
                rows.into_iter()
                    .map(|mut row| {
                        for (rel_name, _sub_query) in include {
                            if let Some(rel) = ent.relations.iter().find(|r| r.name == *rel_name) {
                                let fk_value = row
                                    .get(&rel.field)
                                    .and_then(|v| v.as_str())
                                    .map(|s| s.to_string());
                                if let Some(fk) = fk_value {
                                    if rel.many {
                                        let sub_filter = serde_json::json!({ &rel.field: &fk });
                                        if let Ok(related) =
                                            self.query_filtered(&rel.target, &sub_filter)
                                        {
                                            row[rel_name] = serde_json::json!(related);
                                        }
                                    } else if let Ok(Some(related)) =
                                        self.get_by_id(&rel.target, &fk)
                                    {
                                        row[rel_name] = related;
                                    }
                                }
                            }
                        }
                        row
                    })
                    .collect()
            } else {
                rows
            };
            let rows = if let Some(limit) = opts.get("limit").and_then(|v| v.as_u64()) {
                rows.into_iter().take(limit as usize).collect()
            } else {
                rows
            };
            results.insert(entity_name.clone(), serde_json::json!(rows));
        }
        Ok(serde_json::Value::Object(results))
    }

    fn aggregate(
        &self,
        entity: &str,
        spec: &serde_json::Value,
    ) -> Result<serde_json::Value, DataError> {
        // Aggregations inside a mutation handler run through the same
        // builder the non-tx path uses, executed via the held tx so
        // they see the handler's own pending writes (a count of just-
        // inserted rows, a sum that includes the new row, etc.) —
        // matches what users intuit from "the handler IS the tx".
        let ent = self
            .manifest
            .entities
            .iter()
            .find(|e| e.name == entity)
            .ok_or_else(|| DataError {
                code: "ENTITY_NOT_FOUND".into(),
                message: format!("Unknown entity: \"{entity}\""),
            })?;
        let columns: Vec<String> = ent.fields.iter().map(|f| f.name.clone()).collect();
        let (sql, values, column_names) =
            build_aggregate_sql_pub(entity, spec, &columns).map_err(|e| DataError {
                code: e.code,
                message: e.message,
            })?;
        self.with_tx(|tx| {
            let params = as_pg_params(&values);
            let rows = tx.query(sql.as_str(), &params).map_err(pg_err_to_data)?;
            Ok(aggregate_rows_to_json_pub(&rows, &column_names))
        })
    }

    fn transact(
        &self,
        _ops: &[serde_json::Value],
    ) -> Result<(bool, Vec<serde_json::Value>), DataError> {
        // Nested transactions aren't supported — the mutation handler
        // already IS the transaction. Same shape as the SQLite TxStore.
        Err(DataError {
            code: "NESTED_TRANSACTION".into(),
            message: "ctx.db.transact() is not allowed inside a mutation handler on any backend"
                .into(),
        })
    }
}
