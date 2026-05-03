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
        auth: Default::default(),
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
fn search_on_entity_without_search_config_errors_clearly() {
    use pylon_http::DataStore;
    let Some(url) = pg_url() else {
        return;
    };
    let rt = fresh_runtime(&url);
    // The User entity in `fresh_runtime` declares no `search:` config.
    // The PG search path now rejects with `SEARCH_NOT_CONFIGURED` —
    // same shape as the SQLite path so callers can branch on the
    // code rather than the backend.
    let err = DataStore::search(&rt, "User", &serde_json::json!({"query": "x"})).unwrap_err();
    assert_eq!(err.code, "SEARCH_NOT_CONFIGURED");

    // `$search` inside query_filtered references a `_fts_<entity>`
    // table the planner only creates for searchable entities. On a
    // non-searchable entity the planner emits no shadow table, so
    // the operator hits PG with a missing-table error. Surfaced as
    // PG_QUERY_FAILED, which the SDK can map back to a clear message.
    let err = rt
        .query_filtered("User", &serde_json::json!({"$search": "anything"}))
        .unwrap_err();
    assert!(
        err.code == "PG_QUERY_FAILED" || err.code.contains("FTS"),
        "expected a clear error referencing the missing FTS table; got: {err:?}"
    );
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

#[test]
fn timestamptz_binds_iso_string_correctly() {
    // Regression: the OAuth callback writes ISO 8601 strings into
    // TIMESTAMPTZ columns (User.createdAt / User.emailVerified). The
    // earlier impl bound the raw ASCII bytes through &str::to_sql,
    // which Postgres rejected with "incorrect binary data format in
    // bind parameter N." This test pushes through the same code path
    // (Runtime::insert against an entity with datetime fields) and
    // confirms the row lands without a wire-format error.
    let Some(url) = pg_url() else {
        return;
    };

    let manifest = AppManifest {
        entities: vec![ManifestEntity {
            name: "TsTest".into(),
            fields: vec![
                ManifestField {
                    name: "label".into(),
                    field_type: "string".into(),
                    optional: false,
                    unique: false,
                    crdt: None,
                },
                ManifestField {
                    name: "createdAt".into(),
                    field_type: "datetime".into(),
                    optional: false,
                    unique: false,
                    crdt: None,
                },
                ManifestField {
                    name: "verifiedAt".into(),
                    field_type: "datetime".into(),
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
        .exec_raw("DROP TABLE IF EXISTS \"TsTest\" CASCADE")
        .unwrap();
    let plan = adapter.plan_from_live(&manifest).unwrap();
    adapter.apply_plan(&plan).unwrap();

    let rt = Runtime::open_postgres(&url, manifest).unwrap();
    let id = rt
        .insert(
            "TsTest",
            &serde_json::json!({
                "label": "row1",
                "createdAt": "2026-04-29T14:28:34Z",
                "verifiedAt": serde_json::Value::Null,
            }),
        )
        .expect("TIMESTAMPTZ insert should succeed");

    let row = rt.get_by_id("TsTest", &id).unwrap().unwrap();
    assert_eq!(row["label"], "row1");
    // PG returns timestamps as ISO strings via row_to_json — exact
    // format may differ (with/without 'T' / fractional seconds) but
    // the year + month + day must round-trip.
    let created = row["createdAt"]
        .as_str()
        .expect("createdAt should round-trip as a string");
    assert!(
        created.starts_with("2026-04-29"),
        "expected createdAt to start with 2026-04-29, got {created:?}"
    );
    assert!(
        row["verifiedAt"].is_null(),
        "nullable TIMESTAMPTZ should round-trip as JSON null, got {:?}",
        row["verifiedAt"]
    );
}

#[test]
fn query_filtered_like_substring_match_matches_sqlite() {
    // Codex P2: `$like: "ann"` was substring-matching on SQLite (wraps
    // in %...%) but exact-pattern-matching on PG (forwarded literally).
    // Now the PG path wraps too — `{name: $like "ann"}` finds "Joanne".
    let Some(url) = pg_url() else {
        return;
    };
    let rt = fresh_runtime(&url);
    rt.insert(
        "User",
        &serde_json::json!({"email": "j@x.com", "name": "Joanne"}),
    )
    .unwrap();
    rt.insert(
        "User",
        &serde_json::json!({"email": "b@x.com", "name": "Bob"}),
    )
    .unwrap();
    let hits = rt
        .query_filtered("User", &serde_json::json!({"name": {"$like": "ann"}}))
        .unwrap();
    assert_eq!(
        hits.len(),
        1,
        "expected substring match on Joanne, got {hits:?}"
    );
    assert_eq!(hits[0]["name"], "Joanne");
}

#[test]
fn query_filtered_empty_in_returns_no_rows_no_sql_error() {
    // Codex P2: `$in: []` was emitting `field IN ()` which Postgres
    // rejects as a syntax error. Now short-circuits to FALSE → empty
    // result set, matching the SQLite path's behavior.
    let Some(url) = pg_url() else {
        return;
    };
    let rt = fresh_runtime(&url);
    rt.insert(
        "User",
        &serde_json::json!({"email": "x@x.com", "name": "X"}),
    )
    .unwrap();
    let hits = rt
        .query_filtered("User", &serde_json::json!({"email": {"$in": []}}))
        .expect("empty $in should not fail with a SQL error");
    assert_eq!(hits.len(), 0);
}

#[test]
fn query_filtered_default_order_by_id_matches_sqlite() {
    // Codex P2: SQLite filtered queries defaulted to ORDER BY id, PG
    // had no default order — same query returned rows in different
    // orders across backends. Now both default to ORDER BY id.
    let Some(url) = pg_url() else {
        return;
    };
    let rt = fresh_runtime(&url);
    let mut ids = Vec::new();
    for i in 0..5 {
        ids.push(
            rt.insert(
                "User",
                &serde_json::json!({"email": format!("o{i}@x.com"), "name": format!("o{i}")}),
            )
            .unwrap(),
        );
    }
    let hits = rt.query_filtered("User", &serde_json::json!({})).unwrap();
    let returned_ids: Vec<&str> = hits.iter().map(|r| r["id"].as_str().unwrap()).collect();
    let mut sorted = ids.clone();
    sorted.sort();
    let expected: Vec<&str> = sorted.iter().map(|s| s.as_str()).collect();
    assert_eq!(
        returned_ids, expected,
        "rows should come back in id order without explicit $order"
    );
}

// ---------------------------------------------------------------------------
// CRDT integration — exercises PgLoroStore + sidecar table + reprojection
// ---------------------------------------------------------------------------

fn crdt_runtime(url: &str) -> Runtime {
    let manifest = AppManifest {
        entities: vec![ManifestEntity {
            name: "Note".into(),
            fields: vec![
                ManifestField {
                    name: "title".into(),
                    field_type: "string".into(),
                    optional: false,
                    unique: false,
                    crdt: None,
                },
                ManifestField {
                    name: "body".into(),
                    field_type: "string".into(),
                    optional: true,
                    unique: false,
                    crdt: None,
                },
            ],
            indexes: vec![],
            relations: vec![],
            // crdt: true opts the entity into the LoroDoc projection
            // path. Default field shape is LWW, which we exercise here.
            crdt: true,
            search: None,
        }],
        ..empty_manifest()
    };
    let mut adapter = pylon_storage::postgres::live::LivePostgresAdapter::connect(url)
        .expect("connect to test postgres");
    let _ = adapter.exec_raw("DROP TABLE IF EXISTS \"Note\" CASCADE");
    let _ = adapter.exec_raw("DROP TABLE IF EXISTS _pylon_crdt_snapshots CASCADE");
    let plan = adapter
        .plan_from_live(&manifest)
        .expect("plan against fresh schema");
    adapter.apply_plan(&plan).expect("apply schema");
    Runtime::open_postgres(url, manifest).expect("open postgres runtime")
}

#[test]
fn crdt_snapshot_roundtrips_on_postgres() {
    let Some(url) = pg_url() else {
        return;
    };
    use pylon_http::DataStore;
    let rt = crdt_runtime(&url);
    let id = rt
        .insert(
            "Note",
            &serde_json::json!({"title": "hello", "body": "world"}),
        )
        .unwrap();

    // Snapshot returns the encoded LoroDoc bytes — non-empty after a
    // CRDT-mode insert because the apply_patch ran in the PG path.
    let snap = DataStore::crdt_snapshot(&rt, "Note", &id)
        .expect("crdt_snapshot")
        .expect("Some(snap)");
    assert!(
        !snap.is_empty(),
        "snapshot should be non-empty after insert"
    );

    // Sidecar row exists.
    let mut client = postgres::Client::connect(&url, postgres::NoTls).unwrap();
    let row = client
        .query_one(
            "SELECT COUNT(*) FROM _pylon_crdt_snapshots WHERE entity = 'Note' AND row_id = $1",
            &[&id],
        )
        .expect("sidecar count");
    let count: i64 = row.get(0);
    assert_eq!(count, 1);
}

#[test]
fn crdt_insert_failure_rolls_back_snapshot() {
    // Atomicity regression: a failed entity insert must not leave a
    // stale snapshot in `_pylon_crdt_snapshots`. Pre-fix the snapshot
    // landed in autocommit before the insert ran, so a CHECK / FK /
    // type-cast violation on the entity row would orphan the snapshot.
    let Some(url) = pg_url() else {
        return;
    };
    let rt = crdt_runtime(&url);
    // Force the entity insert to fail with an unknown column. Anything
    // that breaks the SQL after `apply_patch` succeeds works as the
    // probe; this is the easiest one to trigger from outside.
    let bad_insert = rt.insert(
        "Note",
        &serde_json::json!({"title": "x", "definitely_not_a_column": 1}),
    );
    assert!(bad_insert.is_err(), "insert should have failed");

    // Sidecar must have NO row for any Note id — the failed insert's
    // snapshot was rolled back along with the row write.
    let mut client = postgres::Client::connect(&url, postgres::NoTls).unwrap();
    let count: i64 = client
        .query_one(
            "SELECT COUNT(*) FROM _pylon_crdt_snapshots WHERE entity = 'Note'",
            &[],
        )
        .unwrap()
        .get(0);
    assert_eq!(
        count, 0,
        "failed entity insert must roll back the CRDT snapshot too"
    );
}

#[test]
fn crdt_apply_update_reprojects_to_postgres_row() {
    let Some(url) = pg_url() else {
        return;
    };
    use pylon_http::DataStore;
    let rt = crdt_runtime(&url);
    let id = rt
        .insert("Note", &serde_json::json!({"title": "v1", "body": ""}))
        .unwrap();

    // Take the current snapshot so we can build a remote update from
    // a divergent LoroDoc and feed it back through `crdt_apply_update`.
    // The simplest divergent op: another LoroDoc applies a different
    // title, exports its update, and we ship that to the runtime.
    use pylon_crdt::{encode_snapshot, loro::LoroDoc, root_map};
    let snap = DataStore::crdt_snapshot(&rt, "Note", &id).unwrap().unwrap();
    let peer = LoroDoc::new();
    pylon_crdt::apply_update(&peer, &snap).unwrap();
    // The Pylon CRDT shape stores fields in a root map keyed `"row"` —
    // matching what `pylon_crdt::root_map` returns. Insert directly
    // there so the projection picks up our new value.
    root_map(&peer).insert("title", "v2-from-peer").unwrap();
    peer.commit();
    let update = encode_snapshot(&peer);

    let new_snap =
        DataStore::crdt_apply_update(&rt, "Note", &id, &update).expect("crdt_apply_update");
    assert!(!new_snap.is_empty());

    // The materialized row's title column should now reflect the
    // peer's value because crdt_apply_update re-projects into PG.
    let row = rt.get_by_id("Note", &id).unwrap().unwrap();
    assert_eq!(row["title"], "v2-from-peer");
}

// ---------------------------------------------------------------------------
// FTS integration — exercises pg_search create + maintenance + run_search
// ---------------------------------------------------------------------------

fn fts_runtime(url: &str) -> Runtime {
    let manifest = AppManifest {
        entities: vec![ManifestEntity {
            name: "Product".into(),
            fields: vec![
                ManifestField {
                    name: "name".into(),
                    field_type: "string".into(),
                    optional: false,
                    unique: false,
                    crdt: None,
                },
                ManifestField {
                    name: "description".into(),
                    field_type: "string".into(),
                    optional: true,
                    unique: false,
                    crdt: None,
                },
                ManifestField {
                    name: "brand".into(),
                    field_type: "string".into(),
                    optional: false,
                    unique: false,
                    crdt: None,
                },
            ],
            indexes: vec![],
            relations: vec![],
            crdt: false,
            search: Some(ManifestSearchConfig {
                text: vec!["name".into(), "description".into()],
                facets: vec!["brand".into()],
                sortable: vec![],
                language: None,
            }),
        }],
        ..empty_manifest()
    };
    let mut adapter = pylon_storage::postgres::live::LivePostgresAdapter::connect(url)
        .expect("connect to test postgres");
    let _ = adapter.exec_raw("DROP TABLE IF EXISTS \"_fts_Product\" CASCADE");
    let _ = adapter.exec_raw("DROP TABLE IF EXISTS \"Product\" CASCADE");
    let plan = adapter
        .plan_from_live(&manifest)
        .expect("plan against fresh schema");
    adapter.apply_plan(&plan).expect("apply schema");
    Runtime::open_postgres(url, manifest).expect("open postgres runtime")
}

#[test]
fn fts_insert_writes_fts_shadow_row_on_postgres() {
    let Some(url) = pg_url() else {
        return;
    };
    let rt = fts_runtime(&url);
    let _id = rt
        .insert(
            "Product",
            &serde_json::json!({
                "name": "Atlas runner",
                "description": "lightweight trail shoe",
                "brand": "Atlas",
            }),
        )
        .unwrap();

    let mut client = postgres::Client::connect(&url, postgres::NoTls).unwrap();
    let row = client
        .query_one("SELECT COUNT(*) FROM \"_fts_Product\"", &[])
        .expect("fts shadow row count");
    let count: i64 = row.get(0);
    assert_eq!(count, 1, "FTS shadow row should exist after insert");
}

#[test]
fn aggregate_inside_pg_mutation_tx_sees_pending_writes() {
    // Regression: PgTxStore::aggregate previously returned
    // NOT_SUPPORTED_IN_TX. The fix wires the same SQL builder the
    // non-tx path uses so aggregates inside a TS mutation handler
    // run through the held tx and see the handler's own pending
    // writes.
    let Some(url) = pg_url() else {
        return;
    };
    use pylon_storage::pg_datastore::PostgresDataStore;
    let _ = pylon_storage::postgres::live::LivePostgresAdapter::connect(&url)
        .unwrap()
        .exec_raw("DROP TABLE IF EXISTS \"User\" CASCADE");
    let rt = fresh_runtime(&url);
    let store: &PostgresDataStore = rt.pg_data_store_for_tests();
    let count = store
        .with_transaction::<_, serde_json::Value, pylon_http::DataError>(|s| {
            s.insert(
                "User",
                &serde_json::json!({"email": "a@x.com", "name": "a"}),
            )?;
            s.insert(
                "User",
                &serde_json::json!({"email": "b@x.com", "name": "b"}),
            )?;
            // Aggregate runs through the held tx — must see both
            // pending inserts even though they haven't committed yet.
            s.aggregate("User", &serde_json::json!({"count": "*"}))
        })
        .expect("aggregate inside tx should succeed");
    assert_eq!(count["rows"][0]["count"], 2);
}

#[test]
fn crdt_update_on_missing_row_rolls_back_snapshot() {
    // Codex regression: previously apply_patch persisted a snapshot
    // before tx_update ran. If tx_update found no row, the snapshot
    // committed alone — orphaned state pointing at a non-existent
    // row. The fix: tx_update returning false bubbles up as
    // ENTITY_NOT_FOUND so the with_transaction_raw closure rolls back.
    let Some(url) = pg_url() else {
        return;
    };
    let rt = crdt_runtime(&url);
    let updated = rt
        .update("Note", "no-such-id", &serde_json::json!({"title": "ghost"}))
        .expect("update returns Ok(false), not an error");
    assert!(!updated);

    let mut client = postgres::Client::connect(&url, postgres::NoTls).unwrap();
    let count: i64 = client
        .query_one(
            "SELECT COUNT(*) FROM _pylon_crdt_snapshots WHERE entity = 'Note' AND row_id = 'no-such-id'",
            &[],
        )
        .unwrap()
        .get(0);
    assert_eq!(
        count, 0,
        "no snapshot should have been committed for a missing row"
    );
}

#[test]
fn pg_transact_maintains_fts_shadow() {
    // Codex regression: PG /api/transact bypassed tx_insert so FTS
    // shadow rows weren't maintained for batched admin writes. Now
    // PostgresDataStore::transact runs each op through tx_insert/
    // tx_update/tx_delete so FTS stays in sync.
    let Some(url) = pg_url() else {
        return;
    };
    use pylon_http::DataStore;
    let rt = fts_runtime(&url);
    let store = rt.pg_data_store_for_tests();
    let (_committed, results) = store
        .transact(&[serde_json::json!({
            "op": "insert",
            "entity": "Product",
            "data": {
                "name": "tx-batched",
                "description": "lands via /api/transact",
                "brand": "Atlas",
            }
        })])
        .expect("transact succeeds");
    let inserted_id = results[0]["id"].as_str().unwrap().to_string();

    let mut client = postgres::Client::connect(&url, postgres::NoTls).unwrap();
    let row = client
        .query_one(
            "SELECT COUNT(*) FROM \"_fts_Product\" WHERE entity_id = $1",
            &[&inserted_id],
        )
        .unwrap();
    let count: i64 = row.get(0);
    assert_eq!(
        count, 1,
        "FTS shadow row must exist after /api/transact insert"
    );
}

#[test]
fn pgtxstore_crdt_hook_persists_sidecar_on_insert() {
    // Codex regression #3: TS mutation handlers go through
    // FnOpsImpl::call PG branch, which constructs a PgTxStore
    // wrapped in PgBufferedTxStore. Pre-fix that wrapper just
    // forwarded insert/update/delete to tx_insert/update/delete
    // without CRDT projection. Now PgTxStore::with_crdt installs
    // PgCrdtHookImpl so writes on crdt:true entities persist the
    // sidecar in the same tx.
    //
    // Direct test: use PostgresDataStore::with_transaction_crdt to
    // simulate what FnOpsImpl does.
    let Some(url) = pg_url() else {
        return;
    };
    let rt = crdt_runtime(&url);
    let id = rt
        .run_in_pg_mutation_tx_for_tests::<_, String, pylon_http::DataError>(|store| {
            store.insert(
                "Note",
                &serde_json::json!({"title": "via-mutation", "body": "x"}),
            )
        })
        .expect("with_transaction_crdt insert");

    // Sidecar row exists.
    let mut client = postgres::Client::connect(&url, postgres::NoTls).unwrap();
    let count: i64 = client
        .query_one(
            "SELECT COUNT(*) FROM _pylon_crdt_snapshots WHERE entity = 'Note' AND row_id = $1",
            &[&id],
        )
        .unwrap()
        .get(0);
    assert_eq!(
        count, 1,
        "TS-mutation insert via CRDT hook must create sidecar row"
    );
}

#[test]
fn pg_transact_maintains_crdt_sidecar_for_crdt_entities() {
    // Codex regression #2: /api/transact previously bypassed CRDT
    // maintenance — an insert against a `crdt: true` entity wrote
    // the materialized row but no `_pylon_crdt_snapshots` row, so
    // crdt_snapshot returned an empty doc and binary CRDT broadcasts
    // were silently broken. Now Runtime::transact (the DataStore
    // impl) routes through pg_transact_with_crdt which projects
    // through PgLoroStore for crdt:true entities.
    let Some(url) = pg_url() else {
        return;
    };
    use pylon_http::DataStore;
    let rt = crdt_runtime(&url);
    let (_committed, results) = DataStore::transact(
        &rt,
        &[serde_json::json!({
            "op": "insert",
            "entity": "Note",
            "data": {"title": "from-transact", "body": "via /api/transact"}
        })],
    )
    .expect("transact succeeds");
    let id = results[0]["id"].as_str().unwrap().to_string();

    // Sidecar row exists.
    let mut client = postgres::Client::connect(&url, postgres::NoTls).unwrap();
    let count: i64 = client
        .query_one(
            "SELECT COUNT(*) FROM _pylon_crdt_snapshots WHERE entity = 'Note' AND row_id = $1",
            &[&id],
        )
        .unwrap()
        .get(0);
    assert_eq!(
        count, 1,
        "transact insert on crdt:true entity must create sidecar row"
    );

    // crdt_snapshot returns non-empty bytes.
    let snap = DataStore::crdt_snapshot(&rt, "Note", &id).unwrap().unwrap();
    assert!(!snap.is_empty());
}

#[test]
fn pg_update_rejects_id_mutation() {
    // Codex regression: build_update_sql used to include `id` in the
    // SET clause when present in the patch — letting a client move a
    // row out from under its CRDT sidecar / FTS shadow keys. Now it
    // errors with PG_INVALID_UPDATE.
    let Some(url) = pg_url() else {
        return;
    };
    let rt = fresh_runtime(&url);
    let id = rt
        .insert(
            "User",
            &serde_json::json!({"email": "z@x.com", "name": "z"}),
        )
        .unwrap();
    let err = rt
        .update(
            "User",
            &id,
            &serde_json::json!({"id": "different-id", "name": "z2"}),
        )
        .unwrap_err();
    assert_eq!(err.code, "PG_INVALID_UPDATE");
}

#[test]
fn fts_insert_failure_rolls_back_shadow_row() {
    // Atomicity regression: a failed entity insert must not leave a
    // stale FTS shadow row. Pre-fix the FTS apply_insert ran in the
    // same `with_transaction` as the entity insert via PgTxStore, so
    // this is mainly a guard against future refactors that might
    // split them again.
    let Some(url) = pg_url() else {
        return;
    };
    let rt = fts_runtime(&url);
    let bad = rt.insert(
        "Product",
        &serde_json::json!({"name": "x", "definitely_not_a_column": 1}),
    );
    assert!(bad.is_err(), "insert should have failed");

    let mut client = postgres::Client::connect(&url, postgres::NoTls).unwrap();
    let count: i64 = client
        .query_one("SELECT COUNT(*) FROM \"_fts_Product\"", &[])
        .unwrap()
        .get(0);
    assert_eq!(
        count, 0,
        "failed entity insert must roll back the FTS shadow row too"
    );
}

#[test]
fn fts_search_returns_matched_rows_on_postgres() {
    let Some(url) = pg_url() else {
        return;
    };
    use pylon_http::DataStore;
    let rt = fts_runtime(&url);
    rt.insert(
        "Product",
        &serde_json::json!({
            "name": "Atlas runner",
            "description": "lightweight trail shoe",
            "brand": "Atlas",
        }),
    )
    .unwrap();
    rt.insert(
        "Product",
        &serde_json::json!({
            "name": "Summit jacket",
            "description": "waterproof hiking shell",
            "brand": "Summit",
        }),
    )
    .unwrap();

    let result = DataStore::search(
        &rt,
        "Product",
        &serde_json::json!({
            "query": "trail",
            "facets": ["brand"],
            "page": 0,
            "pageSize": 10,
        }),
    )
    .expect("search returns Ok");

    let hits = result["hits"].as_array().expect("hits array");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["name"], "Atlas runner");
    assert_eq!(result["total"], 1);

    // Facet exclusion: the brand facet should report Atlas:1 even
    // though we didn't filter on brand. The non-matching Summit row
    // is excluded by the text query, not by the facet.
    let facets = result["facetCounts"]["brand"]
        .as_object()
        .expect("brand facets");
    assert_eq!(facets["Atlas"], 1);
}
