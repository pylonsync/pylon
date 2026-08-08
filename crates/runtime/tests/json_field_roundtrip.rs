//! `field.json()` round-trip contract: values are stored as serialized
//! TEXT but every read path hands back the PARSED value — including the
//! shape-ambiguous cases (a JSON string `"42"` must come back as the
//! string `"42"`, never the number 42), on both the plain-column path
//! (`crdt: false`) and the CRDT LwwJson path (`crdt: true`, the entity
//! default).

use pylon_kernel::*;
use pylon_runtime::Runtime;
use serde_json::json;

fn field(name: &str, ftype: &str, optional: bool) -> ManifestField {
    ManifestField {
        name: name.into(),
        field_type: ftype.into(),
        optional,
        unique: false,
        crdt: None,
        server_only: false,
        readonly: false,
        default: None,
        enum_values: None,
        encrypted: false,
        sync_omit: false,
    }
}

fn manifest() -> AppManifest {
    let mut plain = ManifestEntity {
        name: "Doc".into(),
        fields: vec![
            field("title", "string", false),
            field("config", "json", false),
            field("meta", "json", true),
        ],
        indexes: vec![],
        relations: vec![],
        search: None,
        crdt: false,
        sync: true,
        ..Default::default()
    };
    plain.crdt = false;
    let crdt_ent = ManifestEntity {
        name: "Board".into(),
        fields: vec![
            field("name", "string", false),
            field("layout", "json", false),
        ],
        indexes: vec![],
        relations: vec![],
        search: None,
        crdt: true,
        sync: true,
        ..Default::default()
    };
    AppManifest {
        manifest_version: 1,
        name: "json-roundtrip-test".into(),
        version: "0.1.0".into(),
        entities: vec![plain, crdt_ent],
        routes: vec![],
        queries: vec![],
        actions: vec![],
        policies: vec![],
        ..Default::default()
    }
}

fn rt() -> Runtime {
    Runtime::in_memory(manifest()).unwrap()
}

#[test]
fn object_round_trips_parsed_on_plain_entity() {
    let rt = rt();
    let cfg = json!({"theme": "dark", "cols": 3, "flags": [true, false]});
    let id = rt
        .insert("Doc", &json!({"title": "a", "config": cfg}))
        .unwrap();
    let row = rt.get_by_id("Doc", &id).unwrap().unwrap();
    assert_eq!(row["config"], cfg, "get_by_id must return the parsed value");
    let listed = rt.list("Doc").unwrap();
    assert_eq!(listed[0]["config"], cfg, "list must return the parsed value");
}

#[test]
fn shape_ambiguous_scalars_round_trip_exactly() {
    let rt = rt();
    // Each value the storage TEXT form could mis-parse back to a
    // different shape if the write side didn't serialize uniformly.
    let cases = vec![
        json!("42"),             // string that parses as a number
        json!("true"),           // string that parses as a bool
        json!("null"),           // string that parses as null
        json!("{\"a\":1}"),      // string that parses as an object
        json!(42),               // real number
        json!(true),             // real bool
        json!([1, "two", null]), // array
    ];
    for v in cases {
        let id = rt
            .insert("Doc", &json!({"title": "x", "config": v}))
            .unwrap();
        let row = rt.get_by_id("Doc", &id).unwrap().unwrap();
        assert_eq!(row["config"], v, "value {v} must round-trip exactly");
    }
}

#[test]
fn update_and_optional_null_round_trip() {
    let rt = rt();
    let id = rt
        .insert(
            "Doc",
            &json!({"title": "a", "config": {"v": 1}, "meta": {"k": "x"}}),
        )
        .unwrap();
    rt.update("Doc", &id, &json!({"config": {"v": 2, "extra": [1]}}))
        .unwrap();
    let row = rt.get_by_id("Doc", &id).unwrap().unwrap();
    assert_eq!(row["config"], json!({"v": 2, "extra": [1]}));
    assert_eq!(row["meta"], json!({"k": "x"}), "untouched field survives");

    rt.update("Doc", &id, &json!({"meta": null})).unwrap();
    let row = rt.get_by_id("Doc", &id).unwrap().unwrap();
    assert_eq!(row["meta"], serde_json::Value::Null);
}

#[test]
fn equality_filter_matches_serialized_form() {
    let rt = rt();
    let cfg = json!({"kind": "a"});
    rt.insert("Doc", &json!({"title": "one", "config": cfg}))
        .unwrap();
    rt.insert("Doc", &json!({"title": "two", "config": {"kind": "b"}}))
        .unwrap();
    let hits = rt
        .query_filtered("Doc", &json!({"config": cfg}))
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["title"], "one");
    assert_eq!(hits[0]["config"], cfg, "filtered rows come back parsed");
}

#[test]
fn crdt_entity_json_field_round_trips() {
    let rt = rt();
    let layout = json!({"panes": [{"id": "p1", "w": 2}], "grid": true});
    let id = rt
        .insert("Board", &json!({"name": "b", "layout": layout}))
        .unwrap();
    let row = rt.get_by_id("Board", &id).unwrap().unwrap();
    assert_eq!(row["layout"], layout, "CRDT LwwJson projects parsed");

    let layout2 = json!({"panes": [], "grid": false, "note": "n"});
    rt.update("Board", &id, &json!({"layout": layout2})).unwrap();
    let row = rt.get_by_id("Board", &id).unwrap().unwrap();
    assert_eq!(row["layout"], layout2, "whole-value LWW replaces");
}

#[test]
fn crdt_json_string_scalar_round_trips_as_string() {
    let rt = rt();
    let id = rt
        .insert("Board", &json!({"name": "b", "layout": "42"}))
        .unwrap();
    let row = rt.get_by_id("Board", &id).unwrap().unwrap();
    assert_eq!(
        row["layout"],
        json!("42"),
        "a JSON string value must not come back as a number"
    );
}
