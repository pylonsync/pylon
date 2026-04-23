//! In-memory vector search plugin.
//!
//! Stores embeddings (Vec<f32>) per (entity, row_id) and exposes nearest-neighbour
//! search via cosine similarity. The plugin itself does *not* compute embeddings —
//! callers pass pre-computed vectors via `index()` (typically from an
//! OpenAI/Anthropic/local embedding model). For production scale move to a
//! dedicated vector store (pgvector, Qdrant, Turso libsql vector); this is the
//! "good enough for thousands of rows" implementation.
//!
//! Why not store vectors directly in SQLite? SQLite has no first-class vector
//! support and naive blob storage means re-decoding every row per query. An
//! in-memory index is far faster for small/medium datasets and survives via
//! a snapshot-on-write to a JSON file (see `persist_path`).
//!
//! Search complexity is O(n * d) per query. With 10k rows and 1024-dim vectors
//! that's ~10M float ops per query — well under 10ms on commodity hardware.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::Plugin;
use pylon_auth::AuthContext;
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct VectorRow {
    entity: String,
    row_id: String,
    vector: Vec<f32>,
    /// L2 norm of the vector, cached so we don't recompute per query.
    norm: f32,
    /// Optional payload returned with hits (e.g. preview text).
    metadata: Option<Value>,
}

#[derive(Debug, Clone)]
pub struct VectorHit {
    pub entity: String,
    pub row_id: String,
    pub score: f32,
    pub metadata: Option<Value>,
}

pub struct VectorSearchPlugin {
    rows: Mutex<HashMap<(String, String), VectorRow>>,
    persist_path: Option<PathBuf>,
}

impl VectorSearchPlugin {
    pub fn new() -> Self {
        Self {
            rows: Mutex::new(HashMap::new()),
            persist_path: None,
        }
    }

    /// Persist the index to disk on every write. Useful for restart safety.
    /// Loads existing data from `path` if it exists.
    pub fn with_persist_path(mut self, path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        if let Ok(bytes) = std::fs::read(&path) {
            if let Ok(rows) = serde_json::from_slice::<Vec<VectorRow>>(&bytes) {
                let mut map = self.rows.lock().unwrap();
                for r in rows {
                    map.insert((r.entity.clone(), r.row_id.clone()), r);
                }
            }
        }
        self.persist_path = Some(path);
        self
    }

    /// Upsert a vector for `(entity, row_id)`.
    pub fn index(&self, entity: &str, row_id: &str, vector: Vec<f32>, metadata: Option<Value>) {
        let norm = l2_norm(&vector);
        let row = VectorRow {
            entity: entity.into(),
            row_id: row_id.into(),
            vector,
            norm,
            metadata,
        };
        {
            let mut map = self.rows.lock().unwrap();
            map.insert((entity.into(), row_id.into()), row);
        }
        self.persist();
    }

    /// Find the k most similar vectors. Optionally restrict to one entity.
    pub fn search(&self, query: &[f32], k: usize, entity_filter: Option<&str>) -> Vec<VectorHit> {
        if query.is_empty() {
            return vec![];
        }
        let q_norm = l2_norm(query);
        if q_norm == 0.0 {
            return vec![];
        }

        let map = self.rows.lock().unwrap();
        let mut hits: Vec<VectorHit> = map
            .values()
            .filter(|r| {
                entity_filter.map(|e| e == r.entity).unwrap_or(true)
                    && r.vector.len() == query.len()
                    && r.norm > 0.0
            })
            .map(|r| {
                let dot: f32 = r.vector.iter().zip(query.iter()).map(|(a, b)| a * b).sum();
                let score = dot / (r.norm * q_norm);
                VectorHit {
                    entity: r.entity.clone(),
                    row_id: r.row_id.clone(),
                    score,
                    metadata: r.metadata.clone(),
                }
            })
            .collect();

        // Highest score first.
        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        hits.truncate(k);
        hits
    }

    /// Number of indexed vectors.
    pub fn len(&self) -> usize {
        self.rows.lock().unwrap().len()
    }

    /// True when the index is empty.
    pub fn is_empty(&self) -> bool {
        self.rows.lock().unwrap().is_empty()
    }

    fn persist(&self) {
        let Some(path) = self.persist_path.as_ref() else {
            return;
        };
        let map = self.rows.lock().unwrap();
        let rows: Vec<&VectorRow> = map.values().collect();
        if let Ok(bytes) = serde_json::to_vec(&rows) {
            let _ = std::fs::write(path, bytes);
        }
    }
}

impl Default for VectorSearchPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl Plugin for VectorSearchPlugin {
    fn name(&self) -> &str {
        "vector_search"
    }

    fn after_delete(&self, entity: &str, id: &str, _auth: &AuthContext) {
        let mut map = self.rows.lock().unwrap();
        map.remove(&(entity.into(), id.into()));
        drop(map);
        self.persist();
    }
}

fn l2_norm(v: &[f32]) -> f32 {
    v.iter().map(|x| x * x).sum::<f32>().sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_returns_most_similar() {
        let p = VectorSearchPlugin::new();
        p.index("Doc", "a", vec![1.0, 0.0, 0.0], None);
        p.index("Doc", "b", vec![0.0, 1.0, 0.0], None);
        p.index("Doc", "c", vec![0.9, 0.1, 0.0], None);

        let hits = p.search(&[1.0, 0.0, 0.0], 2, None);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].row_id, "a");
        assert_eq!(hits[1].row_id, "c");
        assert!(hits[0].score > hits[1].score);
    }

    #[test]
    fn entity_filter_restricts_results() {
        let p = VectorSearchPlugin::new();
        p.index("Doc", "1", vec![1.0, 0.0], None);
        p.index("Note", "2", vec![1.0, 0.0], None);
        let hits = p.search(&[1.0, 0.0], 5, Some("Note"));
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].entity, "Note");
    }

    #[test]
    fn upsert_replaces_previous_vector() {
        let p = VectorSearchPlugin::new();
        p.index("Doc", "x", vec![1.0, 0.0], None);
        p.index("Doc", "x", vec![0.0, 1.0], None);
        assert_eq!(p.len(), 1);
        let hits = p.search(&[0.0, 1.0], 1, None);
        assert!((hits[0].score - 1.0).abs() < 1e-5);
    }

    #[test]
    fn dimension_mismatch_excluded() {
        let p = VectorSearchPlugin::new();
        p.index("Doc", "a", vec![1.0, 0.0], None);
        p.index("Doc", "b", vec![1.0, 0.0, 0.0], None);
        let hits = p.search(&[1.0, 0.0], 5, None);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].row_id, "a");
    }

    #[test]
    fn delete_via_plugin_hook_removes_row() {
        let p = VectorSearchPlugin::new();
        p.index("Doc", "a", vec![1.0, 0.0], None);
        assert_eq!(p.len(), 1);
        p.after_delete("Doc", "a", &AuthContext::anonymous());
        assert!(p.is_empty());
    }

    #[test]
    fn persist_round_trip() {
        let dir = std::env::temp_dir().join(format!("pylon_vec_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("vec.json");

        let p1 = VectorSearchPlugin::new().with_persist_path(&path);
        p1.index(
            "Doc",
            "x",
            vec![0.5, 0.5],
            Some(serde_json::json!({"t": "hi"})),
        );
        drop(p1);

        let p2 = VectorSearchPlugin::new().with_persist_path(&path);
        assert_eq!(p2.len(), 1);
        let hits = p2.search(&[0.5, 0.5], 1, None);
        assert_eq!(hits[0].row_id, "x");
        assert_eq!(hits[0].metadata.as_ref().unwrap()["t"], "hi");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_query_returns_nothing() {
        let p = VectorSearchPlugin::new();
        p.index("Doc", "a", vec![1.0, 0.0], None);
        assert!(p.search(&[], 5, None).is_empty());
        assert!(p.search(&[0.0, 0.0], 5, None).is_empty());
    }
}
