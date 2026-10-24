//! FTS5 + facet bitmap maintenance on insert / update / delete.
//!
//! Called by the runtime inside the same SQLite connection that wrote
//! the entity row. All changes land in one transaction — the search
//! index can never drift from the live data.
//!
//! Rowid strategy: SQLite's implicit `rowid` (the INTEGER primary key
//! every table has) is the bitmap identifier. It's a compact u32-range
//! value that fits a Roaring bitmap natively. We translate `entity.id`
//! (ULID) → rowid once per maintenance call via a WHERE id = ? lookup.
//! Cheap — always backed by the unique `id` index Pylon creates.

use roaring::RoaringBitmap;
use rusqlite::Connection;
use serde_json::Value;

use crate::search::{
    deserialize_bitmap, merge_row, serialize_bitmap, stringify_facet, SearchConfig,
};
use crate::StorageError;

// ---------------------------------------------------------------------------
// Public entry points — what the runtime calls
// ---------------------------------------------------------------------------

/// Called immediately after an entity row is INSERTed. Writes the FTS5
/// shadow row + flips the "1" bit in every relevant facet bitmap. All
/// operations share the caller's connection so the whole thing is one
/// transaction.
pub fn apply_insert(
    conn: &Connection,
    entity: &str,
    id: &str,
    data: &Value,
    config: &SearchConfig,
) -> Result<(), StorageError> {
    if config.is_empty() {
        return Ok(());
    }
    let rowid = rowid_for_id(conn, entity, id)?;
    write_fts_row(conn, entity, rowid, data, config)?;
    for facet_field in &config.facets {
        if let Some(value) = data.get(facet_field) {
            if let Some(v) = stringify_facet(value) {
                bitmap_set_bit(conn, entity, facet_field, &v, rowid, true)?;
            }
        }
    }
    Ok(())
}

/// Called after an UPDATE. Caller supplies the pre-UPDATE row so we
/// can diff its facet values against the patch without re-reading
/// (which would see the already-applied new values). Only changed
/// facet fields touch bitmap rows.
pub fn apply_update(
    conn: &Connection,
    entity: &str,
    id: &str,
    old_row: &Value,
    patch: &Value,
    config: &SearchConfig,
) -> Result<(), StorageError> {
    if config.is_empty() {
        return Ok(());
    }
    let rowid = rowid_for_id(conn, entity, id)?;

    // If the update touched any text field, rebuild the FTS row. FTS5
    // doesn't have a cheap in-place column update; the pattern is
    // DELETE + INSERT addressed by the FTS rowid, which matches the entity rowid.
    // Merge old_row + patch so the rebuilt FTS row reflects the new
    // state across all declared text fields.
    let touches_text = config.text.iter().any(|f| patch.get(f).is_some());
    if touches_text {
        let merged = merge_row(old_row, patch);
        delete_fts_row(conn, entity, rowid)?;
        write_fts_row(conn, entity, rowid, &merged, config)?;
    }

    for facet_field in &config.facets {
        let Some(new_val) = patch.get(facet_field) else {
            continue; // field not in patch — bitmap stays put
        };
        let new_str = stringify_facet(new_val);
        let old_str = old_row.get(facet_field).and_then(stringify_facet);
        if old_str == new_str {
            continue;
        }
        if let Some(old) = old_str {
            bitmap_set_bit(conn, entity, facet_field, &old, rowid, false)?;
        }
        if let Some(new) = new_str {
            bitmap_set_bit(conn, entity, facet_field, &new, rowid, true)?;
        }
    }
    Ok(())
}

/// Called BEFORE the entity row is removed — caller passes the pre-DELETE
/// row so we know which facet bits to clear. The runtime fetches the
/// row, then calls this, then runs the DELETE SQL.
pub fn apply_delete(
    conn: &Connection,
    entity: &str,
    id: &str,
    old_row: &Value,
    config: &SearchConfig,
) -> Result<(), StorageError> {
    if config.is_empty() {
        return Ok(());
    }
    let rowid = match rowid_for_id(conn, entity, id) {
        Ok(r) => r,
        Err(_) => return Ok(()), // row already gone — nothing to maintain
    };
    // Only touch the FTS shadow table if the config actually declared
    // text fields — facet-only searchable entities have no `_fts_<E>`
    // to delete from, and an unconditional DELETE would raise
    // "no such table".
    if !config.text.is_empty() {
        delete_fts_row(conn, entity, rowid)?;
    }
    for facet_field in &config.facets {
        if let Some(v) = old_row.get(facet_field).and_then(stringify_facet) {
            bitmap_set_bit(conn, entity, facet_field, &v, rowid, false)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Build — index rows that were written before the index existed
// ---------------------------------------------------------------------------
//
// The hooks above only see rows written after the `search:` block existed.
// Adding the block to a table with rows, or changing its lists, needs a
// build: `start_index_build` clears the entity's FTS rows and facet bitmaps
// and records the config with cursor 0; then `index_build_page` indexes the
// rows after the cursor, a page at a time, each page in its own write
// transaction. Writes between pages keep going through the hooks: a row the
// build has passed is kept current by them, a row it has not reached is
// indexed (again) when its page comes, and a row deleted before its page is
// never seen. Every step is idempotent, so a build that stops part way
// continues from the cursor.

/// What the entity's index needs. `Current` when the entity has no table.
pub fn index_build_state(
    conn: &Connection,
    entity: &str,
    config: &SearchConfig,
) -> Result<crate::search::IndexBuild, StorageError> {
    let err = |e: rusqlite::Error| StorageError::new("SEARCH_STATE_READ_FAILED", &e.to_string());
    let table_exists: bool = conn
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
            [entity],
            |r| r.get(0),
        )
        .map_err(err)?;
    if !table_exists {
        return Ok(crate::search::IndexBuild::Current);
    }
    conn.execute_batch(crate::search::create_index_state_table_sql())
        .map_err(err)?;
    let state = conn
        .query_row(
            "SELECT fingerprint, cursor FROM \"_pylon_search_index_state\" WHERE entity = ?1",
            [entity],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            e => Err(err(e)),
        })?;
    Ok(crate::search::IndexBuild::from_state(state, config))
}

/// Clear the entity's FTS rows and facet bitmaps, create the tables and
/// sort indexes for `config`, and record a build for `config` at cursor 0.
pub fn start_index_build(
    conn: &Connection,
    entity: &str,
    config: &SearchConfig,
) -> Result<(), StorageError> {
    let exec = |sql: &str| {
        conn.execute_batch(sql)
            .map_err(|e| StorageError::new("SEARCH_BUILD_FAILED", &format!("{sql}: {e}")))
    };
    exec(crate::search::create_facet_table_sql())?;
    exec(crate::search::create_index_state_table_sql())?;
    exec(&format!("DROP TABLE IF EXISTS \"_fts_{entity}\""))?;
    if let Some(sql) = crate::search::create_fts_table_sql(entity, config) {
        exec(&sql)?;
    }
    conn.execute("DELETE FROM \"_facet_bitmap\" WHERE entity = ?1", [entity])
        .map_err(|e| StorageError::new("SEARCH_BUILD_FAILED", &e.to_string()))?;
    for field in &config.sortable {
        exec(&format!(
            "CREATE INDEX IF NOT EXISTS \"{entity}_sort_{field}\" ON \"{entity}\" (\"{field}\")"
        ))?;
    }
    conn.execute(
        "INSERT INTO \"_pylon_search_index_state\" (entity, fingerprint, cursor) VALUES (?1, ?2, '0') \
         ON CONFLICT(entity) DO UPDATE SET fingerprint = excluded.fingerprint, cursor = excluded.cursor",
        [entity, &crate::search::config_fingerprint(config)],
    )
    .map_err(|e| StorageError::new("SEARCH_STATE_WRITE_FAILED", &e.to_string()))?;
    Ok(())
}

/// The rowid after which the build continues, or `None` when no build is
/// in progress.
pub fn index_build_cursor(conn: &Connection, entity: &str) -> Result<Option<i64>, StorageError> {
    let cursor: Option<String> = conn
        .query_row(
            "SELECT cursor FROM \"_pylon_search_index_state\" WHERE entity = ?1",
            [entity],
            |r| r.get(0),
        )
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            e => Err(StorageError::new(
                "SEARCH_STATE_READ_FAILED",
                &e.to_string(),
            )),
        })?;
    cursor
        .map(|c| {
            c.parse::<i64>().map_err(|_| {
                StorageError::new("SEARCH_STATE_READ_FAILED", &format!("bad cursor {c:?}"))
            })
        })
        .transpose()
}

/// Index one page of rows (`(rowid, row)`, rows as the write path sees
/// them, bools as JSON booleans) and move the cursor to `next`, or mark the
/// build complete when `next` is `None`. Run inside the caller's write
/// transaction, with rows read in that transaction.
pub fn index_build_page(
    conn: &Connection,
    entity: &str,
    config: &SearchConfig,
    rows: &[(u32, Value)],
    next: Option<i64>,
) -> Result<(), StorageError> {
    let mut bitmaps: std::collections::HashMap<(&str, String), RoaringBitmap> =
        std::collections::HashMap::new();
    for (rowid, row) in rows {
        if !config.text.is_empty() {
            delete_fts_row(conn, entity, *rowid)?;
            write_fts_row(conn, entity, *rowid, row, config)?;
        }
        for facet in &config.facets {
            if let Some(v) = row.get(facet).and_then(stringify_facet) {
                bitmaps
                    .entry((facet.as_str(), v))
                    .or_default()
                    .insert(*rowid);
            }
        }
    }
    for ((facet, value), page_bits) in &bitmaps {
        update_bitmap(conn, entity, facet, value, |b| *b |= page_bits)?;
    }
    conn.execute(
        "UPDATE \"_pylon_search_index_state\" SET cursor = ?2 WHERE entity = ?1",
        rusqlite::params![entity, next.map(|n| n.to_string())],
    )
    .map_err(|e| StorageError::new("SEARCH_STATE_WRITE_FAILED", &e.to_string()))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

/// SQLite's implicit rowid for the row whose `id` column matches. We
/// lean on SQLite's auto-created index on id; this is a one-row lookup.
fn rowid_for_id(conn: &Connection, entity: &str, id: &str) -> Result<u32, StorageError> {
    let sql = format!("SELECT rowid FROM \"{entity}\" WHERE \"id\" = ?1");
    conn.query_row(&sql, [id], |r| r.get::<_, i64>(0))
        .map(|v| v as u32)
        .map_err(|e| StorageError::new("ROWID_LOOKUP_FAILED", &e.to_string()))
}

fn write_fts_row(
    conn: &Connection,
    entity: &str,
    rowid: u32,
    data: &Value,
    config: &SearchConfig,
) -> Result<(), StorageError> {
    if config.text.is_empty() {
        return Ok(());
    }
    // Pin the FTS5 rowid to the entity rowid. FTS5 auto-assigns one
    // otherwise, which breaks any query that joins FTS5 matches back to
    // other tables via the rowid. BM25 scoring is keyed on FTS rowid, so
    // we want the two to agree.
    let cols = std::iter::once("rowid".to_string())
        .chain(std::iter::once("entity_id".to_string()))
        .chain(config.text.iter().map(|f| format!("\"{f}\"")))
        .collect::<Vec<_>>()
        .join(", ");
    let placeholders = (1..=(2 + config.text.len()))
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!("INSERT INTO \"_fts_{entity}\" ({cols}) VALUES ({placeholders});");

    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> =
        vec![Box::new(rowid as i64), Box::new(rowid as i64)];
    for field in &config.text {
        let text = data
            .get(field)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        params.push(Box::new(text));
    }
    let refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    conn.execute(&sql, refs.as_slice())
        .map(|_| ())
        .map_err(|e| StorageError::new("FTS_WRITE_FAILED", &e.to_string()))
}

fn delete_fts_row(conn: &Connection, entity: &str, rowid: u32) -> Result<(), StorageError> {
    let sql = format!("DELETE FROM \"_fts_{entity}\" WHERE rowid = ?1;");
    conn.execute(&sql, [rowid as i64])
        .map(|_| ())
        .map_err(|e| StorageError::new("FTS_DELETE_FAILED", &e.to_string()))
}

/// Set or clear one bit in the bitmap row for `(entity, facet, value)`.
/// Creates the row if it's the first write for that value; drops the
/// row when the bitmap empties so facet listings stay clean.
fn bitmap_set_bit(
    conn: &Connection,
    entity: &str,
    facet: &str,
    value: &str,
    rowid: u32,
    set: bool,
) -> Result<(), StorageError> {
    update_bitmap(conn, entity, facet, value, |bitmap| {
        if set {
            bitmap.insert(rowid);
        } else {
            bitmap.remove(rowid);
        }
    })
}

/// Read one facet value's bitmap, change it, and write it back (or delete
/// the row when the bitmap is empty).
fn update_bitmap(
    conn: &Connection,
    entity: &str,
    facet: &str,
    value: &str,
    change: impl FnOnce(&mut RoaringBitmap),
) -> Result<(), StorageError> {
    let existing: Option<Vec<u8>> = conn
        .query_row(
            "SELECT bitmap FROM \"_facet_bitmap\" \
             WHERE entity = ?1 AND facet = ?2 AND value = ?3",
            [entity, facet, value],
            |r| r.get::<_, Vec<u8>>(0),
        )
        .ok();

    let mut bitmap = match &existing {
        Some(bytes) => deserialize_bitmap(bytes)?,
        None => RoaringBitmap::new(),
    };
    change(&mut bitmap);
    let new_count = bitmap.len() as i64;

    if new_count == 0 {
        conn.execute(
            "DELETE FROM \"_facet_bitmap\" \
             WHERE entity = ?1 AND facet = ?2 AND value = ?3",
            [entity, facet, value],
        )
        .map_err(|e| StorageError::new("BITMAP_DELETE_FAILED", &e.to_string()))?;
    } else {
        let bytes = serialize_bitmap(&bitmap)?;
        conn.execute(
            "INSERT INTO \"_facet_bitmap\" (entity, facet, value, bitmap, row_count) \
             VALUES (?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT(entity, facet, value) DO UPDATE SET bitmap = excluded.bitmap, row_count = excluded.row_count",
            rusqlite::params![entity, facet, value, bytes, new_count],
        )
        .map_err(|e| StorageError::new("BITMAP_WRITE_FAILED", &e.to_string()))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::{create_facet_table_sql, create_fts_table_sql};
    use rusqlite::Connection;

    fn open_test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE \"Product\" (id TEXT PRIMARY KEY, name TEXT, brand TEXT, category TEXT, price REAL);",
        )
        .unwrap();
        conn.execute(create_facet_table_sql(), []).unwrap();
        let cfg = SearchConfig {
            text: vec!["name".into()],
            facets: vec!["brand".into(), "category".into()],
            sortable: vec!["price".into()],
            language: None,
        };
        conn.execute(&create_fts_table_sql("Product", &cfg).unwrap(), [])
            .unwrap();
        conn
    }

    fn insert_product(conn: &Connection, id: &str, name: &str, brand: &str, category: &str) {
        conn.execute(
            "INSERT INTO \"Product\" (id, name, brand, category, price) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![id, name, brand, category, 99.99],
        )
        .unwrap();
    }

    #[test]
    fn insert_maintains_fts_and_facets() {
        let conn = open_test_db();
        let cfg = SearchConfig {
            text: vec!["name".into()],
            facets: vec!["brand".into(), "category".into()],
            sortable: vec![],
            language: None,
        };

        insert_product(&conn, "p1", "Nike Air Max", "Nike", "shoes");
        apply_insert(
            &conn,
            "Product",
            "p1",
            &serde_json::json!({
                "name": "Nike Air Max",
                "brand": "Nike",
                "category": "shoes",
            }),
            &cfg,
        )
        .unwrap();

        // FTS row landed.
        let fts_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM \"_fts_Product\"", [], |r| r.get(0))
            .unwrap();
        assert_eq!(fts_count, 1);

        // Facet bitmap for brand=Nike exists and has one bit.
        let (_bytes, row_count): (Vec<u8>, i64) = conn
            .query_row(
                "SELECT bitmap, row_count FROM \"_facet_bitmap\" \
                 WHERE entity = 'Product' AND facet = 'brand' AND value = 'Nike'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(row_count, 1);
    }

    #[test]
    fn sparse_fts_rowids_survive_updates_and_targeted_deletes() {
        let conn = open_test_db();
        let cfg = SearchConfig {
            text: vec!["name".into()],
            facets: vec![],
            sortable: vec![],
            language: None,
        };
        for (rowid, id, name) in [(10, "early", "alpha"), (50_000, "late", "omega")] {
            conn.execute(
                "INSERT INTO Product (rowid, id, name) VALUES (?1, ?2, ?3)",
                rusqlite::params![rowid, id, name],
            )
            .unwrap();
            apply_insert(
                &conn,
                "Product",
                id,
                &serde_json::json!({"name":name}),
                &cfg,
            )
            .unwrap();
        }
        for name in ["updated", "final"] {
            conn.execute("UPDATE Product SET name = ?1 WHERE id = 'late'", [name])
                .unwrap();
            apply_update(
                &conn,
                "Product",
                "late",
                &serde_json::json!({"name":"omega"}),
                &serde_json::json!({"name":name}),
                &cfg,
            )
            .unwrap();
            let ids: Vec<i64> = conn
                .prepare("SELECT rowid FROM _fts_Product ORDER BY rowid")
                .unwrap()
                .query_map([], |r| r.get(0))
                .unwrap()
                .map(Result::unwrap)
                .collect();
            assert_eq!(ids, [10, 50_000]);
            let matched: i64 = conn
                .query_row(
                    "SELECT entity_id FROM _fts_Product WHERE _fts_Product MATCH ?1",
                    [name],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(matched, 50_000);
        }
        apply_delete(
            &conn,
            "Product",
            "late",
            &serde_json::json!({"name":"final"}),
            &cfg,
        )
        .unwrap();
        let left: (i64, String) = conn
            .query_row("SELECT rowid, name FROM _fts_Product", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(left, (10, "alpha".into()));
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM _fts_Product", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn delete_clears_bits_and_drops_empty_bitmap_rows() {
        let conn = open_test_db();
        let cfg = SearchConfig {
            text: vec!["name".into()],
            facets: vec!["brand".into()],
            sortable: vec![],
            language: None,
        };
        insert_product(&conn, "p1", "Air Max", "Nike", "shoes");
        apply_insert(
            &conn,
            "Product",
            "p1",
            &serde_json::json!({ "name": "Air Max", "brand": "Nike" }),
            &cfg,
        )
        .unwrap();

        apply_delete(
            &conn,
            "Product",
            "p1",
            &serde_json::json!({ "name": "Air Max", "brand": "Nike" }),
            &cfg,
        )
        .unwrap();

        // Bitmap row for brand=Nike should be gone (cardinality hit 0).
        let nike_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM \"_facet_bitmap\" \
                 WHERE entity = 'Product' AND facet = 'brand' AND value = 'Nike'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(nike_rows, 0);

        // FTS row gone.
        let fts_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM \"_fts_Product\"", [], |r| r.get(0))
            .unwrap();
        assert_eq!(fts_count, 0);
    }

    #[test]
    fn facet_only_config_deletes_without_fts_table() {
        // When an entity declares `facets` but no `text`, no _fts_<E>
        // table exists. apply_delete must not try to DELETE FROM it.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute(
            "CREATE TABLE \"Product\" (id TEXT PRIMARY KEY, brand TEXT)",
            [],
        )
        .unwrap();
        conn.execute(create_facet_table_sql(), []).unwrap();
        let cfg = SearchConfig {
            text: vec![],
            facets: vec!["brand".into()],
            sortable: vec![],
            language: None,
        };
        conn.execute(
            "INSERT INTO \"Product\" (id, brand) VALUES ('p1', 'Nike')",
            [],
        )
        .unwrap();
        apply_insert(
            &conn,
            "Product",
            "p1",
            &serde_json::json!({ "brand": "Nike" }),
            &cfg,
        )
        .unwrap();

        let facet_rows = || -> i64 {
            conn.query_row(
                "SELECT COUNT(*) FROM \"_facet_bitmap\" WHERE entity = 'Product' AND facet = 'brand' AND value = 'Nike'",
                [], |r| r.get(0),
            ).unwrap()
        };
        assert_eq!(facet_rows(), 1);

        // Should succeed — no FTS delete attempted.
        apply_delete(
            &conn,
            "Product",
            "p1",
            &serde_json::json!({ "brand": "Nike" }),
            &cfg,
        )
        .unwrap();
        assert_eq!(facet_rows(), 0);
    }

    #[test]
    fn update_moves_bit_between_facet_values() {
        let conn = open_test_db();
        let cfg = SearchConfig {
            text: vec!["name".into()],
            facets: vec!["brand".into()],
            sortable: vec![],
            language: None,
        };
        insert_product(&conn, "p1", "Air Max", "Nike", "shoes");
        apply_insert(
            &conn,
            "Product",
            "p1",
            &serde_json::json!({ "name": "Air Max", "brand": "Nike" }),
            &cfg,
        )
        .unwrap();

        // Row gets re-branded. Caller captures old_row FIRST (that's
        // the contract of apply_update), then runs the UPDATE, then
        // calls apply_update with the old row + the patch.
        let old_row = serde_json::json!({ "name": "Air Max", "brand": "Nike" });
        conn.execute(
            "UPDATE \"Product\" SET brand = 'Adidas' WHERE id = 'p1'",
            [],
        )
        .unwrap();
        apply_update(
            &conn,
            "Product",
            "p1",
            &old_row,
            &serde_json::json!({ "brand": "Adidas" }),
            &cfg,
        )
        .unwrap();

        // Nike bitmap gone; Adidas bitmap has one bit.
        let nike_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM \"_facet_bitmap\" \
                 WHERE facet = 'brand' AND value = 'Nike'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(nike_rows, 0);

        let (_bytes, count): (Vec<u8>, i64) = conn
            .query_row(
                "SELECT bitmap, row_count FROM \"_facet_bitmap\" \
                 WHERE facet = 'brand' AND value = 'Adidas'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(count, 1);
    }
}
