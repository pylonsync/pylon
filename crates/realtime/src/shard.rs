//! The core [`Shard`] abstraction.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{de::DeserializeOwned, Serialize};

use crate::outbound::{OutboundConfig, OutboundQueue};
use crate::subscriber::{Subscriber, SubscriberId};
use crate::wire::InputRejection;

// ---------------------------------------------------------------------------
// ShardAuth — auth context passed to authorization hooks
// ---------------------------------------------------------------------------

/// Auth context for shard operations.
///
/// Mirrors the HTTP auth context but shaped for shard-level checks.
/// Implementations of `SimState::authorize_subscribe` and
/// `SimState::authorize_input` use it to decide whether a subscriber
/// can join a match or submit a given input.
#[derive(Debug, Clone)]
pub struct ShardAuth {
    pub user_id: Option<String>,
    pub is_admin: bool,
}

impl ShardAuth {
    pub fn anonymous() -> Self {
        Self {
            user_id: None,
            is_admin: false,
        }
    }
}

// ---------------------------------------------------------------------------
// SimState — user-defined game/simulation logic
// ---------------------------------------------------------------------------

/// User-defined simulation state for a shard.
///
/// Implemented once per game type / workload. The [`Shard`] owns an instance
/// of `Self` and drives it through its input queue and tick loop.
///
/// # Contract
///
/// - `apply_input` is called synchronously for every queued input, in order.
///   Implementations must validate inputs — the shard does not.
/// - `tick` is called at `config.tick_rate_hz` with the elapsed duration.
///   It may be called with `dt = 0` on the first tick.
/// - `snapshot` must be cheap — it runs after every tick. Use lazy
///   computation if needed; clone only what clients see.
/// - `snapshot_for` lets each subscriber get a filtered view (area-of-interest,
///   fog-of-war, role-based visibility). Default: same snapshot for everyone.
pub trait SimState: Send + 'static {
    type Input: DeserializeOwned + Send + 'static;
    type Snapshot: Serialize + Send + Clone + 'static;
    type Error: std::fmt::Debug + Send + 'static;

    /// Apply a player/client input.
    ///
    /// Called for each queued input on every tick, in FIFO order.
    /// Returning `Err` logs the error but doesn't halt the simulation.
    fn apply_input(
        &mut self,
        subscriber_id: &SubscriberId,
        input: Self::Input,
        now: Instant,
    ) -> Result<(), Self::Error>;

    /// Advance simulation time by `dt`.
    fn tick(&mut self, dt: Duration);

    /// Produce a broadcast snapshot.
    fn snapshot(&self) -> Self::Snapshot;

    /// Produce a per-subscriber snapshot (for area-of-interest filtering,
    /// fog-of-war, etc.). Default: same snapshot for all.
    fn snapshot_for(&self, _subscriber_id: &SubscriberId) -> Self::Snapshot {
        self.snapshot()
    }

    /// Return true when the shard should shut down (e.g. match ended).
    /// Called after every tick. Default: never ends.
    fn is_finished(&self) -> bool {
        false
    }

    /// Authorize a subscriber joining this shard. Return `Err(reason)` to reject.
    ///
    /// Default: require that the caller's `auth.user_id` matches the requested
    /// `subscriber_id`, OR the caller is admin. Previously this allowed any
    /// authenticated user to subscribe with any sid, letting Alice impersonate
    /// Bob and receive events intended for him. Apps that want looser coupling
    /// (e.g. spectator mode) should override this hook explicitly.
    fn authorize_subscribe(
        &self,
        subscriber_id: &SubscriberId,
        auth: &ShardAuth,
    ) -> Result<(), String> {
        if auth.is_admin {
            return Ok(());
        }
        match &auth.user_id {
            Some(uid) if uid == subscriber_id.as_str() => Ok(()),
            Some(_) => Err(format!(
                "subscriber id \"{}\" does not match authenticated user",
                subscriber_id.as_str()
            )),
            None => Err("authenticated user required".into()),
        }
    }

    /// Authorize an input before it enters the queue. Return `Err(reason)` to reject.
    ///
    /// Default: allow all. Override to enforce "subscriber may only move
    /// their own unit", cheat detection, etc.
    fn authorize_input(
        &self,
        _subscriber_id: &SubscriberId,
        _auth: &ShardAuth,
        _input: &Self::Input,
    ) -> Result<(), String> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// ShardConfig
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ShardConfig {
    /// Tick rate in Hz. `0` means event-driven (ticks only when inputs arrive).
    pub tick_rate_hz: u32,
    /// Pass `SimState::tick` a constant `dt` of `1 / tick_rate_hz` instead
    /// of the measured time since the last tick, so cooldowns and
    /// damage-over-time do not drift with server load and a replay
    /// reproduces the run. Ignored when `tick_rate_hz` is 0.
    pub fixed_timestep: bool,
    /// When the tick loop falls behind, run at most this many extra ticks
    /// back to back to catch up; further missed ticks are skipped and
    /// counted in [`Shard::overrun_ticks`].
    pub max_catch_up_ticks: u32,
    /// Max subscribers permitted. 0 = unlimited.
    pub max_subscribers: usize,
    /// Shut down the shard after this many consecutive empty ticks
    /// (no inputs, no subscribers). 0 = never.
    pub idle_ticks_before_shutdown: u32,
    /// Drop inputs if the shard's whole queue exceeds this. The last guard
    /// behind the per-subscriber limits below. 0 = unlimited (careful).
    pub max_input_queue: usize,
    /// Inputs one subscriber may have queued. Further inputs from that
    /// subscriber are dropped until the tick drains some. 0 = unlimited.
    pub max_queued_inputs_per_subscriber: usize,
    /// Inputs applied per subscriber per tick. The rest wait for the next
    /// tick, in order. 0 = unlimited.
    pub max_inputs_per_subscriber_per_tick: usize,
    /// Sustained inputs per second per subscriber (token bucket refill
    /// rate). 0 = unlimited.
    pub input_rate_per_subscriber: f64,
    /// Token bucket size: inputs a subscriber may send in a burst above the
    /// sustained rate.
    pub input_burst_per_subscriber: f64,
    /// Frames each queued subscriber's outbound queue holds before snapshots
    /// are dropped for newer ones.
    pub outbound_queue_frames: usize,
    /// Disconnect a subscriber whose outbound queue filled and whose writer
    /// then took no frame for this long.
    pub slow_subscriber_timeout: Duration,
    /// Snapshot format used on the wire.
    pub snapshot_format: crate::snapshot::SnapshotFormat,
}

impl Default for ShardConfig {
    fn default() -> Self {
        Self {
            tick_rate_hz: 20,
            fixed_timestep: true,
            max_catch_up_ticks: 5,
            max_subscribers: 256,
            idle_ticks_before_shutdown: 60 * 30, // 30s at 20Hz
            max_input_queue: 10_000,
            max_queued_inputs_per_subscriber: 256,
            max_inputs_per_subscriber_per_tick: 32,
            input_rate_per_subscriber: 120.0,
            input_burst_per_subscriber: 240.0,
            outbound_queue_frames: 64,
            slow_subscriber_timeout: Duration::from_secs(10),
            snapshot_format: crate::snapshot::SnapshotFormat::Json,
        }
    }
}

impl ShardConfig {
    /// The outbound queue settings for one subscriber.
    pub fn outbound(&self) -> OutboundConfig {
        OutboundConfig {
            max_frames: self.outbound_queue_frames,
            disconnect_after: self.slow_subscriber_timeout,
        }
    }
}

// ---------------------------------------------------------------------------
// ShardError
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum ShardError {
    Full,
    InputQueueFull,
    /// This subscriber is over its own input limits. Other subscribers are
    /// not affected.
    InputRateLimited,
    Stopped,
    SubscriberNotFound,
    Unauthorized(String),
    Other(String),
}

impl std::fmt::Display for ShardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Full => write!(f, "shard is at max subscribers"),
            Self::InputQueueFull => write!(f, "shard input queue is full"),
            Self::InputRateLimited => write!(f, "too many inputs from this subscriber"),
            Self::Stopped => write!(f, "shard is stopped"),
            Self::SubscriberNotFound => write!(f, "subscriber not found"),
            Self::Unauthorized(reason) => write!(f, "unauthorized: {reason}"),
            Self::Other(s) => write!(f, "{s}"),
        }
    }
}

impl std::error::Error for ShardError {}

// ---------------------------------------------------------------------------
// Pending input — bundles the input with its originator
// ---------------------------------------------------------------------------

struct PendingInput<I> {
    subscriber_id: SubscriberId,
    input: I,
    /// The client's sequence number, echoed back as the subscriber's ack
    /// (see [`crate::wire`]).
    seq: Option<u64>,
    received_at: Instant,
}

/// Per-subscriber input accounting.
struct SubscriberInputs {
    /// Inputs this subscriber has in the queue.
    queued: usize,
    /// Token bucket for `input_rate_per_subscriber`.
    tokens: f64,
    last_refill: Instant,
    /// Inputs dropped since the last log line for this subscriber.
    dropped_since_log: u64,
    last_log: Option<Instant>,
}

type InputRecorder<I> = Box<dyn Fn(u64, &SubscriberId, &I) + Send + Sync>;

/// Log dropped inputs for one subscriber at most this often.
const DROP_LOG_INTERVAL: Duration = Duration::from_secs(5);

/// The shard's input queue plus the per-subscriber accounting, under one
/// lock so the counts always match the queue.
struct InputQueue<I> {
    queue: VecDeque<PendingInput<I>>,
    per_subscriber: HashMap<SubscriberId, SubscriberInputs>,
}

// ---------------------------------------------------------------------------
// Shard
// ---------------------------------------------------------------------------

/// An isolated, authoritative simulation driven by a tick loop.
///
/// Each shard has its own lock, state, inputs, and subscribers. Shards
/// run independently — there is no shared state between them.
pub struct Shard<S: SimState> {
    id: String,
    config: ShardConfig,
    state: Mutex<S>,
    inputs: Mutex<InputQueue<S::Input>>,
    subscribers: Mutex<Vec<Arc<Subscriber<S::Snapshot>>>>,
    running: AtomicBool,
    /// Monotonically increasing tick number. Used for reconciliation and
    /// lockstep protocols.
    tick_no: Mutex<u64>,
    /// Monotonic input sequence counter (global per shard).
    input_seq: Mutex<u64>,
    /// Per subscriber, the highest `client_seq` processed so far (applied
    /// or rejected by `apply_input`). Sent in each snapshot frame.
    acks: Mutex<HashMap<SubscriberId, u64>>,
    created_at: Instant,
    last_input_at: Mutex<Instant>,
    last_tick_at: Mutex<Option<Instant>>,
    /// Count of consecutive idle ticks (no inputs, no subscribers).
    idle_ticks: Mutex<u32>,
    /// Optional hook: user callback invoked after each tick, while the
    /// state lock is held. See [`Shard::set_on_tick`].
    on_tick: Mutex<Option<Box<dyn Fn(&S, u64) + Send + Sync>>>,
    /// Optional hook: called with every input just before it is applied,
    /// with its tick number. See [`crate::ReplayLog::attach`].
    input_recorder: Mutex<Option<InputRecorder<S::Input>>>,
    /// Ticks the tick loop skipped because it fell too far behind.
    overrun_ticks: std::sync::atomic::AtomicU64,
}

impl<S: SimState> Shard<S> {
    /// Create a new shard with an initial simulation state.
    pub fn new(id: impl Into<String>, initial: S, config: ShardConfig) -> Arc<Self> {
        let now = Instant::now();
        Arc::new(Self {
            id: id.into(),
            config,
            state: Mutex::new(initial),
            inputs: Mutex::new(InputQueue {
                queue: VecDeque::new(),
                per_subscriber: HashMap::new(),
            }),
            subscribers: Mutex::new(Vec::new()),
            running: AtomicBool::new(true),
            tick_no: Mutex::new(0),
            input_seq: Mutex::new(0),
            acks: Mutex::new(HashMap::new()),
            created_at: now,
            last_input_at: Mutex::new(now),
            last_tick_at: Mutex::new(None),
            idle_ticks: Mutex::new(0),
            on_tick: Mutex::new(None),
            input_recorder: Mutex::new(None),
            overrun_ticks: std::sync::atomic::AtomicU64::new(0),
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn config(&self) -> &ShardConfig {
        &self.config
    }

    /// Register a callback that runs after every tick with the state and the
    /// tick number.
    ///
    /// It runs on the tick thread while the state lock is held, so it must
    /// not do I/O or anything else that can wait: the whole shard stops for
    /// as long as it runs. To save state, use
    /// [`crate::persist_every_ticks`], which copies the state here and
    /// writes it from a separate thread.
    pub fn set_on_tick(&self, callback: impl Fn(&S, u64) + Send + Sync + 'static) {
        *self.on_tick.lock().unwrap() = Some(Box::new(callback));
    }

    /// Call `record` with each input, its subscriber, and the tick it is
    /// applied on, just before it is applied. Runs under the state lock, so
    /// it must only copy the input. [`crate::ReplayLog::attach`] uses it.
    pub fn set_input_recorder(
        &self,
        record: impl Fn(u64, &SubscriberId, &S::Input) + Send + Sync + 'static,
    ) {
        *self.input_recorder.lock().unwrap() = Some(Box::new(record));
    }

    /// Ticks skipped so far because the tick loop fell more than
    /// `max_catch_up_ticks` behind.
    pub fn overrun_ticks(&self) -> u64 {
        self.overrun_ticks.load(Ordering::Relaxed)
    }

    pub(crate) fn record_overrun(&self, skipped: u64) {
        let total = self.overrun_ticks.fetch_add(skipped, Ordering::Relaxed) + skipped;
        tracing::warn!(
            "[realtime] shard {} fell behind and skipped {skipped} tick(s) ({total} so far)",
            self.id
        );
    }

    #[cfg(test)]
    pub(crate) fn state_for_test(&self) -> std::sync::MutexGuard<'_, S> {
        self.state.lock().unwrap()
    }

    /// The `dt` passed to `SimState::tick` under `fixed_timestep`.
    pub fn fixed_dt(&self) -> Option<Duration> {
        (self.config.fixed_timestep && self.config.tick_rate_hz > 0)
            .then(|| Duration::from_nanos(1_000_000_000 / self.config.tick_rate_hz as u64))
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Acquire)
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::Release);
    }

    pub fn created_at(&self) -> Instant {
        self.created_at
    }

    pub fn tick_number(&self) -> u64 {
        *self.tick_no.lock().unwrap()
    }

    /// Read a snapshot of the current state. Useful for tests and admin UIs.
    /// In hot paths, prefer pushing a snapshot through the broadcast/subscriber
    /// channel rather than calling this on every read.
    pub fn snapshot(&self) -> S::Snapshot {
        self.state.lock().unwrap().snapshot()
    }

    pub fn subscriber_count(&self) -> usize {
        self.subscribers.lock().unwrap().len()
    }

    /// The highest `client_seq` processed for a subscriber (0 = none).
    pub fn ack(&self, id: &SubscriberId) -> u64 {
        self.acks.lock().unwrap().get(id).copied().unwrap_or(0)
    }

    pub fn input_queue_len(&self) -> usize {
        self.inputs.lock().unwrap().queue.len()
    }

    // -----------------------------------------------------------------------
    // Subscribers
    // -----------------------------------------------------------------------

    /// Unauthenticated subscribe. **Do not expose this over the network.**
    ///
    /// This adds a subscriber with no `authorize_subscribe` hook call — every
    /// transport that accepts connections from clients should use
    /// [`add_subscriber_authorized`] instead. The non-authorized variant
    /// exists for tests, in-process fan-in, and trusted server-side code.
    ///
    /// If a future transport grows up that doesn't use the authorized path
    /// by accident, this becomes an auth bypass on shard state.
    #[doc(hidden)]
    pub fn add_subscriber(&self, sub: Subscriber<S::Snapshot>) -> Result<(), ShardError> {
        if !self.is_running() {
            return Err(ShardError::Stopped);
        }
        let mut subs = self.subscribers.lock().unwrap();
        if self.config.max_subscribers > 0 && subs.len() >= self.config.max_subscribers {
            return Err(ShardError::Full);
        }
        subs.push(Arc::new(sub));
        Ok(())
    }

    /// Add a subscriber after running the user's authorization hook.
    pub fn add_subscriber_authorized(
        &self,
        sub: Subscriber<S::Snapshot>,
        auth: &ShardAuth,
    ) -> Result<(), ShardError> {
        {
            let state = self.state.lock().unwrap();
            state
                .authorize_subscribe(sub.id(), auth)
                .map_err(ShardError::Unauthorized)?;
        }
        self.add_subscriber(sub)
    }

    /// Add a subscriber that receives frames through a new outbound queue,
    /// sized from the shard config, after the authorization hook. The
    /// transport drains the returned queue.
    pub fn add_queued_subscriber_authorized(
        &self,
        id: SubscriberId,
        auth: &ShardAuth,
    ) -> Result<Arc<OutboundQueue>, ShardError> {
        let queue = OutboundQueue::new(self.config.outbound());
        self.add_subscriber_authorized(Subscriber::with_queue(id, Arc::clone(&queue)), auth)?;
        Ok(queue)
    }

    pub fn remove_subscriber(&self, id: &SubscriberId) -> bool {
        let removed = {
            let mut subs = self.subscribers.lock().unwrap();
            let before = subs.len();
            subs.retain(|s| {
                if s.id() == id {
                    // Wake the transport's writer so it ends too.
                    if let Some(q) = s.queue() {
                        q.close();
                    }
                    false
                } else {
                    true
                }
            });
            before != subs.len()
        };
        if removed {
            self.acks.lock().unwrap().remove(id);
            // Keep the entry while inputs from this subscriber are still
            // queued; the tick removes it once they drain.
            let mut inputs = self.inputs.lock().unwrap();
            if inputs.per_subscriber.get(id).is_some_and(|e| e.queued == 0) {
                inputs.per_subscriber.remove(id);
            }
        }
        removed
    }

    // -----------------------------------------------------------------------
    // Input queue
    // -----------------------------------------------------------------------

    /// Queue an input from a subscriber. Returns the assigned server-side
    /// sequence number, which clients use for reconciliation.
    ///
    /// **Unauthenticated.** This does not verify that `subscriber_id` matches
    /// an attached subscriber or that the caller is allowed to act as that
    /// subscriber. Transports that accept inputs from clients must use
    /// [`push_input_authorized`] instead. The non-authorized variant is for
    /// tests, simulation harnesses, and trusted server-side fan-in.
    #[doc(hidden)]
    pub fn push_input(
        &self,
        subscriber_id: SubscriberId,
        input: S::Input,
        client_seq: Option<u64>,
    ) -> Result<u64, ShardError> {
        if !self.is_running() {
            return Err(ShardError::Stopped);
        }

        let now = Instant::now();
        {
            let mut q = self.inputs.lock().unwrap();
            let cfg = &self.config;
            let entry = q
                .per_subscriber
                .entry(subscriber_id.clone())
                .or_insert_with(|| SubscriberInputs {
                    queued: 0,
                    tokens: cfg.input_burst_per_subscriber.max(1.0),
                    last_refill: now,
                    dropped_since_log: 0,
                    last_log: None,
                });

            // This subscriber's limits first, so one client flooding the
            // shard only loses its own inputs.
            let mut limited = cfg.max_queued_inputs_per_subscriber > 0
                && entry.queued >= cfg.max_queued_inputs_per_subscriber;
            if !limited && cfg.input_rate_per_subscriber > 0.0 {
                let burst = cfg.input_burst_per_subscriber.max(1.0);
                let elapsed = now.duration_since(entry.last_refill).as_secs_f64();
                entry.tokens = (entry.tokens + elapsed * cfg.input_rate_per_subscriber).min(burst);
                entry.last_refill = now;
                if entry.tokens >= 1.0 {
                    entry.tokens -= 1.0;
                } else {
                    limited = true;
                }
            }
            if limited {
                entry.dropped_since_log += 1;
                if entry
                    .last_log
                    .is_none_or(|t| now.duration_since(t) >= DROP_LOG_INTERVAL)
                {
                    tracing::warn!(
                        "[realtime] shard {}: dropped {} input(s) from subscriber {} over its input limit",
                        self.id,
                        entry.dropped_since_log,
                        subscriber_id
                    );
                    entry.dropped_since_log = 0;
                    entry.last_log = Some(now);
                }
                return Err(ShardError::InputRateLimited);
            }

            if cfg.max_input_queue > 0 && q.queue.len() >= cfg.max_input_queue {
                return Err(ShardError::InputQueueFull);
            }
            q.per_subscriber
                .get_mut(&subscriber_id)
                .expect("entry inserted above")
                .queued += 1;
            q.queue.push_back(PendingInput {
                subscriber_id,
                input,
                seq: client_seq,
                received_at: now,
            });
        }
        *self.last_input_at.lock().unwrap() = now;

        let mut seq_guard = self.input_seq.lock().unwrap();
        *seq_guard += 1;
        Ok(*seq_guard)
    }

    /// Queue an input after running the user's authorization hook.
    ///
    /// Also verifies that `subscriber_id` matches an attached subscriber.
    /// Without this check a caller could push inputs on behalf of any
    /// subscriber id the server has seen — the authorize_input hook only
    /// gets the id, not whether it corresponds to an active connection.
    pub fn push_input_authorized(
        &self,
        subscriber_id: SubscriberId,
        input: S::Input,
        client_seq: Option<u64>,
        auth: &ShardAuth,
    ) -> Result<u64, ShardError> {
        // Confirm the id is actually attached to this shard. A missing
        // subscriber means either the client disconnected between opening
        // a channel and sending their input, or the caller is forging.
        {
            let subs = self.subscribers.lock().unwrap();
            if !subs.iter().any(|s| s.id() == &subscriber_id) {
                return Err(ShardError::Unauthorized(format!(
                    "subscriber {subscriber_id:?} is not attached to this shard"
                )));
            }
        }
        {
            let state = self.state.lock().unwrap();
            state
                .authorize_input(&subscriber_id, auth, &input)
                .map_err(ShardError::Unauthorized)?;
        }
        self.push_input(subscriber_id, input, client_seq)
    }

    // -----------------------------------------------------------------------
    // Tick — the heart of the shard
    // -----------------------------------------------------------------------

    /// Advance the shard by one tick:
    /// 1. Drain the input queue, applying each input to state (at most
    ///    `max_inputs_per_subscriber_per_tick` per subscriber; the rest wait
    ///    for the next tick).
    /// 2. Advance simulation time by `dt`.
    /// 3. Run the user's `on_tick` hook if set.
    /// 4. Take a per-subscriber snapshot.
    /// 5. Release the state lock, then encode and deliver the snapshots.
    /// 6. Check finish / idle-shutdown conditions.
    ///
    /// Only steps 1-4 hold the state lock. Delivery never waits on a client:
    /// a queued subscriber's frame goes into its outbound queue, and a
    /// subscriber whose queue closed is removed.
    pub fn run_tick(&self) {
        if !self.is_running() {
            return;
        }

        let now = Instant::now();
        let measured = self
            .last_tick_at
            .lock()
            .unwrap()
            .map(|prev| now.duration_since(prev))
            .unwrap_or_default();
        *self.last_tick_at.lock().unwrap() = Some(now);
        let dt = self.fixed_dt().unwrap_or(measured);

        let mut tick_no_guard = self.tick_no.lock().unwrap();
        *tick_no_guard += 1;
        let tick_number = *tick_no_guard;
        drop(tick_no_guard);

        let drained = self.drain_inputs();
        let had_inputs = !drained.is_empty();
        // Clone the list so delivery below runs without the subscribers
        // lock: a transport can add or remove subscribers meanwhile.
        let subs: Vec<Arc<Subscriber<S::Snapshot>>> = self.subscribers.lock().unwrap().clone();
        let sub_count = subs.len();

        let mut failed: Vec<(SubscriberId, InputRejection)> = Vec::new();
        let (snapshots, finished) = {
            let mut state = self.state.lock().unwrap();
            let mut acks = self.acks.lock().unwrap();
            let recorder = self.input_recorder.lock().unwrap();
            for pending in drained {
                if let Some(record) = &*recorder {
                    record(tick_number, &pending.subscriber_id, &pending.input);
                }
                if let Some(seq) = pending.seq {
                    let ack = acks.entry(pending.subscriber_id.clone()).or_insert(0);
                    *ack = (*ack).max(seq);
                }
                if let Err(e) =
                    state.apply_input(&pending.subscriber_id, pending.input, pending.received_at)
                {
                    tracing::warn!("[realtime] apply_input error in shard {}: {:?}", self.id, e);
                    failed.push((
                        pending.subscriber_id,
                        InputRejection {
                            client_seq: pending.seq,
                            code: "apply_failed".into(),
                            message: format!("{e:?}"),
                        },
                    ));
                }
            }
            drop(acks);
            drop(recorder);

            state.tick(dt);

            if let Some(cb) = &*self.on_tick.lock().unwrap() {
                cb(&state, tick_number);
            }

            let snapshots: Vec<S::Snapshot> = subs
                .iter()
                .map(|sub| state.snapshot_for(sub.id()))
                .collect();
            (snapshots, state.is_finished())
        };

        // Encode and deliver outside the state lock. Rejections go first so
        // the client learns about a failed input before the snapshot that
        // acks it.
        let acks = self.acks.lock().unwrap().clone();
        let ack_of = |id: &SubscriberId| acks.get(id).copied().unwrap_or(0);
        for (id, rejection) in &failed {
            if let Some(sub) = subs.iter().find(|s| s.id() == id) {
                sub.reject(
                    tick_number,
                    ack_of(id),
                    rejection,
                    self.config.snapshot_format,
                );
            }
        }
        let mut closed = false;
        for (sub, snap) in subs.iter().zip(snapshots.iter()) {
            sub.send(
                tick_number,
                snap,
                self.config.snapshot_format,
                ack_of(sub.id()),
            );
            closed |= sub.is_closed();
        }
        if closed {
            let mut all = self.subscribers.lock().unwrap();
            all.retain(|s| {
                if s.is_closed() {
                    tracing::warn!(
                        "[realtime] shard {}: disconnecting subscriber {} (outbound queue closed)",
                        self.id,
                        s.id()
                    );
                    false
                } else {
                    true
                }
            });
        }

        if finished {
            self.running.store(false, Ordering::Release);
            return;
        }

        // Idle tracking.
        let mut idle = self.idle_ticks.lock().unwrap();
        if had_inputs || sub_count > 0 {
            *idle = 0;
        } else {
            *idle += 1;
            if self.config.idle_ticks_before_shutdown > 0
                && *idle >= self.config.idle_ticks_before_shutdown
            {
                self.running.store(false, Ordering::Release);
            }
        }
    }

    /// Take this tick's inputs off the queue: all of them, except that a
    /// subscriber over `max_inputs_per_subscriber_per_tick` keeps the rest
    /// queued, in order, for the next tick.
    fn drain_inputs(&self) -> Vec<PendingInput<S::Input>> {
        let limit = self.config.max_inputs_per_subscriber_per_tick;
        let mut q = self.inputs.lock().unwrap();
        let InputQueue {
            queue,
            per_subscriber,
        } = &mut *q;
        let mut taken = Vec::with_capacity(queue.len());
        if limit == 0 {
            taken.extend(queue.drain(..));
        } else {
            let mut this_tick: HashMap<SubscriberId, usize> = HashMap::new();
            let mut carry = VecDeque::new();
            for pending in queue.drain(..) {
                let n = this_tick.entry(pending.subscriber_id.clone()).or_insert(0);
                if *n < limit {
                    *n += 1;
                    taken.push(pending);
                } else {
                    carry.push_back(pending);
                }
            }
            *queue = carry;
        }
        for pending in &taken {
            if let Some(e) = per_subscriber.get_mut(&pending.subscriber_id) {
                e.queued = e.queued.saturating_sub(1);
            }
        }
        // Forget subscribers that left and have nothing queued. An attached
        // subscriber keeps its entry (and its token bucket).
        if !taken.is_empty() {
            let attached = self.subscribers.lock().unwrap();
            per_subscriber.retain(|id, e| e.queued > 0 || attached.iter().any(|s| s.id() == id));
        }
        taken
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subscriber::{Subscriber, SubscriberId};
    use std::sync::atomic::{AtomicU64, Ordering};

    // A trivial counter simulation for testing.
    struct Counter {
        value: u64,
        finished: bool,
    }

    impl SimState for Counter {
        type Input = i64;
        type Snapshot = u64;
        type Error = String;

        fn apply_input(
            &mut self,
            _sub: &SubscriberId,
            input: Self::Input,
            _now: Instant,
        ) -> Result<(), Self::Error> {
            if input >= 0 {
                self.value += input as u64;
            } else {
                let abs = (-input) as u64;
                self.value = self.value.saturating_sub(abs);
            }
            Ok(())
        }

        fn tick(&mut self, _dt: Duration) {}
        fn snapshot(&self) -> Self::Snapshot {
            self.value
        }
        fn is_finished(&self) -> bool {
            self.finished
        }
    }

    #[test]
    fn shard_applies_inputs_on_tick() {
        let shard = Shard::new(
            "test",
            Counter {
                value: 0,
                finished: false,
            },
            ShardConfig::default(),
        );

        let sub_id = SubscriberId::new("p1");
        let seq1 = shard.push_input(sub_id.clone(), 5, None).unwrap();
        let seq2 = shard.push_input(sub_id.clone(), 3, None).unwrap();
        assert_eq!(seq1, 1);
        assert_eq!(seq2, 2);

        shard.run_tick();

        assert_eq!(shard.state.lock().unwrap().value, 8);
        assert_eq!(shard.tick_number(), 1);
    }

    #[test]
    fn shard_stops_when_finished() {
        let shard = Shard::new(
            "test",
            Counter {
                value: 0,
                finished: true,
            },
            ShardConfig::default(),
        );

        shard.run_tick();
        assert!(!shard.is_running());
    }

    #[test]
    fn shard_respects_max_subscribers() {
        let config = ShardConfig {
            max_subscribers: 2,
            ..Default::default()
        };
        let shard: Arc<Shard<Counter>> = Shard::new(
            "t",
            Counter {
                value: 0,
                finished: false,
            },
            config,
        );

        let counter = Arc::new(AtomicU64::new(0));
        let make_sub = |i: u32| -> Subscriber<u64> {
            let c = Arc::clone(&counter);
            Subscriber::new(
                SubscriberId::new(format!("s{i}")),
                Box::new(move |_tick, _bytes| {
                    c.fetch_add(1, Ordering::Relaxed);
                }),
            )
        };

        shard.add_subscriber(make_sub(1)).unwrap();
        shard.add_subscriber(make_sub(2)).unwrap();
        assert!(matches!(
            shard.add_subscriber(make_sub(3)),
            Err(ShardError::Full)
        ));
    }

    #[test]
    fn shard_broadcasts_snapshot_to_subscribers() {
        let shard: Arc<Shard<Counter>> = Shard::new(
            "t",
            Counter {
                value: 0,
                finished: false,
            },
            ShardConfig::default(),
        );

        let received = Arc::new(Mutex::new(Vec::<(u64, Vec<u8>)>::new()));
        let received_clone = Arc::clone(&received);
        let sub = Subscriber::new(
            SubscriberId::new("p1"),
            Box::new(move |tick, bytes| {
                received_clone.lock().unwrap().push((tick, bytes.to_vec()));
            }),
        );
        shard.add_subscriber(sub).unwrap();

        shard.push_input(SubscriberId::new("p1"), 42, None).unwrap();
        shard.run_tick();

        let r = received.lock().unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].0, 1);
        // JSON format by default: snapshot is "42"
        assert_eq!(r[0].1, b"42");
    }

    #[test]
    fn default_authorize_subscribe_requires_matching_user_id() {
        // The default hook requires the auth user_id to match the subscriber
        // id, so Alice cannot subscribe to Bob's sid.
        let shard: Arc<Shard<Counter>> = Shard::new(
            "t",
            Counter {
                value: 0,
                finished: false,
            },
            ShardConfig::default(),
        );
        let sub = Subscriber::new(SubscriberId::new("bob"), Box::new(|_tick, _bytes| {}));
        let alice = ShardAuth {
            user_id: Some("alice".into()),
            is_admin: false,
        };
        let err = shard.add_subscriber_authorized(sub, &alice);
        assert!(matches!(err, Err(ShardError::Unauthorized(_))));
    }

    #[test]
    fn default_authorize_subscribe_allows_matching_user_id() {
        let shard: Arc<Shard<Counter>> = Shard::new(
            "t",
            Counter {
                value: 0,
                finished: false,
            },
            ShardConfig::default(),
        );
        let sub = Subscriber::new(SubscriberId::new("alice"), Box::new(|_tick, _bytes| {}));
        let alice = ShardAuth {
            user_id: Some("alice".into()),
            is_admin: false,
        };
        shard.add_subscriber_authorized(sub, &alice).unwrap();
    }

    #[test]
    fn default_authorize_subscribe_admin_passes() {
        let shard: Arc<Shard<Counter>> = Shard::new(
            "t",
            Counter {
                value: 0,
                finished: false,
            },
            ShardConfig::default(),
        );
        let sub = Subscriber::new(SubscriberId::new("whoever"), Box::new(|_tick, _bytes| {}));
        let admin = ShardAuth {
            user_id: None,
            is_admin: true,
        };
        shard.add_subscriber_authorized(sub, &admin).unwrap();
    }

    fn counter() -> Counter {
        Counter {
            value: 0,
            finished: false,
        }
    }

    #[test]
    fn one_flooding_subscriber_does_not_crowd_out_another() {
        let shard: Arc<Shard<Counter>> = Shard::new("t", counter(), ShardConfig::default());
        let a = SubscriberId::new("a");
        let b = SubscriberId::new("b");
        let mut a_rejected = 0;
        for _ in 0..50_000 {
            if shard.push_input(a.clone(), 1, None).is_err() {
                a_rejected += 1;
            }
        }
        assert!(
            a_rejected > 49_000,
            "a's flood was limited ({a_rejected} rejected)"
        );
        shard.push_input(b.clone(), 1_000_000, None).unwrap();

        shard.run_tick();
        let value = shard.state.lock().unwrap().value;
        assert!(
            value >= 1_000_000,
            "b's input was applied on this tick (value {value})"
        );
    }

    #[test]
    fn inputs_over_the_per_tick_limit_wait_for_the_next_tick_in_order() {
        let config = ShardConfig {
            max_inputs_per_subscriber_per_tick: 3,
            ..Default::default()
        };
        let shard: Arc<Shard<Counter>> = Shard::new("t", counter(), config);
        let a = SubscriberId::new("a");
        for i in 0..5 {
            shard.push_input(a.clone(), 10_i64.pow(i), None).unwrap();
        }
        shard
            .push_input(SubscriberId::new("b"), 100_000, None)
            .unwrap();
        shard.run_tick();
        // a: 1 + 10 + 100; b: 100000. a's 1000 and 10000 wait.
        assert_eq!(shard.state.lock().unwrap().value, 100_111);
        assert_eq!(shard.input_queue_len(), 2);
        shard.run_tick();
        assert_eq!(shard.state.lock().unwrap().value, 111_111);
        assert_eq!(shard.input_queue_len(), 0);
    }

    #[test]
    fn the_token_bucket_refills_over_time() {
        let config = ShardConfig {
            input_rate_per_subscriber: 1000.0,
            input_burst_per_subscriber: 5.0,
            max_queued_inputs_per_subscriber: 0,
            ..Default::default()
        };
        let shard: Arc<Shard<Counter>> = Shard::new("t", counter(), config);
        let a = SubscriberId::new("a");
        let accepted = (0..20)
            .filter(|_| shard.push_input(a.clone(), 1, None).is_ok())
            .count();
        assert_eq!(accepted, 5, "the burst allows 5");
        assert!(matches!(
            shard.push_input(a.clone(), 1, None),
            Err(ShardError::InputRateLimited)
        ));
        std::thread::sleep(Duration::from_millis(10)); // ~10 tokens at 1000/s
        assert!(shard.push_input(a, 1, None).is_ok());
    }

    #[test]
    fn a_stalled_client_does_not_slow_the_tick_or_other_subscribers() {
        use crate::outbound::OutboundQueue;
        let shard: Arc<Shard<Counter>> = Shard::new("t", counter(), ShardConfig::default());

        // A queued subscriber whose writer takes 2 s per frame.
        let slow = OutboundQueue::new(shard.config().outbound());
        let slow_writer = Arc::clone(&slow);
        std::thread::spawn(move || {
            while slow_writer.pop_blocking(Duration::from_secs(5)).is_some() {
                std::thread::sleep(Duration::from_secs(2));
            }
        });
        shard
            .add_subscriber(Subscriber::with_queue(
                SubscriberId::new("slow"),
                Arc::clone(&slow),
            ))
            .unwrap();

        // A healthy direct subscriber counts the frames it gets.
        let got = Arc::new(AtomicU64::new(0));
        let g = Arc::clone(&got);
        shard
            .add_subscriber(Subscriber::new(
                SubscriberId::new("ok"),
                Box::new(move |_t, _b| {
                    g.fetch_add(1, Ordering::Relaxed);
                }),
            ))
            .unwrap();

        let mut worst = Duration::ZERO;
        for _ in 0..100 {
            let start = Instant::now();
            shard.run_tick();
            worst = worst.max(start.elapsed());
        }
        assert!(worst < Duration::from_millis(5), "a tick took {worst:?}");
        assert_eq!(
            got.load(Ordering::Relaxed),
            100,
            "the healthy subscriber got every tick"
        );
        // The slow queue is at its cap with only the newest snapshots.
        assert!(slow.len() <= shard.config().outbound_queue_frames);
        assert!(slow.dropped_snapshots() > 0);
    }

    #[test]
    fn a_subscriber_whose_queue_closes_is_removed() {
        use crate::outbound::OutboundQueue;
        let config = ShardConfig {
            outbound_queue_frames: 2,
            slow_subscriber_timeout: Duration::from_millis(20),
            ..Default::default()
        };
        let shard: Arc<Shard<Counter>> = Shard::new("t", counter(), config);
        let queue: Arc<OutboundQueue> = shard
            .add_queued_subscriber_authorized(
                SubscriberId::new("admin"),
                &ShardAuth {
                    user_id: None,
                    is_admin: true,
                },
            )
            .unwrap();
        // Nobody drains the queue.
        for _ in 0..3 {
            shard.run_tick();
        }
        assert_eq!(shard.subscriber_count(), 1);
        std::thread::sleep(Duration::from_millis(30));
        shard.run_tick();
        assert!(queue.is_closed());
        assert_eq!(shard.subscriber_count(), 0);
    }

    #[test]
    fn remove_subscriber_closes_its_queue() {
        let shard: Arc<Shard<Counter>> = Shard::new("t", counter(), ShardConfig::default());
        let admin = ShardAuth {
            user_id: None,
            is_admin: true,
        };
        let q = shard
            .add_queued_subscriber_authorized(SubscriberId::new("x"), &admin)
            .unwrap();
        assert!(shard.remove_subscriber(&SubscriberId::new("x")));
        assert!(q.is_closed());
    }

    fn admin() -> ShardAuth {
        ShardAuth {
            user_id: None,
            is_admin: true,
        }
    }

    #[test]
    fn each_snapshot_carries_the_subscribers_own_ack() {
        let shard: Arc<Shard<Counter>> = Shard::new("t", counter(), ShardConfig::default());
        let qa = shard
            .add_queued_subscriber_authorized(SubscriberId::new("a"), &admin())
            .unwrap();
        let qb = shard
            .add_queued_subscriber_authorized(SubscriberId::new("b"), &admin())
            .unwrap();
        for seq in 1..=5 {
            shard
                .push_input(SubscriberId::new("a"), 1, Some(seq))
                .unwrap();
        }
        shard
            .push_input(SubscriberId::new("b"), 1, Some(42))
            .unwrap();
        shard.run_tick();

        let fa = qa.pop().unwrap();
        let fb = qb.pop().unwrap();
        assert_eq!((fa.kind, fa.ack), (crate::outbound::FrameKind::Snapshot, 5));
        assert_eq!(fb.ack, 42);
        // The ack persists on later ticks with no new input.
        shard.run_tick();
        assert_eq!(qa.pop().unwrap().ack, 5);
    }

    #[test]
    fn a_failed_apply_sends_a_rejection_before_the_snapshot() {
        struct Picky;
        impl SimState for Picky {
            type Input = i64;
            type Snapshot = u64;
            type Error = String;
            fn apply_input(
                &mut self,
                _s: &SubscriberId,
                i: i64,
                _n: Instant,
            ) -> Result<(), String> {
                if i < 0 {
                    Err("negative".into())
                } else {
                    Ok(())
                }
            }
            fn tick(&mut self, _dt: Duration) {}
            fn snapshot(&self) -> u64 {
                0
            }
        }
        let shard: Arc<Shard<Picky>> = Shard::new("t", Picky, ShardConfig::default());
        let q = shard
            .add_queued_subscriber_authorized(SubscriberId::new("a"), &admin())
            .unwrap();
        shard
            .push_input(SubscriberId::new("a"), -1, Some(7))
            .unwrap();
        shard.run_tick();

        let rej = q.pop().unwrap();
        assert_eq!(rej.kind, crate::outbound::FrameKind::InputRejected);
        let body: crate::wire::InputRejection = serde_json::from_slice(&rej.bytes).unwrap();
        assert_eq!(body.client_seq, Some(7));
        assert_eq!(body.code, "apply_failed");
        assert!(body.message.contains("negative"));
        // The failed input still counts as processed.
        assert_eq!(q.pop().unwrap().ack, 7);
    }

    #[test]
    fn shard_idle_shutdown() {
        let config = ShardConfig {
            idle_ticks_before_shutdown: 3,
            tick_rate_hz: 0,
            ..Default::default()
        };
        let shard: Arc<Shard<Counter>> = Shard::new(
            "t",
            Counter {
                value: 0,
                finished: false,
            },
            config,
        );

        shard.run_tick();
        shard.run_tick();
        shard.run_tick();
        assert!(!shard.is_running());
    }
}
