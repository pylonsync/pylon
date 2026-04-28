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
        Self::from_connection(conn)
    }

    /// Use an in-memory database (for tests).
    pub fn in_memory() -> Result<Self, String> {
        let conn = Connection::open_in_memory().map_err(|e| format!("open: {e}"))?;
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
                tenant_id TEXT
            );
            CREATE INDEX IF NOT EXISTS {TABLE}_user_idx ON {TABLE}(user_id);
            CREATE INDEX IF NOT EXISTS {TABLE}_exp_idx ON {TABLE}(expires_at);"
        ))
        .map_err(|e| format!("init schema: {e}"))?;
        // Idempotent migration for pre-existing session DBs.
        let _ = conn.execute(
            &format!("ALTER TABLE {TABLE} ADD COLUMN tenant_id TEXT"),
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
            "SELECT token, user_id, expires_at, created_at, device, tenant_id FROM {TABLE}"
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
                    "INSERT INTO {TABLE} (token, user_id, expires_at, created_at, device, tenant_id)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT(token) DO UPDATE SET
                       user_id=excluded.user_id,
                       expires_at=excluded.expires_at,
                       device=excluded.device,
                       tenant_id=excluded.tenant_id"
                ),
                rusqlite::params![
                    session.token,
                    session.user_id,
                    session.expires_at as i64,
                    session.created_at as i64,
                    session.device,
                    session.tenant_id,
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
    use postgres::Client;
    use std::sync::Mutex;

    const PG_TABLE: &str = "_pylon_sessions";

    /// Postgres-backed session store. Schema mirrors the SQLite version
    /// — same column set + same indexes — so a deploy that flips
    /// `DATABASE_URL` from a local SQLite file to a managed PG cluster
    /// only changes WHERE the rows live, not what the rows mean.
    pub struct PostgresSessionBackend {
        client: Mutex<Client>,
    }

    impl PostgresSessionBackend {
        pub fn connect(url: &str) -> Result<Self, String> {
            let mut client =
                Client::connect(url, postgres::NoTls).map_err(|e| format!("PG connect: {e}"))?;
            client
                .batch_execute(&format!(
                    "CREATE TABLE IF NOT EXISTS {PG_TABLE} (
                        token TEXT PRIMARY KEY,
                        user_id TEXT NOT NULL,
                        expires_at BIGINT NOT NULL,
                        created_at BIGINT NOT NULL,
                        device TEXT,
                        tenant_id TEXT
                    );
                    CREATE INDEX IF NOT EXISTS {PG_TABLE}_user_idx ON {PG_TABLE}(user_id);
                    CREATE INDEX IF NOT EXISTS {PG_TABLE}_exp_idx ON {PG_TABLE}(expires_at);"
                ))
                .map_err(|e| format!("PG init schema: {e}"))?;
            Ok(Self {
                client: Mutex::new(client),
            })
        }
    }

    impl SessionBackend for PostgresSessionBackend {
        fn load_all(&self) -> Vec<Session> {
            let Ok(mut c) = self.client.lock() else {
                return Vec::new();
            };
            let rows = c
                .query(
                    &format!(
                        "SELECT token, user_id, expires_at, created_at, device, tenant_id
                         FROM {PG_TABLE}"
                    ),
                    &[],
                )
                .unwrap_or_default();
            rows.iter()
                .map(|row| Session {
                    token: row.get(0),
                    user_id: row.get(1),
                    expires_at: row.get::<_, i64>(2) as u64,
                    created_at: row.get::<_, i64>(3) as u64,
                    device: row.get::<_, Option<String>>(4),
                    tenant_id: row.get::<_, Option<String>>(5),
                })
                .collect()
        }

        fn save(&self, session: &Session) {
            if let Ok(mut c) = self.client.lock() {
                let _ = c.execute(
                    &format!(
                        "INSERT INTO {PG_TABLE} (token, user_id, expires_at, created_at, device, tenant_id)
                         VALUES ($1, $2, $3, $4, $5, $6)
                         ON CONFLICT (token) DO UPDATE SET
                           user_id = EXCLUDED.user_id,
                           expires_at = EXCLUDED.expires_at,
                           device = EXCLUDED.device,
                           tenant_id = EXCLUDED.tenant_id"
                    ),
                    &[
                        &session.token,
                        &session.user_id,
                        &(session.expires_at as i64),
                        &(session.created_at as i64),
                        &session.device,
                        &session.tenant_id,
                    ],
                );
            }
        }

        fn remove(&self, token: &str) {
            if let Ok(mut c) = self.client.lock() {
                let _ = c.execute(
                    &format!("DELETE FROM {PG_TABLE} WHERE token = $1"),
                    &[&token],
                );
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
