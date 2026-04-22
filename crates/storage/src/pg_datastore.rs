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

use statecraft_core::AppManifest;
use statecraft_http::{DataError, DataStore};

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
}

impl DataStore for PostgresDataStore {
    fn manifest(&self) -> &AppManifest {
        &self.manifest
    }

    fn insert(&self, entity: &str, data: &serde_json::Value) -> Result<String, DataError> {
        let mut guard = self.inner.lock().map_err(|_| DataError {
            code: "LOCK_POISONED".into(),
            message: "connection mutex poisoned".into(),
        })?;
        guard.insert(entity, data).map_err(Self::map_err)
    }

    fn get_by_id(
        &self,
        entity: &str,
        id: &str,
    ) -> Result<Option<serde_json::Value>, DataError> {
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
        guard.list_after(entity, after, limit).map_err(Self::map_err)
    }

    fn update(
        &self,
        entity: &str,
        id: &str,
        data: &serde_json::Value,
    ) -> Result<bool, DataError> {
        let mut guard = self.inner.lock().map_err(|_| DataError {
            code: "LOCK_POISONED".into(),
            message: "connection mutex poisoned".into(),
        })?;
        guard.update(entity, id, data).map_err(Self::map_err)
    }

    fn delete(&self, entity: &str, id: &str) -> Result<bool, DataError> {
        let mut guard = self.inner.lock().map_err(|_| DataError {
            code: "LOCK_POISONED".into(),
            message: "connection mutex poisoned".into(),
        })?;
        guard.delete(entity, id).map_err(Self::map_err)
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
        guard.lookup_field(entity, field, value).map_err(Self::map_err)
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
        let rel = ent.relations.iter().find(|r| r.name == relation).ok_or_else(|| {
            DataError {
                code: "RELATION_NOT_FOUND".into(),
                message: format!("Relation \"{relation}\" not found"),
            }
        })?;
        let data = serde_json::json!({ rel.field.clone(): target_id });
        self.update(entity, id, &data)
    }

    fn unlink(
        &self,
        entity: &str,
        id: &str,
        relation: &str,
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
        let rel = ent.relations.iter().find(|r| r.name == relation).ok_or_else(|| {
            DataError {
                code: "RELATION_NOT_FOUND".into(),
                message: format!("Relation \"{relation}\" not found"),
            }
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

        let mut guard = self.inner.lock().map_err(|_| DataError {
            code: "LOCK_POISONED".into(),
            message: "connection mutex poisoned".into(),
        })?;
        guard
            .query_filtered(entity, filter, &columns)
            .map_err(Self::map_err)
    }

    fn query_graph(
        &self,
        query: &serde_json::Value,
    ) -> Result<serde_json::Value, DataError> {
        let obj = query.as_object().ok_or_else(|| DataError {
            code: "INVALID_QUERY".into(),
            message: "Graph query must be a JSON object".into(),
        })?;
        let mut results = serde_json::Map::new();
        for (entity_name, opts) in obj {
            let filter = opts.get("where").cloned().unwrap_or(serde_json::json!({}));
            let rows = self.query_filtered(entity_name, &filter)?;
            results.insert(entity_name.clone(), serde_json::json!(rows));
        }
        Ok(serde_json::Value::Object(results))
    }

    fn transact(
        &self,
        ops: &[serde_json::Value],
    ) -> Result<(bool, Vec<serde_json::Value>), DataError> {
        use crate::postgres::live::{TxOp, TxResult};

        // Translate JSON ops -> typed TxOp variants. Reject the whole batch if
        // any op is malformed — a bad op should never quietly succeed.
        let mut typed: Vec<TxOp<'_>> = Vec::with_capacity(ops.len());
        for op in ops {
            let op_type = op.get("op").and_then(|v| v.as_str()).unwrap_or("");
            let entity = op.get("entity").and_then(|v| v.as_str()).ok_or_else(|| DataError {
                code: "TX_INVALID_OP".into(),
                message: "Each transact op must have an \"entity\" field".into(),
            })?;
            match op_type {
                "insert" => {
                    let data = op.get("data").ok_or_else(|| DataError {
                        code: "TX_INVALID_OP".into(),
                        message: "insert op requires \"data\"".into(),
                    })?;
                    typed.push(TxOp::Insert { entity, data });
                }
                "update" => {
                    let id = op.get("id").and_then(|v| v.as_str()).ok_or_else(|| DataError {
                        code: "TX_INVALID_OP".into(),
                        message: "update op requires \"id\"".into(),
                    })?;
                    let data = op.get("data").ok_or_else(|| DataError {
                        code: "TX_INVALID_OP".into(),
                        message: "update op requires \"data\"".into(),
                    })?;
                    typed.push(TxOp::Update { entity, id, data });
                }
                "delete" => {
                    let id = op.get("id").and_then(|v| v.as_str()).ok_or_else(|| DataError {
                        code: "TX_INVALID_OP".into(),
                        message: "delete op requires \"id\"".into(),
                    })?;
                    typed.push(TxOp::Delete { entity, id });
                }
                other => {
                    return Err(DataError {
                        code: "TX_INVALID_OP".into(),
                        message: format!("unknown op \"{other}\""),
                    });
                }
            }
        }

        let mut guard = self.inner.lock().map_err(|_| DataError {
            code: "LOCK_POISONED".into(),
            message: "connection mutex poisoned".into(),
        })?;

        // Real Postgres transaction — Transaction::commit() commits, Drop rolls
        // back. So if anything fails mid-batch the ROLLBACK is automatic.
        let pg_results = guard.transact(&typed).map_err(Self::map_err)?;

        let json_results: Vec<serde_json::Value> = typed
            .iter()
            .zip(pg_results.iter())
            .map(|(op, r)| match (op, r) {
                (TxOp::Insert { .. }, TxResult::Inserted(id)) => {
                    serde_json::json!({"op":"insert","id":id})
                }
                (TxOp::Update { id, .. }, TxResult::Updated(found)) => {
                    serde_json::json!({"op":"update","id":id,"updated":found})
                }
                (TxOp::Delete { id, .. }, TxResult::Deleted(found)) => {
                    serde_json::json!({"op":"delete","id":id,"deleted":found})
                }
                _ => serde_json::json!({"error":"result/op mismatch"}),
            })
            .collect();

        Ok((true, json_results))
    }
}
