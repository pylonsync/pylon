//! How many statements a function's writes cost on Postgres, counted on
//! the wire. Each statement waits on the database, so on a remote database
//! the count, not the work, sets how long an import takes.
//!
//! An action calls a mutation (`ctx.runMutation`) that writes rows of an
//! entity with `audit: true`, a `search:` block with facets, and CRDT on
//! (the default): the shape of an importer. The test runs it for a few
//! rows and for more rows, and checks the cost of each extra row and that
//! no statement is parsed again per row.
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
    const ids = [];
    for (let i = 0; i < args.count; i++) {
      ids.push(await ctx.db.insert("Lead", { name: `lead ${i}`, stage: "new", visits: 0, teamId: "t1" }));
    }
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
    for (const row of rows) {
      await ctx.db.update("Lead", row.id, { stage: args.stage, visits: row.visits + 1 });
    }
    return { updated: rows.length };
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
/// table, the CRDT sidecar and lock, the change-log seq, and the audit log.
const WRITE_STATEMENTS: [&str; 5] = [
    "\"Lead\"",
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
             DROP TABLE IF EXISTS _fts_Lead CASCADE;
             DROP TABLE IF EXISTS _pylon_crdt_snapshots CASCADE;
             DROP TABLE IF EXISTS _pylon_crdt_updates CASCADE;
             DROP TABLE IF EXISTS _pylon_audit_events CASCADE;",
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

    // Inserts: 10 rows, then 30.
    let seed = |count: usize| -> (Vec<String>, Counts) {
        wire.reset();
        let result = call(port, "seed", &format!(r#"{{"count":{count}}}"#));
        let ids = result["ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|id| id.as_str().unwrap().to_string())
            .collect();
        (ids, wire.counts())
    };
    let (few_ids, few) = seed(10);
    let (more_ids, more) = seed(30);
    // Per row: the CRDT lock, the doc read, the doc write, the INSERT.
    let per_insert = (write_runs(&more) - write_runs(&few)) as f64 / 20.0;
    assert!(
        per_insert <= 4.0,
        "an insert runs {per_insert} statements; expected at most 4\n{more:#?}"
    );
    assert_eq!(
        write_parses(&more),
        write_parses(&few),
        "a statement is parsed again per inserted row\n{more:#?}"
    );

    // Updates through action -> runMutation, as an importer runs them.
    let update = |ids: &[String], stage: &str| -> Counts {
        wire.reset();
        let body = serde_json::json!({ "ids": ids, "stage": stage }).to_string();
        let result = call(port, "importPage", &body);
        assert_eq!(result["updated"], ids.len(), "{result}");
        wire.counts()
    };
    let few = update(&few_ids, "contacted");
    let more = update(&more_ids, "contacted");
    // Per row: the CRDT lock, the row lock and read, the doc read, the doc
    // write, the UPDATE ... RETURNING.
    let per_update = (write_runs(&more) - write_runs(&few)) as f64 / 20.0;
    eprintln!(
        "statements per row: insert {per_insert}, update {per_update}; \
         statements parsed per mutation: {}",
        write_parses(&more)
    );
    assert!(
        per_update <= 5.0,
        "an update runs {per_update} statements; expected at most 5\n{more:#?}"
    );
    assert_eq!(
        write_parses(&more),
        write_parses(&few),
        "a statement is parsed again per updated row\n{more:#?}"
    );
    // Once per mutation, not per row: the change-log seqs and the audit rows.
    assert_eq!(more.run_matching("pylon_change_seq"), 1, "{more:#?}");
    assert_eq!(
        more.run_matching("INSERT INTO _pylon_audit_events"),
        1,
        "{more:#?}"
    );
    // No client subscribes to a row: no CRDT frame is read after commit.
    assert_eq!(more.run_matching("s.stamp"), 0, "{more:#?}");

    // The writes landed with their side effects.
    for id in &more_ids {
        let row = rt.get_by_id("Lead", id).unwrap().expect("row");
        assert_eq!(row["stage"], "contacted", "{row}");
        assert_eq!(row["visits"], 1, "{row}");
    }
    let audited: i64 = admin
        .query_one(
            "SELECT count(*) FROM _pylon_audit_events \
             WHERE entity = 'Lead' AND action = 'entity.update'",
            &[],
        )
        .unwrap()
        .get(0);
    assert_eq!(audited, 40);
    let docs: i64 = admin
        .query_one(
            "SELECT count(*) FROM _pylon_crdt_snapshots WHERE entity = 'Lead'",
            &[],
        )
        .unwrap()
        .get(0);
    assert_eq!(docs, 40);
}

/// The functions pool loads after the server listens: wait for a call.
fn functions_ready(port: u16) -> bool {
    let req = format!(
        "POST /api/fn/seed HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{{\"count\":0}}"
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
