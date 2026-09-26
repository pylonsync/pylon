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

/// How long a ticket in a deploy's transfer frame is good for.
const TICKET_TTL_SECS: u64 = 600;

/// The drain time when `PYLON_SHARD_DRAIN_SECS` is unset. A platform's
/// kill timeout must be longer (Fly: `kill_timeout` in fly.toml).
pub(super) const DRAIN_SECS: u64 = 20;

pub(super) fn drain_time() -> Duration {
    Duration::from_secs(
        std::env::var("PYLON_SHARD_DRAIN_SECS")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(DRAIN_SECS),
    )
}

impl WasmShardHost {
    /// Hand shard `id`, held here under `epoch`, paused and saved, to
    /// another machine, within `deadline`. True when that machine runs it
    /// now and its connections were told; false leaves it placed here (the
    /// caller saves it again and stops it).
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
        let op = RemoteOp::HandOver {
            id: id.to_string(),
            from: c.me.id.clone(),
            epoch,
        };
        for target in targets {
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                return false;
            };
            let Some(address) = target.address.as_deref() else {
                continue;
            };
            // Bounded by the drain time left, so a target that does not
            // answer cannot hold the shutdown past it.
            let agent = ureq::AgentBuilder::new()
                .timeout_connect(left.min(Duration::from_secs(3)))
                .timeout(left.min(Duration::from_secs(10)))
                .redirects(0)
                .build();
            match shard_cluster::call_with(&agent, &target.id, address, &op) {
                Ok(RemoteReply::Ok(_)) => {
                    self.tell_moved(c, id, shard);
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
                    self.tell_moved(c, id, shard);
                    return true;
                }
                _ => {}
            }
        }
        false
    }

    /// Pause shard `id`, write its buffered entity writes, and save its
    /// state under `epoch`. True when all of that went through: only then
    /// may another machine take it.
    fn pause_and_save(
        &self,
        c: &ClusterState,
        id: &str,
        epoch: i64,
        shard: &Shard<WasmSim>,
    ) -> bool {
        shard.pause();
        if self.flush_writes(super::data::Flush::Shard(id)) > 0 {
            tracing::warn!("[shard {id}] not handed over: its entity writes failed");
            return false;
        }
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

    /// Tell each connection of shard `id` where to go, and close it: a
    /// subscriber that moved to another shard (a move that finished while
    /// this machine shut down) to that shard, with a ticket for it; one
    /// whose move is still open, nowhere (its connection closes when the
    /// shard stops, and the machine that takes the shard finishes the
    /// move); every other, to the same shard, with a new ticket for its
    /// user and claims.
    fn tell_moved(&self, c: &ClusterState, id: &str, shard: &Shard<WasmSim>) {
        let finished: std::collections::HashMap<String, shard_cluster::Transfer> =
            match c.dir.recent_moves_from(id, TICKET_TTL_SECS as i64 * 1000) {
                Ok(moves) => moves
                    .into_iter()
                    .map(|(t, _)| (t.subscriber.clone(), t))
                    .collect(),
                // Unsure: no notice at all; the clients reconnect on their own.
                Err(e) => {
                    tracing::warn!("[shard {id}] its connections are closed with no notice: {e}");
                    return;
                }
            };
        let open: std::collections::HashSet<String> = match c.dir.open_transfers_from(id) {
            Ok(rows) => rows.into_iter().map(|t| t.subscriber).collect(),
            Err(e) => {
                tracing::warn!("[shard {id}] its connections are closed with no notice: {e}");
                return;
            }
        };
        for (sid, _) in shard.subscriber_ids() {
            let moving_here = self
                .transferring
                .lock()
                .unwrap()
                .contains_key(&(id.to_string(), sid.as_str().to_string()));
            if open.contains(sid.as_str()) || moving_here {
                continue;
            }
            let notice = match finished.get(sid.as_str()) {
                Some(t) => {
                    let to = self.ticket_for(t);
                    pylon_realtime::wire::TransferNotice {
                        shard: to.shard,
                        ticket: to.ticket,
                    }
                }
                None => {
                    let auth = shard.subscriber_auth(&sid).unwrap_or_default();
                    let ticket = auth.ticket.as_ref();
                    pylon_realtime::wire::TransferNotice {
                        shard: id.to_string(),
                        ticket: crate::shard_tickets::mint(
                            id,
                            sid.as_str(),
                            // The ticket's user first: a client with a
                            // ticket and no session has only that.
                            ticket
                                .and_then(|t| t.user_id.clone())
                                .or_else(|| auth.user_id.clone()),
                            ticket
                                .map(|t| t.claims.clone())
                                .unwrap_or(serde_json::Value::Null),
                            Some(TICKET_TTL_SECS),
                        ),
                    }
                }
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
        // Ending here: its release deletes any placement of the id on this
        // machine, so it is not placed here until then.
        if self.ending.lock().unwrap().contains(id) {
            return Err(refuse(
                "SHARD_UNAVAILABLE",
                format!("shard {id} is ending on this machine; try again shortly"),
            ));
        }
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
    /// time. First every shard is paused and saved (a kill during the
    /// hand-overs loses no state); then each goes over the network. The
    /// ones left over are returned.
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
        let ready: Vec<bool> = held
            .iter()
            .map(|(id, epoch, shard, _)| self.pause_and_save(c, id, *epoch, shard))
            .collect();
        let mut left = Vec::new();
        for ((id, epoch, shard, saves), ready) in held.into_iter().zip(ready) {
            // A shard with no saved state starts from `init` there; its
            // clients still reconnect at once.
            if ready && Instant::now() < deadline && self.hand_over(c, &id, epoch, &shard, deadline)
            {
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
