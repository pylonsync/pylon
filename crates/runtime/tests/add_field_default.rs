//! A field added to an existing entity with `.default(value)` fills the
//! rows that already exist with that value, on SQLite and on Postgres
//! (when `PYLON_TEST_PG_URL` is set).
//!
//! Before, the column was added with the type's zero value, so a new
//! `field.boolean().default(true)` read `false` on every existing row.

use pylon_kernel::*;
use pylon_runtime::Runtime;

fn field(
    name: &str,
    ty: &str,
    optional: bool,
    default: Option<serde_json::Value>,
) -> ManifestField {
    ManifestField {
        name: name.into(),
        field_type: ty.into(),
        optional,
        unique: false,
        crdt: None,
        server_only: false,
        readonly: false,
        default,
        enum_values: None,
        encrypted: false,
        sync_omit: false,
        ..Default::default()
    }
}

fn manifest(with_new_fields: bool) -> AppManifest {
    let mut fields = vec![field("name", "string", false, None)];
    if with_new_fields {
        fields.push(field("dailyReport", "bool", false, Some(true.into())));
        fields.push(field("digestHour", "int", true, Some(7.into())));
        fields.push(field("greeting", "string", false, Some("it's on".into())));
        // Filled per insert by the runtime; existing rows keep the zero value.
        fields.push(field("stampedAt", "datetime", false, Some("now".into())));
    }
    AppManifest {
        manifest_version: MANIFEST_VERSION,
        name: "add-field-default".into(),
        version: "0.1.0".into(),
        entities: vec![ManifestEntity {
            name: "Dealer".into(),
            fields,
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn check_existing_row(backend: &str, rt: &Runtime, id: &str) {
    let row = rt.get_by_id("Dealer", id).unwrap().expect("row");
    assert_eq!(row["dailyReport"], true, "{backend}: {row}");
    assert_eq!(row["digestHour"], 7, "{backend}: {row}");
    assert_eq!(row["greeting"], "it's on", "{backend}: {row}");
    // A new row still gets the defaults from the runtime.
    let new_id = rt
        .insert("Dealer", &serde_json::json!({ "name": "Beta" }))
        .unwrap();
    let new_row = rt.get_by_id("Dealer", &new_id).unwrap().unwrap();
    assert_eq!(new_row["dailyReport"], true, "{backend}: {new_row}");
}

#[test]
fn sqlite_backfills_existing_rows_with_the_declared_default() {
    let dir = std::env::temp_dir().join(format!(
        "pylon-add-field-default-{}-{}.db",
        std::process::id(),
        rand::random::<u32>()
    ));
    let path = dir.to_string_lossy().to_string();
    // `pylon start` / `pylon dev` apply the schema plan through the
    // storage adapter before opening the runtime.
    let apply = |m: &AppManifest| {
        let adapter = pylon_storage::sqlite::SqliteAdapter::open(&path).unwrap();
        let plan = adapter.plan_from_live(m).unwrap();
        let meta = pylon_storage::sqlite::PushMetadata {
            manifest_version: m.manifest_version,
            app_version: &m.version,
            baseline: "test",
        };
        adapter.apply_with_history(&plan, &meta).unwrap();
    };
    let id = {
        apply(&manifest(false));
        let rt = Runtime::open(&path, manifest(false)).unwrap();
        rt.insert("Dealer", &serde_json::json!({ "name": "Plaza" }))
            .unwrap()
    };
    apply(&manifest(true));
    let rt = Runtime::open(&path, manifest(true)).unwrap();
    check_existing_row("sqlite", &rt, &id);
    drop(rt);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn postgres_backfills_existing_rows_with_the_declared_default() {
    let Ok(url) = std::env::var("PYLON_TEST_PG_URL") else {
        eprintln!("skipping: set PYLON_TEST_PG_URL to enable");
        return;
    };
    let mut adapter = pylon_storage::postgres::live::LivePostgresAdapter::connect(&url)
        .expect("connect to test postgres");
    let _ = adapter.exec_raw("DROP TABLE IF EXISTS \"Dealer\" CASCADE");
    let before = manifest(false);
    let plan = adapter.plan_from_live(&before).expect("plan");
    adapter.apply_plan(&plan).expect("apply schema");
    let id = {
        let rt = Runtime::open_postgres(&url, before).unwrap();
        rt.insert("Dealer", &serde_json::json!({ "name": "Plaza" }))
            .unwrap()
    };
    let after = manifest(true);
    let plan = adapter.plan_from_live(&after).expect("plan");
    adapter.apply_plan(&plan).expect("apply the added fields");
    let rt = Runtime::open_postgres(&url, after).unwrap();
    check_existing_row("postgres", &rt, &id);
}
