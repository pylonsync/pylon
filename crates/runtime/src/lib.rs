pub mod cache_handlers;
pub mod cache_server;
pub mod cron;
pub mod job_store;
pub mod jobs;
pub mod log;
pub mod metrics;
pub mod openapi;
pub mod presence;
pub mod pubsub;
pub mod rate_limit;
pub mod resp;
pub mod resp_server;
pub mod rooms;
pub mod scheduler;
pub mod server;
pub mod sse;
pub mod tls;
pub mod workflow_store;
pub mod workflows;
pub mod ws;

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use agentdb_core::{AppManifest, ManifestEntity};
use rusqlite::Connection;

// ---------------------------------------------------------------------------
// Runtime errors
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct RuntimeError {
    pub code: String,
    pub message: String,
}

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for RuntimeError {}

// ---------------------------------------------------------------------------
// SQL safety helpers
// ---------------------------------------------------------------------------

/// Quote a SQL identifier with double quotes to prevent injection.
/// Any embedded double quotes are escaped by doubling them (SQL standard).
fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Validate that `name` is a known column on the given entity.
/// Always allows "id" (the primary key). Returns an error listing valid
/// columns when validation fails.
fn validate_column_name(name: &str, entity: &ManifestEntity) -> Result<(), RuntimeError> {
    if name == "id" {
        return Ok(());
    }
    if entity.fields.iter().any(|f| f.name == name) {
        return Ok(());
    }
    Err(RuntimeError {
        code: "INVALID_COLUMN".into(),
        message: format!(
            "Unknown column \"{}\" -- valid columns: id, {}",
            name,
            entity
                .fields
                .iter()
                .map(|f| f.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    })
}

// ---------------------------------------------------------------------------
// Read connection guard
// ---------------------------------------------------------------------------

/// A guard that dereferences to a `Connection`, abstracting over whether
/// it came from the read pool or fell back to the write connection.
enum ReadConnGuard<'a> {
    Pooled(std::sync::MutexGuard<'a, Connection>),
    Write(std::sync::MutexGuard<'a, Connection>),
}

impl<'a> std::ops::Deref for ReadConnGuard<'a> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        match self {
            ReadConnGuard::Pooled(g) => g,
            ReadConnGuard::Write(g) => g,
        }
    }
}

// ---------------------------------------------------------------------------
// Runtime — the core execution engine
// ---------------------------------------------------------------------------

/// A minimal runtime that executes CRUD operations against a SQLite database
/// based on a manifest contract.
///
/// In WAL mode SQLite allows one writer and multiple concurrent readers.
/// This struct exploits that by keeping a single write connection behind a
/// mutex and a pool of read-only connections that can be acquired in
/// parallel, so read operations never block on (or are blocked by) writes.
pub struct Runtime {
    /// Write connection — single mutex, serializes writes.
    write_conn: Mutex<Connection>,
    /// Read connections — pool of connections for concurrent reads.
    /// Empty for in-memory databases where extra connections are not possible.
    read_pool: Vec<Mutex<Connection>>,
    /// Counter for round-robin read pool selection.
    read_counter: AtomicUsize,
    manifest: AppManifest,
    entities: HashMap<String, ManifestEntity>,
}

/// Number of read-only connections to open in the pool.
const READ_POOL_SIZE: usize = 4;

impl Runtime {
    /// Open a runtime against an existing SQLite database.
    pub fn open(db_path: &str, manifest: AppManifest) -> Result<Self, RuntimeError> {
        let conn = Connection::open(db_path).map_err(|e| RuntimeError {
            code: "RUNTIME_OPEN_FAILED".into(),
            message: format!("Failed to open database: {e}"),
        })?;
        Self::from_connection(conn, manifest)
    }

    /// Create an in-memory runtime (useful for tests and benchmarks).
    pub fn in_memory(manifest: AppManifest) -> Result<Self, RuntimeError> {
        let conn = Connection::open_in_memory().map_err(|e| RuntimeError {
            code: "RUNTIME_OPEN_FAILED".into(),
            message: format!("Failed to open in-memory database: {e}"),
        })?;
        Self::from_connection(conn, manifest)
    }

    fn from_connection(conn: Connection, manifest: AppManifest) -> Result<Self, RuntimeError> {
        // Enable WAL mode for better concurrency.
        conn.execute_batch("PRAGMA journal_mode=WAL;").ok();

        // Build entity lookup map.
        let entities: HashMap<String, ManifestEntity> = manifest
            .entities
            .iter()
            .map(|e| (e.name.clone(), e.clone()))
            .collect();

        // Create tables for all entities.
        for entity in &manifest.entities {
            let fields: Vec<String> = entity
                .fields
                .iter()
                .map(|f| {
                    let col_type = match f.field_type.as_str() {
                        "int" => "INTEGER",
                        "float" => "REAL",
                        "bool" => "INTEGER",
                        _ => "TEXT",
                    };
                    let not_null = if f.optional { "" } else { " NOT NULL" };
                    let unique = if f.unique { " UNIQUE" } else { "" };
                    format!("{} {col_type}{not_null}{unique}", quote_ident(&f.name))
                })
                .collect();

            let mut cols = vec!["\"id\" TEXT PRIMARY KEY NOT NULL".to_string()];
            cols.extend(fields);
            let sql = format!(
                "CREATE TABLE IF NOT EXISTS {} ({})",
                quote_ident(&entity.name),
                cols.join(", ")
            );
            conn.execute(&sql, []).map_err(|e| RuntimeError {
                code: "SCHEMA_INIT_FAILED".into(),
                message: format!("Failed to create table {}: {e}", entity.name),
            })?;

            // Create indexes.
            for idx in &entity.indexes {
                let unique_kw = if idx.unique { "UNIQUE " } else { "" };
                let quoted_fields: Vec<String> =
                    idx.fields.iter().map(|f| quote_ident(f)).collect();
                let idx_sql = format!(
                    "CREATE {unique_kw}INDEX IF NOT EXISTS {} ON {} ({})",
                    quote_ident(&idx.name),
                    quote_ident(&entity.name),
                    quoted_fields.join(", ")
                );
                conn.execute(&idx_sql, []).ok();
            }
        }

        // Open read-only connection pool for file-backed databases.
        // In-memory databases cannot share connections, so the pool stays empty
        // and reads fall back to the write connection.
        let db_path = conn.path().filter(|p| !p.is_empty()).map(|p| p.to_string());

        let read_pool = if let Some(ref path) = db_path {
            let mut pool = Vec::with_capacity(READ_POOL_SIZE);
            for _ in 0..READ_POOL_SIZE {
                let read_conn = Connection::open_with_flags(
                    path,
                    rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
                        | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
                )
                .map_err(|e| RuntimeError {
                    code: "POOL_OPEN_FAILED".into(),
                    message: format!("Failed to open read connection: {e}"),
                })?;
                read_conn.execute_batch("PRAGMA journal_mode=WAL;").ok();
                pool.push(Mutex::new(read_conn));
            }
            pool
        } else {
            // In-memory DB — no separate read connections possible.
            Vec::new()
        };

        Ok(Self {
            write_conn: Mutex::new(conn),
            read_pool,
            read_counter: AtomicUsize::new(0),
            manifest,
            entities,
        })
    }

    /// Return a reference to the app manifest.
    pub fn manifest(&self) -> &AppManifest {
        &self.manifest
    }

    /// Expose the write connection mutex for transactional operations.
    pub fn lock_conn_pub(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, Connection>, RuntimeError> {
        self.lock_write_conn()
    }

    /// Return the number of read connections in the pool (0 for in-memory DBs).
    pub fn read_pool_size(&self) -> usize {
        self.read_pool.len()
    }

    // -----------------------------------------------------------------------
    // CRUD operations
    // -----------------------------------------------------------------------

    /// Insert a new row. Returns the generated ID.
    pub fn insert(&self, entity: &str, data: &serde_json::Value) -> Result<String, RuntimeError> {
        let ent = self.require_entity(entity)?;
        let conn = self.lock_write_conn()?;

        let id = generate_id();

        let obj = data.as_object().ok_or_else(|| RuntimeError {
            code: "INVALID_DATA".into(),
            message: "Insert data must be a JSON object".into(),
        })?;

        let mut col_names = vec![quote_ident("id")];
        let mut placeholders = vec!["?1".to_string()];
        let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(id.clone())];

        let mut idx = 2;
        for (key, val) in obj {
            if key == "id" {
                continue;
            }
            validate_column_name(key, ent)?;
            col_names.push(quote_ident(key));
            placeholders.push(format!("?{idx}"));
            values.push(json_to_sql(val));
            idx += 1;
        }

        let sql = format!(
            "INSERT INTO {} ({}) VALUES ({})",
            quote_ident(entity),
            col_names.join(", "),
            placeholders.join(", ")
        );

        let params: Vec<&dyn rusqlite::types::ToSql> = values.iter().map(|v| v.as_ref()).collect();
        conn.execute(&sql, params.as_slice()).map_err(|e| RuntimeError {
            code: "INSERT_FAILED".into(),
            message: format!("Insert into {entity} failed: {e}"),
        })?;

        Ok(id)
    }

    /// Get a single row by ID.
    pub fn get_by_id(
        &self,
        entity: &str,
        id: &str,
    ) -> Result<Option<serde_json::Value>, RuntimeError> {
        let ent = self.require_entity(entity)?;
        let conn = self.lock_read_conn()?;

        let sql = format!(
            "SELECT * FROM {} WHERE \"id\" = ?1",
            quote_ident(entity)
        );
        let mut stmt = conn.prepare(&sql).map_err(|e| RuntimeError {
            code: "QUERY_FAILED".into(),
            message: format!("Failed to prepare query: {e}"),
        })?;

        let columns: Vec<String> = ent
            .fields
            .iter()
            .map(|f| f.name.clone())
            .collect();

        let result = stmt
            .query_row(rusqlite::params![id], |row| {
                Ok(row_to_json(row, &columns))
            })
            .ok();

        Ok(result)
    }

    /// List all rows for an entity.
    pub fn list(&self, entity: &str) -> Result<Vec<serde_json::Value>, RuntimeError> {
        let ent = self.require_entity(entity)?;
        let conn = self.lock_read_conn()?;

        let sql = format!(
            "SELECT * FROM {} ORDER BY \"id\"",
            quote_ident(entity)
        );
        let mut stmt = conn.prepare(&sql).map_err(|e| RuntimeError {
            code: "QUERY_FAILED".into(),
            message: format!("Failed to prepare query: {e}"),
        })?;

        let columns: Vec<String> = ent.fields.iter().map(|f| f.name.clone()).collect();

        let rows = stmt
            .query_map([], |row| Ok(row_to_json(row, &columns)))
            .map_err(|e| RuntimeError {
                code: "QUERY_FAILED".into(),
                message: format!("Query failed: {e}"),
            })?;

        let mut result = Vec::new();
        for row in rows {
            if let Ok(val) = row {
                result.push(val);
            }
        }
        Ok(result)
    }

    /// List rows after a cursor ID (for cursor-based pagination).
    pub fn list_after(
        &self,
        entity: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>, RuntimeError> {
        let ent = self.require_entity(entity)?;
        let conn = self.lock_read_conn()?;

        let columns: Vec<String> = ent.fields.iter().map(|f| f.name.clone()).collect();
        let table = quote_ident(entity);

        let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match after {
            Some(cursor) => (
                format!(
                    "SELECT * FROM {} WHERE \"id\" > ?1 ORDER BY \"id\" LIMIT ?2",
                    table
                ),
                vec![
                    Box::new(cursor.to_string()),
                    Box::new(limit as i64),
                ],
            ),
            None => (
                format!(
                    "SELECT * FROM {} ORDER BY \"id\" LIMIT ?1",
                    table
                ),
                vec![Box::new(limit as i64)],
            ),
        };

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|v| v.as_ref()).collect();

        let mut stmt = conn.prepare(&sql).map_err(|e| RuntimeError {
            code: "QUERY_FAILED".into(),
            message: format!("Failed to prepare query: {e}"),
        })?;

        let rows = stmt
            .query_map(param_refs.as_slice(), |row| Ok(row_to_json(row, &columns)))
            .map_err(|e| RuntimeError {
                code: "QUERY_FAILED".into(),
                message: format!("Query failed: {e}"),
            })?;

        let mut result = Vec::new();
        for row in rows {
            if let Ok(val) = row {
                result.push(val);
            }
        }
        Ok(result)
    }

    /// Update a row by ID. Returns true if a row was found and updated.
    pub fn update(
        &self,
        entity: &str,
        id: &str,
        data: &serde_json::Value,
    ) -> Result<bool, RuntimeError> {
        let ent = self.require_entity(entity)?;
        let conn = self.lock_write_conn()?;

        let obj = data.as_object().ok_or_else(|| RuntimeError {
            code: "INVALID_DATA".into(),
            message: "Update data must be a JSON object".into(),
        })?;

        let mut set_clauses = Vec::new();
        let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        let mut idx = 1;
        for (key, val) in obj {
            if key == "id" {
                continue;
            }
            validate_column_name(key, ent)?;
            set_clauses.push(format!("{} = ?{idx}", quote_ident(key)));
            values.push(json_to_sql(val));
            idx += 1;
        }

        if set_clauses.is_empty() {
            return Ok(false);
        }

        values.push(Box::new(id.to_string()));
        let sql = format!(
            "UPDATE {} SET {} WHERE \"id\" = ?{idx}",
            quote_ident(entity),
            set_clauses.join(", ")
        );

        let params: Vec<&dyn rusqlite::types::ToSql> = values.iter().map(|v| v.as_ref()).collect();
        let affected = conn.execute(&sql, params.as_slice()).map_err(|e| RuntimeError {
            code: "UPDATE_FAILED".into(),
            message: format!("Update {entity}/{id} failed: {e}"),
        })?;

        Ok(affected > 0)
    }

    /// Delete a row by ID. Returns true if a row was actually deleted.
    pub fn delete(&self, entity: &str, id: &str) -> Result<bool, RuntimeError> {
        let _ent = self.require_entity(entity)?;
        let conn = self.lock_write_conn()?;

        let sql = format!(
            "DELETE FROM {} WHERE \"id\" = ?1",
            quote_ident(entity)
        );
        let affected = conn.execute(&sql, rusqlite::params![id]).map_err(|e| RuntimeError {
            code: "DELETE_FAILED".into(),
            message: format!("Delete {entity}/{id} failed: {e}"),
        })?;

        Ok(affected > 0)
    }

    /// Lookup a single row by a field value (e.g., email).
    pub fn lookup(
        &self,
        entity: &str,
        field: &str,
        value: &str,
    ) -> Result<Option<serde_json::Value>, RuntimeError> {
        let ent = self.require_entity(entity)?;
        validate_column_name(field, ent)?;
        let conn = self.lock_read_conn()?;

        let sql = format!(
            "SELECT * FROM {} WHERE {} = ?1 LIMIT 1",
            quote_ident(entity),
            quote_ident(field)
        );
        let columns: Vec<String> = ent.fields.iter().map(|f| f.name.clone()).collect();

        let result = conn
            .prepare(&sql)
            .ok()
            .and_then(|mut stmt| {
                stmt.query_row(rusqlite::params![value], |row| {
                    Ok(row_to_json(row, &columns))
                })
                .ok()
            });

        Ok(result)
    }

    /// Link two entities by setting a foreign-key field.
    pub fn link(
        &self,
        entity: &str,
        id: &str,
        relation: &str,
        target_id: &str,
    ) -> Result<bool, RuntimeError> {
        let ent = self.require_entity(entity)?;

        // Find the relation definition to determine which field to set.
        let rel = ent
            .relations
            .iter()
            .find(|r| r.name == relation)
            .ok_or_else(|| RuntimeError {
                code: "RELATION_NOT_FOUND".into(),
                message: format!("Relation \"{relation}\" not found on entity \"{entity}\""),
            })?;

        let data = serde_json::json!({ rel.field.clone(): target_id });
        self.update(entity, id, &data)
    }

    /// Unlink a relation by setting the foreign-key field to null.
    pub fn unlink(
        &self,
        entity: &str,
        id: &str,
        relation: &str,
    ) -> Result<bool, RuntimeError> {
        let ent = self.require_entity(entity)?;

        let rel = ent
            .relations
            .iter()
            .find(|r| r.name == relation)
            .ok_or_else(|| RuntimeError {
                code: "RELATION_NOT_FOUND".into(),
                message: format!("Relation \"{relation}\" not found on entity \"{entity}\""),
            })?;

        let data = serde_json::json!({ rel.field.clone(): null });
        self.update(entity, id, &data)
    }

    /// Execute a filtered query with operators ($not, $gt, $in, $like, $order, $limit).
    pub fn query_filtered(
        &self,
        entity: &str,
        filter: &serde_json::Value,
    ) -> Result<Vec<serde_json::Value>, RuntimeError> {
        let ent = self.require_entity(entity)?;
        let conn = self.lock_read_conn()?;

        let columns: Vec<String> = ent.fields.iter().map(|f| f.name.clone()).collect();
        let obj = filter.as_object().unwrap_or(&serde_json::Map::new()).clone();

        let mut where_clauses = Vec::new();
        let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        let mut order_clause = String::new();
        let mut limit_clause = String::new();
        let mut idx = 1;

        for (key, val) in &obj {
            match key.as_str() {
                "$order" => {
                    if let Some(order_obj) = val.as_object() {
                        let mut parts: Vec<String> = Vec::new();
                        for (col, dir) in order_obj {
                            validate_column_name(col, ent)?;
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
                    if let Some(n) = val.as_u64() {
                        limit_clause = format!(" LIMIT {n}");
                    }
                }
                _ => {
                    validate_column_name(key, ent)?;
                    let quoted_key = quote_ident(key);

                    if let Some(op_obj) = val.as_object() {
                        for (op, op_val) in op_obj {
                            match op.as_str() {
                                "$not" => {
                                    where_clauses.push(format!("{quoted_key} != ?{idx}"));
                                    values.push(json_to_sql(op_val));
                                    idx += 1;
                                }
                                "$gt" => {
                                    where_clauses.push(format!("{quoted_key} > ?{idx}"));
                                    values.push(json_to_sql(op_val));
                                    idx += 1;
                                }
                                "$gte" => {
                                    where_clauses.push(format!("{quoted_key} >= ?{idx}"));
                                    values.push(json_to_sql(op_val));
                                    idx += 1;
                                }
                                "$lt" => {
                                    where_clauses.push(format!("{quoted_key} < ?{idx}"));
                                    values.push(json_to_sql(op_val));
                                    idx += 1;
                                }
                                "$lte" => {
                                    where_clauses.push(format!("{quoted_key} <= ?{idx}"));
                                    values.push(json_to_sql(op_val));
                                    idx += 1;
                                }
                                "$like" => {
                                    where_clauses.push(format!("{quoted_key} LIKE ?{idx}"));
                                    let pattern = format!(
                                        "%{}%",
                                        op_val.as_str().unwrap_or("")
                                    );
                                    values.push(Box::new(pattern));
                                    idx += 1;
                                }
                                "$in" => {
                                    if let Some(arr) = op_val.as_array() {
                                        let placeholders: Vec<String> = arr
                                            .iter()
                                            .map(|v| {
                                                let p = format!("?{idx}");
                                                values.push(json_to_sql(v));
                                                idx += 1;
                                                p
                                            })
                                            .collect();
                                        if !placeholders.is_empty() {
                                            where_clauses.push(format!(
                                                "{quoted_key} IN ({})",
                                                placeholders.join(", ")
                                            ));
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    } else {
                        // Simple equality.
                        where_clauses.push(format!("{quoted_key} = ?{idx}"));
                        values.push(json_to_sql(val));
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
            order_clause = " ORDER BY \"id\"".to_string();
        }

        let sql = format!(
            "SELECT * FROM {}{}{}{}",
            quote_ident(entity),
            where_sql,
            order_clause,
            limit_clause
        );
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            values.iter().map(|v| v.as_ref()).collect();

        let mut stmt = conn.prepare(&sql).map_err(|e| RuntimeError {
            code: "QUERY_FAILED".into(),
            message: format!("Failed to prepare filtered query: {e}"),
        })?;

        let rows = stmt
            .query_map(param_refs.as_slice(), |row| Ok(row_to_json(row, &columns)))
            .map_err(|e| RuntimeError {
                code: "QUERY_FAILED".into(),
                message: format!("Filtered query failed: {e}"),
            })?;

        let mut result = Vec::new();
        for row in rows {
            if let Ok(val) = row {
                result.push(val);
            }
        }
        Ok(result)
    }

    /// Execute a graph-style query.
    ///
    /// Input: `{ "User": { "where": { "email": "..." }, "include": { "posts": {} } } }`
    /// Returns nested results following relations.
    pub fn query_graph(
        &self,
        query: &serde_json::Value,
    ) -> Result<serde_json::Value, RuntimeError> {
        let obj = query.as_object().ok_or_else(|| RuntimeError {
            code: "INVALID_QUERY".into(),
            message: "Graph query must be a JSON object".into(),
        })?;

        let mut results = serde_json::Map::new();

        for (entity_name, query_opts) in obj {
            let _ent = self.require_entity(entity_name)?;

            // Apply where clause if present.
            let filter = query_opts.get("where").cloned().unwrap_or(serde_json::json!({}));
            let rows = self.query_filtered(entity_name, &filter)?;

            // Apply includes (relations) if present.
            let rows = if let Some(include) = query_opts.get("include").and_then(|v| v.as_object()) {
                let ent = self.entities.get(entity_name).unwrap();
                rows.into_iter()
                    .map(|mut row| {
                        for (rel_name, _sub_query) in include {
                            if let Some(rel) = ent.relations.iter().find(|r| r.name == *rel_name) {
                                let fk_value = row.get(&rel.field).and_then(|v| v.as_str()).map(|s| s.to_string());
                                if let Some(fk) = fk_value {
                                    if rel.many {
                                        // One-to-many: find rows in target where field matches id.
                                        let sub_filter = serde_json::json!({ &rel.field: &fk });
                                        if let Ok(related) = self.query_filtered(&rel.target, &sub_filter) {
                                            row[rel_name] = serde_json::json!(related);
                                        }
                                    } else {
                                        // One-to-one / many-to-one: get by id.
                                        if let Ok(Some(related)) = self.get_by_id(&rel.target, &fk) {
                                            row[rel_name] = related;
                                        }
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

            // Apply limit if present.
            let rows = if let Some(limit) = query_opts.get("limit").and_then(|v| v.as_u64()) {
                rows.into_iter().take(limit as usize).collect()
            } else {
                rows
            };

            results.insert(entity_name.clone(), serde_json::json!(rows));
        }

        Ok(serde_json::Value::Object(results))
    }

    // -----------------------------------------------------------------------
    // Transaction-safe variants (use a pre-held connection guard)
    // -----------------------------------------------------------------------

    /// Insert using an already-locked connection (for transactions).
    pub fn insert_with_conn(
        &self,
        conn: &Connection,
        entity: &str,
        data: &serde_json::Value,
    ) -> Result<String, RuntimeError> {
        let ent = self.require_entity(entity)?;
        let id = generate_id();
        let obj = data.as_object().ok_or_else(|| RuntimeError {
            code: "INVALID_DATA".into(),
            message: "Insert data must be a JSON object".into(),
        })?;

        let mut col_names = vec![quote_ident("id")];
        let mut placeholders = vec!["?1".to_string()];
        let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(id.clone())];
        let mut idx = 2;
        for (key, val) in obj {
            if key == "id" { continue; }
            validate_column_name(key, ent)?;
            col_names.push(quote_ident(key));
            placeholders.push(format!("?{idx}"));
            values.push(json_to_sql(val));
            idx += 1;
        }

        let sql = format!(
            "INSERT INTO {} ({}) VALUES ({})",
            quote_ident(entity), col_names.join(", "), placeholders.join(", ")
        );
        let params: Vec<&dyn rusqlite::types::ToSql> = values.iter().map(|v| v.as_ref()).collect();
        conn.execute(&sql, params.as_slice()).map_err(|e| RuntimeError {
            code: "INSERT_FAILED".into(),
            message: format!("Insert into {entity} failed: {e}"),
        })?;
        Ok(id)
    }

    /// Update using an already-locked connection (for transactions).
    pub fn update_with_conn(
        &self,
        conn: &Connection,
        entity: &str,
        id: &str,
        data: &serde_json::Value,
    ) -> Result<bool, RuntimeError> {
        let ent = self.require_entity(entity)?;
        let obj = data.as_object().ok_or_else(|| RuntimeError {
            code: "INVALID_DATA".into(),
            message: "Update data must be a JSON object".into(),
        })?;

        let mut set_clauses = Vec::new();
        let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        let mut idx = 1;
        for (key, val) in obj {
            if key == "id" { continue; }
            validate_column_name(key, ent)?;
            set_clauses.push(format!("{} = ?{idx}", quote_ident(key)));
            values.push(json_to_sql(val));
            idx += 1;
        }
        if set_clauses.is_empty() { return Ok(false); }

        values.push(Box::new(id.to_string()));
        let sql = format!(
            "UPDATE {} SET {} WHERE \"id\" = ?{idx}",
            quote_ident(entity), set_clauses.join(", ")
        );
        let params: Vec<&dyn rusqlite::types::ToSql> = values.iter().map(|v| v.as_ref()).collect();
        let affected = conn.execute(&sql, params.as_slice()).map_err(|e| RuntimeError {
            code: "UPDATE_FAILED".into(),
            message: format!("Update {entity}/{id} failed: {e}"),
        })?;
        Ok(affected > 0)
    }

    /// Delete using an already-locked connection (for transactions).
    pub fn delete_with_conn(
        &self,
        conn: &Connection,
        entity: &str,
        id: &str,
    ) -> Result<bool, RuntimeError> {
        let _ent = self.require_entity(entity)?;
        let sql = format!("DELETE FROM {} WHERE \"id\" = ?1", quote_ident(entity));
        let affected = conn.execute(&sql, rusqlite::params![id]).map_err(|e| RuntimeError {
            code: "DELETE_FAILED".into(),
            message: format!("Delete {entity}/{id} failed: {e}"),
        })?;
        Ok(affected > 0)
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn require_entity(&self, name: &str) -> Result<&ManifestEntity, RuntimeError> {
        self.entities.get(name).ok_or_else(|| RuntimeError {
            code: "ENTITY_NOT_FOUND".into(),
            message: format!("Unknown entity: \"{name}\""),
        })
    }

    /// Acquire the write connection. Used for INSERT, UPDATE, DELETE.
    fn lock_write_conn(&self) -> Result<std::sync::MutexGuard<'_, Connection>, RuntimeError> {
        self.write_conn.lock().map_err(|e| RuntimeError {
            code: "LOCK_FAILED".into(),
            message: format!("Failed to acquire write connection lock: {e}"),
        })
    }

    /// Acquire a read connection. Uses the read pool if available (file-backed
    /// databases), otherwise falls back to the write connection (in-memory).
    /// Connections are selected round-robin to spread load evenly.
    fn lock_read_conn(&self) -> Result<ReadConnGuard<'_>, RuntimeError> {
        if !self.read_pool.is_empty() {
            let idx = self.read_counter.fetch_add(1, Ordering::Relaxed) % self.read_pool.len();
            let guard = self.read_pool[idx].lock().map_err(|e| RuntimeError {
                code: "LOCK_FAILED".into(),
                message: format!("Failed to acquire read connection: {e}"),
            })?;
            Ok(ReadConnGuard::Pooled(guard))
        } else {
            // Fall back to write connection for in-memory DBs.
            let guard = self.write_conn.lock().map_err(|e| RuntimeError {
                code: "LOCK_FAILED".into(),
                message: format!("Failed to acquire connection: {e}"),
            })?;
            Ok(ReadConnGuard::Write(guard))
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Generate a short, URL-safe random ID.
fn generate_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{:x}", nanos)
}

/// Convert a `serde_json::Value` to a boxed `ToSql` for rusqlite.
fn json_to_sql(val: &serde_json::Value) -> Box<dyn rusqlite::types::ToSql> {
    match val {
        serde_json::Value::Null => Box::new(rusqlite::types::Null),
        serde_json::Value::Bool(b) => Box::new(*b as i32),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Box::new(i)
            } else if let Some(f) = n.as_f64() {
                Box::new(f)
            } else {
                Box::new(n.to_string())
            }
        }
        serde_json::Value::String(s) => Box::new(s.clone()),
        other => Box::new(other.to_string()),
    }
}

/// Convert a rusqlite row to a JSON value given the column names from the entity.
fn row_to_json(row: &rusqlite::Row<'_>, field_names: &[String]) -> serde_json::Value {
    let mut obj = serde_json::Map::new();

    // First column is always `id`.
    if let Ok(id) = row.get::<_, String>(0) {
        obj.insert("id".into(), serde_json::Value::String(id));
    }

    for (i, name) in field_names.iter().enumerate() {
        let col_idx = i + 1; // +1 because id is at index 0
        // Try string first, then integer, then float, then null.
        if let Ok(s) = row.get::<_, String>(col_idx) {
            obj.insert(name.clone(), serde_json::Value::String(s));
        } else if let Ok(n) = row.get::<_, i64>(col_idx) {
            obj.insert(
                name.clone(),
                serde_json::Value::Number(serde_json::Number::from(n)),
            );
        } else if let Ok(f) = row.get::<_, f64>(col_idx) {
            if let Some(num) = serde_json::Number::from_f64(f) {
                obj.insert(name.clone(), serde_json::Value::Number(num));
            } else {
                obj.insert(name.clone(), serde_json::Value::Null);
            }
        } else {
            obj.insert(name.clone(), serde_json::Value::Null);
        }
    }

    serde_json::Value::Object(obj)
}


#[cfg(test)]
mod tests {
    use super::*;
    use agentdb_core::{ManifestField, ManifestIndex};

    fn test_manifest() -> AppManifest {
        AppManifest {
            manifest_version: 1,
            name: "Test".into(),
            version: "0.1.0".into(),
            entities: vec![agentdb_core::ManifestEntity {
                name: "User".into(),
                fields: vec![
                    ManifestField {
                        name: "email".into(),
                        field_type: "string".into(),
                        optional: false,
                        unique: true,
                    },
                    ManifestField {
                        name: "displayName".into(),
                        field_type: "string".into(),
                        optional: false,
                        unique: false,
                    },
                ],
                indexes: vec![ManifestIndex {
                    name: "user_email".into(),
                    fields: vec!["email".into()],
                    unique: true,
                }],
                relations: vec![],
            }],
            routes: vec![],
            queries: vec![],
            actions: vec![],
            policies: vec![],
        }
    }

    #[test]
    fn insert_and_get() {
        let rt = Runtime::in_memory(test_manifest()).unwrap();
        let id = rt
            .insert("User", &serde_json::json!({"email": "a@b.com", "displayName": "A"}))
            .unwrap();
        let row = rt.get_by_id("User", &id).unwrap().unwrap();
        assert_eq!(row["email"], "a@b.com");
    }

    #[test]
    fn list_entities() {
        let rt = Runtime::in_memory(test_manifest()).unwrap();
        rt.insert("User", &serde_json::json!({"email": "a@b.com", "displayName": "A"})).unwrap();
        rt.insert("User", &serde_json::json!({"email": "b@c.com", "displayName": "B"})).unwrap();
        let rows = rt.list("User").unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn update_entity() {
        let rt = Runtime::in_memory(test_manifest()).unwrap();
        let id = rt
            .insert("User", &serde_json::json!({"email": "a@b.com", "displayName": "A"}))
            .unwrap();
        let updated = rt.update("User", &id, &serde_json::json!({"displayName": "Updated"})).unwrap();
        assert!(updated);
        let row = rt.get_by_id("User", &id).unwrap().unwrap();
        assert_eq!(row["displayName"], "Updated");
    }

    #[test]
    fn delete_entity() {
        let rt = Runtime::in_memory(test_manifest()).unwrap();
        let id = rt
            .insert("User", &serde_json::json!({"email": "a@b.com", "displayName": "A"}))
            .unwrap();
        let deleted = rt.delete("User", &id).unwrap();
        assert!(deleted);
        assert!(rt.get_by_id("User", &id).unwrap().is_none());
    }

    #[test]
    fn lookup_by_field() {
        let rt = Runtime::in_memory(test_manifest()).unwrap();
        rt.insert("User", &serde_json::json!({"email": "a@b.com", "displayName": "A"})).unwrap();
        let row = rt.lookup("User", "email", "a@b.com").unwrap().unwrap();
        assert_eq!(row["displayName"], "A");
    }

    #[test]
    fn unknown_entity_returns_error() {
        let rt = Runtime::in_memory(test_manifest()).unwrap();
        let err = rt.list("Nonexistent").unwrap_err();
        assert_eq!(err.code, "ENTITY_NOT_FOUND");
    }

    #[test]
    fn insert_rejects_unknown_column() {
        let rt = Runtime::in_memory(test_manifest()).unwrap();
        let err = rt
            .insert("User", &serde_json::json!({"email": "a@b.com", "displayName": "A", "evil_col": "x"}))
            .unwrap_err();
        assert_eq!(err.code, "INVALID_COLUMN");
    }

    #[test]
    fn update_rejects_unknown_column() {
        let rt = Runtime::in_memory(test_manifest()).unwrap();
        let id = rt
            .insert("User", &serde_json::json!({"email": "a@b.com", "displayName": "A"}))
            .unwrap();
        let err = rt.update("User", &id, &serde_json::json!({"bad_field": "x"})).unwrap_err();
        assert_eq!(err.code, "INVALID_COLUMN");
    }

    #[test]
    fn lookup_rejects_unknown_column() {
        let rt = Runtime::in_memory(test_manifest()).unwrap();
        let err = rt.lookup("User", "nonexistent", "val").unwrap_err();
        assert_eq!(err.code, "INVALID_COLUMN");
    }

    #[test]
    fn query_filtered_rejects_unknown_column() {
        let rt = Runtime::in_memory(test_manifest()).unwrap();
        let err = rt
            .query_filtered("User", &serde_json::json!({"bad_col": "x"}))
            .unwrap_err();
        assert_eq!(err.code, "INVALID_COLUMN");
    }

    #[test]
    fn query_filtered_rejects_unknown_order_column() {
        let rt = Runtime::in_memory(test_manifest()).unwrap();
        let err = rt
            .query_filtered("User", &serde_json::json!({"$order": {"bad_col": "asc"}}))
            .unwrap_err();
        assert_eq!(err.code, "INVALID_COLUMN");
    }

    #[test]
    fn query_filtered_sanitizes_order_direction() {
        let rt = Runtime::in_memory(test_manifest()).unwrap();
        rt.insert("User", &serde_json::json!({"email": "a@b.com", "displayName": "A"})).unwrap();
        // Even a malicious direction value should be normalized to ASC.
        let rows = rt
            .query_filtered("User", &serde_json::json!({"$order": {"email": "DROP TABLE User"}}))
            .unwrap();
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn in_memory_has_no_read_pool() {
        let rt = Runtime::in_memory(test_manifest()).unwrap();
        assert_eq!(rt.read_pool_size(), 0);
    }

    #[test]
    fn open_creates_read_pool() {
        let dir = std::env::temp_dir().join(format!("agentdb_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("test_read_pool.db");

        let rt = Runtime::open(db_path.to_str().unwrap(), test_manifest()).unwrap();
        assert_eq!(rt.read_pool_size(), READ_POOL_SIZE);

        // Write then read through the pool.
        let id = rt
            .insert("User", &serde_json::json!({"email": "pool@test.com", "displayName": "Pool"}))
            .unwrap();
        let row = rt.get_by_id("User", &id).unwrap().unwrap();
        assert_eq!(row["email"], "pool@test.com");

        // Clean up.
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn concurrent_reads_dont_block_on_write() {
        use std::sync::Arc;

        let dir = std::env::temp_dir().join(format!("agentdb_conc_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("test_concurrent.db");

        let rt = Arc::new(Runtime::open(db_path.to_str().unwrap(), test_manifest()).unwrap());

        // Seed some data so reads have something to return.
        rt.insert("User", &serde_json::json!({"email": "a@b.com", "displayName": "A"})).unwrap();
        rt.insert("User", &serde_json::json!({"email": "b@c.com", "displayName": "B"})).unwrap();

        // Hold the write lock to simulate a long write.
        let write_guard = rt.lock_write_conn().unwrap();

        // Spawn reader threads that should succeed despite the held write lock.
        let mut handles = Vec::new();
        for _ in 0..4 {
            let rt_clone = Arc::clone(&rt);
            handles.push(std::thread::spawn(move || {
                let rows = rt_clone.list("User").unwrap();
                assert_eq!(rows.len(), 2);
            }));
        }

        for h in handles {
            h.join().expect("reader thread panicked");
        }

        // Release the write lock.
        drop(write_guard);

        // Clean up.
        let _ = std::fs::remove_dir_all(&dir);
    }
}
