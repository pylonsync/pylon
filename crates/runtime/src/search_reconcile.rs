//! Builds search indexes from rows that existed before the index did.
//!
//! The write path keeps an entity's FTS rows and facet bitmaps current for
//! every insert, update, and delete, but only from the moment the
//! `search:` block exists. A block added to a table that already has rows,
//! or a change to its text, facet, or sortable lists, leaves those rows out
//! (or indexed under the old lists). `_pylon_search_index_state` records the
//! config each entity's index was built for; on open, every searchable
//! entity whose recorded config differs is rebuilt from its rows. An
//! index built before this table existed has no record and is rebuilt
//! once.

use crate::{row_to_json, with_write_tx, Runtime, RuntimeError};

/// Rows read per query while rebuilding a SQLite index.
const PAGE: i64 = 1000;

impl Runtime {
    /// Rebuild every searchable entity's index whose recorded config is
    /// not the manifest's. Returns the entities it rebuilt.
    pub fn reconcile_search_indexes(&self) -> Result<Vec<String>, RuntimeError> {
        let mut rebuilt = Vec::new();
        for entity in &self.manifest.entities {
            let Some(config) = entity.search.as_ref().filter(|c| !c.is_empty()) else {
                continue;
            };
            let did = if let Some(pg) = self.pg_data_store() {
                pg.with_client(|client| -> Result<bool, pylon_http::DataError> {
                    pylon_storage::pg_search::reconcile_search_index(client, &entity.name, config)
                        .map_err(|e| pylon_http::DataError {
                            code: e.code,
                            message: e.message,
                        })
                })
                .map_err(|e| RuntimeError {
                    code: e.code,
                    message: format!("rebuild search index for {}: {}", entity.name, e.message),
                })?
            } else {
                self.reconcile_sqlite(&entity.name, config)?
            };
            if did {
                rebuilt.push(entity.name.clone());
            }
        }
        Ok(rebuilt)
    }

    /// Like [`Runtime::reconcile_search_indexes`], but logs instead of
    /// failing: a search index that cannot be built must not stop the app
    /// from starting. The next open tries again.
    pub(crate) fn reconcile_search_indexes_on_open(&self) {
        match self.reconcile_search_indexes() {
            Ok(rebuilt) => {
                for entity in rebuilt {
                    tracing::info!("[search] rebuilt the search index for {entity}");
                }
            }
            Err(e) => tracing::error!("[search] {}: {}", e.code, e.message),
        }
    }

    fn reconcile_sqlite(
        &self,
        entity: &str,
        config: &pylon_storage::search::SearchConfig,
    ) -> Result<bool, RuntimeError> {
        let fail = |e: String| RuntimeError {
            code: "SEARCH_REBUILD_FAILED".into(),
            message: format!("rebuild search index for {entity}: {e}"),
        };
        let fields = match self.entities.get(entity) {
            Some(ent) => ent.fields.clone(),
            None => return Ok(false),
        };
        let conn = self.lock_write_conn()?;
        let table_exists: bool = conn
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
                [entity],
                |r| r.get(0),
            )
            .map_err(|e| fail(e.to_string()))?;
        if !table_exists {
            return Ok(false);
        }
        let current = pylon_storage::search::config_fingerprint(config);
        if pylon_storage::search_maintenance::read_index_fingerprint(&conn, entity).as_deref()
            == Some(current.as_str())
        {
            return Ok(false);
        }

        let sql = format!(
            "SELECT rowid AS \"__pylon_rowid\", * FROM \"{entity}\" \
             WHERE rowid > ?1 ORDER BY rowid LIMIT ?2"
        );
        with_write_tx(self, &conn, || {
            let mut rebuild =
                pylon_storage::search_maintenance::IndexRebuild::reset(&conn, entity, config)
                    .map_err(|e| fail(e.message))?;
            let mut after: i64 = 0;
            loop {
                let page: Vec<(i64, serde_json::Value)> = {
                    let mut stmt = conn.prepare(&sql).map_err(|e| fail(e.to_string()))?;
                    let rows = stmt
                        .query_map(rusqlite::params![after, PAGE], |row| {
                            let rowid: i64 = row.get(0)?;
                            Ok((rowid, row_to_json(row, &fields)))
                        })
                        .map_err(|e| fail(e.to_string()))?;
                    rows.collect::<Result<_, _>>()
                        .map_err(|e| fail(e.to_string()))?
                };
                let Some((last, _)) = page.last() else {
                    break;
                };
                after = *last;
                for (rowid, mut row) in page {
                    if let Some(obj) = row.as_object_mut() {
                        obj.remove("__pylon_rowid");
                    }
                    self.normalize_row_on_read(entity, &mut row);
                    rebuild
                        .add(&conn, entity, rowid as u32, &row, config)
                        .map_err(|e| fail(e.message))?;
                }
            }
            rebuild
                .finish(&conn, entity, config)
                .map_err(|e| fail(e.message))?;
            Ok(true)
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::Runtime;
    use pylon_http::DataStore;
    use pylon_kernel::{AppManifest, ManifestEntity, ManifestField, ManifestSearchConfig};
    use serde_json::{json, Value};

    fn field(name: &str, ty: &str) -> ManifestField {
        ManifestField {
            name: name.into(),
            field_type: ty.into(),
            optional: true,
            unique: false,
            crdt: None,
            server_only: false,
            readonly: false,
            default: None,
            enum_values: None,
            encrypted: false,
            sync_omit: false,
            max_length: None,
        }
    }

    fn manifest(search: Option<ManifestSearchConfig>) -> AppManifest {
        AppManifest {
            manifest_version: 1,
            name: "search-reconcile".into(),
            version: "0.1.0".into(),
            entities: vec![ManifestEntity {
                name: "Property".into(),
                fields: vec![
                    field("address", "string"),
                    field("status", "string"),
                    field("market", "string"),
                    field("active", "bool"),
                ],
                crdt: false,
                search,
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn config(facets: &[&str]) -> ManifestSearchConfig {
        ManifestSearchConfig {
            text: vec!["address".into()],
            facets: facets.iter().map(|f| f.to_string()).collect(),
            sortable: vec![],
            language: None,
        }
    }

    fn search(rt: &Runtime, query: Value) -> Value {
        DataStore::search(rt, "Property", &query).expect("search")
    }

    fn seed(rt: &Runtime, n: usize) -> Vec<String> {
        (0..n)
            .map(|i| {
                rt.insert(
                    "Property",
                    &json!({
                        "address": format!("{i} Main Street"),
                        "status": if i % 3 == 0 { "sold" } else { "open" },
                        "market": if i % 2 == 0 { "north" } else { "south" },
                        "active": i % 2 == 0,
                    }),
                )
                .expect("insert")
            })
            .collect()
    }

    /// Rows written before the `search:` block existed are in the facet
    /// counts, the total, and the text index after the next open. 2500
    /// rows span three rebuild pages.
    #[test]
    fn rows_written_before_search_was_added_are_indexed_on_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db");
        let path = path.to_str().unwrap();
        let ids = seed(&Runtime::open(path, manifest(None)).unwrap(), 2500);

        let rt = Runtime::open(path, manifest(Some(config(&["status", "active"])))).unwrap();
        let out = search(&rt, json!({ "facets": ["status", "active"] }));
        assert_eq!(out["total"], 2500);
        assert_eq!(out["facetCounts"]["status"]["sold"], 834);
        assert_eq!(out["facetCounts"]["status"]["open"], 1666);
        assert_eq!(out["facetCounts"]["active"]["true"], 1250);
        let out = search(&rt, json!({ "query": "1234 main" }));
        assert_eq!(out["total"], 1);
        assert_eq!(out["hits"][0]["address"], "1234 Main Street");

        // An update after the rebuild moves the row's facet bit and text.
        // Row 0 was "sold" and "0 Main Street".
        rt.update(
            "Property",
            &ids[0],
            &json!({ "status": "open", "address": "9 Harbor Road" }),
        )
        .unwrap();
        let out = search(&rt, json!({ "facets": ["status", "active"] }));
        assert_eq!(out["facetCounts"]["status"]["sold"], 833);
        assert_eq!(out["facetCounts"]["status"]["open"], 1667);
        assert_eq!(out["facetCounts"]["active"]["true"], 1250);
        assert_eq!(search(&rt, json!({ "query": "harbor" }))["total"], 1);
        let out = search(
            &rt,
            json!({ "filters": { "status": "sold" }, "query": "main" }),
        );
        assert_eq!(out["total"], 833);

        // Rows written after the rebuild keep the index current, and the
        // next open does not rebuild again.
        seed(&rt, 3);
        drop(rt);
        let rt = Runtime::open(path, manifest(Some(config(&["status", "active"])))).unwrap();
        assert!(rt.reconcile_search_indexes().unwrap().is_empty());
        assert_eq!(search(&rt, json!({}))["total"], 2503);
    }

    /// A changed facet list rebuilds the index: the new facet counts every
    /// row, and the dropped facet's bitmaps are gone.
    #[test]
    fn a_changed_facet_list_rebuilds_the_index() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db");
        let path = path.to_str().unwrap();
        seed(
            &Runtime::open(path, manifest(Some(config(&["status"])))).unwrap(),
            10,
        );

        let rt = Runtime::open(path, manifest(Some(config(&["market"])))).unwrap();
        let out = search(&rt, json!({ "facets": ["market"] }));
        assert_eq!(out["facetCounts"]["market"]["north"], 5);
        assert_eq!(out["facetCounts"]["market"]["south"], 5);
        let old: i64 = {
            let conn = rt.lock_write_conn().unwrap();
            conn.query_row(
                "SELECT count(*) FROM \"_facet_bitmap\" WHERE entity = 'Property' AND facet = 'status'",
                [],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(old, 0);
    }

    /// A partial index from before the state table existed (bits only for
    /// rows written after the `search:` block) is rebuilt once.
    #[test]
    fn an_index_with_no_recorded_config_is_rebuilt() {
        let rt = Runtime::in_memory(manifest(Some(config(&["status"])))).unwrap();
        seed(&rt, 6);
        {
            let conn = rt.lock_write_conn().unwrap();
            conn.execute_batch(
                "DELETE FROM \"_facet_bitmap\"; DELETE FROM \"_pylon_search_index_state\"",
            )
            .unwrap();
        }
        assert_eq!(rt.reconcile_search_indexes().unwrap(), vec!["Property"]);
        let out = search(&rt, json!({ "facets": ["status"] }));
        assert_eq!(out["facetCounts"]["status"]["sold"], 2);
        assert_eq!(out["facetCounts"]["status"]["open"], 4);
    }

    /// `pageSize`, the spelling the `ctx.db.search` TS types use, sets the
    /// page size.
    #[test]
    fn page_size_accepts_the_camel_case_spelling() {
        let rt = Runtime::in_memory(manifest(Some(config(&["status"])))).unwrap();
        seed(&rt, 30);
        let out = search(&rt, json!({ "pageSize": 25 }));
        assert_eq!(out["hits"].as_array().unwrap().len(), 25);
        let out = search(&rt, json!({ "page_size": 5 }));
        assert_eq!(out["hits"].as_array().unwrap().len(), 5);
    }
}
