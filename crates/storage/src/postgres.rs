use crate::{FieldSpec, SchemaOperation, SchemaPlan, StorageAdapter, StorageError};
use pylon_kernel::AppManifest;

// ---------------------------------------------------------------------------
// Type mapping: manifest field types -> PostgreSQL column types
//
//   string    -> TEXT
//   int       -> INTEGER
//   float     -> DOUBLE PRECISION
//   bool      -> BOOLEAN
//   datetime  -> TIMESTAMPTZ
//   richtext  -> TEXT
//   id(...)   -> TEXT
// ---------------------------------------------------------------------------

fn pg_column_type(field_type: &str) -> &'static str {
    match field_type {
        "string" => "TEXT",
        "int" => "INTEGER",
        "float" => "DOUBLE PRECISION",
        "bool" => "BOOLEAN",
        "datetime" => "TIMESTAMPTZ",
        "richtext" => "TEXT",
        _ if field_type.starts_with("id(") => "TEXT",
        _ => "TEXT",
    }
}

// ---------------------------------------------------------------------------
// Identifier quoting
// ---------------------------------------------------------------------------

/// Quote a SQL identifier, escaping embedded double-quotes by doubling them.
///
/// PostgreSQL standard: `"foo""bar"` represents the identifier `foo"bar`.
fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

// ---------------------------------------------------------------------------
// SQL generation
// ---------------------------------------------------------------------------

/// Generate a Postgres CREATE TABLE statement.
pub fn create_table_sql(entity_name: &str, fields: &[FieldSpec]) -> String {
    let mut columns = vec!["id TEXT PRIMARY KEY NOT NULL".to_string()];

    for field in fields {
        let col_type = pg_column_type(&field.field_type);
        let not_null = if field.optional { "" } else { " NOT NULL" };
        let unique = if field.unique { " UNIQUE" } else { "" };
        columns.push(format!(
            "{} {}{}{}",
            quote_ident(&field.name),
            col_type,
            not_null,
            unique
        ));
    }

    format!(
        "CREATE TABLE IF NOT EXISTS {} ({})",
        quote_ident(entity_name),
        columns.join(", ")
    )
}

/// Generate a Postgres ALTER TABLE ADD COLUMN statement.
/// NOT NULL is omitted on ADD COLUMN to avoid requiring DEFAULT values.
/// Required-ness is tracked in the manifest; enforcement deferred.
pub fn add_column_sql(entity_name: &str, field: &FieldSpec) -> String {
    let col_type = pg_column_type(&field.field_type);
    let unique = if field.unique { " UNIQUE" } else { "" };
    format!(
        "ALTER TABLE {} ADD COLUMN {} {}{}",
        quote_ident(entity_name),
        quote_ident(&field.name),
        col_type,
        unique
    )
}

/// Generate a Postgres CREATE INDEX statement.
pub fn create_index_sql(
    entity_name: &str,
    index_name: &str,
    fields: &[String],
    unique: bool,
) -> String {
    let unique_str = if unique { "UNIQUE " } else { "" };
    let full_index_name = format!("{}_{}", entity_name, index_name);
    let quoted_fields: Vec<String> = fields.iter().map(|f| quote_ident(f)).collect();
    format!(
        "CREATE {}INDEX IF NOT EXISTS {} ON {} ({})",
        unique_str,
        quote_ident(&full_index_name),
        quote_ident(entity_name),
        quoted_fields.join(", ")
    )
}

// ---------------------------------------------------------------------------
// PostgresAdapter — planning-only adapter
// ---------------------------------------------------------------------------

/// A Postgres storage adapter. Currently supports planning only.
/// No live connection — SQL generation and planning from manifest.
pub struct PostgresAdapter;

impl StorageAdapter for PostgresAdapter {
    fn plan_schema(&self, target: &AppManifest) -> Result<SchemaPlan, StorageError> {
        // Plan from empty baseline.
        let mut operations = Vec::new();

        for entity in &target.entities {
            let fields: Vec<FieldSpec> = entity
                .fields
                .iter()
                .map(|f| FieldSpec {
                    name: f.name.clone(),
                    field_type: f.field_type.clone(),
                    optional: f.optional,
                    unique: f.unique,
                })
                .collect();

            operations.push(SchemaOperation::CreateEntity {
                name: entity.name.clone(),
                fields,
            });

            for index in &entity.indexes {
                operations.push(SchemaOperation::AddIndex {
                    entity: entity.name.clone(),
                    name: index.name.clone(),
                    fields: index.fields.clone(),
                    unique: index.unique,
                });
            }
        }

        if operations.is_empty() {
            operations.push(SchemaOperation::Noop);
        }

        Ok(SchemaPlan { operations })
    }

    // apply_schema intentionally not implemented — uses default trait error.
}

/// Generate all SQL statements for a plan, in order.
/// Useful for dry-run preview of what Postgres DDL would be executed.
pub fn plan_to_sql(plan: &SchemaPlan) -> Result<Vec<String>, StorageError> {
    let mut statements = Vec::new();

    for op in &plan.operations {
        match op {
            SchemaOperation::CreateEntity { name, fields } => {
                statements.push(create_table_sql(name, fields));
            }
            SchemaOperation::AddField { entity, field } => {
                statements.push(add_column_sql(entity, field));
            }
            SchemaOperation::AddIndex {
                entity,
                name,
                fields,
                unique,
            } => {
                statements.push(create_index_sql(entity, name, fields, *unique));
            }
            SchemaOperation::Noop => {}
            other => {
                return Err(StorageError {
                    code: "PG_OP_UNSUPPORTED".into(),
                    message: format!("Operation not supported by Postgres adapter: {other:?}"),
                });
            }
        }
    }

    Ok(statements)
}

// ---------------------------------------------------------------------------
// Introspection SQL helpers
//
// These generate the SQL queries that a live Postgres connection would run
// to read the current schema. No connection required — just SQL strings.
// ---------------------------------------------------------------------------

/// SQL to list user tables in the public schema.
pub const INTROSPECT_TABLES_SQL: &str = "\
    SELECT table_name \
    FROM information_schema.tables \
    WHERE table_schema = 'public' \
      AND table_type = 'BASE TABLE' \
      AND table_name NOT LIKE '_pylon_%' \
    ORDER BY table_name";

/// SQL to list columns for a given table.
/// Use with parameter: table_name.
pub const INTROSPECT_COLUMNS_SQL: &str = "\
    SELECT column_name, data_type, is_nullable, \
           (SELECT COUNT(*) FROM information_schema.table_constraints tc \
            JOIN information_schema.key_column_usage kcu \
              ON tc.constraint_name = kcu.constraint_name \
            WHERE tc.table_name = c.table_name \
              AND kcu.column_name = c.column_name \
              AND tc.constraint_type = 'PRIMARY KEY') as is_pk \
    FROM information_schema.columns c \
    WHERE table_schema = 'public' AND table_name = $1 \
    ORDER BY ordinal_position";

/// SQL to list indexes for a given table.
/// Use with parameter: table_name.
pub const INTROSPECT_INDEXES_SQL: &str = "\
    SELECT i.relname as index_name, \
           ix.indisunique as is_unique, \
           array_agg(a.attname ORDER BY array_position(ix.indkey, a.attnum)) as columns \
    FROM pg_index ix \
    JOIN pg_class t ON t.oid = ix.indrelid \
    JOIN pg_class i ON i.oid = ix.indexrelid \
    JOIN pg_attribute a ON a.attrelid = t.oid AND a.attnum = ANY(ix.indkey) \
    JOIN pg_namespace n ON n.oid = t.relnamespace \
    WHERE n.nspname = 'public' \
      AND t.relname = $1 \
      AND NOT ix.indisprimary \
    GROUP BY i.relname, ix.indisunique \
    ORDER BY i.relname";

/// Plan from a snapshot (reuses the shared plan_from_snapshot).
/// This allows Postgres to plan incrementally once introspection data is available.
pub fn plan_from_snapshot(snapshot: &crate::SchemaSnapshot, target: &AppManifest) -> SchemaPlan {
    crate::plan_from_snapshot(snapshot, target)
}

// ---------------------------------------------------------------------------
// CRUD SQL generation helpers (used by live adapter, testable without a DB)
// ---------------------------------------------------------------------------

/// Generate a lex-sortable, monotonic-ish unique ID.
///
/// Format: 32 hex chars of `as_nanos()` (zero-padded) followed by 8 hex chars
/// of a per-process atomic counter. The counter prevents collisions when two
/// inserts hit the same nanosecond and — critically — keeps order stable: an
/// id minted at the same nanosecond is monotonically greater than the
/// previous one. Width is fixed at 40 chars so lexicographic comparison
/// matches creation order, which is what cursor pagination relies on.
pub fn generate_id() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{ts:032x}{seq:08x}")
}

/// Convert a JSON value to its string representation for use as a SQL parameter.
pub fn json_value_to_string(val: &serde_json::Value) -> String {
    match val {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Build an INSERT SQL statement and collect string parameter values.
/// Returns `(sql, values)` where `values[0]` is the generated ID.
pub fn build_insert_sql(
    entity: &str,
    data: &serde_json::Value,
) -> Result<(String, Vec<String>), StorageError> {
    let id = generate_id();
    let obj = data.as_object().ok_or_else(|| StorageError {
        code: "PG_INVALID_DATA".into(),
        message: "Insert data must be a JSON object".into(),
    })?;

    let mut col_names = vec!["id".to_string()];
    let mut placeholders = vec!["$1".to_string()];
    let mut values: Vec<String> = vec![id];

    for (i, (key, val)) in obj.iter().enumerate() {
        col_names.push(quote_ident(key));
        placeholders.push(format!("${}", i + 2));
        values.push(json_value_to_string(val));
    }

    let sql = format!(
        "INSERT INTO {} ({}) VALUES ({})",
        quote_ident(entity),
        col_names.join(", "),
        placeholders.join(", ")
    );

    Ok((sql, values))
}

/// Build an UPDATE SQL statement and collect string parameter values.
/// Returns `(sql, values)` where `values[0]` is the row ID.
pub fn build_update_sql(
    entity: &str,
    id: &str,
    data: &serde_json::Value,
) -> Result<(String, Vec<String>), StorageError> {
    let obj = data.as_object().ok_or_else(|| StorageError {
        code: "PG_INVALID_DATA".into(),
        message: "Update data must be a JSON object".into(),
    })?;

    if obj.is_empty() {
        return Err(StorageError {
            code: "PG_INVALID_DATA".into(),
            message: "Update data must contain at least one field".into(),
        });
    }

    let mut set_clauses = Vec::new();
    let mut values: Vec<String> = vec![id.to_string()];

    for (i, (key, val)) in obj.iter().enumerate() {
        set_clauses.push(format!("{} = ${}", quote_ident(key), i + 2));
        values.push(json_value_to_string(val));
    }

    let sql = format!(
        "UPDATE {} SET {} WHERE id = $1",
        quote_ident(entity),
        set_clauses.join(", ")
    );

    Ok((sql, values))
}

// ---------------------------------------------------------------------------
// Live Postgres adapter (requires "postgres-live" feature)
// ---------------------------------------------------------------------------

#[cfg(feature = "postgres-live")]
pub mod live {
    use super::*;
    use crate::{
        ColumnSnapshot, IndexSnapshot, SchemaSnapshot, StorageAdapter, StorageError, TableSnapshot,
    };

    /// A live Postgres adapter with a real database connection.
    pub struct LivePostgresAdapter {
        client: postgres::Client,
    }

    impl LivePostgresAdapter {
        /// Connect to a Postgres database.
        pub fn connect(url: &str) -> Result<Self, StorageError> {
            let client =
                postgres::Client::connect(url, postgres::NoTls).map_err(|e| StorageError {
                    code: "PG_CONNECT_FAILED".into(),
                    message: format!("Failed to connect to Postgres: {e}"),
                })?;
            Ok(Self { client })
        }

        /// Read the current schema from the live database.
        pub fn read_schema(&mut self) -> Result<SchemaSnapshot, StorageError> {
            let table_rows = self
                .client
                .query(INTROSPECT_TABLES_SQL, &[])
                .map_err(pg_err)?;

            let mut tables = Vec::new();
            for row in &table_rows {
                let table_name: String = row.get(0);
                let columns = self.read_columns(&table_name)?;
                let indexes = self.read_indexes(&table_name)?;
                tables.push(TableSnapshot {
                    name: table_name,
                    columns,
                    indexes,
                });
            }

            Ok(SchemaSnapshot { tables })
        }

        fn read_columns(&mut self, table: &str) -> Result<Vec<ColumnSnapshot>, StorageError> {
            let rows = self
                .client
                .query(INTROSPECT_COLUMNS_SQL, &[&table])
                .map_err(pg_err)?;

            let mut columns = Vec::new();
            for row in &rows {
                let name: String = row.get(0);
                let data_type: String = row.get(1);
                let is_nullable: String = row.get(2);
                let is_pk: i64 = row.get(3);
                columns.push(ColumnSnapshot {
                    name,
                    column_type: data_type,
                    notnull: is_nullable == "NO",
                    primary_key: is_pk > 0,
                });
            }
            Ok(columns)
        }

        fn read_indexes(&mut self, table: &str) -> Result<Vec<IndexSnapshot>, StorageError> {
            let rows = self
                .client
                .query(INTROSPECT_INDEXES_SQL, &[&table])
                .map_err(pg_err)?;

            let mut indexes = Vec::new();
            for row in &rows {
                let name: String = row.get(0);
                let unique: bool = row.get(1);
                let columns: Vec<String> = row.get(2);
                indexes.push(IndexSnapshot {
                    name,
                    columns,
                    unique,
                });
            }
            Ok(indexes)
        }

        /// Plan from live database state.
        pub fn plan_from_live(&mut self, target: &AppManifest) -> Result<SchemaPlan, StorageError> {
            let snapshot = self.read_schema()?;
            Ok(crate::plan_from_snapshot(&snapshot, target))
        }
    }

    impl StorageAdapter for LivePostgresAdapter {
        fn plan_schema(&self, _target: &AppManifest) -> Result<SchemaPlan, StorageError> {
            Err(StorageError {
                code: "PG_PLAN_NEEDS_MUTABLE".into(),
                message: "Use plan_from_live() instead for live Postgres planning".into(),
            })
        }

        fn apply_schema(&self, _plan: &SchemaPlan) -> Result<(), StorageError> {
            Err(StorageError {
                code: "PG_APPLY_USE_METHOD".into(),
                message: "Use apply_plan() instead of the trait method for live Postgres".into(),
            })
        }
    }

    impl LivePostgresAdapter {
        /// Apply a schema plan to the live database.
        pub fn apply_plan(&mut self, plan: &SchemaPlan) -> Result<(), StorageError> {
            let statements = plan_to_sql(plan)?;
            for sql in &statements {
                self.client.execute(sql.as_str(), &[]).map_err(pg_err)?;
            }
            Ok(())
        }

        /// Insert a row. Returns the generated ID.
        pub fn insert(
            &mut self,
            entity: &str,
            data: &serde_json::Value,
        ) -> Result<String, StorageError> {
            let (sql, values) = build_insert_sql(entity, data)?;
            let id = values[0].clone();

            let params: Vec<&(dyn postgres::types::ToSql + Sync)> = values
                .iter()
                .map(|v| v as &(dyn postgres::types::ToSql + Sync))
                .collect();

            self.client.execute(sql.as_str(), &params).map_err(pg_err)?;
            Ok(id)
        }

        /// Get a row by ID.
        pub fn get_by_id(
            &mut self,
            entity: &str,
            id: &str,
        ) -> Result<Option<serde_json::Value>, StorageError> {
            let sql = format!("SELECT * FROM {} WHERE id = $1", quote_ident(entity));
            let rows = self.client.query(sql.as_str(), &[&id]).map_err(pg_err)?;

            match rows.first() {
                Some(row) => Ok(Some(row_to_json(row))),
                None => Ok(None),
            }
        }

        /// List all rows from an entity.
        pub fn list(&mut self, entity: &str) -> Result<Vec<serde_json::Value>, StorageError> {
            let sql = format!("SELECT * FROM {}", quote_ident(entity));
            let rows = self.client.query(sql.as_str(), &[]).map_err(pg_err)?;

            Ok(rows.iter().map(row_to_json).collect())
        }

        /// Cursor-paginated list. `after` is the last `id` from the previous
        /// page; the result contains rows with `id > after` (lex order),
        /// limited to `limit`. Used for sync push/pull.
        pub fn list_after(
            &mut self,
            entity: &str,
            after: Option<&str>,
            limit: usize,
        ) -> Result<Vec<serde_json::Value>, StorageError> {
            // Cap limit at a sensible upper bound so a malicious client can't
            // stream the whole table by passing limit=u64::MAX.
            let capped: i64 = limit.min(10_000) as i64;
            let sql = match after {
                Some(_) => format!(
                    "SELECT * FROM {} WHERE id > $1 ORDER BY id ASC LIMIT $2",
                    quote_ident(entity)
                ),
                None => format!(
                    "SELECT * FROM {} ORDER BY id ASC LIMIT $1",
                    quote_ident(entity)
                ),
            };
            let rows = match after {
                Some(cursor) => self
                    .client
                    .query(sql.as_str(), &[&cursor, &capped])
                    .map_err(pg_err)?,
                None => self
                    .client
                    .query(sql.as_str(), &[&capped])
                    .map_err(pg_err)?,
            };
            Ok(rows.iter().map(row_to_json).collect())
        }

        /// Update a row by ID. Returns true if the row was found and updated.
        pub fn update(
            &mut self,
            entity: &str,
            id: &str,
            data: &serde_json::Value,
        ) -> Result<bool, StorageError> {
            let (sql, values) = build_update_sql(entity, id, data)?;

            let params: Vec<&(dyn postgres::types::ToSql + Sync)> = values
                .iter()
                .map(|v| v as &(dyn postgres::types::ToSql + Sync))
                .collect();

            let rows_affected = self.client.execute(sql.as_str(), &params).map_err(pg_err)?;
            Ok(rows_affected > 0)
        }

        /// Delete a row by ID. Returns true if the row was found and deleted.
        pub fn delete(&mut self, entity: &str, id: &str) -> Result<bool, StorageError> {
            let sql = format!("DELETE FROM {} WHERE id = $1", quote_ident(entity));
            let rows_affected = self.client.execute(sql.as_str(), &[&id]).map_err(pg_err)?;
            Ok(rows_affected > 0)
        }

        /// Look up a row by `field = value`. Caller must validate `field`
        /// against the manifest before calling — we still `quote_ident` it
        /// but won't catch a typo against the entity definition.
        pub fn lookup_field(
            &mut self,
            entity: &str,
            field: &str,
            value: &str,
        ) -> Result<Option<serde_json::Value>, StorageError> {
            let sql = format!(
                "SELECT * FROM {} WHERE {} = $1 LIMIT 1",
                quote_ident(entity),
                quote_ident(field),
            );
            let rows = self.client.query(sql.as_str(), &[&value]).map_err(pg_err)?;
            Ok(rows.first().map(row_to_json))
        }

        /// Push a `query_filtered` filter down to a real Postgres `WHERE`.
        ///
        /// Supported operators: equality (`field: value`), `$gt`, `$gte`,
        /// `$lt`, `$lte`, `$like`, `$in: [..]`, plus the meta operators
        /// `$order: { field: "asc"|"desc" }`, `$limit`, `$offset`.
        ///
        /// Anything else is silently ignored (matches the in-memory fallback's
        /// permissive behavior). Field names are validated against `valid_columns`
        /// to prevent SQL injection — pass the entity's column set.
        pub fn query_filtered(
            &mut self,
            entity: &str,
            filter: &serde_json::Value,
            valid_columns: &[String],
        ) -> Result<Vec<serde_json::Value>, StorageError> {
            let empty = serde_json::Map::new();
            let obj = filter.as_object().unwrap_or(&empty);

            let validate = |col: &str| -> Result<(), StorageError> {
                if col == "id" || valid_columns.iter().any(|c| c == col) {
                    Ok(())
                } else {
                    Err(StorageError {
                        code: "UNKNOWN_COLUMN".into(),
                        message: format!("Unknown column \"{col}\" on entity \"{entity}\""),
                    })
                }
            };

            let mut where_clauses: Vec<String> = Vec::new();
            let mut order_clause = String::new();
            let mut limit_clause = String::new();
            let mut offset_clause = String::new();
            // Collect (col, op, value) so placeholder numbers can be assigned
            // in a single materialization pass after the parse loop.
            let mut planned: Vec<(String, String, String)> = Vec::new();

            for (key, val) in obj {
                match key.as_str() {
                    "$order" => {
                        if let Some(ord) = val.as_object() {
                            let mut parts = Vec::new();
                            for (col, dir) in ord {
                                validate(col)?;
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
                            limit_clause = format!(" LIMIT {}", n);
                        }
                    }
                    "$offset" => {
                        if let Some(n) = val.as_u64() {
                            offset_clause = format!(" OFFSET {}", n);
                        }
                    }
                    field => {
                        validate(field)?;
                        match val {
                            serde_json::Value::Object(ops) => {
                                for (op, v) in ops {
                                    match op.as_str() {
                                        "$gt" => {
                                            planned.push((field.into(), ">".into(), value_to_pg(v)))
                                        }
                                        "$gte" => planned.push((
                                            field.into(),
                                            ">=".into(),
                                            value_to_pg(v),
                                        )),
                                        "$lt" => {
                                            planned.push((field.into(), "<".into(), value_to_pg(v)))
                                        }
                                        "$lte" => planned.push((
                                            field.into(),
                                            "<=".into(),
                                            value_to_pg(v),
                                        )),
                                        "$like" => planned.push((
                                            field.into(),
                                            "LIKE".into(),
                                            value_to_pg(v),
                                        )),
                                        "$in" => {
                                            if let Some(arr) = v.as_array() {
                                                let placeholders: Vec<String> = (0..arr.len())
                                                    .map(|i| format!("${}", planned.len() + 1 + i))
                                                    .collect();
                                                where_clauses.push(format!(
                                                    "{} IN ({})",
                                                    quote_ident(field),
                                                    placeholders.join(", "),
                                                ));
                                                for x in arr {
                                                    planned.push((
                                                        format!("__inline_{}", planned.len()),
                                                        "__INLINE__".into(),
                                                        value_to_pg(x),
                                                    ));
                                                }
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }
                            _ => planned.push((field.into(), "=".into(), value_to_pg(val))),
                        }
                    }
                }
            }

            // Materialize planned -> SQL + params.
            let mut params: Vec<String> = Vec::with_capacity(planned.len());
            for (field, op, v) in &planned {
                if op == "__INLINE__" {
                    // Already emitted via the IN-clause path; just push the value.
                    params.push(v.clone());
                } else {
                    let placeholder = format!("${}", params.len() + 1);
                    where_clauses.push(format!("{} {} {}", quote_ident(field), op, placeholder));
                    params.push(v.clone());
                }
            }

            let where_sql = if where_clauses.is_empty() {
                String::new()
            } else {
                format!(" WHERE {}", where_clauses.join(" AND "))
            };
            let sql = format!(
                "SELECT * FROM {}{}{}{}{}",
                quote_ident(entity),
                where_sql,
                order_clause,
                limit_clause,
                offset_clause,
            );

            let pg_params: Vec<&(dyn postgres::types::ToSql + Sync)> = params
                .iter()
                .map(|s| s as &(dyn postgres::types::ToSql + Sync))
                .collect();

            let rows = self
                .client
                .query(sql.as_str(), &pg_params)
                .map_err(pg_err)?;
            Ok(rows.iter().map(row_to_json).collect())
        }
    }

    /// Atomic operation describing a single mutation inside [`LivePostgresAdapter::transact`].
    pub enum TxOp<'a> {
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

    /// Result of a single op inside a transaction.
    #[derive(Debug, Clone)]
    pub enum TxResult {
        Inserted(String),
        Updated(bool),
        Deleted(bool),
    }

    impl LivePostgresAdapter {
        /// Run `ops` inside a single Postgres transaction. Either all of them
        /// commit together or none of them do — there is no partial state on
        /// failure. The ROLLBACK happens implicitly when the `Transaction`
        /// guard is dropped without `commit()` being called.
        pub fn transact(&mut self, ops: &[TxOp<'_>]) -> Result<Vec<TxResult>, StorageError> {
            let mut tx = self.client.transaction().map_err(pg_err)?;
            let mut results: Vec<TxResult> = Vec::with_capacity(ops.len());

            for op in ops {
                match op {
                    TxOp::Insert { entity, data } => {
                        let (sql, values) = build_insert_sql(entity, data)?;
                        let id = values[0].clone();
                        let params: Vec<&(dyn postgres::types::ToSql + Sync)> = values
                            .iter()
                            .map(|v| v as &(dyn postgres::types::ToSql + Sync))
                            .collect();
                        tx.execute(sql.as_str(), &params).map_err(pg_err)?;
                        results.push(TxResult::Inserted(id));
                    }
                    TxOp::Update { entity, id, data } => {
                        let (sql, values) = build_update_sql(entity, id, data)?;
                        let params: Vec<&(dyn postgres::types::ToSql + Sync)> = values
                            .iter()
                            .map(|v| v as &(dyn postgres::types::ToSql + Sync))
                            .collect();
                        let n = tx.execute(sql.as_str(), &params).map_err(pg_err)?;
                        results.push(TxResult::Updated(n > 0));
                    }
                    TxOp::Delete { entity, id } => {
                        let sql = format!("DELETE FROM {} WHERE id = $1", quote_ident(entity));
                        let n = tx.execute(sql.as_str(), &[id]).map_err(pg_err)?;
                        results.push(TxResult::Deleted(n > 0));
                    }
                }
            }

            tx.commit().map_err(pg_err)?;
            Ok(results)
        }
    }

    fn value_to_pg(v: &serde_json::Value) -> String {
        match v {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::Bool(b) => b.to_string(),
            serde_json::Value::Null => String::new(),
            other => other.to_string(),
        }
    }

    fn row_to_json(row: &postgres::Row) -> serde_json::Value {
        use postgres::types::Type;
        let mut obj = serde_json::Map::new();
        for (i, col) in row.columns().iter().enumerate() {
            let name = col.name().to_string();

            // Use `try_get` everywhere — `Row::get` panics on decode mismatch,
            // and a panic in a query handler poisons the connection mutex,
            // taking down all subsequent reads on this datastore. Anything
            // that fails to decode becomes Null with a one-shot warning.
            //
            // Timestamps and the catch-all path explicitly DON'T request
            // `String` — the postgres crate uses binary protocol by default
            // and there's no `FromSql<String>` impl for TIMESTAMPTZ etc. We
            // ask for `Vec<u8>` and lossy-stringify, which works for all
            // text-shaped columns in either protocol.
            let value: serde_json::Value = match *col.type_() {
                Type::BOOL => try_get_or_null::<Option<bool>>(row, i)
                    .flatten()
                    .map(serde_json::Value::Bool)
                    .unwrap_or(serde_json::Value::Null),
                Type::INT2 => try_get_or_null::<Option<i16>>(row, i)
                    .flatten()
                    .map(|v| serde_json::Value::Number(v.into()))
                    .unwrap_or(serde_json::Value::Null),
                Type::INT4 => try_get_or_null::<Option<i32>>(row, i)
                    .flatten()
                    .map(|v| serde_json::Value::Number(v.into()))
                    .unwrap_or(serde_json::Value::Null),
                Type::INT8 => try_get_or_null::<Option<i64>>(row, i)
                    .flatten()
                    .map(|v| serde_json::Value::Number(v.into()))
                    .unwrap_or(serde_json::Value::Null),
                Type::FLOAT4 => try_get_or_null::<Option<f32>>(row, i)
                    .flatten()
                    .and_then(|v| serde_json::Number::from_f64(v as f64))
                    .map(serde_json::Value::Number)
                    .unwrap_or(serde_json::Value::Null),
                Type::FLOAT8 => try_get_or_null::<Option<f64>>(row, i)
                    .flatten()
                    .and_then(serde_json::Number::from_f64)
                    .map(serde_json::Value::Number)
                    .unwrap_or(serde_json::Value::Null),
                Type::JSON | Type::JSONB => try_get_or_null::<Option<serde_json::Value>>(row, i)
                    .flatten()
                    .unwrap_or(serde_json::Value::Null),
                Type::BYTEA => try_get_or_null::<Option<Vec<u8>>>(row, i)
                    .flatten()
                    .map(|b| serde_json::Value::String(b64(&b)))
                    .unwrap_or(serde_json::Value::Null),
                Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME | Type::UNKNOWN => {
                    try_get_or_null::<Option<String>>(row, i)
                        .flatten()
                        .map(serde_json::Value::String)
                        .unwrap_or(serde_json::Value::Null)
                }
                _ => {
                    // Last resort: ask Postgres to render anything else as
                    // text via a stringifying decode through Vec<u8>. If even
                    // that fails (rare — Postgres types not implementing the
                    // text format), fall through to Null with a warning.
                    match row.try_get::<_, Option<String>>(i) {
                        Ok(Some(s)) => serde_json::Value::String(s),
                        Ok(None) => serde_json::Value::Null,
                        Err(_) => match row.try_get::<_, Option<Vec<u8>>>(i) {
                            Ok(Some(bytes)) => serde_json::Value::String(
                                String::from_utf8_lossy(&bytes).into_owned(),
                            ),
                            _ => serde_json::Value::Null,
                        },
                    }
                }
            };
            obj.insert(name, value);
        }
        serde_json::Value::Object(obj)
    }

    fn try_get_or_null<'a, T>(row: &'a postgres::Row, i: usize) -> Option<T>
    where
        T: postgres::types::FromSql<'a>,
    {
        match row.try_get::<_, T>(i) {
            Ok(v) => Some(v),
            Err(e) => {
                tracing::warn!(
                    "[postgres] decode failed for column {} ({}): {e}",
                    i,
                    row.columns()[i].name()
                );
                None
            }
        }
    }

    /// Minimal base64 encoder so we don't need another dependency just for
    /// the BYTEA column edge case.
    fn b64(bytes: &[u8]) -> String {
        const TABLE: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
        let chunks = bytes.chunks(3);
        for chunk in chunks {
            let b = [
                chunk.first().copied().unwrap_or(0),
                chunk.get(1).copied().unwrap_or(0),
                chunk.get(2).copied().unwrap_or(0),
            ];
            out.push(TABLE[(b[0] >> 2) as usize] as char);
            out.push(TABLE[((b[0] & 0x03) << 4 | b[1] >> 4) as usize] as char);
            if chunk.len() > 1 {
                out.push(TABLE[((b[1] & 0x0F) << 2 | b[2] >> 6) as usize] as char);
            } else {
                out.push('=');
            }
            if chunk.len() > 2 {
                out.push(TABLE[(b[2] & 0x3F) as usize] as char);
            } else {
                out.push('=');
            }
        }
        out
    }

    fn pg_err(e: postgres::Error) -> StorageError {
        StorageError {
            code: "PG_QUERY_FAILED".into(),
            message: format!("Postgres query failed: {e}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Hand-rolled fixture that matches the snapshots in the tests
    /// below. Decoupled from any example's `pylon.manifest.json` so
    /// changing an example schema doesn't bleed into adapter tests.
    fn test_manifest() -> AppManifest {
        use pylon_kernel::{ManifestEntity, ManifestField, ManifestIndex};
        let f = |name: &str, ty: &str, opt: bool, uniq: bool| ManifestField {
            name: name.into(),
            field_type: ty.into(),
            optional: opt,
            unique: uniq,
            crdt: None,
        };
        AppManifest {
            manifest_version: 1,
            name: "test".into(),
            version: "0.0.0".into(),
            entities: vec![
                ManifestEntity {
                    name: "User".into(),
                    fields: vec![
                        f("email", "string", false, true),
                        f("displayName", "string", false, false),
                        f("createdAt", "datetime", false, false),
                    ],
                    indexes: vec![],
                    relations: vec![],
                    search: None,
                                    crdt: true,
                },
                ManifestEntity {
                    name: "Todo".into(),
                    fields: vec![
                        f("title", "string", false, false),
                        f("done", "bool", false, false),
                        f("userId", "id(User)", false, false),
                        f("createdAt", "datetime", false, false),
                    ],
                    indexes: vec![ManifestIndex {
                        name: "by_user".into(),
                        fields: vec!["userId".into()],
                        unique: false,
                    }],
                    relations: vec![],
                    search: None,
                                    crdt: true,
                },
            ],
            queries: vec![],
            actions: vec![],
            policies: vec![],
            routes: vec![],
        }
    }

    #[test]
    fn pg_type_mapping() {
        assert_eq!(pg_column_type("string"), "TEXT");
        assert_eq!(pg_column_type("int"), "INTEGER");
        assert_eq!(pg_column_type("float"), "DOUBLE PRECISION");
        assert_eq!(pg_column_type("bool"), "BOOLEAN");
        assert_eq!(pg_column_type("datetime"), "TIMESTAMPTZ");
        assert_eq!(pg_column_type("richtext"), "TEXT");
        assert_eq!(pg_column_type("id(User)"), "TEXT");
    }

    #[test]
    fn quote_ident_simple() {
        assert_eq!(quote_ident("User"), "\"User\"");
        assert_eq!(quote_ident("email"), "\"email\"");
    }

    #[test]
    fn quote_ident_escapes_embedded_double_quotes() {
        assert_eq!(quote_ident("col\"name"), "\"col\"\"name\"");
        assert_eq!(quote_ident("a\"b\"c"), "\"a\"\"b\"\"c\"");
    }

    #[test]
    fn create_table_sql_basic() {
        let fields = vec![
            FieldSpec {
                name: "email".into(),
                field_type: "string".into(),
                optional: false,
                unique: true,
            },
            FieldSpec {
                name: "age".into(),
                field_type: "int".into(),
                optional: true,
                unique: false,
            },
        ];
        let sql = create_table_sql("User", &fields);
        assert_eq!(
            sql,
            "CREATE TABLE IF NOT EXISTS \"User\" (id TEXT PRIMARY KEY NOT NULL, \"email\" TEXT NOT NULL UNIQUE, \"age\" INTEGER)"
        );
    }

    #[test]
    fn create_table_sql_escapes_identifiers() {
        let fields = vec![FieldSpec {
            name: "col\"x".into(),
            field_type: "string".into(),
            optional: false,
            unique: false,
        }];
        let sql = create_table_sql("my\"table", &fields);
        assert!(sql.contains("\"my\"\"table\""));
        assert!(sql.contains("\"col\"\"x\""));
    }

    #[test]
    fn create_index_sql_unique() {
        let sql = create_index_sql("User", "by_email", &["email".into()], true);
        assert_eq!(
            sql,
            "CREATE UNIQUE INDEX IF NOT EXISTS \"User_by_email\" ON \"User\" (\"email\")"
        );
    }

    #[test]
    fn create_index_sql_non_unique() {
        let sql = create_index_sql("Todo", "by_user", &["userId".into()], false);
        assert_eq!(
            sql,
            "CREATE INDEX IF NOT EXISTS \"Todo_by_user\" ON \"Todo\" (\"userId\")"
        );
    }

    #[test]
    fn add_column_sql_basic() {
        let field = FieldSpec {
            name: "bio".into(),
            field_type: "string".into(),
            optional: true,
            unique: false,
        };
        let sql = add_column_sql("User", &field);
        assert_eq!(sql, "ALTER TABLE \"User\" ADD COLUMN \"bio\" TEXT");
    }

    #[test]
    fn plan_from_manifest() {
        let adapter = PostgresAdapter;
        let manifest = test_manifest();
        let plan = adapter.plan_schema(&manifest).unwrap();

        // Should have CreateEntity for User and Todo, plus AddIndex for by_user.
        assert!(plan.operations.iter().any(|op| matches!(
            op,
            SchemaOperation::CreateEntity { name, .. } if name == "User"
        )));
        assert!(plan.operations.iter().any(|op| matches!(
            op,
            SchemaOperation::CreateEntity { name, .. } if name == "Todo"
        )));
        assert!(plan.operations.iter().any(|op| matches!(
            op,
            SchemaOperation::AddIndex { entity, name, .. } if entity == "Todo" && name == "by_user"
        )));
    }

    #[test]
    fn plan_to_sql_produces_statements() {
        let adapter = PostgresAdapter;
        let manifest = test_manifest();
        let plan = adapter.plan_schema(&manifest).unwrap();
        let stmts = plan_to_sql(&plan).unwrap();

        // 2 CREATE TABLE (User, Todo) + 1 CREATE INDEX for Todo.by_user
        // + 1 CREATE INDEX for Todo.by_user_done. The Todo manifest also
        // declares a unique by_email index on User which lands as part of
        // the table. Final count: 2 tables + 2 indexes.
        let create_tables = stmts.iter().filter(|s| s.starts_with("CREATE TABLE")).count();
        let create_indexes = stmts.iter().filter(|s| s.starts_with("CREATE INDEX") || s.starts_with("CREATE UNIQUE INDEX")).count();
        assert_eq!(create_tables, 2);
        assert!(create_indexes >= 1);
        assert!(stmts[0].starts_with("CREATE TABLE"));
        assert!(stmts[1].starts_with("CREATE TABLE"));
    }

    #[test]
    fn plan_to_sql_rejects_unsupported() {
        let plan = SchemaPlan {
            operations: vec![SchemaOperation::RemoveEntity {
                name: "User".into(),
            }],
        };
        let result = plan_to_sql(&plan);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, "PG_OP_UNSUPPORTED");
    }

    #[test]
    fn apply_not_implemented() {
        let adapter = PostgresAdapter;
        let plan = SchemaPlan {
            operations: vec![SchemaOperation::Noop],
        };
        let result = adapter.apply_schema(&plan);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, "APPLY_NOT_IMPLEMENTED");
    }

    #[test]
    fn sql_uses_quoted_identifiers() {
        let fields = vec![FieldSpec {
            name: "createdAt".into(),
            field_type: "datetime".into(),
            optional: false,
            unique: false,
        }];
        let sql = create_table_sql("User", &fields);
        // Postgres identifiers should be quoted for case-sensitivity.
        assert!(sql.contains("\"User\""));
        assert!(sql.contains("\"createdAt\""));
        assert!(sql.contains("TIMESTAMPTZ"));
    }

    // -- Introspection SQL tests --

    #[test]
    fn introspect_sql_constants_are_valid() {
        // Sanity checks that the SQL strings exist and look reasonable.
        assert!(INTROSPECT_TABLES_SQL.contains("information_schema.tables"));
        assert!(INTROSPECT_COLUMNS_SQL.contains("$1"));
        assert!(INTROSPECT_INDEXES_SQL.contains("$1"));
        assert!(INTROSPECT_TABLES_SQL.contains("_pylon_"));
    }

    // -- Plan from snapshot tests --

    #[test]
    fn plan_from_empty_snapshot_creates_all() {
        let snapshot = crate::SchemaSnapshot { tables: vec![] };
        let manifest = test_manifest();
        let plan = plan_from_snapshot(&snapshot, &manifest);

        assert!(plan.operations.iter().any(|op| matches!(
            op,
            SchemaOperation::CreateEntity { name, .. } if name == "User"
        )));
        assert!(plan.operations.iter().any(|op| matches!(
            op,
            SchemaOperation::CreateEntity { name, .. } if name == "Todo"
        )));
        assert!(plan.operations.iter().any(|op| matches!(
            op,
            SchemaOperation::AddIndex { entity, name, .. } if entity == "Todo" && name == "by_user"
        )));
    }

    #[test]
    fn plan_from_full_snapshot_is_noop() {
        let snapshot = crate::SchemaSnapshot {
            tables: vec![
                crate::TableSnapshot {
                    name: "User".into(),
                    columns: vec![
                        crate::ColumnSnapshot {
                            name: "id".into(),
                            column_type: "TEXT".into(),
                            notnull: true,
                            primary_key: true,
                        },
                        crate::ColumnSnapshot {
                            name: "email".into(),
                            column_type: "TEXT".into(),
                            notnull: true,
                            primary_key: false,
                        },
                        crate::ColumnSnapshot {
                            name: "displayName".into(),
                            column_type: "TEXT".into(),
                            notnull: true,
                            primary_key: false,
                        },
                        crate::ColumnSnapshot {
                            name: "createdAt".into(),
                            column_type: "TIMESTAMPTZ".into(),
                            notnull: true,
                            primary_key: false,
                        },
                    ],
                    indexes: vec![],
                },
                crate::TableSnapshot {
                    name: "Todo".into(),
                    columns: vec![
                        crate::ColumnSnapshot {
                            name: "id".into(),
                            column_type: "TEXT".into(),
                            notnull: true,
                            primary_key: true,
                        },
                        crate::ColumnSnapshot {
                            name: "title".into(),
                            column_type: "TEXT".into(),
                            notnull: true,
                            primary_key: false,
                        },
                        crate::ColumnSnapshot {
                            name: "done".into(),
                            column_type: "BOOLEAN".into(),
                            notnull: true,
                            primary_key: false,
                        },
                        crate::ColumnSnapshot {
                            name: "userId".into(),
                            column_type: "TEXT".into(),
                            notnull: true,
                            primary_key: false,
                        },
                        crate::ColumnSnapshot {
                            name: "createdAt".into(),
                            column_type: "TIMESTAMPTZ".into(),
                            notnull: true,
                            primary_key: false,
                        },
                    ],
                    indexes: vec![crate::IndexSnapshot {
                        name: "Todo_by_user".into(),
                        columns: vec!["userId".into()],
                        unique: false,
                    }],
                },
            ],
        };
        let manifest = test_manifest();
        let plan = plan_from_snapshot(&snapshot, &manifest);
        assert!(plan.is_empty());
    }

    #[test]
    fn plan_detects_missing_column_in_snapshot() {
        let snapshot = crate::SchemaSnapshot {
            tables: vec![
                crate::TableSnapshot {
                    name: "User".into(),
                    columns: vec![
                        crate::ColumnSnapshot {
                            name: "id".into(),
                            column_type: "TEXT".into(),
                            notnull: true,
                            primary_key: true,
                        },
                        crate::ColumnSnapshot {
                            name: "email".into(),
                            column_type: "TEXT".into(),
                            notnull: true,
                            primary_key: false,
                        },
                        // missing displayName and createdAt
                    ],
                    indexes: vec![],
                },
                crate::TableSnapshot {
                    name: "Todo".into(),
                    columns: vec![
                        crate::ColumnSnapshot {
                            name: "id".into(),
                            column_type: "TEXT".into(),
                            notnull: true,
                            primary_key: true,
                        },
                        crate::ColumnSnapshot {
                            name: "title".into(),
                            column_type: "TEXT".into(),
                            notnull: true,
                            primary_key: false,
                        },
                        crate::ColumnSnapshot {
                            name: "done".into(),
                            column_type: "BOOLEAN".into(),
                            notnull: true,
                            primary_key: false,
                        },
                        crate::ColumnSnapshot {
                            name: "userId".into(),
                            column_type: "TEXT".into(),
                            notnull: true,
                            primary_key: false,
                        },
                        crate::ColumnSnapshot {
                            name: "createdAt".into(),
                            column_type: "TIMESTAMPTZ".into(),
                            notnull: true,
                            primary_key: false,
                        },
                    ],
                    indexes: vec![crate::IndexSnapshot {
                        name: "Todo_by_user".into(),
                        columns: vec!["userId".into()],
                        unique: false,
                    }],
                },
            ],
        };
        let manifest = test_manifest();
        let plan = plan_from_snapshot(&snapshot, &manifest);

        let add_fields: Vec<_> = plan
            .operations
            .iter()
            .filter(|op| matches!(op, SchemaOperation::AddField { .. }))
            .collect();
        assert_eq!(add_fields.len(), 2); // displayName + createdAt
    }

    // -- CRUD helper tests (no live database required) --

    #[test]
    fn json_value_to_string_handles_all_types() {
        assert_eq!(
            json_value_to_string(&serde_json::Value::String("hello".into())),
            "hello"
        );
        assert_eq!(json_value_to_string(&serde_json::json!(42)), "42");
        assert_eq!(json_value_to_string(&serde_json::json!(1.5)), "1.5");
        assert_eq!(json_value_to_string(&serde_json::Value::Bool(true)), "true");
        assert_eq!(
            json_value_to_string(&serde_json::Value::Bool(false)),
            "false"
        );
        assert_eq!(json_value_to_string(&serde_json::Value::Null), "");
        // Arrays and objects get their JSON representation.
        assert_eq!(
            json_value_to_string(&serde_json::json!([1, 2, 3])),
            "[1,2,3]"
        );
        assert_eq!(
            json_value_to_string(&serde_json::json!({"a": 1})),
            "{\"a\":1}"
        );
    }

    #[test]
    fn generate_id_returns_hex_string() {
        let id = generate_id();
        assert!(!id.is_empty());
        // Must be valid hex characters.
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn generate_id_is_unique_across_calls() {
        let id1 = generate_id();
        let id2 = generate_id();
        assert_ne!(id1, id2);
    }

    #[test]
    fn generate_id_is_lex_sortable() {
        // 1000 IDs back-to-back must come out in monotonically increasing
        // lexicographic order. This is what makes cursor pagination correct.
        let mut ids: Vec<String> = (0..1000).map(|_| generate_id()).collect();
        let sorted = {
            let mut s = ids.clone();
            s.sort();
            s
        };
        assert_eq!(ids, sorted, "generate_id must be lex-monotonic");
        // And every id must be the same width (otherwise lex comparison is
        // wrong at width boundaries).
        let len0 = ids[0].len();
        assert!(ids.iter().all(|id| id.len() == len0));
        ids.dedup();
        assert_eq!(ids.len(), 1000, "no collisions in a tight loop");
    }

    #[test]
    fn build_insert_sql_simple() {
        let data = serde_json::json!({
            "email": "alice@example.com",
            "displayName": "Alice"
        });
        let (sql, values) = build_insert_sql("User", &data).unwrap();

        assert!(sql.starts_with("INSERT INTO \"User\""));
        assert!(sql.contains("id"));
        assert!(sql.contains("$1"));
        assert!(sql.contains("$2"));
        assert!(sql.contains("$3"));
        // First value is the generated ID.
        assert!(!values[0].is_empty());
        assert_eq!(values.len(), 3); // id + 2 fields
    }

    #[test]
    fn build_insert_sql_quotes_column_names() {
        let data = serde_json::json!({"createdAt": "2026-01-01"});
        let (sql, _) = build_insert_sql("Todo", &data).unwrap();
        assert!(sql.contains("\"createdAt\""));
        assert!(sql.contains("\"Todo\""));
    }

    #[test]
    fn build_insert_sql_rejects_non_object() {
        let data = serde_json::json!("not an object");
        let result = build_insert_sql("User", &data);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, "PG_INVALID_DATA");
    }

    #[test]
    fn build_update_sql_simple() {
        let data = serde_json::json!({
            "displayName": "Bob",
            "email": "bob@example.com"
        });
        let (sql, values) = build_update_sql("User", "abc123", &data).unwrap();

        assert!(sql.starts_with("UPDATE \"User\" SET"));
        assert!(sql.contains("WHERE id = $1"));
        assert!(sql.contains("$2"));
        assert!(sql.contains("$3"));
        assert_eq!(values[0], "abc123");
        assert_eq!(values.len(), 3); // id + 2 fields
    }

    #[test]
    fn build_update_sql_quotes_column_names() {
        let data = serde_json::json!({"displayName": "Carol"});
        let (sql, _) = build_update_sql("User", "id1", &data).unwrap();
        assert!(sql.contains("\"displayName\" = $2"));
    }

    #[test]
    fn build_update_sql_rejects_non_object() {
        let data = serde_json::json!(42);
        let result = build_update_sql("User", "id1", &data);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, "PG_INVALID_DATA");
    }

    #[test]
    fn build_update_sql_rejects_empty_object() {
        let data = serde_json::json!({});
        let err = build_update_sql("User", "id1", &data).unwrap_err();
        assert_eq!(err.code, "PG_INVALID_DATA");
        assert!(err.message.contains("at least one field"));
    }
}
