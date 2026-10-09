//! Live tests. Set TEST_POSTGRES_URL to an isolated PostgreSQL database.
#![cfg(feature = "postgres-live")]

use postgres::{types::ToSql, Client, NoTls, Row};
use pylon_http::{DataError, DataStore};
use pylon_kernel::{AppManifest, ManifestEntity};
use pylon_storage::vector::{pack_f32, score, TopK, VectorMetric};
use pylon_storage::{pg_datastore::PostgresDataStore, pg_exec::PgConn, pg_vector};
use serde_json::{json, Value};
use std::collections::HashMap;

fn manifest() -> AppManifest {
    AppManifest {
        entities: vec![
            serde_json::from_value(json!({"name":"PerfParent", "fields":[
                {"name":"group_key", "type":"string", "optional":true, "unique":false},
                {"name":"owner_id", "type":"string", "optional":true, "unique":false}
            ], "indexes":[], "relations":[
                {"name":"children", "target":"PerfChild", "field":"group_key", "many":true},
                {"name":"owner", "target":"PerfOwner", "field":"owner_id", "many":false}
            ]}))
            .unwrap(),
            serde_json::from_value(json!({"name":"PerfChild", "fields":[
                {"name":"group_key", "type":"string", "optional":true, "unique":false},
                {"name":"payload", "type":"json", "optional":true, "unique":false}
            ], "indexes":[]}))
            .unwrap(),
            ManifestEntity {
                name: "PerfOwner".into(),
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

// The original per-parent algorithm provides the compatibility oracle.
fn legacy_graph(store: &dyn DataStore, filter: &Value) -> Value {
    let mut rows = store.query_filtered("PerfParent", filter).unwrap();
    for row in &mut rows {
        if let Some(key) = row["group_key"].as_str() {
            row["children"] = json!(store
                .query_filtered("PerfChild", &json!({"group_key":key}))
                .unwrap());
        }
        if let Some(key) = row["owner_id"].as_str() {
            if let Some(owner) = store.get_by_id("PerfOwner", key).unwrap() {
                row["owner"] = owner;
            }
        }
    }
    json!({"PerfParent":rows})
}

#[test]
fn graph_batches_preserve_parent_child_limits_and_transaction_visibility() {
    let Ok(url) = std::env::var("TEST_POSTGRES_URL") else {
        return;
    };
    let mut client = Client::connect(&url, NoTls).unwrap();
    client.batch_execute("DROP TABLE IF EXISTS \"PerfParent\", \"PerfChild\", \"PerfOwner\";
        CREATE TABLE \"PerfParent\" (id text PRIMARY KEY, group_key text, owner_id text);
        CREATE TABLE \"PerfChild\" (id text PRIMARY KEY, group_key text, payload jsonb);
        CREATE TABLE \"PerfOwner\" (id text PRIMARY KEY);
        INSERT INTO \"PerfOwner\" VALUES ('owner');
        INSERT INTO \"PerfParent\" VALUES ('p1','a','owner'), ('p2','b','owner'), ('p3','a','absent'), ('p4','absent','absent'), ('p5',NULL,NULL);").unwrap();
    let cap = pylon_kernel::util::effective_query_limit(None) as i64;
    client.execute("INSERT INTO \"PerfChild\" SELECT 'a' || lpad(n::text,10,'0'), 'a', jsonb_build_object('n',n) FROM generate_series(1,$1::bigint) n", &[&(cap+2)]).unwrap();
    client
        .batch_execute(
            "INSERT INTO \"PerfChild\" VALUES ('b2','b','{\"n\":2}'), ('b1','b','{\"n\":1}')",
        )
        .unwrap();
    let store = PostgresDataStore::connect(&url, manifest()).unwrap();
    for limit in [0, 2, 5] {
        let filter = json!({"$limit":limit});
        let query = json!({"PerfParent":{"limit":limit,"include":{"children":{},"owner":{}}}});
        assert_eq!(
            store.query_graph(&query).unwrap(),
            legacy_graph(&store, &filter)
        );
    }
    let query = json!({"PerfParent":{"limit":2,"include":{"children":{},"owner":{}}}});
    let graph = store.query_graph(&query).unwrap();
    assert_eq!(
        graph["PerfParent"][0]["children"].as_array().unwrap().len(),
        cap as usize
    );
    assert_eq!(
        graph["PerfParent"][1]["children"].as_array().unwrap().len(),
        2
    );
    store
        .with_transaction::<_, _, DataError>(|tx| {
            tx.insert("PerfChild", &json!({"id":"b0", "group_key":"b"}))?;
            let actual = tx.query_graph(&query)?;
            assert_eq!(actual, legacy_graph(tx, &json!({"$limit":2})));
            assert_eq!(actual["PerfParent"][1]["children"][0]["id"], "b0");
            Ok(())
        })
        .unwrap();
    client
        .batch_execute("DROP TABLE \"PerfParent\", \"PerfChild\", \"PerfOwner\"")
        .unwrap();
}

struct StreamingOnly<'a, C> {
    inner: &'a mut C,
    calls: usize,
    rows: usize,
}
impl<C: PgConn> PgConn for StreamingOnly<'_, C> {
    fn execute(&mut self, _: &str, _: &[&(dyn ToSql + Sync)]) -> Result<u64, postgres::Error> {
        panic!("scan must not execute writes")
    }
    fn query(&mut self, _: &str, _: &[&(dyn ToSql + Sync)]) -> Result<Vec<Row>, postgres::Error> {
        panic!("scan must not collect all rows")
    }
    fn query_opt(
        &mut self,
        _: &str,
        _: &[&(dyn ToSql + Sync)],
    ) -> Result<Option<Row>, postgres::Error> {
        panic!("scan must stream")
    }
    fn query_each(
        &mut self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
        visit: &mut dyn FnMut(Row),
    ) -> Result<(), postgres::Error> {
        self.calls += 1;
        self.inner.query_each(sql, params, &mut |row| {
            self.rows += 1;
            visit(row);
        })
    }
}

fn check_scan<C: PgConn>(conn: &mut C, candidates: &[(String, Vec<f32>)], extra_rows: usize) {
    for metric in [VectorMetric::Cosine, VectorMetric::Dot, VectorMetric::L2] {
        let query = [1.0, 0.5];
        let mut expected = TopK::new(metric, 7);
        for (id, vector) in candidates {
            expected.push(id.clone(), score(metric, &query, vector));
        }
        let mut stream = StreamingOnly {
            inner: conn,
            calls: 0,
            rows: 0,
        };
        let actual = pg_vector::scan_topk(
            &mut stream,
            "PerfVector",
            "embedding",
            &query,
            metric,
            7,
            &HashMap::new(),
        )
        .unwrap();
        assert_eq!(actual, expected.into_sorted());
        assert_eq!(stream.calls, 1);
        assert_eq!(stream.rows, candidates.len() + extra_rows);
    }
}

#[test]
fn vector_scan_streams_exact_results_on_client_and_transaction() {
    let Ok(url) = std::env::var("TEST_POSTGRES_URL") else {
        return;
    };
    let mut client = Client::connect(&url, NoTls).unwrap();
    client
        .batch_execute("CREATE TEMP TABLE \"PerfVector\" (id text PRIMARY KEY, embedding bytea)")
        .unwrap();
    let mut candidates = Vec::new();
    for i in 1..=256 {
        let id = format!("v{i:04}");
        let vector = vec![i as f32, (i % 13) as f32];
        client
            .execute(
                "INSERT INTO \"PerfVector\" VALUES ($1,$2)",
                &[&id, &pack_f32(&vector)],
            )
            .unwrap();
        candidates.push((id, vector));
    }
    for (id, blob) in [
        ("malformed", vec![1u8]),
        ("wrong_dims", pack_f32(&[1.0])),
        ("nan", pack_f32(&[f32::NAN, 1.0])),
    ] {
        client
            .execute("INSERT INTO \"PerfVector\" VALUES ($1,$2)", &[&id, &blob])
            .unwrap();
    }
    check_scan(&mut client, &candidates, 3);
    let mut tx = client.transaction().unwrap();
    let vector = vec![500.0, 250.0];
    tx.execute(
        "INSERT INTO \"PerfVector\" VALUES ('pending',$1)",
        &[&pack_f32(&vector)],
    )
    .unwrap();
    candidates.push(("pending".into(), vector));
    check_scan(&mut tx, &candidates, 3);
    tx.rollback().unwrap();
    candidates.pop();
    check_scan(&mut client, &candidates, 3);
}

#[test]
fn streaming_queries_propagate_server_errors() {
    let Ok(url) = std::env::var("TEST_POSTGRES_URL") else {
        return;
    };
    let mut client = Client::connect(&url, NoTls).unwrap();
    let sql = "SELECT 1 / (n - 3) FROM generate_series(1,4) AS n";
    let err = client.query_each(sql, &[], &mut |_| {}).unwrap_err();
    assert_eq!(
        err.code(),
        Some(&postgres::error::SqlState::DIVISION_BY_ZERO)
    );
    assert_eq!(
        client
            .query_one("SELECT 1::int", &[])
            .unwrap()
            .get::<_, i32>(0),
        1
    );
    let mut tx = client.transaction().unwrap();
    let err = tx.query_each(sql, &[], &mut |_| {}).unwrap_err();
    assert_eq!(
        err.code(),
        Some(&postgres::error::SqlState::DIVISION_BY_ZERO)
    );
    tx.rollback().unwrap();
}
