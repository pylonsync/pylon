//! Shards whose simulation is a WebAssembly module the app ships.
//!
//! An app declares shard kinds in `app.ts` (`shards: [shard({ name, wasm })]`)
//! and builds each module with the `pylon-shard-guest` crate, or with any
//! toolchain that implements its ABI (see that crate's docs). The stock
//! `pylon` binary compiles the modules at boot. `ctx.shards.create` in a
//! function starts one instance per shard id.
//!
//! Each shard owns one module instance. The instance runs on the shard's
//! tick thread through [`WasmSim`], an ordinary [`SimState`], so the tick
//! loop, input limits, outbound queues, and transports are the same as for
//! a shard written in Rust.
//!
//! Limits:
//! - Each tick (its inputs, `tick`, and the snapshots) has one time budget,
//!   and so does each authorize call and init. A module that runs past it
//!   traps, and the shard stops.
//! - Each instance has a memory cap. A module that grows past it traps.
//! - A module may import only `pylon.log`. It gets no clock, randomness,
//!   files, or network, so the same inputs replay to the same state.
//!   NaN bit patterns are canonicalized, so floats agree across CPUs.
//! - A trap stops the shard and is logged with the module's last error line
//!   (its panic message).

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock, Weak};
use std::time::{Duration, Instant};

use pylon_realtime::{
    EntityId, EntityPos, InterestArea, InterestConfig, Plane, RawInput, RawSnapshot, Replicated,
    ReplicatedRef, ReplicationConfig, Shard, ShardAuth, ShardConfig, ShardRegistry, SimState,
    SnapshotFormat, SubscriberId, Visibility,
};
use serde::Serialize;
use wasmtime::{
    Caller, Config, Engine, Linker, Memory, Module, Store, StoreLimits, StoreLimitsBuilder, Trap,
    TypedFunc, UpdateDeadline,
};

/// The guest ABI version this host speaks.
pub const ABI_VERSION: i32 = 1;

/// How often the engine's epoch advances: how often a running call checks
/// its deadline.
const EPOCH_TICK: Duration = Duration::from_millis(2);

/// Native stack a guest call may use.
const MAX_WASM_STACK: usize = 128 << 10;

/// Longest guest log line kept, in bytes.
const MAX_LOG_LINE: usize = 2048;

const STATUS_OK: i32 = 0;
const STATUS_ERR: i32 = 1;
const STATUS_SAME_AS_BROADCAST: i32 = 2;

/// The process-wide engine, and the thread that advances its epoch.
fn engine() -> &'static Engine {
    static ENGINE: OnceLock<Engine> = OnceLock::new();
    ENGINE.get_or_init(|| {
        let mut config = Config::new();
        config.epoch_interruption(true);
        config.cranelift_nan_canonicalization(true);
        config.relaxed_simd_deterministic(true);
        config.wasm_multi_memory(false);
        config.wasm_memory64(false);
        // Guest calls run on threads with 512 KiB stacks (HTTP workers call
        // the authorize hooks). Keep the guest's share well under that, so a
        // deep recursion traps instead of overflowing the native stack.
        config.max_wasm_stack(MAX_WASM_STACK);
        let engine = Engine::new(&config).expect("wasmtime engine config is valid");
        let ticker = engine.weak();
        std::thread::Builder::new()
            .name("pylon-wasm-epoch".into())
            .spawn(move || {
                while let Some(engine) = ticker.upgrade() {
                    engine.increment_epoch();
                    drop(engine);
                    std::thread::sleep(EPOCH_TICK);
                }
            })
            .expect("spawn the wasm epoch thread");
        engine
    })
}

// ---------------------------------------------------------------------------
// Kinds: one compiled module per declared shard kind
// ---------------------------------------------------------------------------

/// Limits for one shard kind's instances.
#[derive(Debug, Clone)]
pub struct WasmLimits {
    /// Linear memory cap per instance.
    pub memory_bytes: usize,
    /// Time budget for one tick (its inputs, `tick`, and the snapshots), or
    /// for one authorize call or init. Past it the module traps.
    pub budget: Duration,
    /// Instances of this kind that may run at once.
    pub max_instances: usize,
    /// Log lines one instance may write per second (burst: 4x).
    pub log_lines_per_sec: f64,
    /// Stop a shard that has had no subscribers for this long. Zero never
    /// stops it.
    pub idle_shutdown: Duration,
}

impl Default for WasmLimits {
    fn default() -> Self {
        Self {
            memory_bytes: 64 << 20,
            budget: Duration::from_millis(100),
            max_instances: 64,
            log_lines_per_sec: 20.0,
            idle_shutdown: Duration::from_secs(90),
        }
    }
}

/// A shard kind: a compiled module plus the config its shards run with.
pub struct WasmShardKind {
    name: String,
    module: Module,
    linker: Linker<HostState>,
    config: ShardConfig,
    limits: WasmLimits,
}

const REQUIRED_EXPORTS: &[&str] = &[
    "memory",
    "pylon_shard_abi",
    "pylon_scratch",
    "pylon_output_ptr",
    "pylon_output_len",
    "pylon_init",
    "pylon_apply_input",
    "pylon_tick",
    "pylon_snapshot",
    "pylon_snapshot_for",
    "pylon_is_finished",
    "pylon_authorize_subscribe",
    "pylon_authorize_input",
];

/// Interest management exports: all or none (see pylon-shard-guest).
const INTEREST_EXPORTS: &[&str] = &[
    "pylon_interest",
    "pylon_entities",
    "pylon_interest_area",
    "pylon_filter_visible",
    "pylon_snapshot_visible",
];

impl WasmShardKind {
    /// Compile and check a module. Fails on a codec other than JSON or
    /// MessagePack, an import other than `pylon.log`, or a missing export.
    pub fn compile(
        name: &str,
        wasm: &[u8],
        config: ShardConfig,
        limits: WasmLimits,
    ) -> Result<Self, String> {
        codec_id(config.snapshot_format)?;
        let engine = engine();
        let module = Module::from_binary(engine, wasm)
            .map_err(|e| format!("shard \"{name}\": invalid WebAssembly module: {e:#}"))?;
        for import in module.imports() {
            if (import.module(), import.name()) != ("pylon", "log") {
                return Err(format!(
                    "shard \"{name}\": the module imports {}.{}; a shard module may import only pylon.log \
                     (build for wasm32-unknown-unknown, not WASI)",
                    import.module(),
                    import.name()
                ));
            }
        }
        let exported: Vec<&str> = module.exports().map(|e| e.name()).collect();
        let missing: Vec<&str> = REQUIRED_EXPORTS
            .iter()
            .copied()
            .filter(|n| !exported.contains(n))
            .collect();
        if !missing.is_empty() {
            return Err(format!(
                "shard \"{name}\": the module is missing exports {}; build it with pylon-shard-guest's export_shard!",
                missing.join(", ")
            ));
        }
        let interest_present: Vec<&str> = INTEREST_EXPORTS
            .iter()
            .copied()
            .filter(|n| exported.contains(n))
            .collect();
        if !interest_present.is_empty() && interest_present.len() != INTEREST_EXPORTS.len() {
            return Err(format!(
                "shard \"{name}\": the module exports {} but not all of {}",
                interest_present.join(", "),
                INTEREST_EXPORTS.join(", ")
            ));
        }
        let mut linker = Linker::new(engine);
        linker
            .func_wrap("pylon", "log", guest_log)
            .map_err(|e| format!("shard \"{name}\": {e:#}"))?;
        Ok(Self {
            name: name.to_string(),
            module,
            linker,
            config,
            limits,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn config(&self) -> &ShardConfig {
        &self.config
    }

    pub fn limits(&self) -> &WasmLimits {
        &self.limits
    }

    /// Instantiate the module and run `pylon_init` with `params`.
    pub fn instantiate(
        &self,
        shard_id: &str,
        params: &serde_json::Value,
    ) -> Result<WasmSim, String> {
        let codec = codec_id(self.config.snapshot_format)?;
        let limits = StoreLimitsBuilder::new()
            .memory_size(self.limits.memory_bytes)
            .memories(1)
            .instances(1)
            .tables(4)
            .table_elements(100_000)
            .trap_on_grow_failure(true)
            .build();
        let mut store = Store::new(
            engine(),
            HostState {
                limits,
                shard_id: shard_id.to_string(),
                log_budget: LogBudget::new(self.limits.log_lines_per_sec),
                last_error_line: None,
                deadline: Instant::now() + self.limits.budget,
            },
        );
        store.limiter(|s| &mut s.limits);
        // The epoch thread only wakes the check. The deadline itself is wall
        // time: on a loaded machine the thread's 2 ms sleeps run long, and a
        // budget counted in epoch ticks would stretch with them.
        store.epoch_deadline_callback(|ctx| {
            Ok(if Instant::now() >= ctx.data().deadline {
                UpdateDeadline::Interrupt
            } else {
                UpdateDeadline::Continue(1)
            })
        });
        store.set_epoch_deadline(1);

        let instance = self
            .linker
            .instantiate(&mut store, &self.module)
            .map_err(|e| describe_error(&store, &e, self.limits.budget))?;
        let memory = instance
            .get_memory(&mut store, "memory")
            .ok_or("the module's `memory` export is not a memory")?;
        macro_rules! func {
            ($name:literal) => {
                instance
                    .get_typed_func(&mut store, $name)
                    .map_err(|e| format!("export {} has the wrong signature: {e:#}", $name))?
            };
        }
        let abi: TypedFunc<(), i32> = func!("pylon_shard_abi");
        let exports = Exports {
            scratch: func!("pylon_scratch"),
            output_ptr: func!("pylon_output_ptr"),
            output_len: func!("pylon_output_len"),
            init: func!("pylon_init"),
            apply_input: func!("pylon_apply_input"),
            tick: func!("pylon_tick"),
            snapshot: func!("pylon_snapshot"),
            snapshot_for: func!("pylon_snapshot_for"),
            is_finished: func!("pylon_is_finished"),
            authorize_subscribe: func!("pylon_authorize_subscribe"),
            authorize_input: func!("pylon_authorize_input"),
            replication: if instance
                .get_export(&mut store, "pylon_replication")
                .is_some()
            {
                Some(func!("pylon_replication"))
            } else {
                None
            },
            interest: if instance.get_export(&mut store, "pylon_interest").is_some() {
                Some(InterestExports {
                    config: func!("pylon_interest"),
                    entities: func!("pylon_entities"),
                    area: func!("pylon_interest_area"),
                    filter: func!("pylon_filter_visible"),
                    snapshot: func!("pylon_snapshot_visible"),
                })
            } else {
                None
            },
        };
        let mut inner = Inner {
            store,
            memory,
            exports,
            budget: self.limits.budget,
            in_tick: false,
            format: self.config.snapshot_format,
            broadcast: None,
            last_good: None,
            failed: None,
            interest_shared: false,
        };

        let version = inner.call(&abi, ())?;
        if version != ABI_VERSION {
            return Err(format!(
                "the module speaks shard ABI {version}; this pylon speaks {ABI_VERSION}"
            ));
        }
        let init = serde_json::to_vec(&serde_json::json!({ "shard": shard_id, "params": params }))
            .map_err(|e| e.to_string())?;
        let init_fn = inner.exports.init.clone();
        let (ptr, len) = inner.write_args(&[&init])?[0];
        match inner.call(&init_fn, (codec, ptr, len))? {
            STATUS_OK => {}
            STATUS_ERR => return Err(format!("init refused: {}", inner.output_text()?)),
            other => return Err(format!("pylon_init returned status {other}")),
        }
        Ok(WasmSim {
            inner: RefCell::new(inner),
            mirror: RefCell::new(Replicated::new()),
            replication: std::cell::Cell::new(None),
        })
    }
}

fn codec_id(format: SnapshotFormat) -> Result<i32, String> {
    match format {
        SnapshotFormat::Json | SnapshotFormat::JsonCompact => Ok(0),
        SnapshotFormat::MessagePack => Ok(1),
        SnapshotFormat::Bincode => {
            Err("a WebAssembly shard uses the json or msgpack codec, not bincode".into())
        }
    }
}

// ---------------------------------------------------------------------------
// Host state and the log import
// ---------------------------------------------------------------------------

struct HostState {
    limits: StoreLimits,
    shard_id: String,
    log_budget: LogBudget,
    /// The last error-level line the module logged. The guest SDK logs a
    /// panic's message just before it traps.
    last_error_line: Option<String>,
    /// When the current operation's time budget ends.
    deadline: Instant,
}

struct LogBudget {
    rate: f64,
    tokens: f64,
    last: Instant,
    dropped: u64,
}

impl LogBudget {
    fn new(rate: f64) -> Self {
        Self {
            rate,
            tokens: rate * 4.0,
            last: Instant::now(),
            dropped: 0,
        }
    }

    fn take(&mut self) -> bool {
        let now = Instant::now();
        let refill = now.duration_since(self.last).as_secs_f64() * self.rate;
        self.tokens = (self.tokens + refill).min(self.rate * 4.0);
        self.last = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            self.dropped += 1;
            false
        }
    }
}

fn guest_log(mut caller: Caller<'_, HostState>, level: i32, ptr: i32, len: i32) {
    let Some(memory) = caller.get_export("memory").and_then(|e| e.into_memory()) else {
        return;
    };
    let len = (len.max(0) as usize).min(MAX_LOG_LINE);
    let mut buf = vec![0u8; len];
    if memory.read(&caller, ptr as u32 as usize, &mut buf).is_err() {
        return;
    }
    let line = String::from_utf8_lossy(&buf).into_owned();
    let state = caller.data_mut();
    if level >= 3 {
        state.last_error_line = Some(line.clone());
    }
    if !state.log_budget.take() {
        return;
    }
    let dropped = std::mem::take(&mut state.log_budget.dropped);
    let shard = &state.shard_id;
    if dropped > 0 {
        tracing::warn!("[shard {shard}] dropped {dropped} log lines over the rate limit");
    }
    match level {
        0 => tracing::debug!("[shard {shard}] {line}"),
        1 => tracing::info!("[shard {shard}] {line}"),
        2 => tracing::warn!("[shard {shard}] {line}"),
        _ => tracing::error!("[shard {shard}] {line}"),
    }
}

fn describe_error(store: &Store<HostState>, err: &wasmtime::Error, budget: Duration) -> String {
    let base = match err.downcast_ref::<Trap>() {
        Some(Trap::Interrupt) => format!("the module ran past its {budget:?} time budget"),
        Some(trap) => format!("the module trapped: {trap}"),
        // The top-level message is "error while executing at wasm
        // backtrace"; the cause (a memory limit, a host error) is at the root.
        None => format!("the module trapped: {}", err.root_cause()),
    };
    tracing::debug!("[shard {}] {err:?}", store.data().shard_id);
    match &store.data().last_error_line {
        Some(line) => format!("{base} ({line})"),
        None => base,
    }
}

// ---------------------------------------------------------------------------
// WasmSim: SimState over one module instance
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Exports {
    scratch: TypedFunc<i32, i32>,
    output_ptr: TypedFunc<(), i32>,
    output_len: TypedFunc<(), i32>,
    init: TypedFunc<(i32, i32, i32), i32>,
    apply_input: TypedFunc<(i32, i32, i32, i32), i32>,
    tick: TypedFunc<i64, ()>,
    snapshot: TypedFunc<(), i32>,
    snapshot_for: TypedFunc<(i32, i32), i32>,
    is_finished: TypedFunc<(), i32>,
    authorize_subscribe: TypedFunc<(i32, i32, i32, i32), i32>,
    authorize_input: TypedFunc<(i32, i32, i32, i32, i32, i32), i32>,
    interest: Option<InterestExports>,
    replication: Option<TypedFunc<(), i32>>,
}

#[derive(Clone)]
struct InterestExports {
    config: TypedFunc<(), i32>,
    entities: TypedFunc<(), i32>,
    area: TypedFunc<(i32, i32), i32>,
    filter: TypedFunc<(i32, i32, i32, i32), i32>,
    snapshot: TypedFunc<(i32, i32, i32, i32), i32>,
}

struct Inner {
    store: Store<HostState>,
    memory: Memory,
    exports: Exports,
    budget: Duration,
    /// True from the first call of a tick (an input or `tick`) through
    /// `is_finished`, which the shard calls last. The whole tick shares one
    /// deadline.
    in_tick: bool,
    format: SnapshotFormat,
    /// This tick's broadcast snapshot, shared by every subscriber the module
    /// sends the broadcast to. Cleared by an input or a tick.
    broadcast: Option<RawSnapshot>,
    /// The last snapshot the module produced, sent in place of a snapshot
    /// the module failed to produce.
    last_good: Option<RawSnapshot>,
    /// Why the module stopped. Once set, no call reaches the module again.
    failed: Option<String>,
    /// The module's last answer: its snapshot depends only on the visible set.
    interest_shared: bool,
}

impl Inner {
    fn call<P: wasmtime::WasmParams, R: wasmtime::WasmResults>(
        &mut self,
        f: &TypedFunc<P, R>,
        params: P,
    ) -> Result<R, String> {
        if let Some(why) = &self.failed {
            return Err(format!("shard stopped: {why}"));
        }
        f.call(&mut self.store, params).map_err(|e| {
            let why = describe_error(&self.store, &e, self.budget);
            tracing::error!("[shard {}] stopped: {why}", self.store.data().shard_id);
            self.failed = Some(why.clone());
            why
        })
    }

    /// Start a time budget. Every call until the next `begin_op` shares it.
    fn begin_op(&mut self) {
        self.store.data_mut().deadline = Instant::now() + self.budget;
        self.store.set_epoch_deadline(1);
    }

    /// Start the tick's budget at its first call.
    fn enter_tick(&mut self) {
        if !self.in_tick {
            self.in_tick = true;
            self.begin_op();
        }
    }

    /// Copy call arguments into the module's scratch buffer. Returns each
    /// argument's (pointer, length).
    fn write_args(&mut self, args: &[&[u8]]) -> Result<Vec<(i32, i32)>, String> {
        let total: usize = args.iter().map(|a| a.len()).sum();
        let total = i32::try_from(total).map_err(|_| "call arguments over 2 GiB".to_string())?;
        let scratch = self.exports.scratch.clone();
        let base = self.call(&scratch, total)? as u32 as usize;
        let mut out = Vec::with_capacity(args.len());
        let mut offset = base;
        for arg in args {
            self.memory
                .write(&mut self.store, offset, arg)
                .map_err(|_| self.fail("pylon_scratch returned a buffer outside memory"))?;
            out.push((offset as i32, arg.len() as i32));
            offset += arg.len();
        }
        Ok(out)
    }

    fn output(&mut self) -> Result<Vec<u8>, String> {
        let (ptr_fn, len_fn) = (
            self.exports.output_ptr.clone(),
            self.exports.output_len.clone(),
        );
        let ptr = self.call(&ptr_fn, ())? as u32 as usize;
        let len = self.call(&len_fn, ())?;
        let len = usize::try_from(len).map_err(|_| self.fail("pylon_output_len is negative"))?;
        // Bounds-check against the module's memory before copying, so a bad
        // length cannot make the host allocate past the memory cap.
        let bytes = ptr
            .checked_add(len)
            .and_then(|end| self.memory.data(&self.store).get(ptr..end))
            .map(<[u8]>::to_vec);
        bytes.ok_or_else(|| self.fail("the output buffer is outside memory"))
    }

    fn output_text(&mut self) -> Result<String, String> {
        Ok(String::from_utf8_lossy(&self.output()?).into_owned())
    }

    /// Stop the module for a broken ABI contract.
    fn fail(&mut self, why: &str) -> String {
        tracing::error!("[shard {}] stopped: {why}", self.store.data().shard_id);
        self.failed = Some(why.to_string());
        why.to_string()
    }

    /// Map a status that carries a message in the output.
    fn status(&mut self, status: i32, export: &str) -> Result<(), String> {
        match status {
            STATUS_OK => Ok(()),
            STATUS_ERR => Err(self.output_text()?),
            other => Err(self.fail(&format!("{export} returned status {other}"))),
        }
    }

    fn broadcast(&mut self) -> Result<RawSnapshot, String> {
        if let Some(snap) = &self.broadcast {
            return Ok(snap.clone());
        }
        let f = self.exports.snapshot.clone();
        let status = self.call(&f, ())?;
        match status {
            STATUS_OK => {}
            STATUS_ERR => {
                let why = format!("pylon_snapshot failed: {}", self.output_text()?);
                return Err(self.fail(&why));
            }
            other => return Err(self.fail(&format!("pylon_snapshot returned status {other}"))),
        }
        let snap = RawSnapshot::new(self.format, self.output()?);
        self.broadcast = Some(snap.clone());
        self.last_good = Some(snap.clone());
        Ok(snap)
    }

    fn snapshot_for(&mut self, sid: &str) -> Result<RawSnapshot, String> {
        let f = self.exports.snapshot_for.clone();
        let (sp, sl) = self.write_args(&[sid.as_bytes()])?[0];
        match self.call(&f, (sp, sl))? {
            STATUS_OK => Ok(RawSnapshot::new(self.format, self.output()?)),
            STATUS_SAME_AS_BROADCAST => self.broadcast(),
            STATUS_ERR => {
                let why = format!("pylon_snapshot_for failed: {}", self.output_text()?);
                Err(self.fail(&why))
            }
            other => Err(self.fail(&format!("pylon_snapshot_for returned status {other}"))),
        }
    }

    /// What every subscriber gets when the module could not produce the
    /// broadcast snapshot: the last broadcast it did produce, which every
    /// subscriber was allowed to see.
    fn fallback_snapshot(&self) -> RawSnapshot {
        self.last_good
            .clone()
            .unwrap_or_else(|| self.null_snapshot())
    }

    /// What one subscriber gets when the module could not produce that
    /// subscriber's snapshot. Never a broadcast: a per-subscriber snapshot
    /// may exist to hide entities the broadcast holds.
    fn null_snapshot(&self) -> RawSnapshot {
        let null: &[u8] = match self.format {
            SnapshotFormat::MessagePack => &[0xc0],
            _ => b"null",
        };
        RawSnapshot::new(self.format, null)
    }
}

/// A [`SimState`] backed by one instance of a shard module.
pub struct WasmSim {
    // SimState takes `&self` for snapshots and authorization, and a wasm
    // call needs `&mut Store`. The shard holds its state behind a mutex, so
    // one call runs at a time and the RefCell never sees a second borrow.
    inner: RefCell<Inner>,
    /// The host's copy of the module's replicated store, rebuilt from the
    /// change logs `pylon_replication` returns. Its own cell, so the shard
    /// can hold a borrow of it while calling the module for interest.
    mirror: RefCell<Replicated>,
    /// The module's last replication settings; None when it does not
    /// replicate.
    replication: std::cell::Cell<Option<ReplicationConfig>>,
}

impl WasmSim {
    /// Why the module stopped, if it did.
    pub fn failure(&self) -> Option<String> {
        self.inner.borrow().failed.clone()
    }
}

/// The auth context in the JSON shape the guest SDK's `Auth` reads.
fn auth_json(auth: &ShardAuth) -> Vec<u8> {
    #[derive(Serialize)]
    struct Wire<'a> {
        user_id: &'a Option<String>,
        is_admin: bool,
        roles: &'a [String],
        tenant_id: &'a Option<String>,
        ticket: &'a Option<pylon_realtime::ShardTicket>,
    }
    serde_json::to_vec(&Wire {
        user_id: &auth.user_id,
        is_admin: auth.is_admin,
        roles: &auth.roles,
        tenant_id: &auth.tenant_id,
        ticket: &auth.ticket,
    })
    .unwrap_or_else(|_| b"{}".to_vec())
}

impl SimState for WasmSim {
    type Input = RawInput;
    type Snapshot = RawSnapshot;
    type Error = String;

    fn apply_input(
        &mut self,
        subscriber_id: &SubscriberId,
        input: RawInput,
        _now: Instant,
    ) -> Result<(), String> {
        let inner = self.inner.get_mut();
        inner.enter_tick();
        inner.broadcast = None;
        let f = inner.exports.apply_input.clone();
        let args = inner.write_args(&[subscriber_id.as_str().as_bytes(), input.bytes()])?;
        let status = inner.call(&f, (args[0].0, args[0].1, args[1].0, args[1].1))?;
        inner.status(status, "pylon_apply_input")
    }

    fn tick(&mut self, dt: Duration) {
        let inner = self.inner.get_mut();
        inner.enter_tick();
        inner.broadcast = None;
        let f = inner.exports.tick.clone();
        let nanos = i64::try_from(dt.as_nanos()).unwrap_or(i64::MAX);
        let _ = inner.call(&f, nanos);
    }

    fn snapshot(&self) -> RawSnapshot {
        let mut inner = self.inner.borrow_mut();
        if !inner.in_tick {
            inner.begin_op();
        }
        inner
            .broadcast()
            .unwrap_or_else(|_| inner.fallback_snapshot())
    }

    fn snapshot_for(&self, subscriber_id: &SubscriberId) -> RawSnapshot {
        let mut inner = self.inner.borrow_mut();
        if !inner.in_tick {
            inner.begin_op();
        }
        inner
            .snapshot_for(subscriber_id.as_str())
            .unwrap_or_else(|_| inner.null_snapshot())
    }

    fn is_finished(&self) -> bool {
        let mut inner = self.inner.borrow_mut();
        // The shard calls this last in a tick: the next call starts a new
        // budget.
        let in_tick = std::mem::take(&mut inner.in_tick);
        if inner.failed.is_some() {
            return true;
        }
        if !in_tick {
            inner.begin_op();
        }
        let f = inner.exports.is_finished.clone();
        match inner.call(&f, ()) {
            Ok(v) => v != 0,
            Err(_) => true,
        }
    }

    fn authorize_subscribe(
        &self,
        subscriber_id: &SubscriberId,
        auth: &ShardAuth,
    ) -> Result<(), String> {
        let mut inner = self.inner.borrow_mut();
        inner.begin_op();
        let f = inner.exports.authorize_subscribe.clone();
        let auth = auth_json(auth);
        let args = inner.write_args(&[subscriber_id.as_str().as_bytes(), &auth])?;
        let status = inner.call(&f, (args[0].0, args[0].1, args[1].0, args[1].1))?;
        inner.status(status, "pylon_authorize_subscribe")
    }

    fn authorize_input(
        &self,
        subscriber_id: &SubscriberId,
        auth: &ShardAuth,
        input: &RawInput,
    ) -> Result<(), String> {
        let mut inner = self.inner.borrow_mut();
        inner.begin_op();
        let f = inner.exports.authorize_input.clone();
        let auth = auth_json(auth);
        let args = inner.write_args(&[subscriber_id.as_str().as_bytes(), &auth, input.bytes()])?;
        let status = inner.call(
            &f,
            (
                args[0].0, args[0].1, args[1].0, args[1].1, args[2].0, args[2].1,
            ),
        )?;
        inner.status(status, "pylon_authorize_input")
    }

    fn interest_config(&self) -> Option<InterestConfig> {
        let mut inner = self.inner.borrow_mut();
        let f = inner.exports.interest.as_ref()?.config.clone();
        if !inner.in_tick {
            inner.begin_op();
        }
        match inner.call(&f, ()).ok()? {
            0 => None,
            1 => {
                let out = inner.output().ok()?;
                if out.len() != 12 {
                    inner.fail("pylon_interest output is not 12 bytes");
                    return None;
                }
                inner.interest_shared = u32::from_le_bytes(out[8..12].try_into().unwrap()) & 1 == 1;
                Some(InterestConfig {
                    cell_size: f32::from_le_bytes(out[0..4].try_into().unwrap()),
                    margin: f32::from_le_bytes(out[4..8].try_into().unwrap()),
                })
            }
            other => {
                inner.fail(&format!("pylon_interest returned {other}"));
                None
            }
        }
    }

    fn entities(&self, out: &mut Vec<EntityPos>) {
        let mut inner = self.inner.borrow_mut();
        let Some(f) = inner.exports.interest.as_ref().map(|i| i.entities.clone()) else {
            return;
        };
        let Ok(status) = inner.call(&f, ()) else {
            return;
        };
        if inner.status(status, "pylon_entities").is_err() {
            return;
        }
        let Ok(bytes) = inner.output() else { return };
        if bytes.len() % 16 != 0 {
            inner.fail("pylon_entities output is not a whole number of 16-byte entries");
            return;
        }
        out.extend(bytes.chunks_exact(16).map(|c| EntityPos {
            id: u64::from_le_bytes(c[0..8].try_into().unwrap()),
            x: f32::from_le_bytes(c[8..12].try_into().unwrap()),
            y: f32::from_le_bytes(c[12..16].try_into().unwrap()),
        }));
    }

    fn interest_area(&self, subscriber_id: &SubscriberId) -> Option<InterestArea> {
        let mut inner = self.inner.borrow_mut();
        let f = inner.exports.interest.as_ref()?.area.clone();
        let (sp, sl) = inner
            .write_args(&[subscriber_id.as_str().as_bytes()])
            .ok()?[0];
        match inner.call(&f, (sp, sl)).ok()? {
            0 => None,
            1 => {
                let out = inner.output().ok()?;
                if out.len() != 12 {
                    inner.fail("pylon_interest_area output is not 12 bytes");
                    return None;
                }
                let f32_at = |i: usize| f32::from_le_bytes(out[i..i + 4].try_into().unwrap());
                Some(InterestArea {
                    x: f32_at(0),
                    y: f32_at(4),
                    radius: f32_at(8),
                })
            }
            other => {
                inner.fail(&format!("pylon_interest_area returned {other}"));
                None
            }
        }
    }

    fn filter_visible(&self, subscriber_id: &SubscriberId, ids: &mut Vec<EntityId>) {
        let mut inner = self.inner.borrow_mut();
        let Some(f) = inner.exports.interest.as_ref().map(|i| i.filter.clone()) else {
            return;
        };
        let packed: Vec<u8> = ids.iter().flat_map(|id| id.to_le_bytes()).collect();
        // A module that fails here stops the shard; show the subscriber
        // nothing rather than everything.
        let filtered = (|| -> Result<Vec<EntityId>, String> {
            let args = inner.write_args(&[subscriber_id.as_str().as_bytes(), &packed])?;
            let status = inner.call(&f, (args[0].0, args[0].1, args[1].0, args[1].1))?;
            inner.status(status, "pylon_filter_visible")?;
            let out = inner.output()?;
            if out.len() % 8 != 0 {
                return Err(inner.fail("pylon_filter_visible output is not whole u64 ids"));
            }
            Ok(out
                .chunks_exact(8)
                .map(|c| u64::from_le_bytes(c.try_into().unwrap()))
                .collect())
        })();
        match filtered {
            Ok(kept) => *ids = kept,
            Err(_) => ids.clear(),
        }
    }

    fn snapshot_visible(&self, subscriber_id: &SubscriberId, view: &Visibility) -> RawSnapshot {
        let mut inner = self.inner.borrow_mut();
        let Some(f) = inner.exports.interest.as_ref().map(|i| i.snapshot.clone()) else {
            drop(inner);
            return self.snapshot_for(subscriber_id);
        };
        let mut packed = Vec::with_capacity(
            12 + 8 * (view.visible.len() + view.entered.len() + view.left.len()),
        );
        for n in [view.visible.len(), view.entered.len(), view.left.len()] {
            packed.extend_from_slice(&(n as u32).to_le_bytes());
        }
        for id in view.visible.iter().chain(&view.entered).chain(&view.left) {
            packed.extend_from_slice(&id.to_le_bytes());
        }
        let result = (|| -> Result<Option<RawSnapshot>, String> {
            let args = inner.write_args(&[subscriber_id.as_str().as_bytes(), &packed])?;
            match inner.call(&f, (args[0].0, args[0].1, args[1].0, args[1].1))? {
                STATUS_OK => Ok(Some(RawSnapshot::new(inner.format, inner.output()?))),
                STATUS_SAME_AS_BROADCAST => Ok(None),
                STATUS_ERR => {
                    let why = format!("pylon_snapshot_visible failed: {}", inner.output_text()?);
                    Err(inner.fail(&why))
                }
                other => Err(inner.fail(&format!("pylon_snapshot_visible returned {other}"))),
            }
        })();
        match result {
            Ok(Some(snap)) => snap,
            Ok(None) => {
                drop(inner);
                self.snapshot_for(subscriber_id)
            }
            Err(_) => inner.null_snapshot(),
        }
    }

    fn shares_visible_snapshots(&self) -> bool {
        self.inner.borrow().interest_shared
    }

    fn replicated(&self) -> Option<ReplicatedRef<'_>> {
        let mut inner = self.inner.borrow_mut();
        let f = inner.exports.replication.clone()?;
        if inner.failed.is_some() {
            // No more changes; keep sending what the mirror holds.
            drop(inner);
            return self
                .replication
                .get()
                .map(|_| ReplicatedRef::Cell(self.mirror.borrow()));
        }
        if !inner.in_tick {
            inner.begin_op();
        }
        let status = inner.call(&f, ()).ok()?;
        if status == 0 {
            self.replication.set(None);
            return None;
        }
        if status != 1 {
            inner.fail(&format!("pylon_replication returned {status}"));
            return None;
        }
        let out = inner.output().ok()?;
        if out.len() < 9 {
            inner.fail("pylon_replication output is shorter than its 9-byte header");
            return None;
        }
        let precision = f32::from_le_bytes(out[0..4].try_into().unwrap());
        let budget = u32::from_le_bytes(out[4..8].try_into().unwrap());
        let flags = out[8];
        self.replication.set(Some(ReplicationConfig {
            precision,
            max_bytes_per_tick: budget as usize,
            plane: if flags & 1 != 0 { Plane::XZ } else { Plane::XY },
        }));
        let mut mirror = self.mirror.borrow_mut();
        if flags & 2 != 0 {
            *mirror = Replicated::new();
        }
        if let Err(e) = mirror.apply_changes(&out[9..]) {
            inner.fail(&format!("pylon_replication sent a bad change log: {e}"));
        }
        drop(mirror);
        drop(inner);
        Some(ReplicatedRef::Cell(self.mirror.borrow()))
    }

    fn replication_config(&self) -> ReplicationConfig {
        self.replication.get().unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// WasmShardHost: the app's shard kinds and live shards
// ---------------------------------------------------------------------------

/// Why `create` failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreateError {
    UnknownKind(String),
    InvalidId(String),
    Exists(String),
    LimitReached { kind: String, max: usize },
    Init(String),
}

impl CreateError {
    pub fn code(&self) -> &'static str {
        match self {
            CreateError::UnknownKind(_) => "SHARD_KIND_NOT_FOUND",
            CreateError::InvalidId(_) => "SHARD_ID_INVALID",
            CreateError::Exists(_) => "SHARD_EXISTS",
            CreateError::LimitReached { .. } => "SHARD_LIMIT_REACHED",
            CreateError::Init(_) => "SHARD_INIT_FAILED",
        }
    }
}

impl std::fmt::Display for CreateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CreateError::UnknownKind(k) => write!(f, "no shard kind named \"{k}\" in app.ts"),
            CreateError::InvalidId(why) => f.write_str(why),
            CreateError::Exists(id) => write!(f, "shard \"{id}\" is already running"),
            CreateError::LimitReached { kind, max } => {
                write!(
                    f,
                    "shard kind \"{kind}\" already has {max} running shards (its maxInstances)"
                )
            }
            CreateError::Init(why) => write!(f, "shard init failed: {why}"),
        }
    }
}

/// A live shard, as `ctx.shards.get` reports it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ShardInfo {
    pub id: String,
    pub kind: String,
    pub tick: u64,
    pub subscribers: usize,
    pub running: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// The app's shard kinds and the shards created from them.
pub struct WasmShardHost {
    kinds: HashMap<String, Arc<WasmShardKind>>,
    registry: ShardRegistry<WasmSim>,
    kind_of: RwLock<HashMap<String, String>>,
    /// When each shard last lost its last subscriber.
    idle_since: Mutex<HashMap<String, Instant>>,
    create_lock: Mutex<()>,
    stopped: AtomicBool,
}

impl WasmShardHost {
    /// A host for `kinds`. Starts a thread that removes stopped shards.
    pub fn new(kinds: Vec<WasmShardKind>) -> Arc<Self> {
        let host = Arc::new(Self {
            kinds: kinds
                .into_iter()
                .map(|k| (k.name.clone(), Arc::new(k)))
                .collect(),
            registry: ShardRegistry::new(),
            kind_of: RwLock::new(HashMap::new()),
            idle_since: Mutex::new(HashMap::new()),
            create_lock: Mutex::new(()),
            stopped: AtomicBool::new(false),
        });
        let weak: Weak<Self> = Arc::downgrade(&host);
        let _ = std::thread::Builder::new()
            .name("pylon-shard-sweep".into())
            .spawn(move || loop {
                std::thread::sleep(Duration::from_secs(2));
                match weak.upgrade() {
                    Some(host) if !host.stopped.load(Ordering::Acquire) => host.sweep(),
                    _ => return,
                }
            });
        host
    }

    pub fn kind_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.kinds.keys().cloned().collect();
        names.sort();
        names
    }

    /// Start shard `id` of `kind`, running the module's init with `params`.
    pub fn create(
        &self,
        kind: &str,
        id: &str,
        params: &serde_json::Value,
    ) -> Result<ShardInfo, CreateError> {
        let spec = self
            .kinds
            .get(kind)
            .ok_or_else(|| CreateError::UnknownKind(kind.to_string()))?;
        validate_shard_id(id)?;
        let _guard = self.create_lock.lock().unwrap();
        if let Some(existing) = self.registry.get(id) {
            if existing.is_running() {
                return Err(CreateError::Exists(id.to_string()));
            }
        }
        // Count only running shards, so a stopped one waiting for the sweep
        // does not hold a slot.
        let running = self
            .kind_of
            .read()
            .unwrap()
            .iter()
            .filter(|(sid, k)| {
                k.as_str() == kind && self.registry.get(sid).is_some_and(|s| s.is_running())
            })
            .count();
        if running >= spec.limits.max_instances {
            return Err(CreateError::LimitReached {
                kind: kind.to_string(),
                max: spec.limits.max_instances,
            });
        }
        let sim = spec.instantiate(id, params).map_err(CreateError::Init)?;
        let shard = Shard::new(id, sim, spec.config.clone());
        let info = ShardInfo {
            id: id.to_string(),
            kind: kind.to_string(),
            tick: shard.tick_number(),
            subscribers: 0,
            running: true,
            error: None,
        };
        self.kind_of
            .write()
            .unwrap()
            .insert(id.to_string(), kind.to_string());
        self.idle_since.lock().unwrap().remove(id);
        self.registry.insert(shard);
        tracing::info!("[shard {id}] started ({kind})");
        Ok(info)
    }

    /// Stop and remove a shard. Its subscribers' connections close.
    pub fn stop(&self, id: &str) -> bool {
        // Serialized with create and the sweep, so a stop never removes the
        // bookkeeping of a shard created under the same id meanwhile.
        let _guard = self.create_lock.lock().unwrap();
        let removed = self.registry.remove(id);
        self.kind_of.write().unwrap().remove(id);
        self.idle_since.lock().unwrap().remove(id);
        removed
    }

    pub fn info(&self, id: &str) -> Option<ShardInfo> {
        let shard = self.registry.get(id)?;
        let kind = self.kind_of.read().unwrap().get(id).cloned()?;
        let error = shard.with_state(|s| s.failure());
        Some(ShardInfo {
            id: id.to_string(),
            kind,
            tick: shard.tick_number(),
            subscribers: shard.subscriber_count(),
            running: shard.is_running(),
            error,
        })
    }

    pub fn list(&self) -> Vec<ShardInfo> {
        let mut ids = self.registry.ids();
        ids.sort();
        ids.iter().filter_map(|id| self.info(id)).collect()
    }

    /// Stop every shard. Called at shutdown.
    pub fn stop_all(&self) {
        self.stopped.store(true, Ordering::Release);
        for id in self.registry.ids() {
            self.stop(&id);
        }
    }

    /// Stop shards idle past their kind's limit, then remove stopped ones.
    fn sweep(&self) {
        let now = Instant::now();
        let kinds: Vec<(String, String)> = self
            .kind_of
            .read()
            .unwrap()
            .iter()
            .map(|(id, k)| (id.clone(), k.clone()))
            .collect();
        {
            let mut idle = self.idle_since.lock().unwrap();
            for (id, kind) in &kinds {
                let Some(shard) = self.registry.get(id) else {
                    continue;
                };
                let limit = self.kinds[kind].limits.idle_shutdown;
                if shard.subscriber_count() > 0 || limit.is_zero() {
                    idle.remove(id);
                    continue;
                }
                let since = *idle.entry(id.clone()).or_insert(now);
                if now.duration_since(since) >= limit {
                    tracing::info!("[shard {id}] stopping: no subscribers for {limit:?}");
                    shard.stop();
                }
            }
        }
        // Under the create lock, so a shard created under a stopped id is
        // not swept with it.
        let _guard = self.create_lock.lock().unwrap();
        let before = self.registry.ids();
        if self.registry.sweep_finished() == 0 {
            return;
        }
        let mut kind_of = self.kind_of.write().unwrap();
        let mut idle = self.idle_since.lock().unwrap();
        for id in before.iter().filter(|id| self.registry.get(id).is_none()) {
            kind_of.remove(id);
            idle.remove(id);
            tracing::info!("[shard {id}] removed after it stopped");
        }
    }
}

impl pylon_realtime::DynShardRegistry for WasmShardHost {
    fn get(&self, id: &str) -> Option<Arc<dyn pylon_realtime::DynShard>> {
        pylon_realtime::DynShardRegistry::get(&self.registry, id)
    }

    fn ids(&self) -> Vec<String> {
        self.registry.ids()
    }

    fn len(&self) -> usize {
        self.registry.len()
    }
}

/// Shard ids go in URLs and logs: 1 to 128 characters of letters, digits,
/// `-`, `_`, `.`, and `:`.
pub fn validate_shard_id(id: &str) -> Result<(), CreateError> {
    let ok = !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'));
    if ok {
        Ok(())
    } else {
        Err(CreateError::InvalidId(format!(
            "shard id \"{id}\" must be 1 to 128 characters of letters, digits, '-', '_', '.', ':'"
        )))
    }
}

/// Compile the shard kinds an app declares. `None` when it declares none.
pub fn load_manifest_shards(
    shards: &[pylon_kernel::ManifestShard],
    app_root: &std::path::Path,
) -> Result<Option<Arc<WasmShardHost>>, String> {
    if shards.is_empty() {
        return Ok(None);
    }
    let started = Instant::now();
    let mut kinds = Vec::with_capacity(shards.len());
    for def in shards {
        let path = app_root.join(&def.wasm);
        let bytes = std::fs::read(&path).map_err(|e| {
            let hint = match &def.crate_dir {
                Some(_) => "run `pylon shards build`",
                None => {
                    "build the module, or set `crate` in shard({...}) so `pylon shards build` can"
                }
            };
            format!(
                "shard \"{}\": cannot read {}: {e}; {hint}",
                def.name,
                path.display()
            )
        })?;
        let (config, limits) = shard_settings(def);
        kinds.push(WasmShardKind::compile(&def.name, &bytes, config, limits)?);
    }
    tracing::info!(
        "[shards] compiled {} kind(s) in {:.0?}: {}",
        kinds.len(),
        started.elapsed(),
        kinds
            .iter()
            .map(|k| k.name())
            .collect::<Vec<_>>()
            .join(", ")
    );
    Ok(Some(WasmShardHost::new(kinds)))
}

/// The shard config and limits for one declared kind.
pub fn shard_settings(def: &pylon_kernel::ManifestShard) -> (ShardConfig, WasmLimits) {
    let base = ShardConfig::default();
    let input = def
        .input
        .clone()
        .unwrap_or(pylon_kernel::ManifestShardInput {
            rate_per_sec: None,
            burst: None,
            max_queued: None,
            max_per_tick: None,
        });
    let config = ShardConfig {
        tick_rate_hz: def.tick_rate,
        fixed_timestep: def.fixed_timestep,
        max_subscribers: def
            .max_subscribers
            .map_or(base.max_subscribers, |n| n as usize),
        // The host stops idle shards itself (see WasmShardHost::sweep), so an
        // event-driven shard, which does not tick while idle, stops too.
        idle_ticks_before_shutdown: 0,
        max_queued_inputs_per_subscriber: input
            .max_queued
            .map_or(base.max_queued_inputs_per_subscriber, |n| n as usize),
        max_inputs_per_subscriber_per_tick: input
            .max_per_tick
            .map_or(base.max_inputs_per_subscriber_per_tick, |n| n as usize),
        input_rate_per_subscriber: input
            .rate_per_sec
            .map_or(base.input_rate_per_subscriber, f64::from),
        input_burst_per_subscriber: input
            .burst
            .map_or(base.input_burst_per_subscriber, f64::from),
        snapshot_format: match def.codec {
            pylon_kernel::ManifestShardCodec::Json => SnapshotFormat::Json,
            pylon_kernel::ManifestShardCodec::Msgpack => SnapshotFormat::MessagePack,
        },
        ..base
    };
    let defaults = WasmLimits::default();
    let limits = WasmLimits {
        memory_bytes: def
            .memory_mb
            .map_or(defaults.memory_bytes, |mb| (mb as usize) << 20),
        budget: def
            .tick_budget_ms
            .map_or(defaults.budget, |ms| Duration::from_millis(ms.into())),
        max_instances: def
            .max_instances
            .map_or(defaults.max_instances, |n| n as usize),
        idle_shutdown: def
            .idle_shutdown_secs
            .map_or(defaults.idle_shutdown, |s| Duration::from_secs(s.into())),
        ..defaults
    };
    (config, limits)
}

/// Serve one `ctx.shards.*` call from a function.
pub fn handle_shard_op(
    host: &WasmShardHost,
    req: &pylon_functions::protocol::ShardOpMessage,
) -> Result<serde_json::Value, (String, String)> {
    let id = || {
        req.id.as_deref().filter(|s| !s.is_empty()).ok_or_else(|| {
            (
                "INVALID_SHARD".to_string(),
                "shard id is required".to_string(),
            )
        })
    };
    let to_json = |info: &ShardInfo| serde_json::to_value(info).unwrap_or_default();
    match req.op.as_str() {
        "create" => {
            let kind = req.kind.as_deref().unwrap_or_default();
            let params = if req.params.is_null() {
                serde_json::json!({})
            } else {
                req.params.clone()
            };
            host.create(kind, id()?, &params)
                .map(|info| to_json(&info))
                .map_err(|e| (e.code().to_string(), e.to_string()))
        }
        "stop" => Ok(serde_json::json!(host.stop(id()?))),
        "get" => Ok(host
            .info(id()?)
            .map(|info| to_json(&info))
            .unwrap_or(serde_json::Value::Null)),
        "list" => Ok(serde_json::to_value(host.list()).unwrap_or_default()),
        other => Err((
            "INVALID_SHARD_OP".into(),
            format!("unknown shard op \"{other}\""),
        )),
    }
}
