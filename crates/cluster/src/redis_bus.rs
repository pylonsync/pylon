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

use crate::best_effort::BestEffort;
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

/// How long the subscriber waits for a message before it PINGs Redis (and
/// then for the answer before it reconnects).
#[cfg(not(test))]
const SUBSCRIBER_IDLE: Duration = Duration::from_secs(15);
#[cfg(test)]
const SUBSCRIBER_IDLE: Duration = Duration::from_millis(200);

/// How long a publish connection waits for Redis before it fails.
const PUBLISH_TIMEOUT: Duration = Duration::from_secs(10);
/// The same for at-most-once envelopes, which are dropped on a failure.
const BEST_EFFORT_TIMEOUT: Duration = Duration::from_secs(2);

/// A connection whose every step, the setup included, fails after
/// `timeout`, so a Redis that takes the connection and stops answering
/// cannot hang a caller. redis-rs runs its setup commands (AUTH, SELECT)
/// before a read timeout can be set: the connection is opened with no
/// password and no database (with `disable-client-setinfo` the setup then
/// sends nothing), the timeouts are set, and AUTH and SELECT are sent here.
/// The connection speaks RESP2.
fn connect_with_timeouts(
    client: &Client,
    timeout: Duration,
) -> Result<redis::Connection, RedisError> {
    let info = client.get_connection_info();
    let bare = redis::ConnectionInfo {
        addr: info.addr.clone(),
        redis: redis::RedisConnectionInfo {
            db: 0,
            username: None,
            password: None,
            protocol: redis::ProtocolVersion::RESP2,
        },
    };
    let mut conn = Client::open(bare)?.get_connection_with_timeout(timeout)?;
    conn.set_read_timeout(Some(timeout))?;
    conn.set_write_timeout(Some(timeout))?;
    if let Some(password) = &info.redis.password {
        let mut auth = redis::cmd("AUTH");
        if let Some(user) = &info.redis.username {
            auth.arg(user);
        }
        match auth.arg(password).query::<()>(&mut conn) {
            Ok(()) => {}
            // Redis before 6 takes no user name (the old `redis://h:pass@`
            // URLs): the password alone, as redis-rs does.
            Err(e)
                if info.redis.username.is_some()
                    && e.to_string().contains("wrong number of arguments") =>
            {
                redis::cmd("AUTH").arg(password).query::<()>(&mut conn)?;
            }
            Err(e) => return Err(e),
        }
    }
    if info.redis.db != 0 {
        redis::cmd("SELECT")
            .arg(info.redis.db)
            .query::<()>(&mut conn)?;
    }
    Ok(conn)
}

pub struct RedisBus {
    client: Client,
    channel: String,
    instance_id: String,
    /// Publish connection. Wrapped in Mutex because the `redis` sync
    /// client isn't Sync at the connection level. The mutex contention
    /// is bounded by publish rate; pylon emits ≤O(mutations/sec) which
    /// is two orders of magnitude below the mutex's ceiling.
    publish_conn: Mutex<redis::Connection>,
    /// At-most-once envelopes (messages between shards), on a connection
    /// of their own, apart from committed changes.
    best_effort: BestEffort,
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
        let publish_conn = connect_with_timeouts(&client, PUBLISH_TIMEOUT)
            .map_err(|e| format!("redis publish connect: {e}"))?;
        let best_effort = {
            let client = client.clone();
            let channel = channel.clone();
            let mut conn: Option<redis::Connection> = None;
            BestEffort::spawn("pylon-cluster-redis-best-effort", move |envelope| {
                let body = serde_json::to_string(envelope).map_err(|e| e.to_string())?;
                if conn.is_none() {
                    conn = Some(
                        connect_with_timeouts(&client, BEST_EFFORT_TIMEOUT)
                            .map_err(|e| e.to_string())?,
                    );
                }
                let c = conn.as_mut().expect("connected above");
                // Once: a PUBLISH whose reply was lost may have been
                // delivered, and a retry would deliver it twice.
                c.publish::<_, _, ()>(&channel, body).map_err(|e| {
                    conn = None;
                    e.to_string()
                })
            })?
        };
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
            best_effort,
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
        match connect_with_timeouts(&self.client, PUBLISH_TIMEOUT) {
            Ok(fresh) => {
                *guard = fresh;
                guard.publish(&self.channel, payload)
            }
            Err(e) => Err(e),
        }
    }
}

impl ClusterBus for RedisBus {
    fn try_publish(&self, envelope: &Envelope) -> bool {
        self.best_effort.try_send(envelope)
    }

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
            Ok(client) => match connect_with_timeouts(&client, PUBLISH_TIMEOUT) {
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
    // In SUBSCRIBE mode for the connection's lifetime. A read that waits
    // SUBSCRIBER_IDLE sends a PING; a PING with no answer by the next
    // wait means the connection is gone (a failover, a NAT drop), and the
    // outer loop reconnects.
    conn.send_packed_command(&redis::cmd("SUBSCRIBE").arg(channel).get_packed_command())?;
    conn.set_read_timeout(Some(SUBSCRIBER_IDLE))?;
    info!("[cluster] redis subscriber listening on channel \"{channel}\"");
    let mut pinged = false;
    loop {
        let value = match conn.recv_response() {
            Ok(value) => value,
            Err(e) if e.is_timeout() && !pinged => {
                conn.send_packed_command(&redis::cmd("PING").get_packed_command())?;
                pinged = true;
                continue;
            }
            Err(e) => return Err(e),
        };
        pinged = false;
        // Subscribe confirmations and PING answers are not messages.
        let Some(msg) = redis::Msg::from_owned_value(value) else {
            continue;
        };
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

    /// A server that takes the connection and never answers: the connect
    /// (with a password, so AUTH waits for an answer) fails within its
    /// timeout, every time.
    #[test]
    fn a_silent_redis_cannot_hang_a_connect() {
        let silent = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = silent.local_addr().unwrap().port();
        let held: Arc<Mutex<Vec<std::net::TcpStream>>> = Arc::default();
        {
            let held = Arc::clone(&held);
            thread::spawn(move || {
                for conn in silent.incoming().flatten() {
                    held.lock().unwrap().push(conn);
                }
            });
        }
        for url in [
            format!("redis://:secret@127.0.0.1:{port}/0"),
            format!("redis://127.0.0.1:{port}/3"),
            format!("redis://:secret@127.0.0.1:{port}/0"),
        ] {
            let client = Client::open(url).unwrap();
            let started = std::time::Instant::now();
            assert!(connect_with_timeouts(&client, Duration::from_millis(200)).is_err());
            assert!(
                started.elapsed() < Duration::from_secs(2),
                "{:?}",
                started.elapsed()
            );
        }
        // No password and no database: nothing waits for an answer.
        let client = Client::open(format!("redis://127.0.0.1:{port}")).unwrap();
        assert!(connect_with_timeouts(&client, Duration::from_millis(200)).is_ok());
    }

    /// A Redis before 6 refuses AUTH with a user name; the connect sends
    /// the password alone, then SELECT.
    #[test]
    fn auth_falls_back_to_the_password_alone() {
        use std::io::{BufRead, BufReader, Write};
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = server.local_addr().unwrap().port();
        let seen: Arc<Mutex<Vec<Vec<String>>>> = Arc::default();
        {
            let seen = Arc::clone(&seen);
            thread::spawn(move || {
                let (conn, _) = server.accept().unwrap();
                let mut out = conn.try_clone().unwrap();
                let mut reader = BufReader::new(conn);
                // Read one RESP array of bulk strings.
                let mut read_cmd = || -> Option<Vec<String>> {
                    let mut line = String::new();
                    reader.read_line(&mut line).ok()?;
                    let n: usize = line.trim().strip_prefix('*')?.parse().ok()?;
                    let mut args = Vec::new();
                    for _ in 0..n {
                        line.clear();
                        reader.read_line(&mut line).ok()?;
                        line.clear();
                        reader.read_line(&mut line).ok()?;
                        args.push(line.trim_end().to_string());
                    }
                    Some(args)
                };
                while let Some(cmd) = read_cmd() {
                    let reply: &[u8] = if cmd.len() == 3 && cmd[0] == "AUTH" {
                        b"-ERR wrong number of arguments for 'auth' command\r\n"
                    } else {
                        b"+OK\r\n"
                    };
                    seen.lock().unwrap().push(cmd);
                    out.write_all(reply).unwrap();
                }
            });
        }
        let client = Client::open(format!("redis://h:secret@127.0.0.1:{port}/2")).unwrap();
        connect_with_timeouts(&client, Duration::from_secs(2)).unwrap();
        assert_eq!(
            *seen.lock().unwrap(),
            [
                vec!["AUTH", "h", "secret"],
                vec!["AUTH", "secret"],
                vec!["SELECT", "2"],
            ]
        );
    }

    /// The subscriber PINGs an idle connection, takes messages after the
    /// answer, and ends (to reconnect) when a PING goes unanswered.
    #[test]
    fn an_idle_subscriber_pings_and_a_silent_one_ends() {
        use std::io::{BufRead, BufReader, Write};
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = server.local_addr().unwrap().port();
        let envelope = serde_json::to_string(&Envelope {
            instance_id: "other".into(),
            kind: "change".into(),
            payload: serde_json::json!({ "n": 1 }),
        })
        .unwrap();
        thread::spawn(move || {
            let (conn, _) = server.accept().unwrap();
            let mut out = conn.try_clone().unwrap();
            let mut reader = BufReader::new(conn);
            let mut read_cmd = || -> Option<Vec<String>> {
                let mut line = String::new();
                reader.read_line(&mut line).ok()?;
                let n: usize = line.trim().strip_prefix('*')?.parse().ok()?;
                let mut args = Vec::new();
                for _ in 0..n {
                    line.clear();
                    reader.read_line(&mut line).ok()?;
                    line.clear();
                    reader.read_line(&mut line).ok()?;
                    args.push(line.trim_end().to_string());
                }
                Some(args)
            };
            let bulk = |s: &str| format!("${}\r\n{s}\r\n", s.len());
            assert_eq!(read_cmd().unwrap()[0], "SUBSCRIBE");
            write!(out, "*3\r\n{}{}:1\r\n", bulk("subscribe"), bulk("ch")).unwrap();
            // Idle: the client PINGs; answer, then send a message.
            assert_eq!(read_cmd().unwrap()[0], "PING");
            write!(out, "*2\r\n{}{}", bulk("pong"), bulk("")).unwrap();
            write!(
                out,
                "*3\r\n{}{}{}",
                bulk("message"),
                bulk("ch"),
                bulk(&envelope)
            )
            .unwrap();
            // Then silent: the next PING gets no answer.
            let _ = read_cmd();
            thread::sleep(Duration::from_secs(5));
        });
        let client = Client::open(format!("redis://127.0.0.1:{port}")).unwrap();
        let conn = connect_with_timeouts(&client, Duration::from_secs(2)).unwrap();
        let got: Arc<Mutex<Vec<Envelope>>> = Arc::default();
        let handlers: Arc<Mutex<Vec<SubscriberHandler>>> = Arc::default();
        {
            let got = Arc::clone(&got);
            handlers
                .lock()
                .unwrap()
                .push(Arc::new(move |e: Envelope| got.lock().unwrap().push(e)));
        }
        let started = std::time::Instant::now();
        let ended = run_one_subscription(conn, "ch", "me", &handlers);
        assert!(ended.is_err());
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "{:?}",
            started.elapsed()
        );
        assert_eq!(got.lock().unwrap().len(), 1);
    }
}
