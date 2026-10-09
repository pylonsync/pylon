//! Run with `cargo bench -p pylon-storage --bench search_updates`.
use pylon_storage::search::{create_fts_table_sql, SearchConfig};
use pylon_storage::search_maintenance::apply_update;
use rusqlite::Connection;
use std::time::Instant;

fn main() {
    for rows in [100, 10_000] {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE Product (id TEXT PRIMARY KEY, name TEXT)")
            .unwrap();
        conn.execute(
            "WITH RECURSIVE ids(id) AS (SELECT 1 UNION ALL SELECT id + 1 FROM ids WHERE id < ?1)
            INSERT INTO Product SELECT printf('p%06d', id), 'old label' FROM ids",
            [rows],
        )
        .unwrap();
        let config = SearchConfig {
            text: vec!["name".into()],
            facets: vec![],
            sortable: vec![],
            language: None,
        };
        conn.execute(&create_fts_table_sql("Product", &config).unwrap(), [])
            .unwrap();
        conn.execute_batch("INSERT INTO _fts_Product(rowid, entity_id, name) SELECT rowid, rowid, name FROM Product; BEGIN").unwrap();
        let id = format!("p{rows:06}");
        let old = serde_json::json!({"name": "old label"});
        let patch = serde_json::json!({"name": "new label"});
        let mut samples = Vec::new();
        for i in 0..120 {
            let start = Instant::now();
            apply_update(&conn, "Product", &id, &old, &patch, &config).unwrap();
            if i >= 20 {
                samples.push(start.elapsed());
            }
        }
        conn.execute_batch("COMMIT").unwrap();
        samples.sort_unstable();
        let matches: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM _fts_Product WHERE _fts_Product MATCH 'new'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(matches, 1);
        println!(
            "update one of {rows} FTS rows: {:?} median",
            samples[samples.len() / 2]
        );
    }
}
