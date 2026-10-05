//! Copy an app's SQLite data into an empty Postgres database.
//!
//! A SQLite app keeps its state in up to four files: the app database
//! (entities, CRDT snapshots, function-call keys, the change log), the auth
//! database (`<db>.sessions.db` or `PYLON_SESSION_DB`), the job queue
//! (`<db>.jobs.db`) and the workflow store (`<db>.workflows.db`). On Postgres
//! all of it lives in one database, sometimes under other table names and
//! column types. [`copy`] moves every row so the app can switch backends
//! without losing data.
//!
//! The target is the runtime the caller opened on Postgres, after the entity
//! schema was applied (see `pylon start`). The copy:
//!
//! 1. creates the internal tables with the same constructors the server uses
//!    at boot, so the target schema is the one the server expects;
//! 2. refuses unless every target table is empty, or returns
//!    [`CopyOutcome::AlreadyCopied`] when an earlier copy finished;
//! 3. converts every value first and reports each one Postgres would refuse,
//!    before it writes anything;
//! 4. copies jobs and workflows through their stores, then the other tables
//!    in one transaction, fills the search index, moves the change-log
//!    sequence, compares row counts, and writes the [`MARKER_TABLE`] row.
//!
//! The app database and the auth database are opened read-only. The job and
//! workflow files are opened through their stores, which can upgrade an old
//! file's schema the same way the SQLite server does at boot.
//!
//! When a step after the empty check fails, the copy empties every table it
//! wrote, so the next attempt starts from the same state. That is safe
//! because the tables were empty at the start and the caller holds the boot
//! lock (`pg_boot_guard`), so no other process wrote to them.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use postgres::types::{ToSql, Type};
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags};

use crate::jobs::JobStatus;
use crate::Runtime;

/// The table that records a finished copy. One row; its presence makes a
/// later copy return [`CopyOutcome::AlreadyCopied`].
pub const MARKER_TABLE: &str = "_pylon_sqlite_import";

/// Failed boot-time copies, one row each. Not emptied with the copied
/// tables, so whoever runs the move (Stack0 Cloud reads it) can show why
/// the copy failed while the server keeps refusing to start.
pub const FAILURE_TABLE: &str = "_pylon_sqlite_import_failures";

/// Most problems listed in one report. The total is always given.
const MAX_LISTED_PROBLEMS: usize = 100;

/// Postgres accepts at most 65535 parameters per statement.
const MAX_PARAMS_PER_STATEMENT: usize = 60_000;

/// Rows per INSERT when that limit allows.
const MAX_ROWS_PER_STATEMENT: usize = 500;

// ---------------------------------------------------------------------------
// Source files
// ---------------------------------------------------------------------------

/// The SQLite files that hold one app's state.
#[derive(Debug, Clone)]
pub struct SqliteSource {
    pub app_db: PathBuf,
    pub sessions_db: PathBuf,
    pub jobs_db: PathBuf,
    pub workflows_db: PathBuf,
}

impl SqliteSource {
    /// The files a SQLite server with this app database uses, resolved the
    /// same way the server resolves them: `PYLON_SESSION_DB`, `PYLON_JOBS_DB`
    /// and `PYLON_WORKFLOWS_DB` win, else `<app_db>.sessions.db`,
    /// `<app_db>.jobs.db` and `<app_db>.workflows.db`.
    pub fn from_env(app_db: &str) -> Self {
        let sibling = |suffix: &str| PathBuf::from(format!("{app_db}.{suffix}"));
        let env_path = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
        Self {
            app_db: PathBuf::from(app_db),
            sessions_db: env_path("PYLON_SESSION_DB")
                .map(PathBuf::from)
                .unwrap_or_else(|| sibling("sessions.db")),
            jobs_db: env_path("PYLON_JOBS_DB")
                .map(PathBuf::from)
                .unwrap_or_else(|| sibling("jobs.db")),
            workflows_db: env_path("PYLON_WORKFLOWS_DB")
                .map(PathBuf::from)
                .unwrap_or_else(|| sibling("workflows.db")),
        }
    }
}

// ---------------------------------------------------------------------------
// Results
// ---------------------------------------------------------------------------

/// Rows copied into one Postgres table.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TableCount {
    pub table: String,
    pub rows: u64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CopyReport {
    pub tables: Vec<TableCount>,
    pub jobs: u64,
    pub workflows: u64,
    /// The value `pylon_change_seq` was moved to; 0 when the app had no
    /// change log.
    pub change_seq: u64,
    /// SQLite tables that are not copied: tables of entities the manifest no
    /// longer declares. Their data stays in the SQLite file.
    pub skipped: Vec<TableCount>,
    /// Entity columns that are not copied: fields the manifest no longer
    /// declares. Their data stays in the SQLite file.
    pub skipped_columns: Vec<SkippedColumn>,
}

/// A column of a field the manifest dropped, and how many rows hold a value.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SkippedColumn {
    pub table: String,
    pub column: String,
    pub rows_with_value: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyMode {
    /// Check every value and report; write no rows.
    Check,
    Write,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum CopyOutcome {
    Copied(CopyReport),
    /// [`CopyMode::Check`] found no problems. The counts are the rows a copy
    /// would write now; `skipped` as in [`CopyReport`].
    Checked {
        tables: Vec<TableCount>,
        skipped: Vec<TableCount>,
        skipped_columns: Vec<SkippedColumn>,
    },
    /// An earlier copy finished. Nothing was written.
    AlreadyCopied {
        copied_at: String,
        source: String,
    },
}

/// One value Postgres would refuse.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Problem {
    pub table: String,
    pub column: String,
    /// The row's key (`id`, or the primary-key columns joined with `/`).
    pub row: String,
    pub message: String,
}

#[derive(Debug)]
pub enum CopyError {
    /// Values Postgres would refuse. Nothing was written.
    Problems {
        total: usize,
        listed: Vec<Problem>,
    },
    /// A target table already holds rows and there is no copy marker.
    NotEmpty {
        tables: Vec<String>,
    },
    Failed(String),
}

impl std::fmt::Display for CopyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CopyError::Problems { total, listed } => {
                writeln!(f, "{total} value(s) cannot be stored in Postgres:")?;
                for p in listed {
                    writeln!(f, "  {}.{} row {}: {}", p.table, p.column, p.row, p.message)?;
                }
                if *total > listed.len() {
                    writeln!(f, "  ... and {} more", total - listed.len())?;
                }
                Ok(())
            }
            CopyError::NotEmpty { tables } => write!(
                f,
                "the target database already has data in: {}. Copy only into an empty database.",
                tables.join(", ")
            ),
            CopyError::Failed(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for CopyError {}

impl From<pylon_http::DataError> for CopyError {
    fn from(e: pylon_http::DataError) -> Self {
        CopyError::Failed(format!("{}: {}", e.code, e.message))
    }
}

impl From<postgres::Error> for CopyError {
    fn from(e: postgres::Error) -> Self {
        CopyError::from(pylon_storage::pg_tx_store::pg_err_to_data(e))
    }
}

fn failed(context: &str, e: impl std::fmt::Display) -> CopyError {
    CopyError::Failed(format!("{context}: {e}"))
}

// ---------------------------------------------------------------------------
// Copy plan
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceDb {
    App,
    Sessions,
}

/// One SQLite table copied into one Postgres table.
#[derive(Debug, Clone)]
struct TablePlan {
    db: SourceDb,
    source: String,
    target: String,
    /// `(sqlite column, postgres column)` pairs whose names differ.
    renames: &'static [(&'static str, &'static str)],
    /// SQLite columns with no Postgres counterpart, each dropped on purpose.
    dropped: &'static [&'static str],
    /// An entity table. SQLite keeps the column of a field the manifest
    /// dropped; such a column is skipped and reported. In any other table an
    /// unknown column is an error.
    entity: bool,
}

impl TablePlan {
    fn same(db: SourceDb, name: &str) -> Self {
        Self {
            db,
            source: name.to_string(),
            target: name.to_string(),
            renames: &[],
            dropped: &[],
            entity: false,
        }
    }

    fn target_column(&self, source_column: &str) -> String {
        self.renames
            .iter()
            .find(|(from, _)| *from == source_column)
            .map(|(_, to)| (*to).to_string())
            .unwrap_or_else(|| source_column.to_string())
    }
}

/// Internal tables in the app database whose name is the same on both
/// backends.
const APP_DB_TABLES: &[&str] = &[
    "_pylon_crdt_snapshots",
    "_pylon_crdt_synthetic",
    "_pylon_crdt_base",
    "_pylon_crdt_links",
    "_pylon_fn_calls",
    "_pylon_shard_state",
    "_pylon_shard_placements",
    "_pylon_shard_transfers",
    "_pylon_shard_cluster",
];

/// Internal tables in the app database that are not copied, and why.
const APP_DB_NOT_COPIED: &[&str] = &[
    // Moved with `setval` on `pylon_change_seq`.
    "_pylon_change_seq",
    // SQLite schema bookkeeping. Postgres keeps `_pylon_schema_state`,
    // which the schema step writes.
    "_pylon_schema_history",
    "_pylon_migrations",
    // CRDT reconcile progress. Postgres skips the reconcile.
    "_pylon_crdt_reconcile",
    // Machines that ran shards. The SQLite server forgets them at every
    // start, and Postgres machines register themselves.
    "_pylon_shard_machines",
    // Search index. The copy fills the Postgres index from the rows.
    "_facet_bitmap",
];

/// Search tables SQLite derives from the rows: `_fts_<Entity>`,
/// `<Entity>_fts`, and the FTS5 shadow tables of both.
fn is_sqlite_search_table(name: &str) -> bool {
    name.starts_with("_fts_") || name.ends_with("_fts") || name.contains("_fts_")
}

/// Auth tables. Same names on both backends; SQLite keeps them in the auth
/// database.
pub(crate) const AUTH_TABLES: &[&str] = &[
    "_pylon_sessions",
    "_pylon_oauth_state",
    "_pylon_magic_codes",
    "_pylon_session_handoff",
    "_pylon_accounts",
    "_pylon_api_keys",
    "_pylon_verification_tokens",
    "_pylon_audit_events",
    "_pylon_trusted_devices",
    "_pylon_org_sso",
    "_pylon_org_sso_state",
    "_pylon_org_saml",
    "_pylon_org_saml_state",
];

/// Tables the job and workflow stores write.
const JOB_WORKFLOW_TABLES: &[&str] = &[
    "_pylon_jobs",
    "_pylon_workflows",
    "_pylon_workflow_steps",
    "_pylon_workflow_events",
];

fn change_log_plan() -> TablePlan {
    TablePlan {
        db: SourceDb::App,
        source: "_pylon_change_log".into(),
        target: "pylon_change_log".into(),
        renames: &[("timestamp", "ts")],
        dropped: &[],
        entity: false,
    }
}

fn build_plan(runtime: &Runtime) -> Vec<TablePlan> {
    let mut plan: Vec<TablePlan> = runtime
        .manifest()
        .entities
        .iter()
        .map(|e| TablePlan {
            entity: true,
            ..TablePlan::same(SourceDb::App, &e.name)
        })
        .collect();
    plan.extend(
        APP_DB_TABLES
            .iter()
            .map(|t| TablePlan::same(SourceDb::App, t)),
    );
    plan.push(change_log_plan());
    plan.extend(
        AUTH_TABLES
            .iter()
            .map(|t| TablePlan::same(SourceDb::Sessions, t)),
    );
    plan
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Copy `source` into the Postgres database behind `runtime`.
///
/// `runtime` must be open on Postgres with the entity schema applied. The
/// caller holds the boot lock for the whole call (`Runtime::open_postgres`
/// takes it).
pub fn copy(
    runtime: &Runtime,
    source: &SqliteSource,
    mode: CopyMode,
) -> Result<CopyOutcome, CopyError> {
    let pg = runtime
        .pg_data_store_pub()
        .ok_or_else(|| CopyError::Failed("the target runtime is not open on Postgres".into()))?;
    if !source.app_db.exists() {
        return Err(CopyError::Failed(format!(
            "the SQLite database {} does not exist",
            source.app_db.display()
        )));
    }

    create_internal_tables(runtime)?;

    if let Some(done) = read_marker(pg)? {
        return Ok(done);
    }

    let plan = build_plan(runtime);
    let app = open_read_only(&source.app_db)?;
    let sessions = if source.sessions_db.exists() {
        Some(open_read_only(&source.sessions_db)?)
    } else {
        None
    };
    let source_conn = |db: SourceDb| -> Option<&Connection> {
        match db {
            SourceDb::App => Some(&app),
            SourceDb::Sessions => sessions.as_ref(),
        }
    };

    let mut target_tables: Vec<String> = plan.iter().map(|t| t.target.clone()).collect();
    target_tables.extend(JOB_WORKFLOW_TABLES.iter().map(|t| t.to_string()));
    let not_empty = non_empty_tables(pg, &target_tables)?;
    if !not_empty.is_empty() {
        return Err(CopyError::NotEmpty { tables: not_empty });
    }

    let columns = target_columns(pg)?;
    let mut shapes = Vec::with_capacity(plan.len());
    for table in &plan {
        let Some(conn) = source_conn(table.db) else {
            continue;
        };
        if !sqlite_table_exists(conn, &table.source)? {
            continue;
        }
        let target = columns.get(&table.target).ok_or_else(|| {
            CopyError::Failed(format!(
                "the Postgres table {} does not exist after schema setup",
                table.target
            ))
        })?;
        shapes.push(shape_table(conn, table, target)?);
    }

    let skipped_columns: Vec<SkippedColumn> = shapes
        .iter()
        .flat_map(|shape| shape.skipped_columns.iter().cloned())
        .collect();
    let mut skipped = Vec::new();
    for (db, conn) in [
        (SourceDb::App, Some(&app)),
        (SourceDb::Sessions, sessions.as_ref()),
    ] {
        let Some(conn) = conn else { continue };
        for (table, rows) in unplanned_tables(conn, db, &plan)? {
            if table.starts_with("_pylon") || table.starts_with("pylon_") {
                return Err(CopyError::Failed(format!(
                    "the SQLite table {table} ({rows} rows) is internal Pylon state this copy \
                     does not know; refusing rather than leave it behind"
                )));
            }
            skipped.push(TableCount { table, rows });
        }
    }

    // Check every value before writing any.
    let mut problems = Vec::new();
    let mut checked = Vec::with_capacity(shapes.len());
    for shape in &shapes {
        let conn = source_conn(shape.plan.db).expect("shaped tables have a source");
        let rows = scan_table(conn, shape, Some(&mut problems), |_| Ok(()))?;
        checked.push(TableCount {
            table: shape.plan.target.clone(),
            rows,
        });
    }
    if !problems.is_empty() {
        let total = problems.len();
        problems.truncate(MAX_LISTED_PROBLEMS);
        return Err(CopyError::Problems {
            total,
            listed: problems,
        });
    }
    if mode == CopyMode::Check {
        return Ok(CopyOutcome::Checked {
            tables: checked,
            skipped,
            skipped_columns,
        });
    }

    let result = write_all(
        runtime,
        pg,
        source,
        &app,
        &source_conn,
        &shapes,
        skipped,
        skipped_columns,
    );
    if result.is_err() {
        if let Err(e) = empty_tables(pg, &target_tables) {
            tracing::error!("[db-copy] could not empty the partly copied tables: {e}");
        }
    }
    result.map(CopyOutcome::Copied)
}

fn write_all<'a>(
    runtime: &Runtime,
    pg: &pylon_storage::pg_datastore::PostgresDataStore,
    source: &SqliteSource,
    app: &Connection,
    source_conn: &'a dyn Fn(SourceDb) -> Option<&'a Connection>,
    shapes: &[TableShape],
    skipped: Vec<TableCount>,
    skipped_columns: Vec<SkippedColumn>,
) -> Result<CopyReport, CopyError> {
    let jobs = copy_jobs(pg, &source.jobs_db)?;
    let workflows = copy_workflows(pg, &source.workflows_db)?;
    let change_seq = sqlite_change_seq(app)?;
    let manifest = runtime.manifest();

    let tables = pg.with_transaction_raw(|tx| -> Result<Vec<TableCount>, CopyError> {
        let mut counts = Vec::with_capacity(shapes.len());
        for shape in shapes {
            let conn = source_conn(shape.plan.db).expect("shaped tables have a source");
            let read = scan_table(conn, shape, None, |rows| insert_rows(tx, shape, rows))?;
            let target = &shape.plan.target;
            let landed: i64 = tx
                .query_one(&format!("SELECT count(*) FROM {}", quote(target)), &[])?
                .get(0);
            if landed as u64 != read {
                return Err(CopyError::Failed(format!(
                    "{target}: read {read} row(s) from SQLite but {landed} landed in Postgres"
                )));
            }
            counts.push(TableCount {
                table: target.clone(),
                rows: read,
            });
        }
        for entity in &manifest.entities {
            fill_search_index(tx, manifest, &entity.name)?;
        }
        if change_seq > 0 {
            let seq = i64::try_from(change_seq)
                .map_err(|_| failed("change sequence", "value does not fit BIGINT"))?;
            tx.execute("SELECT setval('pylon_change_seq', $1, true)", &[&seq])?;
        }
        // Copied transfers keep their numbers; new ones continue after them.
        tx.execute(
            "SELECT setval('_pylon_shard_transfers_seq', \
                    COALESCE((SELECT max(seq) FROM _pylon_shard_transfers), 0) + 1, false)",
            &[],
        )?;
        let counts_json = serde_json::to_value(&counts).map_err(|e| failed("counts", e))?;
        let source_label = source.app_db.display().to_string();
        tx.execute(
            &format!(
                "INSERT INTO {MARKER_TABLE} (id, source, copied_at, counts) \
                 VALUES (1, $1, now(), $2)"
            ),
            &[&source_label, &counts_json],
        )?;
        Ok(counts)
    })?;

    Ok(CopyReport {
        tables,
        jobs,
        workflows,
        change_seq,
        skipped,
        skipped_columns,
    })
}

// ---------------------------------------------------------------------------
// Schema and state on the Postgres side
// ---------------------------------------------------------------------------

/// Create every internal table the server creates at boot, with the same
/// code, plus the marker table.
fn create_internal_tables(runtime: &Runtime) -> Result<(), CopyError> {
    let pg = runtime.pg_data_store_pub().expect("checked by the caller");
    runtime
        .bootstrap_global_change_seq()
        .map_err(|e| failed("change sequence", e.message))?;
    runtime
        .bootstrap_pg_change_log()
        .map_err(|e| failed("change log", e.message))?;
    crate::server::create_pg_auth_tables(pg.shared_pool()).map_err(|e| failed("auth tables", e))?;
    let owner = "db-copy".to_string();
    crate::pg_job_store::PgJobStore::open(pg.shared_pool(), owner.clone())
        .map_err(|e| failed("job table", e))?;
    crate::pg_workflow_store::PgWorkflowStore::open(pg.shared_pool(), owner)
        .map_err(|e| failed("workflow tables", e))?;
    // Schema only: opening the directory would also write a new cluster key,
    // and the copy brings the SQLite one.
    crate::shard_cluster::create_pg_schema(&pg.shared_pool())
        .map_err(|e| failed("shard tables", e))?;
    pg.with_client(|c| -> Result<(), CopyError> {
        c.batch_execute(&format!(
            "CREATE TABLE IF NOT EXISTS {MARKER_TABLE} (\
                 id INTEGER PRIMARY KEY CHECK (id = 1),\
                 source TEXT NOT NULL,\
                 copied_at TIMESTAMPTZ NOT NULL,\
                 counts JSONB NOT NULL\
             )"
        ))?;
        Ok(())
    })
}

/// Record a failed copy in [`FAILURE_TABLE`].
pub fn record_failure(runtime: &Runtime, error: &CopyError) -> Result<(), CopyError> {
    let pg = runtime
        .pg_data_store_pub()
        .ok_or_else(|| CopyError::Failed("the target runtime is not open on Postgres".into()))?;
    let message = error.to_string();
    pg.with_client(|c| -> Result<(), CopyError> {
        c.batch_execute(&format!(
            "CREATE TABLE IF NOT EXISTS {FAILURE_TABLE} (\
                 failed_at TIMESTAMPTZ NOT NULL DEFAULT now(),\
                 message TEXT NOT NULL\
             )"
        ))?;
        c.execute(
            &format!("INSERT INTO {FAILURE_TABLE} (message) VALUES ($1)"),
            &[&message],
        )?;
        Ok(())
    })
}

fn read_marker(
    pg: &pylon_storage::pg_datastore::PostgresDataStore,
) -> Result<Option<CopyOutcome>, CopyError> {
    pg.with_client(|c| -> Result<Option<CopyOutcome>, CopyError> {
        let row = c.query_opt(
            &format!("SELECT source, copied_at::text FROM {MARKER_TABLE} WHERE id = 1"),
            &[],
        )?;
        Ok(row.map(|r| CopyOutcome::AlreadyCopied {
            source: r.get(0),
            copied_at: r.get(1),
        }))
    })
}

/// Whether a finished copy is recorded in this database. A server booting
/// with `PYLON_IMPORT_FROM_SQLITE` uses this to skip the copy on later boots.
pub fn is_copied(runtime: &Runtime) -> Result<bool, CopyError> {
    let Some(pg) = runtime.pg_data_store_pub() else {
        return Ok(false);
    };
    pg.with_client(|c| -> Result<bool, CopyError> {
        let exists: bool = c
            .query_one("SELECT to_regclass($1) IS NOT NULL", &[&MARKER_TABLE])?
            .get(0);
        if !exists {
            return Ok(false);
        }
        Ok(
            c.query_opt(&format!("SELECT 1 FROM {MARKER_TABLE} WHERE id = 1"), &[])?
                .is_some(),
        )
    })
}

fn non_empty_tables(
    pg: &pylon_storage::pg_datastore::PostgresDataStore,
    tables: &[String],
) -> Result<Vec<String>, CopyError> {
    pg.with_client(|c| -> Result<Vec<String>, CopyError> {
        let mut found = Vec::new();
        for table in tables {
            let exists: bool = c
                .query_one("SELECT to_regclass($1) IS NOT NULL", &[&quote(table)])?
                .get(0);
            if !exists {
                continue;
            }
            let has_rows: bool = c
                .query_one(
                    &format!("SELECT EXISTS (SELECT 1 FROM {})", quote(table)),
                    &[],
                )?
                .get(0);
            if has_rows {
                found.push(table.clone());
            }
        }
        Ok(found)
    })
}

fn empty_tables(
    pg: &pylon_storage::pg_datastore::PostgresDataStore,
    tables: &[String],
) -> Result<(), CopyError> {
    pg.with_client(|c| -> Result<(), CopyError> {
        for table in tables {
            let exists: bool = c
                .query_one("SELECT to_regclass($1) IS NOT NULL", &[&quote(table)])?
                .get(0);
            if exists {
                c.batch_execute(&format!("TRUNCATE {} CASCADE", quote(table)))?;
            }
        }
        Ok(())
    })
}

/// A Postgres column: its type and whether it accepts NULL.
#[derive(Debug, Clone)]
struct TargetColumn {
    /// `None` for a type the copy does not convert to. A copied column of
    /// that type is an error; other tables may have one.
    ty: Option<Type>,
    udt: String,
    nullable: bool,
    has_default: bool,
}

/// Every column of every table in the `public` schema.
fn target_columns(
    pg: &pylon_storage::pg_datastore::PostgresDataStore,
) -> Result<HashMap<String, BTreeMap<String, TargetColumn>>, CopyError> {
    pg.with_client(|c| -> Result<_, CopyError> {
        let rows = c.query(
            "SELECT table_name::text, column_name::text, udt_name::text, \
                    is_nullable = 'YES', column_default IS NOT NULL \
             FROM information_schema.columns \
             WHERE table_schema = 'public'",
            &[],
        )?;
        let mut out: HashMap<String, BTreeMap<String, TargetColumn>> = HashMap::new();
        for row in rows {
            let table: String = row.get(0);
            let column: String = row.get(1);
            let udt: String = row.get(2);
            out.entry(table).or_default().insert(
                column,
                TargetColumn {
                    ty: pg_type(&udt),
                    udt,
                    nullable: row.get(3),
                    has_default: row.get(4),
                },
            );
        }
        Ok(out)
    })
}

fn pg_type(udt: &str) -> Option<Type> {
    Some(match udt {
        "bool" => Type::BOOL,
        "int2" => Type::INT2,
        "int4" => Type::INT4,
        "int8" => Type::INT8,
        "float4" => Type::FLOAT4,
        "float8" => Type::FLOAT8,
        "text" => Type::TEXT,
        "varchar" => Type::VARCHAR,
        "bpchar" => Type::BPCHAR,
        "timestamptz" => Type::TIMESTAMPTZ,
        "timestamp" => Type::TIMESTAMP,
        "jsonb" => Type::JSONB,
        "json" => Type::JSON,
        "bytea" => Type::BYTEA,
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// Reading and converting SQLite rows
// ---------------------------------------------------------------------------

fn open_read_only(path: &Path) -> Result<Connection, CopyError> {
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| failed(&format!("open {}", path.display()), e))
}

/// Tables in a SQLite file that the plan does not copy and that are not
/// known to be safe to leave, with their row counts.
fn unplanned_tables(
    conn: &Connection,
    db: SourceDb,
    plan: &[TablePlan],
) -> Result<Vec<(String, u64)>, CopyError> {
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
        .map_err(|e| failed("read sqlite_master", e))?;
    let names = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
        .map_err(|e| failed("read sqlite_master", e))?;
    let mut out = Vec::new();
    for name in names {
        let planned = plan.iter().any(|t| t.db == db && t.source == name);
        let known = name.starts_with("sqlite_")
            || (db == SourceDb::App
                && (APP_DB_NOT_COPIED.contains(&name.as_str()) || is_sqlite_search_table(&name)));
        if planned || known {
            continue;
        }
        let rows: i64 = conn
            .query_row(
                &format!("SELECT count(*) FROM {}", quote(&name)),
                [],
                |row| row.get(0),
            )
            .map_err(|e| failed(&format!("count {name}"), e))?;
        out.push((name, rows as u64));
    }
    Ok(out)
}

fn sqlite_table_exists(conn: &Connection, table: &str) -> Result<bool, CopyError> {
    conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        [table],
        |row| row.get::<_, bool>(0),
    )
    .map_err(|e| failed("read sqlite_master", e))
}

/// A converted value, bound with the exact Postgres type of its column.
#[derive(Debug, Clone)]
enum CopyValue {
    Null,
    Bool(bool),
    I16(i16),
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64),
    Text(String),
    Bytes(Vec<u8>),
    TimestampTz(chrono::DateTime<chrono::Utc>),
    Timestamp(chrono::NaiveDateTime),
    Json(serde_json::Value),
}

impl ToSql for CopyValue {
    fn to_sql(
        &self,
        ty: &Type,
        out: &mut bytes::BytesMut,
    ) -> Result<postgres::types::IsNull, Box<dyn std::error::Error + Sync + Send>> {
        match self {
            CopyValue::Null => Ok(postgres::types::IsNull::Yes),
            CopyValue::Bool(v) => v.to_sql(ty, out),
            CopyValue::I16(v) => v.to_sql(ty, out),
            CopyValue::I32(v) => v.to_sql(ty, out),
            CopyValue::I64(v) => v.to_sql(ty, out),
            CopyValue::F32(v) => v.to_sql(ty, out),
            CopyValue::F64(v) => v.to_sql(ty, out),
            CopyValue::Text(v) => v.to_sql(ty, out),
            CopyValue::Bytes(v) => v.to_sql(ty, out),
            CopyValue::TimestampTz(v) => v.to_sql(ty, out),
            CopyValue::Timestamp(v) => v.to_sql(ty, out),
            CopyValue::Json(v) => v.to_sql(ty, out),
        }
    }

    fn accepts(_ty: &Type) -> bool {
        true
    }

    postgres::types::to_sql_checked!();
}

/// How one SQLite table maps onto its Postgres table.
#[derive(Debug)]
struct TableShape {
    plan: TablePlan,
    /// `(sqlite column, postgres column, postgres type)` in SELECT order.
    pairs: Vec<(String, String, TargetColumn)>,
    key_columns: Vec<String>,
    skipped_columns: Vec<SkippedColumn>,
}

fn shape_table(
    conn: &Connection,
    plan: &TablePlan,
    target: &BTreeMap<String, TargetColumn>,
) -> Result<TableShape, CopyError> {
    let source_columns = sqlite_columns(conn, &plan.source)?;

    // Every SQLite column needs a Postgres column, or a reason to drop it.
    let mut pairs: Vec<(String, String, TargetColumn)> = Vec::new();
    let mut skipped_columns = Vec::new();
    for column in &source_columns {
        if plan.dropped.contains(&column.as_str()) {
            continue;
        }
        let to = plan.target_column(column);
        let Some(tc) = target.get(&to) else {
            if plan.entity {
                let rows_with_value: i64 = conn
                    .query_row(
                        &format!(
                            "SELECT count(*) FROM {} WHERE {} IS NOT NULL",
                            quote(&plan.source),
                            quote(column)
                        ),
                        [],
                        |row| row.get(0),
                    )
                    .map_err(|e| failed(&format!("count {}.{column}", plan.source), e))?;
                skipped_columns.push(SkippedColumn {
                    table: plan.source.clone(),
                    column: column.clone(),
                    rows_with_value: rows_with_value as u64,
                });
                continue;
            }
            return Err(CopyError::Failed(format!(
                "{}.{} has no column in the Postgres table {}; the copy would lose it",
                plan.source, column, plan.target
            )));
        };
        if tc.ty.is_none() {
            return Err(CopyError::Failed(format!(
                "{}.{}: the copy does not handle the Postgres type {}",
                plan.target, to, tc.udt
            )));
        }
        pairs.push((column.clone(), to, tc.clone()));
    }
    // A Postgres column the source lacks must accept NULL or have a default.
    for (name, tc) in target {
        let covered = pairs.iter().any(|(_, to, _)| to == name);
        if !covered && !tc.nullable && !tc.has_default {
            return Err(CopyError::Failed(format!(
                "{}.{} is required in Postgres but {} has no such column",
                plan.target, name, plan.source
            )));
        }
    }
    let key_columns = sqlite_key_columns(conn, &plan.source, &source_columns)?;
    Ok(TableShape {
        plan: plan.clone(),
        pairs,
        key_columns,
        skipped_columns,
    })
}

/// Read every row of a table, convert it, and pass the rows on in chunks
/// sized for one INSERT. With `problems`, a value that does not convert is
/// recorded and the scan goes on; without it, the scan stops there. Returns
/// the number of rows read.
fn scan_table(
    conn: &Connection,
    shape: &TableShape,
    mut problems: Option<&mut Vec<Problem>>,
    mut on_rows: impl FnMut(&[Vec<CopyValue>]) -> Result<(), CopyError>,
) -> Result<u64, CopyError> {
    let plan = &shape.plan;
    if shape.pairs.is_empty() {
        return Ok(0);
    }
    let per_chunk = (MAX_PARAMS_PER_STATEMENT / shape.pairs.len()).clamp(1, MAX_ROWS_PER_STATEMENT);
    let select = format!(
        "SELECT {} FROM {}",
        shape
            .pairs
            .iter()
            .map(|(from, _, _)| quote(from))
            .collect::<Vec<_>>()
            .join(", "),
        quote(&plan.source)
    );
    let read_err = |e: rusqlite::Error| failed(&format!("read {}", plan.source), e);
    let mut stmt = conn.prepare(&select).map_err(read_err)?;
    let mut rows = stmt.query([]).map_err(read_err)?;

    let mut chunk: Vec<Vec<CopyValue>> = Vec::with_capacity(per_chunk);
    let mut read = 0u64;
    while let Some(row) = rows.next().map_err(read_err)? {
        let mut values = Vec::with_capacity(shape.pairs.len());
        for (i, (_, to, tc)) in shape.pairs.iter().enumerate() {
            let raw = row.get_ref(i).map_err(read_err)?;
            match convert(raw, tc) {
                Ok(v) => values.push(v),
                Err(message) => {
                    let problem = Problem {
                        table: plan.source.clone(),
                        column: to.clone(),
                        row: row_key(row, &shape.pairs, &shape.key_columns),
                        message,
                    };
                    match problems.as_deref_mut() {
                        Some(list) => {
                            list.push(problem);
                            values.push(CopyValue::Null);
                        }
                        None => {
                            return Err(CopyError::Problems {
                                total: 1,
                                listed: vec![problem],
                            })
                        }
                    }
                }
            }
        }
        read += 1;
        chunk.push(values);
        if chunk.len() == per_chunk {
            on_rows(&chunk)?;
            chunk.clear();
        }
    }
    if !chunk.is_empty() {
        on_rows(&chunk)?;
    }
    Ok(read)
}

fn sqlite_columns(conn: &Connection, table: &str) -> Result<Vec<String>, CopyError> {
    let mut stmt = conn
        .prepare("SELECT name FROM pragma_table_info(?1) ORDER BY cid")
        .map_err(|e| failed(&format!("columns of {table}"), e))?;
    let names = stmt
        .query_map([table], |row| row.get::<_, String>(0))
        .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
        .map_err(|e| failed(&format!("columns of {table}"), e))?;
    Ok(names)
}

/// The columns that identify a row in problem reports: the primary key, or
/// `id`, or the first column.
fn sqlite_key_columns(
    conn: &Connection,
    table: &str,
    columns: &[String],
) -> Result<Vec<String>, CopyError> {
    let mut stmt = conn
        .prepare("SELECT name FROM pragma_table_info(?1) WHERE pk > 0 ORDER BY pk")
        .map_err(|e| failed(&format!("primary key of {table}"), e))?;
    let pk = stmt
        .query_map([table], |row| row.get::<_, String>(0))
        .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
        .map_err(|e| failed(&format!("primary key of {table}"), e))?;
    if !pk.is_empty() {
        return Ok(pk);
    }
    if columns.iter().any(|c| c == "id") {
        return Ok(vec!["id".into()]);
    }
    Ok(columns.first().cloned().into_iter().collect())
}

fn row_key(
    row: &rusqlite::Row<'_>,
    pairs: &[(String, String, TargetColumn)],
    key_columns: &[String],
) -> String {
    key_columns
        .iter()
        .map(|key| {
            pairs
                .iter()
                .position(|(from, _, _)| from == key)
                .and_then(|i| row.get_ref(i).ok())
                .map(display_value)
                .unwrap_or_else(|| "?".into())
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn display_value(v: ValueRef<'_>) -> String {
    match v {
        ValueRef::Null => "NULL".into(),
        ValueRef::Integer(i) => i.to_string(),
        ValueRef::Real(f) => f.to_string(),
        ValueRef::Text(t) => String::from_utf8_lossy(t).into_owned(),
        ValueRef::Blob(b) => format!("<{} bytes>", b.len()),
    }
}

/// Convert one SQLite value for a Postgres column. SQLite column types are
/// loose (a TEXT column can hold an integer), so the Postgres column type
/// decides, not the SQLite declaration.
fn convert(v: ValueRef<'_>, col: &TargetColumn) -> Result<CopyValue, String> {
    if let ValueRef::Null = v {
        return if col.nullable || col.has_default {
            Ok(CopyValue::Null)
        } else {
            Err("NULL in a column that Postgres requires".into())
        };
    }
    let text = |v: ValueRef<'_>| -> Result<String, String> {
        match v {
            ValueRef::Text(t) => std::str::from_utf8(t)
                .map(str::to_owned)
                .map_err(|_| "text is not valid UTF-8".to_string()),
            ValueRef::Integer(i) => Ok(i.to_string()),
            ValueRef::Real(f) => Ok(f.to_string()),
            ValueRef::Blob(b) => std::str::from_utf8(b)
                .map(str::to_owned)
                .map_err(|_| "binary value in a text column".to_string()),
            ValueRef::Null => unreachable!(),
        }
    };
    let Some(ty) = &col.ty else {
        return Err(format!(
            "the copy does not handle the Postgres type {}",
            col.udt
        ));
    };
    if *ty == Type::BOOL {
        return match v {
            ValueRef::Integer(0) => Ok(CopyValue::Bool(false)),
            ValueRef::Integer(1) => Ok(CopyValue::Bool(true)),
            _ => match text(v)?.as_str() {
                "0" | "false" | "FALSE" => Ok(CopyValue::Bool(false)),
                "1" | "true" | "TRUE" => Ok(CopyValue::Bool(true)),
                other => Err(format!("{other:?} is not a boolean")),
            },
        };
    }
    if *ty == Type::INT2 || *ty == Type::INT4 || *ty == Type::INT8 {
        let n: i64 = match v {
            ValueRef::Integer(i) => i,
            ValueRef::Real(f) if f.fract() == 0.0 && f.abs() < 9.0e15 => f as i64,
            _ => {
                let s = text(v)?;
                s.trim()
                    .parse::<i64>()
                    .map_err(|_| format!("{s:?} is not an integer"))?
            }
        };
        return if *ty == Type::INT2 {
            i16::try_from(n)
                .map(CopyValue::I16)
                .map_err(|_| format!("{n} does not fit SMALLINT"))
        } else if *ty == Type::INT4 {
            i32::try_from(n)
                .map(CopyValue::I32)
                .map_err(|_| format!("{n} does not fit INTEGER"))
        } else {
            Ok(CopyValue::I64(n))
        };
    }
    if *ty == Type::FLOAT4 || *ty == Type::FLOAT8 {
        let f: f64 = match v {
            ValueRef::Integer(i) => i as f64,
            ValueRef::Real(f) => f,
            _ => {
                let s = text(v)?;
                s.trim()
                    .parse::<f64>()
                    .map_err(|_| format!("{s:?} is not a number"))?
            }
        };
        return Ok(if *ty == Type::FLOAT4 {
            CopyValue::F32(f as f32)
        } else {
            CopyValue::F64(f)
        });
    }
    if *ty == Type::TEXT || *ty == Type::VARCHAR || *ty == Type::BPCHAR {
        return text(v).map(CopyValue::Text);
    }
    if *ty == Type::TIMESTAMPTZ || *ty == Type::TIMESTAMP {
        let s = text(v)?;
        let dt = parse_timestamp(&s)?;
        return Ok(if *ty == Type::TIMESTAMPTZ {
            CopyValue::TimestampTz(dt)
        } else {
            CopyValue::Timestamp(dt.naive_utc())
        });
    }
    if *ty == Type::JSONB || *ty == Type::JSON {
        let parsed = match v {
            ValueRef::Blob(b) => serde_json::from_slice(b),
            _ => serde_json::from_str(&text(v)?),
        };
        return parsed
            .map(CopyValue::Json)
            .map_err(|e| format!("not valid JSON: {e}"));
    }
    if *ty == Type::BYTEA {
        return match v {
            ValueRef::Blob(b) => Ok(CopyValue::Bytes(b.to_vec())),
            // A vector field the SQLite schema typed as TEXT holds the
            // embedding as a JSON number array.
            _ => {
                let s = text(v)?;
                let parsed: serde_json::Value =
                    serde_json::from_str(&s).map_err(|_| "binary column holds text".to_string())?;
                let arr = parsed
                    .as_array()
                    .ok_or("binary column holds text that is not a number array")?;
                let floats = arr
                    .iter()
                    .map(|n| n.as_f64().map(|f| f as f32))
                    .collect::<Option<Vec<f32>>>()
                    .ok_or("vector holds a value that is not a number")?;
                Ok(CopyValue::Bytes(pylon_storage::vector::pack_f32(&floats)))
            }
        };
    }
    Err(format!("the copy does not handle the Postgres type {ty}"))
}

/// Parse the timestamp shapes Pylon writes to SQLite: RFC 3339
/// (`2026-10-05T12:00:00.000Z`), SQLite's `CURRENT_TIMESTAMP`
/// (`2026-10-05 12:00:00`, UTC), and the CRDT store's `<epoch seconds>Z`.
fn parse_timestamp(s: &str) -> Result<chrono::DateTime<chrono::Utc>, String> {
    if s.is_empty() {
        return Err("empty string in a timestamp column".into());
    }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&chrono::Utc));
    }
    if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.f") {
        return Ok(naive.and_utc());
    }
    if let Some(secs) = s.strip_suffix('Z').and_then(|d| d.parse::<i64>().ok()) {
        if let Some(dt) = chrono::DateTime::from_timestamp(secs, 0) {
            return Ok(dt);
        }
    }
    Err(format!("{s:?} is not a timestamp Postgres accepts"))
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

fn insert_rows(
    tx: &mut postgres::Transaction<'_>,
    shape: &TableShape,
    rows: &[Vec<CopyValue>],
) -> Result<(), CopyError> {
    let target = &shape.plan.target;
    let column_list = shape
        .pairs
        .iter()
        .map(|(_, to, _)| quote(to))
        .collect::<Vec<_>>()
        .join(", ");
    let mut sql = format!("INSERT INTO {} ({column_list}) VALUES ", quote(target));
    let mut params: Vec<&(dyn ToSql + Sync)> = Vec::with_capacity(rows.len() * shape.pairs.len());
    for (r, row) in rows.iter().enumerate() {
        if r > 0 {
            sql.push_str(", ");
        }
        sql.push('(');
        for (c, value) in row.iter().enumerate() {
            if c > 0 {
                sql.push_str(", ");
            }
            sql.push_str(&format!("${}", params.len() + 1));
            params.push(value);
        }
        sql.push(')');
    }
    tx.execute(&sql, &params)
        .map_err(|e| failed(&format!("insert into {target}"), pg_detail(e)))?;
    Ok(())
}

/// Write the search rows for every copied row of `entity`. Postgres fills
/// them only when a row is written through the runtime, which a bulk copy
/// does not do.
fn fill_search_index(
    tx: &mut postgres::Transaction<'_>,
    manifest: &pylon_kernel::AppManifest,
    entity: &str,
) -> Result<(), CopyError> {
    let Some(config) = pylon_storage::pg_tx_store::search_config_for(manifest, entity) else {
        return Ok(());
    };
    let rows = tx.query(&format!("SELECT * FROM {}", quote(entity)), &[])?;
    for row in rows {
        let data = pylon_storage::postgres::row_to_json_pub(&row);
        let Some(id) = data.get("id").and_then(|v| v.as_str()).map(str::to_owned) else {
            continue;
        };
        pylon_storage::pg_search::apply_insert(tx, entity, &id, &data, &config)
            .map_err(|e| failed(&format!("search index for {entity}"), e.message))?;
    }
    Ok(())
}

/// Copy the job queue. A job that was running when the SQLite server stopped
/// goes back to pending: on Postgres a running job is reclaimed only after
/// its lease expires, and a copied job has no lease.
fn copy_jobs(
    pg: &pylon_storage::pg_datastore::PostgresDataStore,
    path: &Path,
) -> Result<u64, CopyError> {
    if !path.exists() {
        return Ok(0);
    }
    let source = crate::job_store::JobStore::open(&path.to_string_lossy())
        .map_err(|e| failed("open the SQLite job store", e))?;
    let target = crate::pg_job_store::PgJobStore::open(pg.shared_pool(), "db-copy".into())
        .map_err(|e| failed("open the Postgres job store", e))?;
    let jobs = source.load_all().map_err(|e| failed("read jobs", e))?;
    for mut job in jobs.iter().cloned() {
        if job.status == JobStatus::Running {
            job.status = JobStatus::Pending;
            job.started_at = None;
        }
        target
            .enqueue(&job)
            .map_err(|e| failed(&format!("copy job {}", job.id), e))?;
    }
    Ok(jobs.len() as u64)
}

/// Copy workflow runs with their steps and unread events.
fn copy_workflows(
    pg: &pylon_storage::pg_datastore::PostgresDataStore,
    path: &Path,
) -> Result<u64, CopyError> {
    if !path.exists() {
        return Ok(0);
    }
    let source = crate::workflow_store::WorkflowStore::open(&path.to_string_lossy())
        .map_err(|e| failed("open the SQLite workflow store", e))?;
    let target =
        crate::pg_workflow_store::PgWorkflowStore::open(pg.shared_pool(), "db-copy".into())
            .map_err(|e| failed("open the Postgres workflow store", e))?;
    let workflows = source.load_all().map_err(|e| failed("read workflows", e))?;
    for wf in &workflows {
        let mut run = wf.clone();
        // The SQLite store keeps an event until the run consumes it, so every
        // stored event is unread. Postgres numbers events itself.
        let events = std::mem::take(&mut run.pending_events);
        run.consumed_events.clear();
        target
            .save(&run)
            .map_err(|e| failed(&format!("copy workflow {}", run.id), e))?;
        target
            .restore_events(&run.id, &events)
            .map_err(|e| failed(&format!("copy the events of workflow {}", run.id), e))?;
    }
    Ok(workflows.len() as u64)
}

/// The highest change-log sequence the SQLite server reserved. The Postgres
/// sequence must start above it: a client's sync cursor is a sequence
/// number, and a lower Postgres sequence would hand that client unrelated
/// changes as if they came after its cursor.
fn sqlite_change_seq(app: &Connection) -> Result<u64, CopyError> {
    let reserved: i64 = if sqlite_table_exists(app, "_pylon_change_seq")? {
        app.query_row(
            "SELECT COALESCE(MAX(value), 0) FROM _pylon_change_seq",
            [],
            |row| row.get(0),
        )
        .map_err(|e| failed("read _pylon_change_seq", e))?
    } else {
        0
    };
    let logged: i64 = if sqlite_table_exists(app, "_pylon_change_log")? {
        app.query_row(
            "SELECT COALESCE(MAX(seq), 0) FROM _pylon_change_log",
            [],
            |row| row.get(0),
        )
        .map_err(|e| failed("read _pylon_change_log", e))?
    } else {
        0
    };
    Ok(reserved.max(logged).max(0) as u64)
}

fn quote(name: &str) -> String {
    pylon_storage::postgres::quote_ident_pub(name)
}

fn pg_detail(e: postgres::Error) -> String {
    pylon_storage::pg_tx_store::pg_err_to_data(e).message
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(ty: Type, nullable: bool) -> TargetColumn {
        TargetColumn {
            udt: ty.name().to_string(),
            ty: Some(ty),
            nullable,
            has_default: false,
        }
    }

    #[test]
    fn booleans_from_integers_and_text() {
        let c = col(Type::BOOL, false);
        assert!(matches!(
            convert(ValueRef::Integer(1), &c),
            Ok(CopyValue::Bool(true))
        ));
        assert!(matches!(
            convert(ValueRef::Integer(0), &c),
            Ok(CopyValue::Bool(false))
        ));
        assert!(matches!(
            convert(ValueRef::Text(b"true"), &c),
            Ok(CopyValue::Bool(true))
        ));
        assert!(convert(ValueRef::Integer(2), &c).is_err());
    }

    #[test]
    fn timestamps_in_every_shape_pylon_writes() {
        for s in [
            "2026-10-05T12:00:00.000Z",
            "2026-10-05T12:00:00Z",
            "2026-10-05 12:00:00",
            "1791201600Z",
        ] {
            assert!(parse_timestamp(s).is_ok(), "{s}");
        }
        assert_eq!(
            parse_timestamp("1791201600Z").unwrap(),
            parse_timestamp("2026-10-05T12:00:00Z").unwrap()
        );
        assert!(parse_timestamp("").is_err());
        assert!(parse_timestamp("yesterday").is_err());
    }

    #[test]
    fn null_in_a_required_column_is_a_problem() {
        assert!(convert(ValueRef::Null, &col(Type::TEXT, false)).is_err());
        assert!(matches!(
            convert(ValueRef::Null, &col(Type::TEXT, true)),
            Ok(CopyValue::Null)
        ));
    }

    #[test]
    fn integers_keep_their_range() {
        assert!(convert(ValueRef::Integer(70_000), &col(Type::INT2, false)).is_err());
        assert!(matches!(
            convert(ValueRef::Integer(7), &col(Type::INT2, false)),
            Ok(CopyValue::I16(7))
        ));
        assert!(matches!(
            convert(ValueRef::Text(b"42"), &col(Type::INT8, false)),
            Ok(CopyValue::I64(42))
        ));
    }

    #[test]
    fn json_from_text_and_blob() {
        let c = col(Type::JSONB, true);
        assert!(matches!(
            convert(ValueRef::Text(br#"{"a":1}"#), &c),
            Ok(CopyValue::Json(_))
        ));
        assert!(matches!(
            convert(ValueRef::Blob(br#"[1,2]"#), &c),
            Ok(CopyValue::Json(_))
        ));
        assert!(convert(ValueRef::Text(b"{nope"), &c).is_err());
    }

    #[test]
    fn vectors_from_blob_or_json_text() {
        let c = col(Type::BYTEA, true);
        let packed = pylon_storage::vector::pack_f32(&[1.0, 2.0]);
        match convert(ValueRef::Text(b"[1, 2]"), &c) {
            Ok(CopyValue::Bytes(b)) => assert_eq!(b, packed),
            other => panic!("{other:?}"),
        }
        match convert(ValueRef::Blob(&packed), &c) {
            Ok(CopyValue::Bytes(b)) => assert_eq!(b, packed),
            other => panic!("{other:?}"),
        }
    }
}
