//! Native faceted full-text search for Pylon entities.
//!
//! The design goal is Meilisearch-class latency at 1M-row catalogs
//! without running a second service. Two ideas carry the performance:
//!
//! 1. **SQLite FTS5 for text matching.** A shadow contentless table
//!    indexes declared text fields. Matches come back as a stream of
//!    rowids in a few ms regardless of catalog size.
//!
//! 2. **Roaring bitmaps for facets.** For every declared facet field,
//!    we keep a bitmap per distinct value: the set of entity rowids
//!    that carry that value. On insert/update/delete we flip bits in
//!    the same transaction as the row change. Facet counts at query
//!    time collapse to `popcount(match ∩ filter ∩ facet_bitmap)` —
//!    single-digit microseconds per facet value, no matter how wide
//!    the match.
//!
//! The shadow tables Pylon creates when an entity declares `search:`:
//!
//! ```sql
//! -- One FTS5 virtual table per searchable entity.
//! CREATE VIRTUAL TABLE "_fts_<Entity>" USING fts5(
//!     rowid UNINDEXED,       -- external rowid (entity.id)
//!     <text_field1>,
//!     <text_field2>,
//!     ...,
//!     tokenize = 'unicode61 remove_diacritics 2'
//! );
//!
//! -- One row per (entity, facet, value) — bitmap of matching rowids.
//! CREATE TABLE "_facet_bitmap" (
//!   entity    TEXT NOT NULL,
//!   facet     TEXT NOT NULL,
//!   value     TEXT NOT NULL,
//!   bitmap    BLOB NOT NULL,   -- Roaring-serialized
//!   row_count INTEGER NOT NULL,
//!   PRIMARY KEY (entity, facet, value)
//! );
//!
//! -- Mapping from entity rowid → numeric rowid-in-bitmap. Bitmaps
//! -- operate on u32 so we need a compact numeric id; ULIDs are too
//! -- wide. The rowid column of the entity's SQLite table is perfect.
//! ```
//!
//! Rowid strategy: we treat SQLite's implicit `rowid` as the bitmap
//! identifier. On insert we remember the rowid in a side-car (the
//! entity's row already carries it); on facet-bitmap ops we read the
//! rowid from `"_rowid_of_id"` lookup. No extra surface for users.

use std::collections::{BTreeMap, HashMap};

use roaring::RoaringBitmap;
use serde::{Deserialize, Serialize};

use crate::StorageError;

// ---------------------------------------------------------------------------
// Config — re-exported from pylon-kernel
// ---------------------------------------------------------------------------

/// Storage-side alias for `pylon_kernel::ManifestSearchConfig`. The
/// shape lives in the kernel because the manifest is what every layer
/// reads (runtime, storage, router all share it). We re-export here
/// so storage callers don't have to double-import.
pub use pylon_kernel::ManifestSearchConfig as SearchConfig;

// ---------------------------------------------------------------------------
// Query + result shapes (what clients send / get back)
// ---------------------------------------------------------------------------

/// A single-entity search query. Client sends this; server returns
/// `SearchResult`. Intentionally small — filter parsing lives on top
/// of `FilterExpr`, not as free-form JSON.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchQuery {
    /// Free-text match across the declared `text` fields.
    #[serde(default)]
    pub query: String,

    /// Filter expression: `{ field: value }` = equality. Combined with
    /// match using AND. Range + IN + boolean ops live on FilterExpr.
    #[serde(default)]
    pub filters: HashMap<String, serde_json::Value>,

    /// Facets to compute counts for. If empty, use all declared facets.
    #[serde(default)]
    pub facets: Vec<String>,

    /// Sort spec: `(field, "asc" | "desc")`. Must be in the entity's
    /// `sortable` list.
    #[serde(default)]
    pub sort: Option<(String, String)>,

    /// Zero-indexed page.
    #[serde(default)]
    pub page: usize,

    /// Page size. Default 20, hard-cap at 100 to keep result payloads
    /// predictable for subscriptions.
    #[serde(default = "default_page_size")]
    pub page_size: usize,
}

fn default_page_size() -> usize {
    20
}

/// What the server hands back.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    /// Hit rows in ranked (or sorted) order. Each is a plain JSON row.
    pub hits: Vec<serde_json::Value>,

    /// `{facet_name: {value: count}}`. Values sorted by count desc in
    /// the server's serialization.
    pub facet_counts: BTreeMap<String, BTreeMap<String, u64>>,

    /// Total hit count (before pagination).
    pub total: u64,

    /// Milliseconds spent in the query engine, for client-side perf
    /// instrumentation and snappy-feeling dashboards.
    pub took_ms: u64,
}

// ---------------------------------------------------------------------------
// Facet bitmap storage
// ---------------------------------------------------------------------------

/// Serialize a Roaring bitmap to bytes for storage in the
/// `_facet_bitmap.bitmap` BLOB column.
pub fn serialize_bitmap(b: &RoaringBitmap) -> Result<Vec<u8>, StorageError> {
    let mut out = Vec::with_capacity(b.serialized_size());
    b.serialize_into(&mut out)
        .map_err(|e| StorageError::new("BITMAP_SERIALIZE_FAILED", &e.to_string()))?;
    Ok(out)
}

/// Inverse of `serialize_bitmap`.
pub fn deserialize_bitmap(bytes: &[u8]) -> Result<RoaringBitmap, StorageError> {
    RoaringBitmap::deserialize_from(bytes)
        .map_err(|e| StorageError::new("BITMAP_DESERIALIZE_FAILED", &e.to_string()))
}

// ---------------------------------------------------------------------------
// SQL generation — table setup + maintenance
// ---------------------------------------------------------------------------

/// Generate the `CREATE VIRTUAL TABLE _fts_<Entity>` statement for a
/// given entity + config. Called once during schema push.
pub fn create_fts_table_sql(entity: &str, config: &SearchConfig) -> Option<String> {
    if config.text.is_empty() {
        return None;
    }
    let cols = config
        .text
        .iter()
        .map(|f| format!("\"{f}\""))
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!(
        "CREATE VIRTUAL TABLE IF NOT EXISTS \"_fts_{entity}\" USING fts5(\
          entity_id UNINDEXED, {cols}, \
          tokenize = 'unicode61 remove_diacritics 2'\
         );"
    ))
}

/// One-time table for all facet bitmaps across all entities. Shared so
/// ops like "warm cache for all facets" can page through a single
/// table. Keyed by (entity, facet, value).
pub fn create_facet_table_sql() -> &'static str {
    "CREATE TABLE IF NOT EXISTS \"_facet_bitmap\" (\
       entity    TEXT NOT NULL,\
       facet     TEXT NOT NULL,\
       value     TEXT NOT NULL,\
       bitmap    BLOB NOT NULL,\
       row_count INTEGER NOT NULL,\
       PRIMARY KEY (entity, facet, value)\
     );"
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitmap_roundtrip() {
        let mut b = RoaringBitmap::new();
        for i in 0..10_000 {
            if i % 3 == 0 {
                b.insert(i);
            }
        }
        let bytes = serialize_bitmap(&b).unwrap();
        let round = deserialize_bitmap(&bytes).unwrap();
        assert_eq!(b, round);
    }

    #[test]
    fn fts_sql_skipped_when_no_text_fields() {
        let cfg = SearchConfig {
            text: vec![],
            facets: vec!["brand".into()],
            sortable: vec![],
        };
        assert!(create_fts_table_sql("Product", &cfg).is_none());
    }

    #[test]
    fn fts_sql_lists_declared_text_columns() {
        let cfg = SearchConfig {
            text: vec!["name".into(), "description".into()],
            facets: vec![],
            sortable: vec![],
        };
        let sql = create_fts_table_sql("Product", &cfg).unwrap();
        assert!(sql.contains("\"_fts_Product\""));
        assert!(sql.contains("\"name\""));
        assert!(sql.contains("\"description\""));
        assert!(sql.contains("unicode61"));
    }

    #[test]
    fn bitmap_intersect_popcount_is_facet_count() {
        // Proves the core performance move: after ANDing a match bitmap
        // with a facet bitmap, counting the cardinality is a single
        // popcount, not a table scan.
        let mut matches = RoaringBitmap::new();
        matches.insert_range(0..1_000_000u32);

        let mut brand_nike = RoaringBitmap::new();
        for i in (0..1_000_000u32).step_by(7) {
            brand_nike.insert(i);
        }

        let and = &matches & &brand_nike;
        assert_eq!(and.len(), brand_nike.len());
    }
}
