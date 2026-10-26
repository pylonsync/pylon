//! The row filter a store runs for a sync scope keeps every row the scope
//! keeps, on SQLite and on Postgres, including rows whose stored values the
//! row check reads in an unusual way: nulls, a `NaN` or infinite float, an
//! infinite or sub-millisecond time, and (on SQLite) values stored with a
//! storage class other than the column's.
//!
//! For each scope and caller the test reads the whole table, applies
//! `check_sync_scope` to every row, and compares with the rows
//! `list_after_filtered` and `list_last_filtered` return. A filtered read
//! may return more rows, never fewer, and must return fewer than the whole
//! table for the scopes here.
//!
//! Postgres runs when `PYLON_TEST_PG_URL` is set. Shares the database with
//! the other Postgres tests: run with --test-threads=1.

use std::collections::BTreeSet;
use std::sync::Arc;

use pylon_auth::AuthContext;
use pylon_http::DataStore;
use pylon_kernel::*;
use pylon_policy::{PolicyEngine, PolicyResult};
use pylon_runtime::Runtime;
use serde_json::{json, Value};

/// (entity, scope). Every entity has the same fields.
const SCOPES: &[(&str, &str)] = &[
    ("ScopeOwner", "data.ownerId == auth.userId"),
    (
        "ScopeNotOwnerWon",
        "data.ownerId != auth.userId && data.stage == \"won\"",
    ),
    (
        "ScopeTeamRecent",
        "exists(Member where userId == auth.userId and teamId == data.teamId) \
         && data.seenAt >= ago(\"30d\")",
    ),
    (
        "ScopeFewVisits",
        "!(data.visits > 3) || data.archived == true",
    ),
    ("ScopeLowScore", "data.score < 2.5 || data.score == null"),
    ("ScopeNotLowScore", "!(data.score <= 2.5)"),
    (
        "ScopeAfterNewYear",
        "!(data.seenAt <= \"2026-01-01T00:00:00Z\")",
    ),
    (
        "ScopeBeforeNewYear",
        "data.seenAt <= \"2026-01-01T00:00:00.001Z\" && data.teamId != null",
    ),
    (
        "ScopeNotArchived",
        "data.archived != true && data.teamId != null",
    ),
    (
        "ScopeNoStage",
        "data.stage == null || !(data.visits >= 2)",
    ),
    ("ScopeSeenNull", "data.seenAt == null || data.visits == 0"),
    (
        "ScopeLead",
        "(exists(Member where userId == auth.userId and role == \"lead\") \
          && (data.seenAt >= ago(\"30d\") || data.stage == \"won\")) \
         || (exists(Member where userId == auth.userId and teamId == data.teamId) \
          && !(data.ownerId == null) && !exists(Member where userId == data.ownerId and role == \"gone\"))",
    ),
];

const USERS: &[&str] = &["u1", "u2", "lead", "stranger"];

fn field(name: &str, field_type: &str) -> ManifestField {
    ManifestField {
        name: name.into(),
        field_type: field_type.into(),
        optional: true,
        ..Default::default()
    }
}

fn fields() -> Vec<ManifestField> {
    vec![
        field("ownerId", "id(User)"),
        field("teamId", "string"),
        field("stage", "string"),
        field("seenAt", "datetime"),
        field("visits", "int"),
        field("score", "float"),
        field("archived", "bool"),
    ]
}

fn manifest() -> AppManifest {
    let mut entities = vec![
        ManifestEntity {
            name: "User".into(),
            fields: vec![field("email", "string")],
            ..Default::default()
        },
        ManifestEntity {
            name: "Member".into(),
            fields: vec![
                field("userId", "id(User)"),
                field("teamId", "string"),
                field("role", "string"),
            ],
            sync: false,
            ..Default::default()
        },
    ];
    for (name, scope) in SCOPES {
        entities.push(ManifestEntity {
            name: (*name).into(),
            fields: fields(),
            crdt: false,
            sync: true,
            sync_scope: Some((*scope).into()),
            ..Default::default()
        });
    }
    AppManifest {
        manifest_version: MANIFEST_VERSION,
        name: "scope-filter-parity".into(),
        version: "0.1.0".into(),
        entities,
        ..Default::default()
    }
}

/// Ordinary rows: every combination of a few values, nulls included.
fn rows() -> Vec<Value> {
    let now = chrono::Utc::now();
    let at = |days: i64| {
        (now - chrono::Duration::days(days)).to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    };
    let times = [
        None,
        Some(at(1)),
        Some(at(29)),
        Some(at(31)),
        Some("2025-12-31T23:59:59.999Z".to_string()),
        Some("2026-01-01T00:00:00.000Z".to_string()),
        Some("2026-01-01T00:00:00.001Z".to_string()),
    ];
    let mut rows = Vec::new();
    let mut n = 0;
    for owner in [None, Some("u1"), Some("u2"), Some("lead"), Some("gone")] {
        for team in [None, Some("t1"), Some("t2")] {
            for stage in [None, Some("won"), Some("new")] {
                for (i, seen) in times.iter().enumerate() {
                    for visits in [None, Some(0), Some(2), Some(4)] {
                        let score = [None, Some(1.0), Some(2.5), Some(7.25)][(n + i) % 4];
                        let archived = [None, Some(true), Some(false)][n % 3];
                        n += 1;
                        rows.push(json!({
                            "id": format!("{n:040x}"),
                            "ownerId": owner,
                            "teamId": team,
                            "stage": stage,
                            "seenAt": seen,
                            "visits": visits,
                            "score": score,
                            "archived": archived,
                        }));
                    }
                }
            }
        }
    }
    rows
}

fn members() -> Vec<Value> {
    [
        ("u1", "t1", "rep"),
        ("u2", "t2", "rep"),
        ("lead", "t1", "lead"),
        ("gone", "t1", "gone"),
    ]
    .iter()
    .map(|(user, team, role)| json!({"userId": user, "teamId": team, "role": role}))
    .collect()
}

fn ids(rows: &[Value]) -> BTreeSet<String> {
    rows.iter()
        .map(|r| r["id"].as_str().unwrap().to_string())
        .collect()
}

/// Every row of `entity` through the store's filtered reads, in pages, both
/// ways.
fn read_filtered(
    store: &dyn DataStore,
    entity: &str,
    filter: &row_filter::RowFilter,
) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut forward = BTreeSet::new();
    let mut after: Option<String> = None;
    loop {
        let page = store
            .list_after_filtered(entity, after.as_deref(), 37, filter)
            .unwrap();
        after = page.last().map(|r| r["id"].as_str().unwrap().to_string());
        forward.extend(ids(&page));
        if page.len() < 37 {
            break;
        }
    }
    let mut backward = BTreeSet::new();
    let mut before: Option<String> = None;
    loop {
        let page = store
            .list_last_filtered(entity, before.as_deref(), 37, filter)
            .unwrap()
            .expect("the store scans backwards");
        before = page.last().map(|r| r["id"].as_str().unwrap().to_string());
        backward.extend(ids(&page));
        if page.len() < 37 {
            break;
        }
    }
    (forward, backward)
}

/// `unpruned`: `Entity` or `Entity/user` where the store can run no part of
/// the scope that drops a row.
fn check(rt: Arc<Runtime>, odd_ids: &[&str], unpruned: &[&str]) {
    let manifest = manifest();
    let engine = PolicyEngine::from_manifest(&manifest);
    let store: Arc<dyn DataStore> = rt.clone();
    engine.set_resolver(Arc::new(pylon_runtime::datastore::DataStoreResolver::new(
        Arc::clone(&store),
    )));
    for (entity, _) in SCOPES {
        let all = store.list_after(entity, None, 100_000).unwrap();
        // The odd rows are in the table and read back.
        let all_ids = ids(&all);
        for id in odd_ids {
            assert!(all_ids.contains(*id), "{entity}: {id} is missing");
        }
        for user in USERS {
            let auth = AuthContext::authenticated((*user).into());
            let kept: BTreeSet<String> = all
                .iter()
                .filter(|row| {
                    matches!(
                        engine.check_sync_scope(entity, &auth, Some(row)),
                        PolicyResult::Allowed
                    )
                })
                .map(|r| r["id"].as_str().unwrap().to_string())
                .collect();
            let filter = engine.sync_scope_filter(entity, &auth).expect("scoped");
            let (forward, backward) = read_filtered(store.as_ref(), entity, &filter);
            let missing: Vec<&String> = kept.difference(&forward).collect();
            assert!(
                missing.is_empty(),
                "{entity} as {user}: the filtered read drops rows the scope keeps: {missing:?}\n{filter:#?}"
            );
            assert_eq!(
                forward, backward,
                "{entity} as {user}: the two directions differ"
            );
            assert!(
                unpruned.contains(entity)
                    || unpruned.contains(&format!("{entity}/{user}").as_str())
                    || forward.len() < all.len(),
                "{entity} as {user}: the filter kept every row\n{filter:#?}"
            );
        }
    }
}

fn seed(rt: &Runtime) {
    for member in members() {
        rt.insert("Member", &member).unwrap();
    }
    for (entity, _) in SCOPES {
        for row in rows() {
            rt.insert(entity, &row).unwrap();
        }
    }
}

#[test]
fn sqlite_filtered_reads_keep_every_row_the_scope_keeps() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("parity.db");
    let rt = Arc::new(Runtime::open(path.to_str().unwrap(), manifest()).unwrap());
    seed(&rt);

    // Values stored with another storage class than the column's, as an
    // older build or a hand edit could leave them.
    let raw = rusqlite::Connection::open(&path).unwrap();
    for (entity, _) in SCOPES {
        let table = format!("\"{entity}\"");
        raw.execute_batch(&format!(
            "INSERT INTO {table} (id, \"ownerId\", \"teamId\", stage, visits, score, archived) VALUES
               ('x1', X'7531', 't1', X'776f6e', 5, 9e999, 2),
               ('x2', 'u1', 't1', 'won', 'abc', -9e999, 1.5),
               ('x3', 'u2', 't2', 'new', 'inf', 'abc', 'yes'),
               ('x4', 'lead', 't1', 'won', 1.5, 2.5, 0),
               ('x5', NULL, X'7431', NULL, X'00', X'00', X'00');"
        ))
        .unwrap();
    }
    drop(raw);
    // SQLite leaves time comparisons to the row check.
    check(
        rt,
        &["x1", "x2", "x3", "x4", "x5"],
        &["ScopeAfterNewYear", "ScopeLead/lead"],
    );
}

#[test]
fn postgres_filtered_reads_keep_every_row_the_scope_keeps() {
    let Ok(url) = std::env::var("PYLON_TEST_PG_URL") else {
        return;
    };
    let mut admin = postgres::Client::connect(&url, postgres::NoTls).unwrap();
    let mut drop_sql = String::from("DROP TABLE IF EXISTS \"User\", \"Member\"");
    for (entity, _) in SCOPES {
        drop_sql.push_str(&format!(", \"{entity}\""));
    }
    admin.batch_execute(&format!("{drop_sql} CASCADE")).unwrap();
    let mut adapter = pylon_storage::postgres::live::LivePostgresAdapter::connect(&url).unwrap();
    let plan = adapter.plan_from_live(&manifest()).unwrap();
    adapter.apply_plan(&plan).unwrap();
    let rt = Arc::new(Runtime::open_postgres(&url, manifest()).unwrap());
    seed(&rt);

    // Values the decoder reads as null (NaN, infinities) and a time finer
    // than the millisecond the row check reads.
    for (entity, _) in SCOPES {
        admin
            .batch_execute(&format!(
                "INSERT INTO \"{entity}\" (id, \"ownerId\", \"teamId\", stage, \"seenAt\", visits, score, archived) VALUES
                   ('x1', 'u1', 't1', 'won', 'infinity', 5, 'NaN', true),
                   ('x2', 'u2', 't2', 'new', '-infinity', 1, 'Infinity', false),
                   ('x3', 'lead', 't1', NULL, '2026-01-01T00:00:00.0004Z', 3, '-Infinity', NULL),
                   ('x4', 'u1', 't1', 'won', '2025-12-31T23:59:59.9996Z', 0, 2.5, true),
                   ('x5', 'u2', 't1', 'new', '2026-01-01T00:00:00.0016Z', 2, 1, false);"
            ))
            .unwrap();
    }
    check(rt, &["x1", "x2", "x3", "x4", "x5"], &[]);
}
