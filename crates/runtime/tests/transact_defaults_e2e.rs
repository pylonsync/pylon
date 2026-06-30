//! Regression: a batched `DataStore::transact` insert — the path a mutation's
//! `ctx.db.insert` routes through — must apply static field defaults
//! (`.default()` / `.defaultNow()`) the same way the entity-API path
//! (`Runtime::insert`) does.
//!
//! Before the fix, `transact` handed each insert op straight to `tx_insert`
//! without `apply_field_defaults`, so a mutation `ctx.db.insert("X", { … })`
//! that omitted a defaulted NOT NULL field failed at the database with
//! `NOT NULL constraint failed`. Surfaced building the live-qa proof app
//! (`createEvent` couldn't insert an Event whose `createdAt` was `defaultNow()`).

use pylon_http::DataStore;
use pylon_kernel::{AppManifest, ManifestEntity, ManifestField};
use pylon_runtime::Runtime;

fn field(name: &str, ft: &str, default: Option<serde_json::Value>) -> ManifestField {
    ManifestField {
        name: name.into(),
        field_type: ft.into(),
        optional: false,
        unique: false,
        crdt: None,
        server_only: false,
        readonly: false,
        default,
        enum_values: None,
        encrypted: false,
    }
}

fn manifest() -> AppManifest {
    AppManifest {
        manifest_version: 1,
        name: "tx-defaults".into(),
        version: "0.1.0".into(),
        entities: vec![ManifestEntity {
            name: "Note".into(),
            fields: vec![
                field("text", "string", None),
                field("done", "boolean", Some(serde_json::json!(false))),
                field("count", "int", Some(serde_json::json!(0))),
                field("createdAt", "datetime", Some(serde_json::json!("now"))),
            ],
            indexes: vec![],
            relations: vec![],
            search: None,
            crdt: false,
            sync: true,
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

#[test]
fn transact_insert_applies_field_defaults() {
    let rt = Runtime::in_memory(manifest()).unwrap();

    // Exactly what `ctx.db.insert("Note", { text })` inside a mutation produces:
    // an insert op that OMITS every defaulted field.
    let ops = vec![serde_json::json!({
        "op": "insert",
        "entity": "Note",
        "data": { "text": "hello" }
    })];

    let (committed, results) = DataStore::transact(&rt, &ops).expect("transact returns Ok");
    assert!(
        committed,
        "the insert must commit, not roll back on a NOT NULL default: {results:?}"
    );
    let id = results[0]
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| panic!("insert should return an id, got: {:?}", results[0]));

    let row = DataStore::get_by_id(&rt, "Note", id)
        .unwrap()
        .expect("the inserted row exists");

    // The defaults must be FILLED — without the fix the insert rolls back on the
    // first missing NOT NULL field, so these slots being present and non-null is
    // the regression signal. (Exact JSON type varies by backend representation —
    // e.g. in-memory SQLite hands booleans back as "0" — so we assert presence,
    // not a literal type.)
    let present = |k: &str| matches!(row.get(k), Some(v) if !v.is_null());
    assert!(
        present("done"),
        "boolean .default(false) must be applied on the transact path"
    );
    assert!(
        present("count"),
        "int .default(0) must be applied on the transact path"
    );
    assert!(
        row.get("createdAt")
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.is_empty()),
        "datetime .defaultNow() must stamp a timestamp on the transact path, got: {:?}",
        row.get("createdAt")
    );
}
