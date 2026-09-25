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
//! A step whose commit may or may not have landed (the connection died
//! during it) is "unsettled": the shard keeps what it has, and each round
//! reads the row and finishes the step from what it says. A row still `out`
//! (the source crashed, or no side could settle it) is finished by the
//! machine that holds the source: at once after that machine starts the
//! source from saved state, else after 15 s. Until then the player's inputs
//! to the source are refused, so it cannot join the source again.

use std::sync::Arc;

use pylon_realtime::{Shard, ShardAuth, SubscriberId};

use super::{validate_shard_id, WasmShardHost, WasmSim};
use crate::shard_cluster::{self, RemoteOp, RemoteReply, Settle, Transfer};

/// A `out` row this old is finished by the source's machine.
pub(super) const STALE_TRANSFER_MS: i64 = 15_000;
/// How long the ticket for the target is valid, and how long the source
/// repeats the transfer notice to a connection that comes back.
const TICKET_TTL_SECS: u64 = 600;
/// A move a module asked for that failed is not tried again for this long.
const REQUEST_BACKOFF: std::time::Duration = std::time::Duration::from_secs(5);
/// How many module-requested moves wait for the transfer thread.
pub(super) const REQUEST_QUEUE: usize = 1024;

/// A step whose commit is unknown (see the module docs).
#[derive(Debug, Clone)]
pub(super) enum Unsettled {
    /// The source removed the player; the row may not exist.
    Begin(Transfer),
    /// The target added the player; the row may not be `in`.
    Accept(Transfer),
    /// The source took the player back; the row may not be `back`.
    Back(Transfer),
}

impl Unsettled {
    fn transfer(&self) -> &Transfer {
        match self {
            Self::Begin(t) | Self::Accept(t) | Self::Back(t) => t,
        }
    }
}

/// What the row says, read again after a step failed: its status, `None`
/// when there is no row, or `Err` when the directory cannot say.
fn reread(
    dir: &crate::shard_cluster::PgShardDirectory,
    id: &str,
) -> Result<Option<String>, String> {
    let mut last = String::new();
    for attempt in 0..3 {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        match dir.transfer(id) {
            Ok(row) => return Ok(row.map(|t| t.status)),
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
        let t = self.begin(&source, &sid, t)?;
        self.deliver_and_settle(&source, &sid, &t)
    }

    /// Step 1: take the player out of the source and record the transfer.
    fn begin(
        &self,
        source: &Arc<Shard<WasmSim>>,
        sid: &SubscriberId,
        mut t: Transfer,
    ) -> Result<Transfer, TransferError> {
        {
            let mut transferring = self.transferring.lock().unwrap();
            let key = (t.from_shard.clone(), t.subscriber.clone());
            if !transferring.insert(key) {
                return Err(TransferError::Busy(format!(
                    "subscriber \"{}\" is already moving",
                    t.subscriber
                )));
            }
        }
        source.begin_hand_off(sid);
        let started = self.take_out(source, sid, &mut t);
        if started.is_err() && !self.is_unsettled(&t.id) {
            source.cancel_hand_off(sid);
            self.end_transfer(&t);
        }
        started.map(|()| t)
    }

    fn take_out(
        &self,
        source: &Arc<Shard<WasmSim>>,
        sid: &SubscriberId,
        t: &mut Transfer,
    ) -> Result<(), TransferError> {
        let cluster = self.cluster.get();
        // Under the save lock: a periodic save captured with the player in
        // it must not land after the save without it.
        let _save = cluster.map(|c| c.save_lock.lock().unwrap());
        let epoch = match cluster {
            Some(c) => Some(
                c.owned
                    .lock()
                    .unwrap()
                    .get(&t.from_shard)
                    .copied()
                    .ok_or_else(|| TransferError::NotFound(t.from_shard.clone()))?,
            ),
            None => None,
        };
        let saves = self.saves_state(&t.from_shard);
        let auth = back_auth(t);
        source.with_state(|sim| {
            let state = match sim.transfer_out(sid.as_str()) {
                Ok(Some(state)) => state,
                Ok(None) => return Err(TransferError::NoPlayer(t.subscriber.clone())),
                Err(e) => return Err(TransferError::Refused(e)),
            };
            t.state = state;
            let (Some(c), Some(epoch)) = (cluster, epoch) else {
                return Ok(());
            };
            let recorded = sim
                .save()
                .map_err(TransferError::Cluster)
                .map(|s| s.filter(|_| saves))
                .and_then(|source_state| {
                    c.dir
                        .begin_transfer(t, &c.me.id, epoch, source_state.as_deref())
                        .map_err(TransferError::Cluster)
                });
            let put_back = |why: TransferError| {
                if let Err(e) = sim.transfer_in(sid.as_str(), &t.state, &auth, true) {
                    self.strand(t, &e);
                }
                Err(why)
            };
            match recorded {
                Ok(true) => Ok(()),
                // Not held (the lease lapsed): nothing was written.
                Ok(false) => put_back(TransferError::NotFound(t.from_shard.clone())),
                Err(e) => match reread(&c.dir, &t.id) {
                    // The commit landed: carry on.
                    Ok(Some(_)) => Ok(()),
                    Ok(None) => put_back(e),
                    Err(_) => {
                        self.unsettle(Unsettled::Begin(t.clone()));
                        Err(e)
                    }
                },
            }
        })
    }

    /// Steps 2 and 3.
    fn deliver_and_settle(
        &self,
        source: &Arc<Shard<WasmSim>>,
        sid: &SubscriberId,
        t: &Transfer,
    ) -> Result<Transferred, TransferError> {
        let result = match self.deliver(t) {
            Ok(()) => Ok(()),
            Err(e) => match self.take_back(source, sid, t) {
                // The source has the player: the delivery error stands.
                Ok(Back::Returned) => Err(e),
                // The target has it after all (its reply was lost).
                Ok(Back::TargetHas) => Ok(()),
                Err(e) => Err(e),
            },
        };
        match result {
            Ok(()) => {
                let done = self.finish_move(source, sid, t);
                self.end_transfer(t);
                Ok(done)
            }
            Err(e) => {
                // The row still holds the player (`out`), or the outcome is
                // unknown: the player's inputs stay refused and a round
                // finishes it. Otherwise the source has the player again.
                if self.is_unsettled(&t.id) {
                } else if self.row_is_out(t) {
                    self.pending.lock().unwrap().insert(t.id.clone(), t.clone());
                } else {
                    source.cancel_hand_off(sid);
                    self.end_transfer(t);
                }
                Err(e)
            }
        }
    }

    /// The player is in the target: tell its connections to the source.
    fn finish_move(
        &self,
        source: &Shard<WasmSim>,
        sid: &SubscriberId,
        t: &Transfer,
    ) -> Transferred {
        let done = self.ticket_for(t);
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
        done
    }

    fn row_is_out(&self, t: &Transfer) -> bool {
        self.cluster
            .get()
            .is_some_and(|c| matches!(reread(&c.dir, &t.id), Ok(Some(s)) if s == "out"))
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
    /// twice for one transfer: the row is read first, under the save lock.
    pub(super) fn accept(&self, t: &Transfer) -> Result<(), TransferError> {
        let Some(target) = self.registry.get(&t.to_shard).filter(|s| s.is_running()) else {
            return Err(TransferError::NotFound(t.to_shard.clone()));
        };
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
        let epoch = c
            .owned
            .lock()
            .unwrap()
            .get(&t.to_shard)
            .copied()
            .ok_or_else(|| TransferError::NotFound(t.to_shard.clone()))?;
        // An earlier attempt left the player here with the row unknown.
        let already_here = matches!(
            self.unsettled.lock().unwrap().get(&t.id),
            Some(Unsettled::Accept(_))
        );
        match c.dir.transfer(&t.id).map_err(TransferError::Cluster)? {
            Some(row) if row.status == "out" => {}
            Some(row) if row.status == "in" => {
                self.unsettled.lock().unwrap().remove(&t.id);
                return Ok(());
            }
            Some(row) => {
                return Err(TransferError::Refused(format!(
                    "transfer {} is already {}",
                    t.id, row.status
                )))
            }
            None => return Err(TransferError::Refused(format!("no transfer {}", t.id))),
        }
        let saves = self.saves_state(&t.to_shard);
        let outcome = target.with_state(|sim| {
            if !already_here {
                sim.transfer_in(&t.subscriber, &t.state, &auth, false)
                    .map_err(TransferError::Refused)?;
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
                            epoch,
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
                Ok(Settle::Done) => Ok(()),
                Ok(Settle::Taken) => {
                    undo();
                    Err(TransferError::Refused(format!(
                        "transfer {} was settled by the source",
                        t.id
                    )))
                }
                Ok(Settle::NotHeld) => {
                    undo();
                    Err(TransferError::Cluster(format!(
                        "this machine no longer holds shard \"{}\"",
                        t.to_shard
                    )))
                }
                Err(e) => match reread(&c.dir, &t.id) {
                    Ok(Some(s)) if s == "in" => Ok(()),
                    Ok(_) => {
                        undo();
                        Err(e)
                    }
                    Err(_) => {
                        self.unsettle(Unsettled::Accept(t.clone()));
                        Err(e)
                    }
                },
            }
        });
        if outcome.is_ok() || !self.is_unsettled(&t.id) {
            self.unsettled.lock().unwrap().remove(&t.id);
        }
        if outcome.is_ok() {
            self.arrived(&target, &sid, t);
        } else if self.is_unsettled(&t.id) {
            // Kept for the round to settle.
        }
        outcome
    }

    /// The player is in the target now.
    fn arrived(&self, target: &Shard<WasmSim>, sid: &SubscriberId, t: &Transfer) {
        // It no longer moves away from here, and the shard is not idle.
        target.forget_move(sid);
        self.idle_since.lock().unwrap().remove(&t.to_shard);
    }

    /// Step 3: give the player back to the source.
    fn take_back(
        &self,
        source: &Arc<Shard<WasmSim>>,
        sid: &SubscriberId,
        t: &Transfer,
    ) -> Result<Back, TransferError> {
        let auth = back_auth(t);
        let Some(c) = self.cluster.get() else {
            if let Err(e) =
                source.with_state(|sim| sim.transfer_in(sid.as_str(), &t.state, &auth, true))
            {
                self.strand(t, &e);
                return Err(TransferError::Cluster(format!(
                    "the source refused the player back: {e}"
                )));
            }
            return Ok(Back::Returned);
        };
        let _save = c.save_lock.lock().unwrap();
        let Some(epoch) = c.owned.lock().unwrap().get(&t.from_shard).copied() else {
            // The source moved (a lapsed lease): its new holder finishes.
            return Err(TransferError::Cluster(format!(
                "this machine no longer holds shard \"{}\"",
                t.from_shard
            )));
        };
        let already_back = matches!(
            self.unsettled.lock().unwrap().get(&t.id),
            Some(Unsettled::Back(_))
        );
        let saves = self.saves_state(&t.from_shard);
        let outcome = source.with_state(|sim| {
            if !already_back {
                if let Err(e) = sim.transfer_in(sid.as_str(), &t.state, &auth, true) {
                    // The row keeps the player; a later round offers it
                    // again.
                    tracing::error!(
                        "[shard {}] refused subscriber {} back from transfer {}: {e}",
                        t.from_shard,
                        t.subscriber,
                        t.id
                    );
                    return Err(TransferError::Cluster(format!(
                        "the source refused the player back: {e}"
                    )));
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
                            epoch,
                            source_state.as_deref(),
                        )
                        .map_err(TransferError::Cluster)
                });
            let undo = || {
                let _ = sim.transfer_out(sid.as_str());
            };
            match settled {
                Ok(Settle::Done) => Ok(Back::Returned),
                // Only the target moves a row this machine's source holds
                // away from `out`: the target has the player.
                Ok(Settle::Taken) => {
                    undo();
                    Ok(Back::TargetHas)
                }
                Ok(Settle::NotHeld) => {
                    undo();
                    Err(TransferError::Cluster(format!(
                        "this machine no longer holds shard \"{}\"",
                        t.from_shard
                    )))
                }
                Err(e) => match reread(&c.dir, &t.id) {
                    Ok(Some(s)) if s == "back" => Ok(Back::Returned),
                    Ok(Some(s)) if s == "in" => {
                        undo();
                        Ok(Back::TargetHas)
                    }
                    Ok(_) => {
                        undo();
                        Err(e)
                    }
                    Err(_) => {
                        self.unsettle(Unsettled::Back(t.clone()));
                        Err(e)
                    }
                },
            }
        });
        if !matches!(outcome, Err(_)) || !self.is_unsettled(&t.id) {
            if !matches!(outcome, Err(_)) {
                self.unsettled.lock().unwrap().remove(&t.id);
            }
        }
        outcome
    }

    /// One round's transfer work, on a machine in a cluster: settle steps
    /// whose outcome was unknown, then finish rows still `out` whose source
    /// runs here.
    pub(super) fn settle_transfers(&self, epoch: i64) {
        let Some(c) = self.cluster.get() else { return };
        self.resolve_unsettled();
        let mut rows: Vec<Transfer> = self
            .pending
            .lock()
            .unwrap()
            .drain()
            .map(|(_, t)| t)
            .collect();
        let resumed: Vec<String> = self.resume.lock().unwrap().drain().collect();
        for shard in resumed {
            match c.dir.open_transfers_from(&shard) {
                Ok(open) => rows.extend(open),
                Err(e) => {
                    tracing::warn!("[shard {shard}] reading its unfinished transfers failed: {e}");
                    self.resume.lock().unwrap().insert(shard);
                }
            }
        }
        match c.dir.stale_transfers(&c.me.id, epoch, STALE_TRANSFER_MS) {
            Ok(stale) => rows.extend(stale),
            Err(e) => tracing::warn!("[shards] reading unfinished transfers failed: {e}"),
        }
        let mut seen = std::collections::HashSet::new();
        for t in rows {
            if !seen.insert(t.id.clone()) || self.is_unsettled(&t.id) {
                continue;
            }
            let Some(source) = self.registry.get(&t.from_shard).filter(|s| s.is_running()) else {
                self.end_transfer(&t);
                continue;
            };
            // Held already by a live attempt or a pending one; otherwise
            // take it now.
            self.transferring
                .lock()
                .unwrap()
                .insert((t.from_shard.clone(), t.subscriber.clone()));
            tracing::info!(
                "[shard {}] finishing transfer {} of subscriber {} to shard {}",
                t.from_shard,
                t.id,
                t.subscriber,
                t.to_shard
            );
            let sid = SubscriberId::new(t.subscriber.as_str());
            source.begin_hand_off(&sid);
            if let Err(e) = self.deliver_and_settle(&source, &sid, &t) {
                tracing::warn!("[shard {}] transfer {}: {e}", t.from_shard, t.id);
            }
        }
        let _ = c.dir.prune_transfers();
    }

    /// A shard just started here from saved state: its transfers still
    /// `out` are finished in the next round, and until then the players in
    /// them cannot act in it.
    pub(super) fn resume_transfers(&self, shard: &Shard<WasmSim>) {
        let Some(c) = self.cluster.get() else { return };
        let open = match c.dir.open_transfers_from(shard.id()) {
            Ok(open) => open,
            Err(e) => {
                tracing::warn!(
                    "[shard {}] reading its unfinished transfers failed: {e}",
                    shard.id()
                );
                self.resume.lock().unwrap().insert(shard.id().to_string());
                return;
            }
        };
        for t in open {
            shard.begin_hand_off(&SubscriberId::new(t.subscriber.as_str()));
            self.transferring
                .lock()
                .unwrap()
                .insert((t.from_shard.clone(), t.subscriber.clone()));
            self.pending.lock().unwrap().insert(t.id.clone(), t);
        }
    }

    /// Settle each unknown step from its row.
    fn resolve_unsettled(&self) {
        let Some(c) = self.cluster.get() else { return };
        let steps: Vec<Unsettled> = self.unsettled.lock().unwrap().values().cloned().collect();
        for step in steps {
            let t = step.transfer().clone();
            let status = match reread(&c.dir, &t.id) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let source = self.registry.get(&t.from_shard).filter(|s| s.is_running());
            let sid = SubscriberId::new(t.subscriber.as_str());
            match step {
                Unsettled::Begin(_) => {
                    self.unsettled.lock().unwrap().remove(&t.id);
                    match status {
                        // The row exists: finish the move from it.
                        Some(_) => {
                            self.pending.lock().unwrap().insert(t.id.clone(), t);
                        }
                        // It never landed: the player goes back.
                        None => {
                            if let Some(source) = source {
                                let auth = back_auth(&t);
                                if let Err(e) = source.with_state(|sim| {
                                    sim.transfer_in(&t.subscriber, &t.state, &auth, true)
                                }) {
                                    self.strand(&t, &e);
                                }
                                source.cancel_hand_off(&sid);
                            }
                            self.end_transfer(&t);
                        }
                    }
                }
                Unsettled::Accept(_) => match status.as_deref() {
                    Some("in") => {
                        self.unsettled.lock().unwrap().remove(&t.id);
                    }
                    Some("out") => {
                        let _ = self.accept(&t);
                    }
                    _ => {
                        // The source took it back: remove the copy here.
                        self.unsettled.lock().unwrap().remove(&t.id);
                        if let Some(target) = self.registry.get(&t.to_shard) {
                            let _ = target.with_state(|sim| sim.transfer_out(&t.subscriber));
                        }
                    }
                },
                Unsettled::Back(_) => match status.as_deref() {
                    Some("back") => {
                        self.unsettled.lock().unwrap().remove(&t.id);
                        if let Some(source) = source {
                            source.cancel_hand_off(&sid);
                        }
                        self.end_transfer(&t);
                    }
                    Some("out") => {
                        if let Some(source) = source {
                            if let Ok(Back::TargetHas) = self.take_back(&source, &sid, &t) {
                                self.finish_move(&source, &sid, &t);
                            }
                            if !self.is_unsettled(&t.id) {
                                source.cancel_hand_off(&sid);
                                self.end_transfer(&t);
                            }
                        }
                    }
                    _ => {
                        // The target has it: remove the copy here, and send
                        // the player there.
                        self.unsettled.lock().unwrap().remove(&t.id);
                        if let Some(source) = source {
                            let _ = source.with_state(|sim| sim.transfer_out(&t.subscriber));
                            self.finish_move(&source, &sid, &t);
                        }
                        self.end_transfer(&t);
                    }
                },
            }
        }
    }

    fn unsettle(&self, step: Unsettled) {
        let t = step.transfer();
        tracing::warn!(
            "[shard {}] transfer {} of subscriber {}: the directory did not confirm the step; settling it from the row",
            t.from_shard,
            t.id,
            t.subscriber
        );
        self.unsettled.lock().unwrap().insert(t.id.clone(), step);
    }

    fn is_unsettled(&self, id: &str) -> bool {
        self.unsettled.lock().unwrap().contains_key(id)
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
    fn strand(&self, t: &Transfer, why: &str) {
        tracing::error!(
            "[shard {}] refused subscriber {} back ({why}); the host keeps it and offers it again",
            t.from_shard,
            t.subscriber
        );
        self.stranded.lock().unwrap().push(t.clone());
    }

    /// Offer kept players back to their source shards (from the sweep). One
    /// whose source is gone is dropped, with an error log: nothing is left
    /// to return it to.
    pub(super) fn retry_stranded(&self) {
        let kept = std::mem::take(&mut *self.stranded.lock().unwrap());
        for t in kept {
            let Some(source) = self.registry.get(&t.from_shard).filter(|s| s.is_running()) else {
                tracing::error!(
                    "[shard {}] stopped with subscriber {} still waiting to come back; dropped",
                    t.from_shard,
                    t.subscriber
                );
                continue;
            };
            let auth = back_auth(&t);
            match source.with_state(|sim| sim.transfer_in(&t.subscriber, &t.state, &auth, true)) {
                Ok(()) => tracing::info!(
                    "[shard {}] took subscriber {} back",
                    t.from_shard,
                    t.subscriber
                ),
                Err(_) => self.stranded.lock().unwrap().push(t),
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

    fn end_transfer(&self, t: &Transfer) {
        self.transferring
            .lock()
            .unwrap()
            .remove(&(t.from_shard.clone(), t.subscriber.clone()));
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
                .contains(&(from.clone(), sid.clone()));
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
    use pylon_realtime::ShardConfig;
    use pylon_storage::pg_datastore::PgPool;
    use std::time::{Duration, Instant};

    fn has(host: &WasmShardHost, shard: &str, sid: &str) -> bool {
        let shard = host.registry.get(shard).expect("running");
        let saved = shard.with_state(|sim| sim.save()).unwrap().unwrap();
        let players: serde_json::Value = serde_json::from_slice(&saved).unwrap();
        !players[sid].is_null()
    }

    /// Each kind of step whose commit was unknown is settled from its row.
    #[test]
    fn unknown_steps_are_settled_from_the_row() {
        let Ok(url) = std::env::var("PYLON_TEST_PG_URL") else {
            eprintln!("skipping: PYLON_TEST_PG_URL not set");
            return;
        };
        let _serial = crate::shard_cluster::tests::DIRECTORY_TESTS
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let pool = PgPool::connect(&url, 4, Duration::from_secs(5)).unwrap();
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
        let c = host.cluster.get().unwrap();
        let epoch = c.current_epoch().unwrap();
        let player = serde_json::to_vec(&serde_json::json!({
            "x": 0, "hp": 50, "buffs": [], "cooldowns": {}
        }))
        .unwrap();
        let transfer = |id: &str, sid: &str| Transfer {
            id: format!("{id}-{run}"),
            subscriber: sid.into(),
            from_shard: a.clone(),
            to_shard: b.clone(),
            state: player.clone(),
            auth: serde_json::json!({}),
            status: "out".into(),
        };

        // Begin, and no row landed: the player goes back to the source.
        let t1 = transfer("t1", "p1");
        host.unsettle(Unsettled::Begin(t1.clone()));
        host.resolve_unsettled();
        assert!(!host.is_unsettled(&t1.id));
        assert!(has(&host, &a, "p1"));

        // Begin, and the row landed: the move is finished from it.
        let t2 = transfer("t2", "p2");
        assert!(c.dir.begin_transfer(&t2, &me, epoch, None).unwrap());
        host.unsettle(Unsettled::Begin(t2.clone()));
        host.resolve_unsettled();
        host.settle_transfers(epoch);
        assert_eq!(c.dir.transfer(&t2.id).unwrap().unwrap().status, "in");
        assert!(has(&host, &b, "p2") && !has(&host, &a, "p2"));

        // Accept, and the source took the player back meanwhile: the
        // target's copy goes.
        let t3 = transfer("t3", "p3");
        assert!(c.dir.begin_transfer(&t3, &me, epoch, None).unwrap());
        assert_eq!(
            c.dir.return_transfer(&t3.id, &a, &me, epoch, None).unwrap(),
            Settle::Done
        );
        let auth = arrival_auth(&t3);
        host.registry
            .get(&b)
            .unwrap()
            .with_state(|sim| sim.transfer_in("p3", &player, &auth, false))
            .unwrap();
        host.unsettle(Unsettled::Accept(t3.clone()));
        host.resolve_unsettled();
        assert!(!has(&host, &b, "p3"));

        // Accept, and the row is still out: the target settles it and keeps
        // the player, without adding it a second time.
        let t4 = transfer("t4", "p4");
        assert!(c.dir.begin_transfer(&t4, &me, epoch, None).unwrap());
        host.registry
            .get(&b)
            .unwrap()
            .with_state(|sim| sim.transfer_in("p4", &player, &arrival_auth(&t4), false))
            .unwrap();
        host.unsettle(Unsettled::Accept(t4.clone()));
        host.resolve_unsettled();
        assert_eq!(c.dir.transfer(&t4.id).unwrap().unwrap().status, "in");
        assert!(has(&host, &b, "p4") && !host.is_unsettled(&t4.id));

        // Back, and the target took it first: the source's copy goes, and
        // the player is sent on.
        let t5 = transfer("t5", "p5");
        assert!(c.dir.begin_transfer(&t5, &me, epoch, None).unwrap());
        assert_eq!(
            c.dir.accept_transfer(&t5.id, &b, &me, epoch, None).unwrap(),
            Settle::Done
        );
        host.registry
            .get(&a)
            .unwrap()
            .with_state(|sim| sim.transfer_in("p5", &player, &back_auth(&t5), true))
            .unwrap();
        host.unsettle(Unsettled::Back(t5.clone()));
        host.resolve_unsettled();
        assert!(!has(&host, &a, "p5"));

        host.stop(&a);
        host.stop(&b);
        host.stop_all();
    }
}
