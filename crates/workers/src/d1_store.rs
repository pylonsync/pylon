//! [`DataStore`] implementation backed by Cloudflare D1.
//!
//! D1 speaks SQLite SQL, so the SQL generation here mirrors
//! `pylon-storage::sqlite`. The `D1Executor` trait abstracts the actual
//! execution layer so this module stays free of `worker` crate dependencies.
//! The Workers fetch handler provides a concrete `D1Executor` that delegates
//! to the real D1 bindings.

use pylon_http::{DataError, DataStore};
use pylon_kernel::{AppManifest, ManifestEntity};
use serde_json::Value;

// ---------------------------------------------------------------------------
// Executor trait — abstracts D1 vs tests
// ---------------------------------------------------------------------------

/// Executes prepared SQL statements against D1.
///
/// Implementations bridge synchronous [`DataStore`] calls to D1's async API
/// by blocking in the Workers request context (which is single-threaded).
pub trait D1Executor: Send + Sync {
    /// Execute a statement that doesn't return rows. Returns affected row count.
    fn execute(&self, sql: &str, params: &[Value]) -> Result<u64, String>;

    /// Execute a query and return all matching rows as JSON objects.
    fn query(&self, sql: &str, params: &[Value]) -> Result<Vec<Value>, String>;

    /// Execute a query expecting at most one row.
    fn query_one(&self, sql: &str, params: &[Value]) -> Result<Option<Value>, String> {
        let rows = self.query(sql, params)?;
        Ok(rows.into_iter().next())
    }
}

// ---------------------------------------------------------------------------
// D1DataStore
// ---------------------------------------------------------------------------

pub struct D1DataStore<E: D1Executor> {
    executor: E,
    manifest: AppManifest,
}

impl<E: D1Executor> D1DataStore<E> {
    pub fn new(executor: E, manifest: AppManifest) -> Self {
        Self { executor, manifest }
    }

    fn entity(&self, name: &str) -> Result<&ManifestEntity, DataError> {
        self.manifest
            .entities
            .iter()
            .find(|e| e.name == name)
            .ok_or_else(|| DataError {
                code: "ENTITY_NOT_FOUND".into(),
                message: format!("Unknown entity: \"{name}\""),
            })
    }

    fn validate_column(&self, entity: &ManifestEntity, col: &str) -> Result<(), DataError> {
        if col == "id" || entity.fields.iter().any(|f| f.name == col) {
            Ok(())
        } else {
            Err(DataError {
                code: "INVALID_COLUMN".into(),
                message: format!("Unknown column \"{col}\" on entity \"{}\"", entity.name),
            })
        }
    }
}

use pylon_kernel::util::quote_ident;

fn generate_id() -> String {
    // Simple time-based ID. D1 runs in isolates with precise time.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{:x}", now)
}

// ---------------------------------------------------------------------------
// DataStore impl
// ---------------------------------------------------------------------------

impl<E: D1Executor> DataStore for D1DataStore<E> {
    fn manifest(&self) -> &AppManifest {
        &self.manifest
    }

    fn insert(&self, entity: &str, data: &Value) -> Result<String, DataError> {
        let ent = self.entity(entity)?;
        let obj = data.as_object().ok_or_else(|| DataError {
            code: "INVALID_DATA".into(),
            message: "Insert data must be a JSON object".into(),
        })?;

        let id = generate_id();
        let mut cols = vec![quote_ident("id")];
        let mut placeholders = vec!["?1".to_string()];
        let mut params: Vec<Value> = vec![Value::String(id.clone())];
        let mut idx = 2;

        for (k, v) in obj {
            if k == "id" {
                continue;
            }
            self.validate_column(ent, k)?;
            cols.push(quote_ident(k));
            placeholders.push(format!("?{idx}"));
            params.push(v.clone());
            idx += 1;
        }

        let sql = format!(
            "INSERT INTO {} ({}) VALUES ({})",
            quote_ident(entity),
            cols.join(", "),
            placeholders.join(", ")
        );

        self.executor
            .execute(&sql, &params)
            .map_err(|e| DataError {
                code: "INSERT_FAILED".into(),
                message: e,
            })?;

        Ok(id)
    }

    fn get_by_id(&self, entity: &str, id: &str) -> Result<Option<Value>, DataError> {
        let _ = self.entity(entity)?;
        let sql = format!(
            "SELECT * FROM {} WHERE \"id\" = ?1 LIMIT 1",
            quote_ident(entity)
        );
        self.executor
            .query_one(&sql, &[Value::String(id.to_string())])
            .map_err(|e| DataError {
                code: "QUERY_FAILED".into(),
                message: e,
            })
    }

    fn list(&self, entity: &str) -> Result<Vec<Value>, DataError> {
        let _ = self.entity(entity)?;
        let sql = format!("SELECT * FROM {} ORDER BY \"id\"", quote_ident(entity));
        self.executor.query(&sql, &[]).map_err(|e| DataError {
            code: "QUERY_FAILED".into(),
            message: e,
        })
    }

    fn list_after(
        &self,
        entity: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Value>, DataError> {
        let _ = self.entity(entity)?;
        let (sql, params): (String, Vec<Value>) = match after {
            Some(cursor) => (
                format!(
                    "SELECT * FROM {} WHERE \"id\" > ?1 ORDER BY \"id\" LIMIT ?2",
                    quote_ident(entity)
                ),
                vec![
                    Value::String(cursor.to_string()),
                    Value::Number((limit as u64).into()),
                ],
            ),
            None => (
                format!(
                    "SELECT * FROM {} ORDER BY \"id\" LIMIT ?1",
                    quote_ident(entity)
                ),
                vec![Value::Number((limit as u64).into())],
            ),
        };

        self.executor.query(&sql, &params).map_err(|e| DataError {
            code: "QUERY_FAILED".into(),
            message: e,
        })
    }

    fn update(&self, entity: &str, id: &str, data: &Value) -> Result<bool, DataError> {
        let ent = self.entity(entity)?;
        let obj = data.as_object().ok_or_else(|| DataError {
            code: "INVALID_DATA".into(),
            message: "Update data must be a JSON object".into(),
        })?;

        let mut sets = Vec::new();
        let mut params: Vec<Value> = Vec::new();
        let mut idx = 1;
        for (k, v) in obj {
            if k == "id" {
                continue;
            }
            self.validate_column(ent, k)?;
            sets.push(format!("{} = ?{idx}", quote_ident(k)));
            params.push(v.clone());
            idx += 1;
        }
        if sets.is_empty() {
            return Ok(false);
        }
        params.push(Value::String(id.to_string()));
        let sql = format!(
            "UPDATE {} SET {} WHERE \"id\" = ?{idx}",
            quote_ident(entity),
            sets.join(", ")
        );
        let affected = self
            .executor
            .execute(&sql, &params)
            .map_err(|e| DataError {
                code: "UPDATE_FAILED".into(),
                message: e,
            })?;
        Ok(affected > 0)
    }

    fn delete(&self, entity: &str, id: &str) -> Result<bool, DataError> {
        let _ = self.entity(entity)?;
        let sql = format!("DELETE FROM {} WHERE \"id\" = ?1", quote_ident(entity));
        let affected = self
            .executor
            .execute(&sql, &[Value::String(id.to_string())])
            .map_err(|e| DataError {
                code: "DELETE_FAILED".into(),
                message: e,
            })?;
        Ok(affected > 0)
    }

    fn lookup(&self, entity: &str, field: &str, value: &str) -> Result<Option<Value>, DataError> {
        let ent = self.entity(entity)?;
        self.validate_column(ent, field)?;
        let sql = format!(
            "SELECT * FROM {} WHERE {} = ?1 LIMIT 1",
            quote_ident(entity),
            quote_ident(field)
        );
        self.executor
            .query_one(&sql, &[Value::String(value.to_string())])
            .map_err(|e| DataError {
                code: "QUERY_FAILED".into(),
                message: e,
            })
    }

    fn link(
        &self,
        entity: &str,
        id: &str,
        relation: &str,
        target_id: &str,
    ) -> Result<bool, DataError> {
        let ent = self.entity(entity)?;
        let rel = ent
            .relations
            .iter()
            .find(|r| r.name == relation)
            .ok_or_else(|| DataError {
                code: "RELATION_NOT_FOUND".into(),
                message: format!("Relation \"{relation}\" not found on \"{entity}\""),
            })?;
        let data = serde_json::json!({ rel.field.clone(): target_id });
        self.update(entity, id, &data)
    }

    fn unlink(&self, entity: &str, id: &str, relation: &str) -> Result<bool, DataError> {
        let ent = self.entity(entity)?;
        let rel = ent
            .relations
            .iter()
            .find(|r| r.name == relation)
            .ok_or_else(|| DataError {
                code: "RELATION_NOT_FOUND".into(),
                message: format!("Relation \"{relation}\" not found on \"{entity}\""),
            })?;
        let data = serde_json::json!({ rel.field.clone(): Value::Null });
        self.update(entity, id, &data)
    }

    fn query_filtered(&self, entity: &str, filter: &Value) -> Result<Vec<Value>, DataError> {
        let ent = self.entity(entity)?;
        let empty = serde_json::Map::new();
        let obj = filter.as_object().unwrap_or(&empty);

        let mut where_clauses: Vec<String> = Vec::new();
        let mut params: Vec<Value> = Vec::new();
        let mut order_clause = String::new();
        let mut limit_clause = String::new();
        let mut idx = 1;

        for (k, v) in obj {
            match k.as_str() {
                "$order" => {
                    if let Some(o) = v.as_object() {
                        let mut parts = Vec::new();
                        for (col, dir) in o {
                            self.validate_column(ent, col)?;
                            let d = match dir.as_str().unwrap_or("asc") {
                                "desc" | "DESC" => "DESC",
                                _ => "ASC",
                            };
                            parts.push(format!("{} {d}", quote_ident(col)));
                        }
                        if !parts.is_empty() {
                            order_clause = format!(" ORDER BY {}", parts.join(", "));
                        }
                    }
                }
                "$limit" => {
                    if let Some(n) = v.as_u64() {
                        limit_clause = format!(" LIMIT {n}");
                    }
                }
                _ => {
                    self.validate_column(ent, k)?;
                    let qk = quote_ident(k);
                    if let Some(op_obj) = v.as_object() {
                        for (op, op_val) in op_obj {
                            match op.as_str() {
                                "$not" => {
                                    where_clauses.push(format!("{qk} != ?{idx}"));
                                    params.push(op_val.clone());
                                    idx += 1;
                                }
                                "$gt" => {
                                    where_clauses.push(format!("{qk} > ?{idx}"));
                                    params.push(op_val.clone());
                                    idx += 1;
                                }
                                "$gte" => {
                                    where_clauses.push(format!("{qk} >= ?{idx}"));
                                    params.push(op_val.clone());
                                    idx += 1;
                                }
                                "$lt" => {
                                    where_clauses.push(format!("{qk} < ?{idx}"));
                                    params.push(op_val.clone());
                                    idx += 1;
                                }
                                "$lte" => {
                                    where_clauses.push(format!("{qk} <= ?{idx}"));
                                    params.push(op_val.clone());
                                    idx += 1;
                                }
                                "$like" => {
                                    where_clauses.push(format!("{qk} LIKE ?{idx}"));
                                    let pattern = format!("%{}%", op_val.as_str().unwrap_or(""));
                                    params.push(Value::String(pattern));
                                    idx += 1;
                                }
                                "$in" => {
                                    if let Some(arr) = op_val.as_array() {
                                        let ph: Vec<String> = arr
                                            .iter()
                                            .map(|v| {
                                                let p = format!("?{idx}");
                                                params.push(v.clone());
                                                idx += 1;
                                                p
                                            })
                                            .collect();
                                        if !ph.is_empty() {
                                            where_clauses
                                                .push(format!("{qk} IN ({})", ph.join(", ")));
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    } else {
                        where_clauses.push(format!("{qk} = ?{idx}"));
                        params.push(v.clone());
                        idx += 1;
                    }
                }
            }
        }

        let where_sql = if where_clauses.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", where_clauses.join(" AND "))
        };
        if order_clause.is_empty() {
            order_clause = " ORDER BY \"id\"".into();
        }

        let sql = format!(
            "SELECT * FROM {}{}{}{}",
            quote_ident(entity),
            where_sql,
            order_clause,
            limit_clause
        );

        self.executor.query(&sql, &params).map_err(|e| DataError {
            code: "QUERY_FAILED".into(),
            message: e,
        })
    }

    fn query_graph(&self, query: &Value) -> Result<Value, DataError> {
        let obj = query.as_object().ok_or_else(|| DataError {
            code: "INVALID_QUERY".into(),
            message: "Graph query must be a JSON object".into(),
        })?;
        let mut results = serde_json::Map::new();
        for (entity_name, opts) in obj {
            let filter = opts.get("where").cloned().unwrap_or(serde_json::json!({}));
            let rows = self.query_filtered(entity_name, &filter)?;
            results.insert(entity_name.clone(), Value::Array(rows));
        }
        Ok(Value::Object(results))
    }

    fn transact(&self, ops: &[Value]) -> Result<(bool, Vec<Value>), DataError> {
        // D1 doesn't have real transactions from the Worker API — it has
        // a batch API that runs in a single trip. For now, execute ops
        // sequentially and short-circuit on error (no real rollback).
        let mut results = Vec::new();
        let mut rollback = false;
        for op in ops {
            let op_type = op.get("op").and_then(|v| v.as_str()).unwrap_or("");
            let entity = op.get("entity").and_then(|v| v.as_str()).unwrap_or("");
            match op_type {
                "insert" => {
                    let data = op.get("data").cloned().unwrap_or(serde_json::json!({}));
                    match self.insert(entity, &data) {
                        Ok(id) => results.push(serde_json::json!({"op":"insert","id":id})),
                        Err(e) => {
                            rollback = true;
                            results.push(serde_json::json!({"op":"insert","error":e.message}));
                            break;
                        }
                    }
                }
                "update" => {
                    let id = op.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    let data = op.get("data").cloned().unwrap_or(serde_json::json!({}));
                    match self.update(entity, id, &data) {
                        Ok(_) => results.push(serde_json::json!({"op":"update","id":id})),
                        Err(e) => {
                            rollback = true;
                            results.push(serde_json::json!({"op":"update","error":e.message}));
                            break;
                        }
                    }
                }
                "delete" => {
                    let id = op.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    match self.delete(entity, id) {
                        Ok(_) => results.push(serde_json::json!({"op":"delete","id":id})),
                        Err(e) => {
                            rollback = true;
                            results.push(serde_json::json!({"op":"delete","error":e.message}));
                            break;
                        }
                    }
                }
                _ => {
                    results.push(serde_json::json!({"op":op_type,"error":"unknown operation"}));
                }
            }
        }
        Ok((!rollback, results))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Minimal in-memory executor for unit tests.
    struct MockExecutor {
        rows: Mutex<Vec<Value>>,
    }

    impl D1Executor for MockExecutor {
        fn execute(&self, _sql: &str, _params: &[Value]) -> Result<u64, String> {
            Ok(1)
        }
        fn query(&self, _sql: &str, _params: &[Value]) -> Result<Vec<Value>, String> {
            Ok(self.rows.lock().unwrap().clone())
        }
    }

    fn empty_manifest() -> AppManifest {
        AppManifest {
            manifest_version: pylon_kernel::MANIFEST_VERSION,
            name: "t".into(),
            version: "0".into(),
            entities: vec![ManifestEntity {
                name: "Lot".into(),
                fields: vec![pylon_kernel::ManifestField {
                    name: "title".into(),
                    field_type: "string".into(),
                    optional: false,
                    unique: false,
                }],
                indexes: vec![],
                relations: vec![],
            }],
            routes: vec![],
            queries: vec![],
            actions: vec![],
            policies: vec![],
        }
    }

    #[test]
    fn d1_insert_generates_id() {
        let exec = MockExecutor {
            rows: Mutex::new(vec![]),
        };
        let store = D1DataStore::new(exec, empty_manifest());
        let id = store
            .insert("Lot", &serde_json::json!({"title": "Test"}))
            .unwrap();
        assert!(!id.is_empty());
    }

    #[test]
    fn d1_list_returns_rows() {
        let exec = MockExecutor {
            rows: Mutex::new(vec![serde_json::json!({"id":"a","title":"T"})]),
        };
        let store = D1DataStore::new(exec, empty_manifest());
        let rows = store.list("Lot").unwrap();
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn d1_rejects_unknown_entity() {
        let exec = MockExecutor {
            rows: Mutex::new(vec![]),
        };
        let store = D1DataStore::new(exec, empty_manifest());
        let err = store.list("Nope").unwrap_err();
        assert_eq!(err.code, "ENTITY_NOT_FOUND");
    }

    #[test]
    fn d1_rejects_unknown_column() {
        let exec = MockExecutor {
            rows: Mutex::new(vec![]),
        };
        let store = D1DataStore::new(exec, empty_manifest());
        let err = store
            .insert("Lot", &serde_json::json!({"evil": "x"}))
            .unwrap_err();
        assert_eq!(err.code, "INVALID_COLUMN");
    }
}
