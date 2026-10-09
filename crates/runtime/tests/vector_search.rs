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
        max_length: None,
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
fn vector_search_inside_mutation_tx_does_not_deadlock() {
    // Regression: `ctx.db.vectorSearch` (and `ctx.db.search`) inside a
    // MUTATION runs on TxStore while the Rust side already holds the
    // write-connection mutex. Forwarding to the Runtime impl re-locks
    // that mutex and deadlocks the runner. TxStore must run the scan
    // on its own held connection. This test holds the write conn the
    // way a mutation does; a regression hangs here instead of passing.
    let rt = rt();
    let ids = seed(&rt);

    let conn = rt.lock_conn_pub().unwrap();
    let store = pylon_runtime::datastore::TxStore::new(&rt, &conn);
    let result = DataStore::vector_search(
        &store,
        "Doc",
        &json!({"field": "embedding", "vector": [1.0, 0.0, 0.0, 0.0], "limit": 2}),
    )
    .unwrap();
    let hits = result["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0]["id"], json!(ids[0]));
    assert!(hits[0]["doc"].get("embedding").is_none());
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
    DataStore::crdt_apply_update(&rt, "Note", &id, &update, &|_| Ok(())).unwrap();

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
    let root = &json[pylon_crdt::ROOT_MAP];
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

#[test]
fn vector_search_does_not_wait_for_the_sqlite_writer() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vectors.sqlite");
    let rt = std::sync::Arc::new(Runtime::open(path.to_str().unwrap(), manifest()).unwrap());
    let ids = seed(&rt);
    assert!(rt.read_pool_size() > 0);
    let writer = rt.lock_conn_pub().unwrap();
    let (send, receive) = std::sync::mpsc::channel();
    let reader = rt.clone();
    let thread = std::thread::spawn(move || {
        send.send(DataStore::vector_search(
            &*reader,
            "Doc",
            &json!({
                "field": "embedding", "vector": [1.0, 0.0, 0.0, 0.0], "limit": 2
            }),
        ))
        .unwrap();
    });
    let result = receive.recv_timeout(std::time::Duration::from_secs(5));
    // Release the lock before asserting so regressions cannot leave a blocked worker.
    drop(writer);
    thread.join().unwrap();
    let result = result
        .expect("vector reads must not wait for the writer")
        .unwrap();
    assert_eq!(result["hits"][0]["id"], ids[0]);
    assert_eq!(result["hits"].as_array().unwrap().len(), 2);
}

#[test]
fn vector_documents_keep_json_and_bool_fields_and_omit_all_vectors() {
    let mut schema = manifest();
    schema.entities[0].fields.extend([
        field("otherEmbedding", "vector(2)", true),
        field("metadata", "json", true),
        field("published", "bool", true),
    ]);
    let rt = Runtime::in_memory(schema).unwrap();
    let id = rt
        .insert(
            "Doc",
            &json!({
                "title": "typed", "kind": "a", "embedding": [1.0, 0.0, 0.0, 0.0],
                "otherEmbedding": [1.0, 0.0], "metadata": {"tags": ["a"]}, "published": true
            }),
        )
        .unwrap();
    for in_transaction in [false, true] {
        let query = json!({"field": "embedding", "vector": [1.0, 0.0, 0.0, 0.0]});
        let result = if in_transaction {
            let conn = rt.lock_conn_pub().unwrap();
            let store = pylon_runtime::datastore::TxStore::new(&rt, &conn);
            DataStore::vector_search(&store, "Doc", &query).unwrap()
        } else {
            DataStore::vector_search(&rt, "Doc", &query).unwrap()
        };
        let doc = &result["hits"][0]["doc"];
        assert_eq!(doc["id"], id);
        assert_eq!(doc["metadata"], json!({"tags": ["a"]}));
        assert_eq!(doc["published"], true);
        assert!(doc.get("embedding").is_none());
        assert!(doc.get("otherEmbedding").is_none());
    }
}

thread_local! {
    static VECTOR_SQL: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

#[test]
fn vector_hits_use_one_projected_batch_query() {
    let rt = rt();
    for i in 0..200 {
        rt.insert(
            "Doc",
            &json!({"title": format!("d{i}"), "kind": "a", "embedding": [1.0, 0.0, 0.0, 0.0]}),
        )
        .unwrap();
    }
    for in_transaction in [false, true] {
        VECTOR_SQL.with(|sql| sql.borrow_mut().clear());
        rt.lock_conn_pub().unwrap().trace(Some(|sql| {
            VECTOR_SQL.with(|queries| queries.borrow_mut().push(sql.to_owned()));
        }));
        let query = json!({"field": "embedding", "vector": [1.0, 0.0, 0.0, 0.0], "limit": 200});
        let result = if in_transaction {
            let conn = rt.lock_conn_pub().unwrap();
            let store = pylon_runtime::datastore::TxStore::new(&rt, &conn);
            DataStore::vector_search(&store, "Doc", &query).unwrap()
        } else {
            DataStore::vector_search(&rt, "Doc", &query).unwrap()
        };
        rt.lock_conn_pub().unwrap().trace(None);
        assert_eq!(result["hits"].as_array().unwrap().len(), 200);
        VECTOR_SQL.with(|sql| {
            let queries = sql.borrow();
            let selects: Vec<_> = queries
                .iter()
                .filter(|sql| sql.starts_with("SELECT"))
                .collect();
            assert_eq!(
                selects.len(),
                2,
                "one vector scan and one document query: {selects:?}"
            );
            let projection = selects[1].split(" FROM ").next().unwrap();
            assert!(!projection.contains("embedding"));
            assert!(!projection.contains('*'));
        });
    }
}
