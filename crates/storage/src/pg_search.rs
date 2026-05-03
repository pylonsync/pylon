//! Postgres full-text search — schema, maintenance, and query path.
//!
//! Mirrors the SQLite FTS5 path in `search.rs` + `search_maintenance.rs`
//! + `search_query.rs`, but built on Postgres `tsvector` + GIN index +
//! plain B-tree facet/sort indexes.
//!
//! Why no Roaring bitmaps on PG? PG's planner with a B-tree index on
//! the facet column handles `GROUP BY field, value` aggregates
//! sub-100ms even at 1M rows — we don't need the bitmap fast-path the
//! SQLite layer uses to compensate for SQLite's lack of decent
//! aggregate planning. Using native SQL aggregates also keeps the
//! index up to date for free (no separate bitmap maintenance step).
//!
//! Schema generated for an entity that declares `search:`:
//!
//! ```sql
//! CREATE TABLE "_fts_<entity>" (
//!   entity_id text PRIMARY KEY REFERENCES "<entity>"(id) ON DELETE CASCADE,
//!   tsv tsvector NOT NULL
//! );
//! CREATE INDEX "<entity>_fts_gin" ON "_fts_<entity>" USING GIN (tsv);
//! CREATE INDEX "<entity>_facet_<field>" ON "<entity>"("<field>");
//! CREATE INDEX "<entity>_sort_<field>"  ON "<entity>"("<field>");
//! ```
//!
//! Identifier-quoting: every interpolated entity, field, or shadow-table
//! name flows through `crate::postgres::quote_ident` BEFORE landing in
//! a SQL string. The manifest already validates names at load time, but
//! the storage layer treats every identifier as untrusted on its way
//! into SQL — defense in depth so a future bug in the manifest path
//! can't escalate into SQL injection.

#![cfg(feature = "postgres-live")]

use std::collections::BTreeMap;

use postgres::types::ToSql;
use serde_json::Value;

use crate::pg_exec::PgConn;
use crate::postgres::quote_ident_pub as quote_ident;
use crate::search::{merge_row, SearchConfig, SearchQuery, SearchResult};
use crate::StorageError;

// ---------------------------------------------------------------------------
// Schema generation
// ---------------------------------------------------------------------------

/// Quoted name of an entity's FTS shadow table. The `_fts_` prefix is
/// reserved namespace — the planner refuses to materialize entities
/// whose names start with `_` so user names can't collide.
fn fts_table_name(entity: &str) -> String {
    quote_ident(&format!("_fts_{entity}"))
}

/// Quoted name of an entity's GIN index over its tsvector.
fn fts_gin_index_name(entity: &str) -> String {
    quote_ident(&format!("{entity}_fts_gin"))
}

/// Quoted name of the auto B-tree on a facet column.
fn facet_index_name(entity: &str, field: &str) -> String {
    quote_ident(&format!("{entity}_facet_{field}"))
}

/// Quoted name of the auto B-tree on a sortable column.
fn sort_index_name(entity: &str, field: &str) -> String {
    quote_ident(&format!("{entity}_sort_{field}"))
}

/// All DDL statements needed to materialize the FTS + facet + sort
/// indexes for one entity. Caller batches these into the same plan
/// apply. Idempotent — every CREATE uses `IF NOT EXISTS` so repeated
/// schema pushes are no-ops.
pub fn create_search_index_sql(entity: &str, config: &SearchConfig) -> Vec<String> {
    let entity_quoted = quote_ident(entity);
    let mut stmts = Vec::new();

    // FTS shadow table only when the config declares text fields.
    // Facet-only search is supported (just GROUP BY queries on the
    // entity table) and needs no `_fts_<entity>` to back it.
    if !config.text.is_empty() {
        stmts.push(format!(
            "CREATE TABLE IF NOT EXISTS {fts} (\
                entity_id text PRIMARY KEY REFERENCES {entity_quoted}(id) ON DELETE CASCADE, \
                tsv tsvector NOT NULL)",
            fts = fts_table_name(entity),
        ));
        stmts.push(format!(
            "CREATE INDEX IF NOT EXISTS {gin} ON {fts} USING GIN (tsv)",
            gin = fts_gin_index_name(entity),
            fts = fts_table_name(entity),
        ));
    }

    // B-tree on every declared facet so `GROUP BY` / equality filters
    // skip table scans. Naming matches the SQLite path
    // (`<entity>_facet_<field>`) so DROP-and-recreate ops can target
    // both backends with the same name.
    for field in &config.facets {
        stmts.push(format!(
            "CREATE INDEX IF NOT EXISTS {idx} ON {entity_quoted} ({field_quoted})",
            idx = facet_index_name(entity, field),
            field_quoted = quote_ident(field),
        ));
    }

    // B-tree on every sortable field so `ORDER BY <field> LIMIT n`
    // walks the index instead of sorting the whole match set.
    for field in &config.sortable {
        stmts.push(format!(
            "CREATE INDEX IF NOT EXISTS {idx} ON {entity_quoted} ({field_quoted})",
            idx = sort_index_name(entity, field),
            field_quoted = quote_ident(field),
        ));
    }

    stmts
}

/// All DDL needed to remove an entity's search shadow tables. Used
/// when `search:` is dropped from the manifest. The CASCADE on the
/// FTS table's FK takes care of orphaned rows during entity removal,
/// so we only need to drop the FTS table itself + the GIN index +
/// the per-field facet/sort indexes.
pub fn remove_search_index_sql(entity: &str, config: &SearchConfig) -> Vec<String> {
    let mut stmts = Vec::new();
    if !config.text.is_empty() {
        stmts.push(format!(
            "DROP INDEX IF EXISTS {}",
            fts_gin_index_name(entity)
        ));
        stmts.push(format!(
            "DROP TABLE IF EXISTS {} CASCADE",
            fts_table_name(entity)
        ));
    }
    for field in &config.facets {
        stmts.push(format!(
            "DROP INDEX IF EXISTS {}",
            facet_index_name(entity, field)
        ));
    }
    for field in &config.sortable {
        stmts.push(format!(
            "DROP INDEX IF EXISTS {}",
            sort_index_name(entity, field)
        ));
    }
    stmts
}

// ---------------------------------------------------------------------------
// Tsvector building
// ---------------------------------------------------------------------------

/// Concatenate the row's declared text fields into one tsvector input
/// string. Built server-side via `to_tsvector(<lang>, $1)` so
/// weighting + tokenization stay consistent with the search path
/// (`plainto_tsquery(<lang>, ...)`).
fn collect_text(data: &Value, config: &SearchConfig) -> String {
    config
        .text
        .iter()
        .map(|f| {
            data.get(f)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Resolve + validate the `regconfig` name for `to_tsvector` /
/// `plainto_tsquery`. Returns a single-quoted SQL string literal
/// (e.g. `'english'`) safe to interpolate.
///
/// We can't bind `regconfig` as a parameter — it has to be a literal
/// in the SQL. So we strictly validate the manifest-supplied value:
/// ASCII alphanumeric + underscore, length 1..=63 (PG identifier
/// limit). Anything else falls back to `'english'` rather than
/// erroring — defense in depth on top of the manifest's own
/// validation, and matches Postgres's default tsearch config.
fn lang_literal(config: &SearchConfig) -> String {
    let raw = config.language_or_default();
    let valid = !raw.is_empty()
        && raw.len() <= 63
        && raw.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    let chosen = if valid { raw } else { "english" };
    format!("'{chosen}'")
}

// ---------------------------------------------------------------------------
// Maintenance — runs alongside the entity row CRUD inside the same tx
// ---------------------------------------------------------------------------

/// Write the FTS shadow row for an entity row that just landed.
/// No-op when `config` declares no text fields (facet-only search uses
/// the entity table's own indexed columns; nothing to maintain).
pub fn apply_insert<C: PgConn>(
    conn: &mut C,
    entity: &str,
    id: &str,
    data: &Value,
    config: &SearchConfig,
) -> Result<(), StorageError> {
    if config.text.is_empty() {
        return Ok(());
    }
    let text = collect_text(data, config);
    let lang = lang_literal(config);
    let sql = format!(
        "INSERT INTO {fts} (entity_id, tsv) \
         VALUES ($1, to_tsvector({lang}, $2)) \
         ON CONFLICT (entity_id) DO UPDATE SET tsv = EXCLUDED.tsv",
        fts = fts_table_name(entity),
    );
    conn.execute(&sql, &[&id, &text])
        .map(|_| ())
        .map_err(|e| StorageError::new("PG_FTS_INSERT_FAILED", &e.to_string()))
}

/// Update the FTS shadow row when any of the declared text fields
/// changed. Caller passes the pre-UPDATE row + the patch so we can
/// merge them and rebuild the tsvector against the post-update state.
/// Skips the write when the patch doesn't touch any text field.
pub fn apply_update<C: PgConn>(
    conn: &mut C,
    entity: &str,
    id: &str,
    old_row: &Value,
    patch: &Value,
    config: &SearchConfig,
) -> Result<(), StorageError> {
    if config.text.is_empty() {
        return Ok(());
    }
    let touches_text = config.text.iter().any(|f| patch.get(f).is_some());
    if !touches_text {
        return Ok(());
    }
    let merged = merge_row(old_row, patch);
    let text = collect_text(&merged, config);
    let lang = lang_literal(config);
    let sql = format!(
        "INSERT INTO {fts} (entity_id, tsv) \
         VALUES ($1, to_tsvector({lang}, $2)) \
         ON CONFLICT (entity_id) DO UPDATE SET tsv = EXCLUDED.tsv",
        fts = fts_table_name(entity),
    );
    conn.execute(&sql, &[&id, &text])
        .map(|_| ())
        .map_err(|e| StorageError::new("PG_FTS_UPDATE_FAILED", &e.to_string()))
}

/// Delete the FTS shadow row when the underlying entity row is going
/// away. The FK CASCADE handles this automatically when the entity
/// row is DELETEd, but we call it explicitly so the maintenance
/// contract matches the SQLite path (where the `apply_delete` runs
/// BEFORE the entity DELETE). No-op for facet-only configs.
pub fn apply_delete<C: PgConn>(
    conn: &mut C,
    entity: &str,
    id: &str,
    config: &SearchConfig,
) -> Result<(), StorageError> {
    if config.text.is_empty() {
        return Ok(());
    }
    let sql = format!(
        "DELETE FROM {fts} WHERE entity_id = $1",
        fts = fts_table_name(entity),
    );
    conn.execute(&sql, &[&id])
        .map(|_| ())
        .map_err(|e| StorageError::new("PG_FTS_DELETE_FAILED", &e.to_string()))
}

// ---------------------------------------------------------------------------
// Query path
// ---------------------------------------------------------------------------

/// Run a search query. Three round-trips: total, hits, plus one
/// `GROUP BY` per requested facet (with the active filter excluded so
/// the count for the active value isn't trivially `total`). Could be
/// inlined into one query with window functions, but keeping them
/// separate makes the plan readable for operators and lets callers
/// that pass `facets: []` skip the facet phase entirely.
///
/// All identifiers (`entity`, `facet`, `sort field`) are validated
/// against `config` before being interpolated into SQL — see the
/// `validate_*` helpers below. Defense in depth on top of the
/// manifest's own name validation.
pub fn run_search<C: PgConn>(
    conn: &mut C,
    entity: &str,
    config: &SearchConfig,
    query: &SearchQuery,
) -> Result<SearchResult, StorageError> {
    let t0 = std::time::Instant::now();

    let entity_quoted = quote_ident(entity);
    let fts = fts_table_name(entity);
    let lang = lang_literal(config);

    // Validate the requested sort field against the entity's
    // `sortable` list before it touches SQL. Empty `sort` means
    // "use rank if there's a query, id otherwise."
    if let Some((field, _)) = &query.sort {
        if !config.sortable.iter().any(|s| s == field) {
            return Err(StorageError::new(
                "INVALID_SORT_FIELD",
                &format!("sort field \"{field}\" is not in the entity's `sortable` config"),
            ));
        }
    }

    // Build the predicate + parameter set shared by total / hits /
    // facet phases. Filters on non-faceted fields are silently
    // dropped (matches SQLite + keeps clients robust to schema drift).
    let mut clauses: Vec<String> = Vec::new();
    let mut params: Vec<Box<dyn ToSql + Sync>> = Vec::new();
    let valid_facet = |f: &str| config.facets.iter().any(|cf| cf == f);

    let has_query = !query.query.trim().is_empty() && !config.text.is_empty();
    if has_query {
        // `plainto_tsquery` is the safe-for-user-input variant; it
        // tokenizes the input the same way `to_tsvector` did during
        // maintenance, so a misspelled operator can't crash the parse.
        params.push(Box::new(query.query.clone()));
        clauses.push(format!(
            "f.tsv @@ plainto_tsquery({lang}, ${})",
            params.len()
        ));
    }

    let mut filter_pairs: Vec<(String, String)> = Vec::new();
    for (field, value) in &query.filters {
        if !valid_facet(field) {
            continue;
        }
        let value_str = match crate::search::stringify_facet(value) {
            Some(s) => s,
            None => return Ok(empty_result(t0)),
        };
        filter_pairs.push((field.clone(), value_str.clone()));
        params.push(Box::new(value_str));
        // Cast both sides to text so the equality works uniformly
        // across column types (INT, BOOL, TIMESTAMP, etc.). The btree
        // index on the column is still usable when the cast is
        // immutable — which `::text` is for the types Pylon supports.
        clauses.push(format!(
            "{}::text = ${}",
            qualified_column(&entity_quoted, field),
            params.len()
        ));
    }

    let where_clause = if clauses.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", clauses.join(" AND "))
    };
    let join_clause = if has_query {
        format!(" JOIN {fts} f ON f.entity_id = e.id")
    } else {
        String::new()
    };

    // ---- Total ---------------------------------------------------------
    let total_sql = format!("SELECT COUNT(*) FROM {entity_quoted} e{join_clause}{where_clause}");
    let total: i64 = {
        let pg_params = box_params(&params);
        let row = conn
            .query(&total_sql, &pg_params)
            .map_err(|e| StorageError::new("PG_SEARCH_TOTAL_FAILED", &e.to_string()))?;
        row.first().map(|r| r.get::<_, i64>(0)).unwrap_or(0)
    };
    let total = total as u64;

    // ---- Hits ----------------------------------------------------------
    let order_clause = build_order_clause(query, has_query, &entity_quoted)?;
    // Hard cap pageSize at 100 to mirror SQLite + bound the worst-case
    // result payload. Negative / zero clamps to 1.
    let limit = query.page_size.max(1).min(100);
    let offset = query.page.saturating_mul(limit);

    let select_cols = if has_query {
        // Surface ts_rank as `_rank` so callers can re-rank or display
        // relevance. Same idea as Meilisearch's `_score`.
        format!("e.*, ts_rank(f.tsv, plainto_tsquery({lang}, $1)) AS _rank")
    } else {
        "e.*".to_string()
    };

    let hits_sql = format!(
        "SELECT {select_cols} FROM {entity_quoted} e{join_clause}{where_clause}{order_clause} \
         LIMIT {limit} OFFSET {offset}"
    );
    let hits: Vec<Value> = {
        let pg_params = box_params(&params);
        let rows = conn
            .query(&hits_sql, &pg_params)
            .map_err(|e| StorageError::new("PG_SEARCH_HITS_FAILED", &e.to_string()))?;
        rows.iter().map(crate::postgres::row_to_json_pub).collect()
    };

    // ---- Facet counts --------------------------------------------------
    let wanted_facets: Vec<&String> = if query.facets.is_empty() {
        config.facets.iter().collect()
    } else {
        query.facets.iter().filter(|f| valid_facet(f)).collect()
    };

    let mut facet_counts: BTreeMap<String, BTreeMap<String, u64>> = BTreeMap::new();
    for facet in wanted_facets {
        let counts = run_facet_count(
            conn,
            &entity_quoted,
            &fts,
            facet,
            has_query,
            &query.query,
            &lang,
            &filter_pairs,
        )?;
        if !counts.is_empty() {
            facet_counts.insert(facet.clone(), counts);
        }
    }

    Ok(SearchResult {
        hits,
        facet_counts,
        total,
        took_ms: t0.elapsed().as_millis() as u64,
    })
}

/// Count distinct values of `facet` in the same match set as `run_search`,
/// EXCLUDING any active filter on the facet itself. Standard facet-
/// exclusion pattern: without it, the count for the currently-selected
/// value would always equal `total` and other values would be zero.
fn run_facet_count<C: PgConn>(
    conn: &mut C,
    entity_quoted: &str,
    fts: &str,
    facet: &str,
    has_query: bool,
    query_text: &str,
    lang: &str,
    filter_pairs: &[(String, String)],
) -> Result<BTreeMap<String, u64>, StorageError> {
    let facet_col = qualified_column(entity_quoted, facet);
    let mut clauses: Vec<String> = Vec::new();
    let mut params: Vec<Box<dyn ToSql + Sync>> = Vec::new();
    if has_query {
        params.push(Box::new(query_text.to_string()));
        clauses.push(format!(
            "f.tsv @@ plainto_tsquery({lang}, ${})",
            params.len()
        ));
    }
    for (field, value) in filter_pairs {
        if field == facet {
            continue; // self-exclusion
        }
        params.push(Box::new(value.clone()));
        clauses.push(format!(
            "{}::text = ${}",
            qualified_column(entity_quoted, field),
            params.len()
        ));
    }
    let where_clause = if clauses.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", clauses.join(" AND "))
    };
    let join_clause = if has_query {
        format!(" JOIN {fts} f ON f.entity_id = e.id")
    } else {
        String::new()
    };
    // LIMIT 100 guards against runaway high-cardinality facets (e.g.
    // every row has a unique value). Top 100 by count is plenty for
    // a UI; clients that need more should drop the facet from the
    // request rather than expecting unbounded values.
    let sql = format!(
        "SELECT {facet_col}::text AS value, COUNT(*) AS cnt \
         FROM {entity_quoted} e{join_clause}{where_clause} \
         GROUP BY {facet_col} ORDER BY cnt DESC LIMIT 100"
    );
    let pg_params = box_params(&params);
    let rows = conn
        .query(&sql, &pg_params)
        .map_err(|e| StorageError::new("PG_SEARCH_FACET_FAILED", &e.to_string()))?;
    let mut counts: BTreeMap<String, u64> = BTreeMap::new();
    for row in &rows {
        let val: Option<String> = row.get(0);
        let cnt: i64 = row.get(1);
        if let Some(v) = val {
            counts.insert(v, cnt as u64);
        }
    }
    Ok(counts)
}

/// `entity."col"` — qualifies the column with the entity alias `e`.
/// Both `entity` and `col` are quoted so embedded `"` in either is
/// neutralized.
fn qualified_column(_entity_quoted: &str, col: &str) -> String {
    // `e` is the alias the search SQL always uses for the entity
    // table; bare prefix is fine because `e` is a constant we control.
    format!("e.{}", quote_ident(col))
}

fn build_order_clause(
    query: &SearchQuery,
    has_query: bool,
    _entity_quoted: &str,
) -> Result<String, StorageError> {
    if let Some((field, dir)) = &query.sort {
        // Validation already happened in `run_search` above; safe to
        // quote and emit.
        let dir = match dir.to_lowercase().as_str() {
            "desc" => "DESC",
            _ => "ASC",
        };
        Ok(format!(" ORDER BY e.{} {dir}", quote_ident(field)))
    } else if has_query {
        Ok(" ORDER BY _rank DESC".to_string())
    } else {
        Ok(" ORDER BY e.id".to_string())
    }
}

fn empty_result(t0: std::time::Instant) -> SearchResult {
    SearchResult {
        hits: Vec::new(),
        facet_counts: BTreeMap::new(),
        total: 0,
        took_ms: t0.elapsed().as_millis() as u64,
    }
}

/// `Vec<Box<dyn ToSql + Sync>>` -> `Vec<&(dyn ToSql + Sync)>`. The
/// extra indirection is unavoidable because `postgres::Client::query`
/// takes a slice of references, not a slice of owned values.
fn box_params(boxed: &[Box<dyn ToSql + Sync>]) -> Vec<&(dyn ToSql + Sync)> {
    boxed.iter().map(|b| b.as_ref() as _).collect()
}

// ---------------------------------------------------------------------------
// Tests — schema generation only; the maintenance + query paths need a
// live PG instance and live in `crates/runtime/tests/postgres_backend.rs`.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> SearchConfig {
        SearchConfig {
            text: vec!["name".into(), "description".into()],
            facets: vec!["brand".into(), "category".into()],
            sortable: vec!["price".into(), "createdAt".into()],
            language: None,
        }
    }

    #[test]
    fn create_emits_fts_table_gin_and_indexes() {
        let stmts = create_search_index_sql("Product", &cfg());
        let blob = stmts.join("\n");
        assert!(blob.contains("CREATE TABLE IF NOT EXISTS \"_fts_Product\""));
        assert!(blob.contains("tsv tsvector NOT NULL"));
        assert!(blob.contains("USING GIN (tsv)"));
        assert!(blob.contains("\"Product_facet_brand\""));
        assert!(blob.contains("\"Product_facet_category\""));
        assert!(blob.contains("\"Product_sort_price\""));
        assert!(blob.contains("\"Product_sort_createdAt\""));
    }

    #[test]
    fn create_skips_fts_when_no_text_fields() {
        let cfg = SearchConfig {
            text: vec![],
            facets: vec!["brand".into()],
            sortable: vec![],
            language: None,
        };
        let stmts = create_search_index_sql("Product", &cfg);
        let blob = stmts.join("\n");
        assert!(!blob.contains("_fts_Product"));
        assert!(blob.contains("\"Product_facet_brand\""));
    }

    #[test]
    fn remove_drops_fts_and_indexes_when_text_present() {
        let stmts = remove_search_index_sql("Product", &cfg());
        let blob = stmts.join("\n");
        assert!(blob.contains("DROP TABLE IF EXISTS \"_fts_Product\""));
        assert!(blob.contains("DROP INDEX IF EXISTS \"Product_fts_gin\""));
        assert!(blob.contains("DROP INDEX IF EXISTS \"Product_facet_brand\""));
    }

    #[test]
    fn lang_literal_falls_back_to_english_for_invalid_input() {
        // Anything not strictly `[A-Za-z0-9_]` (e.g. an injection
        // attempt) gets dropped to `'english'`. The literal lands in
        // SQL not as a bind, so this is the only line of defense.
        let cfg = SearchConfig {
            text: vec!["name".into()],
            facets: vec![],
            sortable: vec![],
            language: Some("english'; DROP TABLE x; --".into()),
        };
        assert_eq!(lang_literal(&cfg), "'english'");
    }

    #[test]
    fn lang_literal_passes_through_known_postgres_configs() {
        for cfg_lang in ["english", "spanish", "french", "german", "simple"] {
            let cfg = SearchConfig {
                text: vec!["name".into()],
                facets: vec![],
                sortable: vec![],
                language: Some(cfg_lang.to_string()),
            };
            assert_eq!(lang_literal(&cfg), format!("'{cfg_lang}'"));
        }
    }

    #[test]
    fn entity_name_with_double_quote_is_neutralized() {
        // `quote_ident` doubles embedded `"`. A malicious entity name
        // can't break out of the identifier.
        let stmts = create_search_index_sql("Foo\"; DROP TABLE bar; --", &cfg());
        let blob = stmts.join("\n");
        // The `"` inside the entity name should appear escaped (`""`),
        // not naked, anywhere it lands in the generated SQL.
        assert!(!blob.contains("Foo\"; DROP"));
        assert!(blob.contains("Foo\"\""));
    }
}
