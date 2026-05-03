//! Persistent trusted-device backends. Pylon's "remember this device for
//! 30 days" records — apps gate their TOTP step on these by reading
//! `ctx.auth.isTrustedDevice`.
//!
//! Schema is intentionally tiny: token (PK), user_id (indexed for the
//! list-for-user endpoint + revoke-all-for-user during account deletion),
//! label, expires_at (indexed so a periodic GC sweep can prune in one
//! query), created_at.

use std::sync::{Arc, Mutex};

use pylon_auth::trusted_device::{TrustedDevice, TrustedDeviceStore};
use rusqlite::Connection;

const SQLITE_TABLE: &str = "_pylon_trusted_devices";
const PG_TABLE: &str = "_pylon_trusted_devices";

// ---------------------------------------------------------------------------
// SQLite
// ---------------------------------------------------------------------------

pub struct SqliteTrustedDeviceBackend {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteTrustedDeviceBackend {
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
                token TEXT PRIMARY KEY,
                id TEXT NOT NULL UNIQUE,
                user_id TEXT NOT NULL,
                label TEXT,
                created_at TEXT NOT NULL,
                expires_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS {SQLITE_TABLE}_user_idx
                ON {SQLITE_TABLE}(user_id);
            CREATE INDEX IF NOT EXISTS {SQLITE_TABLE}_exp_idx
                ON {SQLITE_TABLE}(expires_at);
            CREATE UNIQUE INDEX IF NOT EXISTS {SQLITE_TABLE}_id_idx
                ON {SQLITE_TABLE}(id);"
        ))
        .map_err(|e| format!("init schema: {e}"))?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }
}

impl TrustedDeviceStore for SqliteTrustedDeviceBackend {
    fn create(&self, device: TrustedDevice) {
        if let Ok(c) = self.conn.lock() {
            // Opportunistic GC of expired rows. The `find` path filters
            // by expiration anyway, but keeping the table small keeps
            // the user_id index hot.
            let now = current_unix_secs_string();
            let _ = c.execute(
                &format!("DELETE FROM {SQLITE_TABLE} WHERE expires_at <= ?1"),
                rusqlite::params![now],
            );
            let _ = c.execute(
                &format!(
                    "INSERT INTO {SQLITE_TABLE}
                       (token, id, user_id, label, created_at, expires_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)"
                ),
                rusqlite::params![
                    device.token,
                    device.id,
                    device.user_id,
                    device.label,
                    device.created_at,
                    device.expires_at,
                ],
            );
        }
    }

    fn find(&self, token: &str) -> Option<TrustedDevice> {
        let c = self.conn.lock().ok()?;
        let mut stmt = c
            .prepare(&format!(
                "SELECT token, id, user_id, label, created_at, expires_at
                 FROM {SQLITE_TABLE}
                 WHERE token = ?1"
            ))
            .ok()?;
        let device = stmt
            .query_row(rusqlite::params![token], row_to_device)
            .ok()?;
        if device.is_expired() {
            return None;
        }
        Some(device)
    }

    fn find_by_id(&self, id: &str) -> Option<TrustedDevice> {
        let c = self.conn.lock().ok()?;
        let mut stmt = c
            .prepare(&format!(
                "SELECT token, id, user_id, label, created_at, expires_at
                 FROM {SQLITE_TABLE}
                 WHERE id = ?1"
            ))
            .ok()?;
        let device = stmt.query_row(rusqlite::params![id], row_to_device).ok()?;
        if device.is_expired() {
            return None;
        }
        Some(device)
    }

    fn list_for_user(&self, user_id: &str) -> Vec<TrustedDevice> {
        let Ok(c) = self.conn.lock() else {
            return vec![];
        };
        let mut stmt = match c.prepare(&format!(
            "SELECT token, id, user_id, label, created_at, expires_at
             FROM {SQLITE_TABLE}
             WHERE user_id = ?1
             ORDER BY expires_at DESC"
        )) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        let iter = match stmt.query_map(rusqlite::params![user_id], row_to_device) {
            Ok(it) => it,
            Err(_) => return vec![],
        };
        iter.filter_map(|r| r.ok())
            .filter(|d| !d.is_expired())
            .collect()
    }

    fn revoke_by_id(&self, id: &str) -> bool {
        let Ok(c) = self.conn.lock() else {
            return false;
        };
        c.execute(
            &format!("DELETE FROM {SQLITE_TABLE} WHERE id = ?1"),
            rusqlite::params![id],
        )
        .map(|n| n > 0)
        .unwrap_or(false)
    }

    fn revoke_all_for_user(&self, user_id: &str) -> usize {
        let Ok(c) = self.conn.lock() else {
            return 0;
        };
        c.execute(
            &format!("DELETE FROM {SQLITE_TABLE} WHERE user_id = ?1"),
            rusqlite::params![user_id],
        )
        .unwrap_or(0)
    }
}

fn row_to_device(row: &rusqlite::Row<'_>) -> rusqlite::Result<TrustedDevice> {
    Ok(TrustedDevice {
        token: row.get(0)?,
        id: row.get(1)?,
        user_id: row.get(2)?,
        label: row.get(3)?,
        created_at: row.get(4)?,
        expires_at: row.get(5)?,
    })
}

fn current_unix_secs_string() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("{secs}")
}

// ---------------------------------------------------------------------------
// Postgres
// ---------------------------------------------------------------------------

pub use pg::PostgresTrustedDeviceBackend;

mod pg {
    use super::*;
    use postgres::Client;

    pub struct PostgresTrustedDeviceBackend {
        client: Mutex<Client>,
    }

    impl PostgresTrustedDeviceBackend {
        pub fn connect(url: &str) -> Result<Self, String> {
            let mut client = pylon_storage::postgres::live::connect_pg(url)?;
            client
                .batch_execute(&format!(
                    "CREATE TABLE IF NOT EXISTS {PG_TABLE} (
                        token TEXT PRIMARY KEY,
                        id TEXT NOT NULL UNIQUE,
                        user_id TEXT NOT NULL,
                        label TEXT,
                        created_at TEXT NOT NULL,
                        expires_at TEXT NOT NULL
                    );
                    CREATE INDEX IF NOT EXISTS {PG_TABLE}_user_idx
                        ON {PG_TABLE}(user_id);
                    CREATE INDEX IF NOT EXISTS {PG_TABLE}_exp_idx
                        ON {PG_TABLE}(expires_at);"
                ))
                .map_err(|e| format!("PG init schema: {e}"))?;
            Ok(Self {
                client: Mutex::new(client),
            })
        }
    }

    impl TrustedDeviceStore for PostgresTrustedDeviceBackend {
        fn create(&self, device: TrustedDevice) {
            if let Ok(mut c) = self.client.lock() {
                let now = super::current_unix_secs_string();
                let _ = c.execute(
                    &format!("DELETE FROM {PG_TABLE} WHERE expires_at <= $1"),
                    &[&now],
                );
                let _ = c.execute(
                    &format!(
                        "INSERT INTO {PG_TABLE}
                           (token, id, user_id, label, created_at, expires_at)
                         VALUES ($1, $2, $3, $4, $5, $6)
                         ON CONFLICT (token) DO NOTHING"
                    ),
                    &[
                        &device.token,
                        &device.id,
                        &device.user_id,
                        &device.label,
                        &device.created_at,
                        &device.expires_at,
                    ],
                );
            }
        }

        fn find(&self, token: &str) -> Option<TrustedDevice> {
            let mut c = self.client.lock().ok()?;
            let row = c
                .query_opt(
                    &format!(
                        "SELECT token, id, user_id, label, created_at, expires_at
                         FROM {PG_TABLE}
                         WHERE token = $1"
                    ),
                    &[&token],
                )
                .ok()?
                .as_ref()
                .map(pg_row_to_device)?;
            if row.is_expired() {
                return None;
            }
            Some(row)
        }

        fn find_by_id(&self, id: &str) -> Option<TrustedDevice> {
            let mut c = self.client.lock().ok()?;
            let row = c
                .query_opt(
                    &format!(
                        "SELECT token, id, user_id, label, created_at, expires_at
                         FROM {PG_TABLE}
                         WHERE id = $1"
                    ),
                    &[&id],
                )
                .ok()?
                .as_ref()
                .map(pg_row_to_device)?;
            if row.is_expired() {
                return None;
            }
            Some(row)
        }

        fn list_for_user(&self, user_id: &str) -> Vec<TrustedDevice> {
            let Ok(mut c) = self.client.lock() else {
                return vec![];
            };
            let rows = match c.query(
                &format!(
                    "SELECT token, id, user_id, label, created_at, expires_at
                     FROM {PG_TABLE}
                     WHERE user_id = $1
                     ORDER BY expires_at DESC"
                ),
                &[&user_id],
            ) {
                Ok(r) => r,
                Err(_) => return vec![],
            };
            rows.iter()
                .map(pg_row_to_device)
                .filter(|d| !d.is_expired())
                .collect()
        }

        fn revoke_by_id(&self, id: &str) -> bool {
            let Ok(mut c) = self.client.lock() else {
                return false;
            };
            c.execute(&format!("DELETE FROM {PG_TABLE} WHERE id = $1"), &[&id])
                .map(|n| n > 0)
                .unwrap_or(false)
        }

        fn revoke_all_for_user(&self, user_id: &str) -> usize {
            let Ok(mut c) = self.client.lock() else {
                return 0;
            };
            c.execute(
                &format!("DELETE FROM {PG_TABLE} WHERE user_id = $1"),
                &[&user_id],
            )
            .map(|n| n as usize)
            .unwrap_or(0)
        }
    }

    fn pg_row_to_device(row: &postgres::Row) -> TrustedDevice {
        TrustedDevice {
            token: row.get(0),
            id: row.get(1),
            user_id: row.get(2),
            label: row.get(3),
            created_at: row.get(4),
            expires_at: row.get(5),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> SqliteTrustedDeviceBackend {
        SqliteTrustedDeviceBackend::in_memory().unwrap()
    }

    #[test]
    fn round_trip_create_find_revoke() {
        let s = store();
        let d = TrustedDevice::mint("u1", Some("Chrome on macOS".into()), 3600);
        let token = d.token.clone();
        let id = d.id.clone();
        s.create(d);
        let got = s.find(&token).expect("must round-trip");
        assert_eq!(got.user_id, "u1");
        assert_eq!(got.label.as_deref(), Some("Chrome on macOS"));
        assert_eq!(got.id, id);
        assert!(s.revoke_by_id(&id));
        assert!(s.find(&token).is_none(), "revoke removes token");
        assert!(s.find_by_id(&id).is_none(), "revoke removes id");
    }

    #[test]
    fn find_by_id_returns_record() {
        let s = store();
        let d = TrustedDevice::mint("u1", None, 60);
        let id = d.id.clone();
        s.create(d);
        let got = s.find_by_id(&id).expect("by_id should resolve");
        assert_eq!(got.user_id, "u1");
    }

    #[test]
    fn find_returns_none_for_expired() {
        let s = store();
        let d = TrustedDevice::mint("u1", None, 0);
        let token = d.token.clone();
        s.create(d);
        assert!(s.find(&token).is_none(), "expired row must not resolve");
    }

    #[test]
    fn list_for_user_isolates_per_user() {
        let s = store();
        s.create(TrustedDevice::mint("u1", None, 60));
        s.create(TrustedDevice::mint("u1", None, 60));
        s.create(TrustedDevice::mint("u2", None, 60));
        assert_eq!(s.list_for_user("u1").len(), 2);
        assert_eq!(s.list_for_user("u2").len(), 1);
        assert_eq!(s.list_for_user("u3").len(), 0);
    }

    #[test]
    fn revoke_all_for_user_scoped() {
        let s = store();
        s.create(TrustedDevice::mint("u1", None, 60));
        s.create(TrustedDevice::mint("u1", None, 60));
        s.create(TrustedDevice::mint("u2", None, 60));
        assert_eq!(s.revoke_all_for_user("u1"), 2);
        assert_eq!(s.list_for_user("u1").len(), 0);
        assert_eq!(s.list_for_user("u2").len(), 1);
    }

    #[test]
    fn create_prunes_expired() {
        let s = store();
        s.create(TrustedDevice::mint("u1", None, 0));
        // The next create triggers the DELETE expired query.
        let alive = TrustedDevice::mint("u1", None, 3600);
        let token = alive.token.clone();
        s.create(alive);
        assert_eq!(s.list_for_user("u1").len(), 1, "expired row should be gone");
        assert!(s.find(&token).is_some(), "alive row should survive");
    }
}
