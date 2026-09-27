//! The CRDT push, seed, and merge rules, run on both backends.
//!
//! Every scenario runs on SQLite. With `PYLON_TEST_PG_URL` set it also runs
//! on Postgres (one database, so `--test-threads=1`):
//!
//! ```sh
//! PYLON_TEST_PG_URL=postgres://postgres:test@localhost:5544/postgres \
//!   cargo test -p pylon-runtime --test crdt_parity -- --test-threads=1
//! ```

use pylon_crdt::loro::{
    Container, ContainerID, ContainerTrait, ExportMode, LoroDoc, LoroText, ValueOrContainer,
    VersionVector,
};
use pylon_crdt::{CrdtField, CrdtFieldKind};
use pylon_http::DataStore;
use pylon_kernel::*;
use pylon_runtime::Runtime;
use serde_json::{json, Value};

enum Backend {
    Sqlite(tempfile::TempDir),
    Postgres(String),
}

struct Env {
    rt: Runtime,
    backend: Backend,
}

fn field(name: &str, ty: &str, crdt: Option<CrdtAnnotation>, optional: bool) -> ManifestField {
    ManifestField {
        name: name.into(),
        field_type: ty.into(),
        optional,
        unique: false,
        crdt,
        server_only: false,
        readonly: false,
        default: None,
        enum_values: None,
        encrypted: false,
        sync_omit: false,
    }
}

/// A `Doc` entity holding every CRDT field kind.
fn manifest() -> AppManifest {
    AppManifest {
        required_env: Vec::new(),
        build: Default::default(),
        shards: Vec::new(),
        manifest_version: 1,
        name: "crdt_parity".into(),
        version: "1".into(),
        entities: vec![ManifestEntity {
            name: "Doc".into(),
            fields: vec![
                field("title", "string", None, false),
                field("meta", "json", None, false),
                field("tags", "json", Some(CrdtAnnotation::List), false),
                field("order", "json", Some(CrdtAnnotation::MovableList), true),
                field("outline", "json", Some(CrdtAnnotation::Tree), true),
                field("likes", "int", Some(CrdtAnnotation::Counter), false),
                field("done", "bool", None, false),
                field("body", "richtext", None, true),
            ],
            indexes: vec![],
            relations: vec![],
            crdt: true,
            sync: true,
            search: None,
            ..Default::default()
        }],
        routes: vec![],
        queries: vec![],
        actions: vec![],
        policies: vec![],
        auth: Default::default(),
        llm: Default::default(),
        connections: vec![],
        crons: vec![],
        fonts: vec![],
    }
}

fn fields() -> Vec<CrdtField> {
    use CrdtFieldKind as K;
    [
        ("title", K::LwwString),
        ("meta", K::LwwJson),
        ("tags", K::List),
        ("order", K::MovableList),
        ("outline", K::Tree),
        ("likes", K::Counter),
        ("done", K::LwwBool),
        ("body", K::Text),
    ]
    .into_iter()
    .map(|(name, kind)| CrdtField {
        name: name.into(),
        kind,
    })
    .collect()
}

fn sqlite() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.db");
    let rt = Runtime::open(path.to_str().unwrap(), manifest()).unwrap();
    Env {
        rt,
        backend: Backend::Sqlite(dir),
    }
}

fn postgres(url: &str) -> Env {
    let manifest = manifest();
    let mut adapter = pylon_storage::postgres::live::LivePostgresAdapter::connect(url)
        .expect("connect to test postgres");
    for table in [
        "\"Doc\"",
        "_pylon_crdt_snapshots",
        "_pylon_crdt_synthetic",
        "_pylon_crdt_base",
        "_pylon_crdt_links",
    ] {
        let _ = adapter.exec_raw(&format!("DROP TABLE IF EXISTS {table} CASCADE"));
    }
    let plan = adapter
        .plan_from_live(&manifest)
        .expect("plan against fresh schema");
    adapter.apply_plan(&plan).expect("apply schema");
    let rt = Runtime::open_postgres(url, manifest).expect("open postgres runtime");
    Env {
        rt,
        backend: Backend::Postgres(url.to_string()),
    }
}

/// Run `scenario` on SQLite, then on Postgres when a test database is set.
fn on_both(scenario: impl Fn(&Env)) {
    scenario(&sqlite());
    if let Ok(url) = std::env::var("PYLON_TEST_PG_URL") {
        scenario(&postgres(&url));
    }
}

impl Env {
    fn name(&self) -> &'static str {
        match self.backend {
            Backend::Sqlite(_) => "sqlite",
            Backend::Postgres(_) => "postgres",
        }
    }

    /// Run `sql` against the backend's database directly.
    fn raw(&self, sqlite_sql: &str, pg_sql: &str, id: &str) -> u64 {
        match &self.backend {
            Backend::Sqlite(dir) => {
                let conn = rusqlite::Connection::open(dir.path().join("app.db")).unwrap();
                conn.query_row(sqlite_sql, [id], |r| r.get::<_, i64>(0))
                    .unwrap_or(0) as u64
            }
            Backend::Postgres(url) => {
                let mut client = postgres::Client::connect(url, postgres::NoTls).unwrap();
                client
                    .query_opt(pg_sql, &[&id])
                    .unwrap()
                    .map_or(0, |r| r.get::<_, i64>(0) as u64)
            }
        }
    }

    /// A row with no doc (from before its entity was CRDT): inserted, then
    /// its snapshot and side records removed underneath the runtime.
    fn bare_row(&self) -> String {
        let id = self.rt.insert("Doc", &fresh_doc()).unwrap();
        for table in [
            "_pylon_crdt_snapshots",
            "_pylon_crdt_synthetic",
            "_pylon_crdt_base",
            "_pylon_crdt_links",
        ] {
            match &self.backend {
                Backend::Sqlite(dir) => {
                    let conn = rusqlite::Connection::open(dir.path().join("app.db")).unwrap();
                    conn.execute(
                        &format!("DELETE FROM {table} WHERE entity = 'Doc' AND row_id = ?1"),
                        [&id],
                    )
                    .unwrap();
                }
                Backend::Postgres(url) => {
                    // A side table a build without it never made is skipped.
                    let mut client = postgres::Client::connect(url, postgres::NoTls).unwrap();
                    let deleted = client.execute(
                        &format!("DELETE FROM {table} WHERE entity = 'Doc' AND row_id = $1"),
                        &[&id],
                    );
                    if table == "_pylon_crdt_snapshots" {
                        deleted.unwrap();
                    }
                }
            }
        }
        if let Backend::Sqlite(_) = self.backend {
            self.rt.crdt_store().clear_cache();
        }
        id
    }

    fn row(&self, id: &str) -> Value {
        self.rt.get_by_id("Doc", id).unwrap().unwrap()
    }

    fn snapshot(&self, id: &str) -> Vec<u8> {
        self.rt.crdt_snapshot("Doc", id).unwrap().unwrap()
    }

    fn push(&self, id: &str, doc: &LoroDoc, since: &VersionVector) {
        let update = doc.export(ExportMode::updates(since)).unwrap();
        self.rt.crdt_apply_update("Doc", id, &update).unwrap();
    }

    /// The peer that made the container holding `field`'s key.
    fn key_owner(&self, id: &str, field: &str) -> u64 {
        let doc = LoroDoc::new();
        pylon_crdt::apply_update(&doc, &self.snapshot(id)).unwrap();
        match pylon_crdt::root_map(&doc).get(field) {
            Some(ValueOrContainer::Container(c)) => match c.id() {
                ContainerID::Normal { peer, .. } => peer,
                other => panic!("{other:?}"),
            },
            other => panic!("{other:?}"),
        }
    }
}

fn fresh_doc() -> Value {
    json!({
        "title": "a", "meta": {"k": 1}, "tags": ["x"], "order": ["m", "n"],
        "outline": [{"id": "root", "parent": null}], "likes": 3,
        "done": false, "body": "hello",
    })
}

/// A client doc with `earlier_ops` ops of its own first: enough raise its
/// next ops' Lamport clock over the seed's, so its containers take their
/// keys.
fn client_doc(peer: u64, earlier_ops: i32) -> LoroDoc {
    let doc = LoroDoc::new();
    doc.set_peer_id(peer).unwrap();
    let scratch = doc.get_map("scratch");
    for i in 0..earlier_ops {
        scratch.insert(&i.to_string(), i).unwrap();
    }
    doc.commit();
    doc
}

/// A client that has the server's doc for the row.
fn online_client(env: &Env, id: &str) -> LoroDoc {
    let doc = client_doc(9, 0);
    pylon_crdt::apply_update(&doc, &env.snapshot(id)).unwrap();
    doc
}

fn text_in(doc: &LoroDoc, field: &str) -> LoroText {
    match pylon_crdt::root_map(doc).get(field) {
        Some(ValueOrContainer::Container(Container::Text(t))) => t,
        other => panic!("{other:?}"),
    }
}

fn patch(doc: &LoroDoc, value: Value) {
    pylon_crdt::apply_patch(doc, &fields(), &value).unwrap();
}

/// A client that edited an empty doc offline keeps its register value and
/// its text when a read seeded the doc first.
#[test]
fn an_offline_edit_on_an_empty_doc_survives_a_seed() {
    on_both(|env| {
        for peer in [1, u64::MAX - 1] {
            let id = env.bare_row();
            let offline = client_doc(peer, 0);
            patch(&offline, json!({"done": true, "body": "typed"}));
            env.snapshot(&id);
            env.push(&id, &offline, &Default::default());
            let row = env.row(&id);
            assert_eq!(row["done"], true, "{}", env.name());
            assert_eq!(row["body"], "typed", "{}", env.name());
            assert_eq!(row["title"], "a", "{}", env.name());
        }
    });
}

/// An online client edited the seed's text; an offline client's text then
/// arrives. Whichever container takes the key, both edits stay.
#[test]
fn a_client_text_keeps_the_edits_to_the_seed() {
    on_both(|env| {
        for (peer, earlier_ops, want) in [
            (1u64, 0, "hello worldtyped"),
            (u64::MAX - 1, 40, "typedhello world"),
        ] {
            let id = env.bare_row();
            let online = online_client(env, &id);
            let before = online.oplog_vv();
            let body = text_in(&online, "body");
            body.insert(body.len_unicode(), " world").unwrap();
            online.commit();
            env.push(&id, &online, &before);
            let offline = client_doc(peer, earlier_ops);
            patch(&offline, json!({"body": "typed"}));
            env.push(&id, &offline, &Default::default());
            assert_eq!(env.key_owner(&id, "body") == peer, earlier_ops > 0);
            assert_eq!(env.row(&id)["body"], want, "{} peer {peer}", env.name());
        }
    });
}

/// A later push from an offline client whose text and list lost their
/// keys: inserts and deletes inside them land where the client made them.
#[test]
fn a_later_push_edits_the_merged_text_where_the_client_edited() {
    on_both(|env| {
        let id = env.bare_row();
        let online = online_client(env, &id);
        let before = online.oplog_vv();
        let body = text_in(&online, "body");
        body.insert(body.len_unicode(), " world").unwrap();
        online.commit();
        env.push(&id, &online, &before);
        let offline = client_doc(1, 0);
        patch(&offline, json!({"body": "typed", "tags": ["a", "b"]}));
        env.push(&id, &offline, &Default::default());
        assert_eq!(env.row(&id)["body"], "hello worldtyped", "{}", env.name());
        assert_eq!(env.row(&id)["tags"], json!(["a", "b"]), "{}", env.name());
        let pushed = offline.oplog_vv();
        let body = text_in(&offline, "body");
        body.insert(0, "!").unwrap();
        body.delete(2, 1).unwrap();
        match pylon_crdt::root_map(&offline).get("tags") {
            Some(ValueOrContainer::Container(Container::List(l))) => {
                l.delete(0, 1).unwrap();
                l.push("c").unwrap();
            }
            other => panic!("{other:?}"),
        }
        offline.commit();
        env.push(&id, &offline, &pushed);
        assert_eq!(env.row(&id)["body"], "hello world!tped", "{}", env.name());
        assert_eq!(env.row(&id)["tags"], json!(["b", "c"]), "{}", env.name());
    });
}

/// An offline client that pulled before it pushed: its offline increment
/// counts once, and its text replaces a seed nobody edited.
#[test]
fn an_offline_client_that_pulled_before_pushing_keeps_its_edits() {
    on_both(|env| {
        let id = env.bare_row();
        let seeded = env.snapshot(&id);
        let client = client_doc(1, 0);
        patch(&client, json!({"likes": 1, "body": "typed"}));
        pylon_crdt::apply_update(&client, &seeded).unwrap();
        let server_vv = {
            let doc = LoroDoc::new();
            pylon_crdt::apply_update(&doc, &seeded).unwrap();
            doc.oplog_vv()
        };
        patch(&client, json!({"likes": 1}));
        env.push(&id, &client, &server_vv);
        let row = env.row(&id);
        assert_eq!(row["likes"], 5, "{}", env.name());
        assert_eq!(row["body"], "typed", "{}", env.name());
    });
}

/// A server write of a text is the value a client's text replaces while
/// nobody has edited it since, whichever container takes the key.
#[test]
fn a_client_text_replaces_the_last_server_write() {
    on_both(|env| {
        for (peer, earlier_ops) in [(1u64, 0), (u64::MAX - 1, 40)] {
            let id = env.bare_row();
            env.snapshot(&id);
            env.rt
                .update("Doc", &id, &json!({"body": "server"}))
                .unwrap();
            let offline = client_doc(peer, earlier_ops);
            patch(&offline, json!({"body": "typed"}));
            env.push(&id, &offline, &Default::default());
            assert_eq!(env.row(&id)["body"], "typed", "{} peer {peer}", env.name());
        }
    });
}

/// An offline client's empty text and list replace a seed nobody edited.
#[test]
fn an_empty_client_value_replaces_the_seed() {
    on_both(|env| {
        let id = env.bare_row();
        env.snapshot(&id);
        let offline = client_doc(1, 0);
        patch(&offline, json!({"body": "", "tags": []}));
        env.push(&id, &offline, &Default::default());
        let row = env.row(&id);
        assert_eq!(row["body"], "", "{}", env.name());
        assert_eq!(row["tags"], json!([]), "{}", env.name());
    });
}

/// A seed movable list whose item another client set: an offline client's
/// list is added to it.
#[test]
fn a_set_in_the_seed_movable_list_keeps_it() {
    on_both(|env| {
        let id = env.bare_row();
        let online = online_client(env, &id);
        let before = online.oplog_vv();
        match pylon_crdt::root_map(&online).get("order") {
            Some(ValueOrContainer::Container(Container::MovableList(l))) => l.set(0, "M").unwrap(),
            other => panic!("{other:?}"),
        }
        online.commit();
        env.push(&id, &online, &before);
        let offline = client_doc(1, 0);
        patch(&offline, json!({"order": ["p"]}));
        env.push(&id, &offline, &Default::default());
        assert_eq!(
            env.row(&id)["order"],
            json!(["M", "n", "p"]),
            "{}",
            env.name()
        );
    });
}

/// An offline client's tree joins the seed's by node id; a later push that
/// deletes one of its nodes and adds another reaches the tree holding the
/// key.
#[test]
fn an_offline_tree_joins_the_seed_tree_by_node_id() {
    on_both(|env| {
        let id = env.bare_row();
        let online = online_client(env, &id);
        let before = online.oplog_vv();
        patch(
            &online,
            json!({"outline": [
                {"id": "root", "parent": null},
                {"id": "x", "parent": "root"},
            ]}),
        );
        env.push(&id, &online, &before);
        let offline = client_doc(1, 0);
        patch(&offline, json!({"outline": [{"id": "c", "parent": null}]}));
        env.push(&id, &offline, &Default::default());
        let ids = || -> Vec<String> {
            let mut ids: Vec<String> = env.row(&id)["outline"]
                .as_array()
                .unwrap()
                .iter()
                .map(|n| n["id"].as_str().unwrap().to_string())
                .collect();
            ids.sort();
            ids
        };
        assert_eq!(ids(), ["c", "root", "x"], "{}", env.name());
        let pushed = offline.oplog_vv();
        patch(&offline, json!({"outline": [{"id": "d", "parent": null}]}));
        env.push(&id, &offline, &pushed);
        assert_eq!(ids(), ["d", "root", "x"], "{}", env.name());
    });
}

/// A server write to a row with no doc keeps the fields it did not set.
#[test]
fn a_server_write_on_a_row_with_no_doc_keeps_the_other_fields() {
    on_both(|env| {
        let id = env.bare_row();
        env.rt.update("Doc", &id, &json!({"title": "b"})).unwrap();
        // A client's push then projects the whole doc into the row.
        let online = online_client(env, &id);
        let before = online.oplog_vv();
        patch(&online, json!({"done": true}));
        env.push(&id, &online, &before);
        let row = env.row(&id);
        assert_eq!(row["title"], "b", "{}", env.name());
        assert_eq!(row["likes"], 3, "{}", env.name());
        assert_eq!(row["tags"], json!(["x"]), "{}", env.name());
        assert_eq!(row["body"], "hello", "{}", env.name());
        assert_eq!(row["done"], true, "{}", env.name());
    });
}

/// A push to a row with no doc (nobody read it since): the update's fields
/// stand, the others keep the row's values, and a counter adds the row's
/// total.
#[test]
fn a_push_to_a_row_with_no_doc_fills_the_rest_from_the_row() {
    on_both(|env| {
        let id = env.bare_row();
        let offline = client_doc(1, 0);
        patch(&offline, json!({"likes": 2, "body": "typed"}));
        env.push(&id, &offline, &Default::default());
        let row = env.row(&id);
        assert_eq!(row["likes"], 5, "{}", env.name());
        assert_eq!(row["body"], "typed", "{}", env.name());
        assert_eq!(row["title"], "a", "{}", env.name());
        assert_eq!(row["tags"], json!(["x"]), "{}", env.name());
    });
}

/// A client can put any container under a key: a text under a tree or
/// counter field does not stop the push or later writes.
#[test]
fn a_container_of_the_wrong_type_under_a_key_does_not_break_writes() {
    on_both(|env| {
        for field in ["outline", "likes"] {
            let id = env.bare_row();
            env.snapshot(&id);
            for peer in [1u64, 2] {
                let client = client_doc(peer, 40);
                let t = pylon_crdt::root_map(&client)
                    .insert_container(field, LoroText::new())
                    .unwrap();
                t.insert(0, "x").unwrap();
                client.commit();
                let _ = env.rt.crdt_apply_update(
                    "Doc",
                    &id,
                    &client.export(ExportMode::all_updates()).unwrap(),
                );
            }
            env.rt
                .update("Doc", &id, &json!({"title": "still writes"}))
                .unwrap();
        }
    });
}

/// Many merging pushes from one offline client: the row's version vector
/// holds a few peers, not one per push.
#[test]
fn merging_pushes_keep_the_version_vector_small() {
    on_both(|env| {
        let id = env.bare_row();
        env.snapshot(&id);
        let offline = client_doc(1, 0);
        patch(&offline, json!({"body": "a"}));
        env.push(&id, &offline, &Default::default());
        for _ in 0..20 {
            let pushed = offline.oplog_vv();
            let body = text_in(&offline, "body");
            body.insert(body.len_unicode(), "b").unwrap();
            offline.commit();
            env.push(&id, &offline, &pushed);
            env.rt.update("Doc", &id, &json!({"title": "t"})).unwrap();
        }
        assert_eq!(env.row(&id)["body"], format!("a{}", "b".repeat(20)));
        let doc = LoroDoc::new();
        pylon_crdt::apply_update(&doc, &env.snapshot(&id)).unwrap();
        assert!(
            doc.oplog_vv().len() <= 4,
            "{} {:?}",
            env.name(),
            doc.oplog_vv()
        );
    });
}

/// Deleting a row removes its snapshot and its side records.
#[test]
fn a_delete_removes_the_side_records() {
    on_both(|env| {
        let id = env.bare_row();
        let offline = client_doc(1, 0);
        patch(&offline, json!({"body": "typed"}));
        env.snapshot(&id);
        env.push(&id, &offline, &Default::default());
        let count = |table: &str| {
            env.raw(
                &format!("SELECT COUNT(*) FROM {table} WHERE entity = 'Doc' AND row_id = ?1"),
                &format!("SELECT COUNT(*) FROM {table} WHERE entity = 'Doc' AND row_id = $1"),
                &id,
            )
        };
        assert!(count("_pylon_crdt_base") > 0, "{}", env.name());
        assert!(count("_pylon_crdt_synthetic") > 0, "{}", env.name());
        assert!(env.rt.delete("Doc", &id).unwrap());
        for table in [
            "_pylon_crdt_snapshots",
            "_pylon_crdt_synthetic",
            "_pylon_crdt_base",
            "_pylon_crdt_links",
        ] {
            assert_eq!(count(table), 0, "{} {table}", env.name());
        }
    });
}
