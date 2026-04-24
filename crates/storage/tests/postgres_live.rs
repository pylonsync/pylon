//! Live Postgres integration test.
//!
//! This test only runs when BOTH conditions hold:
//!
//!   1. The crate is built with `--features postgres-live`, AND
//!   2. The environment variable `TEST_POSTGRES_URL` is set to a
//!      connection string like `postgres://user:pass@localhost/dbname`.
//!
//! Otherwise the test is a no-op — so a plain `cargo test --workspace`
//! never requires Postgres to be installed.
//!
//! CI recipe (GitHub Actions):
//!
//! ```yaml
//! services:
//!   postgres:
//!     image: postgres:16
//!     env:
//!       POSTGRES_PASSWORD: test
//!     ports: ["5432:5432"]
//!     options: >-
//!       --health-cmd "pg_isready -U postgres"
//!       --health-interval 10s
//!       --health-timeout 5s
//!       --health-retries 5
//! env:
//!   TEST_POSTGRES_URL: postgres://postgres:test@localhost/postgres
//! run: cargo test -p pylon-storage --features postgres-live
//! ```

#![cfg(feature = "postgres-live")]

use pylon_http::DataStore;
use pylon_kernel::{AppManifest, ManifestEntity, ManifestField};
use pylon_storage::pg_datastore::PostgresDataStore;

fn require_pg_url() -> Option<String> {
    std::env::var("TEST_POSTGRES_URL").ok()
}

fn test_manifest() -> AppManifest {
    AppManifest {
        manifest_version: 1,
        name: "pg_live_test".into(),
        version: "0.1.0".into(),
        entities: vec![ManifestEntity {
            name: "PgTodo".into(),
            fields: vec![
                ManifestField {
                    name: "title".into(),
                    field_type: "string".into(),
                    optional: false,
                    unique: false,
                },
                ManifestField {
                    name: "done".into(),
                    field_type: "bool".into(),
                    optional: false,
                    unique: false,
                },
            ],
            indexes: vec![],
            relations: vec![],
            search: None,
        }],
        routes: vec![],
        queries: vec![],
        actions: vec![],
        policies: vec![],
    }
}

#[test]
fn crud_roundtrip() {
    let Some(url) = require_pg_url() else {
        eprintln!("TEST_POSTGRES_URL not set — skipping pg_live test");
        return;
    };
    let store = PostgresDataStore::connect(&url, test_manifest()).expect("connect");

    let id = store
        .insert(
            "PgTodo",
            &serde_json::json!({"title": "buy milk", "done": false}),
        )
        .expect("insert");

    let row = store.get_by_id("PgTodo", &id).expect("get").expect("row");
    assert_eq!(row["title"], "buy milk");
    assert_eq!(row["done"], false);

    let updated = store
        .update(
            "PgTodo",
            &id,
            &serde_json::json!({"title": "buy milk", "done": true}),
        )
        .expect("update");
    assert!(updated);

    let row2 = store.get_by_id("PgTodo", &id).expect("get").expect("row");
    assert_eq!(row2["done"], true);

    let rows = store.list("PgTodo").expect("list");
    assert!(rows.iter().any(|r| r["id"] == serde_json::json!(id)));

    let deleted = store.delete("PgTodo", &id).expect("delete");
    assert!(deleted);
    let gone = store.get_by_id("PgTodo", &id).expect("get after delete");
    assert!(gone.is_none());
}

#[test]
fn unknown_entity_returns_error() {
    let Some(url) = require_pg_url() else {
        eprintln!("TEST_POSTGRES_URL not set — skipping pg_live test");
        return;
    };
    let store = PostgresDataStore::connect(&url, test_manifest()).expect("connect");

    let err = store
        .insert("NotAnEntity", &serde_json::json!({}))
        .expect_err("must reject unknown entity");
    assert!(
        err.code.contains("UNKNOWN") || err.code.contains("ENTITY") || !err.message.is_empty(),
        "error should be descriptive, got code={} message={}",
        err.code,
        err.message
    );
}
