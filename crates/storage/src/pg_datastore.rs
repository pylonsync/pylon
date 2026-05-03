//! [`DataStore`] implementation backed by Postgres.
//!
//! Only available with the `postgres-live` feature. Wraps
//! [`LivePostgresAdapter`] behind a mutex to match the synchronous
//! `DataStore` trait contract.
//!
//! SQL dialect differences from SQLite:
//! - Parameters use `$1, $2, ...` instead of `?1, ?2, ...`
//! - `id` column is `TEXT PRIMARY KEY`
//! - No `PRAGMA` statements
//!
//! For now, the advanced operations (lookup with field validation,
//! query_filtered with operators, query_graph, link, unlink, transact)
//! reuse the SQLite quote_ident strategy but issue Postgres-dialect SQL.

#![cfg(feature = "postgres-live")]

use std::sync::Mutex;

use pylon_http::{DataError, DataStore};
use pylon_kernel::AppManifest;

use crate::pg_tx_store;
use crate::postgres::live::LivePostgresAdapter;

/// Wrapper that implements `DataStore` around `LivePostgresAdapter`.
///
/// Writes go through a mutex to align with the sync trait. This matches
/// SQLite's single-writer model. For true concurrent writes under Postgres,
/// a future enhancement can use a connection pool.
pub struct PostgresDataStore {
    inner: Mutex<LivePostgresAdapter>,
    manifest: AppManifest,
}

impl PostgresDataStore {
    pub fn connect(url: &str, manifest: AppManifest) -> Result<Self, DataError> {
        let adapter = LivePostgresAdapter::connect(url).map_err(Self::map_err)?;
        Ok(Self {
            inner: Mutex::new(adapter),
            manifest,
        })
    }

    fn map_err(e: crate::StorageError) -> DataError {
        DataError {
            code: e.code.to_string(),
            message: e.message,
        }
    }

    /// Run `body` inside a real Postgres transaction. The closure
    /// receives a `&dyn DataStore` that routes every read/write
    /// through the transaction's connection — reads see uncommitted
    /// writes from earlier in the same closure, just like SQLite's
    /// in-handler behavior.
    ///
    /// Commit on `Ok`, rollback on `Err`. Errors from the transaction
    /// infrastructure itself (mutex poisoning, BEGIN failure, COMMIT
    /// failure) propagate as `E::from(DataError)`.
    ///
    /// Used by `pylon-runtime` to wire TS-function `Mutation`
    /// handlers through PG transactions — the SQLite path uses
    /// BEGIN/COMMIT on a held connection; this is the equivalent.
    /// The mutex is held for the entire closure body, so concurrent
    /// PG mutations serialize. Acceptable given mutation handlers
    /// are usually fast and pylon's PG sessions are single-shard.
    /// Borrow the underlying postgres client for the duration of the
    /// closure. Used for one-shot bootstrap work (e.g. creating the
    /// CRDT sidecar table on runtime open) that doesn't need an
    /// explicit transaction. Held connection mutex prevents concurrent
    /// PG ops from racing — same single-writer model as every other
    /// CRUD path here.
    pub fn with_client<F, T, E>(&self, body: F) -> Result<T, E>
    where
        F: FnOnce(&mut postgres::Client) -> Result<T, E>,
        E: From<DataError>,
    {
        let mut guard = self.inner.lock().map_err(|_| {
            E::from(DataError {
                code: "LOCK_POISONED".into(),
                message: "connection mutex poisoned".into(),
            })
        })?;
        body(guard.client_mut())
    }

    /// Run `body` inside a real Postgres transaction, exposing the
    /// raw `&mut Transaction` so the caller can interleave CRDT
    /// snapshot writes, materialized entity writes, and FTS shadow
    /// maintenance through the SAME transaction. This is the
    /// atomicity primitive the runtime layer needs — without it, the
    /// snapshot persists as one autocommit and the row write as
    /// another, and a failure between them desyncs the CRDT layer
    /// from the materialized columns.
    ///
    /// Commits on `Ok`, rolls back (via Drop) on `Err`. Errors from
    /// the transaction infrastructure itself (lock poisoning, BEGIN,
    /// COMMIT) propagate as `E::from(DataError)`.
    ///
    /// **Do not nest.** This holds the inner connection mutex for
    /// the whole closure. Calling `with_transaction`, `with_client`,
    /// or another `with_transaction_raw` from inside the closure (or
    /// from any code that runs while `body` is on the stack) will
    /// deadlock — `std::sync::Mutex` is not re-entrant. The runtime
    /// path enforces this by structure: the closures only call free
    /// functions in `pg_tx_store` and `pg_search` that take the
    /// `&mut Transaction` directly.
    pub fn with_transaction_raw<F, T, E>(&self, body: F) -> Result<T, E>
    where
        F: FnOnce(&mut postgres::Transaction<'_>) -> Result<T, E>,
        E: From<DataError>,
    {
        let mut guard = self.inner.lock().map_err(|_| {
            E::from(DataError {
                code: "LOCK_POISONED".into(),
                message: "connection mutex poisoned".into(),
            })
        })?;
        let mut tx = guard.client_mut().transaction().map_err(|e| {
            E::from(DataError {
                code: "PG_BEGIN_FAILED".into(),
                message: format!("BEGIN failed: {e}"),
            })
        })?;
        match body(&mut tx) {
            Ok(value) => {
                tx.commit().map_err(|e| {
                    E::from(DataError {
                        code: "PG_COMMIT_FAILED".into(),
                        message: format!("COMMIT failed: {e}"),
                    })
                })?;
                Ok(value)
            }
            Err(e) => {
                // Drop runs implicit ROLLBACK against the postgres
                // connection. Caller's Err propagates as-is.
                drop(tx);
                Err(e)
            }
        }
    }

    /// Run a search query against an entity's PG-side FTS shadow
    /// (`_fts_<entity>`). Held connection is locked for the duration
    /// of the three round-trips (hits / total / facet counts) — same
    /// serialization model as every other read here.
    pub fn run_search(
        &self,
        entity: &str,
        config: &crate::search::SearchConfig,
        query: &crate::search::SearchQuery,
    ) -> Result<crate::search::SearchResult, crate::StorageError> {
        let mut guard = self.inner.lock().map_err(|_| crate::StorageError {
            code: "LOCK_POISONED".into(),
            message: "connection mutex poisoned".into(),
        })?;
        crate::pg_search::run_search(guard.client_mut(), entity, config, query)
    }

    pub fn with_transaction<F, T, E>(&self, body: F) -> Result<T, E>
    where
        F: FnOnce(&dyn DataStore) -> Result<T, E>,
        E: From<DataError>,
    {
        self.with_transaction_inner(None, body)
    }

    /// Variant of `with_transaction` that installs a runtime-supplied
    /// CRDT hook on the underlying `PgTxStore`. Used by FnOpsImpl's
    /// PG mutation path so a TS handler's `ctx.db.X` calls on
    /// `crdt: true` entities maintain the CRDT sidecar in the same
    /// transaction. Without the hook, those writes commit the
    /// materialized row but skip the LoroDoc snapshot — codex
    /// flagged this as a desync vector.
    pub fn with_transaction_crdt<F, T, E>(
        &self,
        crdt_hook: std::sync::Arc<dyn pg_tx_store::PgCrdtHook>,
        body: F,
    ) -> Result<T, E>
    where
        F: FnOnce(&dyn DataStore) -> Result<T, E>,
        E: From<DataError>,
    {
        self.with_transaction_inner(Some(crdt_hook), body)
    }

    fn with_transaction_inner<F, T, E>(
        &self,
        crdt_hook: Option<std::sync::Arc<dyn pg_tx_store::PgCrdtHook>>,
        body: F,
    ) -> Result<T, E>
    where
        F: FnOnce(&dyn DataStore) -> Result<T, E>,
        E: From<DataError>,
    {
        let mut guard = self.inner.lock().map_err(|_| {
            E::from(DataError {
                code: "LOCK_POISONED".into(),
                message: "connection mutex poisoned".into(),
            })
        })?;
        let tx = guard.client_mut().transaction().map_err(|e| {
            E::from(DataError {
                code: "PG_BEGIN_FAILED".into(),
                message: format!("BEGIN failed: {e}"),
            })
        })?;
        let store = match crdt_hook {
            Some(hook) => pg_tx_store::PgTxStore::with_crdt(tx, &self.manifest, hook),
            None => pg_tx_store::PgTxStore::new(tx, &self.manifest),
        };
        match body(&store) {
            Ok(value) => {
                store.commit().map_err(|e| {
                    E::from(DataError {
                        code: "PG_COMMIT_FAILED".into(),
                        message: format!("COMMIT failed: {e}"),
                    })
                })?;
                Ok(value)
            }
            Err(e) => {
                // PgTxStore::Drop runs ROLLBACK + on_rollback hook.
                drop(store);
                Err(e)
            }
        }
    }
}

impl DataStore for PostgresDataStore {
    fn manifest(&self) -> &AppManifest {
        &self.manifest
    }

    fn insert(&self, entity: &str, data: &serde_json::Value) -> Result<String, DataError> {
        // Wrap CRUD + search maintenance in one transaction so a failure
        // on the FTS shadow row rolls the entity insert back too. Without
        // this the entity row would commit on its own and the FTS row
        // would lag — search results would silently miss the new row.
        self.with_transaction(|store| store.insert(entity, data))
    }

    fn get_by_id(&self, entity: &str, id: &str) -> Result<Option<serde_json::Value>, DataError> {
        let mut guard = self.inner.lock().map_err(|_| DataError {
            code: "LOCK_POISONED".into(),
            message: "connection mutex poisoned".into(),
        })?;
        guard.get_by_id(entity, id).map_err(Self::map_err)
    }

    fn list(&self, entity: &str) -> Result<Vec<serde_json::Value>, DataError> {
        let mut guard = self.inner.lock().map_err(|_| DataError {
            code: "LOCK_POISONED".into(),
            message: "connection mutex poisoned".into(),
        })?;
        guard.list(entity).map_err(Self::map_err)
    }

    fn list_after(
        &self,
        entity: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>, DataError> {
        let mut guard = self.inner.lock().map_err(|_| DataError {
            code: "LOCK_POISONED".into(),
            message: "connection mutex poisoned".into(),
        })?;
        guard
            .list_after(entity, after, limit)
            .map_err(Self::map_err)
    }

    fn update(&self, entity: &str, id: &str, data: &serde_json::Value) -> Result<bool, DataError> {
        // Same atomicity as `insert` — entity write + FTS rebuild share
        // one BEGIN/COMMIT.
        self.with_transaction(|store| store.update(entity, id, data))
    }

    fn delete(&self, entity: &str, id: &str) -> Result<bool, DataError> {
        // FK CASCADE on `_fts_<entity>.entity_id` would clean up
        // automatically, but PgTxStore::delete also runs the explicit
        // `apply_delete` so the maintenance contract matches SQLite.
        self.with_transaction(|store| store.delete(entity, id))
    }

    fn lookup(
        &self,
        entity: &str,
        field: &str,
        value: &str,
    ) -> Result<Option<serde_json::Value>, DataError> {
        // Validate the field against the manifest BEFORE quoting it into SQL.
        // Otherwise a caller could pass any string and we'd happily query an
        // arbitrary column (or get a Postgres error from a missing one).
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

        let mut guard = self.inner.lock().map_err(|_| DataError {
            code: "LOCK_POISONED".into(),
            message: "connection mutex poisoned".into(),
        })?;
        guard
            .lookup_field(entity, field, value)
            .map_err(Self::map_err)
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

    fn aggregate(
        &self,
        entity: &str,
        spec: &serde_json::Value,
    ) -> Result<serde_json::Value, DataError> {
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

        let mut guard = self.inner.lock().map_err(|_| DataError {
            code: "LOCK_POISONED".into(),
            message: "connection mutex poisoned".into(),
        })?;
        guard
            .aggregate(entity, spec, &columns)
            .map_err(Self::map_err)
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

        let mut guard = self.inner.lock().map_err(|_| DataError {
            code: "LOCK_POISONED".into(),
            message: "connection mutex poisoned".into(),
        })?;
        guard
            .query_filtered(entity, filter, &columns)
            .map_err(Self::map_err)
    }

    fn query_graph(&self, query: &serde_json::Value) -> Result<serde_json::Value, DataError> {
        let obj = query.as_object().ok_or_else(|| DataError {
            code: "INVALID_QUERY".into(),
            message: "Graph query must be a JSON object".into(),
        })?;
        let mut results = serde_json::Map::new();
        for (entity_name, opts) in obj {
            // Validate entity exists up front so the error matches the
            // SQLite path (`ENTITY_NOT_FOUND` not `PG_QUERY_FAILED`).
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

            // Apply `include` (relation expansion). One-to-many uses the
            // child side's FK; one-to-one / many-to-one calls get_by_id
            // on the target. Mirrors `Runtime::query_graph` so callers
            // see the same shape on both adapters.
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

            // Apply `limit` after expansion to match SQLite. Docs commit
            // to "the limit applies to the top-level rows," so trimming
            // pre-expansion would change semantics.
            let rows = if let Some(limit) = opts.get("limit").and_then(|v| v.as_u64()) {
                rows.into_iter().take(limit as usize).collect()
            } else {
                rows
            };

            results.insert(entity_name.clone(), serde_json::json!(rows));
        }
        Ok(serde_json::Value::Object(results))
    }

    fn transact(
        &self,
        ops: &[serde_json::Value],
    ) -> Result<(bool, Vec<serde_json::Value>), DataError> {
        use crate::pg_tx_store::{tx_delete, tx_insert, tx_update};

        // Validate every op shape up front so a malformed payload
        // doesn't open a tx and immediately roll back.
        #[derive(Clone)]
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

        // Run each op through the same `tx_*` helpers PgTxStore uses
        // — so FTS shadow rows + future maintenance hooks stay
        // consistent whether the batch arrived via /api/transact or
        // a TS mutation handler. Codex flagged that the previous path
        // bypassed FTS via `LivePostgresAdapter::transact`.
        let manifest = self.manifest.clone();
        self.with_transaction_raw(|tx| -> Result<(bool, Vec<serde_json::Value>), DataError> {
            let mut json_results: Vec<serde_json::Value> = Vec::with_capacity(typed.len());
            for op in &typed {
                let result = match op {
                    Op::Insert { entity, data } => {
                        let id = tx_insert(tx, &manifest, entity, data)?;
                        serde_json::json!({ "op": "insert", "id": id })
                    }
                    Op::Update { entity, id, data } => {
                        let updated = tx_update(tx, &manifest, entity, id, data)?;
                        serde_json::json!({ "op": "update", "id": id, "updated": updated })
                    }
                    Op::Delete { entity, id } => {
                        let deleted = tx_delete(tx, &manifest, entity, id)?;
                        serde_json::json!({ "op": "delete", "id": id, "deleted": deleted })
                    }
                };
                json_results.push(result);
            }
            Ok((true, json_results))
        })
    }
}
