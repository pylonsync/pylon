//! Run with `cargo bench -p pylon-storage --bench vector_scan`.
use pylon_storage::vector::{pack_f32, scan_topk, VectorMetric};
use rusqlite::{params, Connection};
use std::collections::HashMap;
use std::hint::black_box;
use std::time::Instant;

fn main() {
    for dims in [8, 384] {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE Doc (id TEXT PRIMARY KEY, embedding BLOB)")
            .unwrap();
        let tx = conn.transaction().unwrap();
        {
            let mut insert = tx.prepare("INSERT INTO Doc VALUES (?1, ?2)").unwrap();
            for id in 0..10_000 {
                let values: Vec<_> = (0..dims)
                    .map(|i| ((id * 17 + i * 31) % 101) as f32 / 100.0)
                    .collect();
                insert
                    .execute(params![format!("doc-{id:05}"), pack_f32(&values)])
                    .unwrap();
            }
        }
        tx.commit().unwrap();
        let query = vec![0.5; dims];
        let mut times = Vec::new();
        for i in 0..25 {
            let start = Instant::now();
            let hits = black_box(
                scan_topk(
                    &conn,
                    "Doc",
                    "embedding",
                    &query,
                    VectorMetric::Cosine,
                    20,
                    &HashMap::new(),
                )
                .unwrap(),
            );
            if i >= 5 {
                times.push(start.elapsed());
            }
            assert_eq!(hits.len(), 20);
        }
        times.sort_unstable();
        println!(
            "10,000 rows, {dims} dimensions: {:?} median",
            times[times.len() / 2]
        );
    }
}
