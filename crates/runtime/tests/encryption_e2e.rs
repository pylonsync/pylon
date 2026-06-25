//! End-to-end test for `field.encrypted()`.
//!
//! Verifies that with `PYLON_ENCRYPTION_KEY` set + a manifest field
//! marked `encrypted: true`, inserts encrypt the value before the
//! storage backend sees it (raw SQL inspection shows ciphertext)
//! AND reads decrypt transparently (Runtime::get_by_id returns the
//! original plaintext).
//!
//! Threat model under test: an attacker with read access to the
//! SQLite file (DB dump, hot copy, backup) sees only ciphertext for
//! encrypted fields. The Runtime API surfaces plaintext to
//! authorized callers.

use pylon_kernel::{AppManifest, ManifestEntity, ManifestField};
use pylon_runtime::Runtime;
use serde_json::json;
use std::sync::Mutex;

// Tests in this file mutate `PYLON_ENCRYPTION_KEY` env. Cargo runs
// integration tests in parallel by default, and `std::env::set_var`
// is process-wide. Serialize via a Mutex so the key isn't
// concurrently swapped while a Runtime is being constructed.
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn manifest_with_encrypted_ssn() -> AppManifest {
    AppManifest {
        manifest_version: 1,
        name: "encryption-e2e".into(),
        version: "0.1.0".into(),
        entities: vec![ManifestEntity {
            name: "Customer".into(),
            fields: vec![
                ManifestField {
                    name: "name".into(),
                    field_type: "string".into(),
                    optional: false,
                    unique: false,
                    crdt: None,
                    server_only: false,
                    readonly: false,
                    default: None,
                    enum_values: None,
                    encrypted: false,
                },
                ManifestField {
                    name: "ssn".into(),
                    field_type: "string".into(),
                    optional: false,
                    unique: false,
                    crdt: None,
                    server_only: true,
                    readonly: false,
                    default: None,
                    enum_values: None,
                    encrypted: true,
                },
            ],
            indexes: vec![],
            relations: vec![],
            search: None,
            crdt: false,
        }],
        routes: vec![],
        queries: vec![],
        actions: vec![],
        policies: vec![],
        auth: Default::default(),
        llm: Default::default(),
        connections: vec![],
        crons: vec![],
        fonts: vec![],
    }
}

/// Set the encryption key env, run a closure, then clear it.
/// Serialized through `ENV_LOCK` so parallel tests don't swap the
/// env mid-construction.
fn with_encryption_key<F: FnOnce() -> R, R>(f: F) -> R {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    std::env::set_var(
        "PYLON_ENCRYPTION_KEY",
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    );
    let result = f();
    std::env::remove_var("PYLON_ENCRYPTION_KEY");
    result
}

#[test]
fn round_trip_encrypts_at_rest_and_decrypts_on_read() {
    let result = with_encryption_key(|| {
        let rt = Runtime::in_memory(manifest_with_encrypted_ssn()).unwrap();
        // Use a 40-char hex id (Runtime::insert validates this).
        let id = "abcd1234ef567890abcd1234ef567890abcd1234";
        rt.insert(
            "Customer",
            &json!({
                "id": id,
                "name": "Alice",
                "ssn": "111-22-3333",
            }),
        )
        .unwrap();

        // Read via the public Runtime API — value should be plaintext.
        let row = rt.get_by_id("Customer", id).unwrap().unwrap();
        assert_eq!(row["ssn"], "111-22-3333", "API read returns plaintext");
        assert_eq!(row["name"], "Alice");

        // Codex P1: every public API decrypts, so we can't prove the
        // bytes on disk are ciphertext just by calling the API. Open
        // a raw rusqlite connection and SELECT the ssn column to
        // confirm the storage layer holds the `enc:v1:` wire format.
        // This is the LOAD-BEARING assertion that the feature
        // actually does anything.
        let raw_via_api = rt.list("Customer").unwrap();
        assert_eq!(raw_via_api[0]["ssn"], "111-22-3333"); // API: plaintext

        // Now read straight from the runtime's internal connection —
        // bypasses every encryption layer.
        let conn = rt.lock_conn_pub().unwrap();
        let mut stmt = conn
            .prepare("SELECT ssn FROM Customer WHERE id = ?1")
            .unwrap();
        let raw_ssn: String = stmt.query_row([id], |r| r.get(0)).unwrap();
        assert!(
            raw_ssn.starts_with("enc:v1:"),
            "Storage layer must hold ciphertext, not plaintext. Got: {raw_ssn}"
        );
        assert_ne!(raw_ssn, "111-22-3333", "Plaintext leaked to disk!");
    });
    let _ = result;
}

#[test]
fn encrypted_field_unreadable_without_key() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    // Ensure no key is set for this test.
    std::env::remove_var("PYLON_ENCRYPTION_KEY");
    let rt = Runtime::in_memory(manifest_with_encrypted_ssn()).unwrap();
    let id = "11223344556677889900aabbccddeeff11223344";
    let res = rt.insert(
        "Customer",
        &json!({"id": id, "name": "Bob", "ssn": "999-88-7777"}),
    );
    let err = match res {
        Ok(_) => panic!("insert with encrypted field must require the key"),
        Err(e) => e,
    };
    assert_eq!(err.code, "ENCRYPTION_NOT_CONFIGURED");
}

#[test]
fn unencrypted_entity_works_without_key() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    std::env::remove_var("PYLON_ENCRYPTION_KEY");
    // Manifest with NO encrypted fields — no key needed.
    let mut m = manifest_with_encrypted_ssn();
    for f in m.entities[0].fields.iter_mut() {
        // Also drop serverOnly since validation no longer requires it
        // for non-encrypted fields.
        f.encrypted = false;
    }
    let rt = Runtime::in_memory(m).unwrap();
    let id = "22334455667788990011aabbccddeeff22334455";
    rt.insert(
        "Customer",
        &json!({"id": id, "name": "Carol", "ssn": "no-secret"}),
    )
    .unwrap();
    let row = rt.get_by_id("Customer", id).unwrap().unwrap();
    assert_eq!(row["ssn"], "no-secret");
}

#[test]
fn list_after_decrypts_too() {
    with_encryption_key(|| {
        let rt = Runtime::in_memory(manifest_with_encrypted_ssn()).unwrap();
        let id1 = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let id2 = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        rt.insert(
            "Customer",
            &json!({"id": id1, "name": "A", "ssn": "secret-a"}),
        )
        .unwrap();
        rt.insert(
            "Customer",
            &json!({"id": id2, "name": "B", "ssn": "secret-b"}),
        )
        .unwrap();
        let rows = rt.list_after("Customer", None, 10).unwrap();
        assert_eq!(rows.len(), 2);
        for row in &rows {
            let ssn = row["ssn"].as_str().unwrap();
            assert!(ssn == "secret-a" || ssn == "secret-b");
        }
    });
}

#[test]
fn manifest_validation_rejects_encrypted_on_non_string() {
    // Codex P1: encrypted() must only be allowed on string/richtext.
    // An int/datetime/float column can't hold the base64 wire format.
    let mut m = manifest_with_encrypted_ssn();
    m.entities[0].fields[1].field_type = "int".into();
    let err = match Runtime::in_memory(m) {
        Ok(_) => panic!("expected manifest validation to fail"),
        Err(e) => e,
    };
    assert_eq!(err.code, "ENCRYPTION_MANIFEST_INVALID");
    assert!(err.message.contains("requires field type"));
}

#[test]
fn manifest_validation_rejects_encrypted_unique() {
    // Codex P1: unique() + encrypted() is a silent correctness
    // regression — random nonce makes the unique constraint a no-op.
    let mut m = manifest_with_encrypted_ssn();
    m.entities[0].fields[1].unique = true;
    let err = match Runtime::in_memory(m) {
        Ok(_) => panic!("expected manifest validation to fail"),
        Err(e) => e,
    };
    assert_eq!(err.code, "ENCRYPTION_MANIFEST_INVALID");
    assert!(err.message.contains("cannot combine with unique"));
}

#[test]
fn manifest_validation_rejects_encrypted_without_serverOnly() {
    // Codex P1: encrypted() without serverOnly() leaks plaintext over
    // every public wire surface. Refuse the manifest at boot.
    let mut m = manifest_with_encrypted_ssn();
    m.entities[0].fields[1].server_only = false;
    let err = match Runtime::in_memory(m) {
        Ok(_) => panic!("expected manifest validation to fail"),
        Err(e) => e,
    };
    assert_eq!(err.code, "ENCRYPTION_MANIFEST_INVALID");
    assert!(err.message.contains("requires serverOnly"));
}

#[test]
fn boot_fails_on_malformed_encryption_key() {
    // Codex P2: a typo'd PYLON_ENCRYPTION_KEY should fail BOOT, not
    // log a warning and ship a process that rejects every write.
    let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    std::env::set_var("PYLON_ENCRYPTION_KEY", "not-a-valid-key");
    let result = Runtime::in_memory(manifest_with_encrypted_ssn());
    std::env::remove_var("PYLON_ENCRYPTION_KEY");
    let err = match result {
        Ok(_) => panic!("expected boot to fail on malformed key"),
        Err(e) => e,
    };
    assert_eq!(err.code, "ENCRYPTION_KEY_INVALID");
}

#[test]
fn tx_store_path_encrypts() {
    // Codex P1: TxStore (action-handler write path) goes through
    // insert_with_conn / update_with_conn, NOT insert/update.
    // Without encrypting at this layer, every action handler that
    // writes an encrypted field lands plaintext.
    with_encryption_key(|| {
        let rt = Runtime::in_memory(manifest_with_encrypted_ssn()).unwrap();
        let id = "deadbeefcafe1234deadbeefcafe1234deadbeef";
        let conn = rt.lock_conn_pub().unwrap();
        // Write via the _with_conn path (what TxStore uses).
        rt.insert_with_conn(
            &conn,
            "Customer",
            &serde_json::json!({"id": id, "name": "TxAlice", "ssn": "tx-secret"}),
        )
        .unwrap();
        // Now SELECT raw and confirm ciphertext.
        let mut stmt = conn
            .prepare("SELECT ssn FROM Customer WHERE id = ?1")
            .unwrap();
        let raw_ssn: String = stmt.query_row([id], |r| r.get(0)).unwrap();
        assert!(
            raw_ssn.starts_with("enc:v1:"),
            "TxStore (insert_with_conn) must encrypt. Got plaintext: {raw_ssn}"
        );
        // And confirm the read-back via _with_conn decrypts.
        let row = rt
            .get_by_id_with_conn(&conn, "Customer", id)
            .unwrap()
            .unwrap();
        assert_eq!(row["ssn"], "tx-secret");
    });
}

#[test]
fn update_re_encrypts_field() {
    with_encryption_key(|| {
        let rt = Runtime::in_memory(manifest_with_encrypted_ssn()).unwrap();
        let id = "ccccccccccccccccccccccccccccccccccccccccc"[..40].to_string();
        rt.insert(
            "Customer",
            &json!({"id": id.clone(), "name": "D", "ssn": "old-ssn"}),
        )
        .unwrap();
        rt.update("Customer", &id, &json!({"ssn": "new-ssn"}))
            .unwrap();
        let row = rt.get_by_id("Customer", &id).unwrap().unwrap();
        assert_eq!(row["ssn"], "new-ssn");
    });
}
