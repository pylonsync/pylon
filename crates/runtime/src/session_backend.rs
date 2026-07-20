//! SQLite-backed session persistence.
//!
//! Stores sessions in a dedicated `_pylon_sessions` table so users don't
//! get logged out when the server restarts.
//!
//! The schema is intentionally minimal and under-engineered: every session
//! mutation is a single UPSERT/DELETE. Reads happen only at startup via
//! `load_all`. If session-churn ever outgrows this, sharding/indexing can
//! come later without changing the trait contract.

use std::sync::{Arc, Mutex};

use pylon_auth::{Session, SessionBackend};
use rusqlite::Connection;

const TABLE: &str = "_pylon_sessions";

/// Persistent session backend backed by a SQLite connection.
///
/// Holds the connection behind a `Mutex` because SQLite's `Connection`
/// isn't `Sync`. Sessions are low-frequency compared to CRUD — this lock
/// is not a hot path.
pub struct SqliteSessionBackend {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteSessionBackend {
    /// Open or create a SQLite file and ensure the session table exists.
    pub fn open(path: &str) -> Result<Self, String> {
        let conn = Connection::open(path).map_err(|e| format!("open: {e}"))?;
        crate::tune_runtime_connection(&conn, false)
            .map_err(|e| format!("pragma init failed: {e}"))?;
        Self::from_connection(conn)
    }

    /// Use an in-memory database (for tests).
    pub fn in_memory() -> Result<Self, String> {
        let conn = Connection::open_in_memory().map_err(|e| format!("open: {e}"))?;
        crate::tune_runtime_connection(&conn, true)
            .map_err(|e| format!("pragma init failed: {e}"))?;
        Self::from_connection(conn)
    }

    fn from_connection(conn: Connection) -> Result<Self, String> {
        // Base table for new installs. Existing installs miss `tenant_id`
        // and get an ALTER below — ADD COLUMN is a no-op on a table that
        // already has the column, so we swallow its error for idempotency.
        conn.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS {TABLE} (
                token TEXT PRIMARY KEY,
                user_id TEXT NOT NULL,
                expires_at INTEGER NOT NULL,
                created_at INTEGER NOT NULL,
                device TEXT,
                tenant_id TEXT,
                is_guest INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS {TABLE}_user_idx ON {TABLE}(user_id);
            CREATE INDEX IF NOT EXISTS {TABLE}_exp_idx ON {TABLE}(expires_at);"
        ))
        .map_err(|e| format!("init schema: {e}"))?;
        // Idempotent migrations for pre-existing session DBs (ADD COLUMN
        // errors are swallowed — no-op when the column already exists).
        let _ = conn.execute(
            &format!("ALTER TABLE {TABLE} ADD COLUMN tenant_id TEXT"),
            [],
        );
        let _ = conn.execute(
            &format!("ALTER TABLE {TABLE} ADD COLUMN is_guest INTEGER NOT NULL DEFAULT 0"),
            [],
        );
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }
}

impl SessionBackend for SqliteSessionBackend {
    fn load_all(&self) -> Vec<Session> {
        let guard = match self.conn.lock() {
            Ok(g) => g,
            Err(_) => return Vec::new(),
        };
        let mut stmt = match guard.prepare(&format!(
            "SELECT token, user_id, expires_at, created_at, device, tenant_id, is_guest FROM {TABLE}"
        )) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let iter = match stmt.query_map([], |row| {
            Ok(Session {
                token: row.get(0)?,
                user_id: row.get(1)?,
                expires_at: row.get::<_, i64>(2)? as u64,
                created_at: row.get::<_, i64>(3)? as u64,
                device: row.get::<_, Option<String>>(4)?,
                tenant_id: row.get::<_, Option<String>>(5)?,
                is_guest: row.get::<_, i64>(6).unwrap_or(0) != 0,
            })
        }) {
            Ok(i) => i,
            Err(_) => return Vec::new(),
        };
        iter.flatten().collect()
    }

    fn save(&self, session: &Session) {
        if let Ok(guard) = self.conn.lock() {
            let _ = guard.execute(
                &format!(
                    "INSERT INTO {TABLE} (token, user_id, expires_at, created_at, device, tenant_id, is_guest)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                     ON CONFLICT(token) DO UPDATE SET
                       user_id=excluded.user_id,
                       expires_at=excluded.expires_at,
                       device=excluded.device,
                       tenant_id=excluded.tenant_id,
                       is_guest=excluded.is_guest"
                ),
                rusqlite::params![
                    session.token,
                    session.user_id,
                    session.expires_at as i64,
                    session.created_at as i64,
                    session.device,
                    session.tenant_id,
                    session.is_guest as i64,
                ],
            );
        }
    }

    fn remove(&self, token: &str) {
        if let Ok(guard) = self.conn.lock() {
            let _ = guard.execute(
                &format!("DELETE FROM {TABLE} WHERE token = ?1"),
                rusqlite::params![token],
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Postgres backend
// ---------------------------------------------------------------------------

pub use pg::PostgresSessionBackend;

mod pg {
    use super::*;
    use pylon_storage::postgres::live::ReconnectingPgClient;

    const PG_TABLE: &str = "_pylon_sessions";

    /// Postgres-backed session store. Schema mirrors the SQLite version
    /// — same column set + same indexes — so a deploy that flips
    /// `DATABASE_URL` from a local SQLite file to a managed PG cluster
    /// only changes WHERE the rows live, not what the rows mean.
    pub struct PostgresSessionBackend {
        conn: ReconnectingPgClient,
    }

    impl PostgresSessionBackend {
        pub fn connect(url: &str) -> Result<Self, String> {
            let conn = ReconnectingPgClient::connect(url)?;
            conn.with_client(|c| {
                c.batch_execute(&format!(
                    "CREATE TABLE IF NOT EXISTS {PG_TABLE} (
                        token TEXT PRIMARY KEY,
                        user_id TEXT NOT NULL,
                        expires_at BIGINT NOT NULL,
                        created_at BIGINT NOT NULL,
                        device TEXT,
                        tenant_id TEXT,
                        is_guest BOOLEAN NOT NULL DEFAULT FALSE
                    );
                    ALTER TABLE {PG_TABLE} ADD COLUMN IF NOT EXISTS is_guest BOOLEAN NOT NULL DEFAULT FALSE;
                    CREATE INDEX IF NOT EXISTS {PG_TABLE}_user_idx ON {PG_TABLE}(user_id);
                    CREATE INDEX IF NOT EXISTS {PG_TABLE}_exp_idx ON {PG_TABLE}(expires_at);"
                ))
            })
            .map_err(|e| format!("PG init schema: {e}"))?;
            Ok(Self { conn })
        }
    }

    impl SessionBackend for PostgresSessionBackend {
        fn load_all(&self) -> Vec<Session> {
            let rows = match self.conn.with_client(|c| {
                c.query(
                    &format!(
                        "SELECT token, user_id, expires_at, created_at, device, tenant_id, is_guest
                         FROM {PG_TABLE}"
                    ),
                    &[],
                )
            }) {
                Ok(rows) => rows,
                Err(e) => {
                    tracing::warn!("[pg] session load_all failed: {e}");
                    return Vec::new();
                }
            };
            rows.iter()
                .map(|row| Session {
                    token: row.get(0),
                    user_id: row.get(1),
                    expires_at: row.get::<_, i64>(2) as u64,
                    created_at: row.get::<_, i64>(3) as u64,
                    device: row.get::<_, Option<String>>(4),
                    tenant_id: row.get::<_, Option<String>>(5),
                    is_guest: row.try_get::<_, bool>(6).unwrap_or(false),
                })
                .collect()
        }

        fn save(&self, session: &Session) {
            if let Err(e) = self.conn.with_client(|c| {
                c.execute(
                    &format!(
                        "INSERT INTO {PG_TABLE} (token, user_id, expires_at, created_at, device, tenant_id, is_guest)
                         VALUES ($1, $2, $3, $4, $5, $6, $7)
                         ON CONFLICT (token) DO UPDATE SET
                           user_id = EXCLUDED.user_id,
                           expires_at = EXCLUDED.expires_at,
                           device = EXCLUDED.device,
                           tenant_id = EXCLUDED.tenant_id,
                           is_guest = EXCLUDED.is_guest"
                    ),
                    &[
                        &session.token,
                        &session.user_id,
                        &(session.expires_at as i64),
                        &(session.created_at as i64),
                        &session.device,
                        &session.tenant_id,
                        &session.is_guest,
                    ],
                )
            }) {
                tracing::warn!("[pg] session save failed: {e}");
            }
        }

        fn remove(&self, token: &str) {
            if let Err(e) = self.conn.with_client(|c| {
                c.execute(
                    &format!("DELETE FROM {PG_TABLE} WHERE token = $1"),
                    &[&token],
                )
            }) {
                tracing::warn!("[pg] session remove failed: {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pylon_auth::Session;

    #[test]
    fn roundtrip_save_load() {
        let backend = SqliteSessionBackend::in_memory().unwrap();
        let session = Session::new("user_1".to_string());
        backend.save(&session);
        let loaded = backend.load_all();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].user_id, "user_1");
        assert_eq!(loaded[0].token, session.token);
        assert!(!loaded[0].is_guest, "a normal session is not a guest");
    }

    // The guest flag MUST survive the SQL round-trip — else a guest session
    // reloaded from disk (e.g. after a restart, or hydrated on boot via
    // load_all) resolves as a non-guest and the auth:"user" gate is defeated.
    #[test]
    fn guest_flag_survives_roundtrip() {
        let backend = SqliteSessionBackend::in_memory().unwrap();
        let mut session = Session::new("guest_abc".to_string());
        session.is_guest = true;
        backend.save(&session);
        let loaded = backend.load_all();
        assert_eq!(loaded.len(), 1);
        assert!(loaded[0].is_guest, "guest flag must persist through SQLite");
        // And the resolved auth context must be a non-authenticated guest.
        assert!(!loaded[0].to_auth_context().is_authenticated());
    }

    #[test]
    fn remove_takes_effect() {
        let backend = SqliteSessionBackend::in_memory().unwrap();
        let session = Session::new("u".to_string());
        backend.save(&session);
        backend.remove(&session.token);
        assert!(backend.load_all().is_empty());
    }

    #[test]
    fn upsert_on_save_twice() {
        let backend = SqliteSessionBackend::in_memory().unwrap();
        let mut session = Session::new("u".to_string());
        backend.save(&session);
        session.device = Some("Safari on Mac".into());
        backend.save(&session);
        let loaded = backend.load_all();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].device.as_deref(), Some("Safari on Mac"));
    }
}
