//! Persistent organization stores. SQLite + Postgres backends behind
//! the [`pylon_auth::org::OrgBackend`] trait so orgs / memberships /
//! invites survive a server restart.
//!
//! Schema: three tables — orgs (id, name, created_by, created_at),
//! org_memberships (org_id, user_id, role, joined_at) with composite
//! PK, and org_invites (id, org_id, email, role, invited_by,
//! token_hash, token_prefix, created_at, expires_at, accepted_at).
//! `token_prefix` is indexed so accept-by-token is fast.

use std::sync::{Arc, Mutex};

use pylon_auth::org::{Invite, Membership, Org, OrgBackend, OrgRole};
use rusqlite::Connection;

const ORGS_TABLE: &str = "_pylon_orgs";
const MEMBERS_TABLE: &str = "_pylon_org_members";
const INVITES_TABLE: &str = "_pylon_org_invites";

// ---------------------------------------------------------------------------
// SQLite backend
// ---------------------------------------------------------------------------

pub struct SqliteOrgBackend {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteOrgBackend {
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
            "CREATE TABLE IF NOT EXISTS {ORGS_TABLE} (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                created_by TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS {MEMBERS_TABLE} (
                org_id TEXT NOT NULL,
                user_id TEXT NOT NULL,
                role TEXT NOT NULL,
                joined_at INTEGER NOT NULL,
                PRIMARY KEY (org_id, user_id)
            );
            CREATE INDEX IF NOT EXISTS {MEMBERS_TABLE}_user_idx ON {MEMBERS_TABLE}(user_id);
            CREATE TABLE IF NOT EXISTS {INVITES_TABLE} (
                id TEXT PRIMARY KEY,
                org_id TEXT NOT NULL,
                email TEXT NOT NULL,
                role TEXT NOT NULL,
                invited_by TEXT NOT NULL,
                token_hash TEXT NOT NULL,
                token_prefix TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                expires_at INTEGER NOT NULL,
                accepted_at INTEGER
            );
            CREATE INDEX IF NOT EXISTS {INVITES_TABLE}_prefix_idx ON {INVITES_TABLE}(token_prefix);
            CREATE INDEX IF NOT EXISTS {INVITES_TABLE}_org_idx ON {INVITES_TABLE}(org_id);"
        ))
        .map_err(|e| format!("init schema: {e}"))?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }
}

fn role_to_str(r: OrgRole) -> &'static str {
    r.as_str()
}

fn role_from_str(s: &str) -> OrgRole {
    OrgRole::from_str(s).unwrap_or(OrgRole::Member)
}

impl OrgBackend for SqliteOrgBackend {
    fn put_org(&self, org: &Org) {
        if let Ok(c) = self.conn.lock() {
            let _ = c.execute(
                &format!(
                    "INSERT INTO {ORGS_TABLE} (id, name, created_by, created_at)
                     VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(id) DO UPDATE SET
                       name = excluded.name"
                ),
                rusqlite::params![org.id, org.name, org.created_by, org.created_at as i64],
            );
        }
    }

    fn get_org(&self, id: &str) -> Option<Org> {
        let c = self.conn.lock().ok()?;
        c.query_row(
            &format!("SELECT id, name, created_by, created_at FROM {ORGS_TABLE} WHERE id = ?1"),
            rusqlite::params![id],
            |r| {
                Ok(Org {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    created_by: r.get(2)?,
                    created_at: r.get::<_, i64>(3)? as u64,
                })
            },
        )
        .ok()
    }

    fn delete_org(&self, id: &str) -> bool {
        let Ok(c) = self.conn.lock() else {
            return false;
        };
        // Cascade memberships + invites — host schema doesn't have FKs
        // since pylon owns these tables.
        let _ = c.execute(
            &format!("DELETE FROM {MEMBERS_TABLE} WHERE org_id = ?1"),
            rusqlite::params![id],
        );
        let _ = c.execute(
            &format!("DELETE FROM {INVITES_TABLE} WHERE org_id = ?1"),
            rusqlite::params![id],
        );
        c.execute(
            &format!("DELETE FROM {ORGS_TABLE} WHERE id = ?1"),
            rusqlite::params![id],
        )
        .map(|n| n > 0)
        .unwrap_or(false)
    }

    fn list_orgs_for_user(&self, user_id: &str) -> Vec<(Org, OrgRole)> {
        let Ok(c) = self.conn.lock() else {
            return vec![];
        };
        let mut stmt = match c.prepare(&format!(
            "SELECT o.id, o.name, o.created_by, o.created_at, m.role
             FROM {ORGS_TABLE} o JOIN {MEMBERS_TABLE} m ON o.id = m.org_id
             WHERE m.user_id = ?1
             ORDER BY o.created_at DESC"
        )) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        let iter = match stmt.query_map(rusqlite::params![user_id], |r| {
            let role: String = r.get(4)?;
            Ok((
                Org {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    created_by: r.get(2)?,
                    created_at: r.get::<_, i64>(3)? as u64,
                },
                role_from_str(&role),
            ))
        }) {
            Ok(it) => it,
            Err(_) => return vec![],
        };
        iter.filter_map(|r| r.ok()).collect()
    }

    fn put_membership(&self, m: &Membership) {
        if let Ok(c) = self.conn.lock() {
            let _ = c.execute(
                &format!(
                    "INSERT INTO {MEMBERS_TABLE} (org_id, user_id, role, joined_at)
                     VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(org_id, user_id) DO UPDATE SET role = excluded.role"
                ),
                rusqlite::params![m.org_id, m.user_id, role_to_str(m.role), m.joined_at as i64],
            );
        }
    }

    fn get_membership(&self, org_id: &str, user_id: &str) -> Option<Membership> {
        let c = self.conn.lock().ok()?;
        c.query_row(
            &format!(
                "SELECT org_id, user_id, role, joined_at FROM {MEMBERS_TABLE}
                 WHERE org_id = ?1 AND user_id = ?2"
            ),
            rusqlite::params![org_id, user_id],
            |r| {
                let role: String = r.get(2)?;
                Ok(Membership {
                    org_id: r.get(0)?,
                    user_id: r.get(1)?,
                    role: role_from_str(&role),
                    joined_at: r.get::<_, i64>(3)? as u64,
                })
            },
        )
        .ok()
    }

    fn delete_membership(&self, org_id: &str, user_id: &str) -> bool {
        let Ok(c) = self.conn.lock() else {
            return false;
        };
        c.execute(
            &format!("DELETE FROM {MEMBERS_TABLE} WHERE org_id = ?1 AND user_id = ?2"),
            rusqlite::params![org_id, user_id],
        )
        .map(|n| n > 0)
        .unwrap_or(false)
    }

    fn list_members(&self, org_id: &str) -> Vec<Membership> {
        let Ok(c) = self.conn.lock() else {
            return vec![];
        };
        let mut stmt = match c.prepare(&format!(
            "SELECT org_id, user_id, role, joined_at FROM {MEMBERS_TABLE}
             WHERE org_id = ?1 ORDER BY joined_at"
        )) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        let iter = match stmt.query_map(rusqlite::params![org_id], |r| {
            let role: String = r.get(2)?;
            Ok(Membership {
                org_id: r.get(0)?,
                user_id: r.get(1)?,
                role: role_from_str(&role),
                joined_at: r.get::<_, i64>(3)? as u64,
            })
        }) {
            Ok(it) => it,
            Err(_) => return vec![],
        };
        iter.filter_map(|r| r.ok()).collect()
    }

    fn put_invite(&self, inv: &Invite) {
        if let Ok(c) = self.conn.lock() {
            let _ = c.execute(
                &format!(
                    "INSERT INTO {INVITES_TABLE}
                       (id, org_id, email, role, invited_by, token_hash, token_prefix,
                        created_at, expires_at, accepted_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                     ON CONFLICT(id) DO UPDATE SET
                       accepted_at = excluded.accepted_at"
                ),
                rusqlite::params![
                    inv.id,
                    inv.org_id,
                    inv.email,
                    role_to_str(inv.role),
                    inv.invited_by,
                    inv.token_hash,
                    inv.token_prefix,
                    inv.created_at as i64,
                    inv.expires_at as i64,
                    inv.accepted_at.map(|v| v as i64),
                ],
            );
        }
    }

    fn get_invite(&self, id: &str) -> Option<Invite> {
        let c = self.conn.lock().ok()?;
        c.query_row(
            &format!(
                "SELECT id, org_id, email, role, invited_by, token_hash, token_prefix,
                        created_at, expires_at, accepted_at
                 FROM {INVITES_TABLE} WHERE id = ?1"
            ),
            rusqlite::params![id],
            row_to_invite,
        )
        .ok()
    }

    fn list_invites(&self, org_id: &str) -> Vec<Invite> {
        let Ok(c) = self.conn.lock() else {
            return vec![];
        };
        let mut stmt = match c.prepare(&format!(
            "SELECT id, org_id, email, role, invited_by, token_hash, token_prefix,
                    created_at, expires_at, accepted_at
             FROM {INVITES_TABLE}
             WHERE org_id = ?1 AND accepted_at IS NULL
             ORDER BY created_at DESC"
        )) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        let iter = match stmt.query_map(rusqlite::params![org_id], row_to_invite) {
            Ok(it) => it,
            Err(_) => return vec![],
        };
        iter.filter_map(|r| r.ok()).collect()
    }

    fn delete_invite(&self, id: &str) -> bool {
        let Ok(c) = self.conn.lock() else {
            return false;
        };
        c.execute(
            &format!("DELETE FROM {INVITES_TABLE} WHERE id = ?1"),
            rusqlite::params![id],
        )
        .map(|n| n > 0)
        .unwrap_or(false)
    }

    fn invites_by_prefix(&self, prefix: &str) -> Vec<Invite> {
        let Ok(c) = self.conn.lock() else {
            return vec![];
        };
        // Include accepted invites — accept_invite returns
        // AlreadyAccepted by checking the field, not by their absence.
        let mut stmt = match c.prepare(&format!(
            "SELECT id, org_id, email, role, invited_by, token_hash, token_prefix,
                    created_at, expires_at, accepted_at
             FROM {INVITES_TABLE} WHERE token_prefix = ?1"
        )) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        let iter = match stmt.query_map(rusqlite::params![prefix], row_to_invite) {
            Ok(it) => it,
            Err(_) => return vec![],
        };
        iter.filter_map(|r| r.ok()).collect()
    }

    fn mark_invite_accepted(&self, id: &str, now: u64) -> bool {
        let Ok(c) = self.conn.lock() else {
            return false;
        };
        // CAS via SQL: only stamp when accepted_at IS NULL. The
        // affected-row count tells us whether we won the race.
        c.execute(
            &format!(
                "UPDATE {INVITES_TABLE} SET accepted_at = ?2
                 WHERE id = ?1 AND accepted_at IS NULL"
            ),
            rusqlite::params![id, now as i64],
        )
        .map(|n| n > 0)
        .unwrap_or(false)
    }
}

fn row_to_invite(row: &rusqlite::Row<'_>) -> rusqlite::Result<Invite> {
    let role: String = row.get(3)?;
    Ok(Invite {
        id: row.get(0)?,
        org_id: row.get(1)?,
        email: row.get(2)?,
        role: role_from_str(&role),
        invited_by: row.get(4)?,
        token_hash: row.get(5)?,
        token_prefix: row.get(6)?,
        created_at: row.get::<_, i64>(7)? as u64,
        expires_at: row.get::<_, i64>(8)? as u64,
        accepted_at: row.get::<_, Option<i64>>(9)?.map(|v| v as u64),
    })
}

// ---------------------------------------------------------------------------
// Postgres backend
// ---------------------------------------------------------------------------

pub use pg::PostgresOrgBackend;

mod pg {
    use super::*;
    use postgres::Client;

    pub struct PostgresOrgBackend {
        client: Mutex<Client>,
    }

    impl PostgresOrgBackend {
        pub fn connect(url: &str) -> Result<Self, String> {
            let mut client = pylon_storage::postgres::live::connect_pg(url)?;
            client
                .batch_execute(&format!(
                    "CREATE TABLE IF NOT EXISTS {ORGS_TABLE} (
                        id TEXT PRIMARY KEY,
                        name TEXT NOT NULL,
                        created_by TEXT NOT NULL,
                        created_at BIGINT NOT NULL
                    );
                    CREATE TABLE IF NOT EXISTS {MEMBERS_TABLE} (
                        org_id TEXT NOT NULL,
                        user_id TEXT NOT NULL,
                        role TEXT NOT NULL,
                        joined_at BIGINT NOT NULL,
                        PRIMARY KEY (org_id, user_id)
                    );
                    CREATE INDEX IF NOT EXISTS {MEMBERS_TABLE}_user_idx ON {MEMBERS_TABLE}(user_id);
                    CREATE TABLE IF NOT EXISTS {INVITES_TABLE} (
                        id TEXT PRIMARY KEY,
                        org_id TEXT NOT NULL,
                        email TEXT NOT NULL,
                        role TEXT NOT NULL,
                        invited_by TEXT NOT NULL,
                        token_hash TEXT NOT NULL,
                        token_prefix TEXT NOT NULL,
                        created_at BIGINT NOT NULL,
                        expires_at BIGINT NOT NULL,
                        accepted_at BIGINT
                    );
                    CREATE INDEX IF NOT EXISTS {INVITES_TABLE}_prefix_idx ON {INVITES_TABLE}(token_prefix);
                    CREATE INDEX IF NOT EXISTS {INVITES_TABLE}_org_idx ON {INVITES_TABLE}(org_id);"
                ))
                .map_err(|e| format!("PG init schema: {e}"))?;
            Ok(Self {
                client: Mutex::new(client),
            })
        }
    }

    impl OrgBackend for PostgresOrgBackend {
        fn put_org(&self, org: &Org) {
            if let Ok(mut c) = self.client.lock() {
                let _ = c.execute(
                    &format!(
                        "INSERT INTO {ORGS_TABLE} (id, name, created_by, created_at)
                         VALUES ($1, $2, $3, $4)
                         ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name"
                    ),
                    &[&org.id, &org.name, &org.created_by, &(org.created_at as i64)],
                );
            }
        }

        fn get_org(&self, id: &str) -> Option<Org> {
            let mut c = self.client.lock().ok()?;
            let row = c
                .query_opt(
                    &format!(
                        "SELECT id, name, created_by, created_at FROM {ORGS_TABLE} WHERE id = $1"
                    ),
                    &[&id],
                )
                .ok()??;
            Some(Org {
                id: row.get(0),
                name: row.get(1),
                created_by: row.get(2),
                created_at: row.get::<_, i64>(3) as u64,
            })
        }

        fn delete_org(&self, id: &str) -> bool {
            let Ok(mut c) = self.client.lock() else {
                return false;
            };
            let _ = c.execute(
                &format!("DELETE FROM {MEMBERS_TABLE} WHERE org_id = $1"),
                &[&id],
            );
            let _ = c.execute(
                &format!("DELETE FROM {INVITES_TABLE} WHERE org_id = $1"),
                &[&id],
            );
            c.execute(
                &format!("DELETE FROM {ORGS_TABLE} WHERE id = $1"),
                &[&id],
            )
            .map(|n| n > 0)
            .unwrap_or(false)
        }

        fn list_orgs_for_user(&self, user_id: &str) -> Vec<(Org, OrgRole)> {
            let Ok(mut c) = self.client.lock() else {
                return vec![];
            };
            let rows = match c.query(
                &format!(
                    "SELECT o.id, o.name, o.created_by, o.created_at, m.role
                     FROM {ORGS_TABLE} o JOIN {MEMBERS_TABLE} m ON o.id = m.org_id
                     WHERE m.user_id = $1
                     ORDER BY o.created_at DESC"
                ),
                &[&user_id],
            ) {
                Ok(r) => r,
                Err(_) => return vec![],
            };
            rows.iter()
                .map(|row| {
                    let role: String = row.get(4);
                    (
                        Org {
                            id: row.get(0),
                            name: row.get(1),
                            created_by: row.get(2),
                            created_at: row.get::<_, i64>(3) as u64,
                        },
                        role_from_str(&role),
                    )
                })
                .collect()
        }

        fn put_membership(&self, m: &Membership) {
            if let Ok(mut c) = self.client.lock() {
                let _ = c.execute(
                    &format!(
                        "INSERT INTO {MEMBERS_TABLE} (org_id, user_id, role, joined_at)
                         VALUES ($1, $2, $3, $4)
                         ON CONFLICT (org_id, user_id) DO UPDATE SET role = EXCLUDED.role"
                    ),
                    &[&m.org_id, &m.user_id, &role_to_str(m.role), &(m.joined_at as i64)],
                );
            }
        }

        fn get_membership(&self, org_id: &str, user_id: &str) -> Option<Membership> {
            let mut c = self.client.lock().ok()?;
            let row = c
                .query_opt(
                    &format!(
                        "SELECT org_id, user_id, role, joined_at FROM {MEMBERS_TABLE}
                         WHERE org_id = $1 AND user_id = $2"
                    ),
                    &[&org_id, &user_id],
                )
                .ok()??;
            let role: String = row.get(2);
            Some(Membership {
                org_id: row.get(0),
                user_id: row.get(1),
                role: role_from_str(&role),
                joined_at: row.get::<_, i64>(3) as u64,
            })
        }

        fn delete_membership(&self, org_id: &str, user_id: &str) -> bool {
            let Ok(mut c) = self.client.lock() else {
                return false;
            };
            c.execute(
                &format!("DELETE FROM {MEMBERS_TABLE} WHERE org_id = $1 AND user_id = $2"),
                &[&org_id, &user_id],
            )
            .map(|n| n > 0)
            .unwrap_or(false)
        }

        fn list_members(&self, org_id: &str) -> Vec<Membership> {
            let Ok(mut c) = self.client.lock() else {
                return vec![];
            };
            let rows = match c.query(
                &format!(
                    "SELECT org_id, user_id, role, joined_at FROM {MEMBERS_TABLE}
                     WHERE org_id = $1 ORDER BY joined_at"
                ),
                &[&org_id],
            ) {
                Ok(r) => r,
                Err(_) => return vec![],
            };
            rows.iter()
                .map(|row| {
                    let role: String = row.get(2);
                    Membership {
                        org_id: row.get(0),
                        user_id: row.get(1),
                        role: role_from_str(&role),
                        joined_at: row.get::<_, i64>(3) as u64,
                    }
                })
                .collect()
        }

        fn put_invite(&self, inv: &Invite) {
            if let Ok(mut c) = self.client.lock() {
                let _ = c.execute(
                    &format!(
                        "INSERT INTO {INVITES_TABLE}
                           (id, org_id, email, role, invited_by, token_hash, token_prefix,
                            created_at, expires_at, accepted_at)
                         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
                         ON CONFLICT (id) DO UPDATE SET accepted_at = EXCLUDED.accepted_at"
                    ),
                    &[
                        &inv.id,
                        &inv.org_id,
                        &inv.email,
                        &role_to_str(inv.role),
                        &inv.invited_by,
                        &inv.token_hash,
                        &inv.token_prefix,
                        &(inv.created_at as i64),
                        &(inv.expires_at as i64),
                        &inv.accepted_at.map(|v| v as i64),
                    ],
                );
            }
        }

        fn get_invite(&self, id: &str) -> Option<Invite> {
            let mut c = self.client.lock().ok()?;
            let row = c
                .query_opt(
                    &format!(
                        "SELECT id, org_id, email, role, invited_by, token_hash, token_prefix,
                                created_at, expires_at, accepted_at
                         FROM {INVITES_TABLE} WHERE id = $1"
                    ),
                    &[&id],
                )
                .ok()??;
            Some(pg_row_to_invite(&row))
        }

        fn list_invites(&self, org_id: &str) -> Vec<Invite> {
            let Ok(mut c) = self.client.lock() else {
                return vec![];
            };
            let rows = match c.query(
                &format!(
                    "SELECT id, org_id, email, role, invited_by, token_hash, token_prefix,
                            created_at, expires_at, accepted_at
                     FROM {INVITES_TABLE}
                     WHERE org_id = $1 AND accepted_at IS NULL
                     ORDER BY created_at DESC"
                ),
                &[&org_id],
            ) {
                Ok(r) => r,
                Err(_) => return vec![],
            };
            rows.iter().map(pg_row_to_invite).collect()
        }

        fn delete_invite(&self, id: &str) -> bool {
            let Ok(mut c) = self.client.lock() else {
                return false;
            };
            c.execute(
                &format!("DELETE FROM {INVITES_TABLE} WHERE id = $1"),
                &[&id],
            )
            .map(|n| n > 0)
            .unwrap_or(false)
        }

        fn invites_by_prefix(&self, prefix: &str) -> Vec<Invite> {
            let Ok(mut c) = self.client.lock() else {
                return vec![];
            };
            let rows = match c.query(
                &format!(
                    "SELECT id, org_id, email, role, invited_by, token_hash, token_prefix,
                            created_at, expires_at, accepted_at
                     FROM {INVITES_TABLE} WHERE token_prefix = $1"
                ),
                &[&prefix],
            ) {
                Ok(r) => r,
                Err(_) => return vec![],
            };
            rows.iter().map(pg_row_to_invite).collect()
        }

        fn mark_invite_accepted(&self, id: &str, now: u64) -> bool {
            let Ok(mut c) = self.client.lock() else {
                return false;
            };
            c.execute(
                &format!(
                    "UPDATE {INVITES_TABLE} SET accepted_at = $2
                     WHERE id = $1 AND accepted_at IS NULL"
                ),
                &[&id, &(now as i64)],
            )
            .map(|n| n > 0)
            .unwrap_or(false)
        }
    }

    fn pg_row_to_invite(row: &postgres::Row) -> Invite {
        let role: String = row.get(3);
        Invite {
            id: row.get(0),
            org_id: row.get(1),
            email: row.get(2),
            role: role_from_str(&role),
            invited_by: row.get(4),
            token_hash: row.get(5),
            token_prefix: row.get(6),
            created_at: row.get::<_, i64>(7) as u64,
            expires_at: row.get::<_, i64>(8) as u64,
            accepted_at: row.get::<_, Option<i64>>(9).map(|v| v as u64),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pylon_auth::org::{Invite, Membership, Org, OrgRole};

    #[test]
    fn sqlite_org_round_trip() {
        let b = SqliteOrgBackend::in_memory().unwrap();
        let org = Org {
            id: "o1".into(),
            name: "Acme".into(),
            created_by: "u1".into(),
            created_at: 100,
        };
        b.put_org(&org);
        assert_eq!(b.get_org("o1").unwrap().name, "Acme");
        assert!(b.delete_org("o1"));
        assert!(b.get_org("o1").is_none());
    }

    #[test]
    fn sqlite_membership_and_list_for_user() {
        let b = SqliteOrgBackend::in_memory().unwrap();
        b.put_org(&Org {
            id: "o1".into(),
            name: "A".into(),
            created_by: "u1".into(),
            created_at: 100,
        });
        b.put_org(&Org {
            id: "o2".into(),
            name: "B".into(),
            created_by: "u2".into(),
            created_at: 200,
        });
        b.put_membership(&Membership {
            org_id: "o1".into(),
            user_id: "u1".into(),
            role: OrgRole::Owner,
            joined_at: 100,
        });
        b.put_membership(&Membership {
            org_id: "o2".into(),
            user_id: "u1".into(),
            role: OrgRole::Member,
            joined_at: 200,
        });
        let list = b.list_orgs_for_user("u1");
        assert_eq!(list.len(), 2);
        // Newest first.
        assert_eq!(list[0].0.id, "o2");
        assert_eq!(list[0].1, OrgRole::Member);
    }

    #[test]
    fn sqlite_invite_prefix_index_used() {
        let b = SqliteOrgBackend::in_memory().unwrap();
        b.put_org(&Org {
            id: "o1".into(),
            name: "A".into(),
            created_by: "u1".into(),
            created_at: 100,
        });
        b.put_invite(&Invite {
            id: "i1".into(),
            org_id: "o1".into(),
            email: "x@y.com".into(),
            role: OrgRole::Member,
            invited_by: "u1".into(),
            token_hash: "h".into(),
            token_prefix: "abcd1234".into(),
            created_at: 100,
            expires_at: 9_999_999_999,
            accepted_at: None,
        });
        let hits = b.invites_by_prefix("abcd1234");
        assert_eq!(hits.len(), 1);
        let misses = b.invites_by_prefix("nomatch1");
        assert_eq!(misses.len(), 0);
    }

    #[test]
    fn sqlite_delete_org_cascades() {
        let b = SqliteOrgBackend::in_memory().unwrap();
        b.put_org(&Org {
            id: "o1".into(),
            name: "A".into(),
            created_by: "u1".into(),
            created_at: 100,
        });
        b.put_membership(&Membership {
            org_id: "o1".into(),
            user_id: "u1".into(),
            role: OrgRole::Owner,
            joined_at: 100,
        });
        b.put_invite(&Invite {
            id: "i1".into(),
            org_id: "o1".into(),
            email: "x@y.com".into(),
            role: OrgRole::Member,
            invited_by: "u1".into(),
            token_hash: "h".into(),
            token_prefix: "p".into(),
            created_at: 100,
            expires_at: 9_999_999_999,
            accepted_at: None,
        });
        assert!(b.delete_org("o1"));
        assert!(b.list_members("o1").is_empty());
        assert!(b.list_invites("o1").is_empty());
    }
}
