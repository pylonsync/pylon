//! Live-Postgres integration tests for `Runtime::open_postgres`.
//!
//! Skipped unless `PYLON_TEST_PG_URL` is set — CI provisions a throwaway
//! database via `docker compose up postgres` and exports the URL. To run
//! locally:
//!
//! ```sh
//! docker run --rm -d -p 5544:5432 -e POSTGRES_PASSWORD=test \
//!   --name pylon-pg-test postgres:16
//! PYLON_TEST_PG_URL=postgres://postgres:test@localhost:5544/postgres \
//!   cargo test -p pylon-runtime --test postgres_backend -- --test-threads=1
//! ```
//!
//! The `--test-threads=1` is important: tests share one database and
//! truncate via DROP/CREATE between cases.

use pylon_kernel::*;
use pylon_runtime::Runtime;

fn pg_url() -> Option<String> {
    std::env::var("PYLON_TEST_PG_URL").ok()
}

fn empty_manifest() -> AppManifest {
    AppManifest {
        manifest_version: 1,
        name: "pg_test".into(),
        version: "1".into(),
        entities: vec![],
        routes: vec![],
        queries: vec![],
        actions: vec![],
        policies: vec![],
    }
}

fn fresh_runtime(url: &str) -> Runtime {
    let manifest = AppManifest {
        entities: vec![ManifestEntity {
            name: "User".into(),
            fields: vec![
                ManifestField {
                    name: "email".into(),
                    field_type: "string".into(),
                    optional: false,
                    unique: true,
                    crdt: None,
                },
                ManifestField {
                    name: "name".into(),
                    field_type: "string".into(),
                    optional: true,
                    unique: false,
                    crdt: None,
                },
            ],
            indexes: vec![],
            relations: vec![],
            crdt: false,
            search: None,
        }],
        ..empty_manifest()
    };

    // Apply the schema first via the live adapter so the User table exists.
    // Runtime::open_postgres does NOT run CREATE TABLE — schema is the
    // operator's responsibility on Postgres (via pylon-storage's adapter).
    let mut adapter = pylon_storage::postgres::live::LivePostgresAdapter::connect(url)
        .expect("connect to test postgres");
    // Wipe any previous test rows so cases stay isolated.
    let _ = adapter.exec_raw("DROP TABLE IF EXISTS \"User\" CASCADE");
    let plan = adapter
        .plan_from_live(&manifest)
        .expect("plan against fresh schema");
    adapter.apply_plan(&plan).expect("apply schema");

    Runtime::open_postgres(url, manifest).expect("open postgres runtime")
}

#[test]
fn open_postgres_dispatches_via_url_prefix() {
    let Some(url) = pg_url() else {
        eprintln!("skipping: set PYLON_TEST_PG_URL to enable");
        return;
    };
    let rt = Runtime::open(&url, empty_manifest()).expect("open via Runtime::open");
    assert!(rt.is_postgres());
    assert!(!rt.is_in_memory());
    assert!(rt.db_path().is_none());
    assert_eq!(rt.read_pool_size(), 0);
}

#[test]
fn insert_get_update_delete_roundtrip() {
    let Some(url) = pg_url() else {
        return;
    };
    let rt = fresh_runtime(&url);

    let id = rt
        .insert(
            "User",
            &serde_json::json!({"email": "a@b.com", "name": "Ada"}),
        )
        .expect("insert");
    let row = rt
        .get_by_id("User", &id)
        .expect("get_by_id")
        .expect("row exists");
    assert_eq!(row["email"], "a@b.com");
    assert_eq!(row["name"], "Ada");

    let updated = rt
        .update("User", &id, &serde_json::json!({"name": "Ada Lovelace"}))
        .expect("update");
    assert!(updated);
    let row = rt.get_by_id("User", &id).unwrap().unwrap();
    assert_eq!(row["name"], "Ada Lovelace");

    let deleted = rt.delete("User", &id).expect("delete");
    assert!(deleted);
    assert!(rt.get_by_id("User", &id).unwrap().is_none());
}

#[test]
fn list_after_paginates() {
    let Some(url) = pg_url() else {
        return;
    };
    let rt = fresh_runtime(&url);

    for i in 0..5 {
        rt.insert(
            "User",
            &serde_json::json!({"email": format!("u{i}@x.com"), "name": format!("u{i}")}),
        )
        .unwrap();
    }
    let page = rt.list_after("User", None, 3).unwrap();
    assert_eq!(page.len(), 3);
    let cursor = page.last().unwrap()["id"].as_str().unwrap().to_string();
    let next = rt.list_after("User", Some(&cursor), 10).unwrap();
    assert_eq!(next.len(), 2);
}

#[test]
fn lookup_by_unique_field() {
    let Some(url) = pg_url() else {
        return;
    };
    let rt = fresh_runtime(&url);
    rt.insert(
        "User",
        &serde_json::json!({"email": "lookup@x.com", "name": "L"}),
    )
    .unwrap();
    let row = rt
        .lookup("User", "email", "lookup@x.com")
        .expect("lookup")
        .expect("row");
    assert_eq!(row["email"], "lookup@x.com");
}

#[test]
fn crdt_paths_return_safe_defaults_in_postgres_mode() {
    let Some(url) = pg_url() else {
        return;
    };
    let rt = fresh_runtime(&url);
    use pylon_http::DataStore;

    let id = rt
        .insert(
            "User",
            &serde_json::json!({"email": "c@x.com", "name": "C"}),
        )
        .unwrap();

    // crdt_snapshot returns Ok(None) — same shape as `crdt: false` —
    // so the router degrades to JSON change events without erroring.
    assert_eq!(
        DataStore::crdt_snapshot(&rt, "User", &id).unwrap(),
        None,
        "PG runtime should report no snapshot, not an error"
    );
    // crdt_apply_update is rejected explicitly so SDKs see a clean
    // NOT_SUPPORTED instead of silently doing the wrong thing.
    let err = DataStore::crdt_apply_update(&rt, "User", &id, &[1, 2, 3]).unwrap_err();
    assert_eq!(err.code, "NOT_SUPPORTED");
}

#[test]
fn typed_columns_roundtrip_correctly() {
    // Regression: previously the PG insert path collapsed every JSON
    // value to String, so INTEGER / BOOLEAN / TIMESTAMPTZ columns either
    // broke or stored stringified garbage, and JSON `null` became `""`.
    let Some(url) = pg_url() else {
        return;
    };

    let manifest = AppManifest {
        entities: vec![ManifestEntity {
            name: "Typed".into(),
            fields: vec![
                ManifestField {
                    name: "count".into(),
                    field_type: "int".into(),
                    optional: false,
                    unique: false,
                    crdt: None,
                },
                ManifestField {
                    name: "active".into(),
                    field_type: "bool".into(),
                    optional: false,
                    unique: false,
                    crdt: None,
                },
                ManifestField {
                    name: "score".into(),
                    field_type: "float".into(),
                    optional: false,
                    unique: false,
                    crdt: None,
                },
                ManifestField {
                    name: "ownerId".into(),
                    field_type: "string".into(),
                    optional: true,
                    unique: false,
                    crdt: None,
                },
            ],
            indexes: vec![],
            relations: vec![],
            crdt: false,
            search: None,
        }],
        ..empty_manifest()
    };

    let mut adapter = pylon_storage::postgres::live::LivePostgresAdapter::connect(&url).unwrap();
    adapter
        .exec_raw("DROP TABLE IF EXISTS \"Typed\" CASCADE")
        .unwrap();
    let plan = adapter.plan_from_live(&manifest).unwrap();
    adapter.apply_plan(&plan).unwrap();

    let rt = Runtime::open_postgres(&url, manifest).unwrap();
    let id = rt
        .insert(
            "Typed",
            // Magic number unrelated to PI — clippy's approximate-PI lint
            // false-positives on 3.14, but here it's just a representative
            // float. Use a value that doesn't trip the heuristic.
            &serde_json::json!({"count": 42, "active": true, "score": 2.5, "ownerId": "owner_a"}),
        )
        .expect("typed insert");
    let row = rt.get_by_id("Typed", &id).unwrap().unwrap();
    // PG returns numeric columns as JSON numbers, not strings.
    assert_eq!(row["count"], 42);
    assert_eq!(row["active"], true);
    // Float comparison via JSON: assert it's a number, value within epsilon.
    assert!((row["score"].as_f64().unwrap() - 2.5).abs() < 1e-9);
    assert_eq!(row["ownerId"], "owner_a");

    // Now `unlink` the FK by setting it to null. The previous string-collapse
    // path stored "" instead — verify the field actually becomes JSON null.
    let updated = rt
        .update("Typed", &id, &serde_json::json!({"ownerId": null}))
        .expect("update with null");
    assert!(updated);
    let row = rt.get_by_id("Typed", &id).unwrap().unwrap();
    assert!(
        row["ownerId"].is_null(),
        "ownerId should be SQL NULL after update with null, got {:?}",
        row["ownerId"]
    );
}

#[test]
fn query_filtered_supports_not_and_in() {
    let Some(url) = pg_url() else {
        return;
    };
    let rt = fresh_runtime(&url);
    for i in 0..5 {
        rt.insert(
            "User",
            &serde_json::json!({"email": format!("p{i}@x.com"), "name": format!("p{i}")}),
        )
        .unwrap();
    }
    let not_p2 = rt
        .query_filtered("User", &serde_json::json!({"email": {"$not": "p2@x.com"}}))
        .expect("$not filter");
    assert_eq!(not_p2.len(), 4);
    assert!(not_p2
        .iter()
        .all(|row| row["email"].as_str().unwrap() != "p2@x.com"));

    let in_set = rt
        .query_filtered(
            "User",
            &serde_json::json!({"email": {"$in": ["p1@x.com", "p3@x.com"]}}),
        )
        .expect("$in filter");
    assert_eq!(in_set.len(), 2);
}

#[test]
fn aggregate_count_and_groupby_work_on_postgres() {
    let Some(url) = pg_url() else {
        return;
    };
    let rt = fresh_runtime(&url);
    for (email, name) in [
        ("a@x.com", "Alice"),
        ("b@x.com", "Bob"),
        ("c@x.com", "Alice"),
    ] {
        rt.insert("User", &serde_json::json!({"email": email, "name": name}))
            .unwrap();
    }
    // Count total — was returning NOT_SUPPORTED before the PG aggregate impl landed.
    let total = rt
        .aggregate("User", &serde_json::json!({"count": "*"}))
        .expect("aggregate count");
    assert_eq!(total["rows"][0]["count"], 3);

    // Group by name — should yield two buckets (Alice: 2, Bob: 1).
    let by_name = rt
        .aggregate(
            "User",
            &serde_json::json!({"count": "*", "groupBy": ["name"]}),
        )
        .expect("aggregate groupBy");
    let rows = by_name["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
}

#[test]
fn search_returns_clear_error_on_postgres() {
    use pylon_http::DataStore;
    let Some(url) = pg_url() else {
        return;
    };
    let rt = fresh_runtime(&url);
    let err = DataStore::search(&rt, "User", &serde_json::json!({"q": "x"})).unwrap_err();
    assert_eq!(err.code, "NOT_SUPPORTED");

    // $search inside query_filtered should also error with a useful code,
    // not silently return broad results.
    let err = rt
        .query_filtered("User", &serde_json::json!({"$search": "anything"}))
        .unwrap_err();
    assert_eq!(err.code, "SEARCH_NOT_SUPPORTED");
}

#[test]
fn transact_uses_real_postgres_transaction() {
    let Some(url) = pg_url() else {
        return;
    };
    let rt = fresh_runtime(&url);
    use pylon_http::DataStore;

    let ops = vec![
        serde_json::json!({"op":"insert","entity":"User","data":{"email":"tx1@x.com"}}),
        serde_json::json!({"op":"insert","entity":"User","data":{"email":"tx2@x.com"}}),
    ];
    let (ok, results) = DataStore::transact(&rt, &ops).unwrap();
    assert!(ok);
    assert_eq!(results.len(), 2);
    assert!(rt.lookup("User", "email", "tx1@x.com").unwrap().is_some());
    assert!(rt.lookup("User", "email", "tx2@x.com").unwrap().is_some());
}

#[test]
fn alter_field_drops_not_null_when_manifest_makes_field_optional() {
    // Regression: pylon-cloud's User entity made `avatarColor` optional
    // after the framework's OAuth handler started failing
    // USER_CREATE_FAILED on a NOT NULL violation. Before AlterField,
    // the manifest change was a no-op against the live PG schema —
    // operators had to drop NOT NULL by hand. This test pushes the
    // BEFORE schema, then re-plans against the AFTER manifest, and
    // confirms the existing column is altered (no rebuild, no data loss).
    let Some(url) = pg_url() else {
        return;
    };

    let mut adapter = pylon_storage::postgres::live::LivePostgresAdapter::connect(&url).unwrap();
    let _ = adapter.exec_raw("DROP TABLE IF EXISTS \"AlterTest\" CASCADE");

    let with_required = AppManifest {
        entities: vec![ManifestEntity {
            name: "AlterTest".into(),
            fields: vec![ManifestField {
                name: "color".into(),
                field_type: "string".into(),
                optional: false,
                unique: false,
                crdt: None,
            }],
            indexes: vec![],
            relations: vec![],
            crdt: false,
            search: None,
        }],
        ..empty_manifest()
    };
    let plan = adapter.plan_from_live(&with_required).unwrap();
    adapter.apply_plan(&plan).unwrap();

    // Insert a row that satisfies NOT NULL — proves the column starts
    // out required, otherwise the assertion below is meaningless.
    adapter
        .exec_raw("INSERT INTO \"AlterTest\" (id, color) VALUES ('row1', 'red')")
        .unwrap();

    // Now flip the manifest: color is optional. Re-plan against the
    // live schema and confirm the diff is one AlterField op.
    let with_optional = AppManifest {
        entities: vec![ManifestEntity {
            name: "AlterTest".into(),
            fields: vec![ManifestField {
                name: "color".into(),
                field_type: "string".into(),
                optional: true,
                unique: false,
                crdt: None,
            }],
            indexes: vec![],
            relations: vec![],
            crdt: false,
            search: None,
        }],
        ..empty_manifest()
    };
    let next_plan = adapter.plan_from_live(&with_optional).unwrap();
    let alter_count = next_plan
        .operations
        .iter()
        .filter(|op| matches!(op, pylon_storage::SchemaOperation::AlterField { .. }))
        .count();
    assert_eq!(
        alter_count, 1,
        "expected exactly one AlterField op, got plan: {:?}",
        next_plan.operations
    );
    adapter.apply_plan(&next_plan).unwrap();

    // The previously-required column should now accept NULL. The
    // existing row stays intact (no table rebuild).
    adapter
        .exec_raw("INSERT INTO \"AlterTest\" (id, color) VALUES ('row2', NULL)")
        .expect("INSERT NULL should now succeed against the optional column");
    adapter
        .exec_raw("UPDATE \"AlterTest\" SET color = NULL WHERE id = 'row1'")
        .expect("UPDATE to NULL should succeed");
}

#[test]
fn alter_field_set_not_null_succeeds_when_data_compatible() {
    // The reverse direction: optional → required. Postgres only accepts
    // SET NOT NULL when every row already has a non-null value, so we
    // pre-populate, then re-plan, then apply. The framework's job is
    // to emit the right SQL — operators are responsible for ensuring
    // the data satisfies the new constraint before applying.
    let Some(url) = pg_url() else {
        return;
    };

    let mut adapter = pylon_storage::postgres::live::LivePostgresAdapter::connect(&url).unwrap();
    let _ = adapter.exec_raw("DROP TABLE IF EXISTS \"TightenTest\" CASCADE");

    let optional = AppManifest {
        entities: vec![ManifestEntity {
            name: "TightenTest".into(),
            fields: vec![ManifestField {
                name: "name".into(),
                field_type: "string".into(),
                optional: true,
                unique: false,
                crdt: None,
            }],
            indexes: vec![],
            relations: vec![],
            crdt: false,
            search: None,
        }],
        ..empty_manifest()
    };
    let initial_plan = adapter.plan_from_live(&optional).unwrap();
    adapter.apply_plan(&initial_plan).unwrap();
    adapter
        .exec_raw("INSERT INTO \"TightenTest\" (id, name) VALUES ('a', 'something')")
        .unwrap();

    let required = AppManifest {
        entities: vec![ManifestEntity {
            name: "TightenTest".into(),
            fields: vec![ManifestField {
                name: "name".into(),
                field_type: "string".into(),
                optional: false,
                unique: false,
                crdt: None,
            }],
            indexes: vec![],
            relations: vec![],
            crdt: false,
            search: None,
        }],
        ..empty_manifest()
    };
    let plan = adapter.plan_from_live(&required).unwrap();
    adapter.apply_plan(&plan).unwrap();

    // INSERT NULL should now fail.
    let err = adapter
        .exec_raw("INSERT INTO \"TightenTest\" (id, name) VALUES ('b', NULL)")
        .unwrap_err();
    assert!(
        err.message.to_lowercase().contains("null"),
        "expected NOT NULL violation, got: {}",
        err.message
    );
}
