//! Moving a player between shards (issue #31).
//!
//! The source's module removes the player's entity (`transfer_out`), the
//! target's module adds it (`transfer_in`), and the client gets a transfer
//! frame with a ticket for the target. When the target refuses, the source
//! takes the player back.
//!
//! On several machines a row in the directory holds the player while it is
//! in flight, and decides where it ends up (see
//! [`crate::shard_cluster::Transfer`]):
//!
//! 1. The source removes the player, then in one transaction saves its own
//!    state (without the player) and writes the row, status `out`. A crash
//!    after this restarts the source without the player; the row has it.
//! 2. The target adds the player, then in one transaction takes the row
//!    from `out` to `in` and saves its own state (with the player).
//! 3. When the target refuses, cannot be reached, or loses the race, the
//!    source adds the player back and takes the row from `out` to `back`.
//!
//! Steps 2 and 3 are compare-and-sets on the row, so exactly one shard ends
//! up with the player. A shard that added the player and then lost the
//! compare-and-set removes it again.
//!
//! Every step works on one run of a shard (an [`Instance`]: its lease epoch
//! and a number unique to the run), checked under the save lock. A step
//! whose shard was fenced or restarted meanwhile changes nothing, and every
//! map entry a step removes must name the same run, so a slow call never
//! touches the run that replaced its shard.
//!
//! After a step fails, the row is read with [`PgShardDirectory::confirm_status`],
//! which waits out a transaction still in flight, so a commit whose reply
//! was lost is counted. When the directory cannot answer, the step is
//! "unsettled": the shard keeps what it has, and each round settles it from
//! the row. Moves not finished are "waiting": each round reads the row and
//! finishes them, and until then the player's inputs to the source are
//! refused. A source started from saved state finishes its moves still
//! `out` in its first round, with the hand-offs in place before it starts;
//! a row whose source was stopped is finished by the target's machine.
//!
//! [`PgShardDirectory::confirm_status`]: crate::shard_cluster::PgShardDirectory::confirm_status

use std::sync::Arc;

use pylon_realtime::{Shard, ShardAuth, SubscriberId};

use super::{validate_shard_id, WasmShardHost, WasmSim};
use crate::shard_cluster::{self, PgShardDirectory, RemoteOp, RemoteReply, Settle, Transfer};

/// A `out` row this old is finished by the source's (or, when the source
/// was stopped, the target's) machine.
pub(super) const STALE_TRANSFER_MS: i64 = 15_000;
/// How long the ticket for the target is valid, and how long the source
/// repeats the transfer notice to a connection that comes back.
const TICKET_TTL_SECS: u64 = 600;
/// A row whose source was stopped and whose target refuses the player is
/// offered again this often, and abandoned (status `dropped`, state kept)
/// after [`ABANDON_AFTER_MS`].
const OWNERLESS_RETRY: std::time::Duration = std::time::Duration::from_secs(60);
const ABANDON_AFTER_MS: i64 = 30 * 60 * 1000;
/// How often a machine deletes settled rows.
const PRUNE_EVERY: std::time::Duration = std::time::Duration::from_secs(300);
/// A move a module asked for that failed is not tried again for this long.
const REQUEST_BACKOFF: std::time::Duration = std::time::Duration::from_secs(5);
/// How many module-requested moves wait for the transfer thread.
pub(super) const REQUEST_QUEUE: usize = 1024;

/// One run of a shard on this machine: the lease epoch it runs under (0
/// without a directory) and a number unique to the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Instance {
    pub(super) epoch: i64,
    pub(super) serial: u64,
}

/// A step whose commit is unknown (see the module docs). It exists only
/// while its shard's run `at` is in the state the step left it in: the
/// source without the player (Begin), the target with a copy (Accept), the
/// source with a copy (Back).
#[derive(Debug, Clone)]
pub(super) struct Unsettled {
    pub(super) step: Step,
    pub(super) t: Transfer,
    pub(super) at: Instance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Step {
    /// The source removed the player; the row may not exist.
    Begin,
    /// The target added the player; the row may not be `in`.
    Accept,
    /// The source took the player back; the row may not be `back`.
    Back,
}

/// Which shard an unsettled step is about: a source and a target on one
/// machine can each have one for the same transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Role {
    Source,
    Target,
}

impl Step {
    fn role(self) -> Role {
        match self {
            Self::Accept => Role::Target,
            Self::Begin | Self::Back => Role::Source,
        }
    }
}

/// A source's moves to finish when it starts: read before it starts, so the
/// hand-offs are in place before any input reaches it.
pub(super) struct ResumePlan {
    open: Vec<Transfer>,
    recent: Vec<(Transfer, i64)>,
}

/// What the row says once nothing is in flight on it: its status, `None`
/// when it has none, or `Err` when the directory cannot say.
fn confirm(dir: &PgShardDirectory, id: &str) -> Result<Option<String>, String> {
    let mut last = String::new();
    for attempt in 0..3 {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        match dir.confirm_status(id) {
            Ok(status) => return Ok(status),
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// How taking the player back ended.
enum Back {
    /// The source has the player.
    Returned,
    /// The target took it first.
    TargetHas,
}

/// A finished transfer: the player is in `shard`; the client connects there
/// with `ticket`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Transferred {
    pub shard: String,
    pub ticket: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TransferError {
    /// A shard is not running (here or, in a cluster, anywhere).
    NotFound(String),
    InvalidId(String),
    /// The source has no entity for the subscriber.
    NoPlayer(String),
    /// A module does not transfer players.
    Unsupported(String),
    /// A module refused. The player is in the source.
    Refused(String),
    /// The subscriber is already being moved.
    Busy(String),
    Cluster(String),
}

impl TransferError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound(_) => "SHARD_NOT_FOUND",
            Self::InvalidId(_) => "SHARD_ID_INVALID",
            Self::NoPlayer(_) => "SHARD_TRANSFER_NO_PLAYER",
            Self::Unsupported(_) => "SHARD_TRANSFER_UNSUPPORTED",
            Self::Refused(_) => "SHARD_TRANSFER_REFUSED",
            Self::Busy(_) => "SHARD_TRANSFER_BUSY",
            Self::Cluster(_) => "SHARD_CLUSTER_ERROR",
        }
    }

    fn from_code(code: &str, message: String) -> Self {
        match code {
            "SHARD_NOT_FOUND" => Self::NotFound(message),
            "SHARD_ID_INVALID" => Self::InvalidId(message),
            "SHARD_TRANSFER_NO_PLAYER" => Self::NoPlayer(message),
            "SHARD_TRANSFER_UNSUPPORTED" => Self::Unsupported(message),
            "SHARD_TRANSFER_REFUSED" => Self::Refused(message),
            "SHARD_TRANSFER_BUSY" => Self::Busy(message),
            _ => Self::Cluster(message),
        }
    }
}

impl std::fmt::Display for TransferError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(id) => write!(f, "shard \"{id}\" is not running"),
            Self::NoPlayer(sid) => write!(f, "subscriber \"{sid}\" has no entity in the shard"),
            Self::InvalidId(m)
            | Self::Unsupported(m)
            | Self::Refused(m)
            | Self::Busy(m)
            | Self::Cluster(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for TransferError {}

/// The ticket fields a transfer carries to the target.
#[derive(serde::Serialize, serde::Deserialize, Default)]
struct TicketAuth {
    user_id: Option<String>,
    #[serde(default)]
    claims: serde_json::Value,
}

fn new_transfer_id() -> String {
    (0..16)
        .map(|_| format!("{:02x}", rand::random::<u8>()))
        .collect()
}

impl WasmShardHost {
    /// The current run of shard `id` here, when it is running (and, in a
    /// cluster, held).
    pub(super) fn instance(&self, id: &str) -> Option<Instance> {
        let serial = *self.instances.lock().unwrap().get(id)?;
        if !self.registry.get(id).is_some_and(|s| s.is_running()) {
            return None;
        }
        let epoch = match self.cluster.get() {
            Some(c) => *c.owned.lock().unwrap().get(id)?,
            None => 0,
        };
        Some(Instance { epoch, serial })
    }

    fn is_current(&self, id: &str, at: Instance) -> bool {
        self.instance(id) == Some(at)
    }

    /// Move `subscriber`'s entity from shard `from` to shard `to`, possibly
    /// on another machine, and tell its connections to reconnect to `to`
    /// with a ticket carrying `claims`. On an error the player is in `from`
    /// (see the module docs).
    pub fn transfer(
        &self,
        from: &str,
        subscriber: &str,
        to: &str,
        claims: serde_json::Value,
    ) -> Result<Transferred, TransferError> {
        for id in [from, to] {
            validate_shard_id(id).map_err(|e| TransferError::InvalidId(e.to_string()))?;
        }
        if subscriber.is_empty() || subscriber.len() > 256 {
            return Err(TransferError::InvalidId(
                "a subscriber id is 1 to 256 bytes".into(),
            ));
        }
        if from == to {
            return Err(TransferError::Refused(
                "the source and the target are the same shard".into(),
            ));
        }
        let Some(source) = self.registry.get(from).filter(|s| s.is_running()) else {
            return self.transfer_elsewhere(from, subscriber, to, claims);
        };
        let at = self
            .instance(from)
            .ok_or_else(|| TransferError::NotFound(from.to_string()))?;
        // The target must exist before the player leaves the source.
        if self.registry.get(to).filter(|s| s.is_running()).is_none()
            && !matches!(
                self.locate(to),
                pylon_realtime::ShardLocation::Remote { .. }
            )
        {
            return Err(TransferError::NotFound(to.to_string()));
        }
        if !self.kind_transfers(from) {
            return Err(TransferError::Unsupported(format!(
                "shard \"{from}\"'s module does not transfer players"
            )));
        }
        let sid = SubscriberId::new(subscriber);
        // The user a ticket named, else the session's. Claims are the
        // caller's: claims issued for the source mean nothing on the target.
        let auth = source.subscriber_auth(&sid);
        let user_id = auth.as_ref().and_then(|a| {
            a.ticket
                .as_ref()
                .and_then(|t| t.user_id.clone())
                .or_else(|| a.user_id.clone())
        });
        let t = Transfer {
            id: new_transfer_id(),
            subscriber: subscriber.to_string(),
            from_shard: from.to_string(),
            to_shard: to.to_string(),
            state: Vec::new(),
            auth: serde_json::to_value(TicketAuth { user_id, claims }).unwrap_or_default(),
            status: "out".into(),
        };
        let t = self.begin(&source, at, &sid, t)?;
        // A live move: the round leaves its row alone.
        self.live.lock().unwrap().insert(t.id.clone());
        let done = self.deliver_and_settle(&source, at, &sid, &t);
        self.live.lock().unwrap().remove(&t.id);
        done
    }

    /// Step 1: take the player out of the source and record the transfer.
    fn begin(
        &self,
        source: &Arc<Shard<WasmSim>>,
        at: Instance,
        sid: &SubscriberId,
        mut t: Transfer,
    ) -> Result<Transfer, TransferError> {
        {
            let mut transferring = self.transferring.lock().unwrap();
            let key = (t.from_shard.clone(), t.subscriber.clone());
            if transferring.contains_key(&key) {
                return Err(TransferError::Busy(format!(
                    "subscriber \"{}\" is already moving",
                    t.subscriber
                )));
            }
            transferring.insert(key, at);
        }
        source.begin_hand_off(sid);
        let started = self.take_out(source, at, sid, &mut t);
        if started.is_err() && !self.held_elsewhere(&t.id) {
            source.cancel_hand_off(sid);
            self.end_transfer(&t, at);
        }
        started.map(|()| t)
    }

    fn take_out(
        &self,
        source: &Arc<Shard<WasmSim>>,
        at: Instance,
        sid: &SubscriberId,
        t: &mut Transfer,
    ) -> Result<(), TransferError> {
        let cluster = self.cluster.get();
        // Under the save lock: a periodic save captured with the player in
        // it must not land after the save without it.
        let _save = cluster.map(|c| c.save_lock.lock().unwrap());
        if !self.is_current(&t.from_shard, at) {
            return Err(TransferError::NotFound(t.from_shard.clone()));
        }
        let saves = self.saves_state(&t.from_shard);
        let auth = back_auth(t);
        source.with_state(|sim| {
            // The run can end (the module finished) while this waited for
            // its state.
            if !self.is_current(&t.from_shard, at) {
                return Err(TransferError::NotFound(t.from_shard.clone()));
            }
            let state = match sim.transfer_out(sid.as_str()) {
                Ok(Some(state)) => state,
                Ok(None) => return Err(TransferError::NoPlayer(t.subscriber.clone())),
                Err(e) => return Err(TransferError::Refused(e)),
            };
            t.state = state;
            let Some(c) = cluster else {
                return Ok(());
            };
            let recorded = sim
                .save()
                .map_err(TransferError::Cluster)
                .map(|s| s.filter(|_| saves))
                .and_then(|source_state| {
                    c.dir
                        .begin_transfer(t, &c.me.id, at.epoch, source_state.as_deref())
                        .map_err(TransferError::Cluster)
                });
            let put_back = |why: TransferError| {
                if let Err(e) = sim.transfer_in(sid.as_str(), &t.state, &auth, true) {
                    self.strand(t, at, &e);
                }
                Err(why)
            };
            match recorded {
                Ok(true) => Ok(()),
                // Not held (the lease lapsed): nothing was written.
                Ok(false) => put_back(TransferError::NotFound(t.from_shard.clone())),
                Err(e) => match confirm(&c.dir, &t.id) {
                    // The commit landed: carry on.
                    Ok(Some(_)) => Ok(()),
                    Ok(None) => put_back(e),
                    Err(_) => {
                        self.unsettle(Step::Begin, t, at);
                        Err(e)
                    }
                },
            }
        })
    }

    /// Steps 2 and 3. On an error the player is in the source, or (in a
    /// cluster) the move waits for a round to finish it from its row, with
    /// the player's inputs to the source still refused.
    fn deliver_and_settle(
        &self,
        source: &Arc<Shard<WasmSim>>,
        at: Instance,
        sid: &SubscriberId,
        t: &Transfer,
    ) -> Result<Transferred, TransferError> {
        let e = match self.deliver(t) {
            Ok(()) => return Ok(self.complete(source, at, sid, t)),
            Err(e) => e,
        };
        match self.take_back(source, at, sid, t) {
            Ok(Back::Returned) => {
                self.returned(source, at, sid, t);
                Err(e)
            }
            // The target has it after all (its reply was lost).
            Ok(Back::TargetHas) => Ok(self.complete(source, at, sid, t)),
            Err(x) => {
                self.wait(t, at);
                // Without a directory the player is kept (stranded) and the
                // sweep offers it back; its inputs stay refused until then.
                Err(x)
            }
        }
    }

    /// The target has the player: tell the source's connections (when the
    /// run is still the current one; a newer run finishes the move itself),
    /// and end the move for run `at`.
    fn complete(
        &self,
        source: &Shard<WasmSim>,
        at: Instance,
        sid: &SubscriberId,
        t: &Transfer,
    ) -> Transferred {
        let done = self.ticket_for(t);
        if self.is_current(&t.from_shard, at) {
            source.hand_off(
                sid,
                &pylon_realtime::wire::TransferNotice {
                    shard: done.shard.clone(),
                    ticket: done.ticket.clone(),
                },
                std::time::Duration::from_secs(TICKET_TTL_SECS),
            );
            tracing::info!(
                "[shard {}] subscriber {} moved to shard {}",
                t.from_shard,
                t.subscriber,
                t.to_shard
            );
        }
        self.unwait(t, at);
        self.end_transfer(t, at);
        done
    }

    /// The source has the player again: its inputs are accepted again.
    fn returned(&self, source: &Shard<WasmSim>, at: Instance, sid: &SubscriberId, t: &Transfer) {
        source.cancel_hand_off(sid);
        self.unwait(t, at);
        self.end_transfer(t, at);
    }

    /// Leave the move for a round to finish from its row. Only for the
    /// current run: a newer run registered the move itself when it started.
    fn wait(&self, t: &Transfer, at: Instance) {
        if self.cluster.get().is_none() {
            return;
        }
        if self.is_current(&t.from_shard, at) {
            self.waiting
                .lock()
                .unwrap()
                .insert(t.id.clone(), (t.clone(), at));
        } else {
            self.end_transfer(t, at);
        }
    }

    fn unwait(&self, t: &Transfer, at: Instance) {
        let mut waiting = self.waiting.lock().unwrap();
        if waiting.get(&t.id).is_some_and(|(_, w)| *w == at) {
            waiting.remove(&t.id);
        }
    }

    /// Step 2, here or on the target's machine.
    fn deliver(&self, t: &Transfer) -> Result<(), TransferError> {
        if self
            .registry
            .get(&t.to_shard)
            .is_some_and(|s| s.is_running())
        {
            return self.accept(t);
        }
        let pylon_realtime::ShardLocation::Remote {
            machine_id,
            address: Some(address),
            ..
        } = self.locate(&t.to_shard)
        else {
            return Err(TransferError::NotFound(t.to_shard.clone()));
        };
        let op = RemoteOp::TransferIn { id: t.id.clone() };
        match shard_cluster::call(&machine_id, &address, &op) {
            Ok(RemoteReply::Ok(_)) => Ok(()),
            Ok(RemoteReply::Err { code, message }) => Err(TransferError::from_code(&code, message)),
            // The target may or may not have taken it: the row decides.
            Err(e) => Err(TransferError::Cluster(e)),
        }
    }

    /// Add the player in `t` to its target, which runs here. Safe to call
    /// again for one transfer: the row is read first, under the save lock,
    /// and a copy an earlier attempt left in this run is not added twice.
    pub(super) fn accept(&self, t: &Transfer) -> Result<(), TransferError> {
        let not_found = || TransferError::NotFound(t.to_shard.clone());
        let target = self
            .registry
            .get(&t.to_shard)
            .filter(|s| s.is_running())
            .ok_or_else(not_found)?;
        let at = self.instance(&t.to_shard).ok_or_else(not_found)?;
        if !self.kind_transfers(&t.to_shard) {
            return Err(TransferError::Unsupported(format!(
                "shard \"{}\"'s module does not accept players",
                t.to_shard
            )));
        }
        let sid = SubscriberId::new(t.subscriber.as_str());
        let auth = arrival_auth(t);
        let Some(c) = self.cluster.get() else {
            target
                .with_state(|sim| sim.transfer_in(&t.subscriber, &t.state, &auth, false))
                .map_err(TransferError::Refused)?;
            self.arrived(&target, &sid, t);
            return Ok(());
        };
        let _save = c.save_lock.lock().unwrap();
        if !self.is_current(&t.to_shard, at) {
            return Err(not_found());
        }
        let key = (t.id.clone(), Role::Target);
        let has_copy = self.has_copy(&key, at);
        let status = c
            .dir
            .transfer(&t.id)
            .map_err(TransferError::Cluster)?
            .map(|r| r.status);
        match status.as_deref() {
            Some("out") => {}
            // Ours: the copy here is the player.
            Some("in") => {
                self.unsettled.lock().unwrap().remove(&key);
                if has_copy {
                    self.arrived(&target, &sid, t);
                }
                return Ok(());
            }
            other => {
                if has_copy {
                    let _ = target.with_state(|sim| sim.transfer_out(&t.subscriber));
                }
                self.unsettled.lock().unwrap().remove(&key);
                return Err(TransferError::Refused(match other {
                    Some(s) => format!("transfer {} is already {s}", t.id),
                    None => format!("no transfer {}", t.id),
                }));
            }
        }
        let saves = self.saves_state(&t.to_shard);
        // (outcome, whether the target keeps a copy the row may not count)
        let (outcome, unknown) = target.with_state(|sim| {
            if !self.is_current(&t.to_shard, at) {
                return (Err(not_found()), false);
            }
            if !has_copy {
                if let Err(e) = sim.transfer_in(&t.subscriber, &t.state, &auth, false) {
                    return (Err(TransferError::Refused(e)), false);
                }
            }
            let settled = sim
                .save()
                .map_err(TransferError::Cluster)
                .map(|s| s.filter(|_| saves))
                .and_then(|target_state| {
                    c.dir
                        .accept_transfer(
                            &t.id,
                            &t.to_shard,
                            &c.me.id,
                            at.epoch,
                            target_state.as_deref(),
                        )
                        .map_err(TransferError::Cluster)
                });
            let undo = || {
                if let Err(e) = sim.transfer_out(&t.subscriber) {
                    tracing::error!(
                        "[shard {}] could not remove subscriber {} after losing its transfer: {e}",
                        t.to_shard,
                        t.subscriber
                    );
                }
            };
            match settled {
                Ok(Settle::Done) => (Ok(()), false),
                Ok(Settle::Taken) => {
                    undo();
                    let why = format!("transfer {} was settled by the source", t.id);
                    (Err(TransferError::Refused(why)), false)
                }
                Ok(Settle::NotHeld) => {
                    undo();
                    let why = format!("this machine no longer holds shard \"{}\"", t.to_shard);
                    (Err(TransferError::Cluster(why)), false)
                }
                Err(e) => match confirm(&c.dir, &t.id) {
                    Ok(Some(s)) if s == "in" => (Ok(()), false),
                    Ok(_) => {
                        undo();
                        (Err(e), false)
                    }
                    Err(_) => (Err(e), true),
                },
            }
        });
        if unknown {
            self.unsettle(Step::Accept, t, at);
        } else {
            self.unsettled.lock().unwrap().remove(&key);
        }
        if outcome.is_ok() {
            self.arrived(&target, &sid, t);
        }
        outcome
    }

    /// The player is in the target now.
    fn arrived(&self, target: &Shard<WasmSim>, sid: &SubscriberId, t: &Transfer) {
        // It no longer moves away from here, and the shard is not idle.
        target.forget_move(sid);
        self.idle_since.lock().unwrap().remove(&t.to_shard);
    }

    /// Step 3: give the player back to the source's run `at`. The row is
    /// read first, under the save lock: a row another attempt already
    /// settled is not settled again.
    fn take_back(
        &self,
        source: &Arc<Shard<WasmSim>>,
        at: Instance,
        sid: &SubscriberId,
        t: &Transfer,
    ) -> Result<Back, TransferError> {
        let auth = back_auth(t);
        let Some(c) = self.cluster.get() else {
            if let Err(e) =
                source.with_state(|sim| sim.transfer_in(sid.as_str(), &t.state, &auth, true))
            {
                self.strand(t, at, &e);
                return Err(TransferError::Cluster(format!(
                    "the source refused the player back: {e}"
                )));
            }
            return Ok(Back::Returned);
        };
        let _save = c.save_lock.lock().unwrap();
        if !self.is_current(&t.from_shard, at) {
            // Fenced or restarted meanwhile: the run that holds the source
            // now finishes the move from its row.
            return Err(TransferError::Cluster(format!(
                "shard \"{}\" restarted during the move",
                t.from_shard
            )));
        }
        let key = (t.id.clone(), Role::Source);
        let has_copy = self.has_copy(&key, at);
        let status = c
            .dir
            .transfer(&t.id)
            .map_err(TransferError::Cluster)?
            .map(|r| r.status);
        match status.as_deref() {
            Some("out") => {}
            // Settled for the source (by this or another attempt): it has
            // the player.
            Some("back") => {
                self.unsettled.lock().unwrap().remove(&key);
                return Ok(Back::Returned);
            }
            Some("in") => {
                if has_copy {
                    let _ = source.with_state(|sim| sim.transfer_out(sid.as_str()));
                }
                self.unsettled.lock().unwrap().remove(&key);
                return Ok(Back::TargetHas);
            }
            _ => {
                return Err(TransferError::Cluster(format!("no transfer {}", t.id)));
            }
        }
        let saves = self.saves_state(&t.from_shard);
        let (outcome, unknown) = source.with_state(|sim| {
            if !self.is_current(&t.from_shard, at) {
                let why = format!("shard \"{}\" ended during the move", t.from_shard);
                return (Err(TransferError::Cluster(why)), false);
            }
            if !has_copy {
                if let Err(e) = sim.transfer_in(sid.as_str(), &t.state, &auth, true) {
                    // The row keeps the player; a later round offers it
                    // again.
                    tracing::error!(
                        "[shard {}] refused subscriber {} back from transfer {}: {e}",
                        t.from_shard,
                        t.subscriber,
                        t.id
                    );
                    let why = format!("the source refused the player back: {e}");
                    return (Err(TransferError::Cluster(why)), false);
                }
            }
            let settled = sim
                .save()
                .map(|s| s.filter(|_| saves))
                .map_err(TransferError::Cluster)
                .and_then(|source_state| {
                    c.dir
                        .return_transfer(
                            &t.id,
                            &t.from_shard,
                            &c.me.id,
                            at.epoch,
                            source_state.as_deref(),
                        )
                        .map_err(TransferError::Cluster)
                });
            let undo = || {
                let _ = sim.transfer_out(sid.as_str());
            };
            match settled {
                Ok(Settle::Done) => (Ok(Back::Returned), false),
                // The row was `out` under the save lock a moment ago; the
                // target settled it since.
                Ok(Settle::Taken) => {
                    undo();
                    (Ok(Back::TargetHas), false)
                }
                Ok(Settle::NotHeld) => {
                    undo();
                    let why = format!("this machine no longer holds shard \"{}\"", t.from_shard);
                    (Err(TransferError::Cluster(why)), false)
                }
                Err(e) => match confirm(&c.dir, &t.id) {
                    Ok(Some(s)) if s == "back" => (Ok(Back::Returned), false),
                    Ok(Some(s)) if s == "in" => {
                        undo();
                        (Ok(Back::TargetHas), false)
                    }
                    Ok(_) => {
                        undo();
                        (Err(e), false)
                    }
                    Err(_) => (Err(e), true),
                },
            }
        });
        if unknown {
            self.unsettle(Step::Back, t, at);
        } else {
            self.unsettled.lock().unwrap().remove(&key);
        }
        outcome
    }

    /// True when an unsettled step left a copy in run `at` for `key`.
    fn has_copy(&self, key: &(String, Role), at: Instance) -> bool {
        let mut unsettled = self.unsettled.lock().unwrap();
        match unsettled.get(key) {
            Some(u) if u.at == at && u.step != Step::Begin => true,
            // An entry from an earlier run: its copy stopped with that run.
            Some(u) if u.at != at => {
                unsettled.remove(key);
                false
            }
            _ => false,
        }
    }

    /// One round's transfer work, on a machine in a cluster: settle steps
    /// whose outcome was unknown, finish each waiting move and each row
    /// `out` for 15 s whose source runs here, and finish rows whose source
    /// was stopped and whose target runs here.
    pub(super) fn settle_transfers(&self, epoch: i64) {
        let Some(c) = self.cluster.get() else { return };
        self.resolve_unsettled();
        let mut rows: Vec<(Transfer, Option<Instance>)> = self
            .waiting
            .lock()
            .unwrap()
            .values()
            .map(|(t, at)| (t.clone(), Some(*at)))
            .collect();
        match c.dir.stale_transfers(&c.me.id, epoch, STALE_TRANSFER_MS) {
            Ok(stale) => rows.extend(stale.into_iter().map(|t| (t, None))),
            Err(e) => tracing::warn!("[shards] reading unfinished transfers failed: {e}"),
        }
        let mut seen = std::collections::HashSet::new();
        for (t, registered) in rows {
            if !seen.insert(t.id.clone()) {
                continue;
            }
            // A live call, or a step still unknown, owns it for now.
            if self.live.lock().unwrap().contains(&t.id) || self.is_unsettled(&t.id) {
                continue;
            }
            let (Some(source), Some(at)) = (
                self.registry.get(&t.from_shard).filter(|s| s.is_running()),
                self.instance(&t.from_shard),
            ) else {
                // Its source stopped or moved: whoever holds it next
                // finishes the move.
                if let Some(old) = registered {
                    self.unwait(&t, old);
                    self.end_transfer(&t, old);
                }
                continue;
            };
            if let Some(old) = registered.filter(|old| *old != at) {
                // Registered by an earlier run; the current run registered
                // it too when it started.
                self.unwait(&t, old);
                self.end_transfer(&t, old);
                continue;
            }
            let sid = SubscriberId::new(t.subscriber.as_str());
            self.transferring
                .lock()
                .unwrap()
                .insert((t.from_shard.clone(), t.subscriber.clone()), at);
            source.begin_hand_off(&sid);
            self.waiting
                .lock()
                .unwrap()
                .insert(t.id.clone(), (t.clone(), at));
            self.live.lock().unwrap().insert(t.id.clone());
            match confirm(&c.dir, &t.id) {
                Ok(Some(s)) if s == "in" => {
                    self.complete(&source, at, &sid, &t);
                }
                Ok(Some(s)) if s == "back" => self.returned(&source, at, &sid, &t),
                Ok(Some(_)) => {
                    tracing::info!(
                        "[shard {}] finishing transfer {} of subscriber {} to shard {}",
                        t.from_shard,
                        t.id,
                        t.subscriber,
                        t.to_shard
                    );
                    if let Err(e) = self.deliver_and_settle(&source, at, &sid, &t) {
                        tracing::warn!("[shard {}] transfer {}: {e}", t.from_shard, t.id);
                    }
                }
                // No row: nothing to finish.
                Ok(None) => self.returned(&source, at, &sid, &t),
                // Unknown: next round.
                Err(_) => {}
            }
            self.live.lock().unwrap().remove(&t.id);
        }
        // Rows whose source was stopped: the target takes the player. One
        // it refuses is offered again each minute, and abandoned after 30.
        match c
            .dir
            .ownerless_transfers(&c.me.id, epoch, STALE_TRANSFER_MS)
        {
            Ok(rows) => {
                let now = std::time::Instant::now();
                self.ownerless_retry
                    .lock()
                    .unwrap()
                    .retain(|_, next| *next > now);
                for (t, age_ms) in rows {
                    if self.live.lock().unwrap().contains(&t.id)
                        || self.is_unsettled(&t.id)
                        || self.ownerless_retry.lock().unwrap().contains_key(&t.id)
                    {
                        continue;
                    }
                    match self.accept(&t) {
                        Ok(()) => tracing::info!(
                            "[shard {}] took subscriber {} from stopped shard {}",
                            t.to_shard,
                            t.subscriber,
                            t.from_shard
                        ),
                        Err(e @ (TransferError::Refused(_) | TransferError::Unsupported(_)))
                            if age_ms > ABANDON_AFTER_MS =>
                        {
                            if c.dir.abandon_transfer(&t.id).unwrap_or(false) {
                                tracing::error!(
                                    "[shard {}] refused subscriber {} from stopped shard {} for 30 minutes ({e}); transfer {} abandoned, its state kept in the row for a week",
                                    t.to_shard,
                                    t.subscriber,
                                    t.from_shard,
                                    t.id
                                );
                            }
                        }
                        Err(e) => {
                            tracing::warn!(
                                "[shard {}] transfer {} from stopped shard {}: {e}; trying again in a minute",
                                t.to_shard,
                                t.id,
                                t.from_shard
                            );
                            self.ownerless_retry
                                .lock()
                                .unwrap()
                                .insert(t.id.clone(), now + OWNERLESS_RETRY);
                        }
                    }
                }
            }
            Err(e) => tracing::warn!("[shards] reading transfers from stopped shards failed: {e}"),
        }
        let due = {
            let mut last = self.last_prune.lock().unwrap();
            let due = last.is_none_or(|at| at.elapsed() >= PRUNE_EVERY);
            if due {
                *last = Some(std::time::Instant::now());
            }
            due
        };
        if due {
            let _ = c.dir.prune_transfers();
        }
    }

    /// What a source must do when it starts from saved state: its moves
    /// still `out`, and its moves that ended recently (to repeat to
    /// connections that come back). Read before it starts; an error fails
    /// the start, which a later round tries again.
    pub(super) fn resume_plan(&self, shard: &str) -> Result<ResumePlan, String> {
        let Some(c) = self.cluster.get() else {
            return Ok(ResumePlan {
                open: Vec::new(),
                recent: Vec::new(),
            });
        };
        Ok(ResumePlan {
            open: c.dir.open_transfers_from(shard)?,
            recent: c
                .dir
                .recent_moves_from(shard, TICKET_TTL_SECS as i64 * 1000)?,
        })
    }

    /// Put `plan` in place on the new run `at` of `shard`, before it starts:
    /// the players of open moves cannot act in it, and their moves wait for
    /// the first round; recent moves are repeated to connections that come
    /// back.
    pub(super) fn apply_resume_plan(
        &self,
        shard: &Shard<WasmSim>,
        at: Instance,
        plan: &ResumePlan,
    ) {
        for t in &plan.open {
            shard.begin_hand_off(&SubscriberId::new(t.subscriber.as_str()));
            self.transferring
                .lock()
                .unwrap()
                .insert((t.from_shard.clone(), t.subscriber.clone()), at);
            self.waiting
                .lock()
                .unwrap()
                .insert(t.id.clone(), (t.clone(), at));
        }
        for (t, age_ms) in &plan.recent {
            let age = (*age_ms).max(0) as u64 / 1000;
            let left = TICKET_TTL_SECS.saturating_sub(age);
            if left == 0 {
                continue;
            }
            let done = self.ticket_for(t);
            shard.remember_move(
                &SubscriberId::new(t.subscriber.as_str()),
                &pylon_realtime::wire::TransferNotice {
                    shard: done.shard,
                    ticket: done.ticket,
                },
                std::time::Duration::from_secs(left),
                crate::shard_tickets::unix_now().saturating_sub(age),
            );
        }
    }

    /// Settle each unknown step from its row. A step whose run is no longer
    /// the current one is dropped: its copy (or gap) ended with the run,
    /// and the shard's next run works from the row.
    pub(super) fn resolve_unsettled(&self) {
        let Some(c) = self.cluster.get() else { return };
        let steps: Vec<Unsettled> = self.unsettled.lock().unwrap().values().cloned().collect();
        for u in steps {
            let t = u.t.clone();
            let key = (t.id.clone(), u.step.role());
            let shard = match u.step.role() {
                Role::Source => &t.from_shard,
                Role::Target => &t.to_shard,
            };
            if !self.is_current(shard, u.at) {
                self.unsettled.lock().unwrap().remove(&key);
                if u.step.role() == Role::Source {
                    self.end_transfer(&t, u.at);
                }
                continue;
            }
            let sid = SubscriberId::new(t.subscriber.as_str());
            match u.step {
                Step::Begin => {
                    let Some(source) = self.registry.get(&t.from_shard) else {
                        continue;
                    };
                    let status = match confirm(&c.dir, &t.id) {
                        Ok(s) => s,
                        Err(_) => continue,
                    };
                    self.unsettled.lock().unwrap().remove(&key);
                    match status {
                        // The row exists: finish the move from it.
                        Some(_) => self.wait(&t, u.at),
                        // It never landed: the player goes back.
                        None => {
                            let auth = back_auth(&t);
                            match source.with_state(|sim| {
                                sim.transfer_in(&t.subscriber, &t.state, &auth, true)
                            }) {
                                Ok(()) => self.returned(&source, u.at, &sid, &t),
                                Err(e) => self.strand(&t, u.at, &e),
                            }
                        }
                    }
                }
                // `accept` sees the copy and settles from the row.
                Step::Accept => {
                    let _ = self.accept(&t);
                }
                Step::Back => {
                    let Some(source) = self.registry.get(&t.from_shard) else {
                        continue;
                    };
                    match self.take_back(&source, u.at, &sid, &t) {
                        Ok(Back::Returned) => self.returned(&source, u.at, &sid, &t),
                        Ok(Back::TargetHas) => {
                            self.complete(&source, u.at, &sid, &t);
                        }
                        Err(_) => self.wait(&t, u.at),
                    }
                }
            }
        }
    }

    fn unsettle(&self, step: Step, t: &Transfer, at: Instance) {
        tracing::warn!(
            "[shard {}] transfer {} of subscriber {}: the directory did not confirm the step; settling it from the row",
            t.from_shard,
            t.id,
            t.subscriber
        );
        let key = (t.id.clone(), step.role());
        self.unsettled.lock().unwrap().insert(
            key,
            Unsettled {
                step,
                t: t.clone(),
                at,
            },
        );
    }

    /// True when a step on shard `id` (as source or target) is unsettled.
    pub(super) fn unsettled_on(&self, id: &str) -> bool {
        self.unsettled
            .lock()
            .unwrap()
            .values()
            .any(|u| match u.step.role() {
                Role::Source => u.t.from_shard == id,
                Role::Target => u.t.to_shard == id,
            })
    }

    fn is_unsettled(&self, id: &str) -> bool {
        self.unsettled.lock().unwrap().keys().any(|(i, _)| i == id)
    }

    /// True when something other than the caller holds the player of
    /// transfer `id`: an unknown step, or the stranded list.
    fn held_elsewhere(&self, id: &str) -> bool {
        self.is_unsettled(id)
            || self
                .stranded
                .lock()
                .unwrap()
                .iter()
                .any(|(t, _)| t.id == id)
    }

    /// True when shard `id` has a move in progress or waiting: it is not
    /// idle, and its placement must stay.
    pub(super) fn moving_out(&self, id: &str) -> bool {
        self.transferring
            .lock()
            .unwrap()
            .keys()
            .any(|(s, _)| s == id)
            || self
                .waiting
                .lock()
                .unwrap()
                .values()
                .any(|(t, _)| t.from_shard == id)
    }

    /// The source is on another machine: that machine moves the player.
    fn transfer_elsewhere(
        &self,
        from: &str,
        subscriber: &str,
        to: &str,
        claims: serde_json::Value,
    ) -> Result<Transferred, TransferError> {
        let pylon_realtime::ShardLocation::Remote {
            machine_id,
            address: Some(address),
            ..
        } = self.locate(from)
        else {
            return Err(TransferError::NotFound(from.to_string()));
        };
        let op = RemoteOp::Transfer {
            from: from.to_string(),
            subscriber: subscriber.to_string(),
            to: to.to_string(),
            claims,
        };
        match shard_cluster::call(&machine_id, &address, &op) {
            Ok(RemoteReply::Ok(v)) => serde_json::from_value(v)
                .map_err(|e| TransferError::Cluster(format!("bad reply from {machine_id}: {e}"))),
            Ok(RemoteReply::Err { code, message }) => Err(TransferError::from_code(&code, message)),
            Err(e) => Err(TransferError::Cluster(e)),
        }
    }

    /// A ticket for the target, for the user the source knew.
    fn ticket_for(&self, t: &Transfer) -> Transferred {
        let auth: TicketAuth = serde_json::from_value(t.auth.clone()).unwrap_or_default();
        Transferred {
            shard: t.to_shard.clone(),
            ticket: crate::shard_tickets::mint(
                &t.to_shard,
                &t.subscriber,
                auth.user_id,
                auth.claims,
                Some(TICKET_TTL_SECS),
            ),
        }
    }

    /// Keep a player no shard would take back, and no directory row holds,
    /// so the sweep offers it to the source again.
    fn strand(&self, t: &Transfer, at: Instance, why: &str) {
        tracing::error!(
            "[shard {}] refused subscriber {} back ({why}); the host keeps it and offers it again",
            t.from_shard,
            t.subscriber
        );
        self.stranded.lock().unwrap().push((t.clone(), at));
    }

    /// Offer kept players back to their source shards (from the sweep). One
    /// whose source no longer runs is dropped, with an error log: nothing is
    /// left to return it to.
    pub(super) fn retry_stranded(&self) {
        let kept = std::mem::take(&mut *self.stranded.lock().unwrap());
        for (t, at) in kept {
            let Some(source) = self.registry.get(&t.from_shard).filter(|s| s.is_running()) else {
                tracing::error!(
                    "[shard {}] stopped with subscriber {} still waiting to come back; dropped",
                    t.from_shard,
                    t.subscriber
                );
                self.end_transfer(&t, at);
                continue;
            };
            let auth = back_auth(&t);
            match source.with_state(|sim| sim.transfer_in(&t.subscriber, &t.state, &auth, true)) {
                Ok(()) => {
                    tracing::info!(
                        "[shard {}] took subscriber {} back",
                        t.from_shard,
                        t.subscriber
                    );
                    self.returned(&source, at, &SubscriberId::new(t.subscriber.as_str()), &t);
                }
                Err(_) => self.stranded.lock().unwrap().push((t, at)),
            }
        }
    }

    fn kind_transfers(&self, shard: &str) -> bool {
        self.kind_of
            .read()
            .unwrap()
            .get(shard)
            .is_some_and(|k| self.kinds[k].transfers())
    }

    /// Release the move's hold on its player, when run `at` holds it.
    fn end_transfer(&self, t: &Transfer, at: Instance) {
        let mut transferring = self.transferring.lock().unwrap();
        let key = (t.from_shard.clone(), t.subscriber.clone());
        if transferring.get(&key) == Some(&at) {
            transferring.remove(&key);
        }
    }

    /// Run the transfers modules asked for after their ticks, one at a
    /// time. A player already moving is skipped, and a move that failed is
    /// not tried again for [`REQUEST_BACKOFF`].
    pub(super) fn run_requested_transfers(
        weak: std::sync::Weak<Self>,
        requests: std::sync::mpsc::Receiver<(String, String, String)>,
    ) {
        let mut failed: std::collections::HashMap<(String, String, String), std::time::Instant> =
            std::collections::HashMap::new();
        while let Ok(request) = requests.recv() {
            let Some(host) = weak.upgrade() else { return };
            if host.stopped.load(std::sync::atomic::Ordering::Acquire) {
                return;
            }
            let now = std::time::Instant::now();
            failed.retain(|_, until| *until > now);
            let (from, sid, to) = &request;
            let moving = host
                .transferring
                .lock()
                .unwrap()
                .contains_key(&(from.clone(), sid.clone()));
            if moving || failed.contains_key(&request) {
                continue;
            }
            if let Err(e) = host.transfer(from, sid, to, serde_json::Value::Null) {
                tracing::warn!(
                    "[shard {from}] moving subscriber {sid} to shard {to} failed ({}): {e}",
                    e.code()
                );
                failed.insert(request.clone(), now + REQUEST_BACKOFF);
            }
        }
    }
}

/// The auth a target's `transfer_in` sees: the user, and the ticket the
/// player gets for the target.
fn arrival_auth(t: &Transfer) -> ShardAuth {
    let auth: TicketAuth = serde_json::from_value(t.auth.clone()).unwrap_or_default();
    ShardAuth {
        user_id: auth.user_id.clone(),
        ticket: Some(pylon_realtime::ShardTicket {
            shard: t.to_shard.clone(),
            sid: t.subscriber.clone(),
            user_id: auth.user_id,
            exp: crate::shard_tickets::unix_now() + TICKET_TTL_SECS,
            iat: crate::shard_tickets::unix_now(),
            claims: auth.claims,
        }),
        ..Default::default()
    }
}

/// The auth the source sees when its own player comes back.
fn back_auth(t: &Transfer) -> ShardAuth {
    let auth: TicketAuth = serde_json::from_value(t.auth.clone()).unwrap_or_default();
    ShardAuth {
        user_id: auth.user_id,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shard_cluster::{MachineConfig, PgShardDirectory};
    use crate::shard_wasm::{WasmLimits, WasmShardKind};
    use pylon_realtime::{
        FrameKind, OutboundQueue, RawInput, ShardConfig, ShardError, SnapshotFormat,
    };
    use pylon_storage::pg_datastore::PgPool;
    use std::time::{Duration, Instant};

    fn saved(host: &WasmShardHost, shard: &str) -> serde_json::Value {
        let shard = host.registry.get(shard).expect("running");
        let saved = shard.with_state(|sim| sim.save()).unwrap().unwrap();
        serde_json::from_slice(&saved).unwrap()
    }

    fn hp(host: &WasmShardHost, shard: &str, sid: &str) -> i64 {
        saved(host, shard)[sid]["hp"].as_i64().unwrap()
    }

    fn has(host: &WasmShardHost, shard: &str, sid: &str) -> bool {
        !saved(host, shard)[sid].is_null()
    }

    fn user(sid: &str) -> ShardAuth {
        ShardAuth {
            user_id: Some(sid.into()),
            ..Default::default()
        }
    }

    fn join(shard: &Shard<WasmSim>, sid: &str) -> Result<u64, ShardError> {
        shard.push_input(
            SubscriberId::new(sid),
            RawInput::new(SnapshotFormat::Json, b"\"join\"".to_vec()),
            None,
        )
    }

    fn got_transfer(q: &OutboundQueue) -> bool {
        let mut found = false;
        while let Some(f) = q.pop() {
            found |= f.kind == FrameKind::Transfer;
        }
        found
    }

    struct Setup {
        host: Arc<WasmShardHost>,
        run: String,
        me: String,
        a: String,
        b: String,
    }

    fn setup(url: &str) -> Setup {
        let pool = PgPool::connect(url, 4, Duration::from_secs(5)).unwrap();
        let kind = WasmShardKind::compile(
            "zone",
            include_bytes!("../../../../examples/shard-arena/shards/zone.wasm"),
            ShardConfig::default(),
            WasmLimits::default(),
        )
        .unwrap();
        let host = WasmShardHost::new(vec![kind]);
        let run = pylon_cluster::new_instance_id();
        let me = format!("u-{run}");
        host.attach_cluster(
            PgShardDirectory::open(Arc::clone(&pool)).unwrap(),
            MachineConfig {
                id: me.clone(),
                address: None,
                capacity: 10,
                fly_instance: None,
            },
        );
        let (a, b) = (format!("a-{run}"), format!("b-{run}"));
        let deadline = Instant::now() + Duration::from_secs(10);
        while host
            .create_on("zone", &a, &serde_json::json!({}), Some(&me))
            .is_err()
        {
            assert!(Instant::now() < deadline, "no lease");
            std::thread::sleep(Duration::from_millis(100));
        }
        host.create_on("zone", &b, &serde_json::json!({}), Some(&me))
            .unwrap();
        Setup {
            host,
            run,
            me,
            a,
            b,
        }
    }

    fn state(hp: i64) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "x": 0, "hp": hp, "buffs": [], "cooldowns": {}
        }))
        .unwrap()
    }

    /// Each kind of step whose commit was unknown is settled from its row,
    /// and releases its player's hold when done.
    #[test]
    fn unknown_steps_are_settled_from_the_row() {
        let Ok(url) = std::env::var("PYLON_TEST_PG_URL") else {
            eprintln!("skipping: PYLON_TEST_PG_URL not set");
            return;
        };
        let _serial = crate::shard_cluster::tests::DIRECTORY_TESTS
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let Setup {
            host,
            run,
            me,
            a,
            b,
        } = setup(&url);
        let c = host.cluster.get().unwrap();
        let (at_a, at_b) = (host.instance(&a).unwrap(), host.instance(&b).unwrap());
        let epoch = at_a.epoch;
        let source = host.registry.get(&a).unwrap();
        let target = host.registry.get(&b).unwrap();
        let transfer = |id: &str, sid: &str| Transfer {
            id: format!("{id}-{run}"),
            subscriber: sid.into(),
            from_shard: a.clone(),
            to_shard: b.clone(),
            state: state(50),
            auth: serde_json::json!({}),
            status: "out".into(),
        };
        // What `begin` does to the source before a step: the player's hold.
        let hold = |t: &Transfer| {
            host.transferring
                .lock()
                .unwrap()
                .insert((a.clone(), t.subscriber.clone()), at_a);
            source.begin_hand_off(&SubscriberId::new(t.subscriber.as_str()));
        };
        let held = |sid: &str| {
            host.transferring
                .lock()
                .unwrap()
                .contains_key(&(a.clone(), sid.to_string()))
        };

        // Begin, and no row landed: the player goes back to the source, and
        // its inputs are accepted again.
        let t1 = transfer("t1", "p1");
        hold(&t1);
        host.unsettle(Step::Begin, &t1, at_a);
        host.resolve_unsettled();
        assert!(!host.is_unsettled(&t1.id));
        assert!(has(&host, &a, "p1") && !held("p1"));
        assert!(join(&source, "p1").is_ok());

        // Begin, and the row landed: the move is finished from it.
        let t2 = transfer("t2", "p2");
        hold(&t2);
        assert!(c.dir.begin_transfer(&t2, &me, epoch, None).unwrap());
        host.unsettle(Step::Begin, &t2, at_a);
        host.resolve_unsettled();
        assert!(host.waiting.lock().unwrap().contains_key(&t2.id));
        host.settle_transfers(epoch);
        assert_eq!(c.dir.transfer(&t2.id).unwrap().unwrap().status, "in");
        assert!(has(&host, &b, "p2") && !has(&host, &a, "p2"));
        assert!(!held("p2") && host.waiting.lock().unwrap().is_empty());

        // Accept, and the source took the player back meanwhile: the
        // target's copy goes.
        let t3 = transfer("t3", "p3");
        assert!(c.dir.begin_transfer(&t3, &me, epoch, None).unwrap());
        assert_eq!(
            c.dir.return_transfer(&t3.id, &a, &me, epoch, None).unwrap(),
            Settle::Done
        );
        target
            .with_state(|sim| sim.transfer_in("p3", &state(50), &arrival_auth(&t3), false))
            .unwrap();
        host.unsettle(Step::Accept, &t3, at_b);
        host.resolve_unsettled();
        assert!(!has(&host, &b, "p3") && !host.is_unsettled(&t3.id));

        // Accept, and the row is still out: the target settles it and keeps
        // its copy (changed since: hp 77), without adding the row's again.
        let t4 = transfer("t4", "p4");
        assert!(c.dir.begin_transfer(&t4, &me, epoch, None).unwrap());
        target
            .with_state(|sim| sim.transfer_in("p4", &state(77), &arrival_auth(&t4), false))
            .unwrap();
        host.unsettle(Step::Accept, &t4, at_b);
        // While the step is unknown, the target is not saved: its saved
        // state still matches the row.
        host.save_one(c, &b, epoch);
        let saved_b = c.dir.load_owned_state(&b, &me, epoch).unwrap().flatten();
        let saved_b: serde_json::Value = saved_b
            .map(|bytes| serde_json::from_slice(&bytes).unwrap())
            .unwrap_or_default();
        assert!(saved_b["p4"].is_null(), "saved during an unknown step");
        host.resolve_unsettled();
        assert_eq!(c.dir.transfer(&t4.id).unwrap().unwrap().status, "in");
        assert_eq!(hp(&host, &b, "p4"), 77);
        assert!(!host.is_unsettled(&t4.id));

        // Back, and the target took it first: the source's copy goes, and
        // the player's connection is sent on.
        let t5 = transfer("t5", "p5");
        let q5 = source
            .add_queued_subscriber_authorized(SubscriberId::new("p5"), &user("p5"))
            .unwrap();
        hold(&t5);
        assert!(c.dir.begin_transfer(&t5, &me, epoch, None).unwrap());
        assert_eq!(
            c.dir.accept_transfer(&t5.id, &b, &me, epoch, None).unwrap(),
            Settle::Done
        );
        source
            .with_state(|sim| sim.transfer_in("p5", &state(50), &back_auth(&t5), true))
            .unwrap();
        host.unsettle(Step::Back, &t5, at_a);
        host.resolve_unsettled();
        assert!(!has(&host, &a, "p5") && !held("p5"));
        assert!(got_transfer(&q5), "no transfer frame");

        // An Accept whose target no longer runs here is dropped.
        let mut t6 = transfer("t6", "p6");
        t6.to_shard = format!("gone-{run}");
        host.unsettle(Step::Accept, &t6, at_b);
        host.resolve_unsettled();
        assert!(!host.is_unsettled(&t6.id));

        // An entry from an earlier run of the target is dropped, and the
        // current run is not touched.
        let t9 = transfer("t9", "p9");
        assert!(c.dir.begin_transfer(&t9, &me, epoch, None).unwrap());
        let earlier = Instance {
            epoch: at_b.epoch,
            serial: at_b.serial + 1_000_000,
        };
        host.unsettle(Step::Accept, &t9, earlier);
        host.resolve_unsettled();
        assert!(!host.is_unsettled(&t9.id) && !has(&host, &b, "p9"));
        assert_eq!(c.dir.transfer(&t9.id).unwrap().unwrap().status, "out");

        // Taking back a row another attempt already returned does not add
        // the player again (the source's copy changed since: hp 77).
        let t7 = transfer("t7", "p7");
        assert!(c.dir.begin_transfer(&t7, &me, epoch, None).unwrap());
        assert_eq!(
            c.dir.return_transfer(&t7.id, &a, &me, epoch, None).unwrap(),
            Settle::Done
        );
        source
            .with_state(|sim| sim.transfer_in("p7", &state(77), &back_auth(&t7), true))
            .unwrap();
        assert!(matches!(
            host.take_back(&source, at_a, &SubscriberId::new("p7"), &t7),
            Ok(Back::Returned)
        ));
        assert_eq!(hp(&host, &a, "p7"), 77);

        // A waiting move: left alone while a call owns it, then finished
        // from its row (`in`: the connection is sent on).
        let t8 = transfer("t8", "p8");
        let q8 = source
            .add_queued_subscriber_authorized(SubscriberId::new("p8"), &user("p8"))
            .unwrap();
        hold(&t8);
        assert!(c.dir.begin_transfer(&t8, &me, epoch, None).unwrap());
        assert_eq!(
            c.dir.accept_transfer(&t8.id, &b, &me, epoch, None).unwrap(),
            Settle::Done
        );
        host.waiting
            .lock()
            .unwrap()
            .insert(t8.id.clone(), (t8.clone(), at_a));
        host.live.lock().unwrap().insert(t8.id.clone());
        host.settle_transfers(epoch);
        assert!(host.waiting.lock().unwrap().contains_key(&t8.id));
        host.live.lock().unwrap().remove(&t8.id);
        host.settle_transfers(epoch);
        assert!(!host.waiting.lock().unwrap().contains_key(&t8.id) && !held("p8"));
        assert!(got_transfer(&q8), "no transfer frame");

        // `back`: the source has the player; its inputs are accepted again.
        let t10 = transfer("t10", "p10");
        hold(&t10);
        assert!(c.dir.begin_transfer(&t10, &me, epoch, None).unwrap());
        assert_eq!(
            c.dir
                .return_transfer(&t10.id, &a, &me, epoch, None)
                .unwrap(),
            Settle::Done
        );
        host.waiting
            .lock()
            .unwrap()
            .insert(t10.id.clone(), (t10.clone(), at_a));
        host.settle_transfers(epoch);
        assert!(!host.waiting.lock().unwrap().contains_key(&t10.id) && !held("p10"));
        assert!(join(&source, "p10").is_ok());

        host.stop(&a);
        host.stop(&b);
        host.stop_all();
    }

    /// A source restarted on this machine (a lapsed lease, then adopted
    /// again) holds its open move from before it starts, and a slow call
    /// of its earlier run, finishing late, does not touch the new run.
    #[test]
    fn a_restarted_source_owns_its_open_move_and_an_old_call_does_not_touch_it() {
        let Ok(url) = std::env::var("PYLON_TEST_PG_URL") else {
            eprintln!("skipping: PYLON_TEST_PG_URL not set");
            return;
        };
        let _serial = crate::shard_cluster::tests::DIRECTORY_TESTS
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let Setup {
            host,
            run,
            me,
            a,
            b,
        } = setup(&url);
        let c = host.cluster.get().unwrap();
        let old_at = host.instance(&a).unwrap();
        let old_source = host.registry.get(&a).unwrap();
        let epoch = old_at.epoch;
        // The earlier run removed p1 and wrote the row, then its call to the
        // target stalled.
        let t = Transfer {
            id: format!("slow-{run}"),
            subscriber: "p1".into(),
            from_shard: a.clone(),
            to_shard: b.clone(),
            state: state(33),
            auth: serde_json::json!({}),
            status: "out".into(),
        };
        host.transferring
            .lock()
            .unwrap()
            .insert((a.clone(), "p1".into()), old_at);
        assert!(c.dir.begin_transfer(&t, &me, epoch, Some(b"{}")).unwrap());

        // The shard restarts here: a new run from saved state.
        host.stop_local(&a);
        let placement = c.dir.placement(&a).unwrap().unwrap();
        host.adopt(&placement, epoch).unwrap();
        let new_at = host.instance(&a).unwrap();
        assert_ne!(new_at, old_at);
        let source = host.registry.get(&a).unwrap();
        // Before any round: p1 cannot join the new run again.
        assert!(matches!(join(&source, "p1"), Err(ShardError::Transferring)));
        let q = source
            .add_queued_subscriber_authorized(SubscriberId::new("p1"), &user("p1"))
            .unwrap();

        // The old call finishes late: it changes nothing of the new run.
        host.complete(&old_source, old_at, &SubscriberId::new("p1"), &t);
        assert_eq!(
            host.transferring
                .lock()
                .unwrap()
                .get(&(a.clone(), "p1".into())),
            Some(&new_at)
        );
        assert!(host.waiting.lock().unwrap().contains_key(&t.id));
        assert!(matches!(
            host.take_back(&old_source, old_at, &SubscriberId::new("p1"), &t),
            Err(TransferError::Cluster(_))
        ));
        assert_eq!(c.dir.transfer(&t.id).unwrap().unwrap().status, "out");

        // The new run's first round finishes the move.
        host.settle_transfers(epoch);
        assert_eq!(c.dir.transfer(&t.id).unwrap().unwrap().status, "in");
        assert_eq!(hp(&host, &b, "p1"), 33);
        assert!(!has(&host, &a, "p1"));
        assert!(got_transfer(&q), "no transfer frame on the new run");
        assert!(!host
            .transferring
            .lock()
            .unwrap()
            .contains_key(&(a.clone(), "p1".into())));

        host.stop(&a);
        host.stop(&b);
        host.stop_all();
    }

    /// `confirm_status` waits for a transaction in flight on the row, and a
    /// transfer id it found unused cannot be written later.
    #[test]
    fn confirming_waits_for_a_commit_in_flight() {
        let Ok(url) = std::env::var("PYLON_TEST_PG_URL") else {
            eprintln!("skipping: PYLON_TEST_PG_URL not set");
            return;
        };
        let _serial = crate::shard_cluster::tests::DIRECTORY_TESTS
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let pool = PgPool::connect(&url, 4, Duration::from_secs(5)).unwrap();
        let dir = Arc::new(PgShardDirectory::open(Arc::clone(&pool)).unwrap());
        let run = pylon_cluster::new_instance_id();
        let id = format!("c-{run}");

        // An insert of the row, not yet committed.
        let (inserted_tx, inserted_rx) = std::sync::mpsc::channel();
        let (commit_tx, commit_rx) = std::sync::mpsc::channel::<()>();
        let writer = {
            let (pool, id) = (Arc::clone(&pool), id.clone());
            std::thread::spawn(move || {
                pool.with_client_once(|c| {
                    let mut tx = c.transaction()?;
                    tx.execute(
                        "INSERT INTO _pylon_shard_transfers
                            (transfer_id, subscriber, from_shard, to_shard, state, auth, status, created_at)
                         VALUES ($1, 'p', 'a', 'b', ''::bytea, '{}'::jsonb, 'out', 0)",
                        &[&id],
                    )?;
                    inserted_tx.send(()).unwrap();
                    commit_rx.recv().unwrap();
                    tx.commit()
                })
                .unwrap();
            })
        };
        inserted_rx.recv().unwrap();
        let reader = {
            let (dir, id) = (Arc::clone(&dir), id.clone());
            std::thread::spawn(move || dir.confirm_status(&id).unwrap())
        };
        std::thread::sleep(Duration::from_millis(300));
        assert!(!reader.is_finished(), "read before the commit landed");
        commit_tx.send(()).unwrap();
        writer.join().unwrap();
        assert_eq!(reader.join().unwrap().as_deref(), Some("out"));

        // An unused id is taken: a late write of it fails.
        let unused = format!("u-{run}");
        assert_eq!(dir.confirm_status(&unused).unwrap(), None);
        let late = pool.with_client_once(|c| {
            c.execute(
                "INSERT INTO _pylon_shard_transfers
                    (transfer_id, subscriber, from_shard, to_shard, state, auth, status, created_at)
                 VALUES ($1, 'p', 'a', 'b', ''::bytea, '{}'::jsonb, 'out', 0)",
                &[&unused],
            )
        });
        assert!(
            late.is_err(),
            "a late insert of a confirmed-unused id landed"
        );
        assert_eq!(dir.transfer(&unused).unwrap(), None);
    }

    /// A source with a move out is not stopped as idle. A source that ends
    /// with a move still open is released, and the target's machine takes
    /// the player; a player the target keeps refusing is abandoned after 30
    /// minutes, its state kept.
    #[test]
    fn a_move_out_keeps_its_source_and_a_stopped_source_s_row_is_finished() {
        let Ok(url) = std::env::var("PYLON_TEST_PG_URL") else {
            eprintln!("skipping: PYLON_TEST_PG_URL not set");
            return;
        };
        let _serial = crate::shard_cluster::tests::DIRECTORY_TESTS
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let Setup {
            host,
            run,
            me,
            a,
            b,
        } = setup(&url);
        let c = host.cluster.get().unwrap();
        let at_a = host.instance(&a).unwrap();
        let epoch = at_a.epoch;
        let long_ago = Instant::now() - Duration::from_secs(3600);
        let shut = format!("shut-{run}");
        host.create_on(
            "zone",
            &shut,
            &serde_json::json!({ "closed": true }),
            Some(&me),
        )
        .unwrap();

        // Idle for an hour, but a player is moving out: it stays up.
        host.transferring
            .lock()
            .unwrap()
            .insert((a.clone(), "p1".into()), at_a);
        host.idle_since.lock().unwrap().insert(a.clone(), long_ago);
        host.sweep();
        assert!(host.instance(&a).is_some(), "stopped with a move out");
        host.transferring.lock().unwrap().clear();

        // Rows out of `a`, 20 s old and 31 minutes old.
        let row = |id: &str, sid: &str, to: &str, age_ms: i64, hp: i64| {
            c.dir
                .pool_for_tests()
                .with_client_once(|cl| {
                    cl.execute(
                        "INSERT INTO _pylon_shard_transfers
                            (transfer_id, subscriber, from_shard, to_shard, state, auth, status, created_at)
                         VALUES ($1, $2, $3, $4, $5, '{}'::jsonb, 'out',
                                 (extract(epoch from clock_timestamp()) * 1000)::bigint - $6)",
                        &[&id, &sid, &a, &to, &state(hp), &age_ms],
                    )
                })
                .unwrap();
        };
        let (fresh, stuck) = (format!("fresh-{run}"), format!("stuck-{run}"));
        row(&fresh, "p2", &b, 20_000, 21);
        row(&stuck, "p3", &shut, 31 * 60 * 1000, 5);

        // The module ends (finished): the source is released, open rows or
        // not.
        host.registry.get(&a).unwrap().stop();
        host.sweep();
        assert!(host.registry.get(&a).is_none());
        assert!(c.dir.placement(&a).unwrap().is_none());
        assert!(host.instances.lock().unwrap().get(&a).is_none());

        // The target's machine takes p2; the closed zone refuses p3, which
        // is abandoned with its state.
        host.settle_transfers(epoch);
        assert_eq!(c.dir.transfer(&fresh).unwrap().unwrap().status, "in");
        assert_eq!(hp(&host, &b, "p2"), 21);
        let dropped = c.dir.transfer(&stuck).unwrap().unwrap();
        assert_eq!(dropped.status, "dropped");
        assert_eq!(dropped.state, state(5));

        host.stop(&b);
        host.stop(&shut);
        host.stop_all();
    }
}
