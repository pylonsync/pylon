//! Persistent API key stores. SQLite + Postgres backends behind the
//! [`pylon_auth::api_key::ApiKeyBackend`] trait.
//!
//! API keys are long-lived (months to never-expire), so unlike magic
//! codes there's no aggressive expiry sweep — `last_used_at` plus
//! the user-facing management UI handle the "remove unused" pattern.
//!
//! Storage shape mirrors the SQLite default — same column types both
//! sides so a SQLite → Postgres migration is `pg_dump`-style copy
//! without coercions. `secret_hash` is an Argon2 PHC string so old
//! keys keep verifying after a hash-param bump.

use std::sync::{Arc, Mutex};

use pylon_auth::api_key::{ApiKey, ApiKeyBackend};
use rusqlite::Connection;

const SQLITE_TABLE: &str = "_pylon_api_keys";
const PG_TABLE: &str = "_pylon_api_keys";

// ---------------------------------------------------------------------------
// SQLite backend
// ---------------------------------------------------------------------------

pub struct SqliteApiKeyBackend {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteApiKeyBackend {
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
                id TEXT PRIMARY KEY,
                user_id TEXT NOT NULL,
                name TEXT NOT NULL DEFAULT '',
                prefix TEXT NOT NULL DEFAULT '',
                secret_hash TEXT NOT NULL,
                scopes TEXT,
                expires_at INTEGER,
                last_used_at INTEGER,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS {SQLITE_TABLE}_user_idx ON {SQLITE_TABLE}(user_id);"
        ))
        .map_err(|e| format!("init schema: {e}"))?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }
}

impl ApiKeyBackend for SqliteApiKeyBackend {
    fn put(&self, key: &ApiKey) {
        if let Ok(guard) = self.conn.lock() {
            let _ = guard.execute(
                &format!(
                    "INSERT INTO {SQLITE_TABLE}
                       (id, user_id, name, prefix, secret_hash, scopes, expires_at, last_used_at, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                     ON CONFLICT(id) DO UPDATE SET
                       name=excluded.name,
                       prefix=excluded.prefix,
                       secret_hash=excluded.secret_hash,
                       scopes=excluded.scopes,
                       expires_at=excluded.expires_at,
                       last_used_at=excluded.last_used_at"
                ),
                rusqlite::params![
                    key.id,
                    key.user_id,
                    key.name,
                    key.prefix,
                    key.secret_hash,
                    key.scopes,
                    key.expires_at.map(|v| v as i64),
                    key.last_used_at.map(|v| v as i64),
                    key.created_at as i64,
                ],
            );
        }
    }

    fn get(&self, id: &str) -> Option<ApiKey> {
        let guard = self.conn.lock().ok()?;
        guard
            .query_row(
                &format!(
                    "SELECT id, user_id, name, prefix, secret_hash, scopes, expires_at, last_used_at, created_at
                     FROM {SQLITE_TABLE} WHERE id = ?1"
                ),
                rusqlite::params![id],
                row_to_key,
            )
            .ok()
    }

    fn delete(&self, id: &str) -> bool {
        let Ok(guard) = self.conn.lock() else {
            return false;
        };
        guard
            .execute(
                &format!("DELETE FROM {SQLITE_TABLE} WHERE id = ?1"),
                rusqlite::params![id],
            )
            .map(|n| n > 0)
            .unwrap_or(false)
    }

    fn list_for_user(&self, user_id: &str) -> Vec<ApiKey> {
        let Ok(guard) = self.conn.lock() else {
            return vec![];
        };
        let mut stmt = match guard.prepare(&format!(
            "SELECT id, user_id, name, prefix, secret_hash, scopes, expires_at, last_used_at, created_at
             FROM {SQLITE_TABLE} WHERE user_id = ?1 ORDER BY created_at DESC"
        )) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        let iter = match stmt.query_map(rusqlite::params![user_id], row_to_key) {
            Ok(it) => it,
            Err(_) => return vec![],
        };
        iter.filter_map(|r| r.ok()).collect()
    }

    fn touch(&self, id: &str, now: u64) {
        if let Ok(guard) = self.conn.lock() {
            let _ = guard.execute(
                &format!("UPDATE {SQLITE_TABLE} SET last_used_at = ?2 WHERE id = ?1"),
                rusqlite::params![id, now as i64],
            );
        }
    }
}

fn row_to_key(row: &rusqlite::Row<'_>) -> rusqlite::Result<ApiKey> {
    Ok(ApiKey {
        id: row.get(0)?,
        user_id: row.get(1)?,
        name: row.get(2)?,
        prefix: row.get(3)?,
        secret_hash: row.get(4)?,
        scopes: row.get(5)?,
        expires_at: row.get::<_, Option<i64>>(6)?.map(|v| v as u64),
        last_used_at: row.get::<_, Option<i64>>(7)?.map(|v| v as u64),
        created_at: row.get::<_, i64>(8)? as u64,
    })
}

// ---------------------------------------------------------------------------
// Postgres backend
// ---------------------------------------------------------------------------

pub use pg::PostgresApiKeyBackend;

mod pg {
    use super::*;
    use postgres::Client;

    pub struct PostgresApiKeyBackend {
        client: Mutex<Client>,
    }

    impl PostgresApiKeyBackend {
        pub fn connect(url: &str) -> Result<Self, String> {
            let mut client = pylon_storage::postgres::live::connect_pg(url)?;
            client
                .batch_execute(&format!(
                    "CREATE TABLE IF NOT EXISTS {PG_TABLE} (
                        id TEXT PRIMARY KEY,
                        user_id TEXT NOT NULL,
                        name TEXT NOT NULL DEFAULT '',
                        prefix TEXT NOT NULL DEFAULT '',
                        secret_hash TEXT NOT NULL,
                        scopes TEXT,
                        expires_at BIGINT,
                        last_used_at BIGINT,
                        created_at BIGINT NOT NULL
                    );
                    CREATE INDEX IF NOT EXISTS {PG_TABLE}_user_idx ON {PG_TABLE}(user_id);"
                ))
                .map_err(|e| format!("PG init schema: {e}"))?;
            Ok(Self {
                client: Mutex::new(client),
            })
        }
    }

    impl ApiKeyBackend for PostgresApiKeyBackend {
        fn put(&self, key: &ApiKey) {
            if let Ok(mut c) = self.client.lock() {
                let _ = c.execute(
                    &format!(
                        "INSERT INTO {PG_TABLE}
                           (id, user_id, name, prefix, secret_hash, scopes, expires_at, last_used_at, created_at)
                         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                         ON CONFLICT (id) DO UPDATE SET
                           name = EXCLUDED.name,
                           prefix = EXCLUDED.prefix,
                           secret_hash = EXCLUDED.secret_hash,
                           scopes = EXCLUDED.scopes,
                           expires_at = EXCLUDED.expires_at,
                           last_used_at = EXCLUDED.last_used_at"
                    ),
                    &[
                        &key.id,
                        &key.user_id,
                        &key.name,
                        &key.prefix,
                        &key.secret_hash,
                        &key.scopes,
                        &key.expires_at.map(|v| v as i64),
                        &key.last_used_at.map(|v| v as i64),
                        &(key.created_at as i64),
                    ],
                );
            }
        }

        fn get(&self, id: &str) -> Option<ApiKey> {
            let mut c = self.client.lock().ok()?;
            let row = c
                .query_opt(
                    &format!(
                        "SELECT id, user_id, name, prefix, secret_hash, scopes, expires_at, last_used_at, created_at
                         FROM {PG_TABLE} WHERE id = $1"
                    ),
                    &[&id],
                )
                .ok()??;
            Some(pg_row_to_key(&row))
        }

        fn delete(&self, id: &str) -> bool {
            let mut c = match self.client.lock() {
                Ok(c) => c,
                Err(_) => return false,
            };
            c.execute(&format!("DELETE FROM {PG_TABLE} WHERE id = $1"), &[&id])
                .map(|n| n > 0)
                .unwrap_or(false)
        }

        fn list_for_user(&self, user_id: &str) -> Vec<ApiKey> {
            let Ok(mut c) = self.client.lock() else {
                return vec![];
            };
            let rows = match c.query(
                &format!(
                    "SELECT id, user_id, name, prefix, secret_hash, scopes, expires_at, last_used_at, created_at
                     FROM {PG_TABLE} WHERE user_id = $1 ORDER BY created_at DESC"
                ),
                &[&user_id],
            ) {
                Ok(r) => r,
                Err(_) => return vec![],
            };
            rows.iter().map(pg_row_to_key).collect()
        }

        fn touch(&self, id: &str, now: u64) {
            if let Ok(mut c) = self.client.lock() {
                let _ = c.execute(
                    &format!("UPDATE {PG_TABLE} SET last_used_at = $2 WHERE id = $1"),
                    &[&id, &(now as i64)],
                );
            }
        }
    }

    fn pg_row_to_key(row: &postgres::Row) -> ApiKey {
        ApiKey {
            id: row.get(0),
            user_id: row.get(1),
            name: row.get(2),
            prefix: row.get(3),
            secret_hash: row.get(4),
            scopes: row.get(5),
            expires_at: row.get::<_, Option<i64>>(6).map(|v| v as u64),
            last_used_at: row.get::<_, Option<i64>>(7).map(|v| v as u64),
            created_at: row.get::<_, i64>(8) as u64,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqlite_roundtrip() {
        let backend = SqliteApiKeyBackend::in_memory().unwrap();
        let key = ApiKey {
            id: "key_test".into(),
            user_id: "user_1".into(),
            name: "ci".into(),
            prefix: "pk.key_test.…".into(),
            secret_hash: "$argon2id$...".into(),
            scopes: Some("read".into()),
            expires_at: Some(123),
            last_used_at: None,
            created_at: 100,
        };
        backend.put(&key);
        let got = backend.get("key_test").unwrap();
        assert_eq!(got.user_id, "user_1");
        assert_eq!(got.name, "ci");
        assert_eq!(got.scopes.as_deref(), Some("read"));
        assert_eq!(got.expires_at, Some(123));
        // touch updates last_used_at
        backend.touch("key_test", 999);
        assert_eq!(backend.get("key_test").unwrap().last_used_at, Some(999));
        // delete removes
        assert!(backend.delete("key_test"));
        assert!(backend.get("key_test").is_none());
    }

    #[test]
    fn sqlite_list_for_user_orders_newest_first() {
        let backend = SqliteApiKeyBackend::in_memory().unwrap();
        for (i, name) in ["a", "b", "c"].iter().enumerate() {
            backend.put(&ApiKey {
                id: format!("key_{i}"),
                user_id: "u".into(),
                name: name.to_string(),
                prefix: "p".into(),
                secret_hash: "h".into(),
                scopes: None,
                expires_at: None,
                last_used_at: None,
                created_at: 100 + i as u64,
            });
        }
        let list = backend.list_for_user("u");
        // Newest first → "c", "b", "a"
        let names: Vec<_> = list.iter().map(|k| k.name.clone()).collect();
        assert_eq!(names, vec!["c", "b", "a"]);
    }
}
