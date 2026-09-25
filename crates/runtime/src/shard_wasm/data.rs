//! The app's data from shards (issue #33).
//!
//! A module calls the app's functions after each tick (`pylon_calls`), and
//! gets each result at the start of a later tick (`pylon_call_result`). A
//! mutation runs once per key (see `crate::fn_calls`): a grant sent again
//! after a crash returns the first run's result and changes nothing. Worker
//! threads make the calls, so a slow function never delays a tick.
//!
//! A module also writes entity fields after each tick (`pylon_writes`). The
//! host keeps the latest value of each field, per shard, and writes them
//! every [`WRITE_EVERY`] and when the shard stops, through the same path as
//! `PATCH /api/entities/<entity>/<id>` (plugins, change log, sync), with
//! admin rights. Writes since the last flush are lost when the process
//! crashes.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use pylon_realtime::CallResult;
use serde::Deserialize;

use super::WasmShardHost;

/// Calls waiting for a worker, on all shards.
pub(super) const CALL_QUEUE: usize = 4096;
/// Calls one shard has in flight. Past this, the module sends the others
/// again on later ticks.
pub(super) const MAX_CALLS_IN_FLIGHT: usize = 256;
const MAX_KEY: usize = 256;
const MAX_FN_NAME: usize = 128;
/// Rows one shard holds to write. Past this, new rows are dropped.
pub(super) const MAX_DIRTY_ROWS: usize = 10_000;
/// Flushes a row that fails with a store error is kept for.
const WRITE_ATTEMPTS: u32 = 5;
/// Worker threads that make calls, unless `PYLON_SHARD_CALL_WORKERS` sets it.
const CALL_WORKERS: usize = 4;

/// How often buffered writes are flushed, unless
/// `PYLON_SHARD_WRITE_EVERY_MS` sets it.
pub(super) const WRITE_EVERY: Duration = Duration::from_secs(2);

/// A function call a module asked for.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Call {
    pub key: String,
    #[serde(rename = "fn")]
    pub function: String,
    #[serde(default)]
    pub args: serde_json::Value,
}

/// Entity fields a module asked to write.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Write {
    pub entity: String,
    pub id: String,
    pub set: serde_json::Map<String, serde_json::Value>,
}

/// A call waiting for a worker.
pub(super) struct QueuedCall {
    shard: String,
    call: Call,
}

/// Fields waiting to be written to one row.
#[derive(Debug, Default)]
pub(super) struct DirtyRow {
    set: serde_json::Map<String, serde_json::Value>,
    failures: u32,
}

/// Buffered writes, by shard, then by (entity, id).
pub(super) type Dirty = HashMap<String, HashMap<(String, String), DirtyRow>>;

/// The call queue: the sender for the tick hook, the receiver for the
/// workers (taken when functions are attached).
pub(super) fn call_queue() -> (SyncSender<QueuedCall>, Mutex<Option<Receiver<QueuedCall>>>) {
    let (tx, rx) = std::sync::mpsc::sync_channel(CALL_QUEUE);
    (tx, Mutex::new(Some(rx)))
}

fn in_flight_key(call: &Call) -> String {
    format!("{}\u{0}{}", call.function, call.key)
}

fn error_result(key: &str, code: &str, message: &str) -> CallResult {
    CallResult {
        key: key.to_string(),
        ok: false,
        data: Arc::from(format!("{code}: {message}").into_bytes()),
    }
}

/// The identity a shard's calls run with: the server's own, as a scheduled
/// job's or a cron's. Functions trust their arguments from a shard as they
/// do from another function.
fn shard_auth() -> pylon_functions::protocol::AuthInfo {
    pylon_functions::protocol::AuthInfo {
        user_id: None,
        is_admin: true,
        tenant_id: None,
        roles: Vec::new(),
        is_guest: false,
    }
}

fn env_or<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

impl WasmShardHost {
    /// Make shards' function calls with `functions` (`None` when the app
    /// has none: calls then fail with `NO_FUNCTIONS`), and write their
    /// entity fields with `writer`. Set once; later calls do nothing. Until
    /// then, calls and writes wait (shards a machine adopts at boot can
    /// tick before the functions start).
    pub fn attach_data(
        self: &Arc<Self>,
        functions: Option<Weak<dyn pylon_router::FnOps>>,
        writer: Arc<crate::entity_writer::EntityWriter>,
    ) {
        if self.functions.set(functions).is_err() {
            return;
        }
        let _ = self.writer.set(writer);
        let Some(rx) = self.call_rx.lock().unwrap().take() else {
            return;
        };
        let rx = Arc::new(Mutex::new(rx));
        for n in 0..env_or("PYLON_SHARD_CALL_WORKERS", CALL_WORKERS).max(1) {
            let rx = Arc::clone(&rx);
            let host = Arc::downgrade(self);
            let _ = std::thread::Builder::new()
                .name(format!("pylon-shard-calls-{n}"))
                .spawn(move || Self::run_calls(host, rx));
        }
        let host = Arc::downgrade(self);
        let every = Duration::from_millis(env_or(
            "PYLON_SHARD_WRITE_EVERY_MS",
            WRITE_EVERY.as_millis() as u64,
        ));
        let _ = std::thread::Builder::new()
            .name("pylon-shard-writes".into())
            .spawn(move || loop {
                std::thread::sleep(every);
                let Some(host) = host.upgrade() else { return };
                host.flush_writes(None);
            });
    }

    /// Queue the calls a module asked for after a tick. A call whose key is
    /// in flight for this shard is skipped; so are calls past
    /// [`MAX_CALLS_IN_FLIGHT`] or a full queue (the module sends them again).
    pub(super) fn take_calls(&self, shard: &str, calls: Vec<Call>) {
        let Some(target) = self.registry.get(shard) else {
            return;
        };
        if matches!(self.functions.get(), Some(None)) {
            for call in calls {
                target.push_call_result(error_result(
                    &call.key,
                    "NO_FUNCTIONS",
                    "the app runs no functions",
                ));
            }
            return;
        }
        let mut in_flight = self.calls_in_flight.lock().unwrap();
        for call in calls {
            if call.key.is_empty() || call.key.len() > MAX_KEY {
                target.push_call_result(error_result(
                    &call.key,
                    "INVALID_KEY",
                    &format!("a call key is 1 to {MAX_KEY} bytes"),
                ));
                continue;
            }
            if call.function.is_empty() || call.function.len() > MAX_FN_NAME {
                target.push_call_result(error_result(
                    &call.key,
                    "FN_NOT_FOUND",
                    "a function name is 1 to 128 bytes",
                ));
                continue;
            }
            let keys = in_flight.entry(shard.to_string()).or_default();
            let key = in_flight_key(&call);
            if keys.contains(&key) || keys.len() >= MAX_CALLS_IN_FLIGHT {
                continue;
            }
            match self.calls.try_send(QueuedCall {
                shard: shard.to_string(),
                call,
            }) {
                Ok(()) => {
                    keys.insert(key);
                }
                Err(_) => {
                    tracing::warn!(
                        "[shard {shard}] the call queue is full; the module sends the call again"
                    );
                    break;
                }
            }
        }
    }

    fn run_calls(host: Weak<Self>, rx: Arc<Mutex<Receiver<QueuedCall>>>) {
        loop {
            let next = rx.lock().unwrap().recv();
            let Ok(QueuedCall { shard, call }) = next else {
                return;
            };
            let Some(host) = host.upgrade() else { return };
            let flight = in_flight_key(&call);
            let result = match host
                .functions
                .get()
                .cloned()
                .flatten()
                .and_then(|w| w.upgrade())
            {
                Some(functions) => {
                    match functions.call_once(&call.function, call.args, shard_auth(), &call.key) {
                        Ok(value) => CallResult {
                            key: call.key,
                            ok: true,
                            data: Arc::from(value.to_string().into_bytes()),
                        },
                        Err(e) => error_result(&call.key, &e.code, &e.message),
                    }
                }
                None => error_result(&call.key, "NO_FUNCTIONS", "the app runs no functions"),
            };
            // The result reaches the shard before its key leaves the
            // in-flight set, so the module never sends a call again while
            // its result is on the way. A shard that stopped meanwhile
            // sends the call again where it runs next, and gets the same
            // result.
            if let Some(target) = host.registry.get(&shard) {
                target.push_call_result(result);
            }
            let mut in_flight = host.calls_in_flight.lock().unwrap();
            if let Some(keys) = in_flight.get_mut(&shard) {
                keys.remove(&flight);
                if keys.is_empty() {
                    in_flight.remove(&shard);
                }
            }
        }
    }

    /// Buffer the writes a module asked for after a tick: each field keeps
    /// its latest value.
    pub(super) fn take_writes(&self, shard: &str, writes: Vec<Write>) {
        let mut dirty = self.dirty.lock().unwrap();
        let rows = dirty.entry(shard.to_string()).or_default();
        let mut dropped = 0usize;
        for w in writes {
            if w.entity.is_empty() || w.id.is_empty() || w.set.is_empty() {
                continue;
            }
            let key = (w.entity, w.id);
            if !rows.contains_key(&key) && rows.len() >= MAX_DIRTY_ROWS {
                dropped += 1;
                continue;
            }
            rows.entry(key).or_default().set.extend(w.set);
        }
        if dropped > 0 {
            tracing::warn!(
                "[shard {shard}] {dropped} write(s) dropped: {MAX_DIRTY_ROWS} rows already wait"
            );
        }
    }

    /// Write the buffered fields of `shard`, or of every shard. One flush at
    /// a time, so a later value is never overwritten by an earlier one. A
    /// row that fails with a store error is kept (under newer values) for
    /// [`WRITE_ATTEMPTS`] flushes; one refused (missing row, plugin,
    /// validation) is dropped with a log line.
    pub(super) fn flush_writes(&self, shard: Option<&str>) {
        let Some(writer) = self.writer.get() else {
            return;
        };
        let _one = self.flush_lock.lock().unwrap();
        let taken: Dirty = {
            let mut dirty = self.dirty.lock().unwrap();
            match shard {
                Some(id) => dirty
                    .remove(id)
                    .map(|rows| (id.to_string(), rows))
                    .into_iter()
                    .collect(),
                None => std::mem::take(&mut *dirty),
            }
        };
        for (shard, rows) in taken {
            let mut failed: Vec<((String, String), DirtyRow)> = Vec::new();
            for ((entity, id), row) in rows {
                let fields = serde_json::Value::Object(row.set.clone());
                match writer.update(&entity, &id, &fields) {
                    Ok(()) => {}
                    Err(crate::entity_writer::WriteError::Refused(why)) => {
                        tracing::warn!("[shard {shard}] write to {entity} {id} refused: {why}");
                    }
                    Err(crate::entity_writer::WriteError::Store(why)) => {
                        if row.failures + 1 >= WRITE_ATTEMPTS {
                            tracing::warn!(
                                "[shard {shard}] write to {entity} {id} failed {WRITE_ATTEMPTS} times; dropped: {why}"
                            );
                        } else {
                            tracing::warn!(
                                "[shard {shard}] write to {entity} {id} failed; kept: {why}"
                            );
                            failed.push((
                                (entity, id),
                                DirtyRow {
                                    set: row.set,
                                    failures: row.failures + 1,
                                },
                            ));
                        }
                    }
                }
            }
            if failed.is_empty() {
                continue;
            }
            let mut dirty = self.dirty.lock().unwrap();
            let rows = dirty.entry(shard).or_default();
            for (key, old) in failed {
                let row = rows.entry(key).or_default();
                // Values buffered since the flush took this row are newer.
                for (field, value) in old.set {
                    row.set.entry(field).or_insert(value);
                }
                row.failures = row.failures.max(old.failures);
            }
        }
    }

    /// Rows with writes waiting, for tests.
    #[cfg(test)]
    pub(super) fn dirty_rows(&self, shard: &str) -> usize {
        self.dirty.lock().unwrap().get(shard).map_or(0, |r| r.len())
    }
}

/// The calls in flight per shard, by shard id.
pub(super) type InFlight = HashMap<String, HashSet<String>>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shard_wasm::{SendError, WasmLimits, WasmShardKind};
    use pylon_functions::runner::FnCallError;
    use pylon_realtime::{RawInput, ShardAuth, ShardConfig, SnapshotFormat, SubscriberId};
    use std::sync::Condvar;
    use std::time::Instant;

    /// Functions that keep `call_once`'s promise in memory: `loadCharacter`
    /// answers from a fixed row, `grantItem` waits while `hold` is set,
    /// then runs once per key.
    #[derive(Default)]
    struct Fns {
        /// The id of user p1's character.
        character: String,
        stored: Mutex<HashMap<(String, String), serde_json::Value>>,
        runs: Mutex<HashMap<String, usize>>,
        hold: Mutex<bool>,
        released: Condvar,
    }

    impl Fns {
        fn runs(&self, name: &str) -> usize {
            self.runs.lock().unwrap().get(name).copied().unwrap_or(0)
        }

        fn set_hold(&self, hold: bool) {
            *self.hold.lock().unwrap() = hold;
            self.released.notify_all();
        }
    }

    impl pylon_router::FnOps for Fns {
        fn get_fn(&self, _name: &str) -> Option<pylon_functions::registry::FnDef> {
            None
        }
        fn list_fns(&self) -> Vec<pylon_functions::registry::FnDef> {
            vec![]
        }
        fn call(
            &self,
            _fn_name: &str,
            _args: serde_json::Value,
            _auth: pylon_functions::protocol::AuthInfo,
            _on_stream: Option<pylon_functions::runner::StreamCallback>,
            _request: Option<pylon_functions::protocol::RequestInfo>,
            _stream_id: Option<String>,
        ) -> Result<(serde_json::Value, pylon_functions::trace::FnTrace), FnCallError> {
            unreachable!("shards call through call_once")
        }
        #[allow(clippy::too_many_arguments)]
        fn handle_form(
            &self,
            _component: &str,
            _route_path: &str,
            _method: &str,
            _url: &str,
            _params: serde_json::Value,
            _search_params: serde_json::Value,
            _form: serde_json::Value,
            _body: String,
            _headers: HashMap<String, String>,
            _cookies: HashMap<String, String>,
            _auth: pylon_functions::protocol::AuthInfo,
            _on_response_start: Option<pylon_functions::runner::ResponseStartCallback>,
            _on_chunk: pylon_functions::runner::ByteStreamCallback,
        ) -> Result<(), FnCallError> {
            unreachable!()
        }
        fn recent_traces(&self, _limit: usize) -> Vec<pylon_functions::trace::FnTrace> {
            vec![]
        }
        fn call_once(
            &self,
            fn_name: &str,
            args: serde_json::Value,
            auth: pylon_functions::protocol::AuthInfo,
            key: &str,
        ) -> Result<serde_json::Value, FnCallError> {
            assert!(auth.is_admin, "shards call with the server's rights");
            match fn_name {
                "loadCharacter" => {
                    *self.runs.lock().unwrap().entry(fn_name.into()).or_default() += 1;
                    assert_eq!(args["userId"], "p1");
                    Ok(serde_json::json!({
                        "id": self.character, "x": 7, "nextGrant": 3,
                        "items": [{ "id": "i-old", "name": "old" }]
                    }))
                }
                "grantItem" => {
                    let id = (fn_name.to_string(), key.to_string());
                    if let Some(v) = self.stored.lock().unwrap().get(&id) {
                        return Ok(v.clone());
                    }
                    *self.runs.lock().unwrap().entry(fn_name.into()).or_default() += 1;
                    let mut hold = self.hold.lock().unwrap();
                    while *hold {
                        hold = self.released.wait(hold).unwrap();
                    }
                    drop(hold);
                    assert_eq!(args["key"], key);
                    let v = serde_json::json!({ "itemId": format!("item-{key}") });
                    self.stored.lock().unwrap().insert(id, v.clone());
                    Ok(v)
                }
                other => Err(FnCallError {
                    code: "FN_NOT_FOUND".into(),
                    message: other.into(),
                }),
            }
        }
    }

    /// The example app's database, with user p1's character.
    fn runtime() -> (Arc<crate::Runtime>, Arc<Fns>) {
        let manifest: pylon_kernel::AppManifest = serde_json::from_str(include_str!(
            "../../../../examples/shard-arena/pylon.manifest.json"
        ))
        .unwrap();
        let rt = Arc::new(crate::Runtime::in_memory(manifest).unwrap());
        let character = rt
            .insert(
                "Character",
                &serde_json::json!({ "userId": "p1", "x": 0, "nextGrant": 3 }),
            )
            .unwrap();
        let fns = Arc::new(Fns {
            character,
            ..Default::default()
        });
        (rt, fns)
    }

    fn writer(rt: &Arc<crate::Runtime>) -> Arc<crate::entity_writer::EntityWriter> {
        Arc::new(crate::entity_writer::EntityWriter::new(
            Arc::clone(rt),
            Arc::new(pylon_sync::ChangeLog::new()),
            Arc::new(pylon_router::NoopNotifier),
            Arc::new(pylon_policy::PolicyEngine::from_manifest(rt.manifest())),
            Arc::new(pylon_plugin::PluginRegistry::new(rt.manifest().clone())),
        ))
    }

    fn host() -> Arc<WasmShardHost> {
        let zone = WasmShardKind::compile(
            "zone",
            include_bytes!("../../../../examples/shard-arena/shards/zone.wasm"),
            ShardConfig::default(),
            WasmLimits::default(),
        )
        .unwrap();
        WasmShardHost::new(vec![zone])
    }

    fn saved(host: &WasmShardHost, shard: &str) -> serde_json::Value {
        let shard = host.registry.get(shard).expect("running");
        let saved = shard.with_state(|sim| sim.save()).unwrap().unwrap();
        serde_json::from_slice(&saved).unwrap()
    }

    fn input(host: &WasmShardHost, shard: &str, sid: &str, json: &str) {
        host.registry
            .get(shard)
            .expect("running")
            .push_input(
                SubscriberId::new(sid),
                RawInput::new(SnapshotFormat::Json, json.as_bytes().to_vec()),
                None,
            )
            .unwrap();
    }

    fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// A joining player's character loads; a grant is sent once while it
    /// is in flight (the zone asks every tick), shows only after its
    /// result, and a zone restored from before the result sends it again
    /// under the same key and gets the stored result: one item.
    #[test]
    fn a_grant_runs_once_and_shows_after_its_result() {
        let (rt, fns) = runtime();
        let host = host();
        let weak: Weak<dyn pylon_router::FnOps> =
            Arc::downgrade(&(Arc::clone(&fns) as Arc<dyn pylon_router::FnOps>));
        host.attach_data(Some(weak), writer(&rt));
        host.create("zone", "z1", &serde_json::json!({})).unwrap();
        input(&host, "z1", "p1", "\"join\"");
        wait_for("the character to load", || {
            saved(&host, "z1")["players"]["p1"]["loaded"] == true
        });
        let p = &saved(&host, "z1")["players"]["p1"];
        assert_eq!(
            (p["x"].as_i64(), p["next_grant"].as_u64()),
            (Some(7), Some(3))
        );
        assert_eq!(fns.runs("loadCharacter"), 1);

        fns.set_hold(true);
        input(&host, "z1", "p1", r#"{"loot":{"item":"sword"}}"#);
        wait_for("the grant to start", || fns.runs("grantItem") == 1);
        let before_result = saved(&host, "z1");
        let key = format!("loot:{}:3", fns.character);
        assert_eq!(
            before_result["grants"][&key]["character"],
            fns.character.as_str()
        );
        assert_eq!(
            before_result["players"]["p1"]["items"],
            serde_json::json!({ "i-old": "old" })
        );
        // Many ticks resend it; one run.
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(fns.runs("grantItem"), 1);

        fns.set_hold(false);
        let both = serde_json::json!({ "i-old": "old", format!("item-{key}"): "sword" });
        wait_for("the item", || {
            saved(&host, "z1")["players"]["p1"]["items"] == both
        });
        assert_eq!(saved(&host, "z1")["grants"], serde_json::json!({}));
        assert!(host.calls_in_flight.lock().unwrap().is_empty());

        // The zone crashed before it saved the result: restored from the
        // earlier state, it loads the character again, sends the grant
        // again, and gets the stored result. The item is granted once and
        // shows once.
        host.stop("z1");
        let state = serde_json::to_vec(&before_result).unwrap();
        host.create_local(
            "zone",
            "z1",
            &serde_json::json!({}),
            Some(&state),
            None,
            None,
        )
        .unwrap();
        wait_for("the resent grant's result and the reload", || {
            let z = saved(&host, "z1");
            z["grants"] == serde_json::json!({}) && z["players"]["p1"]["loaded"] == true
        });
        let p = &saved(&host, "z1")["players"]["p1"];
        assert_eq!(p["items"], both);
        // The saved x (7), not the row's: the reload keeps the position.
        assert_eq!(
            (p["x"].as_i64(), p["next_grant"].as_u64()),
            (Some(7), Some(4))
        );
        assert_eq!(fns.runs("grantItem"), 1);
        assert_eq!(fns.runs("loadCharacter"), 2);
        host.stop_all();
    }

    /// Writes keep each field's latest value in one row, and the shard's
    /// stop writes them; a write to a missing row is dropped, not kept.
    #[test]
    fn writes_keep_the_latest_value_and_a_stop_writes_them() {
        let (rt, fns) = runtime();
        let host = host();
        let weak: Weak<dyn pylon_router::FnOps> =
            Arc::downgrade(&(Arc::clone(&fns) as Arc<dyn pylon_router::FnOps>));
        host.attach_data(Some(weak), writer(&rt));
        host.create("zone", "z2", &serde_json::json!({})).unwrap();
        input(&host, "z2", "p1", "\"join\"");
        wait_for("the character to load", || {
            saved(&host, "z2")["players"]["p1"]["loaded"] == true
        });
        input(&host, "z2", "p1", r#"{"move":{"dx":5}}"#);
        input(&host, "z2", "p1", r#"{"move":{"dx":3}}"#);
        wait_for("the moves", || {
            saved(&host, "z2")["players"]["p1"]["x"] == 15
        });
        wait_for("the buffered write", || host.dirty_rows("z2") == 1);
        // Not written yet (the flush runs every 2 s).
        assert_eq!(
            rt.get_by_id("Character", &fns.character).unwrap().unwrap()["x"],
            0
        );

        host.take_writes(
            "z2",
            vec![Write {
                entity: "Character".into(),
                id: "gone".into(),
                set: serde_json::Map::from_iter([("x".to_string(), serde_json::json!(1))]),
            }],
        );
        host.stop("z2");
        assert_eq!(
            rt.get_by_id("Character", &fns.character).unwrap().unwrap()["x"],
            15
        );
        assert_eq!(host.dirty_rows("z2"), 0);
        host.stop_all();
    }

    /// Calls wait until the functions are attached; with none, they fail
    /// with NO_FUNCTIONS and nothing stays in flight.
    #[test]
    fn calls_wait_for_the_functions() {
        let (rt, fns) = runtime();
        let host = host();
        host.create("zone", "z3", &serde_json::json!({})).unwrap();
        input(&host, "z3", "p1", "\"join\"");
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(saved(&host, "z3")["players"]["p1"]["loaded"], false);
        let weak: Weak<dyn pylon_router::FnOps> =
            Arc::downgrade(&(Arc::clone(&fns) as Arc<dyn pylon_router::FnOps>));
        host.attach_data(Some(weak), writer(&rt));
        wait_for("the character to load", || {
            saved(&host, "z3")["players"]["p1"]["loaded"] == true
        });
        host.stop_all();

        let host = self::host();
        host.attach_data(None, writer(&rt));
        host.create("zone", "z4", &serde_json::json!({})).unwrap();
        input(&host, "z4", "p1", "\"join\"");
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(saved(&host, "z4")["players"]["p1"]["loaded"], false);
        assert!(host.calls_in_flight.lock().unwrap().is_empty());
        host.stop_all();
    }

    /// A server input reaches the module from the empty subscriber id; a
    /// player cannot send one, nor connect as that id.
    #[test]
    fn a_server_input_comes_from_the_empty_id() {
        let host = host();
        host.create("zone", "z5", &serde_json::json!({})).unwrap();
        input(&host, "z5", "p1", "\"join\"");
        host.send_input(
            "z5",
            &serde_json::json!({ "heal": { "who": "p1", "hp": 5 } }),
        )
        .unwrap();
        wait_for("the heal", || {
            saved(&host, "z5")["players"]["p1"]["hp"] == 105
        });

        // The module refuses a heal from a player.
        input(&host, "z5", "p1", r#"{"heal":{"who":"p1","hp":50}}"#);
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(saved(&host, "z5")["players"]["p1"]["hp"], 105);

        let shard = host.registry.get("z5").unwrap();
        let sub = pylon_realtime::Subscriber::new(SubscriberId::new(""), Box::new(|_, _| {}));
        assert!(matches!(
            shard.add_subscriber_authorized(sub, &ShardAuth::default()),
            Err(pylon_realtime::ShardError::Unauthorized(_))
        ));

        assert_eq!(
            host.send_input("nope", &serde_json::json!("join")),
            Err(SendError::NotFound)
        );
        assert!(matches!(
            host.send_input("z5", &serde_json::json!("x".repeat(70 * 1024))),
            Err(SendError::Invalid(_))
        ));
        host.stop_all();
    }
}
