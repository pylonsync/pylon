//! The server answers the filters in packages/react/src/query-filter-cases.json
//! with the same rows as the client's local replica filter
//! (packages/react/src/queryFilter.ts, tested by queryFilter.test.ts). A
//! range on a datetime field compares by time, a range on a string field
//! by code unit.

use pylon_kernel::{AppManifest, ManifestEntity, ManifestField};
use pylon_runtime::Runtime;
use serde_json::Value;

fn manifest() -> AppManifest {
    AppManifest {
        entities: vec![ManifestEntity {
            name: "Event".into(),
            crdt: false,
            fields: [
                ("key", "string"),
                ("title", "string"),
                ("occurredAt", "datetime"),
                ("rank", "int"),
            ]
            .into_iter()
            .map(|(name, field_type)| ManifestField {
                name: name.into(),
                field_type: field_type.into(),
                optional: true,
                ..Default::default()
            })
            .collect(),
            ..Default::default()
        }],
        ..Default::default()
    }
}

#[test]
fn the_server_matches_the_shared_filter_cases() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../packages/react/src/query-filter-cases.json"
    );
    let fixture: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let runtime = Runtime::in_memory(manifest()).unwrap();
    for row in fixture["rows"].as_array().unwrap() {
        let mut row = row.clone();
        if row["occurredAt"].is_null() {
            row.as_object_mut().unwrap().remove("occurredAt");
        }
        runtime.insert("Event", &row).unwrap();
    }
    for case in fixture["cases"].as_array().unwrap() {
        let mut filter = case["where"].clone();
        filter["$order"] = serde_json::json!({ "key": "asc" });
        let rows = runtime.query_filtered("Event", &filter).unwrap();
        let keys: Vec<&str> = rows.iter().map(|r| r["key"].as_str().unwrap()).collect();
        let expected: Vec<&str> = case["keys"]
            .as_array()
            .unwrap()
            .iter()
            .map(|k| k.as_str().unwrap())
            .collect();
        assert_eq!(keys, expected, "where {}", case["where"]);
    }
}
