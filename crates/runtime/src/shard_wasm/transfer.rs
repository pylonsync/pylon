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
//! compare-and-set removes it again. A row still `out` after a while (the
//! source crashed, or a reply was lost) is finished by the machine that
//! holds the source.

use std::sync::Arc;

use pylon_realtime::{Shard, SubscriberId};

use super::{validate_shard_id, WasmShardHost, WasmSim};
use crate::shard_cluster::{self, RemoteOp, RemoteReply, Transfer};

/// A `out` row this old is finished by the source's machine.
pub(super) const STALE_TRANSFER_MS: i64 = 15_000;
/// How long the ticket for the target is valid.
const TICKET_TTL_SECS: u64 = 60;

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
    /// on another machine, and tell its connections to reconnect to `to`.
    /// On an error the player is in `from` (see the module docs).
    pub fn transfer(
        &self,
        from: &str,
        subscriber: &str,
        to: &str,
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
            return self.transfer_elsewhere(from, subscriber, to);
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
        let auth = source.subscriber_auth(&sid);
        let ticket_auth = TicketAuth {
            user_id: auth.as_ref().and_then(|a| a.user_id.clone()),
            claims: auth
                .as_ref()
                .and_then(|a| a.ticket.as_ref())
                .map(|t| t.claims.clone())
                .unwrap_or(serde_json::Value::Null),
        };
        let t = Transfer {
            id: new_transfer_id(),
            subscriber: subscriber.to_string(),
            from_shard: from.to_string(),
            to_shard: to.to_string(),
            state: Vec::new(),
            auth: serde_json::to_value(&ticket_auth).unwrap_or_default(),
            status: "out".into(),
        };
        self.begin(&source, &sid, t)
            .and_then(|t| self.deliver_and_settle(&source, &sid, &t))
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
        if started.is_err() {
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
        source.with_state(|sim| {
            let state = match sim.transfer_out(sid.as_str()) {
                Ok(Some(state)) => state,
                Ok(None) => return Err(TransferError::NoPlayer(t.subscriber.clone())),
                Err(e) => return Err(TransferError::Refused(e)),
            };
            if let (Some(c), Some(epoch)) = (cluster, epoch) {
                let recorded = sim
                    .save()
                    .map_err(TransferError::Cluster)
                    .map(|s| s.filter(|_| saves))
                    .and_then(|source_state| {
                        t.state = state.clone();
                        c.dir
                            .begin_transfer(t, &c.me.id, epoch, source_state.as_deref())
                            .map_err(TransferError::Cluster)
                    });
                if !matches!(recorded, Ok(true)) {
                    // Nothing recorded: the player goes straight back.
                    if let Err(e) = sim.transfer_in(sid.as_str(), &state) {
                        tracing::error!(
                            "[shard {}] could not take subscriber {} back after a failed transfer: {e}",
                            t.from_shard,
                            t.subscriber
                        );
                    }
                    return Err(match recorded {
                        Err(e) => e,
                        _ => TransferError::NotFound(t.from_shard.clone()),
                    });
                }
            }
            t.state = state;
            Ok(())
        })
    }

    /// Steps 2 and 3.
    fn deliver_and_settle(
        &self,
        source: &Arc<Shard<WasmSim>>,
        sid: &SubscriberId,
        t: &Transfer,
    ) -> Result<Transferred, TransferError> {
        let delivered = self.deliver(t);
        let result = match delivered {
            Ok(()) => Ok(()),
            Err(e) => match self.take_back(source, sid, t) {
                // The source has the player: the delivery error stands.
                Ok(true) => Err(e),
                // The target has it after all (its reply was lost).
                Ok(false) => Ok(()),
                Err(stranded) => Err(stranded),
            },
        };
        let out = result.map(|()| {
            let done = self.ticket_for(t);
            source.hand_off(
                sid,
                &pylon_realtime::wire::TransferNotice {
                    shard: done.shard.clone(),
                    ticket: done.ticket.clone(),
                },
            );
            tracing::info!(
                "[shard {}] subscriber {} moved to shard {}",
                t.from_shard,
                t.subscriber,
                t.to_shard
            );
            done
        });
        if out.is_err() {
            source.cancel_hand_off(sid);
        }
        self.end_transfer(t);
        out
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

    /// Add the player in `t` to its target, which runs here.
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
        let cluster = self.cluster.get();
        let _save = cluster.map(|c| c.save_lock.lock().unwrap());
        let epoch = match cluster {
            Some(c) => Some(
                c.owned
                    .lock()
                    .unwrap()
                    .get(&t.to_shard)
                    .copied()
                    .ok_or_else(|| TransferError::NotFound(t.to_shard.clone()))?,
            ),
            None => None,
        };
        let saves = self.saves_state(&t.to_shard);
        target.with_state(|sim| {
            sim.transfer_in(&t.subscriber, &t.state)
                .map_err(TransferError::Refused)?;
            let (Some(c), Some(epoch)) = (cluster, epoch) else {
                return Ok(());
            };
            let settled = sim
                .save()
                .map_err(TransferError::Cluster)
                .map(|s| s.filter(|_| saves))
                .and_then(|target_state| {
                    c.dir
                        .accept_transfer(&t.id, &t.to_shard, &c.me.id, epoch, target_state.as_deref())
                        .map_err(TransferError::Cluster)
                });
            match settled {
                Ok(true) => Ok(()),
                other => {
                    // Not ours: the source took the player back, or nothing
                    // was recorded. Remove the copy just added.
                    if let Err(e) = sim.transfer_out(&t.subscriber) {
                        tracing::error!(
                            "[shard {}] could not remove subscriber {} after losing its transfer: {e}",
                            t.to_shard,
                            t.subscriber
                        );
                    }
                    Err(match other {
                        Err(e) => e,
                        _ => TransferError::Refused(format!(
                            "transfer {} was settled elsewhere",
                            t.id
                        )),
                    })
                }
            }
        })
    }

    /// Step 3: give the player back to the source. Ok(true): the source has
    /// it. Ok(false): the target took it first.
    fn take_back(
        &self,
        source: &Arc<Shard<WasmSim>>,
        sid: &SubscriberId,
        t: &Transfer,
    ) -> Result<bool, TransferError> {
        let cluster = self.cluster.get();
        let _save = cluster.map(|c| c.save_lock.lock().unwrap());
        let epoch = cluster.and_then(|c| c.owned.lock().unwrap().get(&t.from_shard).copied());
        let saves = self.saves_state(&t.from_shard);
        source.with_state(|sim| {
            if let Err(e) = sim.transfer_in(sid.as_str(), &t.state) {
                // The row keeps the player; a later round tries again.
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
            let Some(c) = cluster else {
                return Ok(true);
            };
            let settled = match epoch {
                Some(epoch) => sim
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
                    }),
                None => Err(TransferError::NotFound(t.from_shard.clone())),
            };
            if !matches!(settled, Ok(true)) {
                // Not ours: remove the copy just added.
                let _ = sim.transfer_out(sid.as_str());
            }
            // Ok(false): the row is no longer `out`, so the target has it.
            settled
        })
    }

    /// Finish transfers still `out` whose source runs here: deliver them,
    /// or take them back.
    pub(super) fn settle_stale_transfers(&self, epoch: i64) {
        let Some(c) = self.cluster.get() else { return };
        let stale = match c.dir.stale_transfers(&c.me.id, epoch, STALE_TRANSFER_MS) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("[shards] reading unfinished transfers failed: {e}");
                return;
            }
        };
        for t in stale {
            let key = (t.from_shard.clone(), t.subscriber.clone());
            if !self.transferring.lock().unwrap().insert(key) {
                continue;
            }
            let Some(source) = self.registry.get(&t.from_shard).filter(|s| s.is_running()) else {
                self.end_transfer(&t);
                continue;
            };
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

    /// The source is on another machine: that machine moves the player.
    fn transfer_elsewhere(
        &self,
        from: &str,
        subscriber: &str,
        to: &str,
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

    /// Run the transfers modules asked for after their ticks, one at a time.
    pub(super) fn run_requested_transfers(
        weak: std::sync::Weak<Self>,
        requests: std::sync::mpsc::Receiver<(String, String, String)>,
    ) {
        while let Ok((from, sid, to)) = requests.recv() {
            let Some(host) = weak.upgrade() else { return };
            if host.stopped.load(std::sync::atomic::Ordering::Acquire) {
                return;
            }
            if let Err(e) = host.transfer(&from, &sid, &to) {
                tracing::warn!(
                    "[shard {from}] moving subscriber {sid} to shard {to} failed ({}): {e}",
                    e.code()
                );
            }
        }
    }
}
