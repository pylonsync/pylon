use pylon_kernel::{AppManifest, ManifestEntity, ManifestField};
use pylon_runtime::Runtime;
use serde_json::json;

fn manifest() -> AppManifest {
    AppManifest {
        entities: vec![ManifestEntity {
            name: "CountNote".into(),
            crdt: false,
            fields: [
                ("title", "string"),
                ("rank", "int"),
                ("active", "bool"),
                ("payload", "json"),
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

fn assert_parity(runtime: &Runtime) {
    for filter in [
        json!({}),
        json!({"title":"needle"}),
        json!({"active":true}),
        json!({"rank":{"$gte":4,"$lt":9}}),
        json!({"rank":{"$in":[]}}),
        json!({"rank":{"$in":[1,3,7]}}),
        json!({"title":{"$like":"eed"}}),
        json!({"title":null}),
        json!({"payload":{"group":"a"}}),
        json!({"$search":"needle"}),
        json!({"$search":"absent"}),
        json!({"$limit":3,"$offset":2,"$order":{"rank":"desc"}}),
        json!({"$limit":0}),
        json!({"$offset":100000}),
    ] {
        let expected = runtime.query_filtered("CountNote", &filter).unwrap().len();
        assert_eq!(
            runtime.count_filtered("CountNote", &filter).unwrap(),
            expected,
            "{filter}"
        );
    }
    assert!(runtime
        .count_filtered("CountNote", &json!({"removed":1}))
        .is_err());
}

#[test]
fn sqlite_counts_reuse_filters_and_preserve_the_row_cap() {
    let runtime = Runtime::in_memory(manifest()).unwrap();
    for rank in 0..20 {
        runtime.insert("CountNote", &json!({"title":if rank % 2 == 0 {"needle"} else {"other"}, "rank":rank, "active":rank % 2 == 0, "payload":{"group":"a"}})).unwrap();
    }
    assert_eq!(
        runtime
            .count_filtered("CountNote", &json!({"$search":"needle"}))
            .unwrap(),
        10
    );
    assert_parity(&runtime);
    let cap = pylon_kernel::util::query_max_limit();
    {
        let conn = runtime.lock_conn_pub().unwrap();
        conn.execute("WITH RECURSIVE numbers(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM numbers WHERE n < ?1) INSERT INTO CountNote(id) SELECT 'extra-' || n FROM numbers", [cap + 1]).unwrap();
    }
    assert_eq!(
        runtime.count_filtered("CountNote", &json!({})).unwrap(),
        cap as usize
    );
    assert_eq!(
        runtime
            .count_filtered("CountNote", &json!({"$limit":cap+100}))
            .unwrap(),
        cap as usize
    );
}

#[test]
fn postgres_counts_reuse_filters_and_preserve_the_row_cap() {
    let Ok(url) = std::env::var("TEST_POSTGRES_URL") else {
        return;
    };
    let mut client = postgres::Client::connect(&url, postgres::NoTls).unwrap();
    client.batch_execute("DROP TABLE IF EXISTS \"CountNote\" CASCADE; CREATE TABLE \"CountNote\" (id TEXT PRIMARY KEY, title TEXT, rank BIGINT, active BOOLEAN, payload TEXT)").unwrap();
    let runtime = Runtime::open_postgres(&url, manifest()).unwrap();
    for rank in 0..20 {
        runtime.insert("CountNote", &json!({"title":if rank % 2 == 0 {"needle"} else {"other"}, "rank":rank, "active":rank % 2 == 0, "payload":{"group":"a"}})).unwrap();
    }
    client.batch_execute("CREATE TABLE IF NOT EXISTS \"_fts_CountNote\" (entity_id TEXT PRIMARY KEY REFERENCES \"CountNote\"(id) ON DELETE CASCADE, tsv TSVECTOR NOT NULL); TRUNCATE \"_fts_CountNote\"; INSERT INTO \"_fts_CountNote\" SELECT id, to_tsvector('english', title) FROM \"CountNote\"").unwrap();
    assert_eq!(
        runtime
            .count_filtered("CountNote", &json!({"$search":"needle"}))
            .unwrap(),
        10
    );
    assert_parity(&runtime);
    let cap = pylon_kernel::util::query_max_limit() as i64;
    client.execute("INSERT INTO \"CountNote\" (id) SELECT 'extra-' || n FROM generate_series(1, $1::bigint) n", &[&(cap + 1)]).unwrap();
    assert_eq!(
        runtime.count_filtered("CountNote", &json!({})).unwrap(),
        cap as usize
    );
}
