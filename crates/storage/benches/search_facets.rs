//! Run with `cargo bench -p pylon-storage --bench search_facets`.
use std::hint::black_box;
use std::time::Instant;

use pylon_storage::search::{create_facet_table_sql, serialize_bitmap, SearchConfig, SearchQuery};
use pylon_storage::search_query::run_search;
use roaring::RoaringBitmap;
use rusqlite::{params, Connection};

fn run(rows: u32, buckets: u32, sorted: bool) {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE Product (id INTEGER PRIMARY KEY);")
        .unwrap();
    conn.execute(
        "WITH RECURSIVE ids(id) AS (SELECT 1 UNION ALL SELECT id + 1 FROM ids WHERE id < ?1)
         INSERT INTO Product SELECT id FROM ids",
        [rows],
    )
    .unwrap();
    conn.execute(create_facet_table_sql(), []).unwrap();
    for bucket in 0..buckets {
        let bitmap: RoaringBitmap = (1..=rows).filter(|id| id % buckets == bucket).collect();
        conn.execute(
            "INSERT INTO _facet_bitmap (entity, facet, value, bitmap, row_count) VALUES ('Product', 'category', ?1, ?2, ?3)",
            params![bucket.to_string(), serialize_bitmap(&bitmap).unwrap(), bitmap.len()],
        ).unwrap();
    }
    let selected: RoaringBitmap = (1..=rows / 2).collect();
    conn.execute(
        "INSERT INTO _facet_bitmap (entity, facet, value, bitmap, row_count) VALUES ('Product', 'selected', 'yes', ?1, ?2)",
        params![serialize_bitmap(&selected).unwrap(), selected.len()],
    ).unwrap();
    let config = SearchConfig {
        text: vec![],
        facets: vec!["category".into(), "selected".into()],
        sortable: vec!["id".into()],
        language: None,
    };
    let query = SearchQuery {
        filters: [("selected".into(), serde_json::json!("yes"))].into(),
        facets: vec!["category".into()],
        page_size: 20,
        sort: sorted.then(|| ("id".into(), "desc".into())),
        ..Default::default()
    };
    let mut times = Vec::new();
    let iterations = if sorted { 25 } else { 300 };
    let warmup = if sorted { 5 } else { 100 };
    for i in 0..iterations {
        let start = Instant::now();
        let result = black_box(run_search(&conn, "Product", &config, &query).unwrap());
        let elapsed = start.elapsed();
        assert_eq!(result.total, rows as u64 / 2);
        assert_eq!(result.hits.len(), 20);
        assert_eq!(
            result.facet_counts["category"].values().sum::<u64>(),
            result.total
        );
        if i >= warmup {
            times.push(elapsed);
        }
    }
    times.sort_unstable();
    println!(
        "{rows} rows, {buckets} categories, sorted={sorted}: p50 {:.2} us/search",
        times[times.len() / 2].as_secs_f64() * 1e6
    );
}

fn main() {
    run(131_072, 8, false);
    run(131_072, 128, false);
    run(131_072, 8, true);
}
