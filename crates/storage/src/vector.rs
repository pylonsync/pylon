//! Vector search over `vector(dims)` fields — shared types + the
//! SQLite scan. The Postgres twin lives in `pg_vector` (feature-gated
//! like `pg_search`).
//!
//! Storage model: a `vector(dims)` field is a regular column on the
//! entity table — BLOB in SQLite, BYTEA in Postgres — holding the
//! embedding as a packed little-endian f32 array. No shadow tables and
//! no write-time maintenance: the column IS the index.
//!
//! Query model: exact k-nearest-neighbor. One SELECT streams
//! `(id, embedding)` for every row whose embedding is non-NULL (plus
//! optional equality filters), scores each against the query vector in
//! Rust, and keeps the top-k in a bounded heap. A 100k-row table at
//! 1536 dims scores in well under a second; the docs are honest that
//! million-scale ANN belongs to a dedicated store. Approximate indexes
//! (HNSW etc.) can slot in behind the same `DataStore::vector_search`
//! surface later without changing the wire shape.
//!
//! Metrics: `cosine` (default, higher = closer), `dot` (higher =
//! closer), `l2` (Euclidean distance, lower = closer). Hits always
//! come back best-first for the chosen metric.

use std::collections::HashMap;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::StorageError;

/// Matches the SDK-side guard in `field.vector(dims)`.
pub const MAX_VECTOR_DIMS: u32 = 8192;

/// Default / maximum `limit` for a vector search. The cap keeps result
/// payloads bounded — each hit carries the full row.
pub const DEFAULT_VECTOR_LIMIT: usize = 10;
pub const MAX_VECTOR_LIMIT: usize = 200;

/// Rows whose stored embedding fails to decode (wrong byte length,
/// legacy garbage) are skipped, not fatal — but the count is surfaced
/// in logs by callers so corruption doesn't hide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum VectorMetric {
    #[default]
    Cosine,
    Dot,
    L2,
}

impl VectorMetric {
    /// True when a larger score means a closer match.
    pub fn higher_is_better(&self) -> bool {
        !matches!(self, VectorMetric::L2)
    }
}

/// Wire shape of a vector search request, shared by `ctx.db.vectorSearch`
/// and `POST /api/vector-search/<entity>`.
#[derive(Debug, Clone, Deserialize)]
pub struct VectorQuery {
    /// The `vector(dims)` field to search. Required — an entity may
    /// declare several embedding fields.
    pub field: String,

    /// The query embedding. Must match the field's declared dims.
    pub vector: Vec<f32>,

    /// Max hits to return. Default 10, hard cap 200.
    #[serde(default)]
    pub limit: Option<usize>,

    #[serde(default)]
    pub metric: VectorMetric,

    /// Equality pre-filter: `{ field: value }` rows must match ALL
    /// entries. An array value means SQL `IN`. Applied in SQL before
    /// scoring, so the scan never decodes rows the filter excludes.
    #[serde(default)]
    pub filter: HashMap<String, serde_json::Value>,
}

/// One search hit. `doc` is the full row (normalized by the runtime —
/// json fields parsed, encrypted fields decrypted, vector fields
/// present as number arrays only for server-side callers).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorHit {
    pub id: String,
    pub score: f64,
    pub doc: serde_json::Value,
}

/// Wire result. camelCase to match `SearchResult` and the TS surface.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VectorSearchResult {
    pub hits: Vec<VectorHit>,
    pub took_ms: u64,
}

/// A validated vector search, ready for a backend scan.
#[derive(Debug, Clone)]
pub struct PreparedVectorQuery {
    pub field: String,
    pub vector: Vec<f32>,
    pub metric: VectorMetric,
    pub limit: usize,
    pub filter: HashMap<String, serde_json::Value>,
}

/// Parse + validate a raw JSON vector-search request against the
/// entity's manifest. Shared by the Runtime dispatch and the in-tx
/// PgTxStore path so the two can't drift on validation rules.
pub fn prepare_query(
    ent: &pylon_kernel::ManifestEntity,
    query_json: &serde_json::Value,
) -> Result<PreparedVectorQuery, StorageError> {
    let parsed: VectorQuery =
        serde_json::from_value(query_json.clone()).map_err(|e| StorageError {
            code: "INVALID_QUERY".into(),
            message: format!("vector search query body: {e}"),
        })?;
    let dims = ent
        .fields
        .iter()
        .find(|f| f.name == parsed.field)
        .and_then(|f| vector_dims(&f.field_type))
        .ok_or_else(|| StorageError {
            code: "VECTOR_FIELD_NOT_FOUND".into(),
            message: format!(
                "\"{}\" is not a vector(dims) field on entity {}",
                parsed.field, ent.name
            ),
        })?;
    if parsed.vector.len() != dims as usize {
        return Err(StorageError {
            code: "INVALID_QUERY".into(),
            message: format!(
                "query vector has {} dimensions; {}.{} declares {dims}",
                parsed.vector.len(),
                ent.name,
                parsed.field
            ),
        });
    }
    // serde turns an out-of-f32-range JSON number (1e300) into
    // f32::INFINITY — reject it before it can NaN-poison scores.
    if let Some(i) = parsed.vector.iter().position(|v| !v.is_finite()) {
        return Err(StorageError {
            code: "INVALID_QUERY".into(),
            message: format!("query vector element {i} is not a finite f32"),
        });
    }
    for key in parsed.filter.keys() {
        let field = ent.fields.iter().find(|f| &f.name == key);
        if key != "id" && field.is_none() {
            return Err(StorageError {
                code: "INVALID_QUERY".into(),
                message: format!("filter references unknown field \"{key}\" on {}", ent.name),
            });
        }
        if field
            .map(|f| vector_dims(&f.field_type).is_some())
            .unwrap_or(false)
        {
            return Err(StorageError {
                code: "INVALID_QUERY".into(),
                message: format!("cannot equality-filter on vector field \"{key}\""),
            });
        }
    }
    let limit = parsed
        .limit
        .unwrap_or(DEFAULT_VECTOR_LIMIT)
        .clamp(1, MAX_VECTOR_LIMIT);
    Ok(PreparedVectorQuery {
        field: parsed.field,
        vector: parsed.vector,
        metric: parsed.metric,
        limit,
        filter: parsed.filter,
    })
}

/// Remove vector fields from a hit row. Embeddings are multi-KB dead
/// weight in search results — callers re-fetch by id when they truly
/// need one.
pub fn strip_vector_fields(ent: &pylon_kernel::ManifestEntity, row: &mut serde_json::Value) {
    let Some(obj) = row.as_object_mut() else {
        return;
    };
    for f in &ent.fields {
        if vector_dims(&f.field_type).is_some() {
            obj.remove(&f.name);
        }
    }
}

// ---------------------------------------------------------------------------
// Packing — the storage format both backends share
// ---------------------------------------------------------------------------

/// Parse the dimension count from a `vector(N)` manifest field-type
/// string. Mirror of `pylon_schema::extract_vector_dims` (the crates
/// don't depend on each other) — keep the 1..=8192 rule in sync.
pub fn vector_dims(field_type: &str) -> Option<u32> {
    let s = field_type.strip_prefix("vector(")?;
    let s = s.strip_suffix(')')?;
    let dims: u32 = s.parse().ok()?;
    if dims == 0 || dims > MAX_VECTOR_DIMS {
        return None;
    }
    Some(dims)
}

/// Pack an embedding as little-endian f32 bytes.
pub fn pack_f32(values: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * 4);
    for v in values {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// Unpack little-endian f32 bytes. `None` when the byte length isn't a
/// multiple of 4 (not a packed vector).
pub fn unpack_f32(bytes: &[u8]) -> Option<Vec<f32>> {
    if bytes.len() % 4 != 0 {
        return None;
    }
    Some(
        bytes
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect(),
    )
}

/// Convert a JSON value (as written by app code) into an f32 vector,
/// enforcing the declared dims and rejecting non-finite elements.
pub fn json_to_f32s(value: &serde_json::Value, dims: u32) -> Result<Vec<f32>, String> {
    let arr = value
        .as_array()
        .ok_or_else(|| format!("expected a number array, got {}", type_name(value)))?;
    if arr.len() != dims as usize {
        return Err(format!("expected {dims} dimensions, got {}", arr.len()));
    }
    let mut out = Vec::with_capacity(arr.len());
    for (i, v) in arr.iter().enumerate() {
        let f = v
            .as_f64()
            .ok_or_else(|| format!("element {i} is not a number"))?;
        let f32_val = f as f32;
        // Check finiteness AFTER the f32 cast — 1e300 is a finite f64
        // but saturates to f32::INFINITY, which would poison every
        // score it touches with NaN.
        if !f32_val.is_finite() {
            return Err(format!("element {i} is not a finite f32 ({f})"));
        }
        out.push(f32_val);
    }
    Ok(out)
}

/// Render an f32 vector back to a JSON number array (read boundary).
pub fn f32s_to_json(values: &[f32]) -> serde_json::Value {
    serde_json::Value::Array(
        values
            .iter()
            .map(|v| {
                serde_json::Number::from_f64(*v as f64)
                    .map(serde_json::Value::Number)
                    .unwrap_or(serde_json::Value::Null)
            })
            .collect(),
    )
}

fn type_name(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

// ---------------------------------------------------------------------------
// Scoring + top-k
// ---------------------------------------------------------------------------

/// Score a candidate against the query under `metric`. f64 accumulation
/// keeps long dot products stable.
pub fn score(metric: VectorMetric, query: &[f32], candidate: &[f32]) -> f64 {
    match metric {
        VectorMetric::Cosine => {
            let mut dot = 0f64;
            let mut qn = 0f64;
            let mut cn = 0f64;
            for (a, b) in query.iter().zip(candidate.iter()) {
                let (a, b) = (*a as f64, *b as f64);
                dot += a * b;
                qn += a * a;
                cn += b * b;
            }
            let denom = qn.sqrt() * cn.sqrt();
            if denom == 0.0 {
                0.0
            } else {
                dot / denom
            }
        }
        VectorMetric::Dot => query
            .iter()
            .zip(candidate.iter())
            .map(|(a, b)| *a as f64 * *b as f64)
            .sum(),
        VectorMetric::L2 => query
            .iter()
            .zip(candidate.iter())
            .map(|(a, b)| {
                let d = *a as f64 - *b as f64;
                d * d
            })
            .sum::<f64>()
            .sqrt(),
    }
}

/// Bounded best-k accumulator. Push every candidate; `into_sorted`
/// returns best-first for the metric.
pub struct TopK {
    metric: VectorMetric,
    k: usize,
    // Kept sorted worst-first so eviction pops index 0. k is <= 200,
    // so insertion-sort behavior is fine and avoids Ord-wrapper noise.
    entries: Vec<(String, f64)>,
}

impl TopK {
    pub fn new(metric: VectorMetric, k: usize) -> Self {
        Self {
            metric,
            k,
            entries: Vec::with_capacity(k + 1),
        }
    }

    fn better(&self, a: f64, b: f64) -> bool {
        if self.metric.higher_is_better() {
            a > b
        } else {
            a < b
        }
    }

    pub fn push(&mut self, id: String, score: f64) {
        // NaN is unordered: it would lodge as an unevictable "worst"
        // entry and block every later candidate. Scan sites also skip
        // NaN — this is the backstop.
        if self.k == 0 || score.is_nan() {
            return;
        }
        if self.entries.len() >= self.k {
            // entries[0] is the current worst kept score.
            if !self.better(score, self.entries[0].1) {
                return;
            }
            self.entries.remove(0);
        }
        let idx = self
            .entries
            .partition_point(|(_, s)| self.better(score, *s));
        // partition_point over worst-first order: entries the new score
        // beats come first; insert after them keeps worst-first.
        self.entries.insert(idx, (id, score));
    }

    /// Best-first.
    pub fn into_sorted(mut self) -> Vec<(String, f64)> {
        self.entries.reverse();
        self.entries
    }
}

// ---------------------------------------------------------------------------
// SQLite scan
// ---------------------------------------------------------------------------

fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn filter_param(value: &serde_json::Value) -> Box<dyn rusqlite::types::ToSql> {
    match value {
        serde_json::Value::Null => Box::new(rusqlite::types::Null),
        serde_json::Value::Bool(b) => Box::new(*b as i64),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Box::new(i)
            } else {
                Box::new(n.as_f64().unwrap_or(0.0))
            }
        }
        serde_json::Value::String(s) => Box::new(s.clone()),
        other => Box::new(other.to_string()),
    }
}

/// Build `(where_sql, params)` for the equality pre-filter. Caller has
/// already validated every filter key against the manifest — this only
/// assembles SQL from trusted identifiers. Shared shape with the PG
/// side (which builds `$n` placeholders instead).
fn build_filter_sql(
    field: &str,
    filter: &HashMap<String, serde_json::Value>,
) -> (String, Vec<Box<dyn rusqlite::types::ToSql>>) {
    let mut clauses = vec![format!("{} IS NOT NULL", quote_ident(field))];
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let mut idx = 1usize;
    // Deterministic clause order for prepared-statement cache hits.
    let mut keys: Vec<&String> = filter.keys().collect();
    keys.sort();
    for key in keys {
        match &filter[key] {
            serde_json::Value::Array(items) => {
                if items.is_empty() {
                    // `IN ()` is a syntax error; an empty IN matches nothing.
                    clauses.push("0 = 1".to_string());
                    continue;
                }
                let marks: Vec<String> = items
                    .iter()
                    .map(|v| {
                        params.push(filter_param(v));
                        let m = format!("?{idx}");
                        idx += 1;
                        m
                    })
                    .collect();
                clauses.push(format!("{} IN ({})", quote_ident(key), marks.join(", ")));
            }
            serde_json::Value::Null => {
                clauses.push(format!("{} IS NULL", quote_ident(key)));
            }
            v => {
                params.push(filter_param(v));
                clauses.push(format!("{} = ?{idx}", quote_ident(key)));
                idx += 1;
            }
        }
    }
    (clauses.join(" AND "), params)
}

/// Exact k-NN scan over the entity table. Returns `(id, score)` pairs,
/// best-first, at most `limit`. `dims` is the field's declared
/// dimension count — the query vector must already match it (callers
/// validate and return a typed error; this asserts as a backstop).
pub fn scan_topk(
    conn: &Connection,
    entity: &str,
    field: &str,
    query_vec: &[f32],
    metric: VectorMetric,
    limit: usize,
    filter: &HashMap<String, serde_json::Value>,
) -> Result<Vec<(String, f64)>, StorageError> {
    let (where_sql, params) = build_filter_sql(field, filter);
    let sql = format!(
        "SELECT \"id\", {} FROM {} WHERE {}",
        quote_ident(field),
        quote_ident(entity),
        where_sql
    );
    let mut stmt = conn.prepare_cached(&sql).map_err(|e| StorageError {
        code: "VECTOR_SCAN_FAILED".into(),
        message: format!("prepare vector scan on {entity}.{field}: {e}"),
    })?;
    let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();

    let mut topk = TopK::new(metric, limit);
    let mut skipped = 0usize;
    let mut rows = stmt
        .query(param_refs.as_slice())
        .map_err(|e| StorageError {
            code: "VECTOR_SCAN_FAILED".into(),
            message: format!("vector scan on {entity}.{field}: {e}"),
        })?;
    while let Some(row) = rows.next().map_err(|e| StorageError {
        code: "VECTOR_SCAN_FAILED".into(),
        message: format!("vector scan row on {entity}.{field}: {e}"),
    })? {
        let id: String = match row.get(0) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let blob: Vec<u8> = match row.get(1) {
            Ok(v) => v,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        let Some(candidate) = unpack_f32(&blob) else {
            skipped += 1;
            continue;
        };
        if candidate.len() != query_vec.len() {
            skipped += 1;
            continue;
        }
        let s = score(metric, query_vec, &candidate);
        if s.is_nan() {
            // Legacy/hostile rows with non-finite components — never
            // let NaN into the ranking.
            skipped += 1;
            continue;
        }
        topk.push(id, s);
    }
    if skipped > 0 {
        tracing::warn!(
            "[vector] scan on {entity}.{field} skipped {skipped} rows with undecodable, wrong-dimension, or non-finite embeddings"
        );
    }
    Ok(topk.into_sorted())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_roundtrip() {
        let v = vec![0.25f32, -1.5, 3.0, f32::MIN_POSITIVE];
        assert_eq!(unpack_f32(&pack_f32(&v)), Some(v));
        assert_eq!(unpack_f32(&[1, 2, 3]), None);
        assert_eq!(unpack_f32(&[]), Some(vec![]));
    }

    #[test]
    fn json_conversion_validates() {
        let ok = serde_json::json!([1.0, 2.0, 3.0]);
        assert_eq!(json_to_f32s(&ok, 3).unwrap(), vec![1.0, 2.0, 3.0]);
        assert!(json_to_f32s(&ok, 4).unwrap_err().contains("4 dimensions"));
        assert!(json_to_f32s(&serde_json::json!("nope"), 3).is_err());
        assert!(json_to_f32s(&serde_json::json!([1.0, "x", 3.0]), 3).is_err());
        // 1e300 is a finite f64 that saturates to f32::INFINITY — must
        // be rejected AFTER the cast, or one row NaN-poisons every scan.
        assert!(json_to_f32s(&serde_json::json!([1e300, 0.0]), 2)
            .unwrap_err()
            .contains("finite"));
    }

    #[test]
    fn topk_ignores_nan_scores() {
        let mut t = TopK::new(VectorMetric::Cosine, 2);
        t.push("nan".into(), f64::NAN);
        t.push("a".into(), 0.3);
        t.push("nan2".into(), f64::NAN);
        t.push("b".into(), 0.8);
        let out = t.into_sorted();
        assert_eq!(
            out.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(),
            vec!["b", "a"],
            "NaN must never occupy a slot or block later candidates"
        );
    }

    #[test]
    fn prepare_query_rejects_non_finite_query_vector() {
        let ent: pylon_kernel::ManifestEntity = serde_json::from_value(serde_json::json!({
            "name": "Doc",
            "fields": [
                {"name": "embedding", "type": "vector(2)", "optional": true, "unique": false}
            ],
            "indexes": []
        }))
        .unwrap();
        // serde parses 1e300 into f32::INFINITY on the Vec<f32> field.
        let err = prepare_query(
            &ent,
            &serde_json::json!({"field": "embedding", "vector": [1e300, 0.0]}),
        )
        .unwrap_err();
        assert_eq!(err.code, "INVALID_QUERY");
        assert!(err.message.contains("finite"), "{}", err.message);
    }

    #[test]
    fn metrics() {
        let a = [1.0f32, 0.0];
        let b = [1.0f32, 0.0];
        let c = [0.0f32, 1.0];
        assert!((score(VectorMetric::Cosine, &a, &b) - 1.0).abs() < 1e-9);
        assert!(score(VectorMetric::Cosine, &a, &c).abs() < 1e-9);
        assert!((score(VectorMetric::Dot, &a, &b) - 1.0).abs() < 1e-9);
        assert!((score(VectorMetric::L2, &a, &c) - std::f64::consts::SQRT_2).abs() < 1e-9);
        // Zero-norm candidate: cosine defined as 0, not NaN.
        assert_eq!(score(VectorMetric::Cosine, &a, &[0.0, 0.0]), 0.0);
    }

    #[test]
    fn topk_orders_best_first() {
        let mut t = TopK::new(VectorMetric::Cosine, 2);
        t.push("a".into(), 0.1);
        t.push("b".into(), 0.9);
        t.push("c".into(), 0.5);
        let out = t.into_sorted();
        assert_eq!(
            out.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(),
            vec!["b", "c"]
        );

        let mut t = TopK::new(VectorMetric::L2, 2);
        t.push("a".into(), 3.0);
        t.push("b".into(), 1.0);
        t.push("c".into(), 2.0);
        let out = t.into_sorted();
        assert_eq!(
            out.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(),
            vec!["b", "c"]
        );
    }

    #[test]
    fn sqlite_scan_end_to_end() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE Doc (id TEXT PRIMARY KEY, kind TEXT, embedding BLOB);")
            .unwrap();
        let rows: Vec<(&str, &str, Vec<f32>)> = vec![
            ("d1", "a", vec![1.0, 0.0]),
            ("d2", "a", vec![0.9, 0.1]),
            ("d3", "b", vec![0.0, 1.0]),
        ];
        for (id, kind, vec_) in &rows {
            conn.execute(
                "INSERT INTO Doc (id, kind, embedding) VALUES (?1, ?2, ?3)",
                rusqlite::params![id, kind, pack_f32(vec_)],
            )
            .unwrap();
        }
        // NULL embedding + wrong-length blob are skipped, not fatal.
        conn.execute("INSERT INTO Doc (id, kind) VALUES ('d4', 'a')", [])
            .unwrap();
        conn.execute(
            "INSERT INTO Doc (id, kind, embedding) VALUES ('d5', 'a', X'0102')",
            [],
        )
        .unwrap();

        let hits = scan_topk(
            &conn,
            "Doc",
            "embedding",
            &[1.0, 0.0],
            VectorMetric::Cosine,
            2,
            &HashMap::new(),
        )
        .unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].0, "d1");
        assert_eq!(hits[1].0, "d2");

        // Equality filter excludes kind=a.
        let mut filter = HashMap::new();
        filter.insert("kind".to_string(), serde_json::json!("b"));
        let hits = scan_topk(
            &conn,
            "Doc",
            "embedding",
            &[1.0, 0.0],
            VectorMetric::Cosine,
            10,
            &filter,
        )
        .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, "d3");

        // IN filter.
        let mut filter = HashMap::new();
        filter.insert("kind".to_string(), serde_json::json!(["a", "b"]));
        let hits = scan_topk(
            &conn,
            "Doc",
            "embedding",
            &[1.0, 0.0],
            VectorMetric::L2,
            10,
            &filter,
        )
        .unwrap();
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0].0, "d1");
    }
}
