//! Postgres twin of the SQLite vector scan in `vector.rs`. Same exact
//! k-NN model: stream `(id, embedding BYTEA)` for candidate rows,
//! decode the packed little-endian f32 array, score in Rust, keep the
//! top-k. Generic over [`PgConn`] so it runs identically on a
//! standalone client or inside an in-flight transaction.
//!
//! Filter equality uses the same `column::text = $n` cast trick as
//! `pg_search` so the comparison works uniformly across column types.

#![cfg(feature = "postgres-live")]

use std::collections::HashMap;

use postgres::types::ToSql;

use crate::pg_exec::PgConn;
use crate::postgres::quote_ident_pub as quote_ident;
use crate::vector::{decode_candidate, Scorer, TopK, VectorMetric};
use crate::StorageError;

/// Fetch vector hit documents in one query without embedding columns.
pub fn fetch_documents<C: PgConn>(
    conn: &mut C,
    entity: &pylon_kernel::ManifestEntity,
    ids: &[String],
) -> Result<Vec<serde_json::Value>, StorageError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let columns = crate::vector::document_columns(entity)
        .into_iter()
        .map(quote_ident)
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT {columns} FROM {} WHERE \"id\" = ANY($1)",
        quote_ident(&entity.name)
    );
    let rows = conn.query(&sql, &[&ids]).map_err(|e| StorageError {
        code: "VECTOR_FETCH_FAILED".into(),
        message: e.to_string(),
    })?;
    Ok(rows.iter().map(crate::postgres::row_to_json_pub).collect())
}

/// Exact k-NN scan; returns `(id, score)` best-first, at most `limit`.
/// Filter keys are already validated against the manifest by the
/// caller — identifiers here are trusted.
pub fn scan_topk<C: PgConn>(
    conn: &mut C,
    entity: &str,
    field: &str,
    query_vec: &[f32],
    metric: VectorMetric,
    limit: usize,
    filter: &HashMap<String, serde_json::Value>,
) -> Result<Vec<(String, f64)>, StorageError> {
    let mut clauses = vec![format!("{} IS NOT NULL", quote_ident(field))];
    let mut params: Vec<Box<dyn ToSql + Sync>> = Vec::new();

    let mut keys: Vec<&String> = filter.keys().collect();
    keys.sort();
    for key in keys {
        match &filter[key] {
            serde_json::Value::Array(items) => {
                if items.is_empty() {
                    clauses.push("FALSE".to_string());
                    continue;
                }
                let marks: Vec<String> = items
                    .iter()
                    .map(|v| {
                        params.push(Box::new(stringify(v)));
                        format!("${}", params.len())
                    })
                    .collect();
                clauses.push(format!(
                    "{}::text IN ({})",
                    quote_ident(key),
                    marks.join(", ")
                ));
            }
            serde_json::Value::Null => {
                clauses.push(format!("{} IS NULL", quote_ident(key)));
            }
            v => {
                params.push(Box::new(stringify(v)));
                clauses.push(format!("{}::text = ${}", quote_ident(key), params.len()));
            }
        }
    }

    let sql = format!(
        "SELECT \"id\", {} FROM {} WHERE {}",
        quote_ident(field),
        quote_ident(entity),
        clauses.join(" AND ")
    );
    let param_refs: Vec<&(dyn ToSql + Sync)> = params.iter().map(|b| b.as_ref() as _).collect();
    let mut topk = TopK::new(metric, limit);
    let mut candidate = Vec::with_capacity(query_vec.len());
    let scorer = Scorer::new(metric, query_vec);
    let mut skipped = 0usize;
    conn.query_each(&sql, &param_refs, &mut |row| {
        let id: &str = match row.try_get(0) {
            Ok(v) => v,
            Err(_) => return,
        };
        let blob: &[u8] = match row.try_get(1) {
            Ok(v) => v,
            Err(_) => {
                skipped += 1;
                return;
            }
        };
        if !decode_candidate(blob, query_vec.len(), &mut candidate) {
            skipped += 1;
            return;
        }
        let s = scorer.score(&candidate);
        if s.is_nan() {
            skipped += 1;
            return;
        }
        if topk.accepts(s) {
            topk.push(id.to_owned(), s);
        }
    })
    .map_err(|e| StorageError {
        code: "VECTOR_SCAN_FAILED".into(),
        message: format!("vector scan on {entity}.{field}: {e}"),
    })?;
    if skipped > 0 {
        tracing::warn!(
            "[vector] pg scan on {entity}.{field} skipped {skipped} rows with undecodable, wrong-dimension, or non-finite embeddings"
        );
    }
    Ok(topk.into_sorted())
}

/// Text form used for the `::text = $n` comparison. Matches Postgres's
/// own text rendering for the column types Pylon filters on (TEXT,
/// INTEGER, BOOLEAN).
fn stringify(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Bool(b) => {
            // Postgres renders BOOLEAN::text as "true"/"false".
            b.to_string()
        }
        serde_json::Value::Number(n) => {
            // Match Postgres's ::text rendering: a whole-number float
            // renders without the fraction ("2", not "2.0"). serde's
            // to_string keeps the ".0" and would never match.
            if let Some(f) = n.as_f64() {
                if f.fract() == 0.0 && f.abs() < 9e15 {
                    return format!("{}", f as i64);
                }
            }
            n.to_string()
        }
        other => other.to_string(),
    }
}
