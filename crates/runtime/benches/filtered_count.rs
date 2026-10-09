use std::{hint::black_box, time::Instant};

use pylon_kernel::{AppManifest, ManifestEntity, ManifestField};
use pylon_runtime::Runtime;
use serde_json::json;

fn measure(label: &str, mut count: impl FnMut() -> usize) {
    for _ in 0..5 {
        assert_eq!(black_box(count()), 1000);
    }
    let mut samples = Vec::new();
    for _ in 0..9 {
        let start = Instant::now();
        for _ in 0..20 {
            assert_eq!(black_box(count()), 1000);
        }
        samples.push(start.elapsed().as_secs_f64() * 1e6 / 20.0);
    }
    samples.sort_by(f64::total_cmp);
    println!("{label}: {:.3} us/count median", samples[4]);
}

fn main() {
    let runtime = Runtime::in_memory(AppManifest {
        entities: vec![ManifestEntity {
            name: "CountRecord".into(),
            crdt: false,
            fields: vec![ManifestField {
                name: "payload".into(),
                field_type: "json".into(),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    })
    .unwrap();
    // Use direct inserts to exclude mutation hooks and CRDT work from setup.
    let payload = json!({"text":"x".repeat(8192)}).to_string();
    {
        let mut conn = runtime.lock_conn_pub().unwrap();
        let tx = conn.transaction().unwrap();
        {
            let mut insert = tx
                .prepare("INSERT INTO CountRecord(id, payload) VALUES (?1, ?2)")
                .unwrap();
            for id in 0..1000 {
                insert.execute([id.to_string(), payload.clone()]).unwrap();
            }
        }
        tx.commit().unwrap();
    }
    let filter = json!({});
    measure("1000 rows, 8 KiB JSON, materialized", || {
        runtime
            .query_filtered("CountRecord", &filter)
            .unwrap()
            .len()
    });
    measure("1000 rows, 8 KiB JSON, SQL count", || {
        runtime.count_filtered("CountRecord", &filter).unwrap()
    });
}
