//! Remember entities whose CRDT history can contain private fields.
//! Removing a field from the current schema must not make its history public.
use std::collections::HashSet;

use pylon_kernel::AppManifest;

fn private_entities(manifest: &AppManifest) -> Vec<&str> {
    let mut entities = vec![manifest.auth.user.entity.as_str()];
    entities.extend(
        manifest
            .entities
            .iter()
            .filter(|entity| {
                entity.name != manifest.auth.user.entity
                    && entity.fields.iter().any(|field| {
                        field.server_only
                            || field.sync_omit
                            || field.field_type.starts_with("vector(")
                    })
            })
            .map(|entity| entity.name.as_str()),
    );
    entities
}

pub(crate) fn sqlite_has_private_history(conn: &rusqlite::Connection, entity: &str) -> bool {
    conn.prepare_cached(
        "SELECT EXISTS(SELECT 1 FROM _pylon_crdt_private_entities WHERE entity = ?1)",
    )
    .and_then(|mut statement| statement.query_row([entity], |row| row.get::<_, bool>(0)))
    .unwrap_or(true)
}

pub(crate) fn sqlite(
    conn: &rusqlite::Connection,
    manifest: &AppManifest,
) -> Result<HashSet<String>, crate::RuntimeError> {
    let result = (|| -> rusqlite::Result<HashSet<String>> {
        let tx = conn.unchecked_transaction()?;
        let initialized: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = '_pylon_crdt_private_entities')",
            [], |row| row.get(0),
        )?;
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS _pylon_crdt_private_entities (entity TEXT PRIMARY KEY)",
        )?;
        if !initialized {
            let snapshots_exist: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = '_pylon_crdt_snapshots')",
                [], |row| row.get(0),
            )?;
            if snapshots_exist {
                // Old histories have no reliable record of removed private fields.
                tx.execute("INSERT OR IGNORE INTO _pylon_crdt_private_entities SELECT DISTINCT entity FROM _pylon_crdt_snapshots", [])?;
            }
        }
        {
            let mut insert = tx.prepare_cached(
                "INSERT OR IGNORE INTO _pylon_crdt_private_entities (entity) VALUES (?1)",
            )?;
            for entity in private_entities(manifest) {
                insert.execute([entity])?;
            }
        }
        let entities = {
            let mut select =
                tx.prepare_cached("SELECT entity FROM _pylon_crdt_private_entities")?;
            let entities = select
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<HashSet<String>>>()?;
            entities
        };
        tx.commit()?;
        Ok(entities)
    })();
    result.map_err(|error| crate::RuntimeError {
        code: "CRDT_PRIVACY_INIT_FAILED".into(),
        message: error.to_string(),
    })
}

pub(crate) fn postgres(
    store: &pylon_storage::pg_datastore::PostgresDataStore,
    manifest: &AppManifest,
) -> Result<HashSet<String>, crate::RuntimeError> {
    store
        .with_client(|client| {
            postgres_on_client(client, manifest).map_err(|error| pylon_http::DataError {
                code: "CRDT_PRIVACY_INIT_FAILED".into(),
                message: error.to_string(),
            })
        })
        .map_err(crate::data_err_to_runtime)
}

fn postgres_on_client(
    client: &mut postgres::Client,
    manifest: &AppManifest,
) -> Result<HashSet<String>, postgres::Error> {
    let mut tx = client.transaction()?;
    // Serialize first-install detection across starting instances.
    tx.query_one("SELECT pg_advisory_xact_lock(1887009134, 1129464916)", &[])?;
    let initialized: bool = tx
        .query_one(
            "SELECT to_regclass('_pylon_crdt_private_entities') IS NOT NULL",
            &[],
        )?
        .get(0);
    tx.batch_execute(
        "CREATE TABLE IF NOT EXISTS _pylon_crdt_private_entities (entity TEXT PRIMARY KEY)",
    )?;
    if !initialized {
        let snapshots_exist: bool = tx
            .query_one(
                "SELECT to_regclass('_pylon_crdt_snapshots') IS NOT NULL",
                &[],
            )?
            .get(0);
        if snapshots_exist {
            tx.execute(
                "INSERT INTO _pylon_crdt_private_entities SELECT DISTINCT entity FROM _pylon_crdt_snapshots ON CONFLICT (entity) DO NOTHING", &[],
            )?;
        }
    }
    tx.execute(
        "INSERT INTO _pylon_crdt_private_entities (entity) SELECT unnest($1::text[]) ON CONFLICT (entity) DO NOTHING",
        &[&private_entities(manifest)],
    )?;
    let entities = tx
        .query("SELECT entity FROM _pylon_crdt_private_entities", &[])?
        .iter()
        .map(|row| row.get(0))
        .collect();
    tx.commit()?;
    Ok(entities)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn postgres_legacy_migration_is_atomic_and_persistent() {
        let Ok(url) = std::env::var("TEST_POSTGRES_URL") else {
            return;
        };
        let mut client = postgres::Client::connect(&url, postgres::NoTls).unwrap();
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let schema = format!("privacy_test_{suffix}");
        client.batch_execute(&format!("CREATE SCHEMA {schema}; SET search_path TO {schema}; CREATE TABLE _pylon_crdt_snapshots (wrong_column TEXT);")).unwrap();
        assert!(postgres_on_client(&mut client, &AppManifest::default()).is_err());
        let initialized: bool = client
            .query_one(
                "SELECT to_regclass('_pylon_crdt_private_entities') IS NOT NULL",
                &[],
            )
            .unwrap()
            .get(0);
        assert!(!initialized);
        client.batch_execute("ALTER TABLE _pylon_crdt_snapshots RENAME COLUMN wrong_column TO entity; INSERT INTO _pylon_crdt_snapshots VALUES ('LegacyDoc');").unwrap();
        assert!(postgres_on_client(&mut client, &AppManifest::default())
            .unwrap()
            .contains("LegacyDoc"));
        client.batch_execute("DELETE FROM _pylon_crdt_snapshots; INSERT INTO _pylon_crdt_snapshots VALUES ('NewPublicDoc');").unwrap();
        let restricted = postgres_on_client(&mut client, &AppManifest::default()).unwrap();
        assert!(restricted.contains("LegacyDoc"));
        assert!(!restricted.contains("NewPublicDoc"));
        client
            .batch_execute(&format!("DROP SCHEMA {schema} CASCADE"))
            .unwrap();
    }

    #[test]
    fn auth_entity_history_survives_renaming_without_an_explicit_entity_definition() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let mut manifest = AppManifest::default();
        manifest.auth.user.entity = "Members".into();
        assert!(sqlite(&conn, &manifest).unwrap().contains("Members"));
        manifest.auth.user.entity = "Accounts".into();
        let restricted = sqlite(&conn, &manifest).unwrap();
        assert!(restricted.contains("Members"));
        assert!(restricted.contains("Accounts"));
    }

    #[test]
    fn legacy_histories_remain_restricted_after_restart() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE _pylon_crdt_snapshots (entity TEXT); INSERT INTO _pylon_crdt_snapshots VALUES ('LegacyDoc');").unwrap();
        let manifest = AppManifest::default();
        assert!(sqlite(&conn, &manifest).unwrap().contains("LegacyDoc"));
        conn.execute("DELETE FROM _pylon_crdt_snapshots", [])
            .unwrap();
        assert!(sqlite(&conn, &manifest).unwrap().contains("LegacyDoc"));
        conn.execute(
            "INSERT INTO _pylon_crdt_snapshots VALUES ('NewPublicDoc')",
            [],
        )
        .unwrap();
        assert!(!sqlite(&conn, &manifest).unwrap().contains("NewPublicDoc"));
    }

    #[test]
    fn legacy_migration_failure_rolls_back_initialization() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE _pylon_crdt_snapshots (wrong_column TEXT)", [])
            .unwrap();
        assert!(sqlite(&conn, &AppManifest::default()).is_err());
        let initialized: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name = '_pylon_crdt_private_entities')", [], |row| row.get(0)).unwrap();
        assert!(!initialized);
        conn.execute(
            "ALTER TABLE _pylon_crdt_snapshots RENAME COLUMN wrong_column TO entity",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO _pylon_crdt_snapshots VALUES ('LegacyDoc')", [])
            .unwrap();
        assert!(sqlite(&conn, &AppManifest::default())
            .unwrap()
            .contains("LegacyDoc"));
    }

    #[test]
    fn history_read_errors_deny_replication() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        assert!(sqlite_has_private_history(&conn, "Doc"));
        sqlite(&conn, &AppManifest::default()).unwrap();
        assert!(!sqlite_has_private_history(&conn, "Doc"));
        conn.execute(
            "INSERT INTO _pylon_crdt_private_entities VALUES ('Doc')",
            [],
        )
        .unwrap();
        assert!(sqlite_has_private_history(&conn, "Doc"));
        conn.execute("DROP TABLE _pylon_crdt_private_entities", [])
            .unwrap();
        assert!(sqlite_has_private_history(&conn, "Doc"));
    }

    #[test]
    fn malformed_privacy_metadata_fails_closed() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute(
            "CREATE TABLE _pylon_crdt_private_entities (wrong_column TEXT)",
            [],
        )
        .unwrap();
        let error = sqlite(&conn, &AppManifest::default()).unwrap_err();
        assert_eq!(error.code, "CRDT_PRIVACY_INIT_FAILED");
    }
}
