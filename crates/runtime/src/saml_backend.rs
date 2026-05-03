//! SQLite + Postgres backends for `pylon_auth::saml::SamlStore`.
//! Two tables per backend, mirroring the org_sso layout:
//!
//! - `_pylon_org_saml` — one row per org with the IdP config + cached
//!   email-domain index (stored as JSON text).
//! - `_pylon_org_saml_state` — short-lived AuthnRequest state tokens
//!   (10-minute TTL, GC'd opportunistically on every save).

use std::sync::{Arc, Mutex};

use pylon_auth::org_sso::DomainConflictError;
use pylon_auth::saml::{SamlConfig, SamlStateRecord, SamlStore, SAML_STATE_TTL_SECS};
use rusqlite::Connection;

const SQLITE_CONFIG: &str = "_pylon_org_saml";
const SQLITE_STATE: &str = "_pylon_org_saml_state";
const PG_CONFIG: &str = "_pylon_org_saml";
const PG_STATE: &str = "_pylon_org_saml_state";

// ---------------------------------------------------------------------------
// SQLite
// ---------------------------------------------------------------------------

pub struct SqliteSamlBackend {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteSamlBackend {
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
            "CREATE TABLE IF NOT EXISTS {SQLITE_CONFIG} (
                org_id TEXT PRIMARY KEY,
                idp_entity_id TEXT NOT NULL,
                idp_sso_url TEXT NOT NULL,
                idp_x509_cert_pem TEXT NOT NULL,
                default_role TEXT NOT NULL,
                email_domains_json TEXT NOT NULL DEFAULT '[]',
                email_attribute TEXT NOT NULL,
                name_attribute TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS {SQLITE_STATE} (
                relay_state TEXT PRIMARY KEY,
                org_id TEXT NOT NULL,
                request_id TEXT NOT NULL,
                callback_url TEXT NOT NULL,
                error_callback_url TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS {SQLITE_STATE}_created_idx
                ON {SQLITE_STATE}(created_at);"
        ))
        .map_err(|e| format!("init schema: {e}"))?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }
}

impl SamlStore for SqliteSamlBackend {
    fn get(&self, org_id: &str) -> Option<SamlConfig> {
        let c = self.conn.lock().ok()?;
        let mut stmt = c
            .prepare(&format!(
                "SELECT org_id, idp_entity_id, idp_sso_url, idp_x509_cert_pem,
                        default_role, email_domains_json, email_attribute,
                        name_attribute, created_at, updated_at
                 FROM {SQLITE_CONFIG} WHERE org_id = ?1"
            ))
            .ok()?;
        stmt.query_row(rusqlite::params![org_id], row_to_config)
            .ok()
    }

    fn upsert(&self, config: SamlConfig) -> Result<(), DomainConflictError> {
        let c = self.conn.lock().map_err(|_| DomainConflictError {
            domain: String::new(),
            claimed_by: String::new(),
        })?;
        // Conflict check: any other org claiming a requested domain
        // wins iff it was there first; we reject this upsert.
        let mut stmt = c
            .prepare(&format!(
                "SELECT org_id, email_domains_json FROM {SQLITE_CONFIG} WHERE org_id != ?1"
            ))
            .map_err(|_| DomainConflictError {
                domain: String::new(),
                claimed_by: String::new(),
            })?;
        let rows = stmt
            .query_map(rusqlite::params![config.org_id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(|_| DomainConflictError {
                domain: String::new(),
                claimed_by: String::new(),
            })?;
        let lower_requested: Vec<String> = config
            .email_domains
            .iter()
            .map(|d| d.to_ascii_lowercase())
            .collect();
        for row in rows.flatten() {
            let other: Vec<String> = serde_json::from_str(&row.1).unwrap_or_default();
            for od in other.iter().map(|s| s.to_ascii_lowercase()) {
                if lower_requested.contains(&od) {
                    return Err(DomainConflictError {
                        domain: od,
                        claimed_by: row.0,
                    });
                }
            }
        }
        drop(stmt);
        let domains = serde_json::to_string(&config.email_domains).unwrap_or_else(|_| "[]".into());
        let _ = c.execute(
            &format!(
                "INSERT INTO {SQLITE_CONFIG}
                   (org_id, idp_entity_id, idp_sso_url, idp_x509_cert_pem,
                    default_role, email_domains_json, email_attribute,
                    name_attribute, created_at, updated_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
                 ON CONFLICT(org_id) DO UPDATE SET
                   idp_entity_id = excluded.idp_entity_id,
                   idp_sso_url = excluded.idp_sso_url,
                   idp_x509_cert_pem = excluded.idp_x509_cert_pem,
                   default_role = excluded.default_role,
                   email_domains_json = excluded.email_domains_json,
                   email_attribute = excluded.email_attribute,
                   name_attribute = excluded.name_attribute,
                   updated_at = excluded.updated_at"
            ),
            rusqlite::params![
                config.org_id,
                config.idp_entity_id,
                config.idp_sso_url,
                config.idp_x509_cert_pem,
                config.default_role,
                domains,
                config.email_attribute,
                config.name_attribute,
                config.created_at as i64,
                config.updated_at as i64,
            ],
        );
        Ok(())
    }

    fn delete(&self, org_id: &str) -> bool {
        let Ok(c) = self.conn.lock() else {
            return false;
        };
        c.execute(
            &format!("DELETE FROM {SQLITE_CONFIG} WHERE org_id = ?1"),
            rusqlite::params![org_id],
        )
        .map(|n| n > 0)
        .unwrap_or(false)
    }

    fn find_by_email_domain(&self, domain: &str) -> Option<String> {
        let c = self.conn.lock().ok()?;
        let lower = domain.to_ascii_lowercase();
        let mut stmt = c
            .prepare(&format!(
                "SELECT org_id, email_domains_json FROM {SQLITE_CONFIG}"
            ))
            .ok()?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .ok()?;
        for row in rows.flatten() {
            let domains: Vec<String> = serde_json::from_str(&row.1).unwrap_or_default();
            if domains.iter().any(|d| d.eq_ignore_ascii_case(&lower)) {
                return Some(row.0);
            }
        }
        None
    }

    fn save_state(&self, record: SamlStateRecord) {
        if let Ok(c) = self.conn.lock() {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            let _ = c.execute(
                &format!("DELETE FROM {SQLITE_STATE} WHERE created_at < ?1"),
                rusqlite::params![now - SAML_STATE_TTL_SECS as i64],
            );
            let _ = c.execute(
                &format!(
                    "INSERT INTO {SQLITE_STATE}
                       (relay_state, org_id, request_id, callback_url, error_callback_url, created_at)
                     VALUES (?1,?2,?3,?4,?5,?6)
                     ON CONFLICT(relay_state) DO NOTHING"
                ),
                rusqlite::params![
                    record.relay_state,
                    record.org_id,
                    record.request_id,
                    record.callback_url,
                    record.error_callback_url,
                    record.created_at as i64,
                ],
            );
        }
    }

    fn take_state(&self, relay_state: &str, expected_org_id: &str) -> Option<SamlStateRecord> {
        let c = self.conn.lock().ok()?;
        let mut stmt = c
            .prepare(&format!(
                "SELECT relay_state, org_id, request_id, callback_url, error_callback_url, created_at
                 FROM {SQLITE_STATE} WHERE relay_state = ?1"
            ))
            .ok()?;
        let record: SamlStateRecord = stmt
            .query_row(rusqlite::params![relay_state], state_row_to_record)
            .ok()?;
        if record.org_id != expected_org_id {
            return None;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if now.saturating_sub(record.created_at) >= SAML_STATE_TTL_SECS {
            let _ = c.execute(
                &format!("DELETE FROM {SQLITE_STATE} WHERE relay_state = ?1"),
                rusqlite::params![relay_state],
            );
            return None;
        }
        let _ = c.execute(
            &format!("DELETE FROM {SQLITE_STATE} WHERE relay_state = ?1"),
            rusqlite::params![relay_state],
        );
        Some(record)
    }
}

fn row_to_config(row: &rusqlite::Row<'_>) -> rusqlite::Result<SamlConfig> {
    let domains_json: String = row.get(5)?;
    let email_domains: Vec<String> = serde_json::from_str(&domains_json).unwrap_or_default();
    Ok(SamlConfig {
        org_id: row.get(0)?,
        idp_entity_id: row.get(1)?,
        idp_sso_url: row.get(2)?,
        idp_x509_cert_pem: row.get(3)?,
        default_role: row.get(4)?,
        email_domains,
        email_attribute: row.get(6)?,
        name_attribute: row.get(7)?,
        created_at: row.get::<_, i64>(8)? as u64,
        updated_at: row.get::<_, i64>(9)? as u64,
    })
}

fn state_row_to_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<SamlStateRecord> {
    Ok(SamlStateRecord {
        relay_state: row.get(0)?,
        org_id: row.get(1)?,
        request_id: row.get(2)?,
        callback_url: row.get(3)?,
        error_callback_url: row.get(4)?,
        created_at: row.get::<_, i64>(5)? as u64,
    })
}

// ---------------------------------------------------------------------------
// Postgres
// ---------------------------------------------------------------------------

pub use pg::PostgresSamlBackend;

mod pg {
    use super::*;
    use postgres::Client;

    pub struct PostgresSamlBackend {
        client: Mutex<Client>,
    }

    impl PostgresSamlBackend {
        pub fn connect(url: &str) -> Result<Self, String> {
            let mut client = pylon_storage::postgres::live::connect_pg(url)?;
            client
                .batch_execute(&format!(
                    "CREATE TABLE IF NOT EXISTS {PG_CONFIG} (
                        org_id TEXT PRIMARY KEY,
                        idp_entity_id TEXT NOT NULL,
                        idp_sso_url TEXT NOT NULL,
                        idp_x509_cert_pem TEXT NOT NULL,
                        default_role TEXT NOT NULL,
                        email_domains_json TEXT NOT NULL DEFAULT '[]',
                        email_attribute TEXT NOT NULL,
                        name_attribute TEXT,
                        created_at BIGINT NOT NULL,
                        updated_at BIGINT NOT NULL
                    );
                    CREATE TABLE IF NOT EXISTS {PG_STATE} (
                        relay_state TEXT PRIMARY KEY,
                        org_id TEXT NOT NULL,
                        request_id TEXT NOT NULL,
                        callback_url TEXT NOT NULL,
                        error_callback_url TEXT NOT NULL,
                        created_at BIGINT NOT NULL
                    );
                    CREATE INDEX IF NOT EXISTS {PG_STATE}_created_idx
                        ON {PG_STATE}(created_at);"
                ))
                .map_err(|e| format!("PG init schema: {e}"))?;
            Ok(Self {
                client: Mutex::new(client),
            })
        }
    }

    impl SamlStore for PostgresSamlBackend {
        fn get(&self, org_id: &str) -> Option<SamlConfig> {
            let mut c = self.client.lock().ok()?;
            let row = c
                .query_opt(
                    &format!(
                        "SELECT org_id, idp_entity_id, idp_sso_url, idp_x509_cert_pem,
                                default_role, email_domains_json, email_attribute,
                                name_attribute, created_at, updated_at
                         FROM {PG_CONFIG} WHERE org_id = $1"
                    ),
                    &[&org_id],
                )
                .ok()??;
            Some(pg_row_to_config(&row))
        }

        fn upsert(&self, config: SamlConfig) -> Result<(), DomainConflictError> {
            let mut c = self.client.lock().map_err(|_| DomainConflictError {
                domain: String::new(),
                claimed_by: String::new(),
            })?;
            let rows = c
                .query(
                    &format!(
                        "SELECT org_id, email_domains_json FROM {PG_CONFIG} WHERE org_id != $1"
                    ),
                    &[&config.org_id],
                )
                .map_err(|_| DomainConflictError {
                    domain: String::new(),
                    claimed_by: String::new(),
                })?;
            let lower_requested: Vec<String> = config
                .email_domains
                .iter()
                .map(|d| d.to_ascii_lowercase())
                .collect();
            for r in rows {
                let other_org: String = r.get(0);
                let other_json: String = r.get(1);
                let other: Vec<String> = serde_json::from_str(&other_json).unwrap_or_default();
                for od in other.iter().map(|s| s.to_ascii_lowercase()) {
                    if lower_requested.contains(&od) {
                        return Err(DomainConflictError {
                            domain: od,
                            claimed_by: other_org,
                        });
                    }
                }
            }
            let domains =
                serde_json::to_string(&config.email_domains).unwrap_or_else(|_| "[]".into());
            let _ = c.execute(
                &format!(
                    "INSERT INTO {PG_CONFIG}
                       (org_id, idp_entity_id, idp_sso_url, idp_x509_cert_pem,
                        default_role, email_domains_json, email_attribute,
                        name_attribute, created_at, updated_at)
                     VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)
                     ON CONFLICT(org_id) DO UPDATE SET
                       idp_entity_id = EXCLUDED.idp_entity_id,
                       idp_sso_url = EXCLUDED.idp_sso_url,
                       idp_x509_cert_pem = EXCLUDED.idp_x509_cert_pem,
                       default_role = EXCLUDED.default_role,
                       email_domains_json = EXCLUDED.email_domains_json,
                       email_attribute = EXCLUDED.email_attribute,
                       name_attribute = EXCLUDED.name_attribute,
                       updated_at = EXCLUDED.updated_at"
                ),
                &[
                    &config.org_id,
                    &config.idp_entity_id,
                    &config.idp_sso_url,
                    &config.idp_x509_cert_pem,
                    &config.default_role,
                    &domains,
                    &config.email_attribute,
                    &config.name_attribute,
                    &(config.created_at as i64),
                    &(config.updated_at as i64),
                ],
            );
            Ok(())
        }

        fn delete(&self, org_id: &str) -> bool {
            let Ok(mut c) = self.client.lock() else {
                return false;
            };
            c.execute(
                &format!("DELETE FROM {PG_CONFIG} WHERE org_id = $1"),
                &[&org_id],
            )
            .map(|n| n > 0)
            .unwrap_or(false)
        }

        fn find_by_email_domain(&self, domain: &str) -> Option<String> {
            let mut c = self.client.lock().ok()?;
            let lower = domain.to_ascii_lowercase();
            let rows = c
                .query(
                    &format!("SELECT org_id, email_domains_json FROM {PG_CONFIG}"),
                    &[],
                )
                .ok()?;
            for r in rows {
                let domains_json: String = r.get(1);
                let domains: Vec<String> = serde_json::from_str(&domains_json).unwrap_or_default();
                if domains.iter().any(|d| d.eq_ignore_ascii_case(&lower)) {
                    let oid: String = r.get(0);
                    return Some(oid);
                }
            }
            None
        }

        fn save_state(&self, record: SamlStateRecord) {
            if let Ok(mut c) = self.client.lock() {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                let _ = c.execute(
                    &format!("DELETE FROM {PG_STATE} WHERE created_at < $1"),
                    &[&(now - SAML_STATE_TTL_SECS as i64)],
                );
                let _ = c.execute(
                    &format!(
                        "INSERT INTO {PG_STATE}
                           (relay_state, org_id, request_id, callback_url, error_callback_url, created_at)
                         VALUES ($1,$2,$3,$4,$5,$6)
                         ON CONFLICT(relay_state) DO NOTHING"
                    ),
                    &[
                        &record.relay_state,
                        &record.org_id,
                        &record.request_id,
                        &record.callback_url,
                        &record.error_callback_url,
                        &(record.created_at as i64),
                    ],
                );
            }
        }

        fn take_state(&self, relay_state: &str, expected_org_id: &str) -> Option<SamlStateRecord> {
            let mut c = self.client.lock().ok()?;
            let row = c
                .query_opt(
                    &format!(
                        "SELECT relay_state, org_id, request_id, callback_url, error_callback_url, created_at
                         FROM {PG_STATE} WHERE relay_state = $1"
                    ),
                    &[&relay_state],
                )
                .ok()??;
            let record = pg_state_row_to_record(&row);
            if record.org_id != expected_org_id {
                return None;
            }
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            if now.saturating_sub(record.created_at) >= SAML_STATE_TTL_SECS {
                let _ = c.execute(
                    &format!("DELETE FROM {PG_STATE} WHERE relay_state = $1"),
                    &[&relay_state],
                );
                return None;
            }
            let _ = c.execute(
                &format!("DELETE FROM {PG_STATE} WHERE relay_state = $1"),
                &[&relay_state],
            );
            Some(record)
        }
    }

    fn pg_row_to_config(row: &postgres::Row) -> SamlConfig {
        let domains_json: String = row.get(5);
        let email_domains: Vec<String> = serde_json::from_str(&domains_json).unwrap_or_default();
        SamlConfig {
            org_id: row.get(0),
            idp_entity_id: row.get(1),
            idp_sso_url: row.get(2),
            idp_x509_cert_pem: row.get(3),
            default_role: row.get(4),
            email_domains,
            email_attribute: row.get(6),
            name_attribute: row.get(7),
            created_at: row.get::<_, i64>(8) as u64,
            updated_at: row.get::<_, i64>(9) as u64,
        }
    }

    fn pg_state_row_to_record(row: &postgres::Row) -> SamlStateRecord {
        SamlStateRecord {
            relay_state: row.get(0),
            org_id: row.get(1),
            request_id: row.get(2),
            callback_url: row.get(3),
            error_callback_url: row.get(4),
            created_at: row.get::<_, i64>(5) as u64,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> SqliteSamlBackend {
        SqliteSamlBackend::in_memory().unwrap()
    }

    fn cfg(org: &str, domains: Vec<&str>) -> SamlConfig {
        SamlConfig {
            org_id: org.into(),
            idp_entity_id: "https://acme.okta.com/saml".into(),
            idp_sso_url: "https://acme.okta.com/sso".into(),
            idp_x509_cert_pem: "-----BEGIN CERTIFICATE-----\nMI...\n-----END CERTIFICATE-----"
                .into(),
            default_role: "Member".into(),
            email_domains: domains.into_iter().map(String::from).collect(),
            email_attribute: "email".into(),
            name_attribute: Some("name".into()),
            created_at: 100,
            updated_at: 100,
        }
    }

    #[test]
    fn round_trip_upsert_get() {
        let s = store();
        s.upsert(cfg("acme", vec!["acme.com"])).unwrap();
        let got = s.get("acme").unwrap();
        assert_eq!(got.idp_sso_url, "https://acme.okta.com/sso");
        assert_eq!(got.email_domains, vec!["acme.com".to_string()]);
    }

    #[test]
    fn upsert_replaces_on_conflict() {
        let s = store();
        s.upsert(cfg("acme", vec!["old.com"])).unwrap();
        let mut updated = cfg("acme", vec!["new.com"]);
        updated.idp_sso_url = "https://acme.okta.com/sso2".into();
        updated.updated_at = 200;
        s.upsert(updated);
        let got = s.get("acme").unwrap();
        assert_eq!(got.idp_sso_url, "https://acme.okta.com/sso2");
        assert_eq!(got.email_domains, vec!["new.com".to_string()]);
        assert_eq!(got.updated_at, 200);
    }

    #[test]
    fn find_by_email_domain_walks_rows() {
        let s = store();
        s.upsert(cfg("acme", vec!["acme.com", "acme.io"])).unwrap();
        s.upsert(cfg("globex", vec!["globex.com"])).unwrap();
        assert_eq!(s.find_by_email_domain("ACME.IO").as_deref(), Some("acme"));
        assert_eq!(
            s.find_by_email_domain("globex.com").as_deref(),
            Some("globex")
        );
        assert_eq!(s.find_by_email_domain("nobody.com"), None);
    }

    #[test]
    fn delete_removes_row() {
        let s = store();
        s.upsert(cfg("acme", vec![])).unwrap();
        assert!(s.delete("acme"));
        assert!(s.get("acme").is_none());
        assert!(!s.delete("acme"));
    }

    #[test]
    fn state_round_trip_and_replay_blocked() {
        let s = store();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let rec = SamlStateRecord {
            relay_state: "rs_1".into(),
            org_id: "acme".into(),
            request_id: "_req".into(),
            callback_url: "https://app/cb".into(),
            error_callback_url: "https://app/err".into(),
            created_at: now,
        };
        s.save_state(rec);
        assert!(s.take_state("rs_1", "acme").is_some());
        assert!(s.take_state("rs_1", "acme").is_none());
    }

    #[test]
    fn state_take_rejects_wrong_org() {
        let s = store();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        s.save_state(SamlStateRecord {
            relay_state: "rs_2".into(),
            org_id: "acme".into(),
            request_id: "_r".into(),
            callback_url: "u".into(),
            error_callback_url: "u".into(),
            created_at: now,
        });
        assert!(s.take_state("rs_2", "evil").is_none());
        assert!(s.take_state("rs_2", "acme").is_some());
    }
}
