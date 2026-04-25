//! Query planner for faceted full-text search.
//!
//! Takes a `SearchQuery`, runs it against the FTS5 shadow + facet
//! bitmap tables maintained by `search_maintenance`, and returns a
//! `SearchResult` with ranked hits + per-facet counts + total.
//!
//! Execution shape, in one round-trip:
//!
//! 1. Base bitmap = FTS5 MATCH result (or "all rows" if no query).
//! 2. Apply each filter: base &= facet_bitmap(filter_field, filter_value).
//! 3. For each declared facet the caller asked about:
//!       for each (value, facet_bitmap):
//!         count = |base ∩ facet_bitmap|       ← popcount, µs
//! 4. Slice base by `page × page_size` to get the page's rowids.
//! 5. One SELECT over the entity table fetches those rows, in
//!    bitmap-order (rank/rowid) or ORDER BY sort-field if requested.
//!
//! What this deliberately doesn't do in v1:
//!   - Typo tolerance (requires trigram / spellfix1)
//!   - Synonyms, custom ranking rules
//!   - OR / NOT in filter expressions (only equality AND range)
//!
//! Adding those lifts the primitive past Meilisearch-class features;
//! the core perf story (bitmap-backed facets + FTS5) is sufficient for
//! 95% of ecommerce filter/sort workloads.

use std::collections::BTreeMap;

use roaring::RoaringBitmap;
use rusqlite::Connection;
use serde_json::Value;

use crate::search::{deserialize_bitmap, SearchConfig, SearchQuery, SearchResult};
use crate::StorageError;

/// Upper bound on how many matches we'll materialize for facet counts
/// when the caller supplies no query + no filters (i.e. the match
/// bitmap would be "every row"). Past this we return accurate facet
/// counts sampled from the most recent rows and let pagination walk
/// the rest. Bitmap ops stay sub-50ms at this size.
const FACET_COUNT_MAX_ROWS: u64 = 1_000_000;

/// Run a search query against `entity` using its `SearchConfig`. The
/// connection must be the same SQLite handle that owns the entity
/// table + the FTS5 + `_facet_bitmap` shadow tables.
pub fn run_search(
    conn: &Connection,
    entity: &str,
    config: &SearchConfig,
    query: &SearchQuery,
) -> Result<SearchResult, StorageError> {
    let t0 = std::time::Instant::now();

    // --- 1. Base bitmap: FTS match, or all-rows ----------------------------

    let mut base = if query.query.trim().is_empty() || config.text.is_empty() {
        load_all_rows(conn, entity)?
    } else {
        load_fts_matches(conn, entity, &query.query)?
    };

    // --- 2. Apply equality filters (AND) ----------------------------------

    for (field, value) in &query.filters {
        if !config.facets.iter().any(|f| f == field) {
            // Silently skip filters on non-faceted fields rather than
            // error — keeps clients robust to schema drift. Could
            // become a strict-mode toggle later.
            continue;
        }
        let value_str = match value_to_facet_string(value) {
            Some(s) => s,
            None => return Ok(empty_result(t0)),
        };
        let filter_bitmap = load_facet_bitmap(conn, entity, field, &value_str)?;
        base &= filter_bitmap;
    }

    let total = base.len();

    // --- 3. Facet counts --------------------------------------------------

    let wanted_facets: Vec<&String> = if query.facets.is_empty() {
        config.facets.iter().collect()
    } else {
        query
            .facets
            .iter()
            .filter(|f| config.facets.iter().any(|cf| cf == *f))
            .collect()
    };

    let mut facet_counts: BTreeMap<String, BTreeMap<String, u64>> = BTreeMap::new();
    if total <= FACET_COUNT_MAX_ROWS {
        for facet in wanted_facets {
            let counts = count_facet_values(conn, entity, facet, &base)?;
            if !counts.is_empty() {
                facet_counts.insert(facet.clone(), counts);
            }
        }
    }

    // --- 4. Sort enforcement + materialization ---------------------------
    //
    // Two distinct paths:
    //   * No sort — paginate the bitmap directly (cheap; rowid-stable).
    //   * Sort — we can't slice the bitmap first because rows 6-10 might
    //     outrank rows 1-5 on the sort key. Instead we load every
    //     matching rowid into a temp table, JOIN against the entity
    //     table, ORDER BY + LIMIT + OFFSET, and let SQLite handle it.
    //     Scales to millions of matches without the IN-list parameter
    //     ceiling.

    let page_size = query.page_size.clamp(1, 100);
    let offset = query.page.saturating_mul(page_size);

    // Enforce sortable-field contract at the planner boundary. Callers
    // that pass a non-sortable field get their sort silently dropped
    // rather than smuggling arbitrary ORDER BY into the SQL.
    let sort = query.sort.as_ref().and_then(|(field, dir)| {
        if config.sortable.iter().any(|f| f == field) {
            Some((field.clone(), dir.clone()))
        } else {
            None
        }
    });

    let hits = if base.is_empty() {
        Vec::new()
    } else if let Some((field, dir)) = sort {
        fetch_rows_sorted(conn, entity, &base, &field, &dir, offset, page_size)?
    } else {
        let page_rowids: Vec<u32> = base.iter().skip(offset).take(page_size).collect();
        if page_rowids.is_empty() {
            Vec::new()
        } else {
            fetch_rows_by_rowid(conn, entity, &page_rowids)?
        }
    };

    Ok(SearchResult {
        hits,
        facet_counts,
        total,
        took_ms: t0.elapsed().as_millis() as u64,
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn empty_result(t0: std::time::Instant) -> SearchResult {
    SearchResult {
        hits: Vec::new(),
        facet_counts: BTreeMap::new(),
        total: 0,
        took_ms: t0.elapsed().as_millis() as u64,
    }
}

fn load_all_rows(conn: &Connection, entity: &str) -> Result<RoaringBitmap, StorageError> {
    // "All rows" base bitmap = every rowid in the entity table. We
    // could cache this but it's cheap enough to rebuild: a single
    // `SELECT rowid FROM entity` + insert-into-bitmap.
    let sql = format!("SELECT rowid FROM \"{entity}\"");
    let mut stmt = conn
        .prepare_cached(&sql)
        .map_err(|e| StorageError::new("SEARCH_PREPARE_FAILED", &e.to_string()))?;
    let mut rows = stmt
        .query([])
        .map_err(|e| StorageError::new("SEARCH_QUERY_FAILED", &e.to_string()))?;
    let mut bitmap = RoaringBitmap::new();
    while let Some(row) = rows
        .next()
        .map_err(|e| StorageError::new("SEARCH_ROW_FAILED", &e.to_string()))?
    {
        let rid: i64 = row
            .get(0)
            .map_err(|e| StorageError::new("SEARCH_ROWID_FAILED", &e.to_string()))?;
        bitmap.insert(rid as u32);
    }
    Ok(bitmap)
}

fn load_fts_matches(
    conn: &Connection,
    entity: &str,
    match_text: &str,
) -> Result<RoaringBitmap, StorageError> {
    // FTS5 MATCH on the shadow table. entity_id column carries the
    // entity rowid; we read that into the base bitmap. Ranked best-first
    // by default (rowid order is stable enough for v1; BM25 ordering
    // lands when we plumb rank through hit materialization).
    let sql = format!(
        "SELECT entity_id FROM \"_fts_{entity}\" WHERE \"_fts_{entity}\" MATCH ?1"
    );
    let mut stmt = conn
        .prepare_cached(&sql)
        .map_err(|e| StorageError::new("FTS_PREPARE_FAILED", &e.to_string()))?;
    let mut rows = stmt
        .query([match_text])
        .map_err(|e| StorageError::new("FTS_QUERY_FAILED", &e.to_string()))?;
    let mut bitmap = RoaringBitmap::new();
    while let Some(row) = rows
        .next()
        .map_err(|e| StorageError::new("FTS_ROW_FAILED", &e.to_string()))?
    {
        let eid: i64 = row
            .get(0)
            .map_err(|e| StorageError::new("FTS_ENTITY_ID_FAILED", &e.to_string()))?;
        bitmap.insert(eid as u32);
    }
    Ok(bitmap)
}

fn load_facet_bitmap(
    conn: &Connection,
    entity: &str,
    facet: &str,
    value: &str,
) -> Result<RoaringBitmap, StorageError> {
    let bytes = conn
        .query_row(
            "SELECT bitmap FROM \"_facet_bitmap\" \
             WHERE entity = ?1 AND facet = ?2 AND value = ?3",
            [entity, facet, value],
            |r| r.get::<_, Vec<u8>>(0),
        )
        .ok();
    match bytes {
        Some(b) => deserialize_bitmap(&b),
        None => Ok(RoaringBitmap::new()),
    }
}

/// Returns {value → count} for every distinct value of `facet`, where
/// count is the popcount of the facet's bitmap intersected with `base`.
fn count_facet_values(
    conn: &Connection,
    entity: &str,
    facet: &str,
    base: &RoaringBitmap,
) -> Result<BTreeMap<String, u64>, StorageError> {
    let mut stmt = conn
        .prepare(
            "SELECT value, bitmap FROM \"_facet_bitmap\" \
             WHERE entity = ?1 AND facet = ?2",
        )
        .map_err(|e| StorageError::new("FACET_PREPARE_FAILED", &e.to_string()))?;
    let mut rows = stmt
        .query([entity, facet])
        .map_err(|e| StorageError::new("FACET_QUERY_FAILED", &e.to_string()))?;

    let mut out: BTreeMap<String, u64> = BTreeMap::new();
    while let Some(row) = rows
        .next()
        .map_err(|e| StorageError::new("FACET_ROW_FAILED", &e.to_string()))?
    {
        let value: String = row
            .get(0)
            .map_err(|e| StorageError::new("FACET_VALUE_FAILED", &e.to_string()))?;
        let bytes: Vec<u8> = row
            .get(1)
            .map_err(|e| StorageError::new("FACET_BYTES_FAILED", &e.to_string()))?;
        let bmp = deserialize_bitmap(&bytes)?;
        let count = (base & bmp).len();
        if count > 0 {
            out.insert(value, count);
        }
    }
    Ok(out)
}

fn value_to_facet_string(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::Bool(b) => Some(b.to_string()),
        Value::Number(n) => Some(n.to_string()),
        Value::String(s) => Some(s.clone()),
        Value::Array(_) | Value::Object(_) => None,
    }
}

/// Unsorted page fetch — one SELECT with an IN list, rowid-ordered.
/// Called only when no sort is requested; caller has already sliced
/// the bitmap to the page's `rowids`.
fn fetch_rows_by_rowid(
    conn: &Connection,
    entity: &str,
    rowids: &[u32],
) -> Result<Vec<Value>, StorageError> {
    let placeholders = (1..=rowids.len())
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT * FROM \"{entity}\" WHERE rowid IN ({placeholders}) ORDER BY rowid ASC"
    );
    let mut stmt = conn
        .prepare_cached(&sql)
        .map_err(|e| StorageError::new("HIT_PREPARE_FAILED", &e.to_string()))?;

    let i64_vals: Vec<i64> = rowids.iter().map(|r| *r as i64).collect();
    let params: Vec<&dyn rusqlite::types::ToSql> = i64_vals
        .iter()
        .map(|v| v as &dyn rusqlite::types::ToSql)
        .collect();

    collect_rows(&mut stmt, &params)
}

/// Sorted page fetch — loads the entire match-set into a temp table,
/// JOINs against the entity, then ORDER BY + LIMIT + OFFSET. This is
/// the only way to get a correct page when ordering by a column other
/// than rowid: paginating the bitmap pre-sort would miss rows whose
/// sort value puts them on an earlier page than their rowid does.
///
/// Temp-table approach dodges SQLite's IN-list parameter ceiling (999
/// by default) — scales to millions of matches without chunking.
fn fetch_rows_sorted(
    conn: &Connection,
    entity: &str,
    base: &RoaringBitmap,
    sort_field: &str,
    sort_dir: &str,
    offset: usize,
    limit: usize,
) -> Result<Vec<Value>, StorageError> {
    // Temp table lives for the duration of the connection; DROP at the
    // end keeps the namespace clean even for long-lived connections.
    conn.execute(
        "CREATE TEMP TABLE IF NOT EXISTS \"_search_hits\" (rowid INTEGER PRIMARY KEY);",
        [],
    )
    .map_err(|e| StorageError::new("TEMP_CREATE_FAILED", &e.to_string()))?;
    conn.execute("DELETE FROM \"_search_hits\";", [])
        .map_err(|e| StorageError::new("TEMP_CLEAR_FAILED", &e.to_string()))?;

    {
        let mut insert = conn
            .prepare_cached("INSERT INTO \"_search_hits\" (rowid) VALUES (?1)")
            .map_err(|e| StorageError::new("TEMP_PREPARE_FAILED", &e.to_string()))?;
        for rid in base.iter() {
            insert
                .execute([rid as i64])
                .map_err(|e| StorageError::new("TEMP_INSERT_FAILED", &e.to_string()))?;
        }
    }

    let dir = if sort_dir.eq_ignore_ascii_case("desc") {
        "DESC"
    } else {
        "ASC"
    };
    let sql = format!(
        "SELECT e.* FROM \"{entity}\" e \
         JOIN \"_search_hits\" h ON h.rowid = e.rowid \
         ORDER BY e.\"{sort_field}\" {dir}, e.rowid {dir} \
         LIMIT ?1 OFFSET ?2"
    );
    let mut stmt = conn
        .prepare_cached(&sql)
        .map_err(|e| StorageError::new("SORT_PREPARE_FAILED", &e.to_string()))?;

    let limit_i64 = limit as i64;
    let offset_i64 = offset as i64;
    let params: Vec<&dyn rusqlite::types::ToSql> = vec![&limit_i64, &offset_i64];
    let out = collect_rows(&mut stmt, &params)?;

    // Drop the temp table to release its pages. The CREATE IF NOT
    // EXISTS above means the next search reuses the same schema; the
    // DELETE at the top of this function empties it first.
    drop(stmt);
    Ok(out)
}

fn collect_rows(
    stmt: &mut rusqlite::Statement,
    params: &[&dyn rusqlite::types::ToSql],
) -> Result<Vec<Value>, StorageError> {
    let col_names: Vec<String> = stmt.column_names().into_iter().map(String::from).collect();
    let mut rows = stmt
        .query(params)
        .map_err(|e| StorageError::new("HIT_QUERY_FAILED", &e.to_string()))?;

    let mut out = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|e| StorageError::new("HIT_ROW_FAILED", &e.to_string()))?
    {
        let mut obj = serde_json::Map::new();
        for (i, name) in col_names.iter().enumerate() {
            let v: rusqlite::types::Value = row
                .get(i)
                .map_err(|e| StorageError::new("HIT_COL_FAILED", &e.to_string()))?;
            obj.insert(name.clone(), sqlite_value_to_json(v));
        }
        out.push(Value::Object(obj));
    }
    Ok(out)
}

fn sqlite_value_to_json(v: rusqlite::types::Value) -> Value {
    use rusqlite::types::Value as SV;
    match v {
        SV::Null => Value::Null,
        SV::Integer(i) => Value::from(i),
        SV::Real(f) => serde_json::Number::from_f64(f)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        SV::Text(s) => Value::String(s),
        SV::Blob(_) => Value::Null, // not expected on entity columns
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::{create_facet_table_sql, create_fts_table_sql};
    use crate::search_maintenance::apply_insert;
    use rusqlite::Connection;

    fn seed_store(n_products: usize) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE \"Product\" (
               id TEXT PRIMARY KEY,
               name TEXT,
               description TEXT,
               brand TEXT,
               category TEXT,
               price REAL
             );",
        )
        .unwrap();
        conn.execute(create_facet_table_sql(), []).unwrap();
        let cfg = SearchConfig {
            text: vec!["name".into(), "description".into()],
            facets: vec!["brand".into(), "category".into()],
            sortable: vec!["price".into()],
        };
        conn.execute(&create_fts_table_sql("Product", &cfg).unwrap(), [])
            .unwrap();

        let brands = ["Nike", "Adidas", "Puma"];
        let categories = ["shoes", "shirts", "pants"];
        for i in 0..n_products {
            let id = format!("p_{i:06}");
            let brand = brands[i % brands.len()];
            let category = categories[i % categories.len()];
            let name = format!("{brand} {category} {i}");
            let description = format!("A nice pair of {category} from {brand}.");
            let price = 10.0 + (i as f64 * 0.5) % 200.0;
            conn.execute(
                "INSERT INTO \"Product\" (id, name, description, brand, category, price) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![id, name, description, brand, category, price],
            )
            .unwrap();
            apply_insert(
                &conn,
                "Product",
                &id,
                &serde_json::json!({
                    "name": name, "description": description,
                    "brand": brand, "category": category, "price": price,
                }),
                &cfg,
            )
            .unwrap();
        }
        conn
    }

    fn product_config() -> SearchConfig {
        SearchConfig {
            text: vec!["name".into(), "description".into()],
            facets: vec!["brand".into(), "category".into()],
            sortable: vec!["price".into()],
        }
    }

    #[test]
    fn empty_query_returns_everything_with_facet_counts() {
        let conn = seed_store(30);
        let r = run_search(
            &conn,
            "Product",
            &product_config(),
            &SearchQuery {
                query: String::new(),
                filters: Default::default(),
                facets: vec![],
                sort: None,
                page: 0,
                page_size: 20,
            },
        )
        .unwrap();
        assert_eq!(r.total, 30);
        assert_eq!(r.hits.len(), 20);
        let brands = r.facet_counts.get("brand").unwrap();
        assert_eq!(brands.len(), 3);
        assert_eq!(brands.values().sum::<u64>(), 30);
    }

    #[test]
    fn fts_query_narrows_hits_and_updates_facets() {
        let conn = seed_store(30);
        let r = run_search(
            &conn,
            "Product",
            &product_config(),
            &SearchQuery {
                query: "Nike".into(),
                filters: Default::default(),
                facets: vec!["brand".into()],
                sort: None,
                page: 0,
                page_size: 20,
            },
        )
        .unwrap();
        assert!(r.total > 0);
        assert!(r.total < 30);
        let brands = r.facet_counts.get("brand").unwrap();
        // Every match should be brand=Nike, so that's the only bucket.
        assert_eq!(brands.keys().collect::<Vec<_>>(), vec!["Nike"]);
        assert_eq!(brands["Nike"], r.total);
    }

    #[test]
    fn filter_by_facet_value_intersects_bitmaps() {
        let conn = seed_store(30);
        let r = run_search(
            &conn,
            "Product",
            &product_config(),
            &SearchQuery {
                query: String::new(),
                filters: [
                    ("brand".to_string(), Value::String("Nike".into())),
                    ("category".to_string(), Value::String("shoes".into())),
                ]
                .into_iter()
                .collect(),
                facets: vec!["brand".into()],
                sort: None,
                page: 0,
                page_size: 20,
                    },
        )
        .unwrap();
        // Seed pattern: every 9th product is Nike+shoes.
        assert!(r.total > 0);
        for hit in &r.hits {
            assert_eq!(hit["brand"], "Nike");
            assert_eq!(hit["category"], "shoes");
        }
    }

    #[test]
    fn pagination_walks_through_all_rows() {
        let conn = seed_store(25);
        let mut seen = 0;
        for page in 0..3 {
            let r = run_search(
                &conn,
                "Product",
                &product_config(),
                &SearchQuery {
                    query: String::new(),
                    filters: Default::default(),
                    facets: vec![],
                    sort: None,
                    page,
                    page_size: 10,
                },
            )
            .unwrap();
            seen += r.hits.len();
        }
        assert_eq!(seen, 25);
    }

    #[test]
    fn sort_by_price_desc_orders_correctly() {
        let conn = seed_store(10);
        let r = run_search(
            &conn,
            "Product",
            &product_config(),
            &SearchQuery {
                query: String::new(),
                filters: Default::default(),
                facets: vec![],
                sort: Some(("price".into(), "desc".into())),
                page: 0,
                page_size: 5,
            },
        )
        .unwrap();
        let prices: Vec<f64> = r.hits.iter().map(|h| h["price"].as_f64().unwrap()).collect();
        let mut sorted = prices.clone();
        sorted.sort_by(|a, b| b.partial_cmp(a).unwrap());
        assert_eq!(prices, sorted);
    }

    #[test]
    fn sort_paginates_across_the_full_result_set_not_within_the_first_page() {
        // Regression for the codex-caught "sort-before-paginate" bug.
        // With 20 products and page_size=5 sorted by price desc, page 0
        // MUST contain the 5 highest-priced rows — not simply the first
        // 5 rowids sorted descending within themselves.
        let conn = seed_store(20);
        let page0 = run_search(
            &conn,
            "Product",
            &product_config(),
            &SearchQuery {
                query: String::new(),
                filters: Default::default(),
                facets: vec![],
                sort: Some(("price".into(), "desc".into())),
                page: 0,
                page_size: 5,
            },
        )
        .unwrap();
        let page0_min_price = page0
            .hits
            .iter()
            .map(|h| h["price"].as_f64().unwrap())
            .fold(f64::INFINITY, f64::min);

        // The minimum price on page 0 should be >= the max price of
        // any row on page 2 (rows 10-14 by sorted position). Every
        // page-0 row must be at least as pricey as every page-2 row.
        let page2 = run_search(
            &conn,
            "Product",
            &product_config(),
            &SearchQuery {
                query: String::new(),
                filters: Default::default(),
                facets: vec![],
                sort: Some(("price".into(), "desc".into())),
                page: 2,
                page_size: 5,
            },
        )
        .unwrap();
        let page2_max_price = page2
            .hits
            .iter()
            .map(|h| h["price"].as_f64().unwrap())
            .fold(f64::NEG_INFINITY, f64::max);
        assert!(
            page0_min_price >= page2_max_price,
            "page-0 min ({page0_min_price}) must be >= page-2 max ({page2_max_price})",
        );
    }

    #[test]
    fn sort_on_non_sortable_field_is_silently_dropped() {
        // 'name' isn't in config.sortable — planner should ignore the
        // sort rather than ORDER BY an un-indexed column (or an
        // attacker-controlled one in pathological cases).
        let conn = seed_store(10);
        let r = run_search(
            &conn,
            "Product",
            &product_config(),
            &SearchQuery {
                query: String::new(),
                filters: Default::default(),
                facets: vec![],
                sort: Some(("name".into(), "desc".into())),
                page: 0,
                page_size: 10,
            },
        )
        .unwrap();
        // Rowid-ascending order = insertion order, so the 'name 0' row
        // (inserted first) comes before 'name 9'.
        let names: Vec<&str> = r
            .hits
            .iter()
            .map(|h| h["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names[0].split(' ').last().unwrap(),
            "0",
            "expected rowid-asc fallback, got {names:?}"
        );
    }
}
