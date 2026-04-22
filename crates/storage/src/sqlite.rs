use std::collections::BTreeMap;

use rusqlite::Connection;
use serde::Serialize;

use crate::{
    ColumnSnapshot, FieldSpec, IndexSnapshot, SchemaOperation, SchemaPlan, SchemaSnapshot,
    StorageAdapter, StorageError, TableSnapshot,
};
use statecraft_core::AppManifest;

// ---------------------------------------------------------------------------
// Type mapping: manifest field types -> SQLite column types
//
//   string    -> TEXT
//   int       -> INTEGER
//   float     -> REAL
//   bool      -> INTEGER
//   datetime  -> TEXT
//   richtext  -> TEXT
//   id(...)   -> TEXT
// ---------------------------------------------------------------------------

fn sqlite_column_type(field_type: &str) -> &'static str {
    match field_type {
        "string" => "TEXT",
        "int" => "INTEGER",
        "float" => "REAL",
        "bool" => "INTEGER",
        "datetime" => "TEXT",
        "richtext" => "TEXT",
        _ if field_type.starts_with("id(") => "TEXT",
        _ => "TEXT",
    }
}

// ---------------------------------------------------------------------------
// SQL identifier quoting
// ---------------------------------------------------------------------------

/// Quote a SQLite identifier using double-quotes, escaping any embedded
/// double-quote characters by doubling them (SQL standard).
fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

// ---------------------------------------------------------------------------
// SQL generation
// ---------------------------------------------------------------------------

/// Generate a CREATE TABLE statement for an entity.
pub fn create_table_sql(entity_name: &str, fields: &[FieldSpec]) -> String {
    let mut columns = vec!["id TEXT PRIMARY KEY NOT NULL".to_string()];

    for field in fields {
        let col_type = sqlite_column_type(&field.field_type);
        let not_null = if field.optional { "" } else { " NOT NULL" };
        let unique = if field.unique { " UNIQUE" } else { "" };
        columns.push(format!("{} {}{}{}", quote_ident(&field.name), col_type, not_null, unique));
    }

    format!("CREATE TABLE IF NOT EXISTS {} ({})", quote_ident(entity_name), columns.join(", "))
}

/// Generate an ALTER TABLE ADD COLUMN statement.
pub fn add_column_sql(entity_name: &str, field: &FieldSpec) -> String {
    let col_type = sqlite_column_type(&field.field_type);
    // SQLite ALTER TABLE ADD COLUMN does not support NOT NULL without a default for existing rows.
    // For optional fields, omit NOT NULL. For required fields, we still omit NOT NULL here
    // because SQLite requires a default value for ADD COLUMN NOT NULL.
    let unique = if field.unique { " UNIQUE" } else { "" };
    format!(
        "ALTER TABLE {} ADD COLUMN {} {}{}",
        quote_ident(entity_name),
        quote_ident(&field.name),
        col_type,
        unique,
    )
}

/// Generate a CREATE INDEX statement.
pub fn create_index_sql(entity_name: &str, index_name: &str, fields: &[String], unique: bool) -> String {
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
// SqliteAdapter
// ---------------------------------------------------------------------------

pub struct SqliteAdapter {
    conn: Connection,
}

impl SqliteAdapter {
    /// Open or create a SQLite database at the given path.
    pub fn open(path: &str) -> Result<Self, StorageError> {
        let conn = Connection::open(path).map_err(|e| StorageError {
            code: "SQLITE_OPEN_FAILED".into(),
            message: format!("Failed to open SQLite database at {path}: {e}"),
        })?;
        Ok(Self { conn })
    }

    /// Create an in-memory SQLite database.
    pub fn in_memory() -> Result<Self, StorageError> {
        let conn = Connection::open_in_memory().map_err(|e| StorageError {
            code: "SQLITE_OPEN_FAILED".into(),
            message: format!("Failed to open in-memory SQLite database: {e}"),
        })?;
        Ok(Self { conn })
    }
}

impl SqliteAdapter {
    /// Plan schema changes by comparing the live DB state against the target manifest.
    /// Only plans additive operations: CreateEntity, AddField, AddIndex.
    pub fn plan_from_live(&self, target: &AppManifest) -> Result<SchemaPlan, StorageError> {
        let snapshot = self.read_schema()?;
        Ok(crate::plan_from_snapshot(&snapshot, target))
    }
}

impl StorageAdapter for SqliteAdapter {
    fn plan_schema(&self, target: &AppManifest) -> Result<SchemaPlan, StorageError> {
        // Plan from live DB state.
        self.plan_from_live(target)
    }

    fn apply_schema(&self, plan: &SchemaPlan) -> Result<(), StorageError> {
        // Wrap the whole plan in a single transaction so that if operation N
        // fails, operations 1..N are rolled back. Without this, a partial
        // migration would leave the database in an inconsistent state that
        // doesn't match either the old or the new manifest.
        self.conn.execute("BEGIN", []).map_err(|e| StorageError {
            code: "SQLITE_EXEC_FAILED".into(),
            message: format!("BEGIN failed: {e}"),
        })?;
        match self.apply_schema_impl(plan) {
            Ok(()) => {
                self.conn.execute("COMMIT", []).map_err(|e| StorageError {
                    code: "SQLITE_EXEC_FAILED".into(),
                    message: format!("COMMIT failed after apply: {e}"),
                })?;
                Ok(())
            }
            Err(e) => {
                if let Err(rb) = self.conn.execute("ROLLBACK", []) {
                    // Log both — a failed rollback leaves the connection in
                    // a broken state but the original error is what the
                    // caller cares about.
                    tracing::warn!("[sqlite] ROLLBACK after apply error failed: {rb}");
                }
                Err(e)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Migration history
// ---------------------------------------------------------------------------

const HISTORY_TABLE: &str = "_statecraft_schema_history";

/// A single row from the schema push history table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HistoryEntry {
    pub id: String,
    pub manifest_version: i64,
    pub app_version: String,
    pub applied_at: String,
    pub operation_count: i64,
    pub baseline: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<SchemaPlan>,
    pub plan_json: String,
}

/// Metadata for a schema push event.
pub struct PushMetadata<'a> {
    pub manifest_version: u32,
    pub app_version: &'a str,
    pub baseline: &'a str,
}

impl SqliteAdapter {
    /// Ensure the history table exists.
    fn ensure_history_table(&self) -> Result<(), StorageError> {
        let sql = format!(
            "CREATE TABLE IF NOT EXISTS {} (\
                id TEXT PRIMARY KEY NOT NULL, \
                manifest_version INTEGER NOT NULL, \
                app_version TEXT NOT NULL, \
                applied_at TEXT NOT NULL, \
                operation_count INTEGER NOT NULL, \
                baseline TEXT NOT NULL, \
                plan_json TEXT NOT NULL\
            )",
            quote_ident(HISTORY_TABLE)
        );
        self.conn.execute(&sql, []).map_err(|e| StorageError {
            code: "SQLITE_EXEC_FAILED".into(),
            message: format!("Failed to create history table: {e}"),
        })?;
        Ok(())
    }

    /// Apply a schema plan and record the push in the history table —
    /// atomically. If either the DDL or the history INSERT fails, the
    /// whole transaction rolls back so the database never ends up with a
    /// schema change that has no history row, or a history row that
    /// points at a failed migration.
    pub fn apply_with_history(
        &self,
        plan: &SchemaPlan,
        meta: &PushMetadata<'_>,
    ) -> Result<(), StorageError> {
        // History table creation runs OUTSIDE the transaction because
        // CREATE TABLE IF NOT EXISTS is a cheap idempotent bootstrap and
        // can safely predate the real migration atomicity boundary.
        self.ensure_history_table()?;

        self.conn.execute("BEGIN", []).map_err(|e| StorageError {
            code: "SQLITE_EXEC_FAILED".into(),
            message: format!("BEGIN failed: {e}"),
        })?;

        let result = (|| -> Result<(), StorageError> {
            self.apply_schema_impl(plan)?;

            let plan_json = serde_json::to_string(plan).map_err(|e| StorageError {
                code: "SQLITE_SERIALIZE_FAILED".into(),
                message: format!("Failed to serialize plan: {e}"),
            })?;

            let id = generate_push_id();
            let now = now_iso8601();
            let op_count = plan
                .operations
                .iter()
                .filter(|op| !matches!(op, SchemaOperation::Noop))
                .count() as i64;

            self.conn
                .execute(
                    &format!(
                        "INSERT INTO {} (id, manifest_version, app_version, applied_at, operation_count, baseline, plan_json) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                        quote_ident(HISTORY_TABLE)
                    ),
                    rusqlite::params![
                        id,
                        meta.manifest_version as i64,
                        meta.app_version,
                        now,
                        op_count,
                        meta.baseline,
                        plan_json,
                    ],
                )
                .map_err(|e| StorageError {
                    code: "SQLITE_EXEC_FAILED".into(),
                    message: format!("Failed to insert history row: {e}"),
                })?;
            Ok(())
        })();

        match result {
            Ok(()) => {
                self.conn.execute("COMMIT", []).map_err(|e| StorageError {
                    code: "SQLITE_EXEC_FAILED".into(),
                    message: format!("COMMIT failed: {e}"),
                })?;
                Ok(())
            }
            Err(e) => {
                if let Err(rb) = self.conn.execute("ROLLBACK", []) {
                    tracing::warn!("[sqlite] ROLLBACK after apply_with_history error failed: {rb}");
                }
                Err(e)
            }
        }
    }

    /// Read schema push history, newest-first.
    /// Returns empty vec if the history table does not exist.
    pub fn read_history(&self, limit: Option<u32>) -> Result<Vec<HistoryEntry>, StorageError> {
        if !self.history_table_exists()? {
            return Ok(Vec::new());
        }

        let quoted = quote_ident(HISTORY_TABLE);
        let sql = match limit {
            Some(n) => format!(
                "SELECT id, manifest_version, app_version, applied_at, operation_count, baseline, plan_json \
                 FROM {} ORDER BY id DESC LIMIT {}",
                quoted, n
            ),
            None => format!(
                "SELECT id, manifest_version, app_version, applied_at, operation_count, baseline, plan_json \
                 FROM {} ORDER BY id DESC",
                quoted
            ),
        };

        let mut stmt = self.conn.prepare(&sql).map_err(sqlite_err)?;

        let entries = stmt
            .query_map([], |row| {
                let plan_json: String = row.get(6)?;
                let plan = serde_json::from_str(&plan_json).ok();
                Ok(HistoryEntry {
                    id: row.get(0)?,
                    manifest_version: row.get(1)?,
                    app_version: row.get(2)?,
                    applied_at: row.get(3)?,
                    operation_count: row.get(4)?,
                    baseline: row.get(5)?,
                    plan,
                    plan_json,
                })
            })
            .map_err(sqlite_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sqlite_err)?;

        Ok(entries)
    }

    /// Read a single history entry by ID.
    /// Returns None if the history table doesn't exist or the ID is not found.
    pub fn read_history_entry(&self, entry_id: &str) -> Result<Option<HistoryEntry>, StorageError> {
        if !self.history_table_exists()? {
            return Ok(None);
        }

        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT id, manifest_version, app_version, applied_at, operation_count, baseline, plan_json \
                 FROM {} WHERE id = ?1",
                quote_ident(HISTORY_TABLE)
            ))
            .map_err(sqlite_err)?;

        let mut rows = stmt
            .query_map([entry_id], |row| {
                let plan_json: String = row.get(6)?;
                let plan = serde_json::from_str(&plan_json).ok();
                Ok(HistoryEntry {
                    id: row.get(0)?,
                    manifest_version: row.get(1)?,
                    app_version: row.get(2)?,
                    applied_at: row.get(3)?,
                    operation_count: row.get(4)?,
                    baseline: row.get(5)?,
                    plan,
                    plan_json,
                })
            })
            .map_err(sqlite_err)?;

        match rows.next() {
            Some(Ok(entry)) => Ok(Some(entry)),
            Some(Err(e)) => Err(sqlite_err(e)),
            None => Ok(None),
        }
    }

    fn history_table_exists(&self) -> Result<bool, StorageError> {
        let exists: bool = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                [HISTORY_TABLE],
                |row| row.get::<_, i64>(0),
            )
            .map_err(sqlite_err)?
            > 0;
        Ok(exists)
    }

    /// Internal apply implementation shared by both `apply_schema` and `apply_with_history`.
    fn apply_schema_impl(&self, plan: &SchemaPlan) -> Result<(), StorageError> {
        for op in &plan.operations {
            match op {
                SchemaOperation::CreateEntity { name, fields } => {
                    let sql = create_table_sql(name, fields);
                    self.conn.execute(&sql, []).map_err(|e| StorageError {
                        code: "SQLITE_EXEC_FAILED".into(),
                        message: format!("Failed to create table {name}: {e}"),
                    })?;
                }
                SchemaOperation::AddField { entity, field } => {
                    let sql = add_column_sql(entity, field);
                    self.conn.execute(&sql, []).map_err(|e| StorageError {
                        code: "SQLITE_EXEC_FAILED".into(),
                        message: format!("Failed to add column {}.{}: {e}", entity, field.name),
                    })?;
                }
                SchemaOperation::AddIndex {
                    entity,
                    name,
                    fields,
                    unique,
                } => {
                    let sql = create_index_sql(entity, name, fields, *unique);
                    self.conn.execute(&sql, []).map_err(|e| StorageError {
                        code: "SQLITE_EXEC_FAILED".into(),
                        message: format!("Failed to create index {entity}.{name}: {e}"),
                    })?;
                }
                SchemaOperation::Noop => {}
                other => {
                    return Err(StorageError {
                        code: "SQLITE_OP_UNSUPPORTED".into(),
                        message: format!("Operation not supported by SQLite adapter: {other:?}"),
                    });
                }
            }
        }
        Ok(())
    }
}

fn generate_push_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}.{:09}", ts.as_secs(), ts.subsec_nanos())
}

fn now_iso8601() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Simple UTC timestamp. Not worth pulling in chrono for this.
    let secs_per_day: u64 = 86400;
    let days = ts / secs_per_day;
    let rem = ts % secs_per_day;
    let hours = rem / 3600;
    let mins = (rem % 3600) / 60;
    let secs = rem % 60;
    // Approximate date from epoch days (good enough for audit purposes).
    let (year, month, day) = epoch_days_to_date(days);
    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{mins:02}:{secs:02}Z")
}

fn epoch_days_to_date(days: u64) -> (u64, u64, u64) {
    // Civil date from epoch days. Algorithm from Howard Hinnant.
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

// ---------------------------------------------------------------------------
// Introspection
// ---------------------------------------------------------------------------

impl SqliteAdapter {
    /// Read the current schema from the live SQLite database.
    /// Only inspects user tables (not sqlite_* internal tables).
    pub fn read_schema(&self) -> Result<SchemaSnapshot, StorageError> {
        // Get all user tables, sorted for determinism.
        let mut stmt = self
            .conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name NOT LIKE '_statecraft_%' ORDER BY name")
            .map_err(sqlite_err)?;

        let table_names: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .map_err(sqlite_err)?
            .collect::<Result<Vec<String>, _>>()
            .map_err(sqlite_err)?;

        let mut tables = Vec::new();
        for table_name in &table_names {
            let columns = self.read_columns(table_name)?;
            let indexes = self.read_indexes(table_name)?;
            tables.push(TableSnapshot {
                name: table_name.clone(),
                columns,
                indexes,
            });
        }

        Ok(SchemaSnapshot { tables })
    }

    fn read_columns(&self, table: &str) -> Result<Vec<ColumnSnapshot>, StorageError> {
        let mut stmt = self
            .conn
            .prepare(&format!("PRAGMA table_info({})", quote_ident(table)))
            .map_err(sqlite_err)?;

        let columns: Vec<ColumnSnapshot> = stmt
            .query_map([], |row| {
                Ok(ColumnSnapshot {
                    name: row.get(1)?,
                    column_type: row.get(2)?,
                    notnull: row.get::<_, i32>(3)? != 0,
                    primary_key: row.get::<_, i32>(5)? != 0,
                })
            })
            .map_err(sqlite_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sqlite_err)?;

        Ok(columns)
    }

    fn read_indexes(&self, table: &str) -> Result<Vec<IndexSnapshot>, StorageError> {
        let mut stmt = self
            .conn
            .prepare(&format!("PRAGMA index_list({})", quote_ident(table)))
            .map_err(sqlite_err)?;

        // Collect index metadata: (name, unique).
        let index_meta: Vec<(String, bool)> = stmt
            .query_map([], |row| {
                let name: String = row.get(1)?;
                let unique: bool = row.get::<_, i32>(2)? != 0;
                Ok((name, unique))
            })
            .map_err(sqlite_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sqlite_err)?;

        // Build ordered map for determinism, then read columns for each index.
        let ordered: BTreeMap<String, bool> = index_meta.into_iter().collect();

        let mut indexes = Vec::new();
        for (name, unique) in &ordered {
            // Skip SQLite autoindexes (internal unique constraint indexes).
            if name.starts_with("sqlite_autoindex_") {
                continue;
            }

            let mut col_stmt = self
                .conn
                .prepare(&format!("PRAGMA index_info({})", quote_ident(name)))
                .map_err(sqlite_err)?;

            let columns: Vec<String> = col_stmt
                .query_map([], |row| row.get(2))
                .map_err(sqlite_err)?
                .collect::<Result<Vec<String>, _>>()
                .map_err(sqlite_err)?;

            indexes.push(IndexSnapshot {
                name: name.clone(),
                columns,
                unique: *unique,
            });
        }

        Ok(indexes)
    }
}

fn sqlite_err(e: rusqlite::Error) -> StorageError {
    StorageError {
        code: "SQLITE_QUERY_FAILED".into(),
        message: format!("SQLite query failed: {e}"),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use statecraft_core::*;

    fn test_manifest() -> AppManifest {
        AppManifest {
            manifest_version: MANIFEST_VERSION,
            name: "test".into(),
            version: "0.1.0".into(),
            entities: vec![ManifestEntity {
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
                    ManifestField {
                        name: "age".into(),
                        field_type: "int".into(),
                        optional: true,
                        unique: false,
                    },
                ],
                indexes: vec![ManifestIndex {
                    name: "by_email".into(),
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
    fn create_table_sql_basic() {
        let fields = vec![
            FieldSpec { name: "email".into(), field_type: "string".into(), optional: false, unique: true },
            FieldSpec { name: "age".into(), field_type: "int".into(), optional: true, unique: false },
        ];
        let sql = create_table_sql("User", &fields);
        assert_eq!(
            sql,
            "CREATE TABLE IF NOT EXISTS \"User\" (id TEXT PRIMARY KEY NOT NULL, \"email\" TEXT NOT NULL UNIQUE, \"age\" INTEGER)"
        );
    }

    #[test]
    fn create_index_sql_basic() {
        let sql = create_index_sql("User", "by_email", &["email".into()], true);
        assert_eq!(sql, "CREATE UNIQUE INDEX IF NOT EXISTS \"User_by_email\" ON \"User\" (\"email\")");
    }

    #[test]
    fn create_index_sql_non_unique() {
        let sql = create_index_sql("Todo", "by_author", &["authorId".into()], false);
        assert_eq!(sql, "CREATE INDEX IF NOT EXISTS \"Todo_by_author\" ON \"Todo\" (\"authorId\")");
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
    fn quote_ident_escapes_double_quotes() {
        assert_eq!(quote_ident("normal"), "\"normal\"");
        assert_eq!(quote_ident("has\"quote"), "\"has\"\"quote\"");
        assert_eq!(quote_ident("Robert'); DROP TABLE Students;--"), "\"Robert'); DROP TABLE Students;--\"");
    }

    #[test]
    fn sqlite_adapter_creates_table() {
        let adapter = SqliteAdapter::in_memory().unwrap();
        let manifest = test_manifest();
        let plan = adapter.plan_schema(&manifest).unwrap();
        adapter.apply_schema(&plan).unwrap();

        // Verify table exists by querying sqlite_master.
        let table_count: i64 = adapter
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='User'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(table_count, 1);
    }

    #[test]
    fn sqlite_adapter_creates_index() {
        let adapter = SqliteAdapter::in_memory().unwrap();
        let manifest = test_manifest();
        let plan = adapter.plan_schema(&manifest).unwrap();
        adapter.apply_schema(&plan).unwrap();

        // Verify index exists.
        let index_count: i64 = adapter
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='User_by_email'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(index_count, 1);
    }

    #[test]
    fn sqlite_adapter_add_field() {
        let adapter = SqliteAdapter::in_memory().unwrap();

        // Create table first.
        let manifest = test_manifest();
        let plan = adapter.plan_schema(&manifest).unwrap();
        adapter.apply_schema(&plan).unwrap();

        // Add a field.
        let add_plan = SchemaPlan {
            operations: vec![SchemaOperation::AddField {
                entity: "User".into(),
                field: FieldSpec {
                    name: "bio".into(),
                    field_type: "string".into(),
                    optional: true,
                    unique: false,
                },
            }],
        };
        adapter.apply_schema(&add_plan).unwrap();

        // Verify column exists by checking pragma.
        let has_bio: bool = adapter
            .conn
            .prepare("PRAGMA table_info(\"User\")")
            .unwrap()
            .query_map([], |row| {
                let name: String = row.get(1)?;
                Ok(name)
            })
            .unwrap()
            .any(|r| r.unwrap() == "bio");
        assert!(has_bio);
    }

    #[test]
    fn sqlite_adapter_rejects_remove_entity() {
        let adapter = SqliteAdapter::in_memory().unwrap();
        let plan = SchemaPlan {
            operations: vec![SchemaOperation::RemoveEntity {
                name: "User".into(),
            }],
        };
        let result = adapter.apply_schema(&plan);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, "SQLITE_OP_UNSUPPORTED");
    }

    #[test]
    fn sqlite_adapter_rejects_remove_field() {
        let adapter = SqliteAdapter::in_memory().unwrap();
        let plan = SchemaPlan {
            operations: vec![SchemaOperation::RemoveField {
                entity: "User".into(),
                field_name: "email".into(),
            }],
        };
        let result = adapter.apply_schema(&plan);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, "SQLITE_OP_UNSUPPORTED");
    }

    #[test]
    fn sqlite_adapter_column_types() {
        assert_eq!(sqlite_column_type("string"), "TEXT");
        assert_eq!(sqlite_column_type("int"), "INTEGER");
        assert_eq!(sqlite_column_type("float"), "REAL");
        assert_eq!(sqlite_column_type("bool"), "INTEGER");
        assert_eq!(sqlite_column_type("datetime"), "TEXT");
        assert_eq!(sqlite_column_type("richtext"), "TEXT");
        assert_eq!(sqlite_column_type("id(User)"), "TEXT");
    }

    // -- Introspection tests --

    #[test]
    fn introspect_empty_db() {
        let adapter = SqliteAdapter::in_memory().unwrap();
        let snapshot = adapter.read_schema().unwrap();
        assert!(snapshot.tables.is_empty());
    }

    #[test]
    fn introspect_after_apply() {
        let adapter = SqliteAdapter::in_memory().unwrap();
        let manifest = test_manifest();
        let plan = adapter.plan_schema(&manifest).unwrap();
        adapter.apply_schema(&plan).unwrap();

        let snapshot = adapter.read_schema().unwrap();

        // Should have one table.
        assert_eq!(snapshot.tables.len(), 1);
        let user = &snapshot.tables[0];
        assert_eq!(user.name, "User");

        // id + 3 manifest fields = 4 columns.
        assert_eq!(user.columns.len(), 4);
        assert_eq!(user.columns[0].name, "id");
        assert!(user.columns[0].primary_key);
        assert_eq!(user.columns[1].name, "email");
        assert_eq!(user.columns[1].column_type, "TEXT");
        assert!(user.columns[1].notnull);
        assert_eq!(user.columns[2].name, "displayName");
        assert_eq!(user.columns[3].name, "age");
        assert!(!user.columns[3].notnull); // optional

        // Should have the by_email index.
        assert_eq!(user.indexes.len(), 1);
        assert_eq!(user.indexes[0].name, "User_by_email");
        assert_eq!(user.indexes[0].columns, vec!["email"]);
        assert!(user.indexes[0].unique);
    }

    #[test]
    fn introspect_multiple_tables() {
        let adapter = SqliteAdapter::in_memory().unwrap();

        let manifest = AppManifest {
            manifest_version: MANIFEST_VERSION,
            name: "test".into(),
            version: "0.1.0".into(),
            entities: vec![
                ManifestEntity {
                    name: "Post".into(),
                    fields: vec![ManifestField {
                        name: "title".into(),
                        field_type: "string".into(),
                        optional: false,
                        unique: false,
                    }],
                    indexes: vec![],
                relations: vec![],
                },
                ManifestEntity {
                    name: "User".into(),
                    fields: vec![ManifestField {
                        name: "email".into(),
                        field_type: "string".into(),
                        optional: false,
                        unique: true,
                    }],
                    indexes: vec![],
                relations: vec![],
                },
            ],
            routes: vec![],
            queries: vec![],
            actions: vec![],
            policies: vec![],
        };

        let plan = adapter.plan_schema(&manifest).unwrap();
        adapter.apply_schema(&plan).unwrap();

        let snapshot = adapter.read_schema().unwrap();

        // Sorted alphabetically.
        assert_eq!(snapshot.tables.len(), 2);
        assert_eq!(snapshot.tables[0].name, "Post");
        assert_eq!(snapshot.tables[1].name, "User");
    }

    #[test]
    fn introspect_after_add_field() {
        let adapter = SqliteAdapter::in_memory().unwrap();
        let manifest = test_manifest();
        let plan = adapter.plan_schema(&manifest).unwrap();
        adapter.apply_schema(&plan).unwrap();

        // Add a column.
        let add_plan = SchemaPlan {
            operations: vec![SchemaOperation::AddField {
                entity: "User".into(),
                field: FieldSpec {
                    name: "bio".into(),
                    field_type: "string".into(),
                    optional: true,
                    unique: false,
                },
            }],
        };
        adapter.apply_schema(&add_plan).unwrap();

        let snapshot = adapter.read_schema().unwrap();
        let user = &snapshot.tables[0];

        // id + 3 original + 1 added = 5.
        assert_eq!(user.columns.len(), 5);
        assert!(user.columns.iter().any(|c| c.name == "bio"));
    }

    #[test]
    fn introspect_snapshot_is_deterministic() {
        let adapter = SqliteAdapter::in_memory().unwrap();
        let manifest = test_manifest();
        let plan = adapter.plan_schema(&manifest).unwrap();
        adapter.apply_schema(&plan).unwrap();

        let s1 = adapter.read_schema().unwrap();
        let s2 = adapter.read_schema().unwrap();
        assert_eq!(s1, s2);
    }

    // -- Live planning tests --

    #[test]
    fn plan_from_empty_db_creates_everything() {
        let adapter = SqliteAdapter::in_memory().unwrap();
        let manifest = test_manifest();

        let plan = adapter.plan_from_live(&manifest).unwrap();

        // Should create the table and its index.
        assert!(plan.operations.iter().any(|op| matches!(
            op,
            SchemaOperation::CreateEntity { name, .. } if name == "User"
        )));
        assert!(plan.operations.iter().any(|op| matches!(
            op,
            SchemaOperation::AddIndex { entity, name, .. } if entity == "User" && name == "by_email"
        )));
    }

    #[test]
    fn plan_from_fully_applied_db_is_noop() {
        let adapter = SqliteAdapter::in_memory().unwrap();
        let manifest = test_manifest();

        // Apply everything first.
        let initial = adapter.plan_from_live(&manifest).unwrap();
        adapter.apply_schema(&initial).unwrap();

        // Plan again — should be noop.
        let plan = adapter.plan_from_live(&manifest).unwrap();
        assert!(plan.is_empty(), "expected noop, got: {:?}", plan.operations);
    }

    #[test]
    fn plan_detects_missing_column() {
        let adapter = SqliteAdapter::in_memory().unwrap();

        // Create table with only email.
        adapter
            .conn
            .execute(
                "CREATE TABLE \"User\" (id TEXT PRIMARY KEY NOT NULL, email TEXT NOT NULL UNIQUE)",
                [],
            )
            .unwrap();

        let manifest = test_manifest();
        let plan = adapter.plan_from_live(&manifest).unwrap();

        // Should plan AddField for displayName and age.
        let add_fields: Vec<_> = plan
            .operations
            .iter()
            .filter(|op| matches!(op, SchemaOperation::AddField { .. }))
            .collect();
        assert_eq!(add_fields.len(), 2);
    }

    #[test]
    fn plan_detects_missing_index() {
        let adapter = SqliteAdapter::in_memory().unwrap();

        // Create table with all columns but no index.
        adapter
            .conn
            .execute(
                "CREATE TABLE \"User\" (id TEXT PRIMARY KEY NOT NULL, email TEXT NOT NULL UNIQUE, \"displayName\" TEXT NOT NULL, age INTEGER)",
                [],
            )
            .unwrap();

        let manifest = test_manifest();
        let plan = adapter.plan_from_live(&manifest).unwrap();

        // Should plan AddIndex only.
        assert!(plan.operations.iter().any(|op| matches!(
            op,
            SchemaOperation::AddIndex { entity, name, .. } if entity == "User" && name == "by_email"
        )));
        assert!(!plan.operations.iter().any(|op| matches!(
            op,
            SchemaOperation::CreateEntity { .. }
        )));
    }

    // -- Migration history tests --

    fn push_meta(baseline: &str) -> PushMetadata<'_> {
        PushMetadata {
            manifest_version: 1,
            app_version: "0.1.0",
            baseline,
        }
    }

    fn history_count(adapter: &SqliteAdapter) -> i64 {
        adapter
            .conn
            .query_row(
                &format!("SELECT COUNT(*) FROM {}", quote_ident(HISTORY_TABLE)),
                [],
                |row| row.get(0),
            )
            .unwrap()
    }

    #[test]
    fn history_table_created_on_apply() {
        let adapter = SqliteAdapter::in_memory().unwrap();
        let manifest = test_manifest();
        let plan = adapter.plan_from_live(&manifest).unwrap();
        adapter.apply_with_history(&plan, &push_meta("live_sqlite")).unwrap();

        let table_exists: i64 = adapter
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                [HISTORY_TABLE],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(table_exists, 1);
    }

    #[test]
    fn history_row_inserted_on_apply() {
        let adapter = SqliteAdapter::in_memory().unwrap();
        let manifest = test_manifest();
        let plan = adapter.plan_from_live(&manifest).unwrap();
        adapter.apply_with_history(&plan, &push_meta("live_sqlite")).unwrap();

        assert_eq!(history_count(&adapter), 1);

        // Verify stored data.
        let (mv, av, baseline, op_count): (i64, String, String, i64) = adapter
            .conn
            .query_row(
                &format!(
                    "SELECT manifest_version, app_version, baseline, operation_count FROM {} LIMIT 1",
                    quote_ident(HISTORY_TABLE)
                ),
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(mv, 1);
        assert_eq!(av, "0.1.0");
        assert_eq!(baseline, "live_sqlite");
        assert_eq!(op_count, 2); // CreateEntity + AddIndex (Noop not counted)
    }

    #[test]
    fn noop_push_also_recorded() {
        let adapter = SqliteAdapter::in_memory().unwrap();
        let manifest = test_manifest();

        // First push creates tables.
        let plan1 = adapter.plan_from_live(&manifest).unwrap();
        adapter.apply_with_history(&plan1, &push_meta("live_sqlite")).unwrap();

        // Second push is noop.
        let plan2 = adapter.plan_from_live(&manifest).unwrap();
        assert!(plan2.is_empty());
        adapter.apply_with_history(&plan2, &push_meta("live_sqlite")).unwrap();

        // Both pushes recorded.
        assert_eq!(history_count(&adapter), 2);

        // Second row has 0 operations.
        let op_count: i64 = adapter
            .conn
            .query_row(
                &format!(
                    "SELECT operation_count FROM {} ORDER BY id DESC LIMIT 1",
                    quote_ident(HISTORY_TABLE)
                ),
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(op_count, 0);
    }

    #[test]
    fn history_plan_json_is_valid() {
        let adapter = SqliteAdapter::in_memory().unwrap();
        let manifest = test_manifest();
        let plan = adapter.plan_from_live(&manifest).unwrap();
        adapter.apply_with_history(&plan, &push_meta("live_sqlite")).unwrap();

        let plan_json: String = adapter
            .conn
            .query_row(
                &format!("SELECT plan_json FROM {} LIMIT 1", quote_ident(HISTORY_TABLE)),
                [],
                |row| row.get(0),
            )
            .unwrap();

        // Should be valid JSON.
        let parsed: serde_json::Value = serde_json::from_str(&plan_json).unwrap();
        assert!(parsed.get("operations").unwrap().is_array());
    }

    #[test]
    fn history_table_excluded_from_introspection() {
        let adapter = SqliteAdapter::in_memory().unwrap();
        let manifest = test_manifest();
        let plan = adapter.plan_from_live(&manifest).unwrap();
        adapter.apply_with_history(&plan, &push_meta("live_sqlite")).unwrap();

        let snapshot = adapter.read_schema().unwrap();
        assert!(!snapshot.tables.iter().any(|t| t.name.starts_with("_statecraft")));
    }

    // -- read_history tests --

    #[test]
    fn read_history_empty_db() {
        let adapter = SqliteAdapter::in_memory().unwrap();
        let entries = adapter.read_history(None).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn read_history_after_one_push() {
        let adapter = SqliteAdapter::in_memory().unwrap();
        let manifest = test_manifest();
        let plan = adapter.plan_from_live(&manifest).unwrap();
        adapter.apply_with_history(&plan, &push_meta("live_sqlite")).unwrap();

        let entries = adapter.read_history(None).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].manifest_version, 1);
        assert_eq!(entries[0].app_version, "0.1.0");
        assert_eq!(entries[0].baseline, "live_sqlite");
        assert_eq!(entries[0].operation_count, 2); // CreateEntity + AddIndex
    }

    #[test]
    fn read_history_after_noop_push() {
        let adapter = SqliteAdapter::in_memory().unwrap();
        let manifest = test_manifest();

        let plan1 = adapter.plan_from_live(&manifest).unwrap();
        adapter.apply_with_history(&plan1, &push_meta("live_sqlite")).unwrap();

        let plan2 = adapter.plan_from_live(&manifest).unwrap();
        adapter.apply_with_history(&plan2, &push_meta("live_sqlite")).unwrap();

        let entries = adapter.read_history(None).unwrap();
        assert_eq!(entries.len(), 2);
        // Newest first.
        assert_eq!(entries[0].operation_count, 0);
        assert_eq!(entries[1].operation_count, 2);
    }

    #[test]
    fn read_history_newest_first() {
        let adapter = SqliteAdapter::in_memory().unwrap();
        let manifest = test_manifest();

        let plan = adapter.plan_from_live(&manifest).unwrap();
        adapter
            .apply_with_history(&plan, &PushMetadata {
                manifest_version: 1,
                app_version: "0.1.0",
                baseline: "first",
            })
            .unwrap();

        // Small delay not needed — timestamps have nanosecond precision.
        let plan2 = adapter.plan_from_live(&manifest).unwrap();
        adapter
            .apply_with_history(&plan2, &PushMetadata {
                manifest_version: 1,
                app_version: "0.2.0",
                baseline: "second",
            })
            .unwrap();

        let entries = adapter.read_history(None).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].baseline, "second");
        assert_eq!(entries[1].baseline, "first");
    }

    #[test]
    fn read_history_with_limit() {
        let adapter = SqliteAdapter::in_memory().unwrap();
        let manifest = test_manifest();

        // Push twice.
        let plan1 = adapter.plan_from_live(&manifest).unwrap();
        adapter.apply_with_history(&plan1, &push_meta("live_sqlite")).unwrap();
        let plan2 = adapter.plan_from_live(&manifest).unwrap();
        adapter.apply_with_history(&plan2, &push_meta("live_sqlite")).unwrap();

        let all = adapter.read_history(None).unwrap();
        assert_eq!(all.len(), 2);

        let limited = adapter.read_history(Some(1)).unwrap();
        assert_eq!(limited.len(), 1);
    }

    #[test]
    fn read_history_entry_by_id() {
        let adapter = SqliteAdapter::in_memory().unwrap();
        let manifest = test_manifest();

        let plan = adapter.plan_from_live(&manifest).unwrap();
        adapter.apply_with_history(&plan, &push_meta("live_sqlite")).unwrap();

        let entries = adapter.read_history(None).unwrap();
        let id = &entries[0].id;

        let entry = adapter.read_history_entry(id).unwrap().unwrap();
        assert_eq!(&entry.id, id);
        assert_eq!(entry.operation_count, 2);
    }

    #[test]
    fn read_history_entry_missing_id() {
        let adapter = SqliteAdapter::in_memory().unwrap();
        let result = adapter.read_history_entry("nonexistent").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn history_entry_has_parsed_plan() {
        let adapter = SqliteAdapter::in_memory().unwrap();
        let manifest = test_manifest();

        let plan = adapter.plan_from_live(&manifest).unwrap();
        adapter.apply_with_history(&plan, &push_meta("live_sqlite")).unwrap();

        let entries = adapter.read_history(None).unwrap();
        let entry = &entries[0];

        // plan should be parsed from plan_json.
        assert!(entry.plan.is_some());
        let parsed_plan = entry.plan.as_ref().unwrap();
        assert!(!parsed_plan.operations.is_empty());

        // plan_json should still be present as raw string.
        assert!(!entry.plan_json.is_empty());
    }
}
