//! `field.vector(dims)` contract: embeddings written as number arrays
//! are stored as packed f32 blobs, read back as number arrays, and
//! searchable via exact k-NN through `DataStore::vector_search` — with
//! dims validation on every write path and vector fields stripped from
//! search hit docs.

use pylon_http::DataStore;
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
    let mut doc = ManifestEntity {
        name: "Doc".into(),
        fields: vec![
            field("title", "string", false),
            field("kind", "string", false),
            field("embedding", "vector(4)", true),
        ],
        indexes: vec![],
        relations: vec![],
        search: None,
        crdt: false,
        sync: true,
        ..Default::default()
    };
    doc.crdt = false;
    let note = ManifestEntity {
        name: "Note".into(),
        fields: vec![
            field("body", "string", false),
            field("embedding", "vector(4)", true),
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
        name: "vector-search-test".into(),
        version: "0.1.0".into(),
        entities: vec![doc, note],
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

// Values chosen to be exactly representable in f32 so round-trip
// equality is exact, not approximate.
fn seed(rt: &Runtime) -> Vec<String> {
    let rows = vec![
        ("d1", "a", json!([1.0, 0.0, 0.0, 0.0])),
        ("d2", "a", json!([0.5, 0.5, 0.0, 0.0])),
        ("d3", "b", json!([0.0, 1.0, 0.0, 0.0])),
    ];
    rows.into_iter()
        .map(|(title, kind, emb)| {
            rt.insert(
                "Doc",
                &json!({"title": title, "kind": kind, "embedding": emb}),
            )
            .unwrap()
        })
        .collect()
}

#[test]
fn embedding_round_trips_as_number_array() {
    let rt = rt();
    let emb = json!([0.25, -1.5, 3.0, 0.0]);
    let id = rt
        .insert("Doc", &json!({"title": "a", "kind": "x", "embedding": emb}))
        .unwrap();
    let row = rt.get_by_id("Doc", &id).unwrap().unwrap();
    assert_eq!(row["embedding"], emb, "get_by_id must decode the blob");
    let listed = rt.list("Doc").unwrap();
    assert_eq!(listed[0]["embedding"], emb, "list must decode the blob");
}

#[test]
fn writes_validate_dims_and_element_types() {
    let rt = rt();
    let err = rt
        .insert(
            "Doc",
            &json!({"title": "a", "kind": "x", "embedding": [1.0, 2.0]}),
        )
        .unwrap_err();
    assert_eq!(err.code, "VECTOR_INVALID");
    assert!(err.message.contains("4 dimensions"), "{}", err.message);

    let err = rt
        .insert(
            "Doc",
            &json!({"title": "a", "kind": "x", "embedding": [1.0, "x", 3.0, 4.0]}),
        )
        .unwrap_err();
    assert_eq!(err.code, "VECTOR_INVALID");

    let err = rt
        .insert(
            "Doc",
            &json!({"title": "a", "kind": "x", "embedding": "not-an-array"}),
        )
        .unwrap_err();
    assert_eq!(err.code, "VECTOR_INVALID");

    // Update path validates too.
    let id = rt
        .insert("Doc", &json!({"title": "a", "kind": "x"}))
        .unwrap();
    let err = rt
        .update("Doc", &id, &json!({"embedding": [1.0]}))
        .unwrap_err();
    assert_eq!(err.code, "VECTOR_INVALID");

    // Null clears an optional embedding.
    rt.update("Doc", &id, &json!({"embedding": null})).unwrap();
    let row = rt.get_by_id("Doc", &id).unwrap().unwrap();
    assert!(row["embedding"].is_null());
}

#[test]
fn vector_search_orders_best_first_and_strips_vectors() {
    let rt = rt();
    let ids = seed(&rt);

    let result = DataStore::vector_search(
        &rt,
        "Doc",
        &json!({"field": "embedding", "vector": [1.0, 0.0, 0.0, 0.0]}),
    )
    .unwrap();
    let hits = result["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 3);
    assert_eq!(hits[0]["id"], json!(ids[0]), "exact match ranks first");
    assert_eq!(hits[0]["doc"]["title"], json!("d1"));
    assert!(
        hits[0]["doc"].get("embedding").is_none(),
        "vector fields must be stripped from hit docs"
    );
    let s0 = hits[0]["score"].as_f64().unwrap();
    let s1 = hits[1]["score"].as_f64().unwrap();
    let s2 = hits[2]["score"].as_f64().unwrap();
    assert!((s0 - 1.0).abs() < 1e-6);
    assert!(s0 > s1 && s1 > s2, "cosine scores descend");
    assert!(result["tookMs"].is_number());
}

#[test]
fn vector_search_limit_filter_and_metric() {
    let rt = rt();
    let ids = seed(&rt);

    // limit
    let result = DataStore::vector_search(
        &rt,
        "Doc",
        &json!({"field": "embedding", "vector": [1.0, 0.0, 0.0, 0.0], "limit": 1}),
    )
    .unwrap();
    assert_eq!(result["hits"].as_array().unwrap().len(), 1);

    // equality filter
    let result = DataStore::vector_search(
        &rt,
        "Doc",
        &json!({
            "field": "embedding",
            "vector": [1.0, 0.0, 0.0, 0.0],
            "filter": {"kind": "b"}
        }),
    )
    .unwrap();
    let hits = result["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["id"], json!(ids[2]));

    // IN filter
    let result = DataStore::vector_search(
        &rt,
        "Doc",
        &json!({
            "field": "embedding",
            "vector": [1.0, 0.0, 0.0, 0.0],
            "filter": {"kind": ["a", "b"]}
        }),
    )
    .unwrap();
    assert_eq!(result["hits"].as_array().unwrap().len(), 3);

    // l2: lower distance = better; exact match first with distance 0.
    let result = DataStore::vector_search(
        &rt,
        "Doc",
        &json!({"field": "embedding", "vector": [0.0, 1.0, 0.0, 0.0], "metric": "l2"}),
    )
    .unwrap();
    let hits = result["hits"].as_array().unwrap();
    assert_eq!(hits[0]["id"], json!(ids[2]));
    assert!(hits[0]["score"].as_f64().unwrap().abs() < 1e-6);
}

#[test]
fn vector_search_validates_request() {
    let rt = rt();
    seed(&rt);

    let err = DataStore::vector_search(
        &rt,
        "Doc",
        &json!({"field": "embedding", "vector": [1.0, 0.0]}),
    )
    .unwrap_err();
    assert_eq!(err.code, "INVALID_QUERY");

    let err = DataStore::vector_search(
        &rt,
        "Doc",
        &json!({"field": "title", "vector": [1.0, 0.0, 0.0, 0.0]}),
    )
    .unwrap_err();
    assert_eq!(err.code, "VECTOR_FIELD_NOT_FOUND");

    let err = DataStore::vector_search(
        &rt,
        "Doc",
        &json!({
            "field": "embedding",
            "vector": [1.0, 0.0, 0.0, 0.0],
            "filter": {"nope": 1}
        }),
    )
    .unwrap_err();
    assert_eq!(err.code, "INVALID_QUERY");

    let err = DataStore::vector_search(
        &rt,
        "Missing",
        &json!({"field": "embedding", "vector": [1.0, 0.0, 0.0, 0.0]}),
    )
    .unwrap_err();
    assert_eq!(err.code, "ENTITY_NOT_FOUND");

    // Rows with no embedding never match.
    rt.insert("Doc", &json!({"title": "bare", "kind": "a"}))
        .unwrap();
    let result = DataStore::vector_search(
        &rt,
        "Doc",
        &json!({"field": "embedding", "vector": [1.0, 0.0, 0.0, 0.0]}),
    )
    .unwrap();
    assert_eq!(result["hits"].as_array().unwrap().len(), 3);
}

#[test]
fn crdt_peer_merge_never_touches_embeddings() {
    // Regression for two review findings on `crdt: true` entities:
    //   1. A peer's CRDT push re-projects the whole doc into the SQL
    //      row; the embedding column must survive untouched (the doc
    //      excludes vector fields entirely).
    //   2. The binary snapshot shipped to clients must not carry the
    //      server-only embedding.
    let rt = rt();
    let emb = serde_json::json!([0.0, 0.0, 1.0, 0.0]);
    let id = rt
        .insert("Note", &json!({"body": "hello", "embedding": emb}))
        .unwrap();

    // Simulate a client peer: import the row's snapshot, edit `body`,
    // push the incremental update back — exactly what /api/crdt does.
    let snapshot = DataStore::crdt_snapshot(&rt, "Note", &id)
        .unwrap()
        .expect("crdt entity must have a snapshot");
    let peer = pylon_crdt::loro::LoroDoc::new();
    peer.import(&snapshot).unwrap();
    let before = peer.oplog_vv();
    pylon_crdt::root_map(&peer)
        .insert("body", "edited by peer")
        .unwrap();
    peer.commit();
    let update = pylon_crdt::encode_update_since(&peer, &before);
    DataStore::crdt_apply_update(&rt, "Note", &id, &update).unwrap();

    let row = rt.get_by_id("Note", &id).unwrap().unwrap();
    assert_eq!(row["body"], json!("edited by peer"), "merge applied");
    assert_eq!(
        row["embedding"], emb,
        "peer merge must not clobber the embedding column"
    );
    let result = DataStore::vector_search(
        &rt,
        "Note",
        &json!({"field": "embedding", "vector": [0.0, 0.0, 1.0, 0.0]}),
    )
    .unwrap();
    assert_eq!(result["hits"].as_array().unwrap().len(), 1);

    // The snapshot a client receives must not contain the embedding —
    // the doc simply has no such key.
    let check = pylon_crdt::loro::LoroDoc::new();
    check.import(&snapshot).unwrap();
    let json = serde_json::to_value(check.get_deep_value()).unwrap();
    let root = &json["root"];
    assert!(
        root.get("embedding").is_none() || root["embedding"].is_null(),
        "binary CRDT frames must not carry server-only embeddings, got {root}"
    );
}

#[test]
fn crdt_entity_supports_vector_fields() {
    let rt = rt();
    let emb = json!([0.0, 0.0, 1.0, 0.0]);
    let id = rt
        .insert("Note", &json!({"body": "hello", "embedding": emb}))
        .unwrap();
    let row = rt.get_by_id("Note", &id).unwrap().unwrap();
    assert_eq!(row["embedding"], emb);

    let result = DataStore::vector_search(
        &rt,
        "Note",
        &json!({"field": "embedding", "vector": [0.0, 0.0, 1.0, 0.0]}),
    )
    .unwrap();
    let hits = result["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["id"], json!(id));
}
