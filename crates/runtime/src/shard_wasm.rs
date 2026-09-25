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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock, Weak};
use std::time::{Duration, Instant};

use pylon_realtime::{
    EntityId, EntityPos, InterestArea, InterestConfig, Plane, RawInput, RawSnapshot, Replicated,
    ReplicatedRef, ReplicationConfig, Shard, ShardAuth, ShardConfig, ShardRegistry, SimState,
    SnapshotFormat, SubscriberId, Visibility,
};
use serde::{Deserialize, Serialize};

use crate::shard_cluster::{
    self, choose, home, Claim, Machine, MachineConfig, PgShardDirectory, Placement, RemoteOp,
    RemoteReply,
};
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
/// `pylon_save`: the shard keeps no saved state.
const STATUS_NONE: i32 = 3;

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
    /// Entities the host's copy of a replicating module's store may hold.
    /// The module's memory cap does not cover that copy.
    pub max_replicated_entities: usize,
}

impl Default for WasmLimits {
    fn default() -> Self {
        Self {
            memory_bytes: 64 << 20,
            budget: Duration::from_millis(100),
            max_instances: 64,
            log_lines_per_sec: 20.0,
            idle_shutdown: Duration::from_secs(90),
            max_replicated_entities: 100_000,
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
    /// The machine's lease, when the host runs in a cluster: set once, and
    /// shared by every instance started afterwards.
    lease: OnceLock<Arc<LeaseClock>>,
}

/// When this machine's lease on the shard directory ends, shared by the
/// stores of the shards it runs. A guest call is refused once it has
/// passed, and a call running at that moment is interrupted, so a shard
/// never runs past the lease even if the fence thread is late.
#[derive(Default)]
pub struct LeaseClock {
    /// Milliseconds after [`LeaseClock::base`]; 0 means no lease.
    until_ms: AtomicU64,
}

impl LeaseClock {
    fn base() -> Instant {
        static BASE: OnceLock<Instant> = OnceLock::new();
        *BASE.get_or_init(Instant::now)
    }

    fn millis(at: Instant) -> u64 {
        at.saturating_duration_since(Self::base()).as_millis() as u64
    }

    /// `at` rounded down to the clock's millisecond: the lease stores its
    /// end this way, so it and the guests' clock expire at the same instant.
    fn floor(at: Instant) -> Instant {
        Self::base() + Duration::from_millis(Self::millis(at))
    }

    /// The lease now ends at `until`, or has ended when None.
    pub fn set(&self, until: Option<Instant>) {
        let ms = until.map_or(0, |u| Self::millis(u).max(1));
        self.until_ms.store(ms, Ordering::Release);
    }

    pub fn expired(&self) -> bool {
        Self::millis(Instant::now()) >= self.until_ms.load(Ordering::Acquire)
    }
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

/// Saved-state exports: both or neither (see pylon-shard-guest).
const SAVE_EXPORTS: &[&str] = &["pylon_save", "pylon_restore"];

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
        for group in [INTEREST_EXPORTS, SAVE_EXPORTS] {
            let present: Vec<&str> = group
                .iter()
                .copied()
                .filter(|n| exported.contains(n))
                .collect();
            if !present.is_empty() && present.len() != group.len() {
                return Err(format!(
                    "shard \"{name}\": the module exports {} but not all of {}",
                    present.join(", "),
                    group.join(", ")
                ));
            }
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
            lease: OnceLock::new(),
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Bind instances started from now on to a machine's lease. Set once
    /// (the cluster host does it when it joins the directory); later calls
    /// do nothing.
    pub fn set_lease_clock(&self, clock: Arc<LeaseClock>) {
        let _ = self.lease.set(clock);
    }

    pub fn config(&self) -> &ShardConfig {
        &self.config
    }

    pub fn limits(&self) -> &WasmLimits {
        &self.limits
    }

    /// True when the module can save and restore its state.
    pub fn saves_state(&self) -> bool {
        self.module.exports().any(|e| e.name() == "pylon_save")
    }

    /// Instantiate the module and run `pylon_init` with `params`.
    pub fn instantiate(
        &self,
        shard_id: &str,
        params: &serde_json::Value,
    ) -> Result<WasmSim, String> {
        self.start(shard_id, params, None)
    }

    /// Instantiate the module and rebuild the shard from `state`, bytes an
    /// earlier instance's [`WasmSim::save`] returned.
    pub fn restore(
        &self,
        shard_id: &str,
        params: &serde_json::Value,
        state: &[u8],
    ) -> Result<WasmSim, String> {
        if !self.saves_state() {
            return Err(format!(
                "shard kind \"{}\" does not save state (its module has no pylon_restore)",
                self.name
            ));
        }
        self.start(shard_id, params, Some(state))
    }

    fn start(
        &self,
        shard_id: &str,
        params: &serde_json::Value,
        state: Option<&[u8]>,
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
                lease: self.lease.get().cloned(),
            },
        );
        store.limiter(|s| &mut s.limits);
        // The epoch thread only wakes the check. The deadline itself is wall
        // time: on a loaded machine the thread's 2 ms sleeps run long, and a
        // budget counted in epoch ticks would stretch with them.
        store.epoch_deadline_callback(|ctx| {
            Ok(
                if Instant::now() >= ctx.data().deadline || ctx.data().lease_lapsed() {
                    UpdateDeadline::Interrupt
                } else {
                    UpdateDeadline::Continue(1)
                },
            )
        });
        store.set_epoch_deadline(1);

        let instance = self
            .linker
            .instantiate(&mut store, &self.module)
            .map_err(|e| {
                // A start function interrupted because the lease ended is
                // not the module's failure.
                if store.data().lease_lapsed() {
                    LEASE_LAPSED.to_string()
                } else {
                    describe_error(&store, &e, self.limits.budget)
                }
            })?;
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
            save: if instance.get_export(&mut store, "pylon_save").is_some() {
                Some(SaveExports {
                    save: func!("pylon_save"),
                    restore: func!("pylon_restore"),
                })
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
        let (status, export) = match (state, inner.exports.save.clone()) {
            (None, _) => {
                let init_fn = inner.exports.init.clone();
                let (ptr, len) = inner.write_args(&[&init])?[0];
                (inner.call(&init_fn, (codec, ptr, len))?, "pylon_init")
            }
            (Some(state), Some(save)) => {
                let args = inner.write_args(&[&init, state])?;
                let ((ip, il), (sp, sl)) = (args[0], args[1]);
                (
                    inner.call(&save.restore, (codec, ip, il, sp, sl))?,
                    "pylon_restore",
                )
            }
            (Some(_), None) => return Err("the module has no pylon_restore".into()),
        };
        match status {
            STATUS_OK => {}
            STATUS_ERR => return Err(format!("{export} refused: {}", inner.output_text()?)),
            other => return Err(format!("{export} returned status {other}")),
        }
        Ok(WasmSim {
            inner: RefCell::new(inner),
            mirror: RefCell::new(Replicated::new()),
            replication: std::cell::Cell::new(None),
            limits: self.limits.clone(),
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
    /// The machine's lease in a cluster.
    lease: Option<Arc<LeaseClock>>,
}

impl HostState {
    fn lease_lapsed(&self) -> bool {
        self.lease.as_ref().is_some_and(|l| l.expired())
    }
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

const LEASE_LAPSED: &str = "this machine's lease on the shard directory lapsed";

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
    save: Option<SaveExports>,
}

#[derive(Clone)]
struct SaveExports {
    save: TypedFunc<(), i32>,
    restore: TypedFunc<(i32, i32, i32, i32, i32), i32>,
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
        if self.store.data().lease_lapsed() {
            let why = LEASE_LAPSED.to_string();
            self.failed = Some(why.clone());
            return Err(why);
        }
        f.call(&mut self.store, params).map_err(|e| {
            let why = if self.store.data().lease_lapsed() {
                LEASE_LAPSED.to_string()
            } else {
                describe_error(&self.store, &e, self.budget)
            };
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
    limits: WasmLimits,
}

impl WasmSim {
    /// Why the module stopped, if it did.
    pub fn failure(&self) -> Option<String> {
        self.inner.borrow().failed.clone()
    }

    /// The state to start the shard again from ([`WasmShardKind::restore`]),
    /// or `None` when the module keeps none. Call it between ticks (the
    /// shard's state lock held): it has its own time budget. An error means
    /// the module trapped or broke the ABI, and the shard has stopped.
    pub fn save(&self) -> Result<Option<Vec<u8>>, String> {
        let mut inner = self.inner.borrow_mut();
        let Some(save) = inner.exports.save.clone() else {
            return Ok(None);
        };
        inner.begin_op();
        match inner.call(&save.save, ())? {
            STATUS_OK => inner.output().map(Some),
            STATUS_NONE => Ok(None),
            STATUS_ERR => Err(format!("pylon_save refused: {}", inner.output_text()?)),
            other => Err(inner.fail(&format!("pylon_save returned status {other}"))),
        }
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
            // A full dump replaces everything. `clear` keeps the change
            // counter running, so entities it re-creates read as new to
            // every subscriber's baseline.
            mirror.clear();
        }
        // The copy lives in host memory, outside the module's cap: apply
        // under limits, refusing the change that would pass one. A store
        // that fits in the module's 32-bit memory takes up to about twice
        // that on a 64-bit host (pointers and map nodes double).
        if let Err(e) = mirror.apply_changes_limited(
            &out[9..],
            self.limits.max_replicated_entities,
            self.limits.memory_bytes.saturating_mul(2),
        ) {
            inner.fail(&format!(
                "pylon_replication sent a change log the host refused: {e}"
            ));
            mirror.clear();
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
    LimitReached {
        kind: String,
        max: usize,
    },
    Init(String),
    /// The shard directory could not be read or written.
    Cluster(String),
    /// The requested machine is not live, or cannot be reached.
    MachineUnavailable(String),
    /// Another machine refused the create.
    Remote {
        code: String,
        message: String,
    },
}

impl CreateError {
    pub fn code(&self) -> &'static str {
        match self {
            CreateError::UnknownKind(_) => "SHARD_KIND_NOT_FOUND",
            CreateError::InvalidId(_) => "SHARD_ID_INVALID",
            CreateError::Exists(_) => "SHARD_EXISTS",
            CreateError::LimitReached { .. } => "SHARD_LIMIT_REACHED",
            CreateError::Init(_) => "SHARD_INIT_FAILED",
            CreateError::Cluster(_) => "SHARD_CLUSTER_ERROR",
            CreateError::MachineUnavailable(_) => "SHARD_MACHINE_UNAVAILABLE",
            CreateError::Remote { .. } => "SHARD_REMOTE_ERROR",
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
            CreateError::Cluster(why) => write!(f, "shard directory: {why}"),
            CreateError::MachineUnavailable(why) => f.write_str(why),
            CreateError::Remote { message, .. } => f.write_str(message),
        }
    }
}

/// A live shard, as `ctx.shards.get` reports it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShardInfo {
    pub id: String,
    pub kind: String,
    pub tick: u64,
    pub subscribers: usize,
    pub running: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The machine that runs it, when the app runs on several.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub machine: Option<String>,
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
    /// The directory, when the app runs on several machines.
    cluster: std::sync::OnceLock<ClusterState>,
}

/// This machine's place in the shard directory.
struct ClusterState {
    dir: PgShardDirectory,
    me: MachineConfig,
    save_every: Duration,
    /// This machine's lease on the directory.
    lease: Mutex<Lease>,
    /// The lease's end, as the shards' stores see it.
    clock: Arc<LeaseClock>,
    /// One save at a time: a state captured earlier is never written after
    /// one captured later (the final save at shutdown).
    save_lock: Mutex<()>,
    /// Set once when another live process holds this machine's id.
    id_conflict: AtomicBool,
    /// Shutdown's final saves are done: the heartbeat stops.
    left: AtomicBool,
    /// Held for a whole heartbeat, and by shutdown while it sets `left` and
    /// leaves: a heartbeat in flight never writes the row back after it.
    beat_lock: Mutex<()>,
    /// Shards running here, each with the epoch it was placed under. Only
    /// these are saved and released: the fence and shutdown empty it, so a
    /// shard either of them stopped keeps its placement for another machine.
    owned: Mutex<HashMap<String, i64>>,
    /// Serializes changes to what this machine holds: create, adopt, stop,
    /// and the round's check against the directory.
    own_lock: Mutex<()>,
    /// When each local shard's state was last saved.
    saved_at: Mutex<HashMap<String, Instant>>,
}

/// This machine's right to run shards (see [`shard_cluster::FENCE_AFTER`]).
struct Lease {
    /// Changes when the lease lapses, so a heartbeat already in flight
    /// cannot renew the old one.
    epoch: i64,
    /// Until when shards may run here. None until a heartbeat of this epoch
    /// reaches the directory.
    until: Option<Instant>,
    /// Shutting down: no shard starts here again.
    closed: bool,
}

/// A shard starts only when the lease has at least this long left.
const LEASE_MARGIN: Duration = Duration::from_secs(1);
/// How often the fence checks the lease. It touches no database, so a
/// blocked directory call never delays it.
const FENCE_EVERY: Duration = Duration::from_millis(100);

impl ClusterState {
    /// This machine as a live-machine row, before its first heartbeat lands.
    fn me_as_machine(&self) -> Machine {
        Machine {
            id: self.me.id.clone(),
            address: self.me.address.clone(),
            fly_instance: self.me.fly_instance.clone(),
            capacity: self.me.capacity,
            load: 0,
            epoch: self.lease.lock().unwrap().epoch,
        }
    }

    /// `machine_id`, when it is live.
    fn live(&self, machine_id: &str) -> Option<Machine> {
        let live = self.dir.live_machines().ok()?;
        live.into_iter().find(|m| m.id == machine_id)
    }

    /// The lease epoch, when the lease has at least [`LEASE_MARGIN`] left.
    fn current_epoch(&self) -> Option<i64> {
        let lease = self.lease.lock().unwrap();
        lease.startable().then_some(lease.epoch)
    }
}

impl Lease {
    /// Extend the lease for a heartbeat of `epoch` sent at `sent` that the
    /// directory accepted, answered at `now`. Returns the new end, or None
    /// when the reply authorizes nothing: it is for a lapsed epoch, it
    /// arrived after its own lease ended, or the current lease has already
    /// run out. Only the fence moves past an expired lease (a new epoch), so
    /// a shard stopped by the lapse is never taken for one that ended.
    fn renew(&mut self, epoch: i64, sent: Instant, now: Instant) -> Option<Instant> {
        let until = LeaseClock::floor(sent + shard_cluster::FENCE_AFTER);
        let expired = self.until.is_some_and(|u| u <= now);
        if self.epoch != epoch || until <= now || expired {
            return None;
        }
        let until = self.until.map_or(until, |u| u.max(until));
        self.until = Some(until);
        Some(until)
    }

    /// True when a shard may start here now.
    fn startable(&self) -> bool {
        !self.closed
            && self
                .until
                .is_some_and(|until| until > Instant::now() + LEASE_MARGIN)
    }
}

fn lease_error() -> CreateError {
    CreateError::Cluster(
        "this machine's lease on the shard directory is not current; try again".into(),
    )
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
            cluster: std::sync::OnceLock::new(),
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
    /// On several machines (see [`WasmShardHost::attach_cluster`]) it runs
    /// on the machine with the most free capacity.
    pub fn create(
        &self,
        kind: &str,
        id: &str,
        params: &serde_json::Value,
    ) -> Result<ShardInfo, CreateError> {
        self.create_on(kind, id, params, None)
    }

    /// [`WasmShardHost::create`] on machine `machine` when it is `Some`.
    pub fn create_on(
        &self,
        kind: &str,
        id: &str,
        params: &serde_json::Value,
        machine: Option<&str>,
    ) -> Result<ShardInfo, CreateError> {
        if !self.kinds.contains_key(kind) {
            return Err(CreateError::UnknownKind(kind.to_string()));
        }
        validate_shard_id(id)?;
        let Some(cluster) = self.cluster.get() else {
            if let Some(m) = machine {
                return Err(CreateError::MachineUnavailable(format!(
                    "this app runs on one machine; there is no machine \"{m}\""
                )));
            }
            return self.create_local(kind, id, params, None, None);
        };
        if self.registry.get(id).is_some_and(|s| s.is_running()) {
            return Err(CreateError::Exists(id.to_string()));
        }
        let live = cluster.dir.live_machines().map_err(CreateError::Cluster)?;
        let target = match machine {
            Some(m) => live.iter().find(|x| x.id == m).cloned().ok_or_else(|| {
                CreateError::MachineUnavailable(format!("machine \"{m}\" is not live"))
            })?,
            None => choose(&live)
                .cloned()
                .unwrap_or_else(|| cluster.me_as_machine()),
        };
        if target.id == cluster.me.id {
            return self.create_claimed(kind, id, params, machine.is_some());
        }
        let Some(address) = target.address.as_deref() else {
            return Err(CreateError::MachineUnavailable(format!(
                "machine \"{}\" has no address (set PYLON_SHARD_ADVERTISE_URL)",
                target.id
            )));
        };
        let op = RemoteOp::Create {
            kind: kind.to_string(),
            id: id.to_string(),
            params: params.clone(),
            pinned: machine.is_some(),
        };
        match shard_cluster::call(&target.id, address, &op) {
            Ok(RemoteReply::Ok(v)) => serde_json::from_value(v)
                .map_err(|e| CreateError::Cluster(format!("bad reply from {}: {e}", target.id))),
            // Known codes keep their meaning, so a caller that catches
            // SHARD_EXISTS still does when the shard is on another machine.
            Ok(RemoteReply::Err { code, message }) => Err(match code.as_str() {
                "SHARD_EXISTS" => CreateError::Exists(id.to_string()),
                "SHARD_LIMIT_REACHED" => CreateError::LimitReached {
                    kind: kind.to_string(),
                    max: self.kinds[kind].limits.max_instances,
                },
                "SHARD_INIT_FAILED" => CreateError::Init(message),
                _ => CreateError::Remote { code, message },
            }),
            Err(e) => Err(CreateError::MachineUnavailable(e)),
        }
    }

    /// Place the shard on this machine in the directory, then start it.
    fn create_claimed(
        &self,
        kind: &str,
        id: &str,
        params: &serde_json::Value,
        pinned: bool,
    ) -> Result<ShardInfo, CreateError> {
        let cluster = self
            .cluster
            .get()
            .expect("create_claimed runs in a cluster");
        let _own = cluster.own_lock.lock().unwrap();
        if self.stopped.load(Ordering::Acquire) {
            return Err(lease_error());
        }
        let epoch = cluster.current_epoch().ok_or_else(lease_error)?;
        let spec = &self.kinds[kind];
        let placement = Placement {
            shard_id: id.to_string(),
            kind: kind.to_string(),
            params: params.clone(),
            machine_id: cluster.me.id.clone(),
            pinned,
            failed: None,
            epoch,
        };
        self.claim_and_start(cluster, &placement, spec.limits.max_instances)
    }

    fn claim_and_start(
        &self,
        cluster: &ClusterState,
        placement: &Placement,
        max: usize,
    ) -> Result<ShardInfo, CreateError> {
        let (kind, id, params) = (&placement.kind, &placement.shard_id, &placement.params);
        match cluster
            .dir
            .claim(placement, max)
            .map_err(CreateError::Cluster)?
        {
            Claim::Claimed => {}
            Claim::Taken => return Err(CreateError::Exists(id.to_string())),
            Claim::LimitReached => {
                return Err(CreateError::LimitReached {
                    kind: kind.to_string(),
                    max,
                })
            }
        }
        self.create_local(kind, id, params, None, Some(placement.epoch))
            .inspect_err(|_| {
                let _ = cluster.dir.release(id, &cluster.me.id, placement.epoch);
            })
    }

    /// Start the shard in this process, from `state` when given. In a
    /// cluster, `epoch` is the lease epoch it is placed under, and it starts
    /// only while that lease is current.
    fn create_local(
        &self,
        kind: &str,
        id: &str,
        params: &serde_json::Value,
        state: Option<&[u8]>,
        epoch: Option<i64>,
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
        // In a cluster the directory counts every machine's shards (see
        // `create_claimed`); here, count only running shards, so a stopped
        // one waiting for the sweep does not hold a slot.
        if self.cluster.get().is_none() {
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
        }
        let sim = match state {
            Some(state) => spec.restore(id, params, state),
            None => spec.instantiate(id, params),
        }
        .map_err(|why| {
            // The lease ran out during init or restore, or changed since this
            // start was authorized: the module did not refuse, so the start
            // can be tried again. A module's own errors always carry a
            // prefix, so it cannot produce the bare lease message.
            let lease_changed = self
                .cluster
                .get()
                .is_some_and(|c| Some(c.lease.lock().unwrap().epoch) != epoch);
            if why == LEASE_LAPSED || lease_changed {
                lease_error()
            } else {
                CreateError::Init(why)
            }
        })?;
        let shard = Shard::new(id, sim, spec.config.clone());
        let info = ShardInfo {
            id: id.to_string(),
            kind: kind.to_string(),
            tick: shard.tick_number(),
            subscribers: 0,
            running: true,
            error: None,
            machine: self.cluster.get().map(|c| c.me.id.clone()),
        };
        // The lease check and the start are one step under the lease lock,
        // which the fence takes to stop everything: a shard never starts
        // after the fence ran.
        let _lease = match self.cluster.get() {
            Some(c) => {
                let lease = c.lease.lock().unwrap();
                let current = lease.startable();
                let Some(epoch) = epoch.filter(|&e| current && e == lease.epoch) else {
                    return Err(lease_error());
                };
                c.owned.lock().unwrap().insert(id.to_string(), epoch);
                Some(lease)
            }
            None => None,
        };
        self.kind_of
            .write()
            .unwrap()
            .insert(id.to_string(), kind.to_string());
        self.idle_since.lock().unwrap().remove(id);
        self.registry.insert(shard);
        tracing::info!(
            "[shard {id}] started ({kind}{})",
            if state.is_some() {
                ", from saved state"
            } else {
                ""
            }
        );
        Ok(info)
    }

    /// Stop and remove a shard. Its subscribers' connections close. On
    /// several machines, the machine that runs it stops it.
    pub fn stop(&self, id: &str) -> bool {
        let Some(c) = self.cluster.get() else {
            return self.registry.get(id).is_some() && self.stop_local(id);
        };
        let own = c.own_lock.lock().unwrap();
        if self.registry.get(id).is_some() {
            let stopped = self.stop_local(id);
            c.saved_at.lock().unwrap().remove(id);
            let held = c.owned.lock().unwrap().remove(id);
            if let Some(epoch) = held {
                if let Err(e) = c.dir.release(id, &c.me.id, epoch) {
                    tracing::warn!("[shard {id}] could not release its placement: {e}");
                }
                return stopped;
            }
            // A copy the fence stopped: its placement is from an earlier
            // lease, and is removed below.
        }
        let placement = match c.dir.placement(id) {
            Ok(Some(p)) => p,
            Ok(None) => return false,
            Err(e) => {
                tracing::warn!("[shard {id}] stop: directory lookup failed: {e}");
                return false;
            }
        };
        let owner = (placement.machine_id != c.me.id)
            .then(|| c.live(&placement.machine_id))
            .flatten()
            .filter(|m| m.epoch == placement.epoch);
        match owner {
            // Its machine runs it: that machine stops it.
            Some(Machine {
                id: machine,
                address: Some(address),
                ..
            }) => {
                drop(own);
                let op = RemoteOp::Stop { id: id.to_string() };
                match shard_cluster::call(&machine, &address, &op) {
                    Ok(RemoteReply::Ok(v)) => v.as_bool().unwrap_or(false),
                    Ok(RemoteReply::Err { message, .. }) | Err(message) => {
                        tracing::warn!(
                            "[shard {id}] stop on {} failed: {message}",
                            placement.machine_id
                        );
                        false
                    }
                }
            }
            // Placed here but not running (it failed to start, or it is from
            // an earlier lease), on a dead machine, or on a machine with no
            // address: remove exactly that placement, so no machine starts
            // it again. A machine that took it over meanwhile keeps it.
            _ => c
                .dir
                .release(id, &placement.machine_id, placement.epoch)
                .unwrap_or(false),
        }
    }

    /// Stop and remove a shard running in this process.
    fn stop_local(&self, id: &str) -> bool {
        // Serialized with create and the sweep, so a stop never removes the
        // bookkeeping of a shard created under the same id meanwhile.
        let _guard = self.create_lock.lock().unwrap();
        let removed = self.registry.remove(id);
        self.kind_of.write().unwrap().remove(id);
        self.idle_since.lock().unwrap().remove(id);
        removed
    }

    /// A shard, on this machine or (in a cluster) any live one.
    pub fn info(&self, id: &str) -> Option<ShardInfo> {
        if let Some(info) = self.info_local(id) {
            return Some(info);
        }
        let c = self.cluster.get()?;
        let placement = c.dir.placement(id).ok()??;
        let not_running = |why: String| ShardInfo {
            id: id.to_string(),
            kind: placement.kind.clone(),
            tick: 0,
            subscribers: 0,
            running: false,
            error: Some(why),
            machine: Some(placement.machine_id.clone()),
        };
        if let Some(why) = &placement.failed {
            return Some(not_running(format!(
                "machine {} could not start it: {why}",
                placement.machine_id
            )));
        }
        if placement.machine_id == c.me.id {
            // Placed here and not running yet: this machine starts it on its
            // next round.
            return Some(not_running("starting on this machine".into()));
        }
        match c.live(&placement.machine_id) {
            Some(Machine {
                id: machine,
                address: Some(address),
                ..
            }) => {
                match shard_cluster::call(&machine, &address, &RemoteOp::Get { id: id.to_string() })
                {
                    Ok(RemoteReply::Ok(v)) => serde_json::from_value(v).ok(),
                    _ => None,
                }
            }
            Some(_) => None,
            // On a dead machine: failover starts it again soon.
            None => Some(not_running(format!(
                "machine {} stopped responding; the shard is starting again on another machine",
                placement.machine_id
            ))),
        }
    }

    /// A shard running in this process.
    fn info_local(&self, id: &str) -> Option<ShardInfo> {
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
            machine: self.cluster.get().map(|c| c.me.id.clone()),
        })
    }

    /// Every shard: this machine's, and in a cluster every live machine's.
    pub fn list(&self) -> Vec<ShardInfo> {
        let mut out = self.list_local();
        if let Some(c) = self.cluster.get() {
            if let Ok(live) = c.dir.live_machines() {
                for m in live.iter().filter(|m| m.id != c.me.id) {
                    let Some(address) = &m.address else { continue };
                    match shard_cluster::call(&m.id, address, &RemoteOp::List) {
                        Ok(RemoteReply::Ok(v)) => out.extend(
                            serde_json::from_value::<Vec<ShardInfo>>(v).unwrap_or_default(),
                        ),
                        other => tracing::warn!("[shards] list on {} failed: {other:?}", m.id),
                    }
                }
            }
            out.sort_by(|a, b| a.id.cmp(&b.id));
            out.dedup_by(|a, b| a.id == b.id);
        }
        out
    }

    fn list_local(&self) -> Vec<ShardInfo> {
        let mut ids = self.registry.ids();
        ids.sort();
        ids.iter().filter_map(|id| self.info_local(id)).collect()
    }

    /// Stop every shard, for a graceful shutdown. In a cluster, each held
    /// shard saves its state after its last tick, then this machine leaves
    /// the directory, so the others start its shards at once from their
    /// latest state.
    pub fn stop_all(&self) {
        self.stopped.store(true, Ordering::Release);
        let Some(c) = self.cluster.get() else {
            for id in self.registry.ids() {
                self.stop(&id);
            }
            return;
        };
        // No create, adoption, or stop runs from here on, and no shard
        // starts here again.
        let _own = c.own_lock.lock().unwrap();
        c.lease.lock().unwrap().closed = true;
        // Take the held shards with their handles, under the create lock the
        // sweep releases under: the sweep releases only held shards, so none
        // of these loses its placement, and the handles outlive the sweep
        // removing them from the registry.
        let held: Vec<(String, i64, Arc<Shard<WasmSim>>, bool)> = {
            let _create = self.create_lock.lock().unwrap();
            let owned: Vec<(String, i64)> = c.owned.lock().unwrap().drain().collect();
            owned
                .into_iter()
                .filter_map(|(id, epoch)| {
                    let shard = self.registry.get(&id)?;
                    let saves = self.saves_state(&id);
                    Some((id, epoch, shard, saves))
                })
                .collect()
        };
        for id in self.registry.ids() {
            if let Some(shard) = self.registry.get(&id) {
                shard.stop();
            }
        }
        // The heartbeat thread keeps renewing the lease until the final
        // saves are done: they run under it.
        for (id, epoch, shard, saves) in &held {
            shard.stop_and_wait();
            if *saves {
                self.save_shard(c, id, *epoch, shard);
            }
        }
        for id in self.registry.ids() {
            self.stop_local(&id);
        }
        let _beat = c.beat_lock.lock().unwrap();
        c.left.store(true, Ordering::Release);
        let epoch = c.lease.lock().unwrap().epoch;
        if let Err(e) = c.dir.leave(&c.me.id, epoch) {
            tracing::warn!("[shards] could not leave the directory: {e}");
        }
    }

    // -- Several machines ------------------------------------------------

    /// Run shards on several machines through the directory in Postgres.
    /// Starts three threads: one renews this machine's lease, one stops
    /// every shard when the lease lapses, and one saves shard state, stops
    /// shards no longer placed here, and starts orphaned shards (their
    /// machine died, left, restarted, or lost its lease) whose new home is
    /// this machine.
    pub fn attach_cluster(self: &Arc<Self>, dir: PgShardDirectory, me: MachineConfig) {
        let save_every = std::env::var("PYLON_SHARD_SAVE_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|&s| s > 0)
            .map(Duration::from_secs)
            .unwrap_or(Duration::from_secs(5));
        tracing::info!(
            "[shards] machine {} in the shard directory (address {}, capacity {})",
            me.id,
            me.address.as_deref().unwrap_or("none"),
            me.capacity
        );
        let clock = Arc::new(LeaseClock::default());
        for kind in self.kinds.values() {
            kind.set_lease_clock(Arc::clone(&clock));
        }
        if self
            .cluster
            .set(ClusterState {
                dir,
                me,
                save_every,
                lease: Mutex::new(Lease {
                    epoch: shard_cluster::new_epoch(),
                    until: None,
                    closed: false,
                }),
                clock: Arc::clone(&clock),
                save_lock: Mutex::new(()),
                id_conflict: AtomicBool::new(false),
                left: AtomicBool::new(false),
                beat_lock: Mutex::new(()),
                owned: Mutex::new(HashMap::new()),
                own_lock: Mutex::new(()),
                saved_at: Mutex::new(HashMap::new()),
            })
            .is_err()
        {
            return;
        }
        // Runs through shutdown until the final saves are done.
        let beat: Weak<Self> = Arc::downgrade(self);
        let _ = std::thread::Builder::new()
            .name("pylon-shard-heartbeat".into())
            .spawn(move || loop {
                match beat.upgrade() {
                    Some(host)
                        if !host
                            .cluster
                            .get()
                            .is_some_and(|c| c.left.load(Ordering::Acquire)) =>
                    {
                        host.heartbeat()
                    }
                    _ => return,
                }
                std::thread::sleep(shard_cluster::HEARTBEAT);
            });
        let fence: Weak<Self> = Arc::downgrade(self);
        let _ = std::thread::Builder::new()
            .name("pylon-shard-fence".into())
            .spawn(move || loop {
                match fence.upgrade() {
                    Some(host) if !host.stopped.load(Ordering::Acquire) => host.fence(),
                    _ => return,
                }
                std::thread::sleep(FENCE_EVERY);
            });
        let round: Weak<Self> = Arc::downgrade(self);
        let _ = std::thread::Builder::new()
            .name("pylon-shard-cluster".into())
            .spawn(move || loop {
                match round.upgrade() {
                    Some(host) if !host.stopped.load(Ordering::Acquire) => host.cluster_round(),
                    _ => return,
                }
                std::thread::sleep(Duration::from_secs(1));
            });
    }

    /// The machine id when the host runs in a cluster.
    pub fn machine_id(&self) -> Option<&str> {
        self.cluster.get().map(|c| c.me.id.as_str())
    }

    /// Renew the lease. The lease runs from when the heartbeat was SENT:
    /// the directory writes it later than that, so every other machine
    /// counts this one live for longer than this machine runs shards.
    fn heartbeat(&self) {
        let Some(c) = self.cluster.get() else { return };
        let _beat = c.beat_lock.lock().unwrap();
        if c.left.load(Ordering::Acquire) {
            return;
        }
        let epoch = c.lease.lock().unwrap().epoch;
        let sent = Instant::now();
        match c.dir.heartbeat(&c.me, epoch) {
            Ok(true) => {
                if c.id_conflict.swap(false, Ordering::Relaxed) {
                    tracing::info!("[shards] machine id {} is free; joining", c.me.id);
                }
                let mut lease = c.lease.lock().unwrap();
                if let Some(until) = lease.renew(epoch, sent, Instant::now()) {
                    c.clock.set(Some(until));
                }
            }
            Ok(false) => {
                if !c.id_conflict.swap(true, Ordering::Relaxed) {
                    tracing::warn!(
                        "[shards] machine id {} is held by another live process; waiting until it \
                         leaves or is silent for {:?}",
                        c.me.id,
                        shard_cluster::DEAD_AFTER
                    );
                }
            }
            Err(e) => tracing::warn!("[shards] heartbeat failed: {e}"),
        }
    }

    /// Stop every shard when the lease has lapsed, and take a new epoch.
    /// Other machines may start these shards once the directory counts this
    /// machine dead; by then they are stopped here, with no tick running.
    fn fence(&self) {
        let Some(c) = self.cluster.get() else { return };
        let stopping: Vec<(String, Arc<Shard<WasmSim>>)> = {
            let mut lease = c.lease.lock().unwrap();
            if lease.until.is_none_or(|until| Instant::now() < until) {
                return;
            }
            lease.until = None;
            lease.epoch = shard_cluster::new_epoch();
            // A guest call from here on is refused, and one running now is
            // interrupted.
            c.clock.set(None);
            // Every held shard, running or not: one whose guest call was
            // just refused has stopped on its own already.
            let held: Vec<String> = c.owned.lock().unwrap().drain().map(|(id, _)| id).collect();
            held.into_iter()
                .filter_map(|id| self.registry.get(&id).map(|s| (id, s)))
                .collect()
        };
        for (_, shard) in &stopping {
            shard.stop();
        }
        for (id, shard) in &stopping {
            shard.stop_and_wait();
            tracing::warn!(
                "[shard {id}] stopped: this machine's lease on the shard directory lapsed"
            );
        }
    }

    /// One pass of the cluster thread.
    fn cluster_round(&self) {
        let Some(c) = self.cluster.get() else { return };
        if c.current_epoch().is_none() {
            return;
        }
        // What runs here against the directory: a shard whose placement is
        // gone or not ours (another machine released it) stops.
        {
            let _own = c.own_lock.lock().unwrap();
            let placed = match c.dir.placements_on(&c.me.id) {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!("[shards] directory read failed: {e}");
                    return;
                }
            };
            let valid: std::collections::HashSet<(&str, i64)> = placed
                .iter()
                .map(|p| (p.shard_id.as_str(), p.epoch))
                .collect();
            let held: Vec<(String, i64)> = c
                .owned
                .lock()
                .unwrap()
                .iter()
                .map(|(id, e)| (id.clone(), *e))
                .collect();
            for (id, epoch) in held {
                if !valid.contains(&(id.as_str(), epoch)) {
                    tracing::warn!("[shard {id}] no longer placed here; stopping this copy");
                    c.owned.lock().unwrap().remove(&id);
                    self.stop_local(&id);
                }
            }
            // Placed here under the current lease with nothing running: an
            // adoption that failed after its takeover (a directory error, or
            // the lease briefly too short to start). Start it again.
            if let Some(epoch) = c.current_epoch() {
                for p in &placed {
                    let held = c.owned.lock().unwrap().contains_key(&p.shard_id);
                    if p.epoch != epoch || held || p.failed.is_some() {
                        continue;
                    }
                    if self
                        .registry
                        .get(&p.shard_id)
                        .is_some_and(|s| s.is_running())
                    {
                        continue;
                    }
                    tracing::info!(
                        "[shard {}] placed here and not running; starting it",
                        p.shard_id
                    );
                    if let Err(e) = self.adopt(p, epoch) {
                        tracing::error!("[shard {}] could not start: {e}", p.shard_id);
                    }
                }
            }
        }
        // Save state that is due.
        let held: Vec<(String, i64)> = c
            .owned
            .lock()
            .unwrap()
            .iter()
            .map(|(id, e)| (id.clone(), *e))
            .collect();
        for (id, epoch) in held {
            let due = c
                .saved_at
                .lock()
                .unwrap()
                .get(&id)
                .is_none_or(|at| at.elapsed() >= c.save_every);
            if due {
                self.save_one(c, &id, epoch);
            }
        }
        // Failover: each orphan has one new home; this machine takes the
        // ones whose home it is.
        let round_started = Instant::now();
        let Ok(live) = c.dir.live_machines() else {
            return;
        };
        let Some(epoch) = c.current_epoch() else {
            return;
        };
        if !live.iter().any(|m| m.id == c.me.id && m.epoch == epoch) {
            return;
        }
        let Ok(orphans) = c.dir.orphans() else {
            return;
        };
        for orphan in orphans {
            if round_started.elapsed() >= Duration::from_secs(3) {
                break;
            }
            if home(&orphan.shard_id, &live).map(|m| m.id.as_str()) != Some(c.me.id.as_str()) {
                continue;
            }
            let _own = c.own_lock.lock().unwrap();
            let Some(epoch) = c.current_epoch() else {
                return;
            };
            match c.dir.take_over(&orphan, &c.me.id, epoch) {
                Ok(true) => {
                    if orphan.machine_id == c.me.id {
                        tracing::info!(
                            "[shard {}] placed here under an earlier lease; starting it again",
                            orphan.shard_id
                        );
                    } else {
                        tracing::info!(
                            "[shard {}] machine {} is dead; starting it here",
                            orphan.shard_id,
                            orphan.machine_id
                        );
                    }
                    if orphan.failed.is_none() {
                        match self.adopt(&orphan, epoch) {
                            Ok(_) => tracing::info!(
                                "[shard {}] took over from machine {}",
                                orphan.shard_id,
                                orphan.machine_id
                            ),
                            Err(e) => {
                                tracing::error!("[shard {}] could not start: {e}", orphan.shard_id)
                            }
                        }
                    }
                }
                Ok(false) => {}
                Err(e) => tracing::warn!("[shard {}] take over failed: {e}", orphan.shard_id),
            }
        }
        let _ = c.dir.prune_machines();
    }

    /// Save one local shard's state to the directory (see
    /// [`WasmShardHost::save_shard`]).
    fn save_one(&self, c: &ClusterState, id: &str, epoch: i64) {
        if !self.saves_state(id) {
            return;
        }
        if let Some(shard) = self.registry.get(id) {
            self.save_shard(c, id, epoch, &shard);
        }
    }

    /// True when shard `id`'s module keeps state.
    fn saves_state(&self, id: &str) -> bool {
        self.kind_of
            .read()
            .unwrap()
            .get(id)
            .is_some_and(|k| self.kinds[k].saves_state())
    }

    /// Save `shard`'s state (its module keeps state), when this machine
    /// still holds it under `epoch`. The capture and the write run under the
    /// save lock, so saves land in the order they were captured.
    fn save_shard(&self, c: &ClusterState, id: &str, epoch: i64, shard: &Shard<WasmSim>) {
        let _save = c.save_lock.lock().unwrap();
        c.saved_at
            .lock()
            .unwrap()
            .insert(id.to_string(), Instant::now());
        match shard.with_state(|sim| sim.save()) {
            Ok(Some(state)) => match c.dir.save_state(id, &c.me.id, epoch, &state) {
                Ok(true) => {}
                Ok(false) => tracing::warn!("[shard {id}] not saved: no longer placed here"),
                Err(e) => tracing::warn!("[shard {id}] save failed: {e}"),
            },
            Ok(None) => {}
            Err(e) => tracing::warn!("[shard {id}] save failed: {e}"),
        }
    }

    /// Start orphan `p`, which this machine just took over under `epoch`,
    /// from its saved state when there is one. The caller holds the own
    /// lock. A shard whose module cannot start it is marked failed, with its
    /// state kept, so an operator can see why; a lapsed lease or a failed
    /// directory read leaves it for a later round.
    fn adopt(&self, p: &Placement, epoch: i64) -> Result<ShardInfo, String> {
        let c = self.cluster.get().expect("adopt runs in a cluster");
        let Some(state) = c.dir.load_owned_state(&p.shard_id, &c.me.id, epoch)? else {
            return Err("no longer placed here".into());
        };
        let saves = self.kinds.get(&p.kind).is_some_and(|k| k.saves_state());
        let started = self.create_local(
            &p.kind,
            &p.shard_id,
            &p.params,
            state.as_deref().filter(|_| saves),
            Some(epoch),
        );
        match started {
            Ok(info) => Ok(info),
            Err(e @ CreateError::Cluster(_)) => Err(e.to_string()),
            Err(e) => {
                let why = e.to_string();
                let _ = c.dir.mark_failed(&p.shard_id, &c.me.id, epoch, &why);
                Err(why)
            }
        }
    }

    /// Run an operation another machine sent, on this machine only.
    pub fn run_remote(&self, op: RemoteOp) -> RemoteReply {
        let err = |e: CreateError| RemoteReply::Err {
            code: e.code().to_string(),
            message: e.to_string(),
        };
        let json = |info: &ShardInfo| serde_json::to_value(info).unwrap_or_default();
        let Some(c) = self.cluster.get() else {
            return RemoteReply::Err {
                code: "SHARD_CLUSTER_ERROR".into(),
                message: "this machine is not in the shard directory".into(),
            };
        };
        match op {
            RemoteOp::Create {
                kind,
                id,
                params,
                pinned,
            } => {
                if !self.kinds.contains_key(&kind) {
                    return err(CreateError::UnknownKind(kind));
                }
                if let Err(e) = validate_shard_id(&id) {
                    return err(e);
                }
                match self.create_claimed(&kind, &id, &params, pinned) {
                    Ok(info) => RemoteReply::Ok(json(&info)),
                    Err(e) => err(e),
                }
            }
            // Stop what runs here, or forget a placement on this machine
            // with nothing running. Never passed on again.
            RemoteOp::Stop { id } => {
                let placed_here = c
                    .dir
                    .placement(&id)
                    .ok()
                    .flatten()
                    .is_some_and(|p| p.machine_id == c.me.id);
                let stopped = if self.registry.get(&id).is_some() || placed_here {
                    self.stop(&id)
                } else {
                    false
                };
                RemoteReply::Ok(serde_json::json!(stopped))
            }
            RemoteOp::Get { id } => RemoteReply::Ok(
                self.info_local(&id)
                    .map(|i| json(&i))
                    .unwrap_or(serde_json::Value::Null),
            ),
            RemoteOp::List => {
                RemoteReply::Ok(serde_json::to_value(self.list_local()).unwrap_or_default())
            }
        }
    }

    /// Where shard `id` runs, for routing a client that reached this machine.
    pub fn locate(&self, id: &str) -> pylon_realtime::ShardLocation {
        use pylon_realtime::ShardLocation;
        if self.registry.get(id).is_some_and(|s| s.is_running()) {
            return ShardLocation::Local;
        }
        let Some(c) = self.cluster.get() else {
            return ShardLocation::Unknown;
        };
        let Ok(Some(p)) = c.dir.placement(id) else {
            return ShardLocation::Unknown;
        };
        if p.machine_id == c.me.id {
            return ShardLocation::Unknown;
        }
        match c.live(&p.machine_id).filter(|m| m.epoch == p.epoch) {
            Some(m) => ShardLocation::Remote {
                machine_id: m.id,
                address: m.address,
                // fly-replay works only between Fly machines.
                fly_replay: c.me.fly_instance.as_ref().and(m.fly_instance),
            },
            None => ShardLocation::Unknown,
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
        let before: HashMap<String, Arc<Shard<WasmSim>>> = self
            .registry
            .ids()
            .into_iter()
            .filter_map(|id| self.registry.get(&id).map(|s| (id, s)))
            .collect();
        if self.registry.sweep_finished() == 0 {
            return;
        }
        let mut kind_of = self.kind_of.write().unwrap();
        let mut idle = self.idle_since.lock().unwrap();
        for (id, shard) in before
            .iter()
            .filter(|(id, _)| self.registry.get(id).is_none())
        {
            kind_of.remove(id);
            idle.remove(id);
            tracing::info!("[shard {id}] removed after it stopped");
            // It ended (finished, idle, or failed): forget its placement,
            // unless another machine runs it now.
            // `release` matches this machine, so a placement another
            // machine took over stays.
            // Only a held shard: one the fence or shutdown stopped keeps
            // its placement.
            if let Some(c) = self.cluster.get() {
                c.saved_at.lock().unwrap().remove(id);
                // A shard stops on its own when its lease lapses (its guest
                // calls are refused) before the fence runs: that is not an
                // ending, and it keeps its placement.
                let lapsed = shard
                    .with_state(|sim| sim.failure())
                    .is_some_and(|why| why == LEASE_LAPSED);
                let held = {
                    let lease = c.lease.lock().unwrap();
                    let in_force = lease.until.is_some_and(|until| Instant::now() < until);
                    let held = c.owned.lock().unwrap().remove(id);
                    held.filter(|&epoch| in_force && !lapsed && epoch == lease.epoch)
                };
                if let Some(epoch) = held {
                    if let Err(e) = c.dir.release(id, &c.me.id, epoch) {
                        tracing::warn!("[shard {id}] could not release its placement: {e}");
                    }
                }
            }
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

    fn kind(&self, id: &str) -> Option<String> {
        self.kind_of.read().unwrap().get(id).cloned()
    }

    fn failure(&self, id: &str) -> Option<String> {
        self.registry.get(id)?.with_state(|s| s.failure())
    }

    fn stop(&self, id: &str) -> bool {
        WasmShardHost::stop(self, id)
    }

    fn locate(&self, id: &str) -> pylon_realtime::ShardLocation {
        WasmShardHost::locate(self, id)
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
            host.create_on(kind, id()?, &params, req.machine.as_deref())
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

#[cfg(test)]
mod lease_tests {
    use super::*;
    use pylon_storage::pg_datastore::PgPool;

    fn wait_for(what: &str, secs: u64, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(secs);
        while !done() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// The whole fence on one machine, against Postgres: the lease lapses,
    /// the shard stops and keeps its placement and state through the sweep,
    /// and the machine takes it back under a new epoch from saved state.
    #[test]
    fn a_lapsed_shard_keeps_its_placement_and_comes_back_from_saved_state() {
        let Ok(url) = std::env::var("PYLON_TEST_PG_URL") else {
            eprintln!("skipping: PYLON_TEST_PG_URL not set");
            return;
        };
        let _serial = crate::shard_cluster::tests::DIRECTORY_TESTS
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let pool = PgPool::connect(&url, 4, Duration::from_secs(5)).expect("test Postgres pool");
        let dir = PgShardDirectory::open(Arc::clone(&pool)).expect("directory");
        let check = PgShardDirectory::open(pool).expect("directory");
        let kind = WasmShardKind::compile(
            "arena",
            include_bytes!("../../../examples/shard-arena/shards/arena.wasm"),
            ShardConfig {
                tick_rate_hz: 20,
                ..ShardConfig::default()
            },
            WasmLimits::default(),
        )
        .unwrap();
        let host = WasmShardHost::new(vec![kind]);
        let run = pylon_cluster::new_instance_id();
        let shard = format!("fence-{run}");
        host.attach_cluster(
            dir,
            MachineConfig {
                id: format!("m-{run}"),
                address: None,
                capacity: 10,
                fly_instance: None,
            },
        );
        let c = host.cluster.get().unwrap();
        wait_for("a lease", 10, || c.current_epoch().is_some());
        host.create_on(
            "arena",
            &shard,
            &serde_json::json!({ "width": 800, "height": 600 }),
            Some(&format!("m-{run}")),
        )
        .unwrap();
        let first = c.lease.lock().unwrap().epoch;
        host.save_one(c, &shard, first);

        let players = |shard: &Shard<WasmSim>| {
            String::from_utf8(shard.with_state(|sim| sim.save()).unwrap().unwrap()).unwrap()
        };
        let running = |id: &str| host.registry.get(id).filter(|s| s.is_running());

        // The guests' clock says the lease ended while the lease itself
        // was renewed (the order a late heartbeat reply produces). The
        // shard stops on its own; the sweep must not take that for an
        // ending, and the machine starts it again from saved state.
        let saved = r#"[{"id":"p1","x":1.0,"y":2.0,"tx":1.0,"ty":2.0,"hue":7}]"#;
        let machine = format!("m-{run}");
        assert!(check
            .save_state(&shard, &machine, first, saved.as_bytes())
            .unwrap());
        c.clock.set(None);
        wait_for("the shard to stop", 5, || running(&shard).is_none());
        c.clock.set(c.lease.lock().unwrap().until);
        std::thread::sleep(Duration::from_millis(2500));
        assert!(check.placement(&shard).unwrap().is_some(), "not released");
        wait_for("the shard back", 10, || running(&shard).is_some());
        assert!(players(&running(&shard).unwrap()).contains("\"p1\""));

        // The lease runs out (the directory stopped answering).
        let saved = r#"[{"id":"p2","x":3.0,"y":4.0,"tx":3.0,"ty":4.0,"hue":9}]"#;
        assert!(check
            .save_state(&shard, &machine, first, saved.as_bytes())
            .unwrap());
        c.lease.lock().unwrap().until = Some(Instant::now());
        wait_for("the fence", 5, || {
            !host.registry.get(&shard).is_some_and(|s| s.is_running())
        });
        assert_ne!(c.lease.lock().unwrap().epoch, first, "a new epoch");
        // Past a sweep: the placement and its state are still there.
        std::thread::sleep(Duration::from_millis(2500));
        let placed = check
            .placement(&shard)
            .unwrap()
            .expect("the placement stays");
        assert_eq!(placed.epoch, first);
        assert!(check
            .load_owned_state(&shard, &placed.machine_id, first)
            .unwrap()
            .expect("still ours")
            .is_some());

        // The old epoch goes silent; the machine takes the shard back, from
        // the state saved under the old epoch.
        wait_for("the shard back", 30, || running(&shard).is_some());
        let placed = check.placement(&shard).unwrap().unwrap();
        assert_eq!(placed.epoch, c.lease.lock().unwrap().epoch);
        let now = players(&running(&shard).unwrap());
        assert!(now.contains("\"p2\"") && !now.contains("\"p1\""), "{now}");

        assert!(host.stop(&shard));
        assert_eq!(check.placement(&shard).unwrap(), None);
        host.stop_all();
    }

    fn lease(until: Option<Instant>) -> Lease {
        Lease {
            epoch: 7,
            until,
            closed: false,
        }
    }

    #[test]
    fn a_lease_renews_from_the_send_time_and_never_after_it_lapsed() {
        let t0 = Instant::now();
        let s = Duration::from_secs;
        let end = |sent: Instant| LeaseClock::floor(sent + shard_cluster::FENCE_AFTER);

        // First heartbeat of the epoch: the lease runs from the send time,
        // rounded down to the guests' clock.
        let mut l = lease(None);
        assert_eq!(l.renew(7, t0, t0 + s(1)), Some(end(t0)));
        // A later one extends it; an earlier-sent reply never shortens it.
        assert_eq!(l.renew(7, t0 + s(2), t0 + s(3)), Some(end(t0 + s(2))));
        assert_eq!(l.renew(7, t0 + s(1), t0 + s(4)), Some(end(t0 + s(2))));

        // A reply for another epoch authorizes nothing.
        assert_eq!(l.renew(8, t0 + s(4), t0 + s(4)), None);

        // A reply that arrives after its own lease would have ended.
        let mut l = lease(Some(t0 + s(10)));
        assert_eq!(l.renew(7, t0, t0 + s(6)), None);
        assert_eq!(l.until, Some(t0 + s(10)));

        // Once the lease has run out, even a fresh heartbeat of the same
        // epoch does not revive it: the fence must move to a new epoch.
        let mut l = lease(Some(end(t0)));
        assert_eq!(l.renew(7, t0 + s(5), t0 + s(6)), None);
        assert_eq!(l.until, Some(end(t0)));

        // The lease and the guests' clock expire together, to the
        // millisecond: no reply revives a lease the guests saw expire.
        let until = end(t0);
        let clock = LeaseClock::default();
        clock.set(Some(until));
        let mut l = lease(Some(until));
        for step in 0..3u64 {
            let now = until + Duration::from_micros(step * 400);
            assert!(clock_expired_at(&clock, now));
            assert_eq!(l.renew(7, now, now), None);
        }
    }

    fn clock_expired_at(clock: &LeaseClock, now: Instant) -> bool {
        LeaseClock::millis(now) >= clock.until_ms.load(Ordering::Acquire)
    }
}
