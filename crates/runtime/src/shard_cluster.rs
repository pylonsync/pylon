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
/// A machine's lease: how long after it SENT a heartbeat that reached the
/// directory it may run shards. The directory writes the heartbeat after it
/// was sent, so other machines count the machine live until at least
/// send + [`DEAD_AFTER`]; the difference is the margin that a cut-off
/// machine has stopped its shards before another may start them.
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
    /// Its current lease epoch (see [`Placement::epoch`]).
    pub epoch: i64,
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
    /// The lease epoch of the machine when it took the shard. A machine
    /// takes a new epoch each time it starts and each time its lease lapses,
    /// so a placement is valid only while its machine is live with the same
    /// epoch; any other placement is an orphan, whatever its machine id.
    pub epoch: i64,
}

/// A player in flight between shards (see [`PgShardDirectory::begin_transfer`]).
///
/// The row decides who holds the player: `out` (the row: the source removed
/// it and nothing has taken it yet), `in` (the target), or `back` (the
/// source again). The target and the source each take the row from `out`
/// with a compare-and-set, so exactly one of them does.
#[derive(Debug, Clone, PartialEq)]
pub struct Transfer {
    pub id: String,
    pub subscriber: String,
    pub from_shard: String,
    pub to_shard: String,
    /// The entity state the source's `transfer_out` produced.
    pub state: Vec<u8>,
    /// The user and ticket claims the target's ticket carries:
    /// `{"user_id": ..., "claims": ...}`.
    pub auth: serde_json::Value,
    pub status: String,
}

/// What settling a transfer row found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settle {
    /// The row was `out` and is now this side's, with the state saved.
    Done,
    /// The row is no longer `out`: the other side settled it first.
    Taken,
    /// This machine no longer holds the shard (its lease lapsed, or it
    /// moved). The row is unchanged; the shard's holder finishes it.
    NotHeld,
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
                        heartbeat_at BIGINT NOT NULL,
                        epoch BIGINT NOT NULL DEFAULT 0
                    )",
                )?;
            }
            if !column(&mut tx, "_pylon_shard_machines", "fly_instance")? {
                tx.batch_execute("ALTER TABLE _pylon_shard_machines ADD COLUMN fly_instance TEXT")?;
            }
            if !column(&mut tx, "_pylon_shard_machines", "epoch")? {
                tx.batch_execute(
                    "ALTER TABLE _pylon_shard_machines ADD COLUMN epoch BIGINT NOT NULL DEFAULT 0",
                )?;
            }
            // An unused column from before; kept (a machine on an older
            // build may still write it) but no longer required.
            if column(&mut tx, "_pylon_shard_machines", "load")? {
                tx.batch_execute(
                    "ALTER TABLE _pylon_shard_machines ALTER COLUMN load DROP NOT NULL",
                )?;
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
                        epoch BIGINT NOT NULL DEFAULT 0,
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
            if !column(&mut tx, "_pylon_shard_placements", "epoch")? {
                tx.batch_execute(
                    "ALTER TABLE _pylon_shard_placements ADD COLUMN epoch BIGINT NOT NULL DEFAULT 0",
                )?;
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
            if !exists(&mut tx, "_pylon_shard_transfers")? {
                tx.batch_execute(
                    "CREATE TABLE _pylon_shard_transfers (
                        transfer_id TEXT PRIMARY KEY,
                        subscriber TEXT NOT NULL,
                        from_shard TEXT NOT NULL,
                        to_shard TEXT NOT NULL,
                        state BYTEA NOT NULL,
                        auth JSONB NOT NULL,
                        status TEXT NOT NULL,
                        created_at BIGINT NOT NULL
                    );
                    CREATE INDEX _pylon_shard_transfers_out_idx
                        ON _pylon_shard_transfers (from_shard) WHERE status = 'out';",
                )?;
            }
            if !column(&mut tx, "_pylon_shard_transfers", "refused_at")? {
                tx.batch_execute("ALTER TABLE _pylon_shard_transfers ADD COLUMN refused_at BIGINT")?;
            }
            if !exists(&mut tx, "_pylon_shard_transfers_age_idx")? {
                tx.batch_execute(
                    "CREATE INDEX _pylon_shard_transfers_age_idx
                        ON _pylon_shard_transfers (status, created_at)",
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

    /// Renew this machine's row under lease `epoch`. Times come from the
    /// database clock, so machines with skewed clocks agree on who is alive.
    ///
    /// False when another process holds the id under another epoch and is
    /// still live: a second process with the same id waits until the first
    /// leaves or goes silent for [`DEAD_AFTER`], so it never takes shards
    /// the first is still running.
    pub fn heartbeat(&self, me: &MachineConfig, epoch: i64) -> Result<bool, String> {
        let capacity = me.capacity as i32;
        let window = DEAD_AFTER.as_millis() as i64;
        self.pool.with_client(|c| {
            let n = c.execute(
                "INSERT INTO _pylon_shard_machines AS m
                    (machine_id, address, fly_instance, capacity, heartbeat_at, epoch)
                 VALUES ($1, $2, $3, $4, (extract(epoch from clock_timestamp()) * 1000)::bigint, $5)
                 ON CONFLICT (machine_id) DO UPDATE SET
                    address = EXCLUDED.address, fly_instance = EXCLUDED.fly_instance,
                    capacity = EXCLUDED.capacity, heartbeat_at = EXCLUDED.heartbeat_at,
                    epoch = EXCLUDED.epoch
                 WHERE m.epoch = EXCLUDED.epoch
                    OR m.heartbeat_at <= (extract(epoch from clock_timestamp()) * 1000)::bigint - $6",
                &[&me.id, &me.address, &me.fly_instance, &capacity, &epoch, &window],
            )?;
            Ok(n == 1)
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
                         WHERE p.machine_id = m.machine_id),
                        m.epoch
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
                    epoch: r.get(5),
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
                    (shard_id, kind, params, machine_id, pinned, epoch, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6, (extract(epoch from clock_timestamp()) * 1000)::bigint)
                 ON CONFLICT (shard_id) DO NOTHING",
                &[&p.shard_id, &p.kind, &p.params, &p.machine_id, &p.pinned, &p.epoch],
            )?;
            tx.commit()?;
            Ok(if n == 1 { Claim::Claimed } else { Claim::Taken })
        })
    }

    pub fn placement(&self, shard_id: &str) -> Result<Option<Placement>, String> {
        self.pool.with_client(|c| {
            let row = c.query_opt(
                "SELECT shard_id, kind, params, machine_id, pinned, failed, epoch
                 FROM _pylon_shard_placements WHERE shard_id = $1",
                &[&shard_id],
            )?;
            Ok(row.map(|r| placement_of(&r)))
        })
    }

    /// Every placement whose machine is not live under the placement's
    /// epoch: the machine died, left, restarted, or had its lease lapse.
    pub fn orphans(&self) -> Result<Vec<Placement>, String> {
        let window = DEAD_AFTER.as_millis() as i64;
        self.pool.with_client(|c| {
            let rows = c.query(
                &format!(
                    "SELECT p.shard_id, p.kind, p.params, p.machine_id, p.pinned, p.failed, p.epoch
                     FROM _pylon_shard_placements p
                     WHERE NOT {} ORDER BY p.shard_id",
                    owner_live_sql("p.machine_id", "p.epoch", "$1")
                ),
                &[&window],
            )?;
            Ok(rows.iter().map(placement_of).collect())
        })
    }

    /// Every placement on `machine_id`.
    pub fn placements_on(&self, machine_id: &str) -> Result<Vec<Placement>, String> {
        self.pool.with_client(|c| {
            let rows = c.query(
                "SELECT shard_id, kind, params, machine_id, pinned, failed, epoch
                 FROM _pylon_shard_placements WHERE machine_id = $1 ORDER BY shard_id",
                &[&machine_id],
            )?;
            Ok(rows.iter().map(placement_of).collect())
        })
    }

    /// Move orphan `p` to machine `to` under lease `to_epoch`. False when
    /// another machine moved it first, it is gone, or its machine is live
    /// under its epoch again: the check and the move are one statement.
    pub fn take_over(&self, p: &Placement, to: &str, to_epoch: i64) -> Result<bool, String> {
        let window = DEAD_AFTER.as_millis() as i64;
        self.pool.with_client_once(|c| {
            let n = c.execute(
                &format!(
                    "UPDATE _pylon_shard_placements p
                     SET machine_id = $4, epoch = $5,
                         moved_at = (extract(epoch from clock_timestamp()) * 1000)::bigint
                     WHERE p.shard_id = $1 AND p.machine_id = $2 AND p.epoch = $3
                       AND NOT {}",
                    owner_live_sql("p.machine_id", "p.epoch", "$6")
                ),
                &[
                    &p.shard_id,
                    &p.machine_id,
                    &p.epoch,
                    &to,
                    &to_epoch,
                    &window,
                ],
            )?;
            Ok(n == 1)
        })
    }

    /// Move shard `shard_id` from `from` (under `from_epoch`) to `to` under
    /// `to_epoch`: `from` is shutting down and hands it over. False when
    /// `from` no longer holds it under that epoch.
    pub fn hand_over(
        &self,
        shard_id: &str,
        from: &str,
        from_epoch: i64,
        to: &str,
        to_epoch: i64,
    ) -> Result<bool, String> {
        self.pool.with_client_once(|c| {
            let n = c.execute(
                "UPDATE _pylon_shard_placements
                 SET machine_id = $4, epoch = $5, failed = NULL,
                     moved_at = (extract(epoch from clock_timestamp()) * 1000)::bigint
                 WHERE shard_id = $1 AND machine_id = $2 AND epoch = $3",
                &[&shard_id, &from, &from_epoch, &to, &to_epoch],
            )?;
            Ok(n == 1)
        })
    }

    /// Record why the machine could not start the shard it holds under
    /// `epoch`. Its state stays.
    pub fn mark_failed(
        &self,
        shard_id: &str,
        machine_id: &str,
        epoch: i64,
        why: &str,
    ) -> Result<(), String> {
        self.pool.with_client(|c| {
            c.execute(
                "UPDATE _pylon_shard_placements SET failed = $4
                 WHERE shard_id = $1 AND machine_id = $2 AND epoch = $3",
                &[&shard_id, &machine_id, &epoch, &why],
            )?;
            Ok(())
        })
    }

    /// Forget the shard `machine_id` holds under `epoch`, with its saved
    /// state: it ended, or an operator stopped it. A placement another
    /// machine (or a later epoch) took over is left alone. False when
    /// nothing matched.
    pub fn release(&self, shard_id: &str, machine_id: &str, epoch: i64) -> Result<bool, String> {
        self.pool.with_client(|c| {
            let mut tx = c.transaction()?;
            let n = tx.execute(
                "DELETE FROM _pylon_shard_placements
                 WHERE shard_id = $1 AND machine_id = $2 AND epoch = $3",
                &[&shard_id, &machine_id, &epoch],
            )?;
            if n == 1 {
                tx.execute(
                    "DELETE FROM _pylon_shard_state WHERE shard_id = $1",
                    &[&shard_id],
                )?;
            }
            tx.commit()?;
            Ok(n == 1)
        })
    }

    /// Store the state of the shard `machine_id` holds under `epoch`. The
    /// placement row stays share-locked until the state is written, so a
    /// takeover waits for this save and every later save by the old owner
    /// is refused: a cut-off machine never overwrites the state of the copy
    /// that replaced it. False when it does not hold the shard.
    pub fn save_state(
        &self,
        shard_id: &str,
        machine_id: &str,
        epoch: i64,
        state: &[u8],
    ) -> Result<bool, String> {
        self.pool.with_client(|c| {
            let mut tx = c.transaction()?;
            let owned = holds(&mut tx, shard_id, machine_id, epoch)?;
            if owned {
                put_state(&mut tx, shard_id, state)?;
            }
            tx.commit()?;
            Ok(owned)
        })
    }

    /// The saved state of the shard `machine_id` holds under `epoch`:
    /// `Ok(None)` when it does not hold it (an operator stopped it, or
    /// another machine took it), `Ok(Some(None))` when there is no state.
    pub fn load_owned_state(
        &self,
        shard_id: &str,
        machine_id: &str,
        epoch: i64,
    ) -> Result<Option<Option<Vec<u8>>>, String> {
        self.pool.with_client(|c| {
            let row = c.query_opt(
                "SELECT s.state FROM _pylon_shard_placements p
                 LEFT JOIN _pylon_shard_state s ON s.shard_id = p.shard_id
                 WHERE p.shard_id = $1 AND p.machine_id = $2 AND p.epoch = $3",
                &[&shard_id, &machine_id, &epoch],
            )?;
            Ok(row.map(|r| r.get(0)))
        })
    }

    /// Record that the source removed a player: the row, in status `out`, and
    /// the source's state without the player (`source_state`, when the shard
    /// saves state), in one transaction, while `machine`/`epoch` holds the
    /// source. False when it does not.
    pub fn begin_transfer(
        &self,
        t: &Transfer,
        machine: &str,
        epoch: i64,
        source_state: Option<&[u8]>,
    ) -> Result<bool, String> {
        self.pool.with_client_once(|c| {
            let mut tx = c.transaction()?;
            // A client that dies mid-transaction releases its locks soon.
            tx.batch_execute("SET LOCAL idle_in_transaction_session_timeout = '15s'")?;
            if !holds(&mut tx, &t.from_shard, machine, epoch)? {
                return Ok(false);
            }
            if let Some(state) = source_state {
                put_state(&mut tx, &t.from_shard, state)?;
            }
            tx.execute(
                "INSERT INTO _pylon_shard_transfers
                    (transfer_id, subscriber, from_shard, to_shard, state, auth, status, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6, 'out',
                         (extract(epoch from clock_timestamp()) * 1000)::bigint)",
                &[
                    &t.id,
                    &t.subscriber,
                    &t.from_shard,
                    &t.to_shard,
                    &t.state,
                    &t.auth,
                ],
            )?;
            tx.commit()?;
            Ok(true)
        })
    }

    /// The target took the player: `out` becomes `in`, with the target's
    /// state (now holding the player) saved in the same transaction, while
    /// `machine`/`epoch` holds the target. False when the row is no longer
    /// `out` (the source took it back) or the target is not held.
    pub fn accept_transfer(
        &self,
        id: &str,
        to_shard: &str,
        machine: &str,
        epoch: i64,
        target_state: Option<&[u8]>,
    ) -> Result<Settle, String> {
        self.settle(id, "in", to_shard, machine, epoch, target_state)
    }

    /// The source took the player back: `out` becomes `back`, with the
    /// source's state saved in the same transaction. False when the row is
    /// no longer `out` (the target took it) or the source is not held.
    pub fn return_transfer(
        &self,
        id: &str,
        from_shard: &str,
        machine: &str,
        epoch: i64,
        source_state: Option<&[u8]>,
    ) -> Result<Settle, String> {
        self.settle(id, "back", from_shard, machine, epoch, source_state)
    }

    fn settle(
        &self,
        id: &str,
        status: &str,
        shard: &str,
        machine: &str,
        epoch: i64,
        state: Option<&[u8]>,
    ) -> Result<Settle, String> {
        self.pool.with_client_once(|c| {
            let mut tx = c.transaction()?;
            tx.batch_execute("SET LOCAL idle_in_transaction_session_timeout = '15s'")?;
            if !holds(&mut tx, shard, machine, epoch)? {
                return Ok(Settle::NotHeld);
            }
            let n = tx.execute(
                "UPDATE _pylon_shard_transfers SET status = $2
                 WHERE transfer_id = $1 AND status = 'out'",
                &[&id, &status],
            )?;
            if n != 1 {
                return Ok(Settle::Taken);
            }
            if let Some(state) = state {
                put_state(&mut tx, shard, state)?;
            }
            tx.commit()?;
            Ok(Settle::Done)
        })
    }

    /// The pool, for tests that write rows directly.
    #[cfg(test)]
    pub(crate) fn pool_for_tests(&self) -> &PgPool {
        &self.pool
    }

    /// Every transfer row of `subscriber`, oldest first.
    pub fn transfers_of(&self, subscriber: &str) -> Result<Vec<Transfer>, String> {
        self.pool.with_client(|c| {
            let rows = c.query(
                "SELECT transfer_id, subscriber, from_shard, to_shard, state, auth, status
                 FROM _pylon_shard_transfers WHERE subscriber = $1 AND status <> 'void'
                 ORDER BY created_at",
                &[&subscriber],
            )?;
            Ok(rows.iter().map(transfer_of).collect())
        })
    }

    /// Transfers still `out` from `shard`, of any age: a shard just started
    /// from saved state finishes them first.
    pub fn open_transfers_from(&self, shard: &str) -> Result<Vec<Transfer>, String> {
        self.pool.with_client(|c| {
            let rows = c.query(
                "SELECT transfer_id, subscriber, from_shard, to_shard, state, auth, status
                 FROM _pylon_shard_transfers WHERE from_shard = $1 AND status = 'out'
                 ORDER BY created_at",
                &[&shard],
            )?;
            Ok(rows.iter().map(transfer_of).collect())
        })
    }

    /// Moves out of `shard` that ended in the target within the last
    /// `within_ms`, the latest per subscriber, with each one's age in ms. A
    /// move the subscriber came back from (a later move into `shard` that
    /// ended there) is left out.
    pub fn recent_moves_from(
        &self,
        shard: &str,
        within_ms: i64,
    ) -> Result<Vec<(Transfer, i64)>, String> {
        self.pool.with_client(|c| {
            let rows = c.query(
                "SELECT DISTINCT ON (subscriber)
                        transfer_id, subscriber, from_shard, to_shard, state, auth, status,
                        (extract(epoch from clock_timestamp()) * 1000)::bigint - created_at
                 FROM _pylon_shard_transfers t
                 WHERE from_shard = $1 AND status = 'in'
                   AND created_at > (extract(epoch from clock_timestamp()) * 1000)::bigint - $2
                   AND NOT EXISTS (
                     SELECT 1 FROM _pylon_shard_transfers back
                     WHERE back.subscriber = t.subscriber AND back.to_shard = $1
                       AND back.status = 'in' AND back.created_at > t.created_at)
                 ORDER BY subscriber, created_at DESC",
                &[&shard, &within_ms],
            )?;
            Ok(rows.iter().map(|r| (transfer_of(r), r.get(7))).collect())
        })
    }

    /// The status of transfer `id` once no transaction on it is still in
    /// flight: `None` when it has no row (and never will: a `void` row takes
    /// the id, so a late insert fails). Waits for an uncommitted insert or
    /// update of the row, so a commit whose reply was lost is counted.
    pub fn confirm_status(&self, id: &str) -> Result<Option<String>, String> {
        self.pool.with_client_once(|c| {
            let mut tx = c.transaction()?;
            tx.batch_execute(
                "SET LOCAL lock_timeout = '5s';
                 SET LOCAL idle_in_transaction_session_timeout = '15s'",
            )?;
            // Blocks behind an uncommitted insert of the same id.
            tx.execute(
                "INSERT INTO _pylon_shard_transfers
                    (transfer_id, subscriber, from_shard, to_shard, state, auth, status, created_at)
                 VALUES ($1, '', '', '', ''::bytea, '{}'::jsonb, 'void',
                         (extract(epoch from clock_timestamp()) * 1000)::bigint)
                 ON CONFLICT (transfer_id) DO NOTHING",
                &[&id],
            )?;
            // Blocks behind an uncommitted update of it.
            let status: String = tx
                .query_one(
                    "SELECT status FROM _pylon_shard_transfers WHERE transfer_id = $1 FOR UPDATE",
                    &[&id],
                )?
                .get(0);
            tx.commit()?;
            Ok((status != "void").then_some(status))
        })
    }

    /// Transfers still `out` for longer than `older_than_ms` whose source
    /// shard has no placement (it was stopped) and whose target `machine`
    /// holds under `epoch`: the target's machine finishes them.
    pub fn ownerless_transfers(
        &self,
        machine: &str,
        epoch: i64,
        older_than_ms: i64,
    ) -> Result<Vec<(Transfer, i64)>, String> {
        self.pool.with_client(|c| {
            let rows = c.query(
                "SELECT t.transfer_id, t.subscriber, t.from_shard, t.to_shard, t.state, t.auth, t.status,
                        (extract(epoch from clock_timestamp()) * 1000)::bigint - t.created_at
                 FROM _pylon_shard_transfers t
                 JOIN _pylon_shard_placements p ON p.shard_id = t.to_shard
                 WHERE t.status = 'out' AND p.machine_id = $1 AND p.epoch = $2
                   AND NOT EXISTS (SELECT 1 FROM _pylon_shard_placements s
                                   WHERE s.shard_id = t.from_shard)
                   AND t.created_at < (extract(epoch from clock_timestamp()) * 1000)::bigint - $3
                 ORDER BY t.created_at",
                &[&machine, &epoch, &older_than_ms],
            )?;
            Ok(rows.iter().map(|r| (transfer_of(r), r.get(7))).collect())
        })
    }

    pub fn transfer(&self, id: &str) -> Result<Option<Transfer>, String> {
        self.pool.with_client(|c| {
            let row = c.query_opt(
                "SELECT transfer_id, subscriber, from_shard, to_shard, state, auth, status
                 FROM _pylon_shard_transfers WHERE transfer_id = $1 AND status <> 'void'",
                &[&id],
            )?;
            Ok(row.map(|r| transfer_of(&r)))
        })
    }

    /// Transfers still `out` for longer than `older_than_ms` whose source
    /// `machine` holds under `epoch`: the source must finish them.
    pub fn stale_transfers(
        &self,
        machine: &str,
        epoch: i64,
        older_than_ms: i64,
    ) -> Result<Vec<Transfer>, String> {
        self.pool.with_client(|c| {
            let rows = c.query(
                "SELECT t.transfer_id, t.subscriber, t.from_shard, t.to_shard, t.state, t.auth, t.status
                 FROM _pylon_shard_transfers t
                 JOIN _pylon_shard_placements p ON p.shard_id = t.from_shard
                 WHERE t.status = 'out' AND p.machine_id = $1 AND p.epoch = $2
                   AND t.created_at < (extract(epoch from clock_timestamp()) * 1000)::bigint - $3
                 ORDER BY t.created_at",
                &[&machine, &epoch, &older_than_ms],
            )?;
            Ok(rows.iter().map(transfer_of).collect())
        })
    }

    /// Drop settled transfers older than an hour, and abandoned ones older
    /// than a week (kept that long for an operator to recover by hand).
    pub fn prune_transfers(&self) -> Result<(), String> {
        self.pool.with_client(|c| {
            c.execute(
                "DELETE FROM _pylon_shard_transfers
                 WHERE (status IN ('in', 'back', 'void')
                        AND created_at < (extract(epoch from clock_timestamp()) * 1000)::bigint - 3600000)
                    OR (status = 'dropped'
                        AND created_at < (extract(epoch from clock_timestamp()) * 1000)::bigint - 604800000)",
                &[],
            )?;
            Ok(())
        })
    }

    /// Record that the target refused transfer `id` (its source stopped),
    /// and return how long it has refused, in ms, since the first time.
    pub fn note_refusal(&self, id: &str) -> Result<i64, String> {
        self.pool.with_client(|c| {
            let row = c.query_opt(
                "UPDATE _pylon_shard_transfers
                 SET refused_at = COALESCE(refused_at, (extract(epoch from clock_timestamp()) * 1000)::bigint)
                 WHERE transfer_id = $1
                 RETURNING (extract(epoch from clock_timestamp()) * 1000)::bigint - refused_at",
                &[&id],
            )?;
            Ok(row.map(|r| r.get(0)).unwrap_or(0))
        })
    }

    /// Mark `dropped` the transfers `out` for longer than `older_than_ms`
    /// whose source and target both have no placement: nothing will take
    /// them. The rows keep the players' states. Returns them.
    pub fn drop_stranded_rows(&self, older_than_ms: i64) -> Result<Vec<Transfer>, String> {
        self.pool.with_client(|c| {
            let rows = c.query(
                "UPDATE _pylon_shard_transfers t SET status = 'dropped'
                 WHERE t.status = 'out'
                   AND t.created_at < (extract(epoch from clock_timestamp()) * 1000)::bigint - $1
                   AND NOT EXISTS (SELECT 1 FROM _pylon_shard_placements p
                                   WHERE p.shard_id IN (t.from_shard, t.to_shard))
                 RETURNING t.transfer_id, t.subscriber, t.from_shard, t.to_shard, t.state, t.auth, t.status",
                &[&older_than_ms],
            )?;
            Ok(rows.iter().map(transfer_of).collect())
        })
    }

    /// Give up on transfer `id`: its source was stopped and its target
    /// keeps refusing the player. `out` becomes `dropped`; the row keeps the
    /// player's state. False when it was no longer `out`.
    pub fn abandon_transfer(&self, id: &str) -> Result<bool, String> {
        self.pool.with_client_once(|c| {
            let n = c.execute(
                "UPDATE _pylon_shard_transfers SET status = 'dropped'
                 WHERE transfer_id = $1 AND status = 'out'",
                &[&id],
            )?;
            Ok(n == 1)
        })
    }

    /// Remove this machine's row under `epoch`: it is shutting down, and
    /// its shards should move now rather than after [`DEAD_AFTER`]. A row a
    /// newer process wrote under the same id stays.
    pub fn leave(&self, machine_id: &str, epoch: i64) -> Result<(), String> {
        self.pool.with_client(|c| {
            c.execute(
                "DELETE FROM _pylon_shard_machines WHERE machine_id = $1 AND epoch = $2",
                &[&machine_id, &epoch],
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
        epoch: r.get(6),
    }
}

fn transfer_of(r: &postgres::Row) -> Transfer {
    Transfer {
        id: r.get(0),
        subscriber: r.get(1),
        from_shard: r.get(2),
        to_shard: r.get(3),
        state: r.get(4),
        auth: r.get(5),
        status: r.get(6),
    }
}

/// True when `machine` holds `shard` under `epoch`. The placement row stays
/// share-locked until the transaction ends, as in `save_state`.
fn holds(
    tx: &mut postgres::Transaction<'_>,
    shard: &str,
    machine: &str,
    epoch: i64,
) -> Result<bool, postgres::Error> {
    Ok(tx
        .query_opt(
            "SELECT 1 FROM _pylon_shard_placements
             WHERE shard_id = $1 AND machine_id = $2 AND epoch = $3
             FOR SHARE",
            &[&shard, &machine, &epoch],
        )?
        .is_some())
}

fn put_state(
    tx: &mut postgres::Transaction<'_>,
    shard: &str,
    state: &[u8],
) -> Result<(), postgres::Error> {
    tx.execute(
        "INSERT INTO _pylon_shard_state (shard_id, state, saved_at)
         VALUES ($1, $2, (extract(epoch from clock_timestamp()) * 1000)::bigint)
         ON CONFLICT (shard_id) DO UPDATE SET
            state = EXCLUDED.state, saved_at = EXCLUDED.saved_at",
        &[&shard, &state],
    )?;
    Ok(())
}

/// SQL that is true when machine `machine` is live under `epoch`, with the
/// liveness window in milliseconds at parameter `window`.
fn owner_live_sql(machine: &str, epoch: &str, window: &str) -> String {
    format!(
        "EXISTS (SELECT 1 FROM _pylon_shard_machines m
                 WHERE m.machine_id = {machine} AND m.epoch = {epoch}
                   AND m.heartbeat_at > (extract(epoch from clock_timestamp()) * 1000)::bigint - {window})"
    )
}

/// A new lease epoch: random, so a restarted machine never reuses one.
pub fn new_epoch() -> i64 {
    rand::random::<i64>()
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
    /// Move `subscriber` from shard `from` (on the receiver) to shard `to`.
    Transfer {
        from: String,
        subscriber: String,
        to: String,
        /// Claims for the target's ticket.
        #[serde(default)]
        claims: serde_json::Value,
    },
    /// Take the player in transfer `id` into its target shard (on the
    /// receiver). The state is in the transfer row.
    TransferIn {
        id: String,
    },
    /// Take shard `id` from machine `from`, which is shutting down, holds it
    /// under `epoch`, and saved its final state: move the placement to the
    /// receiver and start it there.
    HandOver {
        id: String,
        from: String,
        epoch: i64,
    },
    /// Queue a server input (`ctx.shards.send`) for shard `id` on the
    /// receiver.
    Input {
        id: String,
        input: serde_json::Value,
    },
    /// Deliver a message to the shards on the receiver that `to` names.
    Deliver {
        from: String,
        to: String,
        topic: String,
        data_b64: String,
    },
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
/// Nonces of accepted signatures, oldest first, so expiry pops from the
/// front instead of scanning every entry on each call.
#[derive(Default)]
struct Seen {
    order: std::collections::VecDeque<(i64, String)>,
    set: std::collections::HashSet<String>,
}

fn seen() -> &'static Mutex<Seen> {
    static SEEN: OnceLock<Mutex<Seen>> = OnceLock::new();
    SEEN.get_or_init(|| Mutex::new(Seen::default()))
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
    // Keyed by the nonce: the MAC covers it, and `verify_with` accepts only
    // the canonical form of the rest, so a signature has one spelling.
    let (_, rest) = header.split_once('.').ok_or("malformed signature")?;
    let (nonce, _) = rest.split_once('.').ok_or("malformed signature")?;
    let mut seen = seen().lock().unwrap();
    // A signature is valid for CALL_WINDOW_MS either side of now; its nonce
    // is kept twice that long.
    while seen
        .order
        .front()
        .is_some_and(|(at, _)| now.saturating_sub(*at) > CALL_WINDOW_MS * 2)
    {
        let (_, old) = seen.order.pop_front().expect("checked");
        seen.set.remove(&old);
    }
    if !seen.set.insert(nonce.to_string()) {
        return Err("signature already used");
    }
    seen.order.push_back((now, nonce.to_string()));
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
    let at_text = at;
    let at: i64 = at.parse().map_err(|_| "malformed signature")?;
    // One spelling per time ("0012" and "+12" also parse as 12).
    if at.to_string() != at_text || nonce.is_empty() {
        return Err("malformed signature");
    }
    if now.abs_diff(at) > CALL_WINDOW_MS as u64 {
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

/// The HTTP agent machine-to-machine calls use: 3 s to connect, 15 s in
/// all, no redirects. One agent keeps its connections for reuse.
pub fn call_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(3))
        .timeout(Duration::from_secs(15))
        .redirects(0)
        .build()
}

/// Run `op` on machine `to` at `address`.
pub fn call(to: &str, address: &str, op: &RemoteOp) -> Result<RemoteReply, String> {
    call_with(&call_agent(), to, address, op)
}

/// [`call`] with a given agent (see [`call_agent`]).
pub fn call_with(
    agent: &ureq::Agent,
    to: &str,
    address: &str,
    op: &RemoteOp,
) -> Result<RemoteReply, String> {
    let body = serde_json::to_vec(op).map_err(|e| e.to_string())?;
    let signature = sign(to, &body).ok_or("this machine is not in the shard directory")?;
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
pub(crate) mod tests {
    use super::*;
    use std::collections::HashMap;

    fn m(id: &str, capacity: u32, load: u32) -> Machine {
        Machine {
            id: id.into(),
            address: None,
            fly_instance: None,
            capacity,
            load,
            epoch: 0,
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
        // Another spelling of the same time is refused, so a captured
        // signature cannot be replayed under a new cache key.
        for respelled in [format!("0{header}"), format!("+{header}")] {
            assert_eq!(
                verify_with(key, "b", &respelled, body, at),
                Err("malformed signature")
            );
        }
        // Extreme times are refused without overflow.
        for at in [i64::MIN, i64::MAX] {
            let extreme = sign_with(key, at, "n1", "b", body);
            assert_eq!(
                verify_with(key, "b", &extreme, body, 1_000_000),
                Err("signature expired")
            );
            assert_eq!(
                verify_with(key, "b", &extreme, body, -1_000_000),
                Err("signature expired")
            );
        }
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

    /// Held by every test that uses the directory in Postgres: they share
    /// its tables, and one test's live machines change where another's
    /// shards are placed.
    pub(crate) static DIRECTORY_TESTS: Mutex<()> = Mutex::new(());

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
            epoch: 1,
        }
    }

    /// A -> B, then B -> A: A's restart must not send the subscriber to B.
    #[test]
    fn a_move_the_subscriber_came_back_from_is_not_recent() {
        let _serial = DIRECTORY_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let Some(dir) = test_dir() else { return };
        let run = pylon_cluster::new_instance_id();
        let (a, b, sid) = (format!("a-{run}"), format!("b-{run}"), format!("p-{run}"));
        let row = |id: &str, from: &str, to: &str, ago_ms: i64| {
            dir.pool_for_tests()
                .with_client(|c| {
                    c.execute(
                        "INSERT INTO _pylon_shard_transfers
                            (transfer_id, subscriber, from_shard, to_shard, state, auth, status, created_at)
                         VALUES ($1, $2, $3, $4, ''::bytea, '{}'::jsonb, 'in',
                                 (extract(epoch from clock_timestamp()) * 1000)::bigint - $5)",
                        &[&id, &sid, &from, &to, &ago_ms],
                    )?;
                    Ok::<(), postgres::Error>(())
                })
                .unwrap();
        };
        row(&format!("t1-{run}"), &a, &b, 60_000);
        let moves = dir.recent_moves_from(&a, 600_000).unwrap();
        assert_eq!(moves.len(), 1, "A -> B is recent");
        row(&format!("t2-{run}"), &b, &a, 30_000);
        assert!(dir.recent_moves_from(&a, 600_000).unwrap().is_empty());
        // Out again later: that move is the recent one.
        row(&format!("t3-{run}"), &a, &b, 10_000);
        let moves = dir.recent_moves_from(&a, 600_000).unwrap();
        assert_eq!(
            moves.iter().map(|(t, _)| t.id.clone()).collect::<Vec<_>>(),
            [format!("t3-{run}")]
        );
    }

    #[test]
    fn the_directory_claims_moves_fences_fails_and_forgets() {
        let _serial = DIRECTORY_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let Some(dir) = test_dir() else { return };
        // The key is in the directory now, the same for every machine.
        assert!(sign("x", b"y").is_some());
        let run = pylon_cluster::new_instance_id();
        let (a, b) = (format!("a-{run}"), format!("b-{run}"));
        let shard = format!("s-{run}");
        // A kind of its own: the cluster-wide limit counts only this run.
        let kind = format!("arena-{run}");
        assert!(dir.heartbeat(&me(&a), 7).unwrap());
        assert!(dir.heartbeat(&me(&b), 1).unwrap());
        let orphan = |dir: &PgShardDirectory| {
            dir.orphans()
                .unwrap()
                .into_iter()
                .find(|o| o.shard_id == shard)
        };

        let p = placement(&shard, &kind, &b);
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
        assert_eq!(live.iter().find(|m| m.id == b).unwrap().epoch, 1);

        // Only the owner, under its epoch, saves state.
        assert!(dir.save_state(&shard, &b, 1, b"state-1").unwrap());
        assert!(!dir.save_state(&shard, &a, 1, b"stale").unwrap());
        assert!(!dir.save_state(&shard, &b, 2, b"stale").unwrap());
        assert_eq!(
            dir.load_owned_state(&shard, &b, 1).unwrap(),
            Some(Some(b"state-1".to_vec()))
        );
        assert_eq!(dir.load_owned_state(&shard, &a, 1).unwrap(), None);

        // B is live under the placement's epoch: not an orphan, and no
        // machine can take it.
        assert_eq!(orphan(&dir), None);
        assert!(!dir.take_over(&p, &a, 7).unwrap());

        // A second process with B's id cannot take the id while B is live.
        assert!(!dir.heartbeat(&me(&b), 2).unwrap());
        assert_eq!(orphan(&dir), None);
        // B left (or went silent): the new process takes the id under its
        // own epoch, and B's placement is an orphan.
        dir.leave(&b, 2).unwrap();
        assert!(
            dir.live_machines().unwrap().iter().any(|m| m.id == b),
            "a leave under another epoch removes nothing"
        );
        dir.leave(&b, 1).unwrap();
        assert!(dir.heartbeat(&me(&b), 2).unwrap());
        let o = orphan(&dir).expect("an orphan after the epoch changed");
        assert_eq!((o.machine_id.as_str(), o.epoch), (b.as_str(), 1));
        assert!(dir.take_over(&o, &a, 7).unwrap());
        assert!(!dir.take_over(&o, &b, 2).unwrap(), "only one wins");
        assert_eq!(orphan(&dir), None);
        assert!(!dir
            .save_state(&shard, &b, 1, b"from the cut-off machine")
            .unwrap());
        assert!(dir.save_state(&shard, &a, 7, b"state-2").unwrap());

        // A cannot start it: the placement says why, and the state stays.
        dir.mark_failed(&shard, &a, 7, "restore refused").unwrap();
        let failed = dir.placement(&shard).unwrap().unwrap();
        assert_eq!(failed.failed.as_deref(), Some("restore refused"));
        assert_eq!(
            dir.load_owned_state(&shard, &a, 7).unwrap(),
            Some(Some(b"state-2".to_vec()))
        );
        assert_eq!(dir.placements_on(&a).unwrap().len(), 1);

        // A release by the old owner or an old epoch changes nothing; the
        // holder's removes the placement and its state.
        assert!(!dir.release(&shard, &b, 1).unwrap());
        assert!(!dir.release(&shard, &a, 1).unwrap());
        assert!(dir.placement(&shard).unwrap().is_some());
        assert!(dir.release(&shard, &a, 7).unwrap());
        assert_eq!(dir.placement(&shard).unwrap(), None);
        assert_eq!(dir.load_owned_state(&shard, &a, 7).unwrap(), None);

        dir.leave(&a, 7).unwrap();
        dir.leave(&b, 2).unwrap();
        assert!(!dir.live_machines().unwrap().iter().any(|m| m.id == a));
    }

    #[test]
    fn a_takeover_waits_for_a_save_in_progress() {
        let _serial = DIRECTORY_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let Some(dir) = test_dir() else { return };
        let dir = Arc::new(dir);
        let run = pylon_cluster::new_instance_id();
        let (a, b) = (format!("a-{run}"), format!("b-{run}"));
        let shard = format!("s-{run}");
        assert!(dir.heartbeat(&me(&a), 1).unwrap());
        // A kind of its own: the cluster-wide limit counts only this run.
        let p = placement(&shard, &format!("arena-{run}"), &b);
        assert_eq!(dir.claim(&p, 10).unwrap(), Claim::Claimed);
        // B never heartbeat: its placement is an orphan.

        // B's save holds the row lock, as save_state does, until released.
        let (locked_tx, locked_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let saver = {
            let (dir, shard, b) = (Arc::clone(&dir), shard.clone(), b.clone());
            std::thread::spawn(move || {
                dir.pool
                    .with_client_once(|c| {
                        let mut tx = c.transaction()?;
                        tx.query_one(
                            "SELECT 1 FROM _pylon_shard_placements
                             WHERE shard_id = $1 AND machine_id = $2 AND epoch = 1
                             FOR SHARE",
                            &[&shard, &b],
                        )?;
                        locked_tx.send(()).unwrap();
                        release_rx.recv().unwrap();
                        tx.execute(
                            "INSERT INTO _pylon_shard_state (shard_id, state, saved_at)
                             VALUES ($1, 'last', 0)",
                            &[&shard],
                        )?;
                        tx.commit()
                    })
                    .unwrap();
            })
        };
        locked_rx.recv().unwrap();
        let taker = {
            let (dir, p, a) = (Arc::clone(&dir), p.clone(), a.clone());
            std::thread::spawn(move || dir.take_over(&p, &a, 1).unwrap())
        };
        std::thread::sleep(Duration::from_millis(300));
        assert!(!taker.is_finished(), "the takeover waits for the save");
        release_tx.send(()).unwrap();
        saver.join().unwrap();
        assert!(taker.join().unwrap());
        // The new owner reads the last save; the old owner saves no more.
        assert_eq!(
            dir.load_owned_state(&shard, &a, 1).unwrap(),
            Some(Some(b"last".to_vec()))
        );
        assert!(!dir.save_state(&shard, &b, 1, b"late").unwrap());
        assert!(dir.release(&shard, &a, 1).unwrap());
        dir.leave(&a, 1).unwrap();
    }

    #[test]
    fn concurrent_claims_never_pass_the_kind_limit() {
        let _serial = DIRECTORY_TESTS.lock().unwrap_or_else(|e| e.into_inner());
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
            dir.release(&format!("s{i}-{run}"), &format!("m{i}"), 1)
                .unwrap();
        }
    }
}
