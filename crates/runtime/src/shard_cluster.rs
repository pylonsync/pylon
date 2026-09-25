//! Shards across machines: a directory in Postgres, placement, and the
//! calls machines make to each other.
//!
//! An app on Postgres can run on several machines. Each machine registers
//! in `_pylon_shard_machines` and renews a heartbeat every [`HEARTBEAT`].
//! Each shard has one row in `_pylon_shard_placements` naming the machine
//! that runs it, with the kind and create params, so any machine can start
//! it again. A shard whose module saves state (see `pylon_save` in
//! pylon-shard-guest) keeps its latest state in `_pylon_shard_state`.
//!
//! - **Placement.** `ctx.shards.create` on any machine picks the live
//!   machine with the most free capacity ([`choose`]; load is the number of
//!   shards placed on it), or the machine the call pins, and the create runs
//!   there.
//! - **Routing.** A client may reach any machine. One that does not run the
//!   shard sends the connection on: on Fly, with a `fly-replay` response so
//!   Fly's proxy connects the client to the right machine directly;
//!   elsewhere, by proxying it over the machines' private addresses (see
//!   `shard_route`).
//! - **Failover.** A machine with no heartbeat for [`DEAD_AFTER`] is dead.
//!   Each of its shards has one new home, the same on every machine
//!   ([`home`], rendezvous hashing); that machine moves the placement to
//!   itself (a compare-and-set) and starts the shard from its saved state,
//!   or from `init` when its module saves none.
//! - **Restarts.** A machine that comes back under the same id (a crash
//!   and restart, a deploy) finds placements on itself with nothing running
//!   and starts them, from saved state.
//! - **Fencing.** A machine whose last heartbeat is older than
//!   [`FENCE_AFTER`] (it cannot reach the database) stops its shards: the
//!   others may already be starting them. A machine that finds a shard it
//!   runs placed elsewhere stops its copy.
//!
//! Machines call each other at `POST /_pylon/shards/op`, and mark requests
//! they forward, with signatures under a key kept in the directory (every
//! machine with the database has it; nothing to configure). A signature
//! binds the receiving machine and a nonce, and each is accepted once.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use hmac::{Hmac, Mac};
use pylon_storage::pg_datastore::PgPool;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// How often a machine renews its heartbeat.
pub const HEARTBEAT: Duration = Duration::from_secs(2);
/// A machine silent this long is dead and its shards move.
pub const DEAD_AFTER: Duration = Duration::from_secs(10);
/// A machine that has not renewed its heartbeat for this long stops its
/// shards: well before [`DEAD_AFTER`], so a machine cut off from the
/// database is never still running a shard another machine started.
pub const FENCE_AFTER: Duration = Duration::from_secs(5);
/// Signed calls older than this are refused.
const CALL_WINDOW_MS: i64 = 30_000;
/// Header that carries a machine-to-machine call's signature.
pub const AUTH_HEADER: &str = "X-Pylon-Cluster-Auth";
/// Header on a request one machine passed to another: the receiver serves
/// it locally and never passes it on.
pub const FORWARDED_HEADER: &str = "X-Pylon-Shard-Forwarded";

const SCHEMA_LOCK_ID: i64 = 0x5059_5348;

/// This machine, as the directory knows it.
#[derive(Debug, Clone, PartialEq)]
pub struct MachineConfig {
    /// `PYLON_REPLICA_ID`, `FLY_MACHINE_ID`, or `FLY_ALLOC_ID`; else the
    /// host name and process id, so a restart is a new machine (the old id
    /// goes dead and its shards start again from saved state).
    pub id: String,
    /// Base URL other machines reach this one at (`PYLON_SHARD_ADVERTISE_URL`,
    /// or `http://[FLY_PRIVATE_IP]:<port>` on Fly). None: other machines
    /// cannot proxy to or call this one.
    pub address: Option<String>,
    /// Shards this machine takes before placement prefers others
    /// (`PYLON_SHARD_CAPACITY`, default 100).
    pub capacity: u32,
    /// `FLY_MACHINE_ID`: the instance `fly-replay` names. On Fly, routing
    /// answers with `fly-replay` instead of proxying.
    pub fly_instance: Option<String>,
}

impl MachineConfig {
    /// The config from the environment, for a server on `port`.
    pub fn from_env(port: u16) -> Self {
        Self::from_lookup(|k| std::env::var(k).ok(), port, std::process::id())
    }

    fn from_lookup(get: impl Fn(&str) -> Option<String>, port: u16, pid: u32) -> Self {
        let clean = |k: &str| {
            get(k)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let id = ["PYLON_REPLICA_ID", "FLY_MACHINE_ID", "FLY_ALLOC_ID"]
            .iter()
            .find_map(|k| clean(k))
            .unwrap_or_else(|| {
                format!(
                    "{}:{pid}",
                    clean("HOSTNAME").unwrap_or_else(|| "local".into())
                )
            });
        let address = clean("PYLON_SHARD_ADVERTISE_URL")
            .map(|u| u.trim_end_matches('/').to_string())
            .or_else(|| clean("FLY_PRIVATE_IP").map(|ip| format!("http://[{ip}]:{port}")));
        let capacity = clean("PYLON_SHARD_CAPACITY")
            .and_then(|v| v.parse().ok())
            .filter(|&c| c > 0)
            .unwrap_or(100);
        Self {
            id,
            address,
            capacity,
            fly_instance: clean("FLY_MACHINE_ID"),
        }
    }
}

/// A live machine.
#[derive(Debug, Clone, PartialEq)]
pub struct Machine {
    pub id: String,
    pub address: Option<String>,
    pub fly_instance: Option<String>,
    pub capacity: u32,
    /// Shards placed on it.
    pub load: u32,
}

impl Machine {
    fn free(&self) -> i64 {
        self.capacity as i64 - self.load as i64
    }
}

/// Where a shard runs.
#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    pub shard_id: String,
    pub kind: String,
    pub params: serde_json::Value,
    pub machine_id: String,
    pub pinned: bool,
    /// Why the machine could not start it, when it could not. A failed
    /// placement keeps its saved state and is not started again until an
    /// operator stops it.
    pub failed: Option<String>,
}

/// What [`PgShardDirectory::claim`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Claim {
    Claimed,
    /// The shard id is placed already (on any machine).
    Taken,
    /// The kind has `max` shards placed across the cluster.
    LimitReached,
}

/// The machine a new shard should go to: the most free capacity, then the
/// lowest id.
pub fn choose(machines: &[Machine]) -> Option<&Machine> {
    machines
        .iter()
        .max_by(|a, b| a.free().cmp(&b.free()).then_with(|| b.id.cmp(&a.id)))
}

/// The new home of an orphaned shard: rendezvous hashing over the machines
/// with free capacity (all of them when none has). Every machine computes
/// the same answer from the same live set, whatever load it read, and a
/// dead machine's shards spread across the others.
pub fn home<'a>(shard_id: &str, machines: &'a [Machine]) -> Option<&'a Machine> {
    let with_room: Vec<&Machine> = machines.iter().filter(|m| m.free() > 0).collect();
    let pool: Vec<&Machine> = if with_room.is_empty() {
        machines.iter().collect()
    } else {
        with_room
    };
    pool.into_iter().max_by_key(|m| {
        let digest = Sha256::new()
            .chain_update(shard_id.as_bytes())
            .chain_update([0])
            .chain_update(m.id.as_bytes())
            .finalize();
        let mut score = [0u8; 8];
        score.copy_from_slice(&digest[..8]);
        (u64::from_be_bytes(score), m.id.clone())
    })
}

/// The directory in Postgres.
pub struct PgShardDirectory {
    pool: Arc<PgPool>,
}

impl PgShardDirectory {
    /// Create the tables, and load the cluster key into this process.
    pub fn open(pool: Arc<PgPool>) -> Result<Self, String> {
        let dir = Self { pool };
        let key = dir.pool.with_client(|client| {
            let mut tx = client.transaction()?;
            tx.execute("SELECT pg_advisory_xact_lock($1)", &[&SCHEMA_LOCK_ID])?;
            // Only what is missing: DDL on a table in use takes locks that
            // deadlock with other machines' claims (a claim reads, then
            // inserts), and every machine runs this at boot.
            let exists =
                |tx: &mut postgres::Transaction<'_>, name: &str| -> Result<bool, postgres::Error> {
                    Ok(tx
                        .query_one("SELECT to_regclass($1) IS NOT NULL", &[&name])?
                        .get(0))
                };
            let column = |tx: &mut postgres::Transaction<'_>,
                          table: &str,
                          col: &str|
             -> Result<bool, postgres::Error> {
                Ok(tx
                    .query_one(
                        "SELECT EXISTS (SELECT 1 FROM information_schema.columns
                                        WHERE table_name = $1 AND column_name = $2)",
                        &[&table, &col],
                    )?
                    .get(0))
            };
            if !exists(&mut tx, "_pylon_shard_machines")? {
                tx.batch_execute(
                    "CREATE TABLE _pylon_shard_machines (
                        machine_id TEXT PRIMARY KEY,
                        address TEXT,
                        fly_instance TEXT,
                        capacity INTEGER NOT NULL,
                        heartbeat_at BIGINT NOT NULL
                    )",
                )?;
            }
            if !column(&mut tx, "_pylon_shard_machines", "fly_instance")? {
                tx.batch_execute("ALTER TABLE _pylon_shard_machines ADD COLUMN fly_instance TEXT")?;
            }
            if column(&mut tx, "_pylon_shard_machines", "load")? {
                tx.batch_execute("ALTER TABLE _pylon_shard_machines DROP COLUMN load")?;
            }
            if !exists(&mut tx, "_pylon_shard_placements")? {
                tx.batch_execute(
                    "CREATE TABLE _pylon_shard_placements (
                        shard_id TEXT PRIMARY KEY,
                        kind TEXT NOT NULL,
                        params JSONB NOT NULL,
                        machine_id TEXT NOT NULL,
                        pinned BOOLEAN NOT NULL DEFAULT FALSE,
                        failed TEXT,
                        created_at BIGINT NOT NULL,
                        moved_at BIGINT
                    );
                    CREATE INDEX _pylon_shard_placements_machine_idx
                        ON _pylon_shard_placements (machine_id);",
                )?;
            }
            if !column(&mut tx, "_pylon_shard_placements", "failed")? {
                tx.batch_execute("ALTER TABLE _pylon_shard_placements ADD COLUMN failed TEXT")?;
            }
            if !exists(&mut tx, "_pylon_shard_state")? {
                tx.batch_execute(
                    "CREATE TABLE _pylon_shard_state (
                        shard_id TEXT PRIMARY KEY,
                        state BYTEA NOT NULL,
                        saved_at BIGINT NOT NULL
                    )",
                )?;
            }
            if !exists(&mut tx, "_pylon_shard_cluster")? {
                tx.batch_execute(
                    "CREATE TABLE _pylon_shard_cluster (
                        id INTEGER PRIMARY KEY,
                        call_key BYTEA NOT NULL
                    )",
                )?;
            }
            let fresh: Vec<u8> = (0..32).map(|_| rand::random::<u8>()).collect();
            tx.execute(
                "INSERT INTO _pylon_shard_cluster (id, call_key) VALUES (1, $1)
                 ON CONFLICT (id) DO NOTHING",
                &[&fresh],
            )?;
            let key: Vec<u8> = tx
                .query_one(
                    "SELECT call_key FROM _pylon_shard_cluster WHERE id = 1",
                    &[],
                )?
                .get(0);
            tx.commit()?;
            Ok(key)
        })?;
        let _ = CALL_KEY.set(key);
        Ok(dir)
    }

    /// Renew this machine's row. Times come from the database clock, so
    /// machines with skewed clocks agree on who is alive.
    pub fn heartbeat(&self, me: &MachineConfig) -> Result<(), String> {
        let capacity = me.capacity as i32;
        self.pool.with_client(|c| {
            c.execute(
                "INSERT INTO _pylon_shard_machines (machine_id, address, fly_instance, capacity, heartbeat_at)
                 VALUES ($1, $2, $3, $4, (extract(epoch from clock_timestamp()) * 1000)::bigint)
                 ON CONFLICT (machine_id) DO UPDATE SET
                    address = EXCLUDED.address, fly_instance = EXCLUDED.fly_instance,
                    capacity = EXCLUDED.capacity, heartbeat_at = EXCLUDED.heartbeat_at",
                &[&me.id, &me.address, &me.fly_instance, &capacity],
            )?;
            Ok(())
        })
    }

    /// Machines with a heartbeat within [`DEAD_AFTER`], with the number of
    /// shards placed on each.
    pub fn live_machines(&self) -> Result<Vec<Machine>, String> {
        let window = DEAD_AFTER.as_millis() as i64;
        self.pool.with_client(|c| {
            let rows = c.query(
                "SELECT m.machine_id, m.address, m.fly_instance, m.capacity,
                        (SELECT count(*) FROM _pylon_shard_placements p
                         WHERE p.machine_id = m.machine_id)
                 FROM _pylon_shard_machines m
                 WHERE m.heartbeat_at > (extract(epoch from clock_timestamp()) * 1000)::bigint - $1
                 ORDER BY m.machine_id",
                &[&window],
            )?;
            Ok(rows
                .iter()
                .map(|r| Machine {
                    id: r.get(0),
                    address: r.get(1),
                    fly_instance: r.get(2),
                    capacity: r.get::<_, i32>(3).max(0) as u32,
                    load: r.get::<_, i64>(4).clamp(0, u32::MAX as i64) as u32,
                })
                .collect())
        })
    }

    /// Record that `p.machine_id` runs the shard, unless the id is placed
    /// already or its kind has `max` shards across the cluster. The count
    /// and the insert run under a lock on the kind, so two machines cannot
    /// both take the last slot.
    pub fn claim(&self, p: &Placement, max: usize) -> Result<Claim, String> {
        let max = max.min(i64::MAX as usize) as i64;
        self.pool.with_client_once(|c| {
            let mut tx = c.transaction()?;
            tx.execute(
                "SELECT pg_advisory_xact_lock($1, hashtext($2))",
                &[&(SCHEMA_LOCK_ID as i32), &p.kind],
            )?;
            let placed: i64 = tx
                .query_one(
                    "SELECT count(*) FROM _pylon_shard_placements WHERE kind = $1",
                    &[&p.kind],
                )?
                .get(0);
            if placed >= max {
                return Ok(Claim::LimitReached);
            }
            let n = tx.execute(
                "INSERT INTO _pylon_shard_placements
                    (shard_id, kind, params, machine_id, pinned, created_at)
                 VALUES ($1, $2, $3, $4, $5, (extract(epoch from clock_timestamp()) * 1000)::bigint)
                 ON CONFLICT (shard_id) DO NOTHING",
                &[&p.shard_id, &p.kind, &p.params, &p.machine_id, &p.pinned],
            )?;
            tx.commit()?;
            Ok(if n == 1 { Claim::Claimed } else { Claim::Taken })
        })
    }

    pub fn placement(&self, shard_id: &str) -> Result<Option<Placement>, String> {
        self.pool.with_client(|c| {
            let row = c.query_opt(
                "SELECT shard_id, kind, params, machine_id, pinned, failed
                 FROM _pylon_shard_placements WHERE shard_id = $1",
                &[&shard_id],
            )?;
            Ok(row.map(|r| placement_of(&r)))
        })
    }

    /// Every placement on a machine not in `live`.
    pub fn orphans(&self, live: &[String]) -> Result<Vec<Placement>, String> {
        self.pool.with_client(|c| {
            let rows = c.query(
                "SELECT shard_id, kind, params, machine_id, pinned, failed
                 FROM _pylon_shard_placements
                 WHERE NOT (machine_id = ANY($1)) ORDER BY shard_id",
                &[&live],
            )?;
            Ok(rows.iter().map(placement_of).collect())
        })
    }

    /// Every placement on `machine_id`.
    pub fn placements_on(&self, machine_id: &str) -> Result<Vec<Placement>, String> {
        self.pool.with_client(|c| {
            let rows = c.query(
                "SELECT shard_id, kind, params, machine_id, pinned, failed
                 FROM _pylon_shard_placements WHERE machine_id = $1 ORDER BY shard_id",
                &[&machine_id],
            )?;
            Ok(rows.iter().map(placement_of).collect())
        })
    }

    /// Move a placement from `from` to `to`. False when another machine
    /// moved it first (or it is gone).
    pub fn take_over(&self, shard_id: &str, from: &str, to: &str) -> Result<bool, String> {
        self.pool.with_client_once(|c| {
            let n = c.execute(
                "UPDATE _pylon_shard_placements
                 SET machine_id = $3, moved_at = (extract(epoch from clock_timestamp()) * 1000)::bigint
                 WHERE shard_id = $1 AND machine_id = $2",
                &[&shard_id, &from, &to],
            )?;
            Ok(n == 1)
        })
    }

    /// Record why `machine_id` could not start the shard. Its state stays.
    pub fn mark_failed(&self, shard_id: &str, machine_id: &str, why: &str) -> Result<(), String> {
        self.pool.with_client(|c| {
            c.execute(
                "UPDATE _pylon_shard_placements SET failed = $3
                 WHERE shard_id = $1 AND machine_id = $2",
                &[&shard_id, &machine_id, &why],
            )?;
            Ok(())
        })
    }

    /// Forget a shard `machine_id` ran (it ended), with its saved state.
    /// Placements another machine took over are left alone.
    pub fn release(&self, shard_id: &str, machine_id: &str) -> Result<(), String> {
        self.pool.with_client(|c| {
            let mut tx = c.transaction()?;
            let n = tx.execute(
                "DELETE FROM _pylon_shard_placements WHERE shard_id = $1 AND machine_id = $2",
                &[&shard_id, &machine_id],
            )?;
            if n == 1 {
                tx.execute(
                    "DELETE FROM _pylon_shard_state WHERE shard_id = $1",
                    &[&shard_id],
                )?;
            }
            tx.commit()
        })
    }

    /// Forget a shard whatever machine it is on (an operator stopped it).
    pub fn forget(&self, shard_id: &str) -> Result<(), String> {
        self.pool.with_client(|c| {
            let mut tx = c.transaction()?;
            tx.execute(
                "DELETE FROM _pylon_shard_placements WHERE shard_id = $1",
                &[&shard_id],
            )?;
            tx.execute(
                "DELETE FROM _pylon_shard_state WHERE shard_id = $1",
                &[&shard_id],
            )?;
            tx.commit()
        })
    }

    /// Store a shard's state, only while `machine_id` still owns it: a
    /// machine that was cut off cannot overwrite the state of the copy that
    /// replaced it. False when it does not own the shard.
    pub fn save_state(
        &self,
        shard_id: &str,
        machine_id: &str,
        state: &[u8],
    ) -> Result<bool, String> {
        self.pool.with_client(|c| {
            let n = c.execute(
                "INSERT INTO _pylon_shard_state (shard_id, state, saved_at)
                 SELECT $1, $3, (extract(epoch from clock_timestamp()) * 1000)::bigint
                 WHERE EXISTS (SELECT 1 FROM _pylon_shard_placements
                               WHERE shard_id = $1 AND machine_id = $2)
                 ON CONFLICT (shard_id) DO UPDATE SET
                    state = EXCLUDED.state, saved_at = EXCLUDED.saved_at",
                &[&shard_id, &machine_id, &state],
            )?;
            Ok(n == 1)
        })
    }

    pub fn load_state(&self, shard_id: &str) -> Result<Option<Vec<u8>>, String> {
        self.pool.with_client(|c| {
            let row = c.query_opt(
                "SELECT state FROM _pylon_shard_state WHERE shard_id = $1",
                &[&shard_id],
            )?;
            Ok(row.map(|r| r.get(0)))
        })
    }

    /// Remove this machine's row: it is shutting down, and its shards should
    /// move now rather than after [`DEAD_AFTER`].
    pub fn leave(&self, machine_id: &str) -> Result<(), String> {
        self.pool.with_client(|c| {
            c.execute(
                "DELETE FROM _pylon_shard_machines WHERE machine_id = $1",
                &[&machine_id],
            )?;
            Ok(())
        })
    }

    /// Drop machine rows silent for an hour.
    pub fn prune_machines(&self) -> Result<(), String> {
        self.pool.with_client(|c| {
            c.execute(
                "DELETE FROM _pylon_shard_machines
                 WHERE heartbeat_at < (extract(epoch from clock_timestamp()) * 1000)::bigint - 3600000",
                &[],
            )?;
            Ok(())
        })
    }
}

fn placement_of(r: &postgres::Row) -> Placement {
    Placement {
        shard_id: r.get(0),
        kind: r.get(1),
        params: r.get(2),
        machine_id: r.get(3),
        pinned: r.get(4),
        failed: r.get(5),
    }
}

// ---------------------------------------------------------------------------
// Machine-to-machine calls
// ---------------------------------------------------------------------------

/// A shard operation one machine asks another to run locally.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "op")]
pub enum RemoteOp {
    Create {
        kind: String,
        id: String,
        params: serde_json::Value,
        pinned: bool,
    },
    Stop {
        id: String,
    },
    Get {
        id: String,
    },
    List,
}

/// The answer to a [`RemoteOp`]: a JSON value, or an error code and message.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RemoteReply {
    Ok(serde_json::Value),
    Err { code: String, message: String },
}

/// The key machines sign calls with, loaded from the directory.
static CALL_KEY: OnceLock<Vec<u8>> = OnceLock::new();

/// The shard ticket key derived from the directory's key, when this
/// machine joined the directory (see `shard_tickets::ticket_secret`).
pub fn ticket_key() -> Option<Vec<u8>> {
    let key = CALL_KEY.get()?;
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC takes any key length");
    mac.update(b"pylon-shard-ticket-v1");
    Some(mac.finalize().into_bytes().to_vec())
}

/// Signatures seen within the window, so each is accepted once.
fn seen() -> &'static Mutex<HashMap<String, i64>> {
    static SEEN: OnceLock<Mutex<HashMap<String, i64>>> = OnceLock::new();
    SEEN.get_or_init(|| Mutex::new(HashMap::new()))
}

fn mac(key: &[u8], parts: &[&[u8]]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC takes any key length");
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            mac.update(b"\n");
        }
        mac.update(part);
    }
    mac.finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn nonce() -> String {
    (0..12)
        .map(|_| format!("{:02x}", rand::random::<u8>()))
        .collect()
}

/// A signature of `payload` for machine `to`: `<ms>.<nonce>.<mac>`. None
/// before the directory loaded the key.
pub fn sign(to: &str, payload: &[u8]) -> Option<String> {
    let key = CALL_KEY.get()?;
    Some(sign_with(key, now_ms(), &nonce(), to, payload))
}

fn sign_with(key: &[u8], at: i64, nonce: &str, to: &str, payload: &[u8]) -> String {
    let at_s = at.to_string();
    let sig = mac(
        key,
        &[at_s.as_bytes(), nonce.as_bytes(), to.as_bytes(), payload],
    );
    format!("{at}.{nonce}.{sig}")
}

/// Check a signature addressed to machine `me`: the key, its age, and that
/// it was not seen before.
pub fn verify(me: &str, header: &str, payload: &[u8]) -> Result<(), &'static str> {
    let key = CALL_KEY
        .get()
        .ok_or("this machine is not in the shard directory")?;
    let now = now_ms();
    verify_with(key, me, header, payload, now)?;
    let mut seen = seen().lock().unwrap();
    seen.retain(|_, at| now - *at <= CALL_WINDOW_MS * 2);
    if seen.insert(header.to_string(), now).is_some() {
        return Err("signature already used");
    }
    Ok(())
}

fn verify_with(
    key: &[u8],
    me: &str,
    header: &str,
    payload: &[u8],
    now: i64,
) -> Result<(), &'static str> {
    let mut parts = header.splitn(3, '.');
    let (Some(at), Some(nonce), Some(sig)) = (parts.next(), parts.next(), parts.next()) else {
        return Err("malformed signature");
    };
    let at: i64 = at.parse().map_err(|_| "malformed signature")?;
    if (now - at).abs() > CALL_WINDOW_MS {
        return Err("signature expired");
    }
    let at_s = at.to_string();
    let want = mac(
        key,
        &[at_s.as_bytes(), nonce.as_bytes(), me.as_bytes(), payload],
    );
    if pylon_auth::constant_time_eq(want.as_bytes(), sig.as_bytes()) {
        Ok(())
    } else {
        Err("bad signature")
    }
}

/// Run `op` on machine `to` at `address`.
pub fn call(to: &str, address: &str, op: &RemoteOp) -> Result<RemoteReply, String> {
    let body = serde_json::to_vec(op).map_err(|e| e.to_string())?;
    let signature = sign(to, &body).ok_or("this machine is not in the shard directory")?;
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(3))
        .timeout(Duration::from_secs(15))
        .redirects(0)
        .build();
    let url = format!("{address}/_pylon/shards/op");
    let response = agent
        .post(&url)
        .set("Content-Type", "application/json")
        .set(AUTH_HEADER, &signature)
        .send_bytes(&body);
    let response = match response {
        Ok(r) => r,
        Err(ureq::Error::Status(_, r)) => r,
        Err(e) => return Err(format!("machine {to} at {address} unreachable: {e}")),
    };
    let text = response
        .into_string()
        .map_err(|e| format!("reading the reply from {to}: {e}"))?;
    serde_json::from_str(&text).map_err(|e| format!("bad reply from {to}: {e}: {text}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(id: &str, capacity: u32, load: u32) -> Machine {
        Machine {
            id: id.into(),
            address: None,
            fly_instance: None,
            capacity,
            load,
        }
    }

    #[test]
    fn placement_prefers_free_capacity_then_the_lowest_id() {
        let ms = [m("b", 10, 2), m("a", 10, 2), m("c", 10, 5)];
        assert_eq!(choose(&ms).unwrap().id, "a");
        let ms = [m("b", 10, 1), m("a", 10, 2)];
        assert_eq!(choose(&ms).unwrap().id, "b");
        // Over capacity still places (the least over), rather than failing.
        let ms = [m("a", 1, 5), m("b", 1, 3)];
        assert_eq!(choose(&ms).unwrap().id, "b");
        assert!(choose(&[]).is_none());
    }

    #[test]
    fn failover_homes_agree_and_spread() {
        let ms = [m("a", 10, 0), m("b", 10, 9), m("c", 10, 3)];
        // The same answer whatever load each machine read.
        let other_view = [m("a", 10, 5), m("b", 10, 0), m("c", 10, 1)];
        let mut per_machine: HashMap<String, usize> = HashMap::new();
        for i in 0..300 {
            let id = format!("shard-{i}");
            let h = home(&id, &ms).unwrap();
            assert_eq!(h.id, home(&id, &other_view).unwrap().id);
            *per_machine.entry(h.id.clone()).or_default() += 1;
        }
        for (id, n) in &per_machine {
            assert!(*n > 60, "{id} got {n} of 300");
        }
        // A full machine takes none while another has room.
        let full = [m("a", 1, 1), m("b", 10, 0)];
        for i in 0..50 {
            assert_eq!(home(&format!("s{i}"), &full).unwrap().id, "b");
        }
        assert!(home("x", &[]).is_none());
    }

    #[test]
    fn machine_config_from_the_environment() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |k: &str| {
                pairs
                    .iter()
                    .find(|(n, _)| *n == k)
                    .map(|(_, v)| v.to_string())
            }
        };
        let bare = MachineConfig::from_lookup(env(&[("HOSTNAME", "box")]), 4321, 77);
        assert_eq!(bare.id, "box:77");
        assert_eq!((bare.address, bare.fly_instance), (None, None));
        let fly = MachineConfig::from_lookup(
            env(&[("FLY_MACHINE_ID", "e2865"), ("FLY_PRIVATE_IP", "fdaa::3")]),
            8080,
            1,
        );
        assert_eq!(fly.id, "e2865");
        assert_eq!(fly.fly_instance.as_deref(), Some("e2865"));
        assert_eq!(fly.address.as_deref(), Some("http://[fdaa::3]:8080"));
        assert_eq!(fly.capacity, 100);
        // A replica id on Fly names the machine; fly-replay still names the
        // Fly instance.
        let named = MachineConfig::from_lookup(
            env(&[
                ("PYLON_REPLICA_ID", "zone-host-1"),
                ("FLY_MACHINE_ID", "e2865"),
            ]),
            8080,
            1,
        );
        assert_eq!(named.id, "zone-host-1");
        assert_eq!(named.fly_instance.as_deref(), Some("e2865"));
        let own = MachineConfig::from_lookup(
            env(&[
                ("PYLON_REPLICA_ID", "b"),
                ("PYLON_SHARD_ADVERTISE_URL", "http://10.0.0.2:4321/"),
                ("PYLON_SHARD_CAPACITY", "8"),
            ]),
            4321,
            1,
        );
        assert_eq!(own.address.as_deref(), Some("http://10.0.0.2:4321"));
        assert_eq!((own.capacity, own.fly_instance), (8, None));
    }

    #[test]
    fn signatures_bind_the_key_the_receiver_the_payload_and_the_time() {
        let key = b"k";
        let body = br#"{"op":"list"}"#;
        let at = 1_000_000;
        let header = sign_with(key, at, "n1", "b", body);
        assert_eq!(verify_with(key, "b", &header, body, at + 1000), Ok(()));
        assert_eq!(
            verify_with(key, "a", &header, body, at),
            Err("bad signature"),
            "another machine"
        );
        assert_eq!(
            verify_with(key, "b", &header, b"{}", at),
            Err("bad signature"),
            "another body"
        );
        assert_eq!(
            verify_with(key, "b", &header, body, at + CALL_WINDOW_MS + 1),
            Err("signature expired")
        );
        assert_eq!(
            verify_with(b"other", "b", &header, body, at),
            Err("bad signature")
        );
        assert_eq!(
            verify_with(key, "b", "nonsense", body, at),
            Err("malformed signature")
        );
        let tampered = header.replacen("n1", "n2", 1);
        assert_eq!(
            verify_with(key, "b", &tampered, body, at),
            Err("bad signature")
        );
    }

    #[test]
    fn remote_ops_round_trip_as_json() {
        let op = RemoteOp::Create {
            kind: "arena".into(),
            id: "m1".into(),
            params: serde_json::json!({ "w": 1 }),
            pinned: true,
        };
        let text = serde_json::to_string(&op).unwrap();
        assert!(text.contains("\"op\":\"create\""), "{text}");
        assert_eq!(serde_json::from_str::<RemoteOp>(&text).unwrap(), op);
        assert_eq!(
            serde_json::from_str::<RemoteOp>(r#"{"op":"list"}"#).unwrap(),
            RemoteOp::List
        );
    }

    fn test_dir() -> Option<PgShardDirectory> {
        let Ok(url) = std::env::var("PYLON_TEST_PG_URL") else {
            eprintln!("skipping: PYLON_TEST_PG_URL not set");
            return None;
        };
        let pool = PgPool::connect(&url, 4, Duration::from_secs(5)).expect("test Postgres pool");
        Some(PgShardDirectory::open(pool).expect("directory"))
    }

    fn me(id: &str) -> MachineConfig {
        MachineConfig {
            id: id.into(),
            address: Some(format!("http://{id}")),
            capacity: 10,
            fly_instance: None,
        }
    }

    fn placement(shard: &str, kind: &str, machine: &str) -> Placement {
        Placement {
            shard_id: shard.into(),
            kind: kind.into(),
            params: serde_json::json!({ "w": 800 }),
            machine_id: machine.into(),
            pinned: false,
            failed: None,
        }
    }

    #[test]
    fn the_directory_claims_moves_fences_fails_and_forgets() {
        let Some(dir) = test_dir() else { return };
        // The key is in the directory now, the same for every machine.
        assert!(sign("x", b"y").is_some());
        let run = pylon_cluster::new_instance_id();
        let (a, b) = (format!("a-{run}"), format!("b-{run}"));
        let shard = format!("s-{run}");
        dir.heartbeat(&me(&a)).unwrap();
        dir.heartbeat(&me(&b)).unwrap();

        let p = placement(&shard, "arena", &b);
        assert_eq!(dir.claim(&p, 10).unwrap(), Claim::Claimed);
        let other = Placement {
            machine_id: a.clone(),
            ..p.clone()
        };
        assert_eq!(dir.claim(&other, 10).unwrap(), Claim::Taken);
        assert_eq!(dir.placement(&shard).unwrap(), Some(p.clone()));
        // Load is the number of placements.
        let live = dir.live_machines().unwrap();
        assert_eq!(live.iter().find(|m| m.id == b).unwrap().load, 1);
        assert_eq!(live.iter().find(|m| m.id == a).unwrap().load, 0);

        // Only the owner saves state.
        assert!(dir.save_state(&shard, &b, b"state-1").unwrap());
        assert!(!dir.save_state(&shard, &a, b"stale").unwrap());
        assert_eq!(
            dir.load_state(&shard).unwrap().as_deref(),
            Some(&b"state-1"[..])
        );

        // B dies: the shard is an orphan; one machine takes it over.
        let orphans = dir.orphans(std::slice::from_ref(&a)).unwrap();
        assert!(orphans.iter().any(|o| o.shard_id == shard));
        assert!(dir.take_over(&shard, &b, &a).unwrap());
        assert!(!dir.take_over(&shard, &b, &a).unwrap(), "only one wins");
        assert!(!dir
            .save_state(&shard, &b, b"from the cut-off machine")
            .unwrap());
        assert!(dir.save_state(&shard, &a, b"state-2").unwrap());

        // A cannot start it: the placement says why, and the state stays.
        dir.mark_failed(&shard, &a, "restore refused").unwrap();
        let failed = dir.placement(&shard).unwrap().unwrap();
        assert_eq!(failed.failed.as_deref(), Some("restore refused"));
        assert_eq!(
            dir.load_state(&shard).unwrap().as_deref(),
            Some(&b"state-2"[..])
        );
        assert_eq!(dir.placements_on(&a).unwrap().len(), 1);

        // B's release of a shard it lost changes nothing; A's removes it
        // and its state.
        dir.release(&shard, &b).unwrap();
        assert!(dir.placement(&shard).unwrap().is_some());
        dir.release(&shard, &a).unwrap();
        assert_eq!(dir.placement(&shard).unwrap(), None);
        assert_eq!(dir.load_state(&shard).unwrap(), None);

        assert_eq!(dir.claim(&p, 10).unwrap(), Claim::Claimed);
        dir.forget(&shard).unwrap();
        assert_eq!(dir.placement(&shard).unwrap(), None);
        dir.leave(&a).unwrap();
        assert!(!dir.live_machines().unwrap().iter().any(|m| m.id == a));
    }

    #[test]
    fn concurrent_claims_never_pass_the_kind_limit() {
        let Some(dir) = test_dir() else { return };
        let dir = Arc::new(dir);
        let run = pylon_cluster::new_instance_id();
        let kind = format!("k-{run}");
        let threads: Vec<_> = (0..8)
            .map(|i| {
                let dir = Arc::clone(&dir);
                let (kind, shard) = (kind.clone(), format!("s{i}-{run}"));
                std::thread::spawn(move || {
                    dir.claim(&placement(&shard, &kind, &format!("m{i}")), 3)
                        .unwrap()
                })
            })
            .collect();
        let results: Vec<Claim> = threads.into_iter().map(|t| t.join().unwrap()).collect();
        assert_eq!(results.iter().filter(|c| **c == Claim::Claimed).count(), 3);
        assert_eq!(
            results
                .iter()
                .filter(|c| **c == Claim::LimitReached)
                .count(),
            5
        );
        for i in 0..8 {
            dir.forget(&format!("s{i}-{run}")).unwrap();
        }
    }
}
