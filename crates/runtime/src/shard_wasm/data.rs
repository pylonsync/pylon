//! The app's data from shards (issue #33).
//!
//! A module calls the app's functions after each tick (`pylon_calls`), and
//! gets each result at the start of a later tick (`pylon_call_result`). A
//! mutation runs once per key (see `crate::fn_calls`): a grant sent again
//! after a crash returns the first run's result and changes nothing. Worker
//! threads make the calls, one shard's calls in turn with the others', so a
//! slow function never delays a tick and one busy shard does not hold up the
//! rest. A call that fails in the database or the functions runtime (see
//! [`retryable`]) is made again with the same key after a delay; the module
//! gets only a function's answer or its own error.
//!
//! A module also writes entity fields after each tick (`pylon_writes`). The
//! host keeps the latest value of each field of each row and writes them
//! every [`WRITE_EVERY`] and when the shard stops, through the same path as
//! `PATCH /api/entities/<entity>/<id>` (plugins, change log, sync), with
//! admin rights. Writes since the last flush are lost when the process
//! crashes. A copy of a shard that lost its place (its machine's lease
//! lapsed, or another machine runs it now) writes nothing more.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::time::{Duration, Instant};

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
/// Rows waiting to be written, on all shards. Past this, new rows are
/// dropped.
pub(super) const MAX_DIRTY_ROWS: usize = 100_000;
/// Flushes a row that fails with a store error is tried in.
const WRITE_ATTEMPTS: u32 = 5;
/// The most call workers (see [`default_workers`]).
const MAX_CALL_WORKERS: usize = 4;
/// How often buffered writes are flushed, unless
/// `PYLON_SHARD_WRITE_EVERY_MS` sets it (at least [`MIN_WRITE_EVERY`]).
pub(super) const WRITE_EVERY: Duration = Duration::from_secs(2);
const MIN_WRITE_EVERY: Duration = Duration::from_millis(100);
/// The delay before a call that failed in the infrastructure is made
/// again; it doubles up to [`RETRY_MAX`].
const RETRY_FIRST: Duration = Duration::from_millis(250);
const RETRY_MAX: Duration = Duration::from_secs(30);
/// A call waiting to be made again that the module stopped sending for this
/// long is forgotten.
const FORGET_AFTER: Duration = Duration::from_secs(60);

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

/// A call a worker took, for one run of a shard.
pub(super) struct QueuedCall {
    shard: String,
    /// The run (see `WasmShardHost::instances`) that sent it: its result
    /// goes to that run only.
    serial: u64,
    call: Call,
}

/// Calls waiting for a worker: a queue per shard, served in turn.
#[derive(Default)]
pub(super) struct CallQueue {
    state: Mutex<QueueState>,
    ready: Condvar,
}

#[derive(Default)]
struct QueueState {
    by_shard: HashMap<String, VecDeque<(u64, Call)>>,
    /// Shards with calls waiting, in the order they are served.
    turn: VecDeque<String>,
    len: usize,
}

impl CallQueue {
    /// False when [`CALL_QUEUE`] calls wait already.
    fn push(&self, shard: &str, serial: u64, call: Call) -> bool {
        let mut s = self.state.lock().unwrap();
        if s.len >= CALL_QUEUE {
            return false;
        }
        let queue = s.by_shard.entry(shard.to_string()).or_default();
        let first = queue.is_empty();
        queue.push_back((serial, call));
        if first {
            s.turn.push_back(shard.to_string());
        }
        s.len += 1;
        self.ready.notify_one();
        true
    }

    /// The next call, from the next shard in turn; `None` after `wait`
    /// with nothing waiting.
    fn pop(&self, wait: Duration) -> Option<QueuedCall> {
        let mut s = self.state.lock().unwrap();
        if s.len == 0 {
            s = self.ready.wait_timeout(s, wait).unwrap().0;
        }
        let shard = s.turn.pop_front()?;
        let queue = s.by_shard.get_mut(&shard)?;
        let (serial, call) = queue.pop_front()?;
        if queue.is_empty() {
            s.by_shard.remove(&shard);
        } else {
            s.turn.push_back(shard.clone());
        }
        s.len -= 1;
        Some(QueuedCall {
            shard,
            serial,
            call,
        })
    }
}

/// Where one call of a shard is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Flight {
    /// A worker has it or will. `attempt` counts earlier tries that failed
    /// in the infrastructure.
    Running { attempt: u32 },
    /// Its result reached the shard's queue by tick `tick`; the module has
    /// it once a later tick starts.
    Answered { tick: u64 },
    /// It failed in the infrastructure; the module's next send after
    /// `until` makes it again.
    Waiting { until: Instant, attempt: u32 },
}

/// The calls of each shard that are in flight, by shard id: the run they
/// belong to, and each call by function and key. A new run of the shard
/// starts with none (it sends its own calls; a mutation's key still runs
/// once).
pub(super) type InFlight = HashMap<String, (u64, HashMap<String, Flight>)>;

/// One buffered field value, with the run of the shard that wrote it.
#[derive(Debug, Clone)]
struct FieldWrite {
    value: serde_json::Value,
    shard: String,
    /// The lease epoch the shard was held under here (None with no shard
    /// directory): the write is made only while that still holds.
    epoch: Option<i64>,
}

/// Fields waiting to be written to one row.
#[derive(Debug, Default)]
pub(super) struct DirtyRow {
    fields: HashMap<String, FieldWrite>,
    failures: u32,
}

/// Buffered writes, by (entity, id).
pub(super) type Dirty = HashMap<(String, String), DirtyRow>;

/// Which buffered rows a flush writes.
pub(super) enum Flush<'a> {
    /// The rows shard `id` wrote last (it is stopping here, in order).
    Shard(&'a str),
    /// The rows of shards this machine holds under a live lease (every row
    /// with no directory).
    Held,
    /// Every row (the machine is leaving, after its final saves).
    All,
}

fn flight_key(call: &Call) -> String {
    format!("{}\u{0}{}", call.function, call.key)
}

fn error_result(key: &str, code: &str, message: &str) -> CallResult {
    CallResult {
        key: key.to_string(),
        ok: false,
        data: Arc::from(format!("{code}: {message}").into_bytes()),
    }
}

/// True for an error of the database or the functions runtime, not of the
/// function: the call is made again, with the same key, after a delay. A
/// mutation runs once per key, so this never runs it twice.
pub(crate) fn retryable(code: &str) -> bool {
    // Postgres refused the statement itself, or the data was invalid: the
    // same call meets it again.
    if code == "PG_REJECTED" || code.starts_with("PG_INVALID") {
        return false;
    }
    ["PG_", "SQLITE_", "TX_", "RUNNER_"]
        .iter()
        .any(|p| code.starts_with(p))
        || matches!(
            code,
            "FN_TIMEOUT"
                | "IO_ERROR"
                | "HANDLER_CRASH"
                | "CALL_CANCELLED"
                | "LOCK_FAILED"
                | "BEGIN_FAILED"
                | "COMMIT_FAILED"
                | "ROLLBACK_FAILED"
        )
}

fn retry_delay(attempt: u32) -> Duration {
    RETRY_FIRST
        .saturating_mul(1 << attempt.min(16))
        .min(RETRY_MAX)
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

/// Call workers when `PYLON_SHARD_CALL_WORKERS` is unset: a quarter of the
/// Postgres pool (each call holds a connection for its whole run), 1 to
/// [`MAX_CALL_WORKERS`]; 1 on SQLite, which writes on one connection.
fn default_workers(runtime: &crate::Runtime) -> usize {
    match runtime.pg_backend() {
        Some(pg) => (pg.store.shared_pool().max_size() / 4).clamp(1, MAX_CALL_WORKERS),
        None => 1,
    }
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
        let workers = env_or(
            "PYLON_SHARD_CALL_WORKERS",
            default_workers(writer.runtime()),
        );
        let _ = self.writer.set(writer);
        for n in 0..workers.max(1) {
            let host = Arc::downgrade(self);
            let queue = Arc::clone(&self.calls);
            let _ = std::thread::Builder::new()
                .name(format!("pylon-shard-calls-{n}"))
                .spawn(move || Self::run_calls(host, queue));
        }
        let host = Arc::downgrade(self);
        let every = Duration::from_millis(env_or(
            "PYLON_SHARD_WRITE_EVERY_MS",
            WRITE_EVERY.as_millis() as u64,
        ))
        .max(MIN_WRITE_EVERY);
        let _ = std::thread::Builder::new()
            .name("pylon-shard-writes".into())
            .spawn(move || loop {
                std::thread::sleep(every);
                let Some(host) = host.upgrade() else { return };
                host.flush_writes(Flush::Held);
            });
    }

    /// Queue the calls a module asked for after a tick (it sends each until
    /// its result arrives). A call in flight for this shard is skipped, as
    /// is one waiting to be made again, one past [`MAX_CALLS_IN_FLIGHT`],
    /// and one that finds the queue full: the module sends them again.
    pub(super) fn take_calls(&self, shard: &str, calls: Vec<Call>) {
        let Some(target) = self.registry.get(shard) else {
            return;
        };
        let Some(serial) = self.instances.lock().unwrap().get(shard).copied() else {
            return;
        };
        let tick = target.tick_number();
        let now = Instant::now();
        let mut in_flight = self.calls_in_flight.lock().unwrap();
        let mut flights = match in_flight.remove(shard) {
            Some((run, flights)) if run == serial => flights,
            // Another run's calls: this run sends its own.
            _ => HashMap::new(),
        };
        // A result queued before this tick started reached the module at
        // its start; a call the module stopped sending is forgotten.
        flights.retain(|_, f| match *f {
            Flight::Running { .. } => true,
            Flight::Answered { tick: t } => t >= tick,
            Flight::Waiting { until, .. } => now < until + FORGET_AFTER,
        });
        let no_functions = matches!(self.functions.get(), Some(None));
        let mut running = flights
            .values()
            .filter(|f| matches!(f, Flight::Running { .. }))
            .count();
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
            if no_functions {
                target.push_call_result(error_result(
                    &call.key,
                    "NO_FUNCTIONS",
                    "the app runs no functions",
                ));
                continue;
            }
            let key = flight_key(&call);
            let attempt = match flights.get(&key) {
                Some(Flight::Running { .. }) | Some(Flight::Answered { .. }) => continue,
                Some(Flight::Waiting { until, .. }) if now < *until => continue,
                Some(Flight::Waiting { attempt, .. }) => *attempt,
                None => 0,
            };
            if running >= MAX_CALLS_IN_FLIGHT {
                continue;
            }
            if !self.calls.push(shard, serial, call) {
                tracing::warn!(
                    "[shard {shard}] the call queue is full; the module sends the call again"
                );
                break;
            }
            flights.insert(key, Flight::Running { attempt });
            running += 1;
        }
        if !flights.is_empty() {
            in_flight.insert(shard.to_string(), (serial, flights));
        }
    }

    fn run_calls(host: Weak<Self>, queue: Arc<CallQueue>) {
        loop {
            let next = queue.pop(Duration::from_secs(1));
            let Some(host) = host.upgrade() else { return };
            if let Some(QueuedCall {
                shard,
                serial,
                call,
            }) = next
            {
                host.make_call(&shard, serial, call);
            }
        }
    }

    /// Make one call and settle it: deliver its result, or, after an
    /// infrastructure failure, leave it waiting to be made again.
    fn make_call(&self, shard: &str, serial: u64, call: Call) {
        let flight = flight_key(&call);
        let key = call.key.clone();
        let function = call.function.clone();
        let outcome = match self
            .functions
            .get()
            .cloned()
            .flatten()
            .and_then(|w| w.upgrade())
        {
            Some(functions) => std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                functions.call_once(&call.function, call.args, shard_auth(), &call.key)
            }))
            .unwrap_or_else(|_| {
                Err(pylon_functions::runner::FnCallError {
                    code: "CALL_PANICKED".into(),
                    message: format!("calling {function} panicked in the server"),
                })
            }),
            None => Err(pylon_functions::runner::FnCallError {
                code: "NO_FUNCTIONS".into(),
                message: "the app runs no functions".into(),
            }),
        };
        let result = match outcome {
            Ok(value) => CallResult {
                key,
                ok: true,
                data: Arc::from(value.to_string().into_bytes()),
            },
            Err(e) if retryable(&e.code) => {
                let mut in_flight = self.calls_in_flight.lock().unwrap();
                if let Some(f) = in_flight
                    .get_mut(shard)
                    .filter(|(run, _)| *run == serial)
                    .and_then(|(_, m)| m.get_mut(&flight))
                {
                    let attempt = match *f {
                        Flight::Running { attempt } => attempt,
                        _ => 0,
                    };
                    let delay = retry_delay(attempt);
                    tracing::warn!(
                        "[shard {shard}] call {function} {key} failed ({}: {}); trying again in {delay:?}",
                        e.code,
                        e.message
                    );
                    *f = Flight::Waiting {
                        until: Instant::now() + delay,
                        attempt: attempt + 1,
                    };
                }
                return;
            }
            Err(e) => error_result(&key, &e.code, &e.message),
        };
        // The result is in the shard's queue before the call counts as
        // answered, and the answer is kept until a later tick starts: the
        // module never sends a call again while its result is on the way.
        // It goes to the run that sent the call, never to a later one.
        let target = self
            .registry
            .get(shard)
            .filter(|_| self.instances.lock().unwrap().get(shard) == Some(&serial));
        let pushed = target.as_ref().is_some_and(|t| t.push_call_result(result));
        let mut in_flight = self.calls_in_flight.lock().unwrap();
        let Some(flights) = in_flight
            .get_mut(shard)
            .filter(|(run, _)| *run == serial)
            .map(|(_, f)| f)
        else {
            return;
        };
        match (pushed, target) {
            (true, Some(t)) => {
                flights.insert(
                    flight,
                    Flight::Answered {
                        tick: t.tick_number(),
                    },
                );
            }
            // Stopped, or its queue is full: it sends the call again (here
            // or where it runs next) and gets the same result.
            _ => {
                flights.remove(&flight);
                if flights.is_empty() {
                    in_flight.remove(shard);
                }
            }
        }
    }

    /// Forget a stopped shard's calls. A worker still making one delivers
    /// nothing: its run is gone.
    pub(super) fn forget_calls(&self, shard: &str) {
        self.calls_in_flight.lock().unwrap().remove(shard);
    }

    /// Buffer the writes a module asked for after a tick: each field keeps
    /// its latest value, whichever shard wrote it, with that shard's run.
    pub(super) fn take_writes(&self, shard: &str, writes: Vec<Write>) {
        // A tick of a copy removed meanwhile: its writes are not current.
        if self.registry.get(shard).is_none() {
            return;
        }
        let epoch = match self.cluster.get() {
            Some(c) => match c.owned.lock().unwrap().get(shard) {
                Some(&epoch) => Some(epoch),
                // Not held here (fenced, or placed elsewhere): not ours.
                None => return,
            },
            None => None,
        };
        let mut dirty = self.dirty.lock().unwrap();
        let mut dropped = 0usize;
        for w in writes {
            if w.entity.is_empty() || w.id.is_empty() || w.set.is_empty() {
                continue;
            }
            let key = (w.entity, w.id);
            if !dirty.contains_key(&key) && dirty.len() >= MAX_DIRTY_ROWS {
                dropped += 1;
                continue;
            }
            let row = dirty.entry(key).or_default();
            for (field, value) in w.set {
                row.fields.insert(
                    field,
                    FieldWrite {
                        value,
                        shard: shard.to_string(),
                        epoch,
                    },
                );
            }
        }
        if dropped > 0 {
            tracing::warn!(
                "[shard {shard}] {dropped} write(s) dropped: {MAX_DIRTY_ROWS} rows already wait"
            );
        }
    }

    /// Drop the fields shard `id` wrote: a copy that lost its place.
    pub(super) fn discard_writes(&self, id: &str) {
        let mut dirty = self.dirty.lock().unwrap();
        for row in dirty.values_mut() {
            row.fields.retain(|_, f| f.shard != id);
        }
        dirty.retain(|_, row| !row.fields.is_empty());
    }

    /// Write the buffered fields `scope` names, each with the fence of the
    /// shard run that wrote it (see [`crate::entity_writer::Fence`]). One
    /// flush at a time, so a later value is never overwritten by an earlier
    /// one. Fields that fail with a store error are kept (under newer
    /// values) for [`WRITE_ATTEMPTS`] flushes; ones refused (missing row,
    /// plugin, validation, a shard no longer held) are dropped with a log
    /// line. Returns the number of field groups kept after a failure.
    pub(super) fn flush_writes(&self, scope: Flush<'_>) -> usize {
        let Some(writer) = self.writer.get() else {
            return 0;
        };
        let _one = self.flush_lock.lock().unwrap();
        let machine = self.cluster.get().map(|c| c.me.id.clone());
        let held: Option<HashMap<String, i64>> = match (&scope, self.cluster.get()) {
            (Flush::Held, Some(c)) => {
                if c.current_epoch().is_none() {
                    return 0;
                }
                Some(c.owned.lock().unwrap().clone())
            }
            _ => None,
        };
        let wanted = |f: &FieldWrite| match &scope {
            Flush::Shard(id) => f.shard == *id,
            Flush::Held => held
                .as_ref()
                .is_none_or(|h| f.epoch.is_none() || h.get(&f.shard) == f.epoch.as_ref()),
            Flush::All => true,
        };
        // Taken out of the buffer, grouped by row and by the run that wrote
        // them: (entity, id, shard, epoch) -> fields.
        type Group = ((String, String), String, Option<i64>);
        let mut groups: HashMap<Group, (serde_json::Map<String, serde_json::Value>, u32)> =
            HashMap::new();
        {
            let mut dirty = self.dirty.lock().unwrap();
            for (key, row) in dirty.iter_mut() {
                let names: Vec<String> = row
                    .fields
                    .iter()
                    .filter(|(_, f)| wanted(f))
                    .map(|(n, _)| n.clone())
                    .collect();
                for name in names {
                    let f = row.fields.remove(&name).expect("listed above");
                    let group = groups
                        .entry((key.clone(), f.shard, f.epoch))
                        .or_insert_with(|| (serde_json::Map::new(), row.failures));
                    group.0.insert(name, f.value);
                }
            }
            dirty.retain(|_, row| !row.fields.is_empty());
        }
        let mut failed: Vec<(Group, serde_json::Map<String, serde_json::Value>, u32)> = Vec::new();
        for (((entity, id), shard, epoch), (set, failures)) in groups {
            let fence = match (epoch, machine.as_deref()) {
                (Some(epoch), Some(machine)) => Some(crate::entity_writer::Fence {
                    shard: &shard,
                    machine,
                    epoch,
                }),
                _ => None,
            };
            let fields = serde_json::Value::Object(set.clone());
            match writer.update_fenced(&entity, &id, &fields, fence) {
                Ok(()) => {}
                Err(crate::entity_writer::WriteError::Refused(why)) => {
                    tracing::warn!("[shard {shard}] write to {entity} {id} refused: {why}");
                }
                Err(crate::entity_writer::WriteError::Store(why)) => {
                    if failures + 1 >= WRITE_ATTEMPTS {
                        tracing::warn!(
                            "[shard {shard}] write to {entity} {id} failed {WRITE_ATTEMPTS} times; dropped: {why}"
                        );
                    } else {
                        tracing::warn!(
                            "[shard {shard}] write to {entity} {id} failed; kept: {why}"
                        );
                        failed.push((((entity, id), shard, epoch), set, failures + 1));
                    }
                }
            }
        }
        let kept = failed.len();
        if failed.is_empty() {
            return 0;
        }
        let mut dirty = self.dirty.lock().unwrap();
        for ((key, shard, epoch), set, failures) in failed {
            let row = dirty.entry(key).or_default();
            // Values buffered since the flush took these are newer.
            for (field, value) in set {
                row.fields.entry(field).or_insert_with(|| FieldWrite {
                    value,
                    shard: shard.clone(),
                    epoch,
                });
            }
            row.failures = row.failures.max(failures);
        }
        kept
    }

    /// Rows with a field shard `shard` wrote waiting, for tests.
    #[cfg(test)]
    pub(super) fn dirty_rows(&self, shard: &str) -> usize {
        self.dirty
            .lock()
            .unwrap()
            .values()
            .filter(|r| r.fields.values().any(|f| f.shard == shard))
            .count()
    }
}

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
        /// Other users' characters, by user id.
        by_user: Mutex<HashMap<String, String>>,
        stored: Mutex<HashMap<(String, String), serde_json::Value>>,
        runs: Mutex<HashMap<String, usize>>,
        /// grantItem fails this many more times in the database.
        db_failures: Mutex<u32>,
        /// Keys grantItem answers KEY_REUSED for.
        reused: Mutex<Vec<String>>,
        /// loadCharacter waits while this is set.
        hold_loads: Mutex<bool>,
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

        fn hold_loads(&self, hold: bool) {
            *self.hold_loads.lock().unwrap() = hold;
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
                    // Each load reads a newer row: x is 7, then 8, ...
                    let n = {
                        let mut runs = self.runs.lock().unwrap();
                        let n = runs.entry(fn_name.into()).or_default();
                        *n += 1;
                        *n
                    };
                    let mut hold = self.hold_loads.lock().unwrap();
                    while *hold {
                        hold = self.released.wait(hold).unwrap();
                    }
                    drop(hold);
                    let user = args["userId"].as_str().unwrap_or_default();
                    let id = match user {
                        "p1" => self.character.clone(),
                        other => self.by_user.lock().unwrap()[other].clone(),
                    };
                    Ok(serde_json::json!({
                        "id": id, "x": 6 + n as i64, "nextGrant": 3,
                        "items": [{ "id": "i-old", "name": "old" }]
                    }))
                }
                "grantItem" => {
                    match args["item"].as_str() {
                        Some("panic") => panic!("a bug in the server"),
                        Some("cursed") => {
                            return Err(FnCallError {
                                code: "REFUSED".into(),
                                message: "cursed items cannot be granted".into(),
                            })
                        }
                        _ => {}
                    }
                    if self.reused.lock().unwrap().iter().any(|k| k == key) {
                        return Err(FnCallError {
                            code: "KEY_REUSED".into(),
                            message: "other arguments".into(),
                        });
                    }
                    {
                        let mut failures = self.db_failures.lock().unwrap();
                        if *failures > 0 {
                            *failures -= 1;
                            *self.runs.lock().unwrap().entry(fn_name.into()).or_default() += 1;
                            return Err(FnCallError {
                                code: "PG_TX_QUERY_FAILED".into(),
                                message: "connection reset".into(),
                            });
                        }
                    }
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
        wait_for("the calls in flight to clear", || {
            host.calls_in_flight.lock().unwrap().is_empty()
        });

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
        wait_for("the calls in flight to clear", || {
            host.calls_in_flight.lock().unwrap().is_empty()
        });
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

    /// Join p1 in a new zone and wait for the character.
    fn joined(fns: &Arc<Fns>, rt: &Arc<crate::Runtime>, zone: &str) -> Arc<WasmShardHost> {
        let host = host();
        let weak: Weak<dyn pylon_router::FnOps> =
            Arc::downgrade(&(Arc::clone(fns) as Arc<dyn pylon_router::FnOps>));
        host.attach_data(Some(weak), writer(rt));
        host.create("zone", zone, &serde_json::json!({})).unwrap();
        input(&host, zone, "p1", "\"join\"");
        wait_for("the character to load", || {
            saved(&host, zone)["players"]["p1"]["loaded"] == true
        });
        host
    }

    fn items(host: &WasmShardHost, zone: &str) -> Vec<String> {
        saved(host, zone)["players"]["p1"]["items"]
            .as_object()
            .unwrap()
            .values()
            .map(|v| v.as_str().unwrap().to_string())
            .collect()
    }

    /// A call that fails in the database is made again with the same key;
    /// the module never sees the failure. A function's own error, and a
    /// panic in the server, reach the module; the worker lives on.
    #[test]
    fn infrastructure_failures_are_retried_and_answers_are_delivered() {
        let (rt, fns) = runtime();
        *fns.db_failures.lock().unwrap() = 2;
        let host = joined(&fns, &rt, "r1");
        input(&host, "r1", "p1", r#"{"loot":{"item":"shield"}}"#);
        wait_for("the shield", || {
            items(&host, "r1").contains(&"shield".to_string())
        });
        assert_eq!(fns.runs("grantItem"), 3);

        input(&host, "r1", "p1", r#"{"loot":{"item":"cursed"}}"#);
        input(&host, "r1", "p1", r#"{"loot":{"item":"panic"}}"#);
        wait_for("both answers", || {
            saved(&host, "r1")["grants"] == serde_json::json!({})
        });
        assert_eq!(items(&host, "r1"), ["old", "shield"]);

        // The workers still answer.
        input(&host, "r1", "p1", r#"{"loot":{"item":"bow"}}"#);
        wait_for("the bow", || {
            items(&host, "r1").contains(&"bow".to_string())
        });
        std::thread::sleep(Duration::from_millis(100));
        let in_flight = host.calls_in_flight.lock().unwrap();
        assert!(in_flight
            .values()
            .flat_map(|(_, m)| m.values())
            .all(|f| !matches!(f, Flight::Running { .. } | Flight::Waiting { .. })));
        drop(in_flight);
        host.stop_all();
    }

    /// A grant whose key another grant used is sent again under the next
    /// key.
    #[test]
    fn a_reused_key_moves_the_grant_to_the_next_key() {
        let (rt, fns) = runtime();
        fns.reused
            .lock()
            .unwrap()
            .push(format!("loot:{}:3", fns.character));
        let host = joined(&fns, &rt, "k1");
        input(&host, "k1", "p1", r#"{"loot":{"item":"ring"}}"#);
        wait_for("the ring", || {
            items(&host, "k1").contains(&"ring".to_string())
        });
        let key = format!("loot:{}:4", fns.character);
        assert!(fns
            .stored
            .lock()
            .unwrap()
            .contains_key(&("grantItem".to_string(), key)));
        assert_eq!(saved(&host, "k1")["players"]["p1"]["next_grant"], 5);
        host.stop_all();
    }

    /// A call of a run that stopped delivers nothing to the next run of the
    /// shard, which sends its own: an old load (x 7) never overwrites the
    /// new run's (x 8).
    #[test]
    fn a_result_goes_only_to_the_run_that_sent_the_call() {
        let (rt, fns) = runtime();
        let host = host();
        let weak: Weak<dyn pylon_router::FnOps> =
            Arc::downgrade(&(Arc::clone(&fns) as Arc<dyn pylon_router::FnOps>));
        host.attach_data(Some(weak), writer(&rt));
        fns.hold_loads(true);
        host.create("zone", "s1", &serde_json::json!({})).unwrap();
        input(&host, "s1", "p1", "\"join\"");
        wait_for("the first load to start", || fns.runs("loadCharacter") == 1);
        // A new run of s1 while the first run's load waits.
        host.stop("s1");
        host.create("zone", "s1", &serde_json::json!({})).unwrap();
        input(&host, "s1", "p1", "\"join\"");
        std::thread::sleep(Duration::from_millis(200));
        fns.hold_loads(false);
        wait_for("the new run's character", || {
            saved(&host, "s1")["players"]["p1"]["loaded"] == true
        });
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(fns.runs("loadCharacter"), 2);
        assert_eq!(saved(&host, "s1")["players"]["p1"]["x"], 8);
        host.stop_all();
    }

    /// Each field belongs to the shard that wrote it: discarding one
    /// shard's fields keeps another shard's fields of the same row.
    #[test]
    fn discarding_a_shard_keeps_the_other_shards_fields_of_a_row() {
        let (rt, fns) = runtime();
        let host = joined(&fns, &rt, "f1");
        host.create("zone", "f2", &serde_json::json!({})).unwrap();
        let set = |field: &str, n: i64| {
            vec![Write {
                entity: "Character".into(),
                id: fns.character.clone(),
                set: serde_json::Map::from_iter([(field.to_string(), serde_json::json!(n))]),
            }]
        };
        host.take_writes("f1", set("x", 5));
        host.take_writes("f2", set("nextGrant", 9));
        host.discard_writes("f1");
        host.flush_writes(Flush::All);
        let row = rt.get_by_id("Character", &fns.character).unwrap().unwrap();
        assert_eq!(
            (row["x"].clone(), row["nextGrant"].clone()),
            (serde_json::json!(0), serde_json::json!(9))
        );
        host.stop_all();
    }

    /// Shutdown on a cluster: players keep moving while the machine stops,
    /// and each row ends with the x of its zone's final state (no tick's
    /// write is lost between the shard leaving `owned` and its pause).
    #[test]
    fn shutdown_writes_every_tick_of_every_zone() {
        let Ok(url) = std::env::var("PYLON_TEST_PG_URL") else {
            eprintln!("skipping: PYLON_TEST_PG_URL not set");
            return;
        };
        let _serial = crate::shard_cluster::tests::DIRECTORY_TESTS
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let (rt, fns) = runtime();
        let pool =
            pylon_storage::pg_datastore::PgPool::connect(&url, 4, Duration::from_secs(5)).unwrap();
        let run = pylon_cluster::new_instance_id();
        // A kind of its own (the cluster-wide limit counts only this run),
        // and its placements gone when the test ends.
        let kind = format!("zone-{run}");
        let host = WasmShardHost::new(vec![WasmShardKind::compile(
            &kind,
            include_bytes!("../../../../examples/shard-arena/shards/zone.wasm"),
            ShardConfig::default(),
            WasmLimits::default(),
        )
        .unwrap()]);
        struct Forget(Arc<pylon_storage::pg_datastore::PgPool>, String);
        impl Drop for Forget {
            fn drop(&mut self) {
                let _ = self.0.with_client(|c| {
                    c.execute(
                        "DELETE FROM _pylon_shard_placements WHERE kind = $1",
                        &[&self.1],
                    )
                });
            }
        }
        let _forget = Forget(Arc::clone(&pool), kind.clone());
        let me = format!("m-{run}");
        host.attach_cluster(
            crate::shard_cluster::PgShardDirectory::open(pool).unwrap(),
            crate::shard_cluster::MachineConfig {
                id: me.clone(),
                address: None,
                capacity: 10,
                fly_instance: None,
            },
        );
        let weak: Weak<dyn pylon_router::FnOps> =
            Arc::downgrade(&(Arc::clone(&fns) as Arc<dyn pylon_router::FnOps>));
        host.attach_data(Some(weak), writer(&rt));
        let zones: Vec<(String, String, String)> = (0..12)
            .map(|i| {
                let user = format!("u{i}");
                let character = rt
                    .insert(
                        "Character",
                        &serde_json::json!({ "userId": user, "x": 0, "nextGrant": 0 }),
                    )
                    .unwrap();
                fns.by_user
                    .lock()
                    .unwrap()
                    .insert(user.clone(), character.clone());
                (format!("sd{i}-{run}"), user, character)
            })
            .collect();
        let deadline = Instant::now() + Duration::from_secs(10);
        for (zone, user, _) in &zones {
            while host
                .create_on(&kind, zone, &serde_json::json!({}), Some(&me))
                .is_err()
            {
                assert!(Instant::now() < deadline, "no lease");
                std::thread::sleep(Duration::from_millis(50));
            }
            input(&host, zone, user, "\"join\"");
        }
        for (zone, user, _) in &zones {
            wait_for("the characters", || {
                saved(&host, zone)["players"][user]["loaded"] == true
            });
        }
        let shards: Vec<_> = zones
            .iter()
            .map(|(z, _, _)| host.registry.get(z).unwrap())
            .collect();
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mover = {
            let (host, zones, stop) = (Arc::clone(&host), zones.clone(), Arc::clone(&stop));
            std::thread::spawn(move || {
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    for (zone, user, _) in &zones {
                        if let Some(s) = host.registry.get(zone) {
                            let _ = s.push_input(
                                SubscriberId::new(user.as_str()),
                                RawInput::new(
                                    SnapshotFormat::Json,
                                    br#"{"move":{"dx":1}}"#.to_vec(),
                                ),
                                None,
                            );
                        }
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
            })
        };
        std::thread::sleep(Duration::from_millis(300));
        host.stop_all();
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        mover.join().unwrap();
        for ((zone, user, character), shard) in zones.iter().zip(&shards) {
            let x = shard.with_state(|sim| {
                let saved: serde_json::Value =
                    serde_json::from_slice(&sim.save().unwrap().unwrap()).unwrap();
                saved["players"][user]["x"].clone()
            });
            assert_eq!(
                rt.get_by_id("Character", character).unwrap().unwrap()["x"],
                x,
                "{zone}: the row lost the last moves"
            );
        }
    }

    /// A held zone that ended (stopped with no stop_local, as the idle stop
    /// does) writes its last fields when the sweep removes it, before its
    /// placement is released: the fence still finds it ours. All inside the
    /// first 2 s, before the periodic flush.
    #[test]
    fn a_shard_the_sweep_ends_writes_its_last_fields() {
        let Ok(url) = std::env::var("PYLON_TEST_PG_URL") else {
            eprintln!("skipping: PYLON_TEST_PG_URL not set");
            return;
        };
        let _serial = crate::shard_cluster::tests::DIRECTORY_TESTS
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let (rt, fns) = runtime();
        let pool =
            pylon_storage::pg_datastore::PgPool::connect(&url, 4, Duration::from_secs(5)).unwrap();
        let run = pylon_cluster::new_instance_id();
        let kind = format!("idle-{run}");
        let host = WasmShardHost::new(vec![WasmShardKind::compile(
            &kind,
            include_bytes!("../../../../examples/shard-arena/shards/zone.wasm"),
            ShardConfig::default(),
            WasmLimits::default(),
        )
        .unwrap()]);
        let me = format!("m-{run}");
        host.attach_cluster(
            crate::shard_cluster::PgShardDirectory::open(Arc::clone(&pool)).unwrap(),
            crate::shard_cluster::MachineConfig {
                id: me.clone(),
                address: None,
                capacity: 10,
                fly_instance: None,
            },
        );
        let weak: Weak<dyn pylon_router::FnOps> =
            Arc::downgrade(&(Arc::clone(&fns) as Arc<dyn pylon_router::FnOps>));
        host.attach_data(Some(weak), writer(&rt));
        let zone = format!("idle-{run}");
        let deadline = Instant::now() + Duration::from_secs(10);
        while host
            .create_on(&kind, &zone, &serde_json::json!({}), Some(&me))
            .is_err()
        {
            assert!(Instant::now() < deadline, "no lease");
            std::thread::sleep(Duration::from_millis(50));
        }
        host.take_writes(
            &zone,
            vec![Write {
                entity: "Character".into(),
                id: fns.character.clone(),
                set: serde_json::Map::from_iter([("x".to_string(), serde_json::json!(33))]),
            }],
        );
        let started = Instant::now();
        host.registry.get(&zone).unwrap().stop();
        host.sweep();
        assert!(host.registry.get(&zone).is_none());
        let dir = crate::shard_cluster::PgShardDirectory::open(pool).unwrap();
        assert!(dir.placement(&zone).unwrap().is_none(), "not released");
        assert_eq!(
            rt.get_by_id("Character", &fns.character).unwrap().unwrap()["x"],
            33
        );
        assert!(started.elapsed() < WRITE_EVERY);
        host.stop_all();
    }

    #[test]
    fn retryable_codes_are_the_infrastructure_ones() {
        for code in [
            "PG_TX_QUERY_FAILED",
            "PG_POOL_TIMEOUT",
            "SQLITE_BUSY",
            "SQLITE_IOERR",
            "RUNNER_EXITED",
            "FN_TIMEOUT",
            "BEGIN_FAILED",
            "COMMIT_FAILED",
        ] {
            assert!(retryable(code), "{code}");
        }
        for code in [
            "NOT_FOUND",
            "FORBIDDEN",
            "KEY_REUSED",
            "FN_NOT_FOUND",
            "CALL_PANICKED",
            "PG_REJECTED",
            "PG_INVALID_DATA",
            "PG_INVALID_UPDATE",
            "PG_INVALID_ID",
            "INSERT_FAILED",
        ] {
            assert!(!retryable(code), "{code}");
        }
        assert_eq!(retry_delay(0), RETRY_FIRST);
        assert_eq!(retry_delay(40), RETRY_MAX);
    }

    /// Workers take one call of each shard in turn.
    #[test]
    fn the_call_queue_serves_shards_in_turn() {
        let q = CallQueue::default();
        let call = |k: &str| Call {
            key: k.into(),
            function: "f".into(),
            args: serde_json::Value::Null,
        };
        for k in ["a1", "a2", "a3"] {
            assert!(q.push("a", 1, call(k)));
        }
        assert!(q.push("b", 2, call("b1")));
        let order: Vec<String> = std::iter::from_fn(|| q.pop(Duration::ZERO))
            .map(|c| c.call.key)
            .collect();
        assert_eq!(order, ["a1", "b1", "a2", "a3"]);
    }

    /// A row keeps the latest value whichever shard wrote it, belongs to
    /// the shard that wrote it last, and a discarded shard's rows are not
    /// written.
    #[test]
    fn writes_keep_the_latest_value_across_shards_and_discarded_rows_are_dropped() {
        let (rt, fns) = runtime();
        let host = joined(&fns, &rt, "w1");
        host.create("zone", "w2", &serde_json::json!({})).unwrap();
        let x = |n: i64| {
            vec![Write {
                entity: "Character".into(),
                id: fns.character.clone(),
                set: serde_json::Map::from_iter([("x".to_string(), serde_json::json!(n))]),
            }]
        };
        host.take_writes("w1", x(5));
        host.take_writes("w2", x(9));
        assert_eq!((host.dirty_rows("w1"), host.dirty_rows("w2")), (0, 1));
        host.flush_writes(Flush::Shard("w1"));
        assert_eq!(
            rt.get_by_id("Character", &fns.character).unwrap().unwrap()["x"],
            0
        );
        host.flush_writes(Flush::Held);
        assert_eq!(
            rt.get_by_id("Character", &fns.character).unwrap().unwrap()["x"],
            9
        );

        host.take_writes("w2", x(11));
        host.discard_writes("w2");
        host.stop("w2");
        assert_eq!(
            rt.get_by_id("Character", &fns.character).unwrap().unwrap()["x"],
            9
        );
        // A stopped copy's late writes are not taken.
        host.take_writes("w2", x(12));
        assert_eq!(host.dirty_rows("w2"), 0);
        host.stop_all();
    }
}
