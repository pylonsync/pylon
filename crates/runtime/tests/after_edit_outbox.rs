//! The `afterEdit` outbox: a committed write notes its row in the same
//! transaction, a failed write notes nothing, and `dispatch_due` turns due
//! rows into one job per row and window. SQLite always; Postgres when
//! `PYLON_TEST_PG_URL` is set.

use std::sync::Arc;

use pylon_kernel::*;
use pylon_runtime::after_edit::dispatch_due;
use pylon_runtime::jobs::JobQueue;
use pylon_runtime::Runtime;

fn manifest() -> AppManifest {
    AppManifest {
        manifest_version: MANIFEST_VERSION,
        name: "after-edit-outbox".into(),
        version: "0.1.0".into(),
        entities: vec![ManifestEntity {
            name: "Note".into(),
            fields: vec![ManifestField {
                name: "title".into(),
                field_type: "string".into(),
                optional: false,
                unique: true,
                ..Default::default()
            }],
            crdt: true,
            sync: true,
            after_edit: Some(ManifestAfterEdit {
                function: "indexNote".into(),
                delay_secs: 1,
            }),
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn outbox_rows(rt: &Runtime) -> Vec<(String, String)> {
    if let Some(pg) = rt.pg_data_store_pub() {
        return pg
            .with_client(
                |c| -> Result<Vec<(String, String)>, pylon_http::DataError> {
                    Ok(c.query(
                        "SELECT entity, row_id FROM _pylon_after_edit ORDER BY row_id",
                        &[],
                    )
                    .unwrap()
                    .iter()
                    .map(|r| (r.get(0), r.get(1)))
                    .collect())
                },
            )
            .unwrap();
    }
    let conn = rt.lock_conn_pub().unwrap();
    let mut stmt = conn
        .prepare("SELECT entity, row_id FROM _pylon_after_edit ORDER BY row_id")
        .unwrap();
    let rows = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<Vec<(String, String)>, _>>()
        .unwrap();
    rows
}

fn exercise(rt: &Runtime, jobs: &JobQueue, queued: &dyn Fn() -> Vec<String>) {
    let a = rt
        .insert("Note", &serde_json::json!({ "title": "a" }))
        .unwrap();
    rt.update("Note", &a, &serde_json::json!({ "title": "a2" }))
        .unwrap();
    rt.update("Note", &a, &serde_json::json!({ "title": "a3" }))
        .unwrap();
    // A failed write (the title is unique) notes nothing.
    let b = rt
        .insert("Note", &serde_json::json!({ "title": "b" }))
        .unwrap();
    assert!(rt
        .insert("Note", &serde_json::json!({ "title": "b" }))
        .is_err());
    let rows = outbox_rows(rt);
    // One row per (row, window): a window may turn between the writes.
    assert!(rows.iter().all(|(e, _)| e == "Note"));
    let for_a = rows.iter().filter(|(_, r)| *r == a).count();
    let for_b = rows.iter().filter(|(_, r)| *r == b).count();
    assert!((1..=2).contains(&for_a), "{rows:?}");
    assert_eq!(for_b, 1, "{rows:?}");
    assert_eq!(
        rows.len(),
        for_a + for_b,
        "the failed insert noted nothing: {rows:?}"
    );

    // Not due yet: nothing moves.
    assert_eq!(dispatch_due(rt, jobs).unwrap(), 0);
    std::thread::sleep(std::time::Duration::from_millis(2100));
    assert_eq!(dispatch_due(rt, jobs).unwrap(), rows.len());
    assert!(outbox_rows(rt).is_empty());
    let ids = queued();
    assert_eq!(ids.len(), rows.len(), "{ids:?}");
    assert!(
        ids.iter().all(|id| id.starts_with("afterEdit:Note:")),
        "{ids:?}"
    );

    // A live edit (an editor's CRDT push) is an edit too.
    let snapshot = pylon_http::DataStore::crdt_snapshot(rt, "Note", &a)
        .unwrap()
        .unwrap();
    let editor = pylon_crdt::loro::LoroDoc::new();
    pylon_crdt::apply_update(&editor, &snapshot).unwrap();
    let from = editor.oplog_vv();
    pylon_crdt::root_map(&editor)
        .insert("title", "typed")
        .unwrap();
    editor.commit();
    let update = editor
        .export(pylon_crdt::loro::ExportMode::updates(&from))
        .unwrap();
    pylon_http::DataStore::crdt_apply_update(rt, "Note", &a, &update, &|_| Ok(())).unwrap();
    assert_eq!(rt.get_by_id("Note", &a).unwrap().unwrap()["title"], "typed");
    assert_eq!(outbox_rows(rt), vec![("Note".to_string(), a.clone())]);

    // A delete is an edit too.
    rt.delete("Note", &b).unwrap();
    assert_eq!(outbox_rows(rt).len(), 2);
}

#[test]
fn sqlite_outbox_follows_committed_writes() {
    let rt = Runtime::in_memory(manifest()).unwrap();
    let jobs = Arc::new(JobQueue::new(100));
    let queued = || {
        jobs.list_jobs(Some("pending"), None, 100)
            .into_iter()
            .map(|j| j.id)
            .collect::<Vec<_>>()
    };
    exercise(&rt, &jobs, &queued);
}

#[test]
fn postgres_outbox_follows_committed_writes() {
    let Ok(url) = std::env::var("PYLON_TEST_PG_URL") else {
        eprintln!("skipping: PYLON_TEST_PG_URL not set");
        return;
    };
    let manifest = manifest();
    let mut adapter = pylon_storage::postgres::live::LivePostgresAdapter::connect(&url).unwrap();
    for t in ["\"Note\"", "_pylon_after_edit", "_pylon_jobs"] {
        let _ = adapter.exec_raw(&format!("DROP TABLE IF EXISTS {t} CASCADE"));
    }
    let plan = adapter.plan_from_live(&manifest).unwrap();
    adapter.apply_plan(&plan).unwrap();
    let rt = Runtime::open_postgres(&url, manifest).unwrap();
    let store = Arc::new(
        pylon_runtime::pg_job_store::PgJobStore::open(
            rt.pg_data_store_pub().unwrap().shared_pool(),
            "after-edit-test".into(),
        )
        .unwrap(),
    );
    let jobs = Arc::new(JobQueue::new(100));
    jobs.attach_pg_store(Arc::clone(&store));
    let pg_url = url.clone();
    let queued = move || {
        let mut c = postgres::Client::connect(&pg_url, postgres::NoTls).unwrap();
        c.query(
            "SELECT id FROM _pylon_jobs WHERE id LIKE 'afterEdit:%' ORDER BY id",
            &[],
        )
        .unwrap()
        .iter()
        .map(|r| r.get::<_, String>(0))
        .collect::<Vec<_>>()
    };
    exercise(&rt, &jobs, &queued);
}
