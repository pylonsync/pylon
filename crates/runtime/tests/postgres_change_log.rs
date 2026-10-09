//! Run against an isolated database with TEST_POSTGRES_URL.
use postgres::{Client, NoTls};
use pylon_kernel::AppManifest;
use pylon_runtime::Runtime;
use pylon_sync::{ChangeEvent, ChangeKind};
use serde_json::{json, Value};

fn event(seq: u64) -> ChangeEvent {
    ChangeEvent {
        seq,
        entity: "Note".into(),
        row_id: format!("row-{seq}"),
        kind: match seq % 3 {
            0 => ChangeKind::Insert,
            1 => ChangeKind::Update,
            _ => ChangeKind::Delete,
        },
        data: match seq % 3 {
            0 => None,
            1 => Some(Value::Null),
            _ => Some(json!({"title":"quote ' and unicode λ", "seq":seq})),
        },
        prev_data: match seq % 3 {
            0 => Some(json!({"owner":"before"})),
            1 => None,
            _ => Some(Value::Null),
        },
        timestamp: "2026-10-09T00:00:00Z".into(),
    }
}

#[test]
fn postgres_change_batches_preserve_payloads_deduplication_and_atomicity() {
    let Ok(url) = std::env::var("TEST_POSTGRES_URL") else {
        return;
    };
    let runtime = Runtime::open_postgres(&url, AppManifest::default()).unwrap();
    runtime.bootstrap_pg_change_log().unwrap();
    let mut client = Client::connect(&url, NoTls).unwrap();
    client
        .batch_execute(
            "TRUNCATE pylon_change_log;
        CREATE OR REPLACE FUNCTION count_change_batch() RETURNS trigger AS $$
        BEGIN INSERT INTO change_batch_counts VALUES (1); RETURN NULL; END; $$ LANGUAGE plpgsql;
        CREATE TABLE IF NOT EXISTS change_batch_counts (n integer);
        TRUNCATE change_batch_counts;
        DROP TRIGGER IF EXISTS count_change_batch ON pylon_change_log;
        CREATE TRIGGER count_change_batch AFTER INSERT ON pylon_change_log
        FOR EACH STATEMENT EXECUTE FUNCTION count_change_batch();",
        )
        .unwrap();
    let mut events: Vec<_> = (1..=600).map(event).collect();
    let mut duplicate = event(1);
    duplicate.data = Some(json!({"must_not_replace":true}));
    events.insert(10, duplicate.clone());
    events.push(duplicate);
    runtime.pg_change_log_persist_batch(&events).unwrap();
    let loaded = runtime.pg_change_log_load_recent(1000);
    assert_eq!(loaded, (1..=600).map(event).collect::<Vec<_>>());
    let inserts: i64 = client
        .query_one("SELECT count(*) FROM change_batch_counts", &[])
        .unwrap()
        .get(0);
    assert_eq!(inserts, 3, "one SQL INSERT per 256-event chunk");
    let nulls = client.query("SELECT seq, data IS NULL, data = 'null'::jsonb FROM pylon_change_log WHERE seq IN (1,3) ORDER BY seq", &[]).unwrap();
    assert!(!nulls[0].get::<_, bool>(1));
    assert!(nulls[0].get::<_, bool>(2));
    assert!(nulls[1].get::<_, bool>(1));

    let mut large = vec![event(700), event(701), event(702)];
    large[0].data = Some(json!({"text": "\u{0001}".repeat(200_000)}));
    large[1].prev_data = Some(json!({"text": "large".repeat(40_000)}));
    runtime.pg_change_log_persist_batch(&large).unwrap();
    let inserts: i64 = client
        .query_one("SELECT count(*) FROM change_batch_counts", &[])
        .unwrap()
        .get(0);
    assert_eq!(
        inserts, 6,
        "large event payloads use separate INSERT buffers"
    );
    assert_eq!(runtime.pg_change_log_load_recent(3), large);

    client.batch_execute("ALTER TABLE pylon_change_log ADD CONSTRAINT reject_test_event CHECK (entity <> 'reject')").unwrap();
    let mut invalid: Vec<_> = (1000..=1300).map(event).collect();
    invalid.last_mut().unwrap().entity = "reject".into();
    assert!(runtime.pg_change_log_persist_batch(&invalid).is_err());
    let count: i64 = client
        .query_one(
            "SELECT count(*) FROM pylon_change_log WHERE seq >= 1000",
            &[],
        )
        .unwrap()
        .get(0);
    assert_eq!(count, 0, "a later chunk failure rolls back earlier chunks");
    client
        .batch_execute(
            "ALTER TABLE pylon_change_log DROP CONSTRAINT reject_test_event;
        DROP TRIGGER count_change_batch ON pylon_change_log;
        DROP FUNCTION count_change_batch(); DROP TABLE change_batch_counts;",
        )
        .unwrap();
}

#[test]
fn postgres_mutation_crdt_documents_exclude_server_only_vectors() {
    use pylon_http::DataStore;
    use pylon_kernel::{ManifestEntity, ManifestField};
    let Ok(url) = std::env::var("TEST_POSTGRES_URL") else {
        return;
    };
    let mut client = Client::connect(&url, NoTls).unwrap();
    client
        .batch_execute(
            "DROP TABLE IF EXISTS \"CrdtVectorNote\";
        CREATE TABLE \"CrdtVectorNote\" (id text PRIMARY KEY, title text, embedding bytea)",
        )
        .unwrap();
    let runtime = Runtime::open_postgres(
        &url,
        AppManifest {
            entities: vec![ManifestEntity {
                name: "CrdtVectorNote".into(),
                crdt: true,
                fields: vec![
                    ManifestField {
                        name: "title".into(),
                        field_type: "string".into(),
                        ..Default::default()
                    },
                    ManifestField {
                        name: "embedding".into(),
                        field_type: "vector(3)".into(),
                        optional: true,
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        },
    )
    .unwrap();
    let id = runtime
        .run_in_pg_mutation_tx_for_tests::<_, _, pylon_http::DataError>(|store| {
            store.insert(
                "CrdtVectorNote",
                &json!({"title":"first", "embedding":[1.0,2.0,3.0]}),
            )
        })
        .unwrap();
    for expected in ["first", "updated"] {
        if expected == "updated" {
            runtime
                .run_in_pg_mutation_tx_for_tests::<_, _, pylon_http::DataError>(|store| {
                    store.update(
                        "CrdtVectorNote",
                        &id,
                        &json!({"title":"updated", "embedding":[3.0,2.0,1.0]}),
                    )
                })
                .unwrap();
        }
        let snapshot = DataStore::crdt_snapshot(&runtime, "CrdtVectorNote", &id)
            .unwrap()
            .unwrap();
        let doc = pylon_crdt::loro::LoroDoc::new();
        doc.import(&snapshot).unwrap();
        let value = serde_json::to_value(doc.get_deep_value()).unwrap();
        assert_eq!(
            value[pylon_crdt::ROOT_MAP]["title"],
            expected,
            "snapshot: {value}"
        );
        assert!(
            value[pylon_crdt::ROOT_MAP].get("embedding").is_none(),
            "server-only vector leaked into CRDT snapshot: {value}"
        );
        let bytes: Vec<u8> = client
            .query_one(
                "SELECT embedding FROM \"CrdtVectorNote\" WHERE id=$1",
                &[&id],
            )
            .unwrap()
            .get(0);
        assert_eq!(bytes.len(), 12, "the database still stores the vector");
    }
    let mut public_schema = runtime.manifest().clone();
    public_schema
        .entities
        .iter_mut()
        .find(|entity| entity.name == "CrdtVectorNote")
        .unwrap()
        .fields
        .retain(|field| field.name != "embedding");
    drop(runtime);
    let reopened = Runtime::open_postgres(&url, public_schema).unwrap();
    assert!(DataStore::has_private_crdt_history(
        &reopened,
        "CrdtVectorNote"
    ));
    assert!(pylon_router::supports_crdt_replication(
        reopened.manifest(),
        &reopened.manifest().auth.user,
        "CrdtVectorNote"
    ));
}

#[test]
fn postgres_running_instances_read_new_history_restrictions() {
    use pylon_http::DataStore;
    let Ok(url) = std::env::var("TEST_POSTGRES_URL") else {
        return;
    };
    let old = Runtime::open_postgres(&url, AppManifest::default()).unwrap();
    let name = format!(
        "PrivateHistory{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    assert!(!DataStore::has_private_crdt_history(&old, &name));
    let mut schema = AppManifest::default();
    schema.auth.user.entity = name.clone();
    let new = Runtime::open_postgres(&url, schema).unwrap();
    assert!(DataStore::has_private_crdt_history(&new, &name));
    assert!(DataStore::has_private_crdt_history(&old, &name));
}
