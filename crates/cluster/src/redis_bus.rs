//! Redis PUB/SUB transport for [`crate::ClusterBus`].
//!
//! Two connections per bus:
//! - **publish** — single shared connection wrapped in a mutex. PUBLISH
//!   is a single round-trip; serializing through the mutex is cheaper
//!   than maintaining a pool for the publish-rate Pylon emits in
//!   practice (1 event per mutation, low-thousands/sec ceiling).
//! - **subscribe** — dedicated background thread holds its own
//!   connection in SUBSCRIBE mode (Redis protocol requires a
//!   connection in SUBSCRIBE mode to stay subscribed). The thread
//!   reconnects with exponential backoff if the server drops.
//!
//! Reconnect behavior matters: a Fly machine flipping Redis primary
//! during a deploy will sever every subscriber. Without reconnect,
//! every pylon machine in the cluster goes deaf to cross-machine
//! events until manually restarted.

use crate::{new_instance_id, ClusterBus, Envelope, SubscriberHandler};
use redis::{Client, Commands, RedisError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tracing::{debug, error, info, warn};

/// Default Redis channel prefix. Override with the `namespace`
/// constructor argument when multiple unrelated pylon deployments
/// share one Redis instance — without a unique namespace they'd
/// cross-talk and ship each other's mutations.
pub const DEFAULT_CHANNEL: &str = "pylon:cluster:bus";

pub struct RedisBus {
    client: Client,
    channel: String,
    instance_id: String,
    /// Publish connection. Wrapped in Mutex because the `redis` sync
    /// client isn't Sync at the connection level. The mutex contention
    /// is bounded by publish rate; pylon emits ≤O(mutations/sec) which
    /// is two orders of magnitude below the mutex's ceiling.
    publish_conn: Mutex<redis::Connection>,
    /// Handlers registered via [`subscribe`]. The subscriber thread
    /// reads from this Vec on every inbound message and invokes each
    /// handler in turn. Wrapped in Arc<Mutex> so callers can register
    /// after the thread has started — needed because the runtime's
    /// startup order may install the bus before the local notifier.
    handlers: Arc<Mutex<Vec<SubscriberHandler>>>,
}

impl RedisBus {
    /// Construct a RedisBus pointed at `url` (e.g.
    /// `redis://default:pass@host:6379/0`). Spawns the subscriber
    /// thread immediately so any handler registered later is wired up
    /// without an extra start step.
    ///
    /// Connection failures during construction are fatal — the caller
    /// (server.rs) treats them as a misconfiguration and refuses to
    /// boot. Once the thread is running, transient Redis failures are
    /// logged + reconnected to keep the cluster gluing back together
    /// after blips.
    pub fn connect(url: &str, namespace: Option<&str>) -> Result<Self, String> {
        let channel = namespace
            .map(|n| format!("{n}:cluster:bus"))
            .unwrap_or_else(|| DEFAULT_CHANNEL.to_string());
        let client = Client::open(url).map_err(|e| format!("redis Client::open: {e}"))?;
        let publish_conn = client
            .get_connection()
            .map_err(|e| format!("redis publish connect: {e}"))?;
        let instance_id = new_instance_id();
        let handlers: Arc<Mutex<Vec<SubscriberHandler>>> = Arc::new(Mutex::new(Vec::new()));

        // Subscriber thread. Owned by this RedisBus for the process
        // lifetime — we don't expose a stop() because pylon's server
        // also has no stop(), and an unscheduled drop here would race
        // with in-flight envelopes.
        let sub_url = url.to_string();
        let sub_channel = channel.clone();
        let sub_handlers = Arc::clone(&handlers);
        let sub_instance = instance_id.clone();
        thread::Builder::new()
            .name("pylon-cluster-redis-sub".into())
            .spawn(move || {
                run_subscriber_loop(&sub_url, &sub_channel, &sub_instance, sub_handlers);
            })
            .map_err(|e| format!("spawn redis subscriber thread: {e}"))?;

        info!("[cluster] redis bus connected — channel=\"{channel}\" instance_id={instance_id}");

        Ok(Self {
            client,
            channel,
            instance_id,
            publish_conn: Mutex::new(publish_conn),
            handlers,
        })
    }

    fn publish_with_reconnect(&self, payload: &str) -> Result<(), RedisError> {
        // Try once on the cached conn. If it fails (connection died),
        // reconnect and try once more. This handles the most common
        // failure mode — Redis primary cycled overnight while the
        // pylon process kept its TCP conn ostensibly open.
        let mut guard = self.publish_conn.lock().unwrap_or_else(|p| p.into_inner());
        let first: Result<(), RedisError> = guard.publish(&self.channel, payload);
        if first.is_ok() {
            return Ok(());
        }
        warn!(
            "[cluster] redis publish failed ({}); reconnecting + retrying",
            first.as_ref().unwrap_err()
        );
        match self.client.get_connection() {
            Ok(fresh) => {
                *guard = fresh;
                guard.publish(&self.channel, payload)
            }
            Err(e) => Err(e),
        }
    }
}

impl ClusterBus for RedisBus {
    fn publish(&self, envelope: &Envelope) {
        let body = match serde_json::to_string(envelope) {
            Ok(s) => s,
            Err(e) => {
                error!("[cluster] envelope serialize failed: {e}");
                return;
            }
        };
        if let Err(e) = self.publish_with_reconnect(&body) {
            error!("[cluster] redis publish failed after reconnect: {e}");
        }
    }

    fn subscribe(&self, handler: SubscriberHandler) {
        let mut h = self.handlers.lock().unwrap_or_else(|p| p.into_inner());
        h.push(handler);
    }

    fn instance_id(&self) -> &str {
        &self.instance_id
    }
}

/// Long-running subscriber loop. Holds a SUBSCRIBE connection, parses
/// inbound messages as [`Envelope`]s, filters self-originated, fans
/// to every registered handler. Reconnects with exponential backoff
/// (capped at 30s) on connection failures.
fn run_subscriber_loop(
    url: &str,
    channel: &str,
    self_instance_id: &str,
    handlers: Arc<Mutex<Vec<SubscriberHandler>>>,
) {
    let mut backoff_secs: u64 = 1;
    loop {
        match Client::open(url) {
            Ok(client) => match client.get_connection() {
                Ok(conn) => {
                    backoff_secs = 1;
                    if let Err(e) = run_one_subscription(conn, channel, self_instance_id, &handlers)
                    {
                        warn!("[cluster] redis subscriber connection ended: {e}");
                    }
                }
                Err(e) => {
                    warn!("[cluster] redis subscriber get_connection failed: {e}");
                }
            },
            Err(e) => {
                warn!("[cluster] redis subscriber Client::open failed: {e}");
            }
        }
        debug!("[cluster] reconnecting redis subscriber in {backoff_secs}s");
        thread::sleep(Duration::from_secs(backoff_secs));
        backoff_secs = (backoff_secs * 2).min(30);
    }
}

fn run_one_subscription(
    mut conn: redis::Connection,
    channel: &str,
    self_instance_id: &str,
    handlers: &Arc<Mutex<Vec<SubscriberHandler>>>,
) -> Result<(), RedisError> {
    // The conn stays in SUBSCRIBE mode for its lifetime once
    // `as_pubsub` is called. Returning out of `get_message` only
    // happens on connection error; the outer loop reconnects.
    let mut pubsub = conn.as_pubsub();
    pubsub.subscribe(channel)?;
    info!("[cluster] redis subscriber listening on channel \"{channel}\"");
    loop {
        let msg = pubsub.get_message()?;
        let payload: String = match msg.get_payload() {
            Ok(s) => s,
            Err(e) => {
                debug!("[cluster] non-string redis payload dropped: {e}");
                continue;
            }
        };
        let envelope: Envelope = match serde_json::from_str(&payload) {
            Ok(e) => e,
            Err(e) => {
                debug!("[cluster] malformed envelope dropped: {e}");
                continue;
            }
        };
        if envelope.instance_id == self_instance_id {
            // Self-originated message looping back — drop. The local
            // path already broadcast this event in-process.
            continue;
        }
        // Snapshot the handler list under the lock, then drop the
        // lock before invoking handlers so a slow handler doesn't
        // block subscribe() calls from other threads.
        let snapshot: Vec<SubscriberHandler> = {
            let h = handlers.lock().unwrap_or_else(|p| p.into_inner());
            h.iter().map(Arc::clone).collect()
        };
        for handler in snapshot {
            // catch_unwind protects the subscriber thread from a
            // handler panic — pylon's mutation hot path should never
            // panic, but defense-in-depth on the bus thread keeps
            // cross-machine fanout alive through any single bug.
            let env_clone = envelope.clone();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                handler(env_clone);
            }));
            if let Err(panic) = result {
                error!(
                    "[cluster] subscriber handler panicked: {:?}",
                    panic
                        .downcast_ref::<&'static str>()
                        .copied()
                        .or_else(|| panic.downcast_ref::<String>().map(|s| s.as_str()))
                        .unwrap_or("<unknown panic>")
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Integration tests — run only when PYLON_TEST_REDIS_URL is set.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Envelope;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn redis_url() -> Option<String> {
        std::env::var("PYLON_TEST_REDIS_URL").ok()
    }

    #[test]
    fn publish_self_is_filtered() {
        let Some(url) = redis_url() else {
            return;
        };
        let bus = RedisBus::connect(&url, Some("pylon-test-self")).expect("connect");
        let count = Arc::new(AtomicUsize::new(0));
        let count_clone = Arc::clone(&count);
        bus.subscribe(Arc::new(move |_env| {
            count_clone.fetch_add(1, Ordering::SeqCst);
        }));
        // Give subscriber thread a moment to land on SUBSCRIBE.
        std::thread::sleep(Duration::from_millis(200));
        let inst = bus.instance_id().to_string();
        for _ in 0..5 {
            let env = Envelope::change(
                &inst,
                &pylon_sync::ChangeEvent {
                    seq: 1,
                    entity: "T".into(),
                    row_id: "1".into(),
                    kind: pylon_sync::ChangeKind::Insert,
                    data: None,
                    prev_data: None,
                    timestamp: "".into(),
                },
            );
            bus.publish(&env);
        }
        std::thread::sleep(Duration::from_millis(500));
        // Every published envelope is self-originated → filtered.
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn cross_instance_round_trip() {
        let Some(url) = redis_url() else {
            return;
        };
        let bus_a = RedisBus::connect(&url, Some("pylon-test-rt")).expect("a");
        let bus_b = RedisBus::connect(&url, Some("pylon-test-rt")).expect("b");
        let received = Arc::new(Mutex::new(Vec::<String>::new()));
        let received_b = Arc::clone(&received);
        bus_b.subscribe(Arc::new(move |env| {
            received_b.lock().unwrap().push(env.payload.to_string());
        }));
        std::thread::sleep(Duration::from_millis(200));
        let env = Envelope::change(
            bus_a.instance_id(),
            &pylon_sync::ChangeEvent {
                seq: 7,
                entity: "X".into(),
                row_id: "9".into(),
                kind: pylon_sync::ChangeKind::Update,
                data: Some(serde_json::json!({"k": "v"})),
                prev_data: None,
                timestamp: "".into(),
            },
        );
        bus_a.publish(&env);
        std::thread::sleep(Duration::from_millis(500));
        let got = received.lock().unwrap();
        assert!(
            got.iter().any(|p| p.contains("\"k\":\"v\"")),
            "cross-instance delivery: {got:?}"
        );
    }
}
