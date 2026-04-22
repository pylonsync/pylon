//! The core [`Shard`] abstraction.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{de::DeserializeOwned, Serialize};

use crate::subscriber::{Subscriber, SubscriberId};

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
    /// Max subscribers permitted. 0 = unlimited.
    pub max_subscribers: usize,
    /// Shut down the shard after this many consecutive empty ticks
    /// (no inputs, no subscribers). 0 = never.
    pub idle_ticks_before_shutdown: u32,
    /// Drop inputs if the queue exceeds this. 0 = unlimited (careful).
    pub max_input_queue: usize,
    /// Snapshot format used on the wire.
    pub snapshot_format: crate::snapshot::SnapshotFormat,
}

impl Default for ShardConfig {
    fn default() -> Self {
        Self {
            tick_rate_hz: 20,
            max_subscribers: 256,
            idle_ticks_before_shutdown: 60 * 30, // 30s at 20Hz
            max_input_queue: 10_000,
            snapshot_format: crate::snapshot::SnapshotFormat::Json,
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
    /// Optional sequence number so the client can reconcile predictions.
    /// Read via the `InputAck` envelope — not traced yet at the Shard level.
    #[allow(dead_code)]
    seq: Option<u64>,
    received_at: Instant,
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
    inputs: Mutex<VecDeque<PendingInput<S::Input>>>,
    subscribers: Mutex<Vec<Subscriber<S::Snapshot>>>,
    running: AtomicBool,
    /// Monotonically increasing tick number. Used for reconciliation and
    /// lockstep protocols.
    tick_no: Mutex<u64>,
    /// Monotonic input sequence counter (global per shard).
    input_seq: Mutex<u64>,
    created_at: Instant,
    last_input_at: Mutex<Instant>,
    last_tick_at: Mutex<Option<Instant>>,
    /// Count of consecutive idle ticks (no inputs, no subscribers).
    idle_ticks: Mutex<u32>,
    /// Optional hook: user callback invoked after each tick. Useful for
    /// persistence (save state to statecraft every N ticks).
    on_tick: Mutex<Option<Box<dyn Fn(&S, u64) + Send + Sync>>>,
}

impl<S: SimState> Shard<S> {
    /// Create a new shard with an initial simulation state.
    pub fn new(id: impl Into<String>, initial: S, config: ShardConfig) -> Arc<Self> {
        let now = Instant::now();
        Arc::new(Self {
            id: id.into(),
            config,
            state: Mutex::new(initial),
            inputs: Mutex::new(VecDeque::new()),
            subscribers: Mutex::new(Vec::new()),
            running: AtomicBool::new(true),
            tick_no: Mutex::new(0),
            input_seq: Mutex::new(0),
            created_at: now,
            last_input_at: Mutex::new(now),
            last_tick_at: Mutex::new(None),
            idle_ticks: Mutex::new(0),
            on_tick: Mutex::new(None),
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn config(&self) -> &ShardConfig {
        &self.config
    }

    /// Register a callback that runs after every tick — use to persist state
    /// via statecraft, push metrics, or trigger side effects.
    pub fn set_on_tick(&self, callback: impl Fn(&S, u64) + Send + Sync + 'static) {
        *self.on_tick.lock().unwrap() = Some(Box::new(callback));
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

    pub fn input_queue_len(&self) -> usize {
        self.inputs.lock().unwrap().len()
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
    pub fn add_subscriber(
        &self,
        sub: Subscriber<S::Snapshot>,
    ) -> Result<(), ShardError> {
        if !self.is_running() {
            return Err(ShardError::Stopped);
        }
        let mut subs = self.subscribers.lock().unwrap();
        if self.config.max_subscribers > 0 && subs.len() >= self.config.max_subscribers {
            return Err(ShardError::Full);
        }
        subs.push(sub);
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

    pub fn remove_subscriber(&self, id: &SubscriberId) -> bool {
        let mut subs = self.subscribers.lock().unwrap();
        let before = subs.len();
        subs.retain(|s| s.id() != id);
        before != subs.len()
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

        let mut seq_guard = self.input_seq.lock().unwrap();
        *seq_guard += 1;
        let seq = *seq_guard;
        drop(seq_guard);

        let mut q = self.inputs.lock().unwrap();
        if self.config.max_input_queue > 0 && q.len() >= self.config.max_input_queue {
            return Err(ShardError::InputQueueFull);
        }
        q.push_back(PendingInput {
            subscriber_id,
            input,
            seq: client_seq,
            received_at: Instant::now(),
        });
        *self.last_input_at.lock().unwrap() = Instant::now();

        Ok(seq)
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
    /// 1. Drain the input queue, applying each input to state.
    /// 2. Advance simulation time by `dt`.
    /// 3. Broadcast a per-subscriber snapshot.
    /// 4. Run the user's `on_tick` hook if set.
    /// 5. Check finish / idle-shutdown conditions.
    pub fn run_tick(&self) {
        if !self.is_running() {
            return;
        }

        let now = Instant::now();
        let dt = self
            .last_tick_at
            .lock()
            .unwrap()
            .map(|prev| now.duration_since(prev))
            .unwrap_or_default();
        *self.last_tick_at.lock().unwrap() = Some(now);

        let mut tick_no_guard = self.tick_no.lock().unwrap();
        *tick_no_guard += 1;
        let tick_number = *tick_no_guard;
        drop(tick_no_guard);

        // Drain all pending inputs first, before ticking.
        let drained: Vec<PendingInput<S::Input>> = {
            let mut q = self.inputs.lock().unwrap();
            q.drain(..).collect()
        };
        let had_inputs = !drained.is_empty();
        let sub_count = self.subscriber_count();

        {
            let mut state = self.state.lock().unwrap();
            for pending in drained {
                if let Err(e) =
                    state.apply_input(&pending.subscriber_id, pending.input, pending.received_at)
                {
                    tracing::warn!(
                        "[realtime] apply_input error in shard {}: {:?}",
                        self.id, e
                    );
                }
            }

            state.tick(dt);

            // Run the persistence / side-effect hook.
            if let Some(cb) = &*self.on_tick.lock().unwrap() {
                cb(&state, tick_number);
            }

            // Broadcast.
            if sub_count > 0 {
                let subs = self.subscribers.lock().unwrap();
                for sub in subs.iter() {
                    let snap = state.snapshot_for(sub.id());
                    sub.send(tick_number, &snap, self.config.snapshot_format);
                }
            }

            // Check finish.
            if state.is_finished() {
                self.running.store(false, Ordering::Release);
                return;
            }
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
                received_clone
                    .lock()
                    .unwrap()
                    .push((tick, bytes.to_vec()));
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
        // Previously the default hook allowed any authenticated user to
        // subscribe to any sid, which let Alice impersonate Bob.
        let shard: Arc<Shard<Counter>> = Shard::new(
            "t",
            Counter { value: 0, finished: false },
            ShardConfig::default(),
        );
        let sub = Subscriber::new(
            SubscriberId::new("bob"),
            Box::new(|_tick, _bytes| {}),
        );
        let alice = ShardAuth { user_id: Some("alice".into()), is_admin: false };
        let err = shard.add_subscriber_authorized(sub, &alice);
        assert!(matches!(err, Err(ShardError::Unauthorized(_))));
    }

    #[test]
    fn default_authorize_subscribe_allows_matching_user_id() {
        let shard: Arc<Shard<Counter>> = Shard::new(
            "t",
            Counter { value: 0, finished: false },
            ShardConfig::default(),
        );
        let sub = Subscriber::new(
            SubscriberId::new("alice"),
            Box::new(|_tick, _bytes| {}),
        );
        let alice = ShardAuth { user_id: Some("alice".into()), is_admin: false };
        shard.add_subscriber_authorized(sub, &alice).unwrap();
    }

    #[test]
    fn default_authorize_subscribe_admin_passes() {
        let shard: Arc<Shard<Counter>> = Shard::new(
            "t",
            Counter { value: 0, finished: false },
            ShardConfig::default(),
        );
        let sub = Subscriber::new(
            SubscriberId::new("whoever"),
            Box::new(|_tick, _bytes| {}),
        );
        let admin = ShardAuth { user_id: None, is_admin: true };
        shard.add_subscriber_authorized(sub, &admin).unwrap();
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
