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
/// How long the list of live machines, and where a shard runs, are reused.
const PEER_CACHE: std::time::Duration = std::time::Duration::from_secs(2);
/// Limits on what a module or a function sends.
const MAX_MESSAGES_PER_TICK: usize = 64;
const MAX_TOPIC: usize = 128;
const MAX_GROUP: usize = 128;
const MAX_GROUPS: usize = 64;
const MAX_DATA: usize = 64 * 1024;

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
        for _ in 0..count {
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
        if self.outgoing.try_send(m).is_err() {
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

    /// Queue `op` for `machine`'s worker, starting one the first time. A
    /// full queue drops the message: that machine is slow or gone.
    fn to_peer(&self, machine: &str, address: &str, op: RemoteOp) {
        let mut peers = self.peers.lock().unwrap();
        let tx = peers
            .entry(machine.to_string())
            .or_insert_with(|| {
                let (tx, rx) = std::sync::mpsc::sync_channel::<RemoteOp>(PEER_QUEUE);
                let (machine, address) = (machine.to_string(), address.to_string());
                let _ = std::thread::Builder::new()
                    .name(format!("pylon-shard-peer-{machine}"))
                    .spawn(move || {
                        let agent = shard_cluster::call_agent();
                        while let Ok(op) = rx.recv() {
                            if let Err(e) =
                                shard_cluster::call_with(&agent, &machine, &address, &op)
                            {
                                tracing::debug!(
                                    "[shards] message to machine {machine} dropped: {e}"
                                );
                            }
                        }
                    });
                tx
            })
            .clone();
        drop(peers);
        if tx.try_send(op).is_err() {
            tracing::debug!("[shards] message to machine {machine} dropped: it is behind");
        }
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
        queue: std::sync::mpsc::Receiver<Routed>,
    ) {
        while let Ok(m) = queue.recv() {
            let Some(host) = weak.upgrade() else { return };
            if host.stopped.load(std::sync::atomic::Ordering::Acquire) {
                return;
            }
            host.forward(&m);
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
}
