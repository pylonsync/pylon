//! Messages between shards (issue #32).
//!
//! A shard's module sends messages from its outbox after each tick; a
//! server function sends them with `ctx.shards.publish`. A message goes to
//! one shard (`shard:<id>`), to the shards in a group (`group:<name>`, from
//! each module's `groups`), or to every shard (`all`), on any machine. The
//! receiving module's `on_message` runs at the start of its next tick.
//!
//! Shards on this machine get a message at once. For other machines it
//! goes on the cluster bus when the app has one (`PYLON_CLUSTER_BUS`), else
//! as a signed call to each live machine in the shard directory, each
//! machine through its own worker and queue, so a slow machine only delays
//! its own messages. Delivery is at most once: a message to a shard that is
//! stopped, whose queue is full, or on a machine that cannot be reached or
//! is behind, is dropped, and no one is told.
//!
//! A server function also sends an input to one shard with
//! `ctx.shards.send` (see [`WasmShardHost::send_input`]); the module's
//! `apply_input` gets it, from the empty subscriber id. From a mutation it
//! is sent after the commit.

use std::sync::Arc;

use base64::Engine;
use pylon_realtime::ShardMessage;

use super::WasmShardHost;
use crate::shard_cluster::{self, RemoteOp};

/// Envelope kind on the cluster bus.
pub(super) const BUS_KIND: &str = "shard-message";
/// Messages waiting for the routing thread.
pub(super) const ROUTE_QUEUE: usize = 4096;
/// Messages waiting for one other machine.
const PEER_QUEUE: usize = 1024;
/// A worker for another machine that sent nothing for this long exits.
#[cfg(not(test))]
const PEER_IDLE: std::time::Duration = std::time::Duration::from_secs(60);
#[cfg(test)]
const PEER_IDLE: std::time::Duration = std::time::Duration::from_millis(300);

/// The worker that sends messages to one other machine.
pub(super) struct Peer {
    address: String,
    /// Tells this worker from a later one for the same machine.
    serial: u64,
    sender: std::sync::mpsc::SyncSender<RemoteOp>,
}
/// How long the list of live machines, and where a shard runs, are reused.
const PEER_CACHE: std::time::Duration = std::time::Duration::from_secs(2);
/// Limits on what a module or a function sends.
const MAX_MESSAGES_PER_TICK: usize = 64;
const MAX_TOPIC: usize = 128;
const MAX_GROUP: usize = 128;
const MAX_GROUPS: usize = 64;
const MAX_DATA: usize = 64 * 1024;
/// Records (groups or messages) the host reads from one outbox. The SDK
/// sends at most 64 messages and 64 groups; past this, the rest count as
/// dropped unread, so a module cannot make the host parse without end.
const MAX_RECORDS: usize = 256;
/// The largest outbox the host copies out of a module (64 full messages and
/// 64 groups are about 4.2 MB).
pub(super) const MAX_OUTBOX_BYTES: usize = 8 * 1024 * 1024;

/// Where a message goes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Target {
    Shard(String),
    Group(String),
    All,
}

impl Target {
    pub fn parse(s: &str) -> Result<Self, MessageError> {
        if s == "all" {
            return Ok(Target::All);
        }
        if let Some(id) = s.strip_prefix("shard:") {
            super::validate_shard_id(id).map_err(|e| MessageError(e.to_string()))?;
            return Ok(Target::Shard(id.to_string()));
        }
        if let Some(name) = s.strip_prefix("group:") {
            if name.is_empty() || name.len() > MAX_GROUP {
                return Err(MessageError(format!(
                    "a group name is 1 to {MAX_GROUP} bytes"
                )));
            }
            return Ok(Target::Group(name.to_string()));
        }
        Err(MessageError(format!(
            "target \"{s}\" is not \"shard:<id>\", \"group:<name>\", or \"all\""
        )))
    }

    pub fn wire(&self) -> String {
        match self {
            Target::Shard(id) => format!("shard:{id}"),
            Target::Group(name) => format!("group:{name}"),
            Target::All => "all".into(),
        }
    }
}

/// A message that cannot be sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageError(pub String);

impl MessageError {
    pub fn code(&self) -> &'static str {
        "SHARD_MESSAGE_INVALID"
    }
}

impl std::fmt::Display for MessageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for MessageError {}

/// A server input that cannot be sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendError {
    /// No shard with that id runs, here or (in a cluster) anywhere.
    NotFound,
    /// A bad shard id, or an input that is not valid for the shard.
    Invalid(String),
    /// The shard's input queue, or the queue to other machines, is full.
    Busy(String),
}

impl SendError {
    pub fn code(&self) -> &'static str {
        match self {
            SendError::NotFound => "SHARD_NOT_FOUND",
            SendError::Invalid(_) => "SHARD_INPUT_INVALID",
            SendError::Busy(_) => "SHARD_BUSY",
        }
    }
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SendError::NotFound => f.write_str("no shard with that id is running"),
            SendError::Invalid(why) | SendError::Busy(why) => f.write_str(why),
        }
    }
}

impl std::error::Error for SendError {}

/// What the routing thread passes to other machines.
pub(super) enum Outgoing {
    Message(Routed),
    /// A server input for shard `shard`, which does not run here.
    Input {
        shard: String,
        input: serde_json::Value,
    },
}

/// The largest server input, as JSON.
const MAX_INPUT: usize = 64 * 1024;

/// One message for other machines.
#[derive(Debug, Clone)]
pub(super) struct Routed {
    pub(super) from: String,
    pub(super) to: Target,
    pub(super) topic: String,
    pub(super) data: Arc<[u8]>,
}

/// What a module's `pylon_outbox` returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outbox {
    /// Its groups, when they changed.
    pub groups: Option<Vec<String>>,
    /// (target, topic, data)
    pub messages: Vec<(String, String, Vec<u8>)>,
    /// Messages dropped for breaking a limit.
    pub dropped: usize,
}

impl Outbox {
    /// Parse the outbox format (see the guest SDK's ABI docs). Only a
    /// malformed frame is an error; a message over a limit, or past the
    /// first [`MAX_MESSAGES_PER_TICK`], is dropped and counted.
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let mut at = 0usize;
        let mut take = |n: usize| -> Result<&[u8], String> {
            let end = at.checked_add(n).filter(|&e| e <= bytes.len());
            let end = end.ok_or("the outbox ends early")?;
            let slice = &bytes[at..end];
            at = end;
            Ok(slice)
        };
        let u16_at = |b: &[u8]| u16::from_le_bytes([b[0], b[1]]) as usize;
        let u32_at = |b: &[u8]| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize;
        let mut dropped = 0usize;
        let groups = match take(1)?[0] {
            0 => None,
            1 => {
                let n = u16_at(take(2)?);
                if n > MAX_RECORDS {
                    return Err(format!("{n} groups (at most {MAX_RECORDS})"));
                }
                let mut groups = Vec::new();
                for _ in 0..n {
                    let len = u16_at(take(2)?);
                    let name = take(len)?;
                    match std::str::from_utf8(name) {
                        Ok(g)
                            if !g.is_empty()
                                && g.len() <= MAX_GROUP
                                && groups.len() < MAX_GROUPS =>
                        {
                            groups.push(g.to_string())
                        }
                        _ => dropped += 1,
                    }
                }
                Some(groups)
            }
            other => return Err(format!("groups flag {other}")),
        };
        let count = u32_at(take(4)?);
        let mut messages = Vec::new();
        for read in 0..count {
            if read == MAX_RECORDS {
                // The rest are not read: dropped, unchecked.
                dropped += count - read;
                return Ok(Outbox {
                    groups,
                    messages,
                    dropped,
                });
            }
            let len = u16_at(take(2)?);
            let target = take(len)?;
            let len = u16_at(take(2)?);
            let topic = take(len)?;
            let len = u32_at(take(4)?);
            let data = take(len)?;
            let fits = messages.len() < MAX_MESSAGES_PER_TICK
                && !topic.is_empty()
                && topic.len() <= MAX_TOPIC
                && data.len() <= MAX_DATA;
            match (std::str::from_utf8(target), std::str::from_utf8(topic)) {
                (Ok(target), Ok(topic)) if fits => {
                    messages.push((target.to_string(), topic.to_string(), data.to_vec()))
                }
                _ => dropped += 1,
            }
        }
        if at != bytes.len() {
            return Err("bytes after the last message".into());
        }
        Ok(Outbox {
            groups,
            messages,
            dropped,
        })
    }
}

fn check(topic: &str, data: &[u8]) -> Result<(), MessageError> {
    if topic.is_empty() || topic.len() > MAX_TOPIC {
        return Err(MessageError(format!("a topic is 1 to {MAX_TOPIC} bytes")));
    }
    if data.len() > MAX_DATA {
        return Err(MessageError(format!(
            "a message is at most {MAX_DATA} bytes"
        )));
    }
    Ok(())
}

/// Check a server input's shard id and size, before it is sent.
pub fn check_send(id: &str, input: &serde_json::Value) -> Result<(), SendError> {
    check_input(id, input).map(|_| ())
}

/// Check a server input's shard id and size; its JSON text.
fn check_input(id: &str, input: &serde_json::Value) -> Result<String, SendError> {
    super::validate_shard_id(id).map_err(|e| SendError::Invalid(e.to_string()))?;
    let text = input.to_string();
    if text.len() > MAX_INPUT {
        return Err(SendError::Invalid(format!(
            "an input is at most {} KB as JSON",
            MAX_INPUT / 1024
        )));
    }
    Ok(text)
}

/// Other machines, as the routing thread last read them.
#[derive(Default)]
pub(super) struct PeerCache {
    /// Live machines other than this one: (id, address), and when read.
    machines: Option<(std::time::Instant, Vec<(String, String)>)>,
    /// Where a shard runs: its machine and address, or none; and when read.
    located: std::collections::HashMap<String, (std::time::Instant, Option<(String, String)>)>,
}

impl WasmShardHost {
    /// Send a message from a server function (`from` is empty) to `to`.
    /// Shards here have it when this returns; delivery is at most once.
    pub fn publish(&self, to: &str, topic: &str, data: &[u8]) -> Result<(), MessageError> {
        let to = Target::parse(to)?;
        check(topic, data)?;
        self.send_message(Routed {
            from: String::new(),
            to,
            topic: topic.to_string(),
            data: Arc::from(data),
        });
        Ok(())
    }

    /// Queue `input` for shard `id`'s next tick, as from the server (the
    /// empty subscriber id). A shard here has it when this returns; for a
    /// shard on another machine it is sent from the routing thread, at most
    /// once.
    pub fn send_input(&self, id: &str, input: &serde_json::Value) -> Result<(), SendError> {
        let text = check_input(id, input)?;
        if let Some(pushed) = self.push_input_here(id, &text) {
            return pushed;
        }
        // Where it runs, read at most every 2 s: a shard that runs nowhere
        // is refused here, as on one machine.
        if self.cluster.get().is_none() || self.located(id).is_none() {
            return Err(SendError::NotFound);
        }
        if self
            .outgoing
            .try_send(Outgoing::Input {
                shard: id.to_string(),
                input: input.clone(),
            })
            .is_err()
        {
            return Err(SendError::Busy(
                "too many messages wait for other machines".into(),
            ));
        }
        Ok(())
    }

    /// A server input another machine sent: for a shard here only.
    pub(super) fn receive_input(
        &self,
        id: &str,
        input: &serde_json::Value,
    ) -> Result<(), SendError> {
        let text = check_input(id, input)?;
        self.push_input_here(id, &text)
            .unwrap_or(Err(SendError::NotFound))
    }

    /// Queue a server input for shard `id` when it runs here; `None` when
    /// it does not.
    fn push_input_here(&self, id: &str, text: &str) -> Option<Result<(), SendError>> {
        let shard = self.registry.get(id).filter(|s| s.is_running())?;
        let format = self
            .kind_of
            .read()
            .unwrap()
            .get(id)
            .and_then(|kind| self.kinds.get(kind))
            .map(|k| k.config.snapshot_format)?;
        let raw = match <pylon_realtime::RawInput as pylon_realtime::ShardInput>::decode_json(
            text, format,
        ) {
            Ok(raw) => raw,
            Err(why) => return Some(Err(SendError::Invalid(why))),
        };
        Some(
            shard
                .push_server_input(raw)
                .map(|_| ())
                .map_err(|e| match e {
                    pylon_realtime::ShardError::Stopped => SendError::NotFound,
                    other => SendError::Busy(other.to_string()),
                }),
        )
    }

    /// Take a module's outbox (from its tick hook): record its groups and
    /// send its messages. A message with a bad target is dropped with a log
    /// line.
    pub(super) fn take_outbox(&self, from: &str, outbox: Outbox) {
        if let Some(groups) = outbox.groups {
            self.groups.lock().unwrap().insert(from.to_string(), groups);
        }
        if outbox.dropped > 0 {
            tracing::warn!(
                "[shard {from}] {} message(s) or group(s) over a limit dropped",
                outbox.dropped
            );
        }
        for (to, topic, data) in outbox.messages {
            match Target::parse(&to) {
                Ok(to) => self.send_message(Routed {
                    from: from.to_string(),
                    to,
                    topic,
                    data: Arc::from(data),
                }),
                Err(e) => tracing::warn!("[shard {from}] message dropped: {e}"),
            }
        }
    }

    /// Deliver to the shards here now; queue the message for other machines
    /// when it may be for one.
    fn send_message(&self, m: Routed) {
        self.deliver_local(&m.from, &m.to, &m.topic, &m.data);
        let local_shard = matches!(&m.to, Target::Shard(id) if self.registry.get(id).is_some());
        let others = self.bus.get().is_some_and(|b| b.is_active()) || self.cluster.get().is_some();
        if local_shard || !others {
            return;
        }
        if self.outgoing.try_send(Outgoing::Message(m)).is_err() {
            tracing::warn!("[shards] too many messages waiting for other machines; one dropped");
        }
    }

    /// Queue `topic`/`data` for each shard here that `to` names, except the
    /// sender. Returns how many took it.
    pub(super) fn deliver_local(
        &self,
        from: &str,
        to: &Target,
        topic: &str,
        data: &Arc<[u8]>,
    ) -> usize {
        let ids: Vec<String> = match to {
            Target::Shard(id) => vec![id.clone()],
            Target::All => self.registry.ids(),
            Target::Group(name) => self
                .groups
                .lock()
                .unwrap()
                .iter()
                .filter(|(_, groups)| groups.iter().any(|g| g == name))
                .map(|(id, _)| id.clone())
                .collect(),
        };
        let mut delivered = 0;
        for id in ids {
            if id == from {
                continue;
            }
            let Some(shard) = self.registry.get(&id).filter(|s| s.is_running()) else {
                continue;
            };
            let took = shard.push_message(ShardMessage {
                from: from.to_string(),
                topic: topic.to_string(),
                data: Arc::clone(data),
            });
            if took {
                delivered += 1;
            } else {
                tracing::debug!("[shard {id}] message {topic} dropped: its queue is full");
            }
        }
        delivered
    }

    /// Pass `m` to the other machines (from the routing thread): on the
    /// cluster bus when there is one, else to each live machine that may
    /// run a shard it names, through that machine's worker.
    pub(super) fn forward(&self, m: &Routed) {
        let data_b64 = base64::engine::general_purpose::STANDARD.encode(&m.data);
        if let Some(bus) = self.bus.get().filter(|b| b.is_active()) {
            let sent = bus.try_publish(&pylon_cluster::Envelope {
                instance_id: bus.instance_id().to_string(),
                kind: BUS_KIND.to_string(),
                payload: serde_json::json!({
                    "from": m.from,
                    "to": m.to.wire(),
                    "topic": m.topic,
                    "data_b64": data_b64,
                }),
            });
            if !sent {
                tracing::debug!(
                    "[shards] message {} dropped: the cluster bus is behind",
                    m.topic
                );
            }
            return;
        }
        let machines = match &m.to {
            Target::Shard(id) => self.located(id).into_iter().collect(),
            _ => self.other_machines(),
        };
        if machines.is_empty() {
            return;
        }
        let op = RemoteOp::Deliver {
            from: m.from.clone(),
            to: m.to.wire(),
            topic: m.topic.clone(),
            data_b64,
        };
        for (machine, address) in machines {
            self.to_peer(&machine, &address, op.clone());
        }
    }

    /// Live machines other than this one, read at most every 2 s.
    fn other_machines(&self) -> Vec<(String, String)> {
        let Some(c) = self.cluster.get() else {
            return Vec::new();
        };
        let mut cache = self.peer_cache.lock().unwrap();
        if let Some((at, machines)) = &cache.machines {
            if at.elapsed() < PEER_CACHE {
                return machines.clone();
            }
        }
        let machines: Vec<(String, String)> = match c.dir.live_machines() {
            Ok(live) => live
                .into_iter()
                .filter(|m| m.id != c.me.id)
                .filter_map(|m| m.address.map(|a| (m.id, a)))
                .collect(),
            Err(e) => {
                tracing::warn!("[shards] reading live machines for messages failed: {e}");
                Vec::new()
            }
        };
        cache.machines = Some((std::time::Instant::now(), machines.clone()));
        machines
    }

    /// The other machine that runs shard `id`, read at most every 2 s.
    fn located(&self, id: &str) -> Option<(String, String)> {
        {
            let cache = self.peer_cache.lock().unwrap();
            if let Some((at, found)) = cache.located.get(id) {
                if at.elapsed() < PEER_CACHE {
                    return found.clone();
                }
            }
        }
        let found = match self.locate(id) {
            pylon_realtime::ShardLocation::Remote {
                machine_id,
                address: Some(address),
                ..
            } => Some((machine_id, address)),
            _ => None,
        };
        let mut cache = self.peer_cache.lock().unwrap();
        let now = std::time::Instant::now();
        cache
            .located
            .retain(|_, (at, _)| now.duration_since(*at) < PEER_CACHE);
        cache.located.insert(id.to_string(), (now, found.clone()));
        found
    }

    /// Queue `op` for `machine`'s worker at `address`, starting one when
    /// there is none for that address (a machine that restarted under the
    /// same id at a new address gets a new one). A full queue drops the
    /// message: that machine is slow or gone. A worker idle for
    /// [`PEER_IDLE`] exits and leaves the map.
    fn to_peer(&self, machine: &str, address: &str, op: RemoteOp) {
        let tx = {
            let mut peers = self.peers.lock().unwrap();
            match peers.get(machine) {
                Some(p) if p.address == address => p.sender.clone(),
                _ => {
                    // A replaced worker ends when its sender drops.
                    peers.remove(machine);
                    let Some(peer) = self.spawn_peer(machine, address) else {
                        return;
                    };
                    let tx = peer.sender.clone();
                    peers.insert(machine.to_string(), peer);
                    tx
                }
            }
        };
        if tx.try_send(op).is_err() {
            tracing::debug!("[shards] message to machine {machine} dropped: it is behind");
        }
    }

    fn spawn_peer(&self, machine: &str, address: &str) -> Option<Peer> {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let (tx, rx) = std::sync::mpsc::sync_channel::<RemoteOp>(PEER_QUEUE);
        let host = self.weak_self.get().cloned();
        let (m, a) = (machine.to_string(), address.to_string());
        let spawned = std::thread::Builder::new()
            .name(format!("pylon-shard-peer-{machine}"))
            .spawn(move || {
                let agent = shard_cluster::call_agent();
                loop {
                    match rx.recv_timeout(PEER_IDLE) {
                        Ok(op) => {
                            if let Err(e) = shard_cluster::call_with(&agent, &m, &a, &op) {
                                tracing::debug!("[shards] message to machine {m} dropped: {e}");
                            }
                        }
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                            // Idle: leave the map, unless a newer worker
                            // took the slot already.
                            if let Some(host) = host.as_ref().and_then(|w| w.upgrade()) {
                                let mut peers = host.peers.lock().unwrap();
                                if peers.get(&m).is_some_and(|p| p.serial == serial) {
                                    peers.remove(&m);
                                }
                            }
                            return;
                        }
                        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
                    }
                }
            });
        match spawned {
            Ok(_) => Some(Peer {
                address: address.to_string(),
                serial,
                sender: tx,
            }),
            Err(e) => {
                tracing::warn!("[shards] no worker for machine {machine}: {e}");
                None
            }
        }
    }

    /// Other machines with a message worker, for tests.
    #[cfg(test)]
    pub(super) fn peer_count(&self) -> usize {
        self.peers.lock().unwrap().len()
    }

    /// A message another machine sent (a signed call or the bus): deliver
    /// it here only.
    pub(super) fn receive(
        &self,
        from: &str,
        to: &str,
        topic: &str,
        data_b64: &str,
    ) -> Result<usize, String> {
        let to = Target::parse(to).map_err(|e| e.0)?;
        let data = base64::engine::general_purpose::STANDARD
            .decode(data_b64)
            .map_err(|e| format!("message data is not base64: {e}"))?;
        check(topic, &data).map_err(|e| e.0)?;
        Ok(self.deliver_local(from, &to, topic, &Arc::from(data)))
    }

    /// Carry messages on `bus` between machines, and take the ones other
    /// machines publish on it.
    pub fn attach_bus(self: &Arc<Self>, bus: Arc<dyn pylon_cluster::ClusterBus>) {
        if !bus.is_active() {
            return;
        }
        let own = bus.instance_id().to_string();
        let weak = Arc::downgrade(self);
        bus.subscribe(Arc::new(move |envelope: pylon_cluster::Envelope| {
            if envelope.kind != BUS_KIND || envelope.instance_id == own {
                return;
            }
            let Some(host) = weak.upgrade() else { return };
            let field = |k: &str| {
                envelope
                    .payload
                    .get(k)
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
            };
            if let Err(e) = host.receive(
                field("from"),
                field("to"),
                field("topic"),
                field("data_b64"),
            ) {
                tracing::warn!("[shards] a message on the cluster bus was dropped: {e}");
            }
        }));
        let _ = self.bus.set(bus);
    }

    /// Pass queued messages to other machines until the host goes away.
    pub(super) fn run_routing(
        weak: std::sync::Weak<Self>,
        queue: std::sync::mpsc::Receiver<Outgoing>,
    ) {
        while let Ok(m) = queue.recv() {
            let Some(host) = weak.upgrade() else { return };
            if host.stopped.load(std::sync::atomic::Ordering::Acquire) {
                return;
            }
            match m {
                Outgoing::Message(m) => host.forward(&m),
                Outgoing::Input { shard, input } => match host.located(&shard) {
                    Some((machine, address)) => {
                        host.to_peer(&machine, &address, RemoteOp::Input { id: shard, input })
                    }
                    None => tracing::debug!(
                        "[shard {shard}] server input dropped: no machine runs the shard"
                    ),
                },
            }
        }
    }

    /// Forget a stopped shard's groups.
    pub(super) fn forget_groups(&self, id: &str) {
        self.groups.lock().unwrap().remove(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outbox(groups: Option<&[&str]>, messages: &[(&str, &str, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        match groups {
            Some(g) => {
                out.push(1);
                out.extend_from_slice(&(g.len() as u16).to_le_bytes());
                for name in g {
                    out.extend_from_slice(&(name.len() as u16).to_le_bytes());
                    out.extend_from_slice(name.as_bytes());
                }
            }
            None => out.push(0),
        }
        out.extend_from_slice(&(messages.len() as u32).to_le_bytes());
        for (to, topic, data) in messages {
            out.extend_from_slice(&(to.len() as u16).to_le_bytes());
            out.extend_from_slice(to.as_bytes());
            out.extend_from_slice(&(topic.len() as u16).to_le_bytes());
            out.extend_from_slice(topic.as_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(data);
        }
        out
    }

    #[test]
    fn an_outbox_parses_drops_messages_over_a_limit_and_refuses_bad_frames() {
        let ok = Outbox::parse(&outbox(Some(&["north"]), &[("all", "relic", b"taken")])).unwrap();
        assert_eq!(ok.dropped, 0);
        assert_eq!(ok.groups, Some(vec!["north".to_string()]));
        assert_eq!(
            ok.messages,
            vec![("all".to_string(), "relic".to_string(), b"taken".to_vec())]
        );
        assert_eq!(Outbox::parse(&outbox(None, &[])).unwrap().groups, None);

        let mut short = outbox(None, &[("all", "t", b"abc")]);
        short.pop();
        assert!(Outbox::parse(&short).is_err());
        let mut extra = outbox(None, &[]);
        extra.push(0);
        assert!(Outbox::parse(&extra).is_err());
        // Over a limit: that message is dropped and counted; the rest stay.
        let big = vec![0u8; MAX_DATA + 1];
        let over =
            Outbox::parse(&outbox(None, &[("all", "t", &big), ("all", "ok", b"1")])).unwrap();
        assert_eq!((over.messages.len(), over.dropped), (1, 1));
        let long_topic = "t".repeat(MAX_TOPIC + 1);
        let over = Outbox::parse(&outbox(
            None,
            &[("all", &long_topic, b""), ("all", "", b"")],
        ))
        .unwrap();
        assert_eq!((over.messages.len(), over.dropped), (0, 2));
        let many: Vec<(&str, &str, &[u8])> = (0..MAX_MESSAGES_PER_TICK + 3)
            .map(|_| ("all", "t", &b""[..]))
            .collect();
        let capped = Outbox::parse(&outbox(None, &many)).unwrap();
        assert_eq!(
            (capped.messages.len(), capped.dropped),
            (MAX_MESSAGES_PER_TICK, 3)
        );
        // A length past the end, or a huge count, is an error, not a panic.
        assert!(Outbox::parse(&[0, 1, 0, 0, 0, 0xff, 0xff]).is_err());
        assert!(Outbox::parse(&[0, 0xff, 0xff, 0xff, 0xff]).is_err());
    }

    #[test]
    fn targets_parse() {
        assert_eq!(Target::parse("all").unwrap(), Target::All);
        assert_eq!(
            Target::parse("shard:z-1").unwrap(),
            Target::Shard("z-1".into())
        );
        assert_eq!(
            Target::parse("group:north").unwrap(),
            Target::Group("north".into())
        );
        assert!(Target::parse("north").is_err());
        assert!(Target::parse("group:").is_err());
        assert!(Target::parse("shard:has space").is_err());
    }

    fn host() -> std::sync::Arc<WasmShardHost> {
        let zone = crate::shard_wasm::WasmShardKind::compile(
            "zone",
            include_bytes!("../../../../examples/shard-arena/shards/zone.wasm"),
            pylon_realtime::ShardConfig::default(),
            crate::shard_wasm::WasmLimits::default(),
        )
        .unwrap();
        WasmShardHost::new(vec![zone])
    }

    /// A module that returns a huge count of records is read up to
    /// MAX_RECORDS; the rest count as dropped, unread.
    #[test]
    fn an_outbox_is_read_up_to_its_record_limit() {
        let mut out = vec![0u8]; // no groups
        let count: u32 = 100_000;
        out.extend_from_slice(&count.to_le_bytes());
        for _ in 0..count {
            // An empty target, an empty topic, no data: refused.
            out.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0]);
        }
        let parsed = Outbox::parse(&out).unwrap();
        assert!(parsed.messages.is_empty());
        assert_eq!(parsed.dropped, count as usize);

        let mut groups = vec![1u8];
        groups.extend_from_slice(&((MAX_RECORDS + 1) as u16).to_le_bytes());
        assert!(Outbox::parse(&groups).unwrap_err().contains("groups"));
    }

    /// A machine that came back at a new address gets a new worker; an
    /// idle worker leaves the map.
    #[test]
    fn peer_workers_follow_the_address_and_retire_when_idle() {
        let host = host();
        host.to_peer("m", "http://127.0.0.1:1", RemoteOp::List);
        assert_eq!(host.peer_count(), 1);
        host.to_peer("m", "http://127.0.0.1:2", RemoteOp::List);
        assert_eq!(host.peer_count(), 1);
        assert_eq!(
            host.peers.lock().unwrap()["m"].address,
            "http://127.0.0.1:2"
        );
        host.to_peer("n", "http://127.0.0.1:3", RemoteOp::List);
        assert_eq!(host.peer_count(), 2);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while host.peer_count() > 0 {
            assert!(std::time::Instant::now() < deadline, "idle workers stayed");
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
}
