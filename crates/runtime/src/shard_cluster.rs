//! Shards across machines: a directory in Postgres, placement, and the
//! calls machines make to each other.
//!
//! An app on Postgres can run on several machines. Each machine registers
//! in `_pylon_shard_machines` and sends a heartbeat every
//! [`HEARTBEAT`]. Each shard has one row in `_pylon_shard_placements`
//! naming the machine that runs it, with the kind and create params, so any
//! machine can start it again. A shard whose module saves state (see
//! `pylon_save` in pylon-shard-guest) keeps its latest state in
//! `_pylon_shard_state`.
//!
//! - **Placement.** `ctx.shards.create` on any machine picks the live
//!   machine with the most free capacity ([`choose`]), or the machine the
//!   call pins, and the create runs there.
//! - **Routing.** A client may reach any machine. One that does not run the
//!   shard sends the connection on: on Fly, with a `fly-replay` response so
//!   Fly's proxy connects the client to the right machine directly;
//!   elsewhere, by proxying it over the machines' private addresses (see
//!   `shard_route`).
//! - **Failover.** A machine with no heartbeat for [`DEAD_AFTER`] is dead.
//!   Every live machine computes the same new home for each of its shards
//!   ([`choose`]); the chosen machine moves the placement to itself (a
//!   compare-and-set, so only one wins) and starts the shard from its saved
//!   state, or from `init` when its module saves none.
//! - **Fencing.** A machine that finds a shard it runs placed elsewhere (it
//!   was cut off and its shards moved) stops its copy.
//!
//! Machines call each other at `POST /_pylon/shards/op`, signed with a key
//! derived from the shard ticket secret (the same on every machine).

use std::sync::Arc;
use std::time::Duration;

use hmac::{Hmac, Mac};
use pylon_storage::pg_datastore::PgPool;
use serde::{Deserialize, Serialize};
use sha2::Sha256;

/// How often a machine renews its heartbeat, saves state, and checks for
/// dead machines.
pub const HEARTBEAT: Duration = Duration::from_secs(2);
/// A machine silent this long is dead and its shards move.
pub const DEAD_AFTER: Duration = Duration::from_secs(10);
/// Signed calls older than this are refused (replay window).
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
    /// cannot proxy to or call this one, so it only runs shards clients reach
    /// directly (or through `fly-replay`).
    pub address: Option<String>,
    /// Shards this machine takes before placement prefers others
    /// (`PYLON_SHARD_CAPACITY`, default 100).
    pub capacity: u32,
    /// True on Fly: routing answers with `fly-replay`.
    pub fly: bool,
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
        let fly = clean("FLY_MACHINE_ID").is_some();
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
            fly,
        }
    }
}

/// A live machine.
#[derive(Debug, Clone, PartialEq)]
pub struct Machine {
    pub id: String,
    pub address: Option<String>,
    pub capacity: u32,
    pub load: u32,
}

/// Where a shard runs.
#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    pub shard_id: String,
    pub kind: String,
    pub params: serde_json::Value,
    pub machine_id: String,
    pub pinned: bool,
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

/// The machine a new shard should go to: the most free capacity (capacity
/// minus load), then the lowest id. Every machine computes the same answer
/// from the same rows, which failover relies on.
pub fn choose(machines: &[Machine]) -> Option<&Machine> {
    machines.iter().max_by(|a, b| {
        let free = |m: &Machine| m.capacity as i64 - m.load as i64;
        free(a).cmp(&free(b)).then_with(|| b.id.cmp(&a.id))
    })
}

/// The directory in Postgres.
pub struct PgShardDirectory {
    pool: Arc<PgPool>,
}

fn ms(d: Duration) -> i64 {
    d.as_millis() as i64
}

impl PgShardDirectory {
    pub fn open(pool: Arc<PgPool>) -> Result<Self, String> {
        let dir = Self { pool };
        dir.pool.with_client(|client| {
            let mut tx = client.transaction()?;
            tx.execute("SELECT pg_advisory_xact_lock($1)", &[&SCHEMA_LOCK_ID])?;
            tx.batch_execute(
                "CREATE TABLE IF NOT EXISTS _pylon_shard_machines (
                    machine_id TEXT PRIMARY KEY,
                    address TEXT,
                    capacity INTEGER NOT NULL,
                    load INTEGER NOT NULL,
                    heartbeat_at BIGINT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS _pylon_shard_placements (
                    shard_id TEXT PRIMARY KEY,
                    kind TEXT NOT NULL,
                    params JSONB NOT NULL,
                    machine_id TEXT NOT NULL,
                    pinned BOOLEAN NOT NULL DEFAULT FALSE,
                    created_at BIGINT NOT NULL,
                    moved_at BIGINT
                );
                CREATE INDEX IF NOT EXISTS _pylon_shard_placements_machine_idx
                    ON _pylon_shard_placements (machine_id);
                CREATE TABLE IF NOT EXISTS _pylon_shard_state (
                    shard_id TEXT PRIMARY KEY,
                    state BYTEA NOT NULL,
                    saved_at BIGINT NOT NULL
                );",
            )?;
            tx.commit()
        })?;
        Ok(dir)
    }

    /// Renew this machine's row. Times come from the database clock, so
    /// machines with skewed clocks agree on who is alive.
    pub fn heartbeat(&self, me: &MachineConfig, load: u32) -> Result<(), String> {
        let capacity = me.capacity as i32;
        let load = load as i32;
        self.pool.with_client(|c| {
            c.execute(
                "INSERT INTO _pylon_shard_machines (machine_id, address, capacity, load, heartbeat_at)
                 VALUES ($1, $2, $3, $4, (extract(epoch from clock_timestamp()) * 1000)::bigint)
                 ON CONFLICT (machine_id) DO UPDATE SET
                    address = EXCLUDED.address, capacity = EXCLUDED.capacity,
                    load = EXCLUDED.load, heartbeat_at = EXCLUDED.heartbeat_at",
                &[&me.id, &me.address, &capacity, &load],
            )?;
            Ok(())
        })
    }

    /// Machines with a heartbeat within [`DEAD_AFTER`].
    pub fn live_machines(&self) -> Result<Vec<Machine>, String> {
        let window = ms(DEAD_AFTER);
        self.pool.with_client(|c| {
            let rows = c.query(
                "SELECT machine_id, address, capacity, load FROM _pylon_shard_machines
                 WHERE heartbeat_at > (extract(epoch from clock_timestamp()) * 1000)::bigint - $1
                 ORDER BY machine_id",
                &[&window],
            )?;
            Ok(rows
                .iter()
                .map(|r| Machine {
                    id: r.get(0),
                    address: r.get(1),
                    capacity: r.get::<_, i32>(2).max(0) as u32,
                    load: r.get::<_, i32>(3).max(0) as u32,
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

    pub fn placement(&self, shard_id: &str) -> Result<Option<Placement>, String> {
        self.pool.with_client(|c| {
            let row = c.query_opt(
                "SELECT shard_id, kind, params, machine_id, pinned
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
                "SELECT shard_id, kind, params, machine_id, pinned
                 FROM _pylon_shard_placements
                 WHERE NOT (machine_id = ANY($1)) ORDER BY shard_id",
                &[&live],
            )?;
            Ok(rows.iter().map(placement_of).collect())
        })
    }

    /// The shard ids placed on `machine_id`.
    pub fn shards_on(&self, machine_id: &str) -> Result<Vec<String>, String> {
        self.pool.with_client(|c| {
            let rows = c.query(
                "SELECT shard_id FROM _pylon_shard_placements WHERE machine_id = $1",
                &[&machine_id],
            )?;
            Ok(rows.iter().map(|r| r.get(0)).collect())
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

    /// Forget a shard `machine_id` ran (it stopped), with its saved state.
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

    /// Forget a shard whatever machine it is on (an operator stopped a shard
    /// on a dead machine).
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
    /// Start a shard this machine just took over (failover).
    Adopt {
        id: String,
    },
}

/// The answer to a [`RemoteOp`]: a JSON value, or an error code and message.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RemoteReply {
    Ok(serde_json::Value),
    Err { code: String, message: String },
}

fn call_key() -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(crate::shard_tickets::ticket_secret())
        .expect("HMAC takes any key length");
    mac.update(b"pylon-shard-cluster-v1");
    mac.finalize().into_bytes().to_vec()
}

fn signature(key: &[u8], at_ms: i64, body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC takes any key length");
    mac.update(at_ms.to_string().as_bytes());
    mac.update(b"\n");
    mac.update(body);
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

/// The `X-Pylon-Cluster-Auth` value for `body`.
pub fn sign(body: &[u8]) -> String {
    let at = now_ms();
    format!("{at}.{}", signature(&call_key(), at, body))
}

/// Check a call's signature and age.
pub fn verify(header: &str, body: &[u8]) -> Result<(), &'static str> {
    verify_at(&call_key(), header, body, now_ms())
}

fn verify_at(key: &[u8], header: &str, body: &[u8], now: i64) -> Result<(), &'static str> {
    let (at, sig) = header.split_once('.').ok_or("malformed signature")?;
    let at: i64 = at.parse().map_err(|_| "malformed signature")?;
    if (now - at).abs() > CALL_WINDOW_MS {
        return Err("signature expired");
    }
    let want = signature(key, at, body);
    if pylon_auth::constant_time_eq(want.as_bytes(), sig.as_bytes()) {
        Ok(())
    } else {
        Err("bad signature")
    }
}

/// Run `op` on the machine at `address`.
pub fn call(address: &str, op: &RemoteOp) -> Result<RemoteReply, String> {
    let body = serde_json::to_vec(op).map_err(|e| e.to_string())?;
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(3))
        .timeout(Duration::from_secs(15))
        .build();
    let url = format!("{address}/_pylon/shards/op");
    let response = agent
        .post(&url)
        .set("Content-Type", "application/json")
        .set(AUTH_HEADER, &sign(&body))
        .set(FORWARDED_HEADER, "1")
        .send_bytes(&body);
    let response = match response {
        Ok(r) => r,
        Err(ureq::Error::Status(_, r)) => r,
        Err(e) => return Err(format!("machine at {address} unreachable: {e}")),
    };
    let text = response
        .into_string()
        .map_err(|e| format!("reading the reply from {address}: {e}"))?;
    serde_json::from_str(&text).map_err(|e| format!("bad reply from {address}: {e}: {text}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(id: &str, capacity: u32, load: u32) -> Machine {
        Machine {
            id: id.into(),
            address: None,
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
        assert_eq!((bare.address, bare.fly), (None, false));
        let fly = MachineConfig::from_lookup(
            env(&[("FLY_MACHINE_ID", "e2865"), ("FLY_PRIVATE_IP", "fdaa::3")]),
            8080,
            1,
        );
        assert_eq!(fly.id, "e2865");
        assert!(fly.fly);
        assert_eq!(fly.address.as_deref(), Some("http://[fdaa::3]:8080"));
        assert_eq!(fly.capacity, 100);
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
        assert_eq!((own.capacity, own.fly), (8, false));
    }

    #[test]
    fn signed_calls_verify_and_expire() {
        let key = b"k";
        let body = br#"{"op":"list"}"#;
        let at = 1_000_000;
        let header = format!("{at}.{}", signature(key, at, body));
        assert_eq!(verify_at(key, &header, body, at + 1000), Ok(()));
        assert_eq!(
            verify_at(key, &header, b"{}", at),
            Err("bad signature"),
            "another body"
        );
        assert_eq!(
            verify_at(key, &header, body, at + CALL_WINDOW_MS + 1),
            Err("signature expired")
        );
        assert_eq!(verify_at(b"other", &header, body, at), Err("bad signature"));
        assert_eq!(
            verify_at(key, "nonsense", body, at),
            Err("malformed signature")
        );
    }

    fn test_dir() -> Option<PgShardDirectory> {
        let Ok(url) = std::env::var("PYLON_TEST_PG_URL") else {
            eprintln!("skipping: PYLON_TEST_PG_URL not set");
            return None;
        };
        let pool = PgPool::connect(&url, 2, Duration::from_secs(5)).expect("test Postgres pool");
        Some(PgShardDirectory::open(pool).expect("directory"))
    }

    fn me(id: &str) -> MachineConfig {
        MachineConfig {
            id: id.into(),
            address: Some(format!("http://{id}")),
            capacity: 10,
            fly: false,
        }
    }

    #[test]
    fn the_directory_claims_moves_fences_and_forgets() {
        let Some(dir) = test_dir() else { return };
        let run = pylon_cluster::new_instance_id();
        let (a, b) = (format!("a-{run}"), format!("b-{run}"));
        let shard = format!("s-{run}");
        dir.heartbeat(&me(&a), 3).unwrap();
        dir.heartbeat(&me(&b), 1).unwrap();
        let live = dir.live_machines().unwrap();
        assert!(live.iter().any(|m| m.id == a && m.load == 3));
        assert!(live
            .iter()
            .any(|m| m.id == b && m.address.as_deref() == Some(&*format!("http://{b}"))));

        let p = Placement {
            shard_id: shard.clone(),
            kind: "arena".into(),
            params: serde_json::json!({ "w": 800 }),
            machine_id: b.clone(),
            pinned: true,
        };
        assert_eq!(dir.claim(&p, 10).unwrap(), Claim::Claimed);
        let other = Placement {
            machine_id: a.clone(),
            ..p.clone()
        };
        assert_eq!(dir.claim(&other, 10).unwrap(), Claim::Taken);
        // The kind's limit counts placements on every machine.
        let one = Placement {
            shard_id: format!("one-{run}"),
            kind: format!("k-{run}"),
            machine_id: a.clone(),
            ..p.clone()
        };
        assert_eq!(dir.claim(&one, 1).unwrap(), Claim::Claimed);
        let two = Placement {
            shard_id: format!("two-{run}"),
            machine_id: b.clone(),
            ..one.clone()
        };
        assert_eq!(dir.claim(&two, 1).unwrap(), Claim::LimitReached);
        dir.forget(&one.shard_id).unwrap();
        assert_eq!(dir.placement(&shard).unwrap(), Some(p.clone()));
        assert!(dir.shards_on(&b).unwrap().contains(&shard));

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
}
