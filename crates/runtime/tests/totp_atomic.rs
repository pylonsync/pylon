//! Concurrent backup-code writes must have only one winner per snapshot.
use pylon_http::DataStore;
use pylon_kernel::{AppManifest, ManifestEntity, ManifestField};
use pylon_runtime::Runtime;
use serde_json::json;
use std::sync::{Arc, Barrier};

fn user_field(name: &str, ty: &str) -> ManifestField {
    ManifestField {
        name: name.into(),
        field_type: ty.into(),
        optional: true,
        unique: false,
        crdt: None,
        server_only: false,
        readonly: false,
        default: None,
        enum_values: None,
        encrypted: false,
        sync_omit: false,
        max_length: None,
    }
}

fn manifest(crdt: bool) -> AppManifest {
    AppManifest {
        manifest_version: 1,
        name: "totp-atomic".into(),
        version: "0.1.0".into(),
        entities: vec![ManifestEntity {
            name: "BackupUser".into(),
            fields: vec![
                user_field("totpSecret", "string"),
                user_field("totpBackupCodes", "json"),
            ],
            crdt,
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn exercise(stores: Vec<Runtime>) {
    let stores: Vec<_> = stores.into_iter().map(Arc::new).collect();
    let original = json!({"totpSecret":"test-seed", "totpBackupCodes":["hash-a", "hash-b"]});
    for distinct in [false, true] {
        let id = stores[0].insert("BackupUser", &original).unwrap();
        let barrier = Arc::new(Barrier::new(stores.len()));
        let threads: Vec<_> = stores
            .iter()
            .enumerate()
            .map(|(i, store)| {
                let store = store.clone();
                let barrier = barrier.clone();
                let id = id.clone();
                let expected = original.clone();
                std::thread::spawn(move || {
                    let kept = if distinct && i % 2 == 1 {
                        "hash-a"
                    } else {
                        "hash-b"
                    };
                    // All callers hold the same pre-consumption snapshot.
                    barrier.wait();
                    let won = store
                        .update_if_matches(
                            "BackupUser",
                            &id,
                            &expected,
                            &json!({"totpBackupCodes":[kept]}),
                        )
                        .unwrap();
                    (won, kept)
                })
            })
            .collect();
        let winners: Vec<_> = threads
            .into_iter()
            .map(|t| t.join().unwrap())
            .filter(|r| r.0)
            .collect();
        assert_eq!(
            winners.len(),
            1,
            "one snapshot must authorize only one write"
        );
        let row = stores[0].get_by_id("BackupUser", &id).unwrap().unwrap();
        assert_eq!(row["totpBackupCodes"], json!([winners[0].1]));
        assert!(!stores[0]
            .update_if_matches("BackupUser", &id, &original, &json!({"totpBackupCodes":[]}))
            .unwrap());
        assert!(stores[0]
            .update_if_matches(
                "BackupUser",
                &id,
                &json!({"totpSecret":"test-seed", "totpBackupCodes":[winners[0].1]}),
                &json!({"totpBackupCodes":[]})
            )
            .unwrap());
    }
    let id = stores[0].insert("BackupUser", &original).unwrap();
    stores[1]
        .update("BackupUser", &id, &json!({"totpSecret":"new-seed"}))
        .unwrap();
    assert!(!stores[0]
        .update_if_matches("BackupUser", &id, &original, &json!({"totpBackupCodes":[]}))
        .unwrap());
    assert_eq!(
        stores[0].get_by_id("BackupUser", &id).unwrap().unwrap()["totpBackupCodes"],
        original["totpBackupCodes"]
    );
    assert!(!stores[0]
        .update_if_matches(
            "BackupUser",
            "missing",
            &original,
            &json!({"totpBackupCodes":[]})
        )
        .unwrap());
    // Invalid writes must roll back and leave the snapshot usable.
    let id = stores[0].insert("BackupUser", &original).unwrap();
    assert!(stores[0]
        .update_if_matches("BackupUser", &id, &original, &json!({"unknownColumn":true}))
        .is_err());
    assert!(stores[0]
        .update_if_matches("BackupUser", &id, &original, &json!({"totpBackupCodes":[]}))
        .unwrap());
}

#[test]
fn sqlite_backup_codes_are_atomic_across_connections() {
    for crdt in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.db");
        exercise(
            (0..4)
                .map(|_| Runtime::open(path.to_str().unwrap(), manifest(crdt)).unwrap())
                .collect(),
        );
    }
}

#[test]
#[ignore = "requires an isolated PYLON_TEST_PG_URL database"]
fn postgres_backup_codes_are_atomic_across_pools() {
    let url = std::env::var("PYLON_TEST_PG_URL").expect("isolated PostgreSQL URL");
    for crdt in [false, true] {
        let manifest = manifest(crdt);
        let mut adapter =
            pylon_storage::postgres::live::LivePostgresAdapter::connect(&url).unwrap();
        adapter
            .exec_raw("DROP TABLE IF EXISTS \"BackupUser\" CASCADE")
            .unwrap();
        let plan = adapter.plan_from_live(&manifest).unwrap();
        adapter.apply_plan(&plan).unwrap();
        exercise(
            (0..4)
                .map(|_| Runtime::open_postgres(&url, manifest.clone()).unwrap())
                .collect(),
        );
    }
}
