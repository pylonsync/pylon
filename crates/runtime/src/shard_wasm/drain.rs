//! Deploys that do not disconnect players (issue #34).
//!
//! A machine that shuts down (SIGTERM, a rolling deploy) hands each shard
//! it holds to another live machine before it leaves the directory:
//!
//! 1. It pauses the shard (no tick; connections stay open), writes its
//!    buffered entity writes, and saves its state.
//! 2. It asks a live machine that is not shutting down to take it
//!    ([`RemoteOp::HandOver`]). That machine moves the placement to itself
//!    under its own lease and starts the shard from the saved state.
//! 3. It sends each connection a transfer frame for the same shard, with a
//!    new ticket, and closes it. The clients reconnect at once; the
//!    placement now names the new machine, so the connection goes there.
//!
//! A shard it cannot hand over within the drain time
//! (`PYLON_SHARD_DRAIN_SECS`, default [`DRAIN_SECS`]) is saved and left
//! placed here, as before: another machine starts it once this one leaves,
//! and its clients reconnect after the gap. Inputs sent while a shard is
//! paused are lost; its state is not.

use std::sync::Arc;
use std::time::{Duration, Instant};

use pylon_realtime::Shard;

use super::{ClusterState, ShardInfo, WasmShardHost, WasmSim};
use crate::shard_cluster::{self, Machine, Placement, RemoteOp, RemoteReply};

/// The drain time when `PYLON_SHARD_DRAIN_SECS` is unset. A platform's
/// kill timeout must be longer (Fly: `kill_timeout` in fly.toml).
pub(super) const DRAIN_SECS: u64 = 20;
/// How long a new connection's ticket is good for.
const TICKET_TTL_SECS: u64 = 600;

pub(super) fn drain_time() -> Duration {
    Duration::from_secs(
        std::env::var("PYLON_SHARD_DRAIN_SECS")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(DRAIN_SECS),
    )
}

impl WasmShardHost {
    /// Hand shard `id`, held here under `epoch`, to another machine. True
    /// when that machine runs it now and its connections were told; false
    /// leaves it paused and placed here (the caller saves and stops it).
    pub(super) fn hand_over(
        &self,
        c: &ClusterState,
        id: &str,
        epoch: i64,
        shard: &Arc<Shard<WasmSim>>,
        deadline: Instant,
    ) -> bool {
        let targets = match c.dir.live_machines() {
            Ok(live) => targets(live, &c.me.id),
            Err(e) => {
                tracing::warn!("[shard {id}] cannot hand over: {e}");
                return false;
            }
        };
        if targets.is_empty() {
            return false;
        }
        shard.pause();
        self.flush_writes(super::data::Flush::Shard(id));
        if !self.final_save(c, id, epoch, shard) {
            return false;
        }
        let op = RemoteOp::HandOver {
            id: id.to_string(),
            from: c.me.id.clone(),
            epoch,
        };
        for target in targets {
            if Instant::now() >= deadline {
                return false;
            }
            let Some(address) = target.address.as_deref() else {
                continue;
            };
            match shard_cluster::call(&target.id, address, &op) {
                Ok(RemoteReply::Ok(_)) => {
                    self.tell_moved(id, shard);
                    tracing::info!("[shard {id}] handed over to machine {}", target.id);
                    return true;
                }
                Ok(RemoteReply::Err { code, message }) => tracing::warn!(
                    "[shard {id}] machine {} did not take it: {code}: {message}",
                    target.id
                ),
                Err(e) => tracing::warn!("[shard {id}] machine {} unreachable: {e}", target.id),
            }
            // The placement may have moved even when the answer was lost:
            // then the connections go to the machine that has it.
            match c.dir.placement(id) {
                Ok(Some(p)) if p.machine_id != c.me.id || p.epoch != epoch => {
                    self.tell_moved(id, shard);
                    return true;
                }
                _ => {}
            }
        }
        false
    }

    /// Save a paused shard's state under `epoch`. True when saved (or it
    /// keeps no state).
    fn final_save(&self, c: &ClusterState, id: &str, epoch: i64, shard: &Shard<WasmSim>) -> bool {
        if !self.saves_state(id) {
            return true;
        }
        let _save = c.save_lock.lock().unwrap();
        // A transfer step whose commit is unknown: its row decides, and no
        // save may land until then.
        if self.unsettled_on(id) {
            return false;
        }
        match shard.with_state(|sim| sim.save()) {
            Ok(Some(state)) => match c.dir.save_state(id, &c.me.id, epoch, &state) {
                Ok(true) => true,
                Ok(false) => false,
                Err(e) => {
                    tracing::warn!("[shard {id}] final save failed: {e}");
                    false
                }
            },
            Ok(None) => true,
            Err(e) => {
                tracing::warn!("[shard {id}] final save failed: {e}");
                false
            }
        }
    }

    /// Send each connection of shard `id` a transfer frame for the same
    /// shard, with a new ticket for its user and claims, and close it.
    fn tell_moved(&self, id: &str, shard: &Shard<WasmSim>) {
        for (sid, _) in shard.subscriber_ids() {
            let auth = shard.subscriber_auth(&sid).unwrap_or_default();
            let claims = auth
                .ticket
                .as_ref()
                .map(|t| t.claims.clone())
                .unwrap_or(serde_json::Value::Null);
            let notice = pylon_realtime::wire::TransferNotice {
                shard: id.to_string(),
                ticket: crate::shard_tickets::mint(
                    id,
                    sid.as_str(),
                    auth.user_id.clone(),
                    claims,
                    Some(TICKET_TTL_SECS),
                ),
            };
            shard.hand_off(&sid, &notice, Duration::ZERO);
        }
    }

    /// Take shard `id` from machine `from`, which is shutting down and
    /// holds it under `epoch` with its final state saved: move the
    /// placement here and start it from that state.
    pub(super) fn accept_hand_over(
        &self,
        id: &str,
        from: &str,
        epoch: i64,
    ) -> Result<ShardInfo, (String, String)> {
        let refuse = |code: &str, why: String| (code.to_string(), why);
        let Some(c) = self.cluster.get() else {
            return Err(refuse("SHARD_CLUSTER_ERROR", "no shard directory".into()));
        };
        // Not shutting down itself (a machine that is holds its own lock
        // while it drains), with a lease to start it under.
        let not_taking = || {
            refuse(
                "SHARD_UNAVAILABLE",
                "this machine is not taking shards".into(),
            )
        };
        if c.current_epoch().is_none() {
            return Err(not_taking());
        }
        let _own = c.own_lock.lock().unwrap();
        let Some(my_epoch) = c.current_epoch() else {
            return Err(not_taking());
        };
        let p = match c.dir.placement(id) {
            Ok(Some(p)) if p.machine_id == from && p.epoch == epoch => p,
            Ok(_) => {
                return Err(refuse(
                    "SHARD_NOT_FOUND",
                    format!("{from} does not hold {id}"),
                ))
            }
            Err(e) => return Err(refuse("SHARD_CLUSTER_ERROR", e)),
        };
        match c.dir.hand_over(id, from, epoch, &c.me.id, my_epoch) {
            Ok(true) => {}
            Ok(false) => {
                return Err(refuse(
                    "SHARD_NOT_FOUND",
                    format!("{from} no longer holds {id}"),
                ))
            }
            Err(e) => return Err(refuse("SHARD_CLUSTER_ERROR", e)),
        }
        let p = Placement {
            machine_id: c.me.id.clone(),
            epoch: my_epoch,
            ..p
        };
        tracing::info!("[shard {id}] machine {from} is shutting down; starting it here");
        self.adopt(&p, my_epoch)
            .map_err(|why| refuse("SHARD_INIT_FAILED", why))
    }

    /// Hand every shard in `held` to another machine, within the drain
    /// time. The ones left over are returned, paused or not.
    pub(super) fn drain(
        &self,
        c: &ClusterState,
        held: Vec<(String, i64, Arc<Shard<WasmSim>>, bool)>,
    ) -> Vec<(String, i64, Arc<Shard<WasmSim>>, bool)> {
        let time = drain_time();
        if held.is_empty() || time.is_zero() {
            return held;
        }
        let deadline = Instant::now() + time;
        // Others stop choosing this machine for new shards and hand-overs.
        self.heartbeat();
        let mut left = Vec::new();
        for (id, epoch, shard, saves) in held {
            // A shard with no saved state starts from `init` there; its
            // clients still reconnect at once.
            if Instant::now() < deadline && self.hand_over(c, &id, epoch, &shard, deadline) {
                shard.stop_and_wait();
                self.stop_local(&id);
                continue;
            }
            left.push((id, epoch, shard, saves));
        }
        left
    }
}

/// Machines that may take a shard: live, reachable, not `me`, with room
/// (a shutting-down machine reports none), least loaded first.
fn targets(mut live: Vec<Machine>, me: &str) -> Vec<Machine> {
    live.retain(|m| m.id != me && m.address.is_some() && m.capacity > 0);
    live.sort_by_key(|m| {
        (
            m.load >= m.capacity,
            m.load * 1000 / m.capacity,
            m.id.clone(),
        )
    });
    live
}
