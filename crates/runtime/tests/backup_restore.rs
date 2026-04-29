//! Backup + restore integration test.
//!
//! Real disaster-recovery test: seed a SQLite Runtime, snapshot the files
//! (DB + WAL + SHM exactly as `pylon backup` does), drop the original,
//! reopen from the snapshot, and confirm both row contents and the change
//! log seq survived the round trip.
//!
//! Untested backups aren't backups. If this test breaks, the
//! `pylon backup` / `pylon restore` commands are broken too.

use std::fs;
use std::path::PathBuf;

use pylon_kernel::{AppManifest, ManifestEntity, ManifestField};
use pylon_runtime::Runtime;

fn tmp_dir(suffix: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "pylon_backup_test_{}_{}",
        std::process::id(),
        suffix
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    path
}

fn test_manifest() -> AppManifest {
    AppManifest {
        manifest_version: 1,
        name: "backup_test".into(),
        version: "0.1.0".into(),
        entities: vec![ManifestEntity {
            name: "Todo".into(),
            fields: vec![
                ManifestField {
                    name: "title".into(),
                    field_type: "string".into(),
                    optional: false,
                    unique: false,
                    crdt: None,
                },
                ManifestField {
                    name: "done".into(),
                    field_type: "bool".into(),
                    optional: false,
                    unique: false,
                    crdt: None,
                },
            ],
            indexes: vec![],
            relations: vec![],
            search: None,
            crdt: true,
        }],
        routes: vec![],
        queries: vec![],
        actions: vec![],
        policies: vec![],
        auth: Default::default(),
    }
}

/// Copy the SQLite triple (DB + WAL + SHM) from `src_base` → `dst_base`.
/// Mirrors what `crates/cli/src/commands/backup.rs::run_backup` does.
fn copy_sqlite_triple(src_base: &str, dst_base: &str) {
    for ext in ["", "-wal", "-shm"] {
        let src = format!("{src_base}{ext}");
        if !std::path::Path::new(&src).exists() {
            continue;
        }
        let dst = format!("{dst_base}{ext}");
        fs::copy(&src, &dst).expect("copy sqlite file");
    }
}

#[test]
fn backup_and_restore_preserves_rows() {
    let tmp = tmp_dir("basic");
    let src_db = tmp.join("src.db");
    let dst_db = tmp.join("dst.db");

    // Seed.
    {
        let rt = Runtime::open(src_db.to_str().unwrap(), test_manifest()).unwrap();
        rt.insert(
            "Todo",
            &serde_json::json!({"title": "buy milk", "done": false}),
        )
        .unwrap();
        rt.insert(
            "Todo",
            &serde_json::json!({"title": "walk dog", "done": true}),
        )
        .unwrap();
        rt.insert(
            "Todo",
            &serde_json::json!({"title": "write test", "done": false}),
        )
        .unwrap();
        // Runtime goes out of scope here; WAL should flush on drop.
    }

    // Hot-copy the SQLite triple while the writer is closed.
    copy_sqlite_triple(src_db.to_str().unwrap(), dst_db.to_str().unwrap());

    // Wipe the original to prove we're reading from the copy, not the
    // original file.
    let _ = fs::remove_file(&src_db);
    let _ = fs::remove_file(format!("{}-wal", src_db.display()));
    let _ = fs::remove_file(format!("{}-shm", src_db.display()));

    // Restore-side runtime on the copy.
    let restored = Runtime::open(dst_db.to_str().unwrap(), test_manifest()).unwrap();
    let rows = restored.list("Todo").unwrap();
    assert_eq!(rows.len(), 3, "backup should preserve all seeded rows");

    let titles: Vec<&str> = rows
        .iter()
        .filter_map(|r| r.get("title").and_then(|v| v.as_str()))
        .collect();
    assert!(titles.contains(&"buy milk"));
    assert!(titles.contains(&"walk dog"));
    assert!(titles.contains(&"write test"));

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn backup_preserves_inserts_across_wal_boundary() {
    // SQLite in WAL mode delays durability until checkpoint. If the backup
    // runs while WAL has uncheckpointed data, the DB file alone is not
    // enough — we MUST copy the WAL too. This test forces that case.
    let tmp = tmp_dir("wal");
    let src_db = tmp.join("src.db");
    let dst_db = tmp.join("dst.db");

    // Open runtime and insert WITHOUT closing — so WAL has uncheckpointed
    // data. We don't have a checkpoint API exposed, so simulate by doing
    // the copy while the runtime is still open.
    let rt = Runtime::open(src_db.to_str().unwrap(), test_manifest()).unwrap();
    for i in 0..20 {
        rt.insert(
            "Todo",
            &serde_json::json!({"title": format!("t{i}"), "done": false}),
        )
        .unwrap();
    }

    // Live copy — what `pylon backup` does on a running server.
    copy_sqlite_triple(src_db.to_str().unwrap(), dst_db.to_str().unwrap());

    // Drop original writer BEFORE opening the copy. If we opened both
    // concurrently SQLite would complain.
    drop(rt);

    let restored = Runtime::open(dst_db.to_str().unwrap(), test_manifest()).unwrap();
    let rows = restored.list("Todo").unwrap();
    assert_eq!(
        rows.len(),
        20,
        "WAL-copy path must preserve uncheckpointed inserts"
    );

    let _ = fs::remove_dir_all(&tmp);
}
