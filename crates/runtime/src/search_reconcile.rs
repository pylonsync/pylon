//! Builds search indexes from rows that existed before the index did.
//!
//! The write path keeps an entity's FTS rows and facet bitmaps current for
//! every insert, update, and delete, but only from the moment the
//! `search:` block exists. A block added to a table that already has rows,
//! or a change to its text, facet, or sortable lists, leaves those rows out
//! (or indexed under the old lists). `_pylon_search_index_state` records the
//! config each entity's index was built for and, during a build, how far
//! it got. On open, every searchable entity whose index is stale or part
//! built is built from its rows, a page at a time, each page in its own
//! transaction (see `pylon_storage::search_maintenance` and
//! `pylon_storage::pg_search` for why writes between pages stay correct).
//!
//! Up to `INLINE_ROWS` rows are indexed before `Runtime::open` returns. A
//! larger table is indexed by a background thread that the server starts
//! with `Runtime::spawn_search_index_builds` once it is serving, so a big
//! table cannot hold the server past a platform's start-up health check.
//! Until that build finishes, search on the entity fails with
//! `SEARCH_INDEX_BUILDING` (HTTP 503). A build that stops (a restart, a
//! crash) continues from its cursor on the next open.

use std::sync::Arc;
use std::time::{Duration, Instant};

use pylon_storage::search::{IndexBuild, SearchConfig};

use crate::{row_to_json, with_write_tx, Runtime, RuntimeError};

/// Rows indexed per page (and per transaction).
const PAGE: i64 = 1000;

/// A stale index with up to this many rows to index is built before open
/// returns. Measured on a laptop (release build): 200,000 rows take 1 s on
/// SQLite and 6 s on local Postgres; 1,000,000 take 7.7 s on SQLite. A
/// larger one is built in the background, well inside Cloud's 60 s health
/// window for a new machine.
const INLINE_ROWS: i64 = 200_000;

#[cfg(not(test))]
fn inline_rows() -> i64 {
    INLINE_ROWS
}

#[cfg(test)]
thread_local! {
    /// Tests on this thread treat larger indexes as background builds.
    pub(crate) static TEST_INLINE_ROWS: std::cell::Cell<i64> = const { std::cell::Cell::new(INLINE_ROWS) };
}

#[cfg(test)]
fn inline_rows() -> i64 {
    TEST_INLINE_ROWS.with(|n| n.get())
}

/// A build logs its progress every this many rows.
const PROGRESS_EVERY: usize = 100_000;

/// Wait before a background build retries after an error.
const RETRY_AFTER: Duration = Duration::from_secs(30);

impl Runtime {
    /// Build every searchable entity's index that is stale or part built,
    /// all before returning. Returns the entities it built.
    pub fn reconcile_search_indexes(&self) -> Result<Vec<String>, RuntimeError> {
        let mut built = Vec::new();
        for (entity, config) in self.searchable_entities() {
            if self.prepare_index_build(entity, config)? == IndexBuild::Current {
                continue;
            }
            self.run_index_build(entity, config)?;
            self.search_builds_lock().remove(entity);
            built.push(entity.to_string());
        }
        Ok(built)
    }

    /// The open-time pass: start or continue each stale index's build,
    /// finish the small ones now, and leave the large ones to
    /// `spawn_search_index_builds`. Logs instead of failing: a search
    /// index that cannot be built must not stop the app from starting.
    pub(crate) fn reconcile_search_indexes_on_open(&self) {
        for (entity, config) in self.searchable_entities() {
            let result = (|| -> Result<(), RuntimeError> {
                if self.prepare_index_build(entity, config)? == IndexBuild::Current {
                    return Ok(());
                }
                let remaining = self.index_build_remaining(entity)?;
                if remaining <= inline_rows() {
                    return self.run_index_build(entity, config);
                }
                tracing::info!(
                    "[search] {entity}: {remaining} rows to index; building in the background. \
                     Search on {entity} answers 503 until the build finishes."
                );
                self.search_builds_lock().insert(entity.to_string());
                Ok(())
            })();
            if let Err(e) = result {
                // Search on a half-built index would answer wrong counts.
                self.search_builds_lock().insert(entity.to_string());
                tracing::error!("[search] {}: {}", e.code, e.message);
            }
        }
    }

    /// Start a thread that finishes the builds `Runtime::open` left for
    /// the background. A build that fails retries every 30 s. Call once,
    /// after the server is accepting requests; without it, search on those
    /// entities keeps answering `SEARCH_INDEX_BUILDING`.
    pub fn spawn_search_index_builds(self: &Arc<Self>) {
        if self.search_builds_lock().is_empty() {
            return;
        }
        let rt = Arc::clone(self);
        let spawned = std::thread::Builder::new()
            .name("pylon-search-index".into())
            .spawn(move || loop {
                let Some(entity) = rt.search_builds_lock().iter().next().cloned() else {
                    return;
                };
                let config = rt
                    .searchable_entities()
                    .find(|(name, _)| *name == entity)
                    .map(|(_, config)| config.clone());
                let result = match &config {
                    Some(config) => rt
                        .prepare_index_build(&entity, config)
                        .and_then(|_| rt.run_index_build(&entity, config)),
                    None => Ok(()),
                };
                match result {
                    Ok(()) => {
                        rt.search_builds_lock().remove(&entity);
                    }
                    Err(e) => {
                        tracing::error!(
                            "[search] {}: {} (retrying in {} s)",
                            e.code,
                            e.message,
                            RETRY_AFTER.as_secs()
                        );
                        std::thread::sleep(RETRY_AFTER);
                    }
                }
            });
        if let Err(e) = spawned {
            tracing::error!("[search] could not start the index build thread: {e}");
        }
    }

    /// `SEARCH_INDEX_BUILDING` when the entity's index is being built in
    /// the background.
    pub(crate) fn search_index_building(&self, entity: &str) -> Option<pylon_http::DataError> {
        if !self.search_builds_lock().contains(entity) {
            return None;
        }
        Some(pylon_http::DataError {
            code: "SEARCH_INDEX_BUILDING".into(),
            message: format!(
                "The search index for {entity} is being built from its rows. Try again later."
            ),
        })
    }

    fn search_builds_lock(&self) -> std::sync::MutexGuard<'_, std::collections::HashSet<String>> {
        // The set holds names only; a panic while it was held left it valid.
        self.search_builds
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn searchable_entities(&self) -> impl Iterator<Item = (&str, &SearchConfig)> {
        self.manifest.entities.iter().filter_map(|e| {
            e.search
                .as_ref()
                .filter(|c| !c.is_empty())
                .map(|c| (e.name.as_str(), c))
        })
    }

    /// Read the entity's index state and start a build when the index was
    /// built for another config. Returns the state as read.
    fn prepare_index_build(
        &self,
        entity: &str,
        config: &SearchConfig,
    ) -> Result<IndexBuild, RuntimeError> {
        use pylon_storage::{pg_search, search_maintenance};
        let state = if let Some(pg) = self.pg_data_store() {
            pg_call(pg, entity, |client| {
                let state = pg_search::index_build_state(client, entity, config)?;
                if state == IndexBuild::Start {
                    pg_search::start_index_build(client, entity, config)?;
                }
                Ok(state)
            })?
        } else {
            let conn = self.lock_write_conn()?;
            with_write_tx(self, &conn, || {
                let state = search_maintenance::index_build_state(&conn, entity, config)
                    .map_err(|e| build_error(entity, e))?;
                if state == IndexBuild::Start {
                    search_maintenance::start_index_build(&conn, entity, config)
                        .map_err(|e| build_error(entity, e))?;
                }
                Ok(state)
            })?
        };
        Ok(state)
    }

    /// Rows the entity's build has still to index.
    fn index_build_remaining(&self, entity: &str) -> Result<i64, RuntimeError> {
        if let Some(pg) = self.pg_data_store() {
            return pg_call(pg, entity, |client| {
                pylon_storage::pg_search::index_build_remaining(client, entity)
            });
        }
        let conn = self.lock_write_conn()?;
        let Some(after) = pylon_storage::search_maintenance::index_build_cursor(&conn, entity)
            .map_err(|e| build_error(entity, e))?
        else {
            return Ok(0);
        };
        conn.query_row(
            &format!("SELECT count(*) FROM \"{entity}\" WHERE rowid > ?1"),
            [after],
            |r| r.get(0),
        )
        .map_err(|e| build_error(entity, sqlite_error(e)))
    }

    /// Index the entity's rows from the build's cursor to the end, a page
    /// per transaction, logging progress.
    fn run_index_build(&self, entity: &str, config: &SearchConfig) -> Result<(), RuntimeError> {
        let total = self.index_build_remaining(entity)?;
        tracing::info!("[search] building the search index for {entity}: {total} rows to index");
        let started = Instant::now();
        let mut done = 0usize;
        while let Some(n) = self.index_build_page(entity, config)? {
            let before = done;
            done += n;
            if done / PROGRESS_EVERY > before / PROGRESS_EVERY {
                tracing::info!(
                    "[search] {entity}: indexed {done} of {total} rows ({} ms)",
                    started.elapsed().as_millis()
                );
            }
        }
        tracing::info!(
            "[search] built the search index for {entity}: {done} rows in {} ms",
            started.elapsed().as_millis()
        );
        Ok(())
    }

    /// Index one page. Returns the rows indexed, or `None` once the build
    /// is complete.
    pub(crate) fn index_build_page(
        &self,
        entity: &str,
        config: &SearchConfig,
    ) -> Result<Option<usize>, RuntimeError> {
        use pylon_storage::search_maintenance;
        if let Some(pg) = self.pg_data_store() {
            return pg_call(pg, entity, |client| {
                pylon_storage::pg_search::index_build_page(client, entity, config, PAGE)
            });
        }
        let fields = match self.entities.get(entity) {
            Some(ent) => ent.fields.clone(),
            None => return Ok(None),
        };
        let sql = format!(
            "SELECT rowid AS \"__pylon_rowid\", * FROM \"{entity}\" \
             WHERE rowid > ?1 ORDER BY rowid LIMIT ?2"
        );
        let conn = self.lock_write_conn()?;
        with_write_tx(self, &conn, || {
            let fail = |e: pylon_storage::StorageError| build_error(entity, e);
            let Some(after) =
                search_maintenance::index_build_cursor(&conn, entity).map_err(fail)?
            else {
                return Ok(None);
            };
            let mut rows: Vec<(u32, serde_json::Value)> = {
                let mut stmt = conn.prepare(&sql).map_err(|e| fail(sqlite_error(e)))?;
                let mapped = stmt
                    .query_map(rusqlite::params![after, PAGE], |row| {
                        let rowid: i64 = row.get(0)?;
                        Ok((rowid as u32, row_to_json(row, &fields)))
                    })
                    .map_err(|e| fail(sqlite_error(e)))?;
                mapped
                    .collect::<Result<_, _>>()
                    .map_err(|e| fail(sqlite_error(e)))?
            };
            for (_, row) in rows.iter_mut() {
                if let Some(obj) = row.as_object_mut() {
                    obj.remove("__pylon_rowid");
                }
                self.normalize_row_on_read(entity, row);
            }
            // A short page is the last one.
            let next = if (rows.len() as i64) < PAGE {
                None
            } else {
                rows.last().map(|(rowid, _)| i64::from(*rowid))
            };
            search_maintenance::index_build_page(&conn, entity, config, &rows, next)
                .map_err(fail)?;
            Ok(Some(rows.len()))
        })
    }
}

fn sqlite_error(e: rusqlite::Error) -> pylon_storage::StorageError {
    pylon_storage::StorageError::new("SEARCH_BUILD_FAILED", &e.to_string())
}

fn build_error(entity: &str, e: pylon_storage::StorageError) -> RuntimeError {
    RuntimeError {
        code: e.code,
        message: format!("build the search index for {entity}: {}", e.message),
    }
}

/// Run `op` on a pooled Postgres client.
fn pg_call<T>(
    pg: &pylon_storage::pg_datastore::PostgresDataStore,
    entity: &str,
    op: impl FnOnce(&mut postgres::Client) -> Result<T, pylon_storage::StorageError>,
) -> Result<T, RuntimeError> {
    pg.with_client(|client| -> Result<T, pylon_http::DataError> {
        op(client).map_err(|e| pylon_http::DataError {
            code: e.code,
            message: e.message,
        })
    })
    .map_err(|e| RuntimeError {
        code: e.code,
        message: format!("build the search index for {entity}: {}", e.message),
    })
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
                    field("price", "float"),
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

    fn wait_for_builds(rt: &Runtime) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while rt.search_builds_lock().len() > 0 {
            assert!(std::time::Instant::now() < deadline, "build did not finish");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    /// An index too large to build during open is built in the background;
    /// until it is done, search answers SEARCH_INDEX_BUILDING instead of
    /// partial counts.
    #[test]
    fn a_large_index_is_built_in_the_background() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db");
        let path = path.to_str().unwrap();
        seed(&Runtime::open(path, manifest(None)).unwrap(), 2500);

        super::TEST_INLINE_ROWS.with(|n| n.set(100));
        let rt =
            std::sync::Arc::new(Runtime::open(path, manifest(Some(config(&["status"])))).unwrap());
        let err = DataStore::search(&*rt, "Property", &json!({})).unwrap_err();
        assert_eq!(err.code, "SEARCH_INDEX_BUILDING");
        let err = rt
            .query_filtered("Property", &json!({ "$search": "main" }))
            .unwrap_err();
        assert_eq!(err.code, "SEARCH_INDEX_BUILDING");

        rt.spawn_search_index_builds();
        wait_for_builds(&rt);
        let out = search(&rt, json!({ "facets": ["status"] }));
        assert_eq!(out["total"], 2500);
        assert_eq!(out["facetCounts"]["status"]["sold"], 834);
        assert_eq!(search(&rt, json!({ "query": "2499 main" }))["total"], 1);
    }

    /// Writes between build pages, on rows the build has passed and on
    /// rows it has not reached, end up in the index once; and a build that
    /// stops part way continues from its cursor on the next open.
    #[test]
    fn writes_during_a_build_and_a_restart_leave_a_correct_index() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db");
        let path = path.to_str().unwrap();
        let ids = seed(&Runtime::open(path, manifest(None)).unwrap(), 2500);

        super::TEST_INLINE_ROWS.with(|n| n.set(100));
        let cfg = config(&["status"]);
        let rt = Runtime::open(path, manifest(Some(cfg.clone()))).unwrap();
        // Index the first 1000 rows, then write.
        assert_eq!(rt.index_build_page("Property", &cfg).unwrap(), Some(1000));
        // Row 0 (passed) and row 2499 (not reached), both "sold".
        rt.update("Property", &ids[0], &json!({ "status": "lead" }))
            .unwrap();
        rt.update(
            "Property",
            &ids[2499],
            &json!({ "status": "lead", "address": "1 Quay" }),
        )
        .unwrap();
        rt.delete("Property", &ids[2498]).unwrap(); // "open"
        rt.insert(
            "Property",
            &json!({ "address": "2 Quay", "status": "lead" }),
        )
        .unwrap();
        drop(rt);

        // The next open continues from row 1000 (and builds inline: the
        // 1500 rows left are under the default limit).
        super::TEST_INLINE_ROWS.with(|n| n.set(super::INLINE_ROWS));
        let rt = Runtime::open(path, manifest(Some(cfg))).unwrap();
        assert!(rt.search_builds_lock().is_empty());
        let out = search(&rt, json!({ "facets": ["status"] }));
        assert_eq!(out["total"], 2500);
        assert_eq!(out["facetCounts"]["status"]["sold"], 832);
        assert_eq!(out["facetCounts"]["status"]["lead"], 3);
        assert_eq!(out["facetCounts"]["status"]["open"], 1665);
        assert_eq!(search(&rt, json!({ "query": "quay" }))["total"], 2);
        assert_eq!(search(&rt, json!({ "query": "2499" }))["total"], 0);
    }

    /// Postgres: a large index builds in the background (search answers
    /// SEARCH_INDEX_BUILDING meanwhile), and updates between pages, to rows
    /// the build has passed and rows it has not reached, are in the index
    /// when it finishes. Runs when PYLON_TEST_PG_URL is set.
    #[test]
    fn pg_a_large_index_builds_in_the_background_with_writes_between_pages() {
        let Ok(url) = std::env::var("PYLON_TEST_PG_URL") else {
            return;
        };
        let cfg = config(&["status"]);
        let mut adapter =
            pylon_storage::postgres::live::LivePostgresAdapter::connect(&url).unwrap();
        let _ = adapter.exec_raw("DROP TABLE IF EXISTS \"_fts_Property\" CASCADE");
        let _ = adapter.exec_raw("DROP TABLE IF EXISTS \"Property\" CASCADE");
        let _ =
            adapter.exec_raw("DELETE FROM \"_pylon_search_index_state\" WHERE entity = 'Property'");
        let plan = adapter.plan_from_live(&manifest(None)).unwrap();
        adapter.apply_plan(&plan).unwrap();
        let _ = adapter.exec_raw(
            "INSERT INTO \"Property\" (id, address, status) \
             SELECT 'p' || lpad(g::text, 5, '0'), g || ' Main Street', \
                    CASE WHEN g % 3 = 0 THEN 'sold' ELSE 'open' END \
             FROM generate_series(0, 2499) g",
        );
        let plan = adapter
            .plan_from_live(&manifest(Some(cfg.clone())))
            .unwrap();
        adapter.apply_plan(&plan).unwrap();

        super::TEST_INLINE_ROWS.with(|n| n.set(100));
        let rt =
            std::sync::Arc::new(Runtime::open_postgres(&url, manifest(Some(cfg.clone()))).unwrap());
        let err = DataStore::search(&*rt, "Property", &json!({ "query": "main" })).unwrap_err();
        assert_eq!(err.code, "SEARCH_INDEX_BUILDING");

        assert_eq!(rt.index_build_page("Property", &cfg).unwrap(), Some(1000));
        rt.update("Property", "p00000", &json!({ "address": "1 Quay" }))
            .unwrap();
        rt.update("Property", "p02499", &json!({ "address": "2 Quay" }))
            .unwrap();
        rt.spawn_search_index_builds();
        wait_for_builds(&rt);

        assert_eq!(search(&rt, json!({ "query": "main" }))["total"], 2498);
        assert_eq!(search(&rt, json!({ "query": "quay" }))["total"], 2);
        let out = search(&rt, json!({ "query": "main", "facets": ["status"] }));
        assert_eq!(out["facetCounts"]["status"]["sold"], 832);
        let _ = adapter.exec_raw("DROP TABLE IF EXISTS \"Property\" CASCADE");
    }

    /// A whole float has one facet key whether it was written as `3` or
    /// `3.0`, and whether the bit came from a write or a rebuild.
    #[test]
    fn a_whole_float_facet_has_one_key_across_writes_and_rebuilds() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db");
        let path = path.to_str().unwrap();
        let rt = Runtime::open(path, manifest(None)).unwrap();
        rt.insert("Property", &json!({ "address": "a", "price": 3 }))
            .unwrap();
        rt.insert("Property", &json!({ "address": "b", "price": 3.0 }))
            .unwrap();
        drop(rt);

        let rt = Runtime::open(path, manifest(Some(config(&["price"])))).unwrap();
        rt.insert("Property", &json!({ "address": "c", "price": 3 }))
            .unwrap();
        rt.insert("Property", &json!({ "address": "d", "price": 3.0 }))
            .unwrap();
        let out = search(&rt, json!({ "facets": ["price"] }));
        assert_eq!(out["facetCounts"]["price"], json!({ "3": 4 }));
        for filter in [json!(3), json!(3.0)] {
            let out = search(&rt, json!({ "filters": { "price": filter } }));
            assert_eq!(out["total"], 4);
        }
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
