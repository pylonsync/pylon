pub mod account_backend;
pub mod api_key_backend;
pub mod audit_backend;
pub mod cache_handlers;
pub mod cache_server;
pub mod config;
pub mod cron;
pub mod datastore;
pub mod ip_limit;
pub mod job_store;
pub mod jobs;
pub mod log;
pub mod loro_store;
pub mod pg_loro_store;
pub mod magic_code_backend;
pub mod metrics;
pub mod oauth_backend;
pub mod org_backend;
pub mod verification_backend;
pub mod openapi;
pub mod presence;
pub mod pubsub;
pub mod rate_limit;
pub mod resp;
pub mod resp_server;
pub mod rooms;
pub mod scheduler;
pub mod server;
pub mod session_backend;
pub mod shard_ws;
pub mod sse;
pub mod tls;
pub mod workflow_store;
pub mod workflows;
pub mod ws;

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use pylon_kernel::{AppManifest, ManifestEntity};
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

/// Lift a `DataError` (the cross-crate error type for PG `DataStore`
/// operations) into a `RuntimeError`. Used by `PostgresDataStore`
/// closure bounds (`with_client`, `with_transaction`) so callers in
/// the runtime can propagate PG errors with their native error type.
impl From<pylon_http::DataError> for RuntimeError {
    fn from(e: pylon_http::DataError) -> Self {
        RuntimeError {
            code: e.code,
            message: e.message,
        }
    }
}

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
// Connection tuning
// ---------------------------------------------------------------------------

/// Apply the production pragma set on a SQLite connection. Identical
/// values to `pylon_storage::sqlite::tune_connection` — kept here too
/// because the Runtime opens its own connections directly (write +
/// read pool) without going through the storage adapter.
///
/// See `crates/storage/src/sqlite.rs` for the rationale on each
/// pragma. Skipping it on writes drops throughput by 5–10×.
fn tune_runtime_connection(conn: &Connection, in_memory: bool) {
    let pragmas: &[(&str, &str)] = if in_memory {
        &[
            ("temp_store", "MEMORY"),
            ("cache_size", "-65536"),
            ("foreign_keys", "ON"),
        ]
    } else {
        &[
            ("journal_mode", "WAL"),
            ("synchronous", "NORMAL"),
            ("cache_size", "-65536"),
            ("mmap_size", "268435456"),
            ("temp_store", "MEMORY"),
            ("busy_timeout", "5000"),
            ("foreign_keys", "ON"),
            ("wal_autocheckpoint", "1000"),
        ]
    };
    for (key, value) in pragmas {
        let _ = conn.pragma_update(None, key, value);
    }
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

/// A manifest-driven runtime that executes CRUD operations against an
/// underlying data store. Two backends are supported:
///
/// - **SQLite** (default): single-process, file-or-memory, with a write
///   mutex + read pool, FTS5 search, and per-row LoroDoc CRDT snapshots.
/// - **Postgres**: live cluster, suitable for multi-replica deployments.
///   Routes entity CRUD through [`pylon_storage::pg_datastore::PostgresDataStore`].
///   CRDT mode and FTS5-shaped search are SQLite-only at this layer; the
///   Postgres backend returns `NOT_SUPPORTED` for those paths and the router
///   degrades to JSON change events (no binary CRDT broadcasts).
///
/// Pick a backend by passing a `postgres://` URL to [`Runtime::open`]; any
/// other string is treated as a SQLite filesystem path.
pub struct Runtime {
    backend: RuntimeBackend,
    manifest: AppManifest,
    entities: HashMap<String, ManifestEntity>,
    /// True only for the SQLite in-memory variant. Postgres mode reports false.
    /// Gates the test-reset endpoint — a false positive here would let
    /// `/api/__test__/reset` truncate real tables.
    is_in_memory: bool,
}

/// Backend storage for entity CRUD. SQLite variant owns the connection
/// pool and CRDT cache; Postgres variant wraps a `PostgresDataStore`.
enum RuntimeBackend {
    Sqlite(SqliteBackend),
    Postgres(PgBackend),
}

/// SQLite-backed entity store. WAL mode allows one writer and multiple
/// concurrent readers — the struct exploits this with a single write
/// connection behind a mutex plus a pool of read-only connections.
struct SqliteBackend {
    /// Write connection — single mutex, serializes writes.
    write_conn: Mutex<Connection>,
    /// Read connections — pool of connections for concurrent reads.
    /// Empty for in-memory databases where extra connections are not possible.
    read_pool: Vec<Mutex<Connection>>,
    /// Counter for round-robin read pool selection.
    read_counter: AtomicUsize,
    /// Per-row LoroDoc cache + sidecar persistence. Used for entities with
    /// `crdt: true` (the default). Reads still hit SQLite directly via the
    /// read pool — the LoroDoc just produces the projected JSON that gets
    /// materialized into SQLite columns on every write.
    crdt: crate::loro_store::LoroStore,
}

/// Postgres-backed entity store. Wraps `PostgresDataStore` from
/// pylon-storage and delegates the `DataStore` surface directly.
pub(crate) struct PgBackend {
    pub(crate) store: pylon_storage::pg_datastore::PostgresDataStore,
    /// Per-row LoroDoc snapshot store for entities with `crdt: true`.
    /// Arc'd so the runtime layer can hand a clone to PgCrdtHookImpl
    /// (the bridge that lets PgTxStore call back into CRDT machinery
    /// from inside a held tx).
    pub(crate) crdt: std::sync::Arc<crate::pg_loro_store::PgLoroStore>,
}

/// Number of read-only connections to open in the pool.
const READ_POOL_SIZE: usize = 4;

/// True iff `url` is a Postgres connection string. Treats `postgres://`,
/// `postgresql://`, and the ambient-credentials forms as PG; everything
/// else is interpreted as a SQLite filesystem path.
fn is_postgres_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.starts_with("postgres://") || lower.starts_with("postgresql://")
}

/// Convert a `pylon_http::DataError` (returned by `PostgresDataStore`)
/// into the runtime's error type. The codes round-trip; only the type
/// changes.
fn data_err_to_runtime(e: pylon_http::DataError) -> RuntimeError {
    RuntimeError {
        code: e.code,
        message: e.message,
    }
}

impl Runtime {
    /// Open a runtime against either a SQLite file path or a Postgres URL.
    ///
    /// Backend selection is by URL prefix:
    /// - `postgres://...` or `postgresql://...` → Postgres (requires the
    ///   `postgres-live` feature on `pylon-storage`, enabled by default).
    /// - Anything else → SQLite, treating the string as a filesystem path
    ///   (`":memory:"` works via `Runtime::in_memory` instead).
    pub fn open(url: &str, manifest: AppManifest) -> Result<Self, RuntimeError> {
        if is_postgres_url(url) {
            Self::open_postgres(url, manifest)
        } else {
            let conn = Connection::open(url).map_err(|e| RuntimeError {
                code: "RUNTIME_OPEN_FAILED".into(),
                message: format!("Failed to open database: {e}"),
            })?;
            Self::from_connection(conn, manifest, false)
        }
    }

    /// Open a runtime backed by a live Postgres cluster.
    ///
    /// Schema must be applied separately via `pylon migrate` / the
    /// storage adapter's plan apply path — Runtime does not auto-create
    /// tables on Postgres (in contrast to SQLite, where `from_connection`
    /// runs CREATE TABLE IF NOT EXISTS on every open). This matches how
    /// production Postgres deployments are typically managed: schema is
    /// migrated via a controlled, observable step, not as a side effect
    /// of the server starting up.
    pub fn open_postgres(url: &str, manifest: AppManifest) -> Result<Self, RuntimeError> {
        let store = pylon_storage::pg_datastore::PostgresDataStore::connect(url, manifest.clone())
            .map_err(data_err_to_runtime)?;
        // Bootstrap the CRDT sidecar table on every open. Idempotent
        // (`CREATE TABLE IF NOT EXISTS`); same shape as the SQLite
        // path's `ensure_sidecar` call. Without this, the first
        // CRDT-mode write would error because `_pylon_crdt_snapshots`
        // doesn't exist yet on a fresh PG database.
        store.with_client(|c| crate::pg_loro_store::ensure_sidecar(c)).map_err(|e| {
            RuntimeError {
                code: "CRDT_SIDECAR_BOOTSTRAP_FAILED".into(),
                message: format!("ensure pg crdt sidecar: {e}"),
            }
        })?;
        let entities: HashMap<String, ManifestEntity> = manifest
            .entities
            .iter()
            .map(|e| (e.name.clone(), e.clone()))
            .collect();
        Ok(Self {
            backend: RuntimeBackend::Postgres(PgBackend {
                store,
                crdt: std::sync::Arc::new(crate::pg_loro_store::PgLoroStore::new()),
            }),
            manifest,
            entities,
            is_in_memory: false,
        })
    }

    /// Returns true if this runtime is backed by an in-memory SQLite DB.
    ///
    /// Stored at open time rather than queried via `conn.path()` because
    /// the path-based check conflates "no filename" with "in-memory":
    /// `Connection::open("")` yields a file-backed DB with empty path,
    /// and would falsely pass as in-memory. Since we always know at
    /// construction time which constructor was used, track the bit.
    ///
    /// Gates the test-reset endpoint — a false positive here would let
    /// `/api/__test__/reset` truncate real tables.
    pub fn is_in_memory(&self) -> bool {
        self.is_in_memory
    }

    /// Filesystem path to the SQLite database, if this runtime is file-backed.
    /// Returns `None` for in-memory runtimes AND Postgres runtimes (no local
    /// file). Used by the server bootstrap to derive companion paths
    /// (session store, change log persistence) without requiring the caller
    /// to pass them in.
    pub fn db_path(&self) -> Option<String> {
        if self.is_in_memory {
            return None;
        }
        let sb = match &self.backend {
            RuntimeBackend::Sqlite(sb) => sb,
            RuntimeBackend::Postgres(_) => return None,
        };
        let conn = sb.write_conn.lock().ok()?;
        conn.path().filter(|p| !p.is_empty()).map(String::from)
    }

    /// Drop every row from every entity table. Intended for test harnesses
    /// that call `/api/__test__/reset` between cases; refuses to run on
    /// anything but an in-memory database.
    ///
    /// Does NOT drop the tables themselves — schema stays, indexes stay,
    /// triggers stay. Just truncates user data + the change log.
    pub fn reset_for_tests(&self) -> Result<(), RuntimeError> {
        if !self.is_in_memory() {
            return Err(RuntimeError {
                code: "RESET_REFUSED".into(),
                message: "reset_for_tests is only available on in-memory databases".into(),
            });
        }
        let conn = self.lock_write_conn()?;
        let entity_names: Vec<String> = self.entities.values().map(|e| e.name.clone()).collect();
        for name in entity_names {
            let sql = format!("DELETE FROM {}", quote_ident(&name));
            let _ = conn.execute(&sql, []);
            // Also clear any FTS5 shadow table if present.
            let fts_sql = format!("DELETE FROM {}", quote_ident(&format!("{name}_fts")));
            let _ = conn.execute(&fts_sql, []);
        }
        Ok(())
    }

    /// Create an in-memory SQLite-backed runtime (useful for tests and
    /// benchmarks). For Postgres-backed equivalents, use `open_postgres`
    /// with a test-cluster URL.
    pub fn in_memory(manifest: AppManifest) -> Result<Self, RuntimeError> {
        let conn = Connection::open_in_memory().map_err(|e| RuntimeError {
            code: "RUNTIME_OPEN_FAILED".into(),
            message: format!("Failed to open in-memory database: {e}"),
        })?;
        Self::from_connection(conn, manifest, true)
    }

    fn from_connection(
        conn: Connection,
        manifest: AppManifest,
        is_in_memory: bool,
    ) -> Result<Self, RuntimeError> {
        // Apply the production pragma set on the write connection.
        tune_runtime_connection(&conn, is_in_memory);

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

            // Create an FTS5 virtual table over all text-ish fields so clients
            // can do full-text search via the `$search` query operator.
            //
            // Fields that look like "string" / "richtext" / "text" are indexed.
            // The FTS table is a contentless external-content table pointed at
            // the entity table, so SQLite keeps it consistent via triggers we
            // install below.
            let text_fields: Vec<&str> = entity
                .fields
                .iter()
                .filter(|f| matches!(f.field_type.as_str(), "string" | "richtext" | "text"))
                .map(|f| f.name.as_str())
                .collect();
            if !text_fields.is_empty() {
                let fts_name = format!("{}_fts", entity.name);
                let quoted_cols: Vec<String> = text_fields.iter().map(|f| quote_ident(f)).collect();
                let fts_sql = format!(
                    "CREATE VIRTUAL TABLE IF NOT EXISTS {} USING fts5({}, content={}, content_rowid='rowid')",
                    quote_ident(&fts_name),
                    quoted_cols.join(", "),
                    quote_ident(&entity.name),
                );
                // FTS5 may not be compiled in; ignore errors so those builds
                // still work (queries using $search will return empty).
                let fts_ok = conn.execute(&fts_sql, []).is_ok();

                if fts_ok {
                    // Sync triggers: keep FTS index current on INSERT/UPDATE/DELETE.
                    //
                    // Subtle bug fixed: the trigger NAME must be built from
                    // the raw `fts_name` + suffix and THEN quoted once.
                    // Previously this code quoted `fts_name` first and then
                    // appended `_ai`/`_ad`/`_au` AFTER the closing quote,
                    // producing invalid SQL like `"foo_fts"_ai`. The
                    // `.ok()` after execute silently ate the error, so the
                    // triggers were never created and FTS stayed out of
                    // sync on writes.
                    let tbl = quote_ident(&entity.name);
                    let ftb = quote_ident(&fts_name);
                    let cols_list = quoted_cols.join(", ");
                    let new_list: Vec<String> = text_fields
                        .iter()
                        .map(|f| format!("new.{}", quote_ident(f)))
                        .collect();
                    let old_list: Vec<String> = text_fields
                        .iter()
                        .map(|f| format!("old.{}", quote_ident(f)))
                        .collect();

                    let trigger_ai = quote_ident(&format!("{}_ai", fts_name));
                    let trigger_ad = quote_ident(&format!("{}_ad", fts_name));
                    let trigger_au = quote_ident(&format!("{}_au", fts_name));

                    let trigger_ins = format!(
                        "CREATE TRIGGER IF NOT EXISTS {trigger_ai} AFTER INSERT ON {tbl} BEGIN \
                         INSERT INTO {ftb}(rowid, {cols_list}) VALUES (new.rowid, {new_vals}); END",
                        new_vals = new_list.join(", "),
                    );
                    let trigger_del = format!(
                        "CREATE TRIGGER IF NOT EXISTS {trigger_ad} AFTER DELETE ON {tbl} BEGIN \
                         INSERT INTO {ftb}({ftb}, rowid, {cols_list}) VALUES('delete', old.rowid, {old_vals}); END",
                        old_vals = old_list.join(", "),
                    );
                    let trigger_upd = format!(
                        "CREATE TRIGGER IF NOT EXISTS {trigger_au} AFTER UPDATE ON {tbl} BEGIN \
                         INSERT INTO {ftb}({ftb}, rowid, {cols_list}) VALUES('delete', old.rowid, {old_vals}); \
                         INSERT INTO {ftb}(rowid, {cols_list}) VALUES (new.rowid, {new_vals}); END",
                        new_vals = new_list.join(", "),
                        old_vals = old_list.join(", "),
                    );
                    // Log failures instead of silently dropping — FTS going
                    // stale should be visible to operators.
                    for (label, sql) in [
                        ("ai", &trigger_ins),
                        ("ad", &trigger_del),
                        ("au", &trigger_upd),
                    ] {
                        if let Err(e) = conn.execute(sql, []) {
                            tracing::warn!(
                                "[fts] failed to create {label} trigger for {}: {e}",
                                entity.name
                            );
                        }
                    }
                }
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
                tune_runtime_connection(&read_conn, false);
                pool.push(Mutex::new(read_conn));
            }
            pool
        } else {
            // In-memory DB — no separate read connections possible.
            Vec::new()
        };

        // Sidecar table for CRDT snapshots — created always so toggling
        // `crdt: true` on an entity post-deploy doesn't need a migration.
        crate::loro_store::ensure_sidecar(&conn).map_err(|e| RuntimeError {
            code: "CRDT_SIDECAR_FAILED".into(),
            message: format!("create CRDT sidecar table: {e}"),
        })?;

        Ok(Self {
            backend: RuntimeBackend::Sqlite(SqliteBackend {
                write_conn: Mutex::new(conn),
                read_pool,
                read_counter: AtomicUsize::new(0),
                crdt: crate::loro_store::LoroStore::new(),
            }),
            manifest,
            entities,
            is_in_memory,
        })
    }

    /// Create the search index tables (`_facet_bitmap`, per-entity
    /// `_fts_<Entity>`, and a covering index for each declared
    /// sortable field) for every searchable entity in the manifest.
    ///
    /// Production deployments do this via the storage adapter's
    /// `apply_schema` / migration plan; that path also handles
    /// adding/removing the tables when a `search:` block is added or
    /// removed across deploys. This method is a quick path for tests
    /// and benchmarks that build a `Runtime::in_memory(...)` directly
    /// without going through the schema-plan pipeline.
    pub fn ensure_search_indexes(&self) -> Result<(), RuntimeError> {
        // Postgres: schema (FTS, facets) is owned by the storage adapter's
        // migration plan. Tests / benchmarks against Postgres must apply
        // the plan separately; this fast-path is a SQLite-only convenience.
        if matches!(self.backend, RuntimeBackend::Postgres(_)) {
            return Ok(());
        }
        let conn = self.lock_write_conn()?;
        conn.execute(pylon_storage::search::create_facet_table_sql(), [])
            .map_err(|e| RuntimeError {
                code: "FACET_TABLE_FAILED".into(),
                message: format!("create _facet_bitmap: {e}"),
            })?;
        for entity in &self.manifest.entities {
            if let Some(cfg) = &entity.search {
                if let Some(sql) = pylon_storage::search::create_fts_table_sql(&entity.name, cfg) {
                    conn.execute(&sql, []).map_err(|e| RuntimeError {
                        code: "FTS_TABLE_FAILED".into(),
                        message: format!("create FTS table for {}: {e}", entity.name),
                    })?;
                }
                for field in &cfg.sortable {
                    let idx_sql = format!(
                        "CREATE INDEX IF NOT EXISTS \"{}_sort_{field}\" ON \"{}\" (\"{field}\")",
                        entity.name, entity.name,
                    );
                    conn.execute(&idx_sql, []).map_err(|e| RuntimeError {
                        code: "SORT_INDEX_FAILED".into(),
                        message: format!("create sort index for {}.{field}: {e}", entity.name),
                    })?;
                }
            }
        }
        Ok(())
    }

    /// Return a reference to the app manifest.
    pub fn manifest(&self) -> &AppManifest {
        &self.manifest
    }

    /// Expose the write connection mutex for transactional operations.
    /// SQLite-only — Postgres mode returns `NOT_SQLITE_BACKEND`. Callers
    /// that need a transaction on Postgres should use [`Runtime::transact_ops`]
    /// (via the `DataStore` trait), which routes to a real Postgres
    /// transaction inside `PostgresDataStore`.
    pub fn lock_conn_pub(&self) -> Result<std::sync::MutexGuard<'_, Connection>, RuntimeError> {
        self.lock_write_conn()
    }

    /// Return the number of read connections in the pool. Always 0 for
    /// in-memory SQLite (pool is empty by design) and for Postgres mode
    /// (the pool concept doesn't apply — `PostgresDataStore` manages its
    /// own connection internally).
    pub fn read_pool_size(&self) -> usize {
        match &self.backend {
            RuntimeBackend::Sqlite(sb) => sb.read_pool.len(),
            RuntimeBackend::Postgres(_) => 0,
        }
    }

    /// Return true if this runtime is backed by Postgres. Useful for
    /// SQLite-only fast-paths to early-exit cleanly.
    pub fn is_postgres(&self) -> bool {
        matches!(self.backend, RuntimeBackend::Postgres(_))
    }

    // -----------------------------------------------------------------------
    // CRDT helpers
    // -----------------------------------------------------------------------

    /// Map an entity's manifest fields → the [`pylon_crdt::CrdtField`] vec
    /// the LoroStore needs. Resolves each field's CRDT shape from the
    /// (type, annotation) pair via `pylon_crdt::field_kind`. Caches
    /// nothing yet — called per write, fine at our entity counts.
    pub(crate) fn crdt_fields_for(
        &self,
        ent: &ManifestEntity,
    ) -> Result<Vec<pylon_crdt::CrdtField>, RuntimeError> {
        let mut out = Vec::with_capacity(ent.fields.len());
        for f in &ent.fields {
            // Skip the implicit `id` column — it's the row key, not a
            // CRDT-managed value. SQLite's PRIMARY KEY constraint owns it.
            if f.name == "id" {
                continue;
            }
            let kind = pylon_crdt::field_kind(&f.field_type, f.crdt).map_err(|e| RuntimeError {
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

    /// Borrow the CRDT store. SQLite-only — Postgres mode does not yet
    /// support per-row CRDT snapshots at the runtime layer (CRDT
    /// broadcasts degrade to JSON change events).
    ///
    /// # Panics
    /// Panics on Postgres backend. Call sites that may run under either
    /// backend should branch on `is_postgres()` first.
    pub fn crdt_store(&self) -> &crate::loro_store::LoroStore {
        match &self.backend {
            RuntimeBackend::Sqlite(sb) => &sb.crdt,
            RuntimeBackend::Postgres(_) => {
                panic!("crdt_store() called on Postgres-backed Runtime")
            }
        }
    }

    // -----------------------------------------------------------------------
    // CRUD operations
    // -----------------------------------------------------------------------

    /// Insert a new row. Returns the generated ID.
    ///
    /// For entities with `crdt: true` (the default) the LoroDoc snapshot
    /// + the SQLite materialized row are committed together in a single
    /// SQLite transaction so a crash between the two leaves neither.
    /// `crdt: false` entities skip the LoroDoc and use a direct write
    /// (legacy LWW path). Both produce the same on-disk row shape, so
    /// reads, indexes, FTS, and policies don't change between modes.
    pub fn insert(&self, entity: &str, data: &serde_json::Value) -> Result<String, RuntimeError> {
        if let Some(pg) = self.pg_backend() {
            let ent = self.require_entity(entity)?;
            // Both CRDT-mode and non-CRDT writes go through one
            // transaction so the row, the FTS shadow, and (for CRDT)
            // the LoroDoc snapshot either all commit or all roll back.
            // Pre-fix this was three separate autocommits and any
            // failure between them desynced the layers.
            if ent.crdt {
                let crdt_fields = self.crdt_fields_for(ent)?;
                let id = generate_id();
                // Inject the generated id so build_insert_sql reuses
                // it — keeps the snapshot key and the row id aligned.
                let mut row = data.clone();
                if let Some(obj) = row.as_object_mut() {
                    obj.insert("id".into(), serde_json::Value::String(id.clone()));
                }
                let result = pg.store.with_transaction_raw(|tx| -> Result<(), RuntimeError> {
                    pg.crdt
                        .apply_patch(tx, entity, &id, &crdt_fields, data)
                        .map_err(|e| RuntimeError {
                            code: "CRDT_APPLY_FAILED".into(),
                            message: format!("crdt write {entity}/{id}: {e}"),
                        })?;
                    pylon_storage::pg_tx_store::tx_insert(tx, &self.manifest, entity, &row)
                        .map(|_| ())
                        .map_err(data_err_to_runtime)?;
                    pg.crdt.cache_after_commit(tx, entity, &id);
                    Ok(())
                });
                if result.is_err() {
                    // Rollback drops the persisted snapshot, but the
                    // in-memory LoroDoc was mutated in-place by
                    // apply_patch. Evict it so the next access
                    // re-hydrates from disk (which is back in the
                    // pre-apply state). Without this, the cache would
                    // hold a doc ahead of the materialized row.
                    pg.crdt.evict(entity, &id);
                }
                result?;
                return Ok(id);
            }
            // Non-CRDT path: still one tx — the typed `DataStore::insert`
            // already wraps in `with_transaction` internally for FTS
            // atomicity, so we can delegate straight through.
            return pylon_http::DataStore::insert(&pg.store, entity, data)
                .map_err(data_err_to_runtime);
        }
        let ent = self.require_entity(entity)?;
        let conn = self.lock_write_conn()?;

        let id = generate_id();

        let obj = data.as_object().ok_or_else(|| RuntimeError {
            code: "INVALID_DATA".into(),
            message: "Insert data must be a JSON object".into(),
        })?;

        // Validate columns up-front so we don't even open a transaction
        // for a patch that the SQL INSERT will reject.
        for key in obj.keys() {
            if key != "id" {
                validate_column_name(key, ent)?;
            }
        }

        // SQLite-only path past this point — Postgres dispatch happened
        // at the top of `insert()`. Hoist the backend handle here so we
        // can reach the LoroStore from inside the tx closure without
        // a second runtime branch on every iteration.
        let sb = self.sqlite_backend()?;

        // Atomic block — CRDT sidecar snapshot + materialized SQL row +
        // search-index maintenance all land together or none does. SQLite's
        // rollback journal makes this crash-safe end-to-end.
        with_write_tx(&conn, || {
            if ent.crdt {
                let crdt_fields = self.crdt_fields_for(ent)?;
                sb.crdt
                    .apply_patch(&conn, entity, &id, &crdt_fields, data)
                    .map_err(|e| RuntimeError {
                        code: "CRDT_APPLY_FAILED".into(),
                        message: format!("crdt write {entity}/{id}: {e}"),
                    })?;
            }

            let mut col_names = vec![quote_ident("id")];
            let mut placeholders = vec!["?1".to_string()];
            let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(id.clone())];

            let mut idx = 2;
            for (key, val) in obj {
                if key == "id" {
                    continue;
                }
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

            let params: Vec<&dyn rusqlite::types::ToSql> =
                values.iter().map(|v| v.as_ref()).collect();
            conn.execute(&sql, params.as_slice())
                .map_err(|e| RuntimeError {
                    code: "INSERT_FAILED".into(),
                    message: format!("Insert into {entity} failed: {e}"),
                })?;

            // Search-index maintenance lives inside the same tx so a
            // crash between the row insert and the FTS update can't leave
            // the search index inconsistent with the row table.
            if let Some(cfg) = ent.search.as_ref() {
                if !cfg.is_empty() {
                    pylon_storage::search_maintenance::apply_insert(&conn, entity, &id, data, cfg)
                        .map_err(|e| RuntimeError {
                            code: "SEARCH_MAINTENANCE_FAILED".into(),
                            message: format!("search index update on insert {entity}: {e}"),
                        })?;
                }
            }
            Ok(())
        })?;

        Ok(id)
    }

    /// Get a single row by ID.
    pub fn get_by_id(
        &self,
        entity: &str,
        id: &str,
    ) -> Result<Option<serde_json::Value>, RuntimeError> {
        if let Some(pg) = self.pg_backend() {
            return pylon_http::DataStore::get_by_id(&pg.store, entity, id)
                .map_err(data_err_to_runtime);
        }
        let ent = self.require_entity(entity)?;
        let conn = self.lock_read_conn()?;

        let sql = format!("SELECT * FROM {} WHERE \"id\" = ?1", quote_ident(entity));
        let mut stmt = conn.prepare_cached(&sql).map_err(|e| RuntimeError {
            code: "QUERY_FAILED".into(),
            message: format!("Failed to prepare query: {e}"),
        })?;

        let columns: Vec<String> = ent.fields.iter().map(|f| f.name.clone()).collect();

        let result = stmt
            .query_row(rusqlite::params![id], |row| Ok(row_to_json(row, &columns)))
            .ok();

        Ok(result)
    }

    /// List all rows for an entity.
    pub fn list(&self, entity: &str) -> Result<Vec<serde_json::Value>, RuntimeError> {
        if let Some(pg) = self.pg_backend() {
            return pylon_http::DataStore::list(&pg.store, entity).map_err(data_err_to_runtime);
        }
        let ent = self.require_entity(entity)?;
        let conn = self.lock_read_conn()?;

        let sql = format!("SELECT * FROM {} ORDER BY \"id\"", quote_ident(entity));
        let mut stmt = conn.prepare_cached(&sql).map_err(|e| RuntimeError {
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
        if let Some(pg) = self.pg_backend() {
            return pylon_http::DataStore::list_after(&pg.store, entity, after, limit)
                .map_err(data_err_to_runtime);
        }
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
                vec![Box::new(cursor.to_string()), Box::new(limit as i64)],
            ),
            None => (
                format!("SELECT * FROM {} ORDER BY \"id\" LIMIT ?1", table),
                vec![Box::new(limit as i64)],
            ),
        };

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|v| v.as_ref()).collect();

        let mut stmt = conn.prepare_cached(&sql).map_err(|e| RuntimeError {
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
    ///
    /// For entities with `crdt: true` (the default) the LoroDoc receives
    /// the patch first; the SQLite UPDATE writes the same fields so the
    /// materialized view stays in lockstep with the doc state.
    pub fn update(
        &self,
        entity: &str,
        id: &str,
        data: &serde_json::Value,
    ) -> Result<bool, RuntimeError> {
        if let Some(pg) = self.pg_backend() {
            let ent = self.require_entity(entity)?;
            if ent.crdt {
                // CRDT mode: snapshot apply + materialized update +
                // FTS shadow rebuild all share one tx. Pre-fix the
                // snapshot landed in autocommit and the row write in
                // a separate one — a mid-write crash desynced them.
                //
                // The closure also FAILS the tx if `tx_update` returns
                // false (no row matched). Without that, the snapshot
                // would commit alone — orphaned state pointing at a
                // row that doesn't exist. Codex flagged this. On
                // rollback the runtime evicts the cached LoroDoc so
                // the next read re-hydrates from the (unchanged)
                // sidecar.
                let crdt_fields = self.crdt_fields_for(ent)?;
                let result = pg.store.with_transaction_raw(|tx| -> Result<bool, RuntimeError> {
                    pg.crdt
                        .apply_patch(tx, entity, id, &crdt_fields, data)
                        .map_err(|e| RuntimeError {
                            code: "CRDT_APPLY_FAILED".into(),
                            message: format!("crdt update {entity}/{id}: {e}"),
                        })?;
                    let updated = pylon_storage::pg_tx_store::tx_update(
                        tx,
                        &self.manifest,
                        entity,
                        id,
                        data,
                    )
                    .map_err(data_err_to_runtime)?;
                    if !updated {
                        // Roll back via Err so the snapshot doesn't
                        // commit against a missing row.
                        return Err(RuntimeError {
                            code: "ENTITY_NOT_FOUND".into(),
                            message: format!(
                                "Update on {entity}/{id} found no row — refusing to commit \
                                 a CRDT snapshot that would orphan."
                            ),
                        });
                    }
                    // Refresh the cache from the just-persisted
                    // snapshot so post-commit reads on this process
                    // skip the re-hydration round-trip.
                    pg.crdt.cache_after_commit(tx, entity, id);
                    Ok(updated)
                });
                if result.is_err() {
                    pg.crdt.evict(entity, id);
                    // ENTITY_NOT_FOUND from the inner closure is the
                    // intended return for "no such row" — translate
                    // into Ok(false) so callers see the same shape
                    // the SQLite path returns. Real errors (CRDT
                    // apply failed, BEGIN/COMMIT failed) propagate.
                    if let Err(ref e) = result {
                        if e.code == "ENTITY_NOT_FOUND" {
                            return Ok(false);
                        }
                    }
                }
                return result;
            }
            return pylon_http::DataStore::update(&pg.store, entity, id, data)
                .map_err(data_err_to_runtime);
        }
        let ent = self.require_entity(entity)?;
        let conn = self.lock_write_conn()?;

        let obj = data.as_object().ok_or_else(|| RuntimeError {
            code: "INVALID_DATA".into(),
            message: "Update data must be a JSON object".into(),
        })?;

        // Validate up-front and exit cheap if there's nothing to write.
        for key in obj.keys() {
            if key != "id" {
                validate_column_name(key, ent)?;
            }
        }
        let writable_keys: Vec<&String> = obj.keys().filter(|k| *k != "id").collect();
        if writable_keys.is_empty() {
            return Ok(false);
        }

        // SQLite-only path past this point — see note in `insert()`.
        let sb = self.sqlite_backend()?;

        // Atomic block — same shape as insert. CRDT snapshot, SQL UPDATE,
        // and FTS maintenance all commit together.
        let affected = with_write_tx(&conn, || -> Result<i64, RuntimeError> {
            if ent.crdt {
                let crdt_fields = self.crdt_fields_for(ent)?;
                sb.crdt
                    .apply_patch(&conn, entity, id, &crdt_fields, data)
                    .map_err(|e| RuntimeError {
                        code: "CRDT_APPLY_FAILED".into(),
                        message: format!("crdt write {entity}/{id}: {e}"),
                    })?;
            }

            let mut set_clauses = Vec::new();
            let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
            let mut idx = 1;
            for key in &writable_keys {
                set_clauses.push(format!("{} = ?{idx}", quote_ident(key)));
                values.push(json_to_sql(&obj[key.as_str()]));
                idx += 1;
            }

            // Capture pre-UPDATE row for search-maintenance diff INSIDE the
            // tx. Matches the contract of search_maintenance::apply_update
            // — old state must be read before the UPDATE lands.
            let searchable = ent.search.as_ref().map(|c| !c.is_empty()).unwrap_or(false);
            let old_row = if searchable {
                self.get_by_id_with_conn(&conn, entity, id)?
            } else {
                None
            };

            values.push(Box::new(id.to_string()));
            let sql = format!(
                "UPDATE {} SET {} WHERE \"id\" = ?{idx}",
                quote_ident(entity),
                set_clauses.join(", ")
            );

            let params: Vec<&dyn rusqlite::types::ToSql> =
                values.iter().map(|v| v.as_ref()).collect();
            let affected = conn
                .execute(&sql, params.as_slice())
                .map_err(|e| RuntimeError {
                    code: "UPDATE_FAILED".into(),
                    message: format!("Update {entity}/{id} failed: {e}"),
                })? as i64;

            if affected > 0 && searchable {
                if let (Some(cfg), Some(old)) = (ent.search.as_ref(), old_row) {
                    pylon_storage::search_maintenance::apply_update(
                        &conn, entity, id, &old, data, cfg,
                    )
                    .map_err(|e| RuntimeError {
                        code: "SEARCH_MAINTENANCE_FAILED".into(),
                        message: format!("search index update on update {entity}: {e}"),
                    })?;
                }
            }
            Ok(affected)
        })?;

        Ok(affected > 0)
    }

    /// Delete a row by ID. Returns true if a row was actually deleted.
    pub fn delete(&self, entity: &str, id: &str) -> Result<bool, RuntimeError> {
        if let Some(pg) = self.pg_backend() {
            let ent = self.require_entity(entity)?;
            if ent.crdt {
                // Sidecar delete + entity delete + FTS shadow delete
                // share one tx. Eviction of the in-memory cache runs
                // AFTER commit so a rolled-back delete leaves the
                // cache valid (the snapshot is still on disk).
                let result = pg.store.with_transaction_raw(|tx| -> Result<bool, RuntimeError> {
                    tx.execute(
                        "DELETE FROM _pylon_crdt_snapshots WHERE entity = $1 AND row_id = $2",
                        &[&entity, &id],
                    )
                    .map_err(|e| RuntimeError {
                        code: "CRDT_SIDECAR_DELETE_FAILED".into(),
                        message: format!("delete pg crdt snapshot {entity}/{id}: {e}"),
                    })?;
                    pylon_storage::pg_tx_store::tx_delete(tx, &self.manifest, entity, id)
                        .map_err(data_err_to_runtime)
                });
                // Evict regardless of whether tx_delete found a row —
                // we issued the sidecar DELETE inside the same tx, so
                // any cached doc is now stale even if the entity row
                // was already gone (orphan sidecar case codex flagged).
                // Only skip eviction if the WHOLE tx rolled back.
                if result.is_ok() {
                    pg.crdt.evict(entity, id);
                }
                return result;
            }
            return pylon_http::DataStore::delete(&pg.store, entity, id)
                .map_err(data_err_to_runtime);
        }
        let ent = self.require_entity(entity)?;
        let conn = self.lock_write_conn()?;

        // Apply search-maintenance BEFORE the DELETE — we still need
        // the row's facet values to clear the bitmap bits.
        let searchable = ent.search.as_ref().map(|c| !c.is_empty()).unwrap_or(false);
        if searchable {
            if let (Some(cfg), Ok(Some(row))) = (
                ent.search.as_ref(),
                self.get_by_id_with_conn(&conn, entity, id),
            ) {
                pylon_storage::search_maintenance::apply_delete(&conn, entity, id, &row, cfg)
                    .map_err(|e| RuntimeError {
                        code: "SEARCH_MAINTENANCE_FAILED".into(),
                        message: format!("search index update on delete {entity}: {e}"),
                    })?;
            }
        }

        let sql = format!("DELETE FROM {} WHERE \"id\" = ?1", quote_ident(entity));
        let affected = conn
            .execute(&sql, rusqlite::params![id])
            .map_err(|e| RuntimeError {
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
        if let Some(pg) = self.pg_backend() {
            return pylon_http::DataStore::lookup(&pg.store, entity, field, value)
                .map_err(data_err_to_runtime);
        }
        let ent = self.require_entity(entity)?;
        validate_column_name(field, ent)?;
        let conn = self.lock_read_conn()?;

        let sql = format!(
            "SELECT * FROM {} WHERE {} = ?1 LIMIT 1",
            quote_ident(entity),
            quote_ident(field)
        );
        let columns: Vec<String> = ent.fields.iter().map(|f| f.name.clone()).collect();

        let result = conn.prepare_cached(&sql).ok().and_then(|mut stmt| {
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
    pub fn unlink(&self, entity: &str, id: &str, relation: &str) -> Result<bool, RuntimeError> {
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
        if let Some(pg) = self.pg_backend() {
            return pylon_http::DataStore::query_filtered(&pg.store, entity, filter)
                .map_err(data_err_to_runtime);
        }
        let ent = self.require_entity(entity)?;
        let conn = self.lock_read_conn()?;

        let columns: Vec<String> = ent.fields.iter().map(|f| f.name.clone()).collect();
        let obj = filter
            .as_object()
            .unwrap_or(&serde_json::Map::new())
            .clone();

        let mut where_clauses = Vec::new();
        let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        let mut order_clause = String::new();
        let mut limit_clause = String::new();
        let mut join_clause = String::new();
        let mut fts_order = false;
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
                "$offset" => {
                    if let Some(n) = val.as_u64() {
                        // SQLite requires LIMIT before OFFSET; add a default.
                        if limit_clause.is_empty() {
                            limit_clause = " LIMIT -1".into();
                        }
                        limit_clause = format!("{limit_clause} OFFSET {n}");
                    }
                }
                "$search" => {
                    if let Some(q) = val.as_str() {
                        // Join against the entity's FTS5 virtual table.
                        let fts = format!("{}_fts", entity);
                        join_clause = format!(
                            " JOIN {fts} ON {ent}.rowid = {fts}.rowid",
                            fts = quote_ident(&fts),
                            ent = quote_ident(entity),
                        );
                        where_clauses.push(format!("{} MATCH ?{idx}", quote_ident(&fts)));
                        values.push(Box::new(q.to_string()));
                        fts_order = true;
                        idx += 1;
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
                                    let pattern = format!("%{}%", op_val.as_str().unwrap_or(""));
                                    values.push(Box::new(pattern));
                                    idx += 1;
                                }
                                "$in" => {
                                    if let Some(arr) = op_val.as_array() {
                                        if arr.is_empty() {
                                            // Empty $in matches nothing.
                                            // Previously SQLite SKIPPED the
                                            // predicate (returning ALL rows)
                                            // while PG short-circuited to
                                            // FALSE — a real cross-backend
                                            // drift bug codex caught. Both
                                            // now emit `0` (false) so empty
                                            // $in returns an empty set.
                                            where_clauses.push("0".into());
                                        } else {
                                            let placeholders: Vec<String> = arr
                                                .iter()
                                                .map(|v| {
                                                    let p = format!("?{idx}");
                                                    values.push(json_to_sql(v));
                                                    idx += 1;
                                                    p
                                                })
                                                .collect();
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
            order_clause = if fts_order {
                // FTS joins default-order by bm25 relevance.
                " ORDER BY bm25(".to_string() + &quote_ident(&format!("{}_fts", entity)) + ")"
            } else {
                format!(" ORDER BY {}.\"id\"", quote_ident(entity))
            };
        }

        let select_prefix = format!("{}.*", quote_ident(entity));
        let sql = format!(
            "SELECT {} FROM {}{}{}{}{}",
            select_prefix,
            quote_ident(entity),
            join_clause,
            where_sql,
            order_clause,
            limit_clause
        );
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            values.iter().map(|v| v.as_ref()).collect();

        let mut stmt = conn.prepare_cached(&sql).map_err(|e| RuntimeError {
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
            let filter = query_opts
                .get("where")
                .cloned()
                .unwrap_or(serde_json::json!({}));
            let rows = self.query_filtered(entity_name, &filter)?;

            // Apply includes (relations) if present.
            let rows = if let Some(include) = query_opts.get("include").and_then(|v| v.as_object())
            {
                // Internal invariant: if query_filtered succeeded above, the
                // entity must exist. Previously this used .unwrap() which
                // would panic if the invariant broke — a panic inside the
                // handler path poisons the connection mutex and takes down
                // all subsequent reads. Fail the request cleanly instead.
                let ent = self.entities.get(entity_name).ok_or_else(|| RuntimeError {
                    code: "INVARIANT_BROKEN".into(),
                    message: format!(
                        "entity \"{entity_name}\" missing from registry during include expansion"
                    ),
                })?;
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
                                        // One-to-many: find rows in target where field matches id.
                                        let sub_filter = serde_json::json!({ &rel.field: &fk });
                                        if let Ok(related) =
                                            self.query_filtered(&rel.target, &sub_filter)
                                        {
                                            row[rel_name] = serde_json::json!(related);
                                        }
                                    } else {
                                        // One-to-one / many-to-one: get by id.
                                        if let Ok(Some(related)) = self.get_by_id(&rel.target, &fk)
                                        {
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
        conn.execute(&sql, params.as_slice())
            .map_err(|e| RuntimeError {
                code: "INSERT_FAILED".into(),
                message: format!("Insert into {entity} failed: {e}"),
            })?;

        // Faceted-search maintenance in the same transaction. Skipped
        // for entities that don't declare `search:` in their schema.
        if let Some(cfg) = ent.search.as_ref() {
            if !cfg.is_empty() {
                pylon_storage::search_maintenance::apply_insert(conn, entity, &id, data, cfg)
                    .map_err(|e| RuntimeError {
                        code: "SEARCH_MAINTENANCE_FAILED".into(),
                        message: format!("search index update on insert {entity}: {e}"),
                    })?;
            }
        }

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

        // Capture the pre-UPDATE row if we need to diff facet values.
        // Read happens before the UPDATE so apply_update sees the OLD
        // state of any facet field. Cheap — single-row lookup on the
        // `id` primary-key index.
        let searchable = ent.search.as_ref().map(|c| !c.is_empty()).unwrap_or(false);
        let old_row = if searchable {
            self.get_by_id_with_conn(conn, entity, id)?
        } else {
            None
        };

        values.push(Box::new(id.to_string()));
        let sql = format!(
            "UPDATE {} SET {} WHERE \"id\" = ?{idx}",
            quote_ident(entity),
            set_clauses.join(", ")
        );
        let params: Vec<&dyn rusqlite::types::ToSql> = values.iter().map(|v| v.as_ref()).collect();
        let affected = conn
            .execute(&sql, params.as_slice())
            .map_err(|e| RuntimeError {
                code: "UPDATE_FAILED".into(),
                message: format!("Update {entity}/{id} failed: {e}"),
            })?;

        if affected > 0 && searchable {
            if let (Some(cfg), Some(old)) = (ent.search.as_ref(), old_row) {
                pylon_storage::search_maintenance::apply_update(conn, entity, id, &old, data, cfg)
                    .map_err(|e| RuntimeError {
                        code: "SEARCH_MAINTENANCE_FAILED".into(),
                        message: format!("search index update on update {entity}: {e}"),
                    })?;
            }
        }

        Ok(affected > 0)
    }

    /// Delete using an already-locked connection (for transactions).
    pub fn delete_with_conn(
        &self,
        conn: &Connection,
        entity: &str,
        id: &str,
    ) -> Result<bool, RuntimeError> {
        let ent = self.require_entity(entity)?;

        // Apply search maintenance BEFORE the DELETE so we still have
        // the row's facet values to diff against.
        let searchable = ent.search.as_ref().map(|c| !c.is_empty()).unwrap_or(false);
        if searchable {
            if let (Some(cfg), Ok(Some(row))) = (
                ent.search.as_ref(),
                self.get_by_id_with_conn(conn, entity, id),
            ) {
                pylon_storage::search_maintenance::apply_delete(conn, entity, id, &row, cfg)
                    .map_err(|e| RuntimeError {
                        code: "SEARCH_MAINTENANCE_FAILED".into(),
                        message: format!("search index update on delete {entity}: {e}"),
                    })?;
            }
        }

        let sql = format!("DELETE FROM {} WHERE \"id\" = ?1", quote_ident(entity));
        let affected = conn
            .execute(&sql, rusqlite::params![id])
            .map_err(|e| RuntimeError {
                code: "DELETE_FAILED".into(),
                message: format!("Delete {entity}/{id} failed: {e}"),
            })?;
        Ok(affected > 0)
    }

    /// Read a row by id using a pre-held connection (for transactions).
    pub fn get_by_id_with_conn(
        &self,
        conn: &Connection,
        entity: &str,
        id: &str,
    ) -> Result<Option<serde_json::Value>, RuntimeError> {
        let ent = self.require_entity(entity)?;
        let sql = format!("SELECT * FROM {} WHERE \"id\" = ?1", quote_ident(entity));
        let mut stmt = conn.prepare_cached(&sql).map_err(|e| RuntimeError {
            code: "QUERY_FAILED".into(),
            message: format!("Failed to prepare query: {e}"),
        })?;
        let columns: Vec<String> = ent.fields.iter().map(|f| f.name.clone()).collect();
        Ok(stmt
            .query_row(rusqlite::params![id], |row| Ok(row_to_json(row, &columns)))
            .ok())
    }

    /// List rows using a pre-held connection (for transactions).
    pub fn list_with_conn(
        &self,
        conn: &Connection,
        entity: &str,
    ) -> Result<Vec<serde_json::Value>, RuntimeError> {
        let ent = self.require_entity(entity)?;
        let sql = format!("SELECT * FROM {} ORDER BY \"id\"", quote_ident(entity));
        let mut stmt = conn.prepare_cached(&sql).map_err(|e| RuntimeError {
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
        Ok(rows.flatten().collect())
    }

    /// List after cursor using a pre-held connection (for transactions).
    pub fn list_after_with_conn(
        &self,
        conn: &Connection,
        entity: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>, RuntimeError> {
        let ent = self.require_entity(entity)?;
        let columns: Vec<String> = ent.fields.iter().map(|f| f.name.clone()).collect();
        let table = quote_ident(entity);
        let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match after {
            Some(cursor) => (
                format!("SELECT * FROM {table} WHERE \"id\" > ?1 ORDER BY \"id\" LIMIT ?2"),
                vec![Box::new(cursor.to_string()), Box::new(limit as i64)],
            ),
            None => (
                format!("SELECT * FROM {table} ORDER BY \"id\" LIMIT ?1"),
                vec![Box::new(limit as i64)],
            ),
        };
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|v| v.as_ref()).collect();
        let mut stmt = conn.prepare_cached(&sql).map_err(|e| RuntimeError {
            code: "QUERY_FAILED".into(),
            message: format!("Failed to prepare: {e}"),
        })?;
        let rows = stmt
            .query_map(param_refs.as_slice(), |row| Ok(row_to_json(row, &columns)))
            .map_err(|e| RuntimeError {
                code: "QUERY_FAILED".into(),
                message: format!("Query failed: {e}"),
            })?;
        Ok(rows.flatten().collect())
    }

    /// Lookup by field using a pre-held connection (for transactions).
    pub fn lookup_with_conn(
        &self,
        conn: &Connection,
        entity: &str,
        field: &str,
        value: &str,
    ) -> Result<Option<serde_json::Value>, RuntimeError> {
        let ent = self.require_entity(entity)?;
        validate_column_name(field, ent)?;
        let sql = format!(
            "SELECT * FROM {} WHERE {} = ?1 LIMIT 1",
            quote_ident(entity),
            quote_ident(field)
        );
        let columns: Vec<String> = ent.fields.iter().map(|f| f.name.clone()).collect();
        Ok(conn.prepare_cached(&sql).ok().and_then(|mut stmt| {
            stmt.query_row(rusqlite::params![value], |row| {
                Ok(row_to_json(row, &columns))
            })
            .ok()
        }))
    }

    /// Link relation using a pre-held connection (for transactions).
    pub fn link_with_conn(
        &self,
        conn: &Connection,
        entity: &str,
        id: &str,
        relation: &str,
        target_id: &str,
    ) -> Result<bool, RuntimeError> {
        let ent = self.require_entity(entity)?;
        let rel = ent
            .relations
            .iter()
            .find(|r| r.name == relation)
            .ok_or_else(|| RuntimeError {
                code: "RELATION_NOT_FOUND".into(),
                message: format!("Relation \"{relation}\" not found on \"{entity}\""),
            })?;
        let data = serde_json::json!({ rel.field.clone(): target_id });
        self.update_with_conn(conn, entity, id, &data)
    }

    /// Unlink relation using a pre-held connection (for transactions).
    pub fn unlink_with_conn(
        &self,
        conn: &Connection,
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
                message: format!("Relation \"{relation}\" not found on \"{entity}\""),
            })?;
        let data = serde_json::json!({ rel.field.clone(): serde_json::Value::Null });
        self.update_with_conn(conn, entity, id, &data)
    }

    /// Query with filters using a pre-held connection (for transactions).
    ///
    /// Shares the filter-building logic with [`query_filtered`] by executing
    /// against the provided connection rather than acquiring one.
    pub fn query_filtered_with_conn(
        &self,
        conn: &Connection,
        entity: &str,
        filter: &serde_json::Value,
    ) -> Result<Vec<serde_json::Value>, RuntimeError> {
        let ent = self.require_entity(entity)?;
        let columns: Vec<String> = ent.fields.iter().map(|f| f.name.clone()).collect();
        let empty = serde_json::Map::new();
        let obj = filter.as_object().unwrap_or(&empty);

        let mut where_clauses = Vec::new();
        let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        let mut order_clause = String::new();
        let mut limit_clause = String::new();
        let mut idx = 1;

        for (key, val) in obj {
            match key.as_str() {
                "$order" => {
                    if let Some(o) = val.as_object() {
                        let mut parts: Vec<String> = Vec::new();
                        for (col, dir) in o {
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
                "$offset" => {
                    if let Some(n) = val.as_u64() {
                        if limit_clause.is_empty() {
                            limit_clause = " LIMIT -1".into();
                        }
                        limit_clause = format!("{limit_clause} OFFSET {n}");
                    }
                }
                _ => {
                    validate_column_name(key, ent)?;
                    let qk = quote_ident(key);
                    if let Some(op_obj) = val.as_object() {
                        for (op, op_val) in op_obj {
                            match op.as_str() {
                                "$not" => {
                                    where_clauses.push(format!("{qk} != ?{idx}"));
                                    values.push(json_to_sql(op_val));
                                    idx += 1;
                                }
                                "$gt" => {
                                    where_clauses.push(format!("{qk} > ?{idx}"));
                                    values.push(json_to_sql(op_val));
                                    idx += 1;
                                }
                                "$gte" => {
                                    where_clauses.push(format!("{qk} >= ?{idx}"));
                                    values.push(json_to_sql(op_val));
                                    idx += 1;
                                }
                                "$lt" => {
                                    where_clauses.push(format!("{qk} < ?{idx}"));
                                    values.push(json_to_sql(op_val));
                                    idx += 1;
                                }
                                "$lte" => {
                                    where_clauses.push(format!("{qk} <= ?{idx}"));
                                    values.push(json_to_sql(op_val));
                                    idx += 1;
                                }
                                "$like" => {
                                    where_clauses.push(format!("{qk} LIKE ?{idx}"));
                                    let p = format!("%{}%", op_val.as_str().unwrap_or(""));
                                    values.push(Box::new(p));
                                    idx += 1;
                                }
                                "$in" => {
                                    if let Some(arr) = op_val.as_array() {
                                        let ph: Vec<String> = arr
                                            .iter()
                                            .map(|v| {
                                                let p = format!("?{idx}");
                                                values.push(json_to_sql(v));
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
            order_clause = " ORDER BY \"id\"".into();
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
        let mut stmt = conn.prepare_cached(&sql).map_err(|e| RuntimeError {
            code: "QUERY_FAILED".into(),
            message: format!("Failed to prepare: {e}"),
        })?;
        let rows = stmt
            .query_map(param_refs.as_slice(), |row| Ok(row_to_json(row, &columns)))
            .map_err(|e| RuntimeError {
                code: "QUERY_FAILED".into(),
                message: format!("Query failed: {e}"),
            })?;
        Ok(rows.flatten().collect())
    }

    /// Graph query using a pre-held connection (for transactions).
    pub fn query_graph_with_conn(
        &self,
        conn: &Connection,
        query: &serde_json::Value,
    ) -> Result<serde_json::Value, RuntimeError> {
        let obj = query.as_object().ok_or_else(|| RuntimeError {
            code: "INVALID_QUERY".into(),
            message: "Graph query must be a JSON object".into(),
        })?;
        let mut results = serde_json::Map::new();
        for (entity_name, query_opts) in obj {
            let _ent = self.require_entity(entity_name)?;
            let filter = query_opts
                .get("where")
                .cloned()
                .unwrap_or(serde_json::json!({}));
            let rows = self.query_filtered_with_conn(conn, entity_name, &filter)?;
            results.insert(entity_name.clone(), serde_json::json!(rows));
        }
        Ok(serde_json::Value::Object(results))
    }

    // -----------------------------------------------------------------------
    // Aggregations — count, sum, avg, min, max, group by
    // -----------------------------------------------------------------------

    /// Run an aggregation query. See [`pylon_http::DataStore::aggregate`]
    /// for the spec shape.
    pub fn aggregate(
        &self,
        entity: &str,
        spec: &serde_json::Value,
    ) -> Result<serde_json::Value, RuntimeError> {
        if let Some(pg) = self.pg_backend() {
            return pylon_http::DataStore::aggregate(&pg.store, entity, spec)
                .map_err(data_err_to_runtime);
        }
        let ent = self.require_entity(entity)?;
        let conn = self.lock_read_conn()?;
        let obj = spec.as_object().ok_or_else(|| RuntimeError {
            code: "INVALID_QUERY".into(),
            message: "aggregate spec must be an object".into(),
        })?;

        // Build the SELECT list.
        let mut select_parts: Vec<String> = Vec::new();
        let mut result_fields: Vec<String> = Vec::new();

        if let Some(count) = obj.get("count") {
            match count {
                serde_json::Value::String(s) if s == "*" => {
                    select_parts.push("COUNT(*) AS count".into());
                    result_fields.push("count".into());
                }
                serde_json::Value::String(field) => {
                    validate_column_name(field, ent)?;
                    let alias = format!("count_{field}");
                    select_parts.push(format!(
                        "COUNT({}) AS {}",
                        quote_ident(field),
                        quote_ident(&alias)
                    ));
                    result_fields.push(alias);
                }
                _ => {}
            }
        }

        for (fn_name, alias_prefix) in [
            ("sum", "sum_"),
            ("avg", "avg_"),
            ("min", "min_"),
            ("max", "max_"),
        ] {
            if let Some(fields) = obj.get(fn_name).and_then(|v| v.as_array()) {
                for field in fields {
                    if let Some(f) = field.as_str() {
                        validate_column_name(f, ent)?;
                        let alias = format!("{alias_prefix}{f}");
                        let sql_fn = fn_name.to_uppercase();
                        select_parts.push(format!(
                            "{}({}) AS {}",
                            sql_fn,
                            quote_ident(f),
                            quote_ident(&alias)
                        ));
                        result_fields.push(alias);
                    }
                }
            }
        }

        // countDistinct — separate handler because COUNT(DISTINCT) is a
        // distinct SQL form from COUNT(field). Lets dashboards ask "how
        // many unique customers placed orders this month" without a
        // client-side post-processing pass.
        if let Some(fields) = obj.get("countDistinct").and_then(|v| v.as_array()) {
            for field in fields {
                if let Some(f) = field.as_str() {
                    validate_column_name(f, ent)?;
                    let alias = format!("count_distinct_{f}");
                    select_parts.push(format!(
                        "COUNT(DISTINCT {}) AS {}",
                        quote_ident(f),
                        quote_ident(&alias)
                    ));
                    result_fields.push(alias);
                }
            }
        }

        // Group-by fields come first in the SELECT so each row is identifiable.
        // Each entry is either a plain column name (string) or a date-bucket
        // spec — `{ field: "createdAt", bucket: "day" }`. Buckets map to
        // SQLite strftime patterns so aggregation keys collapse to the
        // bucket boundary (hour / day / week / month / year).
        let mut group_by: Vec<String> = Vec::new();
        let mut group_select: Vec<String> = Vec::new();
        let mut group_field_names: Vec<String> = Vec::new();
        if let Some(groups) = obj.get("groupBy").and_then(|v| v.as_array()) {
            for g in groups {
                if let Some(f) = g.as_str() {
                    validate_column_name(f, ent)?;
                    let quoted = quote_ident(f);
                    group_by.push(quoted.clone());
                    group_select.push(quoted);
                    group_field_names.push(f.to_string());
                } else if let Some(spec) = g.as_object() {
                    let field =
                        spec.get("field")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| RuntimeError {
                                code: "INVALID_QUERY".into(),
                                message: "groupBy object spec requires `field`".into(),
                            })?;
                    validate_column_name(field, ent)?;
                    let bucket = spec.get("bucket").and_then(|v| v.as_str()).unwrap_or("day");
                    let fmt = match bucket {
                        "hour" => "%Y-%m-%d %H:00:00",
                        "day" => "%Y-%m-%d",
                        "month" => "%Y-%m",
                        "year" => "%Y",
                        "week" => "%Y-W%W",
                        _ => {
                            return Err(RuntimeError {
                                code: "INVALID_QUERY".into(),
                                message: format!(
                                    "bucket must be one of hour/day/week/month/year, got {bucket}"
                                ),
                            });
                        }
                    };
                    let alias = format!("{field}_{bucket}");
                    let expr = format!("strftime('{}', {})", fmt, quote_ident(field));
                    group_by.push(expr.clone());
                    group_select.push(format!("{} AS {}", expr, quote_ident(&alias)));
                    group_field_names.push(alias);
                }
            }
        }
        let mut full_select = group_select.clone();
        full_select.extend(select_parts.iter().cloned());
        if full_select.is_empty() {
            return Err(RuntimeError {
                code: "INVALID_QUERY".into(),
                message: "aggregate spec must include count/sum/avg/min/max/groupBy".into(),
            });
        }

        // WHERE clause (reuse filter syntax, but only simple equality for now).
        let mut where_clauses = Vec::new();
        let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        let mut idx = 1;
        if let Some(where_obj) = obj.get("where").and_then(|v| v.as_object()) {
            for (k, v) in where_obj {
                validate_column_name(k, ent)?;
                where_clauses.push(format!("{} = ?{idx}", quote_ident(k)));
                values.push(json_to_sql(v));
                idx += 1;
            }
        }
        let where_sql = if where_clauses.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", where_clauses.join(" AND "))
        };

        let group_sql = if group_by.is_empty() {
            String::new()
        } else {
            format!(" GROUP BY {}", group_by.join(", "))
        };

        let sql = format!(
            "SELECT {} FROM {}{}{}",
            full_select.join(", "),
            quote_ident(entity),
            where_sql,
            group_sql
        );

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            values.iter().map(|v| v.as_ref()).collect();
        let mut stmt = conn.prepare_cached(&sql).map_err(|e| RuntimeError {
            code: "QUERY_FAILED".into(),
            message: format!("Failed to prepare aggregate: {e}"),
        })?;

        let column_names: Vec<String> = {
            let mut v = group_field_names.clone();
            v.extend(result_fields.iter().cloned());
            v
        };

        let rows = stmt
            .query_map(param_refs.as_slice(), |row| {
                let mut obj = serde_json::Map::new();
                for (i, name) in column_names.iter().enumerate() {
                    // Try int first (counts/sums), then float, then string, then null.
                    if let Ok(n) = row.get::<_, i64>(i) {
                        obj.insert(name.clone(), serde_json::Value::Number(n.into()));
                    } else if let Ok(f) = row.get::<_, f64>(i) {
                        if let Some(num) = serde_json::Number::from_f64(f) {
                            obj.insert(name.clone(), serde_json::Value::Number(num));
                        } else {
                            obj.insert(name.clone(), serde_json::Value::Null);
                        }
                    } else if let Ok(s) = row.get::<_, String>(i) {
                        obj.insert(name.clone(), serde_json::Value::String(s));
                    } else {
                        obj.insert(name.clone(), serde_json::Value::Null);
                    }
                }
                Ok(serde_json::Value::Object(obj))
            })
            .map_err(|e| RuntimeError {
                code: "QUERY_FAILED".into(),
                message: format!("Aggregate failed: {e}"),
            })?;

        let mut result = Vec::new();
        for row in rows {
            if let Ok(val) = row {
                result.push(val);
            }
        }
        Ok(serde_json::json!({ "rows": result }))
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
    /// SQLite-only — Postgres callers should never reach this (each
    /// public CRUD method branches at the top and dispatches to
    /// `PostgresDataStore` first). Returns `NOT_SQLITE_BACKEND` if
    /// invoked on a Postgres runtime, which indicates a missing dispatch.
    fn lock_write_conn(&self) -> Result<std::sync::MutexGuard<'_, Connection>, RuntimeError> {
        let sb = self.sqlite_backend()?;
        sb.write_conn.lock().map_err(|e| RuntimeError {
            code: "LOCK_FAILED".into(),
            message: format!("Failed to acquire write connection lock: {e}"),
        })
    }

    /// Acquire a read connection. Uses the read pool if available (file-backed
    /// databases), otherwise falls back to the write connection (in-memory).
    /// Connections are selected round-robin to spread load evenly. SQLite-only.
    fn lock_read_conn(&self) -> Result<ReadConnGuard<'_>, RuntimeError> {
        let sb = self.sqlite_backend()?;
        if !sb.read_pool.is_empty() {
            let idx = sb.read_counter.fetch_add(1, Ordering::Relaxed) % sb.read_pool.len();
            let guard = sb.read_pool[idx].lock().map_err(|e| RuntimeError {
                code: "LOCK_FAILED".into(),
                message: format!("Failed to acquire read connection: {e}"),
            })?;
            Ok(ReadConnGuard::Pooled(guard))
        } else {
            // Fall back to write connection for in-memory DBs.
            let guard = sb.write_conn.lock().map_err(|e| RuntimeError {
                code: "LOCK_FAILED".into(),
                message: format!("Failed to acquire connection: {e}"),
            })?;
            Ok(ReadConnGuard::Write(guard))
        }
    }

    /// Borrow the SQLite backend, or fail with `NOT_SQLITE_BACKEND` if
    /// this runtime is Postgres-backed. Used by every SQLite-specific
    /// helper as a single point of dispatch.
    fn sqlite_backend(&self) -> Result<&SqliteBackend, RuntimeError> {
        match &self.backend {
            RuntimeBackend::Sqlite(sb) => Ok(sb),
            RuntimeBackend::Postgres(_) => Err(RuntimeError {
                code: "NOT_SQLITE_BACKEND".into(),
                message: "this operation requires a SQLite-backed Runtime".into(),
            }),
        }
    }

    /// Borrow the Postgres backend, or `None` for SQLite. Used by the
    /// per-method dispatch at the top of each entity-CRUD function
    /// AND by the `DataStore` impl in `datastore.rs` to reach the
    /// CRDT sidecar.
    pub(crate) fn pg_backend(&self) -> Option<&PgBackend> {
        match &self.backend {
            RuntimeBackend::Sqlite(_) => None,
            RuntimeBackend::Postgres(pg) => Some(pg),
        }
    }

    /// Borrow the underlying Postgres `DataStore` if this runtime is
    /// Postgres-backed. Used by the `DataStore` adapter in `datastore.rs`
    /// to delegate `transact`/`search` etc. without re-implementing them.
    /// Accessor for the underlying PostgresDataStore. Used by
    /// integration tests to exercise in-tx primitives directly
    /// without going through a TS function handler. Also useful for
    /// callers that need to drop down to raw PG (e.g. running an
    /// EXPLAIN against the live cluster from an admin tool).
    /// Returns None on SQLite-backed runtimes.
    pub fn pg_data_store_pub(&self) -> Option<&pylon_storage::pg_datastore::PostgresDataStore> {
        self.pg_data_store()
    }

    #[doc(hidden)]
    pub fn pg_data_store_for_tests(&self) -> &pylon_storage::pg_datastore::PostgresDataStore {
        self.pg_data_store().expect("pg backend")
    }

    /// Test-only: run a closure inside a PG mutation tx with the
    /// CRDT hook installed — same code path FnOpsImpl::call uses
    /// for `Mutation` handlers. Lets integration tests verify the
    /// hook without spinning up a Bun runtime.
    #[doc(hidden)]
    pub fn run_in_pg_mutation_tx_for_tests<F, T, E>(&self, body: F) -> Result<T, E>
    where
        F: FnOnce(&dyn pylon_http::DataStore) -> Result<T, E>,
        E: From<pylon_http::DataError>,
    {
        let pg_backend = self.pg_backend().expect("pg backend");
        let crdt_hook: std::sync::Arc<dyn pylon_storage::pg_tx_store::PgCrdtHook> =
            std::sync::Arc::new(crate::pg_loro_store::PgCrdtHookImpl {
                crdt: std::sync::Arc::clone(&pg_backend.crdt),
                manifest: std::sync::Arc::new(self.manifest.clone()),
            });
        pg_backend
            .store
            .with_transaction_crdt(crdt_hook, body)
    }

    pub(crate) fn pg_data_store(&self) -> Option<&pylon_storage::pg_datastore::PostgresDataStore> {
        self.pg_backend().map(|pg| &pg.store)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Generate a lex-sortable, monotonic-ish unique ID.
///
/// Same shape as `pylon_storage::postgres::generate_id` — fixed-width hex
/// of nanoseconds + 8-hex per-process counter (40 chars total). The fixed
/// width is what makes `WHERE id > $1 ORDER BY id` correct for cursor
/// pagination: variable-width hex sorts incorrectly at width boundaries
/// (e.g. `"ff"` lex-sorts after `"100"`).
/// Run `body` inside a SQLite transaction on `conn`. Commits on `Ok`,
/// rolls back on `Err` (or if `body` panics).
///
/// Used to make the multi-statement CRDT write paths (LoroDoc snapshot
/// upsert into `_pylon_crdt_snapshots` + the materialized entity row
/// INSERT/UPDATE + FTS / facet maintenance) atomic so a crash mid-write
/// can never leave the materialized view stale relative to the CRDT
/// snapshot. Uses unmanaged BEGIN/COMMIT/ROLLBACK rather than rusqlite's
/// `Transaction` API because the existing call sites borrow `conn`
/// through inner closures and the lifetime juggling for a `Transaction`
/// guard would force more refactoring than the explicit BEGIN/COMMIT.
///
/// `BEGIN IMMEDIATE` (vs the default `BEGIN DEFERRED`) takes the SQLite
/// reserved lock on entry instead of escalating later — matches the
/// pattern in `datastore.rs::transact` and avoids a SQLITE_BUSY race
/// where a concurrent reader prevents the lock upgrade mid-write.
fn with_write_tx<T, F>(conn: &rusqlite::Connection, body: F) -> Result<T, RuntimeError>
where
    F: FnOnce() -> Result<T, RuntimeError>,
{
    conn.execute("BEGIN IMMEDIATE", [])
        .map_err(|e| RuntimeError {
            code: "TX_BEGIN_FAILED".into(),
            message: format!("BEGIN: {e}"),
        })?;
    match body() {
        Ok(v) => {
            conn.execute("COMMIT", []).map_err(|e| RuntimeError {
                code: "TX_COMMIT_FAILED".into(),
                message: format!("COMMIT: {e}"),
            })?;
            Ok(v)
        }
        Err(e) => {
            // Best-effort rollback; if even ROLLBACK fails we surface
            // the *original* error since that's the more actionable one.
            let _ = conn.execute("ROLLBACK", []);
            Err(e)
        }
    }
}

fn generate_id() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{nanos:032x}{seq:08x}")
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

/// Convert a rusqlite row to a JSON value.
///
/// Reads columns by NAME (via the row's actual column metadata) rather
/// than by positional index. The previous implementation assumed the
/// SQLite table column order matched the manifest field order, which
/// silently breaks when a new field is inserted in the middle of the
/// manifest: SQLite's `ALTER TABLE ADD COLUMN` always appends to the
/// end of the table, so existing data lands in the wrong field on
/// every read.
///
/// `field_names` is still passed (unused in the body, kept for API
/// stability with callers that compute it from the manifest) — the
/// name set comes from the row itself now, which always matches the
/// SELECT's actual column shape.
fn row_to_json(row: &rusqlite::Row<'_>, _field_names: &[String]) -> serde_json::Value {
    let mut obj = serde_json::Map::new();

    let stmt = row.as_ref();
    let count = stmt.column_count();
    for i in 0..count {
        // Column names are short string slices into the prepared
        // statement; copy out into owned Strings before inserting into
        // the map (the slice borrow can't outlive the row).
        let name = match stmt.column_name(i) {
            Ok(n) => n.to_string(),
            Err(_) => continue,
        };
        let value = if let Ok(s) = row.get::<_, String>(i) {
            serde_json::Value::String(s)
        } else if let Ok(n) = row.get::<_, i64>(i) {
            serde_json::Value::Number(serde_json::Number::from(n))
        } else if let Ok(f) = row.get::<_, f64>(i) {
            serde_json::Number::from_f64(f)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null)
        } else {
            serde_json::Value::Null
        };
        obj.insert(name, value);
    }

    serde_json::Value::Object(obj)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pylon_kernel::{ManifestField, ManifestIndex};

    fn test_manifest() -> AppManifest {
        AppManifest {
            manifest_version: 1,
            name: "Test".into(),
            version: "0.1.0".into(),
            entities: vec![pylon_kernel::ManifestEntity {
                name: "User".into(),
                fields: vec![
                    ManifestField {
                        name: "email".into(),
                        field_type: "string".into(),
                        optional: false,
                        unique: true,
                        crdt: None,
                    },
                    ManifestField {
                        name: "displayName".into(),
                        field_type: "string".into(),
                        optional: false,
                        unique: false,
                        crdt: None,
                    },
                ],
                indexes: vec![ManifestIndex {
                    name: "user_email".into(),
                    fields: vec!["email".into()],
                    unique: true,
                }],
                relations: vec![],
                search: None,
                crdt: true,
            }],
            routes: vec![],
            queries: vec![],
            actions: vec![],
            policies: vec![],
            auth: Default::default(),
        }
    }

    #[test]
    fn reset_for_tests_wipes_in_memory() {
        let rt = Runtime::in_memory(test_manifest()).unwrap();
        rt.insert(
            "User",
            &serde_json::json!({"email": "a@b.com", "displayName": "A"}),
        )
        .unwrap();
        assert_eq!(rt.list("User").unwrap().len(), 1);
        rt.reset_for_tests().unwrap();
        assert_eq!(rt.list("User").unwrap().len(), 0);
    }

    #[test]
    fn reset_for_tests_refuses_file_db() {
        let dir = std::env::temp_dir().join("pylon-reset-refuse");
        let _ = std::fs::create_dir_all(&dir);
        let db_path = dir.join("db.sqlite");
        let _ = std::fs::remove_file(&db_path);
        let rt = Runtime::open(db_path.to_str().unwrap(), test_manifest()).unwrap();
        let err = rt.reset_for_tests().unwrap_err();
        assert_eq!(err.code, "RESET_REFUSED");
        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn insert_and_get() {
        let rt = Runtime::in_memory(test_manifest()).unwrap();
        let id = rt
            .insert(
                "User",
                &serde_json::json!({"email": "a@b.com", "displayName": "A"}),
            )
            .unwrap();
        let row = rt.get_by_id("User", &id).unwrap().unwrap();
        assert_eq!(row["email"], "a@b.com");
    }

    /// Regression: when a new field is added in the middle of a manifest,
    /// SQLite ALTER TABLE ADD COLUMN appends it to the end of the table.
    /// The previous `row_to_json` read columns by positional index in
    /// manifest order, so existing data shifted into the wrong fields
    /// on every read (createdAt's value showed up as the new field's,
    /// and vice versa). row_to_json now reads by column name from the
    /// row's own metadata, so the bug can't recur regardless of
    /// migration order.
    #[test]
    fn row_to_json_handles_columns_added_out_of_manifest_order() {
        // Manifest: id, email, displayName, avatarColor, createdAt
        let mut manifest = test_manifest();
        manifest.entities[0].fields = vec![
            ManifestField {
                name: "email".into(),
                field_type: "string".into(),
                optional: false,
                unique: true,
                crdt: None,
            },
            ManifestField {
                name: "displayName".into(),
                field_type: "string".into(),
                optional: false,
                unique: false,
                crdt: None,
            },
            ManifestField {
                name: "avatarColor".into(),
                field_type: "string".into(),
                optional: true,
                unique: false,
                crdt: None,
            },
            ManifestField {
                name: "createdAt".into(),
                field_type: "datetime".into(),
                optional: true,
                unique: false,
                crdt: None,
            },
        ];
        // Important: turn off CRDT mode for this test — CRDT mode writes
        // the projection back to SQLite explicitly per-field, so it
        // wouldn't exercise the column-order bug we're regressing
        // against. The bug bites the legacy path that still does
        // `INSERT (id, email, displayName, ...) VALUES (...)` and then
        // `SELECT * ... → row_to_json` to read it back.
        manifest.entities[0].crdt = false;
        let rt = Runtime::in_memory(manifest).unwrap();
        let id = rt
            .insert(
                "User",
                &serde_json::json!({
                    "email": "a@b.com",
                    "displayName": "Alice",
                    "avatarColor": "#abc",
                    "createdAt": "2026-01-01T00:00:00Z",
                }),
            )
            .unwrap();

        // Simulate an ALTER TABLE ADD COLUMN that appends a new field
        // at the end of the SQLite table even though the manifest
        // places it in the middle. This is the exact shape of what
        // happens when a user adds a new field between existing ones
        // and pylon dev migrates the table forward.
        {
            let conn = rt.lock_write_conn().unwrap();
            conn.execute("ALTER TABLE \"User\" ADD COLUMN \"passwordHash\" TEXT", [])
                .unwrap();
            conn.execute(
                "UPDATE \"User\" SET \"passwordHash\" = ?1 WHERE \"id\" = ?2",
                rusqlite::params!["hashed-password", &id],
            )
            .unwrap();
        }
        // Update the in-memory manifest to reflect the new field
        // sitting between avatarColor and createdAt — this is what the
        // regenerated manifest would look like.
        // (We mutate via the storage path to mirror the actual flow.)

        let row = rt.get_by_id("User", &id).unwrap().unwrap();
        // The crucial assertions: each column maps to its own value,
        // not the value of whichever column happens to share its
        // SQLite position.
        assert_eq!(row["email"], "a@b.com");
        assert_eq!(row["displayName"], "Alice");
        assert_eq!(row["avatarColor"], "#abc");
        assert_eq!(row["createdAt"], "2026-01-01T00:00:00Z");
        assert_eq!(row["passwordHash"], "hashed-password");
    }

    /// CRDT-mode entities (the default) populate the sidecar snapshot
    /// table on every write — the LoroDoc is the source of truth, the
    /// SQLite row is the materialized projection. This proves the CRDT
    /// branch in `insert` actually fires.
    #[test]
    fn crdt_default_writes_through_loro_store() {
        let rt = Runtime::in_memory(test_manifest()).unwrap();
        let id = rt
            .insert(
                "User",
                &serde_json::json!({"email": "x@y.com", "displayName": "Eric"}),
            )
            .unwrap();

        // Sidecar contains exactly one snapshot for the new row.
        let conn = rt.lock_write_conn().unwrap();
        let snap_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM _pylon_crdt_snapshots
                 WHERE entity = ?1 AND row_id = ?2",
                rusqlite::params!["User", &id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(snap_count, 1, "sidecar should have one row after insert");

        // Loro doc is cached in memory after the write — proves
        // get_or_hydrate ran during apply_patch.
        assert!(rt.crdt_store().cached_rows() >= 1);

        // SQLite materialized view has the projected row.
        drop(conn);
        let row = rt.get_by_id("User", &id).unwrap().unwrap();
        assert_eq!(row["email"], "x@y.com");
        assert_eq!(row["displayName"], "Eric");
    }

    /// Updates write through the LoroDoc as well — verifies the sidecar
    /// snapshot grows (Loro tracks new ops) and the materialized row
    /// reflects the new value.
    #[test]
    fn crdt_update_persists_new_snapshot() {
        let rt = Runtime::in_memory(test_manifest()).unwrap();
        let id = rt
            .insert(
                "User",
                &serde_json::json!({"email": "x@y.com", "displayName": "Eric"}),
            )
            .unwrap();

        let snap_after_insert: Vec<u8> = {
            let conn = rt.lock_write_conn().unwrap();
            conn.query_row(
                "SELECT snapshot FROM _pylon_crdt_snapshots
                 WHERE entity = 'User' AND row_id = ?1",
                rusqlite::params![&id],
                |r| r.get(0),
            )
            .unwrap()
        };

        rt.update("User", &id, &serde_json::json!({"displayName": "Eric C"}))
            .unwrap();

        let snap_after_update: Vec<u8> = {
            let conn = rt.lock_write_conn().unwrap();
            conn.query_row(
                "SELECT snapshot FROM _pylon_crdt_snapshots
                 WHERE entity = 'User' AND row_id = ?1",
                rusqlite::params![&id],
                |r| r.get(0),
            )
            .unwrap()
        };

        assert_ne!(
            snap_after_insert, snap_after_update,
            "snapshot bytes should change after an update"
        );

        let row = rt.get_by_id("User", &id).unwrap().unwrap();
        assert_eq!(row["displayName"], "Eric C");
        assert_eq!(row["email"], "x@y.com");
    }

    /// Regression: when the SQL INSERT step inside Runtime::insert fails
    /// (UNIQUE-constraint violation here), the LoroDoc snapshot must
    /// also roll back — neither half lands. Previously the LoroStore
    /// wrote first and committed independently, so a doomed INSERT left
    /// a sidecar row pointing at a doc that the materialized table
    /// never knew about.
    #[test]
    fn crdt_insert_rolls_back_when_sql_step_fails() {
        let rt = Runtime::in_memory(test_manifest()).unwrap();
        // Seed a row.
        rt.insert(
            "User",
            &serde_json::json!({"email": "x@y.com", "displayName": "First"}),
        )
        .unwrap();

        // Snapshot the sidecar row count BEFORE the failing insert.
        let snap_count_before: i64 = {
            let conn = rt.lock_write_conn().unwrap();
            conn.query_row(
                "SELECT COUNT(*) FROM _pylon_crdt_snapshots WHERE entity = 'User'",
                [],
                |r| r.get(0),
            )
            .unwrap()
        };

        // Attempt a duplicate-email insert. SQL UNIQUE rejects.
        let err = rt
            .insert(
                "User",
                &serde_json::json!({"email": "x@y.com", "displayName": "Second"}),
            )
            .expect_err("duplicate email must fail");
        assert_eq!(err.code, "INSERT_FAILED");

        // Sidecar row count unchanged — the LoroDoc snapshot the CRDT
        // path wrote was rolled back along with the failed SQL INSERT.
        let snap_count_after: i64 = {
            let conn = rt.lock_write_conn().unwrap();
            conn.query_row(
                "SELECT COUNT(*) FROM _pylon_crdt_snapshots WHERE entity = 'User'",
                [],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(
            snap_count_after, snap_count_before,
            "failed insert should not leave a sidecar snapshot behind"
        );
    }

    /// Entities with `crdt: false` skip the LoroDoc entirely — no sidecar
    /// row, no Loro cache entry. Proves the opt-out actually opts out.
    #[test]
    fn crdt_false_skips_loro_store() {
        let mut manifest = test_manifest();
        // Flip the User entity to LWW-only mode.
        manifest.entities[0].crdt = false;
        let rt = Runtime::in_memory(manifest).unwrap();

        let id = rt
            .insert(
                "User",
                &serde_json::json!({"email": "lww@example.com", "displayName": "Plain"}),
            )
            .unwrap();

        let conn = rt.lock_write_conn().unwrap();
        let snap_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM _pylon_crdt_snapshots
                 WHERE entity = 'User' AND row_id = ?1",
                rusqlite::params![&id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(snap_count, 0, "crdt:false should not touch the sidecar");
        assert_eq!(
            rt.crdt_store().cached_rows(),
            0,
            "crdt:false should not warm the cache"
        );

        // SQLite path still works — the row landed via the legacy
        // direct-write path.
        drop(conn);
        let row = rt.get_by_id("User", &id).unwrap().unwrap();
        assert_eq!(row["email"], "lww@example.com");
    }

    #[test]
    fn list_entities() {
        let rt = Runtime::in_memory(test_manifest()).unwrap();
        rt.insert(
            "User",
            &serde_json::json!({"email": "a@b.com", "displayName": "A"}),
        )
        .unwrap();
        rt.insert(
            "User",
            &serde_json::json!({"email": "b@c.com", "displayName": "B"}),
        )
        .unwrap();
        let rows = rt.list("User").unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn update_entity() {
        let rt = Runtime::in_memory(test_manifest()).unwrap();
        let id = rt
            .insert(
                "User",
                &serde_json::json!({"email": "a@b.com", "displayName": "A"}),
            )
            .unwrap();
        let updated = rt
            .update("User", &id, &serde_json::json!({"displayName": "Updated"}))
            .unwrap();
        assert!(updated);
        let row = rt.get_by_id("User", &id).unwrap().unwrap();
        assert_eq!(row["displayName"], "Updated");
    }

    #[test]
    fn delete_entity() {
        let rt = Runtime::in_memory(test_manifest()).unwrap();
        let id = rt
            .insert(
                "User",
                &serde_json::json!({"email": "a@b.com", "displayName": "A"}),
            )
            .unwrap();
        let deleted = rt.delete("User", &id).unwrap();
        assert!(deleted);
        assert!(rt.get_by_id("User", &id).unwrap().is_none());
    }

    #[test]
    fn lookup_by_field() {
        let rt = Runtime::in_memory(test_manifest()).unwrap();
        rt.insert(
            "User",
            &serde_json::json!({"email": "a@b.com", "displayName": "A"}),
        )
        .unwrap();
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
            .insert(
                "User",
                &serde_json::json!({"email": "a@b.com", "displayName": "A", "evil_col": "x"}),
            )
            .unwrap_err();
        assert_eq!(err.code, "INVALID_COLUMN");
    }

    #[test]
    fn update_rejects_unknown_column() {
        let rt = Runtime::in_memory(test_manifest()).unwrap();
        let id = rt
            .insert(
                "User",
                &serde_json::json!({"email": "a@b.com", "displayName": "A"}),
            )
            .unwrap();
        let err = rt
            .update("User", &id, &serde_json::json!({"bad_field": "x"}))
            .unwrap_err();
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
        rt.insert(
            "User",
            &serde_json::json!({"email": "a@b.com", "displayName": "A"}),
        )
        .unwrap();
        // Even a malicious direction value should be normalized to ASC.
        let rows = rt
            .query_filtered(
                "User",
                &serde_json::json!({"$order": {"email": "DROP TABLE User"}}),
            )
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
        let dir = std::env::temp_dir().join(format!("pylon_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("test_read_pool.db");

        let rt = Runtime::open(db_path.to_str().unwrap(), test_manifest()).unwrap();
        assert_eq!(rt.read_pool_size(), READ_POOL_SIZE);

        // Write then read through the pool.
        let id = rt
            .insert(
                "User",
                &serde_json::json!({"email": "pool@test.com", "displayName": "Pool"}),
            )
            .unwrap();
        let row = rt.get_by_id("User", &id).unwrap().unwrap();
        assert_eq!(row["email"], "pool@test.com");

        // Clean up.
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn concurrent_reads_dont_block_on_write() {
        use std::sync::Arc;

        let dir = std::env::temp_dir().join(format!("pylon_conc_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("test_concurrent.db");

        let rt = Arc::new(Runtime::open(db_path.to_str().unwrap(), test_manifest()).unwrap());

        // Seed some data so reads have something to return.
        rt.insert(
            "User",
            &serde_json::json!({"email": "a@b.com", "displayName": "A"}),
        )
        .unwrap();
        rt.insert(
            "User",
            &serde_json::json!({"email": "b@c.com", "displayName": "B"}),
        )
        .unwrap();

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
