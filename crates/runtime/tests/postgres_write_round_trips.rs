//! How many statements a function's writes cost on Postgres, counted on
//! the wire. Each statement waits on the database, so on a remote database
//! the count, not the work, sets how long an import takes.
//!
//! An action calls a mutation (`ctx.runMutation`) that writes rows of an
//! entity with `audit: true`, a `search:` block with a text field and
//! facets, and CRDT on (the default): the shape of an importer. The test
//! writes a few rows and more rows, one at a time and with
//! `insertMany` / `updateMany`, and checks the cost of each extra row, that
//! no statement is parsed again per row, and that both ways store the same
//! rows, docs, audit events, search text, and change events.
//!
//! Skipped unless `PYLON_TEST_PG_URL` is set and `bun` is on PATH. Shares
//! the database with the other Postgres tests: run with --test-threads=1.

#[path = "support/pg_wire.rs"]
mod pg_wire;

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pg_wire::{Counts, WireCounter};
use pylon_http::DataStore;
use pylon_kernel::*;
use pylon_runtime::Runtime;

fn bun_available() -> bool {
    std::process::Command::new("bun")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn functions_dir() -> PathBuf {
    let fns = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "pylon-pg-round-trips-{}-{}/functions",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&fns).unwrap();
    let define = repo_root()
        .join("packages/functions/src/define.ts")
        .to_string_lossy()
        .replace('\\', "/");
    let files = [
        (
            "seed",
            r#"export default mutation({
  auth: "public",
  handler: async (ctx, args) => {
    const rows = Array.from({ length: args.count }, (_, i) => ({
      name: `${args.prefix} ${i}`, stage: "new", visits: 0, teamId: "t1",
    }));
    if (args.many) return { ids: await ctx.db.insertMany("Lead", rows) };
    const ids = [];
    for (const row of rows) ids.push(await ctx.db.insert("Lead", row));
    return { ids };
  },
});"#,
        ),
        (
            "touch",
            r#"export default mutation({
  internal: true,
  handler: async (ctx, args) => {
    const rows = await ctx.db.query("Lead", { id: { $in: args.ids } });
    const patches = rows.map((row) => ({
      id: row.id,
      data: { stage: args.stage, visits: row.visits + 1, name: `${row.name}!` },
    }));
    if (args.many) {
      const updated = await ctx.db.updateMany("Lead", patches);
      return { updated: updated.filter(Boolean).length };
    }
    let updated = 0;
    for (const { id, data } of patches) if (await ctx.db.update("Lead", id, data)) updated++;
    return { updated };
  },
});"#,
        ),
        (
            "importPage",
            r#"export default action({
  auth: "public",
  handler: async (ctx, args) => ctx.runMutation("touch", args),
});"#,
        ),
    ];
    for (name, body) in files {
        std::fs::write(
            fns.join(format!("{name}.ts")),
            format!("import {{ action, mutation }} from \"{define}\";\n{body}\n"),
        )
        .unwrap();
    }
    fns
}

fn field(name: &str, field_type: &str) -> ManifestField {
    ManifestField {
        name: name.into(),
        field_type: field_type.into(),
        optional: false,
        ..Default::default()
    }
}

fn manifest() -> AppManifest {
    AppManifest {
        manifest_version: MANIFEST_VERSION,
        name: "pg-round-trips".into(),
        version: "0.1.0".into(),
        entities: vec![ManifestEntity {
            name: "Lead".into(),
            fields: vec![
                field("name", "string"),
                field("stage", "string"),
                field("visits", "int"),
                field("teamId", "string"),
            ],
            crdt: true,
            sync: true,
            audit: true,
            search: Some(ManifestSearchConfig {
                text: vec!["name".into()],
                facets: vec!["stage".into(), "teamId".into()],
                sortable: vec!["visits".into()],
                ..Default::default()
            }),
            ..Default::default()
        }],
        ..Default::default()
    }
}

/// The test database's URL, pointed at the counting proxy.
fn proxied(url: &str) -> (WireCounter, String) {
    let config: postgres::Config = url.parse().expect("PYLON_TEST_PG_URL parses");
    let host = match &config.get_hosts()[0] {
        postgres::config::Host::Tcp(host) => host.clone(),
        #[cfg(unix)]
        other => panic!("the test needs a TCP host, not {other:?}"),
    };
    let port = config.get_ports().first().copied().unwrap_or(5432);
    let counter = WireCounter::start(format!("{host}:{port}"));
    let quote = |s: &str| format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"));
    let mut proxied = format!("host=127.0.0.1 port={} sslmode=disable", counter.port);
    if let Some(user) = config.get_user() {
        proxied.push_str(&format!(" user={}", quote(user)));
    }
    if let Some(password) = config.get_password() {
        proxied.push_str(&format!(
            " password={}",
            quote(&String::from_utf8_lossy(password))
        ));
    }
    if let Some(db) = config.get_dbname() {
        proxied.push_str(&format!(" dbname={}", quote(db)));
    }
    (counter, proxied)
}

fn free_port() -> u16 {
    for _ in 0..200 {
        let base = 20_000 + rand::random::<u16>() % 8_000;
        if (0..4).all(|o| pylon_runtime::listen::port_is_free(base + o)) {
            return base;
        }
    }
    panic!("no free port block");
}

fn call(port: u16, function: &str, body: &str) -> serde_json::Value {
    let req = format!(
        "POST /api/fn/{function} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(60)))
        .unwrap();
    stream.write_all(req.as_bytes()).unwrap();
    let mut raw = String::new();
    stream.read_to_string(&mut raw).ok();
    let status = raw.split_whitespace().nth(1).unwrap_or("");
    let body = raw.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("");
    assert_eq!(status, "200", "{function}: {body}");
    serde_json::from_str(body).unwrap()
}

/// SQL that marks a statement as part of a `Lead` write: the entity's
/// table and search text, the CRDT sidecar and lock, the change-log seq,
/// and the audit log.
const WRITE_STATEMENTS: [&str; 6] = [
    "\"Lead\"",
    "\"_fts_Lead\"",
    "_pylon_crdt_",
    "pg_advisory_xact_lock",
    "pylon_change_seq",
    "_pylon_audit_events",
];

/// Runs of the statements a `Lead` write costs.
fn write_runs(counts: &Counts) -> usize {
    WRITE_STATEMENTS
        .iter()
        .map(|needle| counts.run_matching(needle))
        .sum()
}

/// Parses of the statements a `Lead` write costs.
fn write_parses(counts: &Counts) -> usize {
    WRITE_STATEMENTS
        .iter()
        .map(|needle| counts.parsed_matching(needle))
        .sum()
}

#[test]
fn a_write_costs_a_fixed_number_of_statements_per_row() {
    let Ok(url) = std::env::var("PYLON_TEST_PG_URL") else {
        return;
    };
    if !bun_available() {
        eprintln!("skipped: bun is not on PATH");
        return;
    }
    let mut admin = postgres::Client::connect(&url, postgres::NoTls).unwrap();
    admin
        .batch_execute(
            "DROP TABLE IF EXISTS \"Lead\" CASCADE;
             DROP TABLE IF EXISTS \"_fts_Lead\" CASCADE;
             DROP TABLE IF EXISTS _pylon_crdt_snapshots CASCADE;
             DROP TABLE IF EXISTS _pylon_crdt_updates CASCADE;
             DROP TABLE IF EXISTS _pylon_audit_events CASCADE;
             DROP TABLE IF EXISTS pylon_change_log CASCADE;
             DROP SEQUENCE IF EXISTS pylon_change_seq;",
        )
        .unwrap();
    let mut adapter = pylon_storage::postgres::live::LivePostgresAdapter::connect(&url).unwrap();
    let plan = adapter.plan_from_live(&manifest()).unwrap();
    adapter.apply_plan(&plan).unwrap();

    let (wire, proxied_url) = proxied(&url);
    let fns = functions_dir();
    // SAFETY: set before the server thread starts; this binary has one test.
    unsafe {
        std::env::set_var("PYLON_DEV_MODE", "1");
        std::env::set_var("PYLON_FUNCTIONS_DIR", &fns);
        std::env::set_var(
            "PYLON_FUNCTIONS_RUNTIME",
            repo_root().join("packages/functions/src/runtime.ts"),
        );
        std::env::set_var("PYLON_FN_POOL_SIZE", "1");
    }
    let rt = Arc::new(Runtime::open_postgres(&proxied_url, manifest()).unwrap());
    let port = free_port();
    let server_rt = Arc::clone(&rt);
    std::thread::spawn(move || {
        let _ = pylon_runtime::server::start(server_rt, port);
    });
    let deadline = Instant::now() + Duration::from_secs(60);
    while !(TcpStream::connect(("127.0.0.1", port)).is_ok() && functions_ready(port)) {
        assert!(Instant::now() < deadline, "server never came up");
        std::thread::sleep(Duration::from_millis(200));
    }

    let seed = |count: usize, many: bool| -> (Vec<String>, Counts) {
        wire.reset();
        let prefix = if many { "many" } else { "one" };
        let body = format!(r#"{{"count":{count},"many":{many},"prefix":"{prefix}"}}"#);
        let result = call(port, "seed", &body);
        let ids = result["ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|id| id.as_str().unwrap().to_string())
            .collect();
        (ids, wire.counts())
    };
    // Updates through action -> runMutation, as an importer runs them.
    let update = |ids: &[String], many: bool| -> Counts {
        wire.reset();
        let body = serde_json::json!({ "ids": ids, "stage": "contacted", "many": many });
        let result = call(port, "importPage", &body.to_string());
        assert_eq!(result["updated"], ids.len(), "{result}");
        wire.counts()
    };
    let once_per_mutation = |counts: &Counts| {
        // The change-log seqs and the audit rows.
        assert_eq!(counts.run_matching("pylon_change_seq"), 1, "{counts:#?}");
        assert_eq!(
            counts.run_matching("INSERT INTO _pylon_audit_events"),
            1,
            "{counts:#?}"
        );
        // No client subscribes to a row: no CRDT frame is read after commit.
        assert_eq!(counts.run_matching("s.stamp"), 0, "{counts:#?}");
    };

    // One at a time: 10 rows, then 30.
    let (few_ids, few) = seed(10, false);
    let (one_ids, more) = seed(30, false);
    // Per row: the CRDT lock, the doc read, the doc write, the INSERT, the
    // search text.
    let per_insert = (write_runs(&more) - write_runs(&few)) as f64 / 20.0;
    assert!(
        per_insert <= 5.0,
        "an insert runs {per_insert} statements; expected at most 5\n{more:#?}"
    );
    assert_eq!(
        write_parses(&more),
        write_parses(&few),
        "a statement is parsed again per inserted row\n{more:#?}"
    );
    let few = update(&few_ids, false);
    let more = update(&one_ids, false);
    // Per row: the CRDT lock, the row lock and read, the doc read, the doc
    // write, the UPDATE ... RETURNING, the search text.
    let per_update = (write_runs(&more) - write_runs(&few)) as f64 / 20.0;
    assert!(
        per_update <= 6.0,
        "an update runs {per_update} statements; expected at most 6\n{more:#?}"
    );
    assert_eq!(
        write_parses(&more),
        write_parses(&few),
        "a statement is parsed again per updated row\n{more:#?}"
    );
    once_per_mutation(&more);

    // insertMany / updateMany: the same statements for 10 rows as for 30.
    let (few_ids, few) = seed(10, true);
    let (many_ids, more) = seed(30, true);
    assert_eq!(
        write_runs(&more),
        write_runs(&few),
        "insertMany runs more statements for more rows\n{more:#?}"
    );
    let few = update(&few_ids, true);
    let more = update(&many_ids, true);
    assert_eq!(
        write_runs(&more),
        write_runs(&few),
        "updateMany runs more statements for more rows\n{more:#?}"
    );
    once_per_mutation(&more);
    eprintln!(
        "statements per row one at a time: insert {per_insert}, update {per_update}; \
         per updateMany: {}",
        write_runs(&more)
    );

    // Both ways store the same thing.
    for (one, many) in one_ids.iter().zip(&many_ids) {
        let one_row = rt.get_by_id("Lead", one).unwrap().expect("row");
        let many_row = rt.get_by_id("Lead", many).unwrap().expect("row");
        for row in [&one_row, &many_row] {
            assert_eq!(row["stage"], "contacted", "{row}");
            assert_eq!(row["visits"], 1, "{row}");
            assert!(row["name"].as_str().unwrap().ends_with('!'), "{row}");
        }
        // The CRDT doc holds the row's values.
        for (id, row) in [(one, &one_row), (many, &many_row)] {
            let doc = pylon_crdt::loro::LoroDoc::new();
            pylon_crdt::apply_update(&doc, &rt.crdt_snapshot("Lead", id).unwrap().unwrap())
                .unwrap();
            let held = pylon_crdt::root_map(&doc).get_deep_value();
            let held = format!("{held:?}");
            for field in ["stage", "name"] {
                let value = row[field].as_str().unwrap();
                assert!(held.contains(value), "{id}: {field} {value} not in {held}");
            }
        }
    }
    let count = |admin: &mut postgres::Client, sql: &str, ids: &[String]| -> i64 {
        admin.query_one(sql, &[&ids]).unwrap().get(0)
    };
    for ids in [&one_ids, &many_ids] {
        // One audit event per update, naming the fields the patch set.
        assert_eq!(
            count(
                &mut admin,
                "SELECT count(*) FROM _pylon_audit_events WHERE entity = 'Lead' \
                 AND action = 'entity.update' AND entity_id = ANY($1) \
                 AND metadata_json LIKE '%name,stage,visits%'",
                ids,
            ),
            30
        );
        // The search text is the updated name.
        assert_eq!(
            count(
                &mut admin,
                "SELECT count(*) FROM \"_fts_Lead\" f JOIN \"Lead\" l ON l.id = f.entity_id \
                 WHERE l.id = ANY($1) AND f.tsv = to_tsvector('english', l.name)",
                ids,
            ),
            30
        );
        // Each row has an insert and an update event with the row before.
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let events = count(
                &mut admin,
                "SELECT count(*) FROM pylon_change_log WHERE entity = 'Lead' \
                 AND row_id = ANY($1) AND (kind = 'insert' \
                 OR (kind = 'update' AND prev_data->>'stage' = 'new' \
                     AND data->>'stage' = 'contacted'))",
                ids,
            );
            if events == 60 {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "{events} change events, expected 60"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

/// The functions pool loads after the server listens: wait for a call.
fn functions_ready(port: u16) -> bool {
    let req = format!(
        "POST /api/fn/seed HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: 36\r\nConnection: close\r\n\r\n{{\"count\":0,\"many\":false,\"prefix\":\"\"}}"
    );
    let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    if stream.write_all(req.as_bytes()).is_err() {
        return false;
    }
    let mut raw = String::new();
    stream.read_to_string(&mut raw).ok();
    raw.split_whitespace().nth(1) == Some("200")
}
