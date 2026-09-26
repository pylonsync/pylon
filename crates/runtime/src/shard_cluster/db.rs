//! The shard directory's database: Postgres (several machines) or the app's
//! SQLite file (one machine).
//!
//! Statements are written once per directory operation and built from the
//! [`Dialect`]'s fragments: placeholders, the database clock, row locks.
//! Values and rows go through [`Val`] and [`Row`], so each operation reads
//! the same on both databases.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use pylon_storage::pg_datastore::PgPool;

/// Which SQL a statement is built for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Dialect {
    Pg,
    Sqlite,
}

impl Dialect {
    /// Placeholders 1..=N: `$n` on Postgres, `?n` on SQLite. (SQLite reads
    /// `$n` as a named parameter numbered by first appearance, not by n.)
    pub(crate) fn params<const N: usize>(self) -> [String; N] {
        std::array::from_fn(|i| match self {
            Dialect::Pg => format!("${}", i + 1),
            Dialect::Sqlite => format!("?{}", i + 1),
        })
    }

    /// The database clock, in milliseconds since the Unix epoch.
    pub(crate) fn now_ms(self) -> &'static str {
        match self {
            Dialect::Pg => "(extract(epoch from clock_timestamp()) * 1000)::bigint",
            Dialect::Sqlite => "CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER)",
        }
    }

    /// A row lock that a take-over waits for. SQLite takes the database's
    /// write lock at the start of each directory transaction instead.
    pub(crate) fn for_share(self) -> &'static str {
        match self {
            Dialect::Pg => " FOR SHARE",
            Dialect::Sqlite => "",
        }
    }

    /// See [`Dialect::for_share`].
    pub(crate) fn for_update(self) -> &'static str {
        match self {
            Dialect::Pg => " FOR UPDATE",
            Dialect::Sqlite => "",
        }
    }
}

/// A statement parameter.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Val<'a> {
    I64(i64),
    I32(i32),
    Text(&'a str),
    OptText(Option<&'a str>),
    Bytes(&'a [u8]),
    Bool(bool),
    Json(&'a serde_json::Value),
}

impl postgres::types::ToSql for Val<'_> {
    fn to_sql(
        &self,
        ty: &postgres::types::Type,
        out: &mut bytes::BytesMut,
    ) -> Result<postgres::types::IsNull, Box<dyn std::error::Error + Sync + Send>> {
        match self {
            Val::I64(v) => v.to_sql(ty, out),
            Val::I32(v) => v.to_sql(ty, out),
            Val::Text(v) => v.to_sql(ty, out),
            Val::OptText(v) => v.to_sql(ty, out),
            Val::Bytes(v) => v.to_sql(ty, out),
            Val::Bool(v) => v.to_sql(ty, out),
            Val::Json(v) => v.to_sql(ty, out),
        }
    }

    fn accepts(_: &postgres::types::Type) -> bool {
        // Checked per value in `to_sql_checked`.
        true
    }

    fn to_sql_checked(
        &self,
        ty: &postgres::types::Type,
        out: &mut bytes::BytesMut,
    ) -> Result<postgres::types::IsNull, Box<dyn std::error::Error + Sync + Send>> {
        match self {
            Val::I64(v) => v.to_sql_checked(ty, out),
            Val::I32(v) => v.to_sql_checked(ty, out),
            Val::Text(v) => v.to_sql_checked(ty, out),
            Val::OptText(v) => v.to_sql_checked(ty, out),
            Val::Bytes(v) => v.to_sql_checked(ty, out),
            Val::Bool(v) => v.to_sql_checked(ty, out),
            Val::Json(v) => v.to_sql_checked(ty, out),
        }
    }
}

impl rusqlite::ToSql for Val<'_> {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        use rusqlite::types::{ToSqlOutput, Value, ValueRef};
        Ok(match self {
            Val::I64(v) => ToSqlOutput::Owned(Value::Integer(*v)),
            Val::I32(v) => ToSqlOutput::Owned(Value::Integer(*v as i64)),
            Val::Text(v) => ToSqlOutput::Borrowed(ValueRef::Text(v.as_bytes())),
            Val::OptText(Some(v)) => ToSqlOutput::Borrowed(ValueRef::Text(v.as_bytes())),
            Val::OptText(None) => ToSqlOutput::Owned(Value::Null),
            Val::Bytes(v) => ToSqlOutput::Borrowed(ValueRef::Blob(v)),
            Val::Bool(v) => ToSqlOutput::Owned(Value::Integer(*v as i64)),
            Val::Json(v) => ToSqlOutput::Owned(Value::Text(v.to_string())),
        })
    }
}

/// One column of a result row.
#[derive(Debug, Clone, PartialEq)]
enum Cell {
    Null,
    Int(i64),
    Text(String),
    Bytes(Vec<u8>),
    Bool(bool),
    Json(serde_json::Value),
}

/// A result row. The getters panic on a column of another type, as
/// `postgres::Row::get` does: the directory's own schema decides the types.
#[derive(Debug, Clone)]
pub(crate) struct Row(Vec<Cell>);

impl Row {
    pub(crate) fn i64(&self, i: usize) -> i64 {
        match &self.0[i] {
            Cell::Int(v) => *v,
            other => panic!("column {i}: expected an integer, got {other:?}"),
        }
    }

    pub(crate) fn string(&self, i: usize) -> String {
        match &self.0[i] {
            Cell::Text(v) => v.clone(),
            other => panic!("column {i}: expected text, got {other:?}"),
        }
    }

    pub(crate) fn opt_string(&self, i: usize) -> Option<String> {
        match &self.0[i] {
            Cell::Null => None,
            Cell::Text(v) => Some(v.clone()),
            other => panic!("column {i}: expected text or NULL, got {other:?}"),
        }
    }

    pub(crate) fn bytes(&self, i: usize) -> Vec<u8> {
        match &self.0[i] {
            Cell::Bytes(v) => v.clone(),
            other => panic!("column {i}: expected bytes, got {other:?}"),
        }
    }

    pub(crate) fn opt_bytes(&self, i: usize) -> Option<Vec<u8>> {
        match &self.0[i] {
            Cell::Null => None,
            Cell::Bytes(v) => Some(v.clone()),
            other => panic!("column {i}: expected bytes or NULL, got {other:?}"),
        }
    }

    pub(crate) fn bool(&self, i: usize) -> bool {
        match &self.0[i] {
            Cell::Bool(v) => *v,
            // SQLite stores a boolean as 0 or 1.
            Cell::Int(v) => *v != 0,
            other => panic!("column {i}: expected a boolean, got {other:?}"),
        }
    }

    pub(crate) fn json(&self, i: usize) -> serde_json::Value {
        match &self.0[i] {
            Cell::Json(v) => v.clone(),
            // SQLite stores JSON as text the directory wrote.
            Cell::Text(v) => serde_json::from_str(v)
                .unwrap_or_else(|e| panic!("column {i}: stored JSON does not parse: {e}")),
            other => panic!("column {i}: expected JSON, got {other:?}"),
        }
    }

    fn from_pg(row: &postgres::Row) -> Self {
        use postgres::types::Type;
        let cells = row
            .columns()
            .iter()
            .enumerate()
            .map(|(i, col)| {
                let ty = col.type_();
                if *ty == Type::INT8 {
                    row.get::<_, Option<i64>>(i).map_or(Cell::Null, Cell::Int)
                } else if *ty == Type::INT4 {
                    row.get::<_, Option<i32>>(i)
                        .map_or(Cell::Null, |v| Cell::Int(v as i64))
                } else if *ty == Type::INT2 {
                    row.get::<_, Option<i16>>(i)
                        .map_or(Cell::Null, |v| Cell::Int(v as i64))
                } else if *ty == Type::BOOL {
                    row.get::<_, Option<bool>>(i).map_or(Cell::Null, Cell::Bool)
                } else if *ty == Type::BYTEA {
                    row.get::<_, Option<Vec<u8>>>(i)
                        .map_or(Cell::Null, Cell::Bytes)
                } else if *ty == Type::JSONB || *ty == Type::JSON {
                    row.get::<_, Option<serde_json::Value>>(i)
                        .map_or(Cell::Null, Cell::Json)
                } else {
                    row.get::<_, Option<String>>(i)
                        .map_or(Cell::Null, Cell::Text)
                }
            })
            .collect();
        Row(cells)
    }

    fn from_sqlite(row: &rusqlite::Row<'_>, columns: usize) -> rusqlite::Result<Self> {
        use rusqlite::types::ValueRef;
        let mut cells = Vec::with_capacity(columns);
        for i in 0..columns {
            cells.push(match row.get_ref(i)? {
                ValueRef::Null => Cell::Null,
                ValueRef::Integer(v) => Cell::Int(v),
                // No directory column is REAL; an integer expression that
                // SQLite computed in floating point is truncated.
                ValueRef::Real(v) => Cell::Int(v as i64),
                ValueRef::Text(v) => Cell::Text(String::from_utf8_lossy(v).into_owned()),
                ValueRef::Blob(v) => Cell::Bytes(v.to_vec()),
            });
        }
        Ok(Row(cells))
    }
}

/// An error from a statement.
#[derive(Debug)]
pub(crate) enum DbError {
    Pg(postgres::Error),
    Sqlite(rusqlite::Error),
}

impl From<postgres::Error> for DbError {
    fn from(e: postgres::Error) -> Self {
        DbError::Pg(e)
    }
}

impl From<rusqlite::Error> for DbError {
    fn from(e: rusqlite::Error) -> Self {
        DbError::Sqlite(e)
    }
}

impl std::fmt::Display for DbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DbError::Pg(e) => write!(f, "{e}"),
            DbError::Sqlite(e) => write!(f, "{e}"),
        }
    }
}

/// Where statements run: a Postgres transaction or client, or a SQLite
/// connection (inside `BEGIN IMMEDIATE` for a transaction).
pub(crate) enum Tx<'a, 'b> {
    PgTransaction(&'a mut postgres::Transaction<'b>),
    PgClient(&'a mut postgres::Client),
    Sqlite(&'a rusqlite::Connection),
}

impl Tx<'_, '_> {
    pub(crate) fn dialect(&self) -> Dialect {
        match self {
            Tx::PgTransaction(_) | Tx::PgClient(_) => Dialect::Pg,
            Tx::Sqlite(_) => Dialect::Sqlite,
        }
    }

    /// Run a statement; the number of rows it changed.
    pub(crate) fn execute(&mut self, sql: &str, params: &[Val<'_>]) -> Result<u64, DbError> {
        match self {
            Tx::PgTransaction(t) => Ok(t.execute(sql, &pg_params(params))?),
            Tx::PgClient(c) => Ok(c.execute(sql, &pg_params(params))?),
            Tx::Sqlite(c) => Ok(c.execute(sql, rusqlite::params_from_iter(params.iter()))? as u64),
        }
    }

    pub(crate) fn query(&mut self, sql: &str, params: &[Val<'_>]) -> Result<Vec<Row>, DbError> {
        match self {
            Tx::PgTransaction(t) => Ok(t
                .query(sql, &pg_params(params))?
                .iter()
                .map(Row::from_pg)
                .collect()),
            Tx::PgClient(c) => Ok(c
                .query(sql, &pg_params(params))?
                .iter()
                .map(Row::from_pg)
                .collect()),
            Tx::Sqlite(c) => {
                let mut stmt = c.prepare(sql)?;
                let columns = stmt.column_count();
                let rows = stmt
                    .query_map(rusqlite::params_from_iter(params.iter()), |r| {
                        Row::from_sqlite(r, columns)
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            }
        }
    }

    pub(crate) fn query_opt(
        &mut self,
        sql: &str,
        params: &[Val<'_>],
    ) -> Result<Option<Row>, DbError> {
        Ok(self.query(sql, params)?.into_iter().next())
    }

    /// A statement that returns exactly one row.
    pub(crate) fn query_one(&mut self, sql: &str, params: &[Val<'_>]) -> Result<Row, DbError> {
        match self {
            Tx::PgTransaction(t) => Ok(Row::from_pg(&t.query_one(sql, &pg_params(params))?)),
            Tx::PgClient(c) => Ok(Row::from_pg(&c.query_one(sql, &pg_params(params))?)),
            Tx::Sqlite(c) => {
                let mut stmt = c.prepare(sql)?;
                let columns = stmt.column_count();
                Ok(
                    stmt.query_row(rusqlite::params_from_iter(params.iter()), |r| {
                        Row::from_sqlite(r, columns)
                    })?,
                )
            }
        }
    }

    /// Statements that only Postgres runs (session settings); nothing on
    /// SQLite.
    pub(crate) fn pg_only(&mut self, sql: &str) -> Result<(), DbError> {
        match self {
            Tx::PgTransaction(t) => Ok(t.batch_execute(sql)?),
            Tx::PgClient(c) => Ok(c.batch_execute(sql)?),
            Tx::Sqlite(_) => Ok(()),
        }
    }
}

fn pg_params<'a>(params: &'a [Val<'a>]) -> Vec<&'a (dyn postgres::types::ToSql + Sync)> {
    params
        .iter()
        .map(|p| p as &(dyn postgres::types::ToSql + Sync))
        .collect()
}

/// A Postgres operation after a dead connection: replayed once (`Replay`)
/// or not (`Once`, for a claim or a move whose first run may have
/// committed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Retry {
    Replay,
    Once,
}

/// The app's SQLite file, as the directory uses it: its own connections
/// (so a directory read inside a mutation, which holds the runtime's write
/// connection, never waits on it), and an exclusive lock on the file's
/// shard directory for the life of the process.
pub(crate) struct SqliteDb {
    write: Mutex<rusqlite::Connection>,
    read: Mutex<rusqlite::Connection>,
    /// Held open for the lock; released when the process exits, whatever
    /// way it exits.
    _lock: std::fs::File,
}

/// How long a directory write waits for the database's write lock (another
/// connection's transaction, such as a long mutation).
pub(crate) const SQLITE_BUSY_WAIT: Duration = Duration::from_secs(10);

/// How long opening a SQLite directory waits for the file's directory lock.
pub(crate) const SQLITE_LOCK_WAIT: Duration = Duration::from_secs(10);

impl SqliteDb {
    /// Open the directory's connections to the SQLite file at `path`, after
    /// taking the exclusive lock at `<path>.shards.lock`. A process that is
    /// still exiting (a restart) gets `wait` to let go of it; after that,
    /// an error: one process runs a file's shards.
    pub(crate) fn open_waiting(path: &str, wait: Duration) -> Result<Self, String> {
        // One lock file for the database file, whatever path names it (a
        // relative path, or a symbolic link). A file not created yet is
        // named by its resolved directory.
        let resolve = |p: &std::path::Path| {
            std::fs::canonicalize(p).map_err(|e| format!("could not resolve {path}: {e}"))
        };
        let given = std::path::Path::new(path);
        let real = if given.exists() {
            resolve(given)?
        } else {
            let dir = match given.parent() {
                Some(d) if !d.as_os_str().is_empty() => d,
                _ => std::path::Path::new("."),
            };
            let name = given
                .file_name()
                .ok_or_else(|| format!("{path} names no file"))?;
            resolve(dir)?.join(name)
        };
        let lock_path = format!("{}.shards.lock", real.display());
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .map_err(|e| format!("could not open {lock_path}: {e}"))?;
        let deadline = std::time::Instant::now() + wait;
        loop {
            match lock.try_lock() {
                Ok(()) => break,
                Err(std::fs::TryLockError::WouldBlock) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(std::fs::TryLockError::WouldBlock) => {
                    return Err(format!(
                        "another process runs the shards of {path} (it holds {lock_path}); \
                         one process runs a SQLite app's shards. Stop it, move this app to \
                         Postgres to run several machines, or set PYLON_SHARD_DIRECTORY=off \
                         to run shards with no directory"
                    ))
                }
                Err(std::fs::TryLockError::Error(e)) => {
                    return Err(format!("could not lock {lock_path}: {e}"))
                }
            }
        }
        let open = |flags: rusqlite::OpenFlags| -> Result<rusqlite::Connection, String> {
            let conn = rusqlite::Connection::open_with_flags(path, flags)
                .map_err(|e| format!("could not open {path} for the shard directory: {e}"))?;
            crate::tune_runtime_connection(&conn, false)
                .map_err(|e| format!("could not set up {path} for the shard directory: {e}"))?;
            conn.busy_timeout(SQLITE_BUSY_WAIT)
                .map_err(|e| format!("could not set up {path} for the shard directory: {e}"))?;
            Ok(conn)
        };
        let write = open(
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
                | rusqlite::OpenFlags::SQLITE_OPEN_CREATE
                | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        let read = open(
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        Ok(Self {
            write: Mutex::new(write),
            read: Mutex::new(read),
            _lock: lock,
        })
    }
}

/// The directory's database.
pub(crate) enum Db {
    Pg(Arc<PgPool>),
    Sqlite(Box<SqliteDb>),
}

impl Db {
    pub(crate) fn dialect(&self) -> Dialect {
        match self {
            Db::Pg(_) => Dialect::Pg,
            Db::Sqlite(_) => Dialect::Sqlite,
        }
    }

    /// Run `body` in one transaction: committed when it returns `Ok`,
    /// rolled back when it returns `Err`. On SQLite the transaction holds
    /// the database's write lock from its start.
    pub(crate) fn tx<T>(
        &self,
        retry: Retry,
        mut body: impl FnMut(&mut Tx<'_, '_>) -> Result<T, DbError>,
    ) -> Result<T, String> {
        match self {
            Db::Pg(pool) => {
                let run = |c: &mut postgres::Client| -> Result<T, postgres::Error> {
                    let mut t = c.transaction()?;
                    let value = body(&mut Tx::PgTransaction(&mut t)).map_err(pg_error)?;
                    t.commit()?;
                    Ok(value)
                };
                match retry {
                    Retry::Replay => pool.with_client(run),
                    Retry::Once => pool.with_client_once(run),
                }
            }
            Db::Sqlite(s) => {
                let conn = s.write.lock().unwrap_or_else(|e| e.into_inner());
                // A transaction a panic left open (the guard below normally
                // ends it) is rolled back before this one begins.
                if !conn.is_autocommit() {
                    let _ = conn.execute_batch("ROLLBACK");
                }
                conn.execute_batch("BEGIN IMMEDIATE")
                    .map_err(|e| format!("shard directory: {e}"))?;
                // A panic in the body rolls back at once: the transaction
                // holds the database's write lock, which every app write
                // waits for.
                struct RollbackOnPanic<'c>(&'c rusqlite::Connection);
                impl Drop for RollbackOnPanic<'_> {
                    fn drop(&mut self) {
                        if std::thread::panicking() && !self.0.is_autocommit() {
                            let _ = self.0.execute_batch("ROLLBACK");
                        }
                    }
                }
                let _guard = RollbackOnPanic(&conn);
                match body(&mut Tx::Sqlite(&conn)) {
                    Ok(value) => match conn.execute_batch("COMMIT") {
                        Ok(()) => Ok(value),
                        Err(e) => {
                            let _ = conn.execute_batch("ROLLBACK");
                            Err(format!("shard directory: {e}"))
                        }
                    },
                    Err(e) => {
                        let _ = conn.execute_batch("ROLLBACK");
                        Err(format!("shard directory: {e}"))
                    }
                }
            }
        }
    }

    /// Run `body` outside a transaction, for reads and single statements:
    /// a pooled Postgres client, or SQLite's read connection (`write`
    /// false) or write connection.
    pub(crate) fn run<T>(
        &self,
        write: bool,
        mut body: impl FnMut(&mut Tx<'_, '_>) -> Result<T, DbError>,
    ) -> Result<T, String> {
        match self {
            Db::Pg(pool) => pool.with_client(|c| body(&mut Tx::PgClient(c)).map_err(pg_error)),
            Db::Sqlite(s) => {
                let side = if write { &s.write } else { &s.read };
                let conn = side.lock().unwrap_or_else(|e| e.into_inner());
                body(&mut Tx::Sqlite(&conn)).map_err(|e| format!("shard directory: {e}"))
            }
        }
    }

    /// The Postgres pool, for tests that write rows directly.
    #[cfg(test)]
    pub(crate) fn pg_pool(&self) -> Option<&Arc<PgPool>> {
        match self {
            Db::Pg(pool) => Some(pool),
            Db::Sqlite(_) => None,
        }
    }
}

/// The Postgres error inside a directory error on the Postgres side.
fn pg_error(e: DbError) -> postgres::Error {
    match e {
        DbError::Pg(e) => e,
        DbError::Sqlite(e) => unreachable!("a SQLite error on a Postgres connection: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A panic inside a SQLite directory transaction ends it: the database's
    /// write lock is free for the app's writes and for the next directory
    /// transaction.
    #[test]
    fn a_panic_in_a_sqlite_transaction_releases_the_write_lock() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db");
        let path = path.to_str().unwrap();
        let db = Db::Sqlite(Box::new(
            SqliteDb::open_waiting(path, Duration::ZERO).unwrap(),
        ));
        db.tx(Retry::Once, |t| {
            t.execute("CREATE TABLE t (x INTEGER)", &[])?;
            Ok(())
        })
        .unwrap();
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = db.tx(Retry::Once, |t| -> Result<(), DbError> {
                t.execute("INSERT INTO t VALUES (1)", &[])?;
                panic!("a row of the wrong type");
            });
        }));
        assert!(panicked.is_err());
        let app = rusqlite::Connection::open(path).unwrap();
        app.busy_timeout(Duration::from_millis(100)).unwrap();
        app.execute("INSERT INTO t VALUES (2)", [])
            .expect("the write lock is still held");
        db.tx(Retry::Once, |t| {
            t.execute("INSERT INTO t VALUES (3)", &[])?;
            Ok(())
        })
        .unwrap();
        let rows: i64 = app
            .query_row("SELECT count(*) FROM t", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 2, "the panicked insert was rolled back");
    }
}
