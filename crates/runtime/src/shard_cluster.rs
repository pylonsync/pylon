//! Shards across machines: a directory in the database, placement, and the
//! calls machines make to each other.
//!
//! An app on Postgres can run on several machines. An app on SQLite runs on
//! one: its directory is in the app's file, and one process holds the file's
//! directory lock (see [`ShardDirectory::open_sqlite`]). The same directory
//! logic saves shard state, starts shards again after a restart, and keeps
//! transfers between shards consistent on both.
//!
//! Each machine registers
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

mod db;

use db::{Db, DbError, Dialect, Retry, Row, SqliteDb, Tx, Val};

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

/// A player in flight between shards (see [`ShardDirectory::begin_transfer`]).
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

/// What [`ShardDirectory::claim`] found.
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

/// How often a machine renews its lease, how long a lease runs after the
/// heartbeat that renewed it was sent, and how long a silent machine counts
/// as live. `fence_after` is shorter than `dead_after`: a cut-off machine
/// stops its shards before another may start them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timing {
    pub heartbeat: Duration,
    pub fence_after: Duration,
    pub dead_after: Duration,
}

/// Postgres: several machines, so a machine that cannot reach the database
/// must stop its shards soon.
pub const PG_TIMING: Timing = Timing {
    heartbeat: HEARTBEAT,
    fence_after: FENCE_AFTER,
    dead_after: DEAD_AFTER,
};

/// SQLite: one process holds the file's directory lock, so no other
/// process can take its shards. The lease only has to outlast a heartbeat
/// that waits for the database's write lock (a long mutation holds it).
pub const SQLITE_TIMING: Timing = Timing {
    heartbeat: HEARTBEAT,
    fence_after: Duration::from_secs(30),
    dead_after: Duration::from_secs(60),
};

/// The shard directory: in Postgres (several machines), or in the app's
/// SQLite file (one machine).
pub struct ShardDirectory {
    db: Db,
    timing: Timing,
}

impl ShardDirectory {
    /// The directory in Postgres: create the tables, and load the cluster
    /// key into this process.
    pub fn open_pg(pool: Arc<PgPool>) -> Result<Self, String> {
        create_pg_schema(&pool)?;
        let dir = Self {
            db: Db::Pg(pool),
            timing: PG_TIMING,
        };
        dir.load_key()?;
        Ok(dir)
    }

    /// The directory in the SQLite file at `path`, for the one process that
    /// holds the file's directory lock: create the tables, forget every
    /// machine row (each is from a process that has exited, so its shards
    /// start here from their saved state), and load the cluster key. An
    /// error when another process still holds the lock after a few seconds.
    pub fn open_sqlite(path: &str) -> Result<Self, String> {
        Self::open_sqlite_waiting(path, db::SQLITE_LOCK_WAIT)
    }

    /// [`Self::open_sqlite`], waiting `wait` for the directory lock.
    pub(crate) fn open_sqlite_waiting(path: &str, wait: Duration) -> Result<Self, String> {
        let dir = Self {
            db: Db::Sqlite(Box::new(SqliteDb::open_waiting(path, wait)?)),
            timing: SQLITE_TIMING,
        };
        dir.db.tx(Retry::Replay, |tx| {
            for statement in SQLITE_SCHEMA {
                tx.execute(statement, &[])?;
            }
            tx.execute("DELETE FROM _pylon_shard_machines", &[])?;
            Ok(())
        })?;
        dir.load_key()?;
        Ok(dir)
    }

    /// The lease and liveness times for this directory.
    pub fn timing(&self) -> Timing {
        self.timing
    }

    fn d(&self) -> Dialect {
        self.db.dialect()
    }

    fn window(&self) -> i64 {
        self.timing.dead_after.as_millis() as i64
    }

    fn load_key(&self) -> Result<(), String> {
        let [p1] = self.d().params();
        let fresh: Vec<u8> = (0..32).map(|_| rand::random::<u8>()).collect();
        let key = self.db.tx(Retry::Replay, |tx| {
            tx.execute(
                &format!(
                    "INSERT INTO _pylon_shard_cluster (id, call_key) VALUES (1, {p1})
                     ON CONFLICT (id) DO NOTHING"
                ),
                &[Val::Bytes(&fresh)],
            )?;
            Ok(tx
                .query_one(
                    "SELECT call_key FROM _pylon_shard_cluster WHERE id = 1",
                    &[],
                )?
                .bytes(0))
        })?;
        let _ = CALL_KEY.set(key);
        Ok(())
    }

    /// Renew this machine's row under lease `epoch`. Times come from the
    /// database clock, so machines with skewed clocks agree on who is alive.
    ///
    /// False when another process holds the id under another epoch and is
    /// still live: a second process with the same id waits until the first
    /// leaves or goes silent for the dead time, so it never takes shards
    /// the first is still running.
    pub fn heartbeat(&self, me: &MachineConfig, epoch: i64) -> Result<bool, String> {
        let d = self.d();
        let [p1, p2, p3, p4, p5, p6] = d.params();
        let now = d.now_ms();
        let sql = format!(
            "INSERT INTO _pylon_shard_machines AS m
                (machine_id, address, fly_instance, capacity, heartbeat_at, epoch)
             VALUES ({p1}, {p2}, {p3}, {p4}, {now}, {p5})
             ON CONFLICT (machine_id) DO UPDATE SET
                address = EXCLUDED.address, fly_instance = EXCLUDED.fly_instance,
                capacity = EXCLUDED.capacity, heartbeat_at = EXCLUDED.heartbeat_at,
                epoch = EXCLUDED.epoch
             WHERE m.epoch = EXCLUDED.epoch OR m.heartbeat_at <= {now} - {p6}"
        );
        let window = self.window();
        self.db.run(true, |tx| {
            Ok(tx.execute(
                &sql,
                &[
                    Val::Text(&me.id),
                    Val::OptText(me.address.as_deref()),
                    Val::OptText(me.fly_instance.as_deref()),
                    Val::I32(me.capacity as i32),
                    Val::I64(epoch),
                    Val::I64(window),
                ],
            )? == 1)
        })
    }

    /// Machines with a heartbeat within the dead time, with the number of
    /// shards placed on each.
    pub fn live_machines(&self) -> Result<Vec<Machine>, String> {
        let d = self.d();
        let [p1] = d.params();
        let now = d.now_ms();
        let sql = format!(
            "SELECT m.machine_id, m.address, m.fly_instance, m.capacity,
                    (SELECT count(*) FROM _pylon_shard_placements p
                     WHERE p.machine_id = m.machine_id),
                    m.epoch
             FROM _pylon_shard_machines m
             WHERE m.heartbeat_at > {now} - {p1}
             ORDER BY m.machine_id"
        );
        let window = self.window();
        self.db.run(false, |tx| {
            Ok(tx
                .query(&sql, &[Val::I64(window)])?
                .iter()
                .map(|r| Machine {
                    id: r.string(0),
                    address: r.opt_string(1),
                    fly_instance: r.opt_string(2),
                    capacity: r.i64(3).max(0) as u32,
                    load: r.i64(4).clamp(0, u32::MAX as i64) as u32,
                    epoch: r.i64(5),
                })
                .collect())
        })
    }

    /// Record that `p.machine_id` runs the shard, unless the id is placed
    /// already (`Taken`, whatever the kind's count) or its kind has `max`
    /// shards across the cluster. The count and the insert run under a lock
    /// on the kind (on SQLite, the database's write lock), so two machines
    /// cannot both take the last slot.
    pub fn claim(&self, p: &Placement, max: usize) -> Result<Claim, String> {
        let max = max.min(i64::MAX as usize) as i64;
        let d = self.d();
        let [p1, p2, p3, p4, p5, p6] = d.params();
        let now = d.now_ms();
        let insert = format!(
            "INSERT INTO _pylon_shard_placements
                (shard_id, kind, params, machine_id, pinned, epoch, created_at)
             VALUES ({p1}, {p2}, {p3}, {p4}, {p5}, {p6}, {now})
             ON CONFLICT (shard_id) DO NOTHING"
        );
        let placed = format!("SELECT 1 FROM _pylon_shard_placements WHERE shard_id = {p1}");
        let count = format!("SELECT count(*) FROM _pylon_shard_placements WHERE kind = {p1}");
        self.db.tx(Retry::Once, |tx| {
            if tx.dialect() == Dialect::Pg {
                tx.execute(
                    "SELECT pg_advisory_xact_lock($1, hashtext($2))",
                    &[Val::I32(SCHEMA_LOCK_ID as i32), Val::Text(&p.kind)],
                )?;
            }
            // The same id from two first callers: the second finds it placed,
            // not the kind full.
            if tx.query_opt(&placed, &[Val::Text(&p.shard_id)])?.is_some() {
                return Ok(Claim::Taken);
            }
            if tx.query_one(&count, &[Val::Text(&p.kind)])?.i64(0) >= max {
                return Ok(Claim::LimitReached);
            }
            let n = tx.execute(
                &insert,
                &[
                    Val::Text(&p.shard_id),
                    Val::Text(&p.kind),
                    Val::Json(&p.params),
                    Val::Text(&p.machine_id),
                    Val::Bool(p.pinned),
                    Val::I64(p.epoch),
                ],
            )?;
            Ok(if n == 1 { Claim::Claimed } else { Claim::Taken })
        })
    }

    pub fn placement(&self, shard_id: &str) -> Result<Option<Placement>, String> {
        let [p1] = self.d().params();
        let sql = format!(
            "SELECT shard_id, kind, params, machine_id, pinned, failed, epoch
             FROM _pylon_shard_placements WHERE shard_id = {p1}"
        );
        self.db.run(false, |tx| {
            Ok(tx
                .query_opt(&sql, &[Val::Text(shard_id)])?
                .map(|r| placement_of(&r)))
        })
    }

    /// Every placement whose machine is not live under the placement's
    /// epoch: the machine died, left, restarted, or had its lease lapse.
    pub fn orphans(&self) -> Result<Vec<Placement>, String> {
        let d = self.d();
        let [p1] = d.params();
        let sql = format!(
            "SELECT p.shard_id, p.kind, p.params, p.machine_id, p.pinned, p.failed, p.epoch
             FROM _pylon_shard_placements p
             WHERE NOT {} ORDER BY p.shard_id",
            owner_live_sql(d, "p.machine_id", "p.epoch", &p1)
        );
        let window = self.window();
        self.db.run(false, |tx| {
            Ok(tx
                .query(&sql, &[Val::I64(window)])?
                .iter()
                .map(placement_of)
                .collect())
        })
    }

    /// Every placement on `machine_id`.
    pub fn placements_on(&self, machine_id: &str) -> Result<Vec<Placement>, String> {
        let [p1] = self.d().params();
        let sql = format!(
            "SELECT shard_id, kind, params, machine_id, pinned, failed, epoch
             FROM _pylon_shard_placements WHERE machine_id = {p1} ORDER BY shard_id"
        );
        self.db.run(false, |tx| {
            Ok(tx
                .query(&sql, &[Val::Text(machine_id)])?
                .iter()
                .map(placement_of)
                .collect())
        })
    }

    /// Move orphan `p` to machine `to` under lease `to_epoch`. False when
    /// another machine moved it first, it is gone, or its machine is live
    /// under its epoch again: the check and the move are one statement.
    pub fn take_over(&self, p: &Placement, to: &str, to_epoch: i64) -> Result<bool, String> {
        let d = self.d();
        let [p1, p2, p3, p4, p5, p6] = d.params();
        let now = d.now_ms();
        let sql = format!(
            "UPDATE _pylon_shard_placements AS p
             SET machine_id = {p4}, epoch = {p5}, moved_at = {now}
             WHERE p.shard_id = {p1} AND p.machine_id = {p2} AND p.epoch = {p3}
               AND NOT {}",
            owner_live_sql(d, "p.machine_id", "p.epoch", &p6)
        );
        let window = self.window();
        self.db.tx(Retry::Once, |tx| {
            Ok(tx.execute(
                &sql,
                &[
                    Val::Text(&p.shard_id),
                    Val::Text(&p.machine_id),
                    Val::I64(p.epoch),
                    Val::Text(to),
                    Val::I64(to_epoch),
                    Val::I64(window),
                ],
            )? == 1)
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
        let d = self.d();
        let [p1, p2, p3, p4, p5] = d.params();
        let now = d.now_ms();
        let sql = format!(
            "UPDATE _pylon_shard_placements
             SET machine_id = {p4}, epoch = {p5}, failed = NULL, moved_at = {now}
             WHERE shard_id = {p1} AND machine_id = {p2} AND epoch = {p3}"
        );
        self.db.tx(Retry::Once, |tx| {
            Ok(tx.execute(
                &sql,
                &[
                    Val::Text(shard_id),
                    Val::Text(from),
                    Val::I64(from_epoch),
                    Val::Text(to),
                    Val::I64(to_epoch),
                ],
            )? == 1)
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
        let [p1, p2, p3, p4] = self.d().params();
        let sql = format!(
            "UPDATE _pylon_shard_placements SET failed = {p4}
             WHERE shard_id = {p1} AND machine_id = {p2} AND epoch = {p3}"
        );
        self.db.run(true, |tx| {
            tx.execute(
                &sql,
                &[
                    Val::Text(shard_id),
                    Val::Text(machine_id),
                    Val::I64(epoch),
                    Val::Text(why),
                ],
            )?;
            Ok(())
        })
    }

    /// Forget the shard `machine_id` holds under `epoch`, with its saved
    /// state: it ended, or an operator stopped it. A placement another
    /// machine (or a later epoch) took over is left alone. False when
    /// nothing matched.
    pub fn release(&self, shard_id: &str, machine_id: &str, epoch: i64) -> Result<bool, String> {
        let [p1, p2, p3] = self.d().params();
        let sql = format!(
            "DELETE FROM _pylon_shard_placements
             WHERE shard_id = {p1} AND machine_id = {p2} AND epoch = {p3}"
        );
        self.db.tx(Retry::Replay, |tx| {
            let n = tx.execute(
                &sql,
                &[Val::Text(shard_id), Val::Text(machine_id), Val::I64(epoch)],
            )?;
            if n == 1 {
                delete_state(tx, shard_id)?;
            }
            Ok(n == 1)
        })
    }

    /// Forget the shard `machine_id` holds, under any epoch, with its saved
    /// state: for a shard that ended on that machine while no new run of it
    /// can start there (a take-over by the same machine under a newer lease
    /// is released too). A placement another machine took is left alone.
    pub fn release_here(&self, shard_id: &str, machine_id: &str) -> Result<bool, String> {
        let [p1, p2] = self.d().params();
        let sql = format!(
            "DELETE FROM _pylon_shard_placements WHERE shard_id = {p1} AND machine_id = {p2}"
        );
        self.db.tx(Retry::Replay, |tx| {
            let n = tx.execute(&sql, &[Val::Text(shard_id), Val::Text(machine_id)])?;
            if n == 1 {
                delete_state(tx, shard_id)?;
            }
            Ok(n == 1)
        })
    }

    /// Store the state of the shard `machine_id` holds under `epoch`. The
    /// placement row stays share-locked (on SQLite, the database's write
    /// lock is held) until the state is written, so a takeover waits for
    /// this save and every later save by the old owner is refused: a
    /// cut-off machine never overwrites the state of the copy that
    /// replaced it. False when it does not hold the shard.
    pub fn save_state(
        &self,
        shard_id: &str,
        machine_id: &str,
        epoch: i64,
        state: &[u8],
    ) -> Result<bool, String> {
        self.db.tx(Retry::Replay, |tx| {
            let owned = holds(tx, shard_id, machine_id, epoch)?;
            if owned {
                put_state(tx, shard_id, state)?;
            }
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
        let [p1, p2, p3] = self.d().params();
        let sql = format!(
            "SELECT s.state FROM _pylon_shard_placements p
             LEFT JOIN _pylon_shard_state s ON s.shard_id = p.shard_id
             WHERE p.shard_id = {p1} AND p.machine_id = {p2} AND p.epoch = {p3}"
        );
        self.db.run(false, |tx| {
            Ok(tx
                .query_opt(
                    &sql,
                    &[Val::Text(shard_id), Val::Text(machine_id), Val::I64(epoch)],
                )?
                .map(|r| r.opt_bytes(0)))
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
        let d = self.d();
        let [p1, p2, p3, p4, p5, p6] = d.params();
        let now = d.now_ms();
        let (seq_col, seq_val) = next_seq(d);
        let insert = format!(
            "INSERT INTO _pylon_shard_transfers
                (transfer_id, subscriber, from_shard, to_shard, state, auth, status,
                 created_at{seq_col})
             VALUES ({p1}, {p2}, {p3}, {p4}, {p5}, {p6}, 'out', {now}{seq_val})"
        );
        self.db.tx(Retry::Once, |tx| {
            // A client that dies mid-transaction releases its locks soon.
            tx.pg_only("SET LOCAL idle_in_transaction_session_timeout = '15s'")?;
            if !holds(tx, &t.from_shard, machine, epoch)? {
                return Ok(false);
            }
            if let Some(state) = source_state {
                put_state(tx, &t.from_shard, state)?;
            }
            tx.execute(
                &insert,
                &[
                    Val::Text(&t.id),
                    Val::Text(&t.subscriber),
                    Val::Text(&t.from_shard),
                    Val::Text(&t.to_shard),
                    Val::Bytes(&t.state),
                    Val::Json(&t.auth),
                ],
            )?;
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
        let [p1, p2] = self.d().params();
        let sql = format!(
            "UPDATE _pylon_shard_transfers SET status = {p2}
             WHERE transfer_id = {p1} AND status = 'out'"
        );
        self.db.tx(Retry::Once, |tx| {
            tx.pg_only("SET LOCAL idle_in_transaction_session_timeout = '15s'")?;
            if !holds(tx, shard, machine, epoch)? {
                return Ok(Settle::NotHeld);
            }
            if tx.execute(&sql, &[Val::Text(id), Val::Text(status)])? != 1 {
                return Ok(Settle::Taken);
            }
            if let Some(state) = state {
                put_state(tx, shard, state)?;
            }
            Ok(Settle::Done)
        })
    }

    /// The database, for tests that write rows directly.
    #[cfg(test)]
    pub(crate) fn db_for_tests(&self) -> &Db {
        &self.db
    }

    /// Every transfer row of `subscriber`, oldest first.
    pub fn transfers_of(&self, subscriber: &str) -> Result<Vec<Transfer>, String> {
        let [p1] = self.d().params();
        let sql = format!(
            "SELECT transfer_id, subscriber, from_shard, to_shard, state, auth, status
             FROM _pylon_shard_transfers WHERE subscriber = {p1} AND status <> 'void'
             ORDER BY seq"
        );
        self.db.run(false, |tx| {
            Ok(tx
                .query(&sql, &[Val::Text(subscriber)])?
                .iter()
                .map(transfer_of)
                .collect())
        })
    }

    /// Transfers still `out` from `shard`, of any age: a shard just started
    /// from saved state finishes them first.
    pub fn open_transfers_from(&self, shard: &str) -> Result<Vec<Transfer>, String> {
        let [p1] = self.d().params();
        let sql = format!(
            "SELECT transfer_id, subscriber, from_shard, to_shard, state, auth, status
             FROM _pylon_shard_transfers WHERE from_shard = {p1} AND status = 'out'
             ORDER BY seq"
        );
        self.db.run(false, |tx| {
            Ok(tx
                .query(&sql, &[Val::Text(shard)])?
                .iter()
                .map(transfer_of)
                .collect())
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
        let d = self.d();
        let [p1, p2] = d.params();
        let now = d.now_ms();
        let sql = format!(
            "SELECT transfer_id, subscriber, from_shard, to_shard, state, auth, status, age
             FROM (SELECT t.transfer_id, t.subscriber, t.from_shard, t.to_shard, t.state,
                          t.auth, t.status, {now} - t.created_at AS age,
                          row_number() OVER (PARTITION BY t.subscriber ORDER BY t.seq DESC) AS n
                   FROM _pylon_shard_transfers t
                   WHERE t.from_shard = {p1} AND t.status = 'in'
                     AND t.created_at > {now} - {p2}
                     AND NOT EXISTS (
                       SELECT 1 FROM _pylon_shard_transfers back
                       WHERE back.subscriber = t.subscriber AND back.to_shard = {p1}
                         AND back.status = 'in' AND back.seq > t.seq)) latest
             WHERE n = 1
             ORDER BY subscriber"
        );
        self.db.run(false, |tx| {
            Ok(tx
                .query(&sql, &[Val::Text(shard), Val::I64(within_ms)])?
                .iter()
                .map(|r| (transfer_of(r), r.i64(7)))
                .collect())
        })
    }

    /// The status of transfer `id` once no transaction on it is still in
    /// flight: `None` when it has no row (and never will: a `void` row takes
    /// the id, so a late insert fails). Waits for an uncommitted insert or
    /// update of the row, so a commit whose reply was lost is counted. (On
    /// SQLite the transaction starts with the database's write lock, which
    /// waits for any other writer.)
    pub fn confirm_status(&self, id: &str) -> Result<Option<String>, String> {
        let d = self.d();
        let [p1, p2, p3] = d.params();
        let now = d.now_ms();
        let (seq_col, seq_val) = next_seq(d);
        let insert = format!(
            "INSERT INTO _pylon_shard_transfers
                (transfer_id, subscriber, from_shard, to_shard, state, auth, status,
                 created_at{seq_col})
             VALUES ({p1}, '', '', '', {p2}, {p3}, 'void', {now}{seq_val})
             ON CONFLICT (transfer_id) DO NOTHING"
        );
        let select = format!(
            "SELECT status FROM _pylon_shard_transfers WHERE transfer_id = {p1}{}",
            d.for_update()
        );
        let none = serde_json::json!({});
        let status = self.db.tx(Retry::Once, |tx| {
            tx.pg_only(
                "SET LOCAL lock_timeout = '5s';
                 SET LOCAL idle_in_transaction_session_timeout = '15s'",
            )?;
            // Blocks behind an uncommitted insert of the same id.
            tx.execute(&insert, &[Val::Text(id), Val::Bytes(&[]), Val::Json(&none)])?;
            // Blocks behind an uncommitted update of it.
            Ok(tx.query_one(&select, &[Val::Text(id)])?.string(0))
        })?;
        Ok((status != "void").then_some(status))
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
        let d = self.d();
        let [p1, p2, p3] = d.params();
        let now = d.now_ms();
        let sql = format!(
            "SELECT t.transfer_id, t.subscriber, t.from_shard, t.to_shard, t.state, t.auth,
                    t.status, {now} - t.created_at
             FROM _pylon_shard_transfers t
             JOIN _pylon_shard_placements p ON p.shard_id = t.to_shard
             WHERE t.status = 'out' AND p.machine_id = {p1} AND p.epoch = {p2}
               AND NOT EXISTS (SELECT 1 FROM _pylon_shard_placements s
                               WHERE s.shard_id = t.from_shard)
               AND t.created_at < {now} - {p3}
             ORDER BY t.seq"
        );
        self.db.run(false, |tx| {
            Ok(tx
                .query(
                    &sql,
                    &[Val::Text(machine), Val::I64(epoch), Val::I64(older_than_ms)],
                )?
                .iter()
                .map(|r| (transfer_of(r), r.i64(7)))
                .collect())
        })
    }

    pub fn transfer(&self, id: &str) -> Result<Option<Transfer>, String> {
        let [p1] = self.d().params();
        let sql = format!(
            "SELECT transfer_id, subscriber, from_shard, to_shard, state, auth, status
             FROM _pylon_shard_transfers WHERE transfer_id = {p1} AND status <> 'void'"
        );
        self.db.run(false, |tx| {
            Ok(tx
                .query_opt(&sql, &[Val::Text(id)])?
                .map(|r| transfer_of(&r)))
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
        let d = self.d();
        let [p1, p2, p3] = d.params();
        let now = d.now_ms();
        let sql = format!(
            "SELECT t.transfer_id, t.subscriber, t.from_shard, t.to_shard, t.state, t.auth,
                    t.status
             FROM _pylon_shard_transfers t
             JOIN _pylon_shard_placements p ON p.shard_id = t.from_shard
             WHERE t.status = 'out' AND p.machine_id = {p1} AND p.epoch = {p2}
               AND t.created_at < {now} - {p3}
             ORDER BY t.seq"
        );
        self.db.run(false, |tx| {
            Ok(tx
                .query(
                    &sql,
                    &[Val::Text(machine), Val::I64(epoch), Val::I64(older_than_ms)],
                )?
                .iter()
                .map(transfer_of)
                .collect())
        })
    }

    /// Drop settled transfers older than an hour, and abandoned ones older
    /// than a week (kept that long for an operator to recover by hand).
    pub fn prune_transfers(&self) -> Result<(), String> {
        let now = self.d().now_ms();
        let sql = format!(
            "DELETE FROM _pylon_shard_transfers
             WHERE (status IN ('in', 'back', 'void') AND created_at < {now} - 3600000)
                OR (status = 'dropped' AND created_at < {now} - 604800000)"
        );
        self.db.run(true, |tx| {
            tx.execute(&sql, &[])?;
            Ok(())
        })
    }

    /// Record that the target refused transfer `id` (its source stopped),
    /// and return how long it has refused, in ms, since the first time.
    pub fn note_refusal(&self, id: &str) -> Result<i64, String> {
        let d = self.d();
        let [p1] = d.params();
        let now = d.now_ms();
        let sql = format!(
            "UPDATE _pylon_shard_transfers
             SET refused_at = COALESCE(refused_at, {now})
             WHERE transfer_id = {p1}
             RETURNING {now} - refused_at"
        );
        self.db.run(true, |tx| {
            Ok(tx
                .query_opt(&sql, &[Val::Text(id)])?
                .map(|r| r.i64(0))
                .unwrap_or(0))
        })
    }

    /// Mark `dropped` the transfers `out` for longer than `older_than_ms`
    /// whose source and target both have no placement: nothing will take
    /// them. The rows keep the players' states. Returns them.
    pub fn drop_stranded_rows(&self, older_than_ms: i64) -> Result<Vec<Transfer>, String> {
        let d = self.d();
        let [p1] = d.params();
        let now = d.now_ms();
        let sql = format!(
            "UPDATE _pylon_shard_transfers AS t SET status = 'dropped'
             WHERE t.status = 'out'
               AND t.created_at < {now} - {p1}
               AND NOT EXISTS (SELECT 1 FROM _pylon_shard_placements p
                               WHERE p.shard_id IN (t.from_shard, t.to_shard))
             RETURNING transfer_id, subscriber, from_shard, to_shard, state, auth, status"
        );
        self.db.run(true, |tx| {
            Ok(tx
                .query(&sql, &[Val::I64(older_than_ms)])?
                .iter()
                .map(transfer_of)
                .collect())
        })
    }

    /// Give up on transfer `id`: its source was stopped and its target
    /// keeps refusing the player. `out` becomes `dropped`; the row keeps the
    /// player's state. False when it was no longer `out`.
    pub fn abandon_transfer(&self, id: &str) -> Result<bool, String> {
        let [p1] = self.d().params();
        let sql = format!(
            "UPDATE _pylon_shard_transfers SET status = 'dropped'
             WHERE transfer_id = {p1} AND status = 'out'"
        );
        self.db.tx(Retry::Once, |tx| {
            Ok(tx.execute(&sql, &[Val::Text(id)])? == 1)
        })
    }

    /// Remove this machine's row under `epoch`: it is shutting down, and
    /// its shards should move now rather than after the dead time. A row a
    /// newer process wrote under the same id stays.
    pub fn leave(&self, machine_id: &str, epoch: i64) -> Result<(), String> {
        let [p1, p2] = self.d().params();
        let sql =
            format!("DELETE FROM _pylon_shard_machines WHERE machine_id = {p1} AND epoch = {p2}");
        self.db.run(true, |tx| {
            tx.execute(&sql, &[Val::Text(machine_id), Val::I64(epoch)])?;
            Ok(())
        })
    }

    /// Drop machine rows silent for an hour.
    pub fn prune_machines(&self) -> Result<(), String> {
        let now = self.d().now_ms();
        let sql = format!("DELETE FROM _pylon_shard_machines WHERE heartbeat_at < {now} - 3600000");
        self.db.run(true, |tx| {
            tx.execute(&sql, &[])?;
            Ok(())
        })
    }
}

/// The directory's tables in Postgres, created or brought up to date.
fn create_pg_schema(pool: &PgPool) -> Result<(), String> {
    pool.with_client(|client| {
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
                    "SELECT EXISTS (SELECT 1 FROM pg_attribute
                                    WHERE attrelid = to_regclass($1)
                                      AND attname = $2 AND NOT attisdropped)",
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
        // An unused column from before; kept (a machine on an older build
        // may still write it) but no longer required.
        if column(&mut tx, "_pylon_shard_machines", "load")? {
            tx.batch_execute("ALTER TABLE _pylon_shard_machines ALTER COLUMN load DROP NOT NULL")?;
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
        // Insert order: a move out of a shard is written after the move in
        // that brought the player there, whatever the clocks say.
        if !column(&mut tx, "_pylon_shard_transfers", "seq")? {
            // Existing rows are numbered by created_at (a serial column
            // would number them in scan order, which updates shuffle); then
            // a sequence numbers new rows. One transaction: the ALTER locks
            // the table until the commit, so no row goes in without a
            // number.
            tx.batch_execute(
                "ALTER TABLE _pylon_shard_transfers ADD COLUMN seq BIGINT;
                 UPDATE _pylon_shard_transfers t SET seq = o.n
                   FROM (SELECT transfer_id,
                                row_number() OVER (ORDER BY created_at, transfer_id) AS n
                         FROM _pylon_shard_transfers) o
                  WHERE t.transfer_id = o.transfer_id;
                 CREATE SEQUENCE _pylon_shard_transfers_seq
                   OWNED BY _pylon_shard_transfers.seq;
                 SELECT setval('_pylon_shard_transfers_seq',
                               COALESCE((SELECT max(seq) FROM _pylon_shard_transfers), 0) + 1,
                               false);
                 ALTER TABLE _pylon_shard_transfers
                   ALTER COLUMN seq SET DEFAULT nextval('_pylon_shard_transfers_seq'),
                   ALTER COLUMN seq SET NOT NULL;",
            )?;
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
        tx.commit()
    })
}

/// The directory's tables in SQLite. JSON is stored as text, and a transfer
/// row's `seq` is the next number at its insert (the insert holds the
/// database's write lock).
const SQLITE_SCHEMA: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS _pylon_shard_machines (
        machine_id TEXT PRIMARY KEY,
        address TEXT,
        fly_instance TEXT,
        capacity INTEGER NOT NULL,
        heartbeat_at INTEGER NOT NULL,
        epoch INTEGER NOT NULL DEFAULT 0
    )",
    "CREATE TABLE IF NOT EXISTS _pylon_shard_placements (
        shard_id TEXT PRIMARY KEY,
        kind TEXT NOT NULL,
        params TEXT NOT NULL,
        machine_id TEXT NOT NULL,
        pinned INTEGER NOT NULL DEFAULT 0,
        failed TEXT,
        epoch INTEGER NOT NULL DEFAULT 0,
        created_at INTEGER NOT NULL,
        moved_at INTEGER
    )",
    "CREATE INDEX IF NOT EXISTS _pylon_shard_placements_machine_idx
        ON _pylon_shard_placements (machine_id)",
    "CREATE TABLE IF NOT EXISTS _pylon_shard_state (
        shard_id TEXT PRIMARY KEY,
        state BLOB NOT NULL,
        saved_at INTEGER NOT NULL
    )",
    "CREATE TABLE IF NOT EXISTS _pylon_shard_transfers (
        transfer_id TEXT PRIMARY KEY,
        subscriber TEXT NOT NULL,
        from_shard TEXT NOT NULL,
        to_shard TEXT NOT NULL,
        state BLOB NOT NULL,
        auth TEXT NOT NULL,
        status TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        refused_at INTEGER,
        seq INTEGER NOT NULL
    )",
    "CREATE INDEX IF NOT EXISTS _pylon_shard_transfers_out_idx
        ON _pylon_shard_transfers (from_shard) WHERE status = 'out'",
    "CREATE INDEX IF NOT EXISTS _pylon_shard_transfers_age_idx
        ON _pylon_shard_transfers (status, created_at)",
    "CREATE UNIQUE INDEX IF NOT EXISTS _pylon_shard_transfers_seq_idx
        ON _pylon_shard_transfers (seq)",
    "CREATE TABLE IF NOT EXISTS _pylon_shard_cluster (
        id INTEGER PRIMARY KEY,
        call_key BLOB NOT NULL
    )",
];

/// The `seq` column and value a transfer insert adds: Postgres numbers the
/// row from a sequence; SQLite takes the next number under its write lock.
fn next_seq(d: Dialect) -> (&'static str, &'static str) {
    match d {
        Dialect::Pg => ("", ""),
        Dialect::Sqlite => (
            ", seq",
            ", (SELECT COALESCE(MAX(seq), 0) + 1 FROM _pylon_shard_transfers)",
        ),
    }
}

fn placement_of(r: &Row) -> Placement {
    Placement {
        shard_id: r.string(0),
        kind: r.string(1),
        params: r.json(2),
        machine_id: r.string(3),
        pinned: r.bool(4),
        failed: r.opt_string(5),
        epoch: r.i64(6),
    }
}

fn transfer_of(r: &Row) -> Transfer {
    Transfer {
        id: r.string(0),
        subscriber: r.string(1),
        from_shard: r.string(2),
        to_shard: r.string(3),
        state: r.bytes(4),
        auth: r.json(5),
        status: r.string(6),
    }
}

/// True when `machine` holds `shard` under `epoch`. On Postgres the
/// placement row stays share-locked until the transaction ends, as in
/// `save_state`; on SQLite the transaction holds the write lock.
fn holds(tx: &mut Tx<'_, '_>, shard: &str, machine: &str, epoch: i64) -> Result<bool, DbError> {
    let d = tx.dialect();
    let [p1, p2, p3] = d.params();
    let sql = format!(
        "SELECT 1 FROM _pylon_shard_placements
         WHERE shard_id = {p1} AND machine_id = {p2} AND epoch = {p3}{}",
        d.for_share()
    );
    Ok(tx
        .query_opt(
            &sql,
            &[Val::Text(shard), Val::Text(machine), Val::I64(epoch)],
        )?
        .is_some())
}

fn put_state(tx: &mut Tx<'_, '_>, shard: &str, state: &[u8]) -> Result<(), DbError> {
    let d = tx.dialect();
    let [p1, p2] = d.params();
    let now = d.now_ms();
    tx.execute(
        &format!(
            "INSERT INTO _pylon_shard_state (shard_id, state, saved_at)
             VALUES ({p1}, {p2}, {now})
             ON CONFLICT (shard_id) DO UPDATE SET
                state = EXCLUDED.state, saved_at = EXCLUDED.saved_at"
        ),
        &[Val::Text(shard), Val::Bytes(state)],
    )?;
    Ok(())
}

fn delete_state(tx: &mut Tx<'_, '_>, shard: &str) -> Result<(), DbError> {
    let [p1] = tx.dialect().params();
    tx.execute(
        &format!("DELETE FROM _pylon_shard_state WHERE shard_id = {p1}"),
        &[Val::Text(shard)],
    )?;
    Ok(())
}

/// SQL that is true when machine `machine` is live under `epoch`, with the
/// liveness window in milliseconds at parameter `window`.
fn owner_live_sql(d: Dialect, machine: &str, epoch: &str, window: &str) -> String {
    let now = d.now_ms();
    format!(
        "EXISTS (SELECT 1 FROM _pylon_shard_machines m
                 WHERE m.machine_id = {machine} AND m.epoch = {epoch}
                   AND m.heartbeat_at > {now} - {window})"
    )
}

/// True when `machine` holds `shard` under `epoch`, read on `conn` inside
/// its open write transaction: the fence of a shard's entity write on
/// SQLite (see `EntityWriter::update_fenced`). False when the app has no
/// directory tables.
pub(crate) fn sqlite_fence(
    conn: &rusqlite::Connection,
    shard: &str,
    machine: &str,
    epoch: i64,
) -> Result<bool, rusqlite::Error> {
    let tables: i64 = conn.query_row(
        "SELECT count(*) FROM sqlite_master
         WHERE type = 'table' AND name = '_pylon_shard_placements'",
        [],
        |r| r.get(0),
    )?;
    if tables == 0 {
        return Ok(false);
    }
    let mut tx = Tx::Sqlite(conn);
    holds(&mut tx, shard, machine, epoch).map_err(|e| match e {
        DbError::Sqlite(e) => e,
        DbError::Pg(e) => unreachable!("a Postgres error on a SQLite connection: {e}"),
    })
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

/// A call key for tests with no directory (a directory's own key wins when
/// a test opened one first).
#[cfg(test)]
pub(crate) fn call_key_for_tests() {
    let _ = CALL_KEY.set(vec![7; 32]);
}

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

    /// Run `test` on a directory in a fresh SQLite file, then (when
    /// `PYLON_TEST_PG_URL` is set) on the directory in Postgres.
    fn on_each_directory(test: impl Fn(Arc<ShardDirectory>)) {
        let file = tempfile::tempdir().expect("temp dir");
        let path = file.path().join("app.db");
        let sqlite = ShardDirectory::open_sqlite(path.to_str().unwrap()).expect("SQLite directory");
        test(Arc::new(sqlite));
        let Ok(url) = std::env::var("PYLON_TEST_PG_URL") else {
            eprintln!("Postgres part skipped: PYLON_TEST_PG_URL not set");
            return;
        };
        let _serial = DIRECTORY_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let pool = PgPool::connect(&url, 4, Duration::from_secs(5)).expect("test Postgres pool");
        test(Arc::new(ShardDirectory::open_pg(pool).expect("directory")));
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

    /// release_here removes the machine's placement under any epoch (a
    /// take-over by the same machine under a newer lease), and leaves one
    /// another machine holds.
    #[test]
    fn release_here_takes_any_epoch_of_this_machine_only() {
        on_each_directory(|dir| {
            let run = pylon_cluster::new_instance_id();
            let (mine, theirs, me, other) = (
                format!("rh-a-{run}"),
                format!("rh-b-{run}"),
                format!("me-{run}"),
                format!("other-{run}"),
            );
            let kind = format!("rh-{run}");
            for (shard, machine) in [(&mine, &me), (&theirs, &other)] {
                let p = Placement {
                    machine_id: machine.clone(),
                    ..placement(shard, &kind, machine)
                };
                assert_eq!(dir.claim(&p, 10).unwrap(), Claim::Claimed);
            }
            // Taken over by the same machine under a newer lease.
            assert!(dir.hand_over(&mine, &me, 1, &me, 2).unwrap());
            assert!(dir.release_here(&mine, &me).unwrap());
            assert_eq!(dir.placement(&mine).unwrap(), None);
            assert!(!dir.release_here(&theirs, &me).unwrap());
            assert!(dir.placement(&theirs).unwrap().is_some());
            assert!(dir.release_here(&theirs, &other).unwrap());
        });
    }

    /// A table from before the insert sequence: its rows are numbered by
    /// created_at, not in scan order, and new rows follow them. (Postgres
    /// only: a SQLite directory's table has the column from its start.)
    #[test]
    fn the_insert_sequence_numbers_old_rows_by_time() {
        let _serial = DIRECTORY_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let Ok(url) = std::env::var("PYLON_TEST_PG_URL") else {
            eprintln!("skipping: PYLON_TEST_PG_URL not set");
            return;
        };
        let schema = format!("mig_{}", pylon_cluster::new_instance_id().replace('-', "_"));
        let admin = PgPool::connect(&url, 1, Duration::from_secs(5)).unwrap();
        admin
            .with_client(|c| c.batch_execute(&format!("CREATE SCHEMA {schema}")))
            .unwrap();
        let sep = if url.contains('?') { '&' } else { '?' };
        let scoped = format!("{url}{sep}options=-c%20search_path%3D{schema}");
        let pool = PgPool::connect(&scoped, 2, Duration::from_secs(5)).unwrap();
        // The table as the last release made it, rows inserted out of time
        // order and one updated (which moves it in the heap).
        pool.with_client(|c| {
            c.batch_execute(
                "CREATE TABLE _pylon_shard_transfers (
                    transfer_id TEXT PRIMARY KEY, subscriber TEXT NOT NULL,
                    from_shard TEXT NOT NULL, to_shard TEXT NOT NULL,
                    state BYTEA NOT NULL, auth JSONB NOT NULL,
                    status TEXT NOT NULL, created_at BIGINT NOT NULL,
                    refused_at BIGINT);
                 INSERT INTO _pylon_shard_transfers VALUES
                    ('late', 'p', 'b', 'a', '', '{}', 'out', 300, NULL),
                    ('early', 'p', 'a', 'b', '', '{}', 'out', 100, NULL),
                    ('middle', 'q', 'a', 'b', '', '{}', 'out', 200, NULL);
                 UPDATE _pylon_shard_transfers SET status = 'in' WHERE transfer_id = 'early';",
            )
        })
        .unwrap();
        let dir = ShardDirectory::open_pg(Arc::clone(&pool)).unwrap();
        let order: Vec<String> = pool
            .with_client(|c| {
                Ok::<_, postgres::Error>(
                    c.query(
                        "SELECT transfer_id FROM _pylon_shard_transfers ORDER BY seq",
                        &[],
                    )?
                    .iter()
                    .map(|r| r.get(0))
                    .collect(),
                )
            })
            .unwrap();
        assert_eq!(order, ["early", "middle", "late"]);
        // A new row comes after them.
        pool.with_client(|c| {
            c.batch_execute(
                "INSERT INTO _pylon_shard_transfers
                    (transfer_id, subscriber, from_shard, to_shard, state, auth, status, created_at)
                 VALUES ('new', 'p', 'a', 'c', '', '{}', 'out', 50)",
            )
        })
        .unwrap();
        let last: String = pool
            .with_client(|c| {
                Ok::<_, postgres::Error>(
                    c.query_one(
                        "SELECT transfer_id FROM _pylon_shard_transfers ORDER BY seq DESC LIMIT 1",
                        &[],
                    )?
                    .get(0),
                )
            })
            .unwrap();
        assert_eq!(last, "new");
        drop(dir);
        admin
            .with_client(|c| c.batch_execute(&format!("DROP SCHEMA {schema} CASCADE")))
            .unwrap();
    }

    /// Write a settled transfer row directly, `ago_ms` before now, numbered
    /// after every row before it.
    fn settled_row(dir: &ShardDirectory, id: &str, sid: &str, from: &str, to: &str, ago_ms: i64) {
        let d = dir.db_for_tests().dialect();
        let [p1, p2, p3, p4, p5, p6, p7] = d.params();
        let (seq_col, seq_val) = next_seq(d);
        let at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64
            - ago_ms;
        let sql = format!(
            "INSERT INTO _pylon_shard_transfers
                (transfer_id, subscriber, from_shard, to_shard, state, auth, status,
                 created_at{seq_col})
             VALUES ({p1}, {p2}, {p3}, {p4}, {p5}, {p6}, 'in', {p7}{seq_val})"
        );
        let auth = serde_json::json!({});
        dir.db_for_tests()
            .tx(Retry::Replay, |tx| {
                tx.execute(
                    &sql,
                    &[
                        Val::Text(id),
                        Val::Text(sid),
                        Val::Text(from),
                        Val::Text(to),
                        Val::Bytes(&[]),
                        Val::Json(&auth),
                        Val::I64(at),
                    ],
                )?;
                Ok(())
            })
            .unwrap();
    }

    /// A -> B, then B -> A: A's restart must not send the subscriber to B,
    /// even when both rows carry the same time (the insert order decides).
    #[test]
    fn a_move_the_subscriber_came_back_from_is_not_recent() {
        on_each_directory(|dir| {
            let run = pylon_cluster::new_instance_id();
            let (a, b, sid) = (format!("a-{run}"), format!("b-{run}"), format!("p-{run}"));
            settled_row(&dir, &format!("t1-{run}"), &sid, &a, &b, 60_000);
            let moves = dir.recent_moves_from(&a, 600_000).unwrap();
            assert_eq!(moves.len(), 1, "A -> B is recent");
            // Written later, with the same clock reading.
            settled_row(&dir, &format!("t2-{run}"), &sid, &b, &a, 60_000);
            assert!(dir.recent_moves_from(&a, 600_000).unwrap().is_empty());
            // Out again later: that move is the recent one.
            settled_row(&dir, &format!("t3-{run}"), &sid, &a, &b, 10_000);
            let moves = dir.recent_moves_from(&a, 600_000).unwrap();
            assert_eq!(
                moves.iter().map(|(t, _)| t.id.clone()).collect::<Vec<_>>(),
                [format!("t3-{run}")]
            );
            let age = moves[0].1;
            assert!((9_000..60_000).contains(&age), "age {age} ms");
        });
    }

    #[test]
    fn the_directory_claims_moves_fences_fails_and_forgets() {
        on_each_directory(|dir| {
            // The key is in the directory now, the same for every machine.
            assert!(sign("x", b"y").is_some());
            let run = pylon_cluster::new_instance_id();
            let (a, b) = (format!("a-{run}"), format!("b-{run}"));
            let shard = format!("s-{run}");
            // A kind of its own: the cluster-wide limit counts only this run.
            let kind = format!("arena-{run}");
            assert!(dir.heartbeat(&me(&a), 7).unwrap());
            assert!(dir.heartbeat(&me(&b), 1).unwrap());
            let orphan = |dir: &ShardDirectory| {
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
        });
    }

    /// A move's rows, from out to settled, and the checks around them.
    #[test]
    fn transfers_settle_once_and_age() {
        on_each_directory(|dir| {
            let run = pylon_cluster::new_instance_id();
            let (a, b, m) = (format!("a-{run}"), format!("b-{run}"), format!("m-{run}"));
            let kind = format!("t-{run}");
            for shard in [&a, &b] {
                let p = Placement {
                    machine_id: m.clone(),
                    ..placement(shard, &kind, &m)
                };
                assert_eq!(dir.claim(&p, 10).unwrap(), Claim::Claimed);
            }
            let t = Transfer {
                id: format!("x-{run}"),
                subscriber: format!("p-{run}"),
                from_shard: a.clone(),
                to_shard: b.clone(),
                state: b"player".to_vec(),
                auth: serde_json::json!({ "user_id": "p" }),
                status: "out".into(),
            };
            // Only the source's holder writes the row.
            assert!(!dir.begin_transfer(&t, &m, 2, Some(b"a-without")).unwrap());
            assert!(dir.begin_transfer(&t, &m, 1, Some(b"a-without")).unwrap());
            assert_eq!(dir.transfer(&t.id).unwrap(), Some(t.clone()));
            assert_eq!(dir.open_transfers_from(&a).unwrap(), vec![t.clone()]);
            assert_eq!(
                dir.load_owned_state(&a, &m, 1).unwrap(),
                Some(Some(b"a-without".to_vec()))
            );
            // Not stale yet; stale once older than 0 ms.
            assert!(dir.stale_transfers(&m, 1, 60_000).unwrap().is_empty());
            std::thread::sleep(Duration::from_millis(5));
            assert_eq!(dir.stale_transfers(&m, 1, 0).unwrap().len(), 1);
            // One side settles it; the other finds it taken.
            assert_eq!(
                dir.accept_transfer(&t.id, &b, &m, 1, Some(b"b-with"))
                    .unwrap(),
                Settle::Done
            );
            assert_eq!(
                dir.return_transfer(&t.id, &a, &m, 1, Some(b"a-with"))
                    .unwrap(),
                Settle::Taken
            );
            assert_eq!(
                dir.accept_transfer(&t.id, &b, &m, 9, None).unwrap(),
                Settle::NotHeld
            );
            assert_eq!(
                dir.load_owned_state(&b, &m, 1).unwrap(),
                Some(Some(b"b-with".to_vec()))
            );
            assert_eq!(dir.confirm_status(&t.id).unwrap().as_deref(), Some("in"));
            assert_eq!(dir.transfers_of(&t.subscriber).unwrap().len(), 1);
            // An id with no row gets a void row: a late insert of it fails.
            let lost = format!("lost-{run}");
            assert_eq!(dir.confirm_status(&lost).unwrap(), None);
            let late = Transfer {
                id: lost.clone(),
                ..t.clone()
            };
            assert!(dir.begin_transfer(&late, &m, 1, None).is_err());
            assert_eq!(dir.transfer(&lost).unwrap(), None);
            // A refusal is timed from the first one.
            let refused = Transfer {
                id: format!("r-{run}"),
                ..t.clone()
            };
            assert!(dir.begin_transfer(&refused, &m, 1, None).unwrap());
            // Within the statement the clock can move a millisecond on Postgres.
            assert!(dir.note_refusal(&refused.id).unwrap() <= 1);
            std::thread::sleep(Duration::from_millis(20));
            assert!(dir.note_refusal(&refused.id).unwrap() >= 10);
            assert!(dir.abandon_transfer(&refused.id).unwrap());
            assert!(!dir.abandon_transfer(&refused.id).unwrap());
            // Both ends gone: a row still out is dropped, with its state.
            let stranded = Transfer {
                id: format!("s-{run}"),
                ..t.clone()
            };
            assert!(dir.begin_transfer(&stranded, &m, 1, None).unwrap());
            assert!(dir.release(&a, &m, 1).unwrap());
            std::thread::sleep(Duration::from_millis(5));
            assert_eq!(dir.ownerless_transfers(&m, 1, 0).unwrap().len(), 1);
            assert!(dir.release(&b, &m, 1).unwrap());
            std::thread::sleep(Duration::from_millis(5));
            // Only this run's row: a shared Postgres test database can hold
            // other runs' stranded rows.
            let dropped: Vec<Transfer> = dir
                .drop_stranded_rows(0)
                .unwrap()
                .into_iter()
                .filter(|t| t.id.ends_with(&run))
                .collect();
            assert_eq!(dropped.len(), 1);
            assert_eq!(dropped[0].id, stranded.id);
            assert_eq!(dropped[0].state, b"player");
            dir.prune_transfers().unwrap();
            dir.prune_machines().unwrap();
        });
    }

    #[test]
    fn a_takeover_waits_for_a_save_in_progress() {
        on_each_directory(|dir| {
            let run = pylon_cluster::new_instance_id();
            let (a, b) = (format!("a-{run}"), format!("b-{run}"));
            let shard = format!("s-{run}");
            assert!(dir.heartbeat(&me(&a), 1).unwrap());
            // A kind of its own: the cluster-wide limit counts only this run.
            let p = placement(&shard, &format!("arena-{run}"), &b);
            assert_eq!(dir.claim(&p, 10).unwrap(), Claim::Claimed);
            // B never heartbeat: its placement is an orphan.

            // B's save holds what save_state holds (the row lock, or on
            // SQLite the write lock), until released.
            let (locked_tx, locked_rx) = std::sync::mpsc::channel();
            let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
            let release_rx = Mutex::new(release_rx);
            let saver = {
                let (dir, shard, b) = (Arc::clone(&dir), shard.clone(), b.clone());
                std::thread::spawn(move || {
                    dir.db_for_tests()
                        .tx(Retry::Once, |tx| {
                            assert!(holds(tx, &shard, &b, 1)?);
                            locked_tx.send(()).unwrap();
                            release_rx.lock().unwrap().recv().unwrap();
                            put_state(tx, &shard, b"last")
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
        });
    }

    /// A claim of an id already placed is `Taken` even when its kind is at
    /// its limit: the caller treats it as the shard existing.
    #[test]
    fn a_placed_id_is_taken_even_at_the_kind_limit() {
        on_each_directory(|dir| {
            let run = pylon_cluster::new_instance_id();
            let (kind, shard) = (format!("one-{run}"), format!("only-{run}"));
            assert_eq!(
                dir.claim(&placement(&shard, &kind, "m1"), 1).unwrap(),
                Claim::Claimed
            );
            assert_eq!(
                dir.claim(&placement(&shard, &kind, "m2"), 1).unwrap(),
                Claim::Taken
            );
            assert_eq!(
                dir.claim(&placement(&format!("other-{run}"), &kind, "m2"), 1)
                    .unwrap(),
                Claim::LimitReached
            );
            assert!(dir.release(&shard, "m1", 1).unwrap());
        });
    }

    #[test]
    fn concurrent_claims_never_pass_the_kind_limit() {
        on_each_directory(|dir| {
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
        });
    }

    /// One process runs a SQLite file's shards: a second directory on the
    /// file is refused while the first is open, and opens once it closes,
    /// with the first process's machine rows gone (it has exited).
    #[test]
    fn one_process_runs_a_sqlite_files_shards() {
        let file = tempfile::tempdir().unwrap();
        let path = file.path().join("app.db");
        let path = path.to_str().unwrap();
        let first = ShardDirectory::open_sqlite(path).unwrap();
        assert!(first.heartbeat(&me("old"), 1).unwrap());
        let p = placement("zone", "zone", "old");
        assert_eq!(first.claim(&p, 10).unwrap(), Claim::Claimed);
        assert!(first.save_state("zone", "old", 1, b"saved").unwrap());
        let refused = ShardDirectory::open_sqlite_waiting(path, Duration::from_millis(300))
            .err()
            .expect("refused");
        assert!(
            refused.contains("another process runs the shards"),
            "{refused}"
        );
        // The same file through a symbolic link is the same lock.
        #[cfg(unix)]
        {
            let link = file.path().join("link.db");
            std::os::unix::fs::symlink(path, &link).unwrap();
            let refused =
                ShardDirectory::open_sqlite_waiting(link.to_str().unwrap(), Duration::ZERO)
                    .err()
                    .expect("refused through the link");
            assert!(
                refused.contains("another process runs the shards"),
                "{refused}"
            );
        }
        // A process still exiting lets go within the wait.
        let exiting = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            drop(first);
        });
        let second = ShardDirectory::open_sqlite_waiting(path, Duration::from_secs(5)).unwrap();
        exiting.join().unwrap();
        assert!(second.live_machines().unwrap().is_empty());
        // Its placement is an orphan at once, with its state.
        let orphans = second.orphans().unwrap();
        assert_eq!(orphans, vec![p.clone()]);
        assert!(second.heartbeat(&me("new"), 5).unwrap());
        assert!(second.take_over(&p, "new", 5).unwrap());
        assert_eq!(
            second.load_owned_state("zone", "new", 5).unwrap(),
            Some(Some(b"saved".to_vec()))
        );
        assert_eq!(second.timing(), SQLITE_TIMING);
    }

    /// The fence of a shard's entity write on SQLite reads the placement in
    /// the writer's own transaction.
    #[test]
    fn the_sqlite_fence_reads_the_placement() {
        let file = tempfile::tempdir().unwrap();
        let path = file.path().join("app.db");
        let path = path.to_str().unwrap();
        let conn = rusqlite::Connection::open(path).unwrap();
        // No directory tables: no shard holds anything.
        assert!(!sqlite_fence(&conn, "zone", "m", 1).unwrap());
        let dir = ShardDirectory::open_sqlite(path).unwrap();
        assert_eq!(
            dir.claim(&placement("zone", "zone", "m"), 10).unwrap(),
            Claim::Claimed
        );
        assert!(sqlite_fence(&conn, "zone", "m", 1).unwrap());
        assert!(!sqlite_fence(&conn, "zone", "m", 2).unwrap());
        assert!(dir.hand_over("zone", "m", 1, "other", 3).unwrap());
        assert!(!sqlite_fence(&conn, "zone", "m", 1).unwrap());
    }
}
