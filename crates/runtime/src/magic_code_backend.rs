//! Persistent magic-code stores. Two backends ship today:
//!
//! - [`SqliteMagicCodeBackend`] — single-file durability. Default for
//!   self-hosted single-replica deploys. Lives in the same SQLite file
//!   as sessions/oauth-state (`PYLON_SESSION_DB`).
//! - [`PostgresMagicCodeBackend`] — multi-replica deploys against
//!   `DATABASE_URL=postgres://...`. Magic codes are short-lived (10 min)
//!   so a row-level pkey is sufficient — no GIN/index gymnastics needed.
//!
//! Both back the [`pylon_auth::MagicCodeBackend`] trait. The store layer
//! in pylon-auth keeps an in-memory cache as the authoritative read path;
//! these backends are write-through + load-on-startup so a server
//! restart between "send magic code" and "verify" doesn't kill the
//! user's pending login.

use std::sync::{Arc, Mutex};

use pylon_auth::{MagicCode, MagicCodeBackend};
use rusqlite::Connection;

const SQLITE_TABLE: &str = "_pylon_magic_codes";
const PG_TABLE: &str = "_pylon_magic_codes";

// ---------------------------------------------------------------------------
// SQLite backend
// ---------------------------------------------------------------------------

pub struct SqliteMagicCodeBackend {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteMagicCodeBackend {
    pub fn open(path: &str) -> Result<Self, String> {
        let conn = Connection::open(path).map_err(|e| format!("open: {e}"))?;
        Self::from_connection(conn)
    }

    pub fn in_memory() -> Result<Self, String> {
        let conn = Connection::open_in_memory().map_err(|e| format!("open: {e}"))?;
        Self::from_connection(conn)
    }

    fn from_connection(conn: Connection) -> Result<Self, String> {
        conn.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS {SQLITE_TABLE} (
                email TEXT PRIMARY KEY,
                code TEXT NOT NULL,
                expires_at INTEGER NOT NULL,
                attempts INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS {SQLITE_TABLE}_exp_idx ON {SQLITE_TABLE}(expires_at);"
        ))
        .map_err(|e| format!("init schema: {e}"))?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }
}

impl MagicCodeBackend for SqliteMagicCodeBackend {
    fn put(&self, email: &str, code: &MagicCode) {
        if let Ok(guard) = self.conn.lock() {
            let _ = guard.execute(
                &format!(
                    "INSERT INTO {SQLITE_TABLE} (email, code, expires_at, attempts)
                     VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(email) DO UPDATE SET
                       code=excluded.code,
                       expires_at=excluded.expires_at,
                       attempts=excluded.attempts"
                ),
                rusqlite::params![
                    email,
                    code.code,
                    code.expires_at as i64,
                    code.attempts as i64
                ],
            );
        }
    }
    fn get(&self, email: &str) -> Option<MagicCode> {
        let guard = self.conn.lock().ok()?;
        guard
            .query_row(
                &format!(
                    "SELECT email, code, expires_at, attempts FROM {SQLITE_TABLE} WHERE email = ?1"
                ),
                rusqlite::params![email],
                |row| {
                    Ok(MagicCode {
                        email: row.get(0)?,
                        code: row.get(1)?,
                        expires_at: row.get::<_, i64>(2)? as u64,
                        attempts: row.get::<_, i64>(3)? as u32,
                    })
                },
            )
            .ok()
    }
    fn remove(&self, email: &str) {
        if let Ok(guard) = self.conn.lock() {
            let _ = guard.execute(
                &format!("DELETE FROM {SQLITE_TABLE} WHERE email = ?1"),
                rusqlite::params![email],
            );
        }
    }
    fn bump_attempts(&self, email: &str) {
        if let Ok(guard) = self.conn.lock() {
            let _ = guard.execute(
                &format!("UPDATE {SQLITE_TABLE} SET attempts = attempts + 1 WHERE email = ?1"),
                rusqlite::params![email],
            );
        }
    }
    fn load_all(&self) -> Vec<MagicCode> {
        let Ok(guard) = self.conn.lock() else {
            return Vec::new();
        };
        let mut stmt = match guard.prepare(&format!(
            "SELECT email, code, expires_at, attempts FROM {SQLITE_TABLE}"
        )) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let iter = match stmt.query_map([], |row| {
            Ok(MagicCode {
                email: row.get(0)?,
                code: row.get(1)?,
                expires_at: row.get::<_, i64>(2)? as u64,
                attempts: row.get::<_, i64>(3)? as u32,
            })
        }) {
            Ok(i) => i,
            Err(_) => return Vec::new(),
        };
        iter.flatten().collect()
    }
}

// ---------------------------------------------------------------------------
// Postgres backend
// ---------------------------------------------------------------------------

pub use pg::PostgresMagicCodeBackend;

mod pg {
    use super::*;
    use postgres::Client;

    pub struct PostgresMagicCodeBackend {
        client: Mutex<Client>,
    }

    impl PostgresMagicCodeBackend {
        pub fn connect(url: &str) -> Result<Self, String> {
            let mut client = pylon_storage::postgres::live::connect_pg(url)?;
            client
                .batch_execute(&format!(
                    "CREATE TABLE IF NOT EXISTS {PG_TABLE} (
                        email TEXT PRIMARY KEY,
                        code TEXT NOT NULL,
                        expires_at BIGINT NOT NULL,
                        attempts INTEGER NOT NULL DEFAULT 0
                    );
                    CREATE INDEX IF NOT EXISTS {PG_TABLE}_exp_idx ON {PG_TABLE}(expires_at);"
                ))
                .map_err(|e| format!("PG init schema: {e}"))?;
            Ok(Self {
                client: Mutex::new(client),
            })
        }
    }

    impl MagicCodeBackend for PostgresMagicCodeBackend {
        fn put(&self, email: &str, code: &MagicCode) {
            let Ok(mut c) = self.client.lock() else {
                return;
            };
            let _ = c.execute(
                &format!(
                    "INSERT INTO {PG_TABLE} (email, code, expires_at, attempts)
                     VALUES ($1, $2, $3, $4)
                     ON CONFLICT (email) DO UPDATE SET
                       code = EXCLUDED.code,
                       expires_at = EXCLUDED.expires_at,
                       attempts = EXCLUDED.attempts"
                ),
                &[
                    &email,
                    &code.code,
                    &(code.expires_at as i64),
                    &(code.attempts as i32),
                ],
            );
        }
        fn get(&self, email: &str) -> Option<MagicCode> {
            let mut c = self.client.lock().ok()?;
            let row = c
                .query_opt(
                    &format!(
                        "SELECT email, code, expires_at, attempts FROM {PG_TABLE} WHERE email = $1"
                    ),
                    &[&email],
                )
                .ok()??;
            Some(MagicCode {
                email: row.get(0),
                code: row.get(1),
                expires_at: row.get::<_, i64>(2) as u64,
                attempts: row.get::<_, i32>(3) as u32,
            })
        }
        fn remove(&self, email: &str) {
            if let Ok(mut c) = self.client.lock() {
                let _ = c.execute(
                    &format!("DELETE FROM {PG_TABLE} WHERE email = $1"),
                    &[&email],
                );
            }
        }
        fn bump_attempts(&self, email: &str) {
            if let Ok(mut c) = self.client.lock() {
                let _ = c.execute(
                    &format!("UPDATE {PG_TABLE} SET attempts = attempts + 1 WHERE email = $1"),
                    &[&email],
                );
            }
        }
        fn load_all(&self) -> Vec<MagicCode> {
            let Ok(mut c) = self.client.lock() else {
                return Vec::new();
            };
            let rows = c
                .query(
                    &format!("SELECT email, code, expires_at, attempts FROM {PG_TABLE}"),
                    &[],
                )
                .unwrap_or_default();
            rows.iter()
                .map(|row| MagicCode {
                    email: row.get(0),
                    code: row.get(1),
                    expires_at: row.get::<_, i64>(2) as u64,
                    attempts: row.get::<_, i32>(3) as u32,
                })
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqlite_roundtrip_put_get_remove() {
        let b = SqliteMagicCodeBackend::in_memory().unwrap();
        let mc = MagicCode {
            email: "a@b.com".into(),
            code: "123456".into(),
            expires_at: 9999999999,
            attempts: 0,
        };
        b.put(&mc.email, &mc);
        let got = b.get(&mc.email).unwrap();
        assert_eq!(got.code, "123456");
        b.bump_attempts(&mc.email);
        assert_eq!(b.get(&mc.email).unwrap().attempts, 1);
        b.remove(&mc.email);
        assert!(b.get(&mc.email).is_none());
    }
}
