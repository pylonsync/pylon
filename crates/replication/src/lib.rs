//! Entity replication for Pylon realtime shards.
//!
//! A simulation keeps its replicated state in a [`Replicated`] store:
//! entities with stable u64 ids, a 3D position, and components (bytes in
//! the game's own encoding, keyed by a u8 id). Every change bumps the
//! store's change counter, so the server can tell what changed since it
//! last wrote to a given subscriber without diffing snapshots.
//!
//! The server sends each subscriber replication frames ([`frame`]): spawn
//! (full state) for entities that came into view, update (only changed
//! axes and components) for entities in view, and despawn for entities
//! that left view. A client applies them to a [`ReplicaTable`].
//!
//! A WebAssembly shard module records its store's changes
//! ([`Replicated::record_changes`]) and hands them to the host each tick;
//! the host applies them to its copy ([`Replicated::apply_changes`]).
//!
//! This crate has no dependencies, so guests build it for
//! wasm32-unknown-unknown.

use std::collections::BTreeMap;

pub mod frame;
pub mod varint;

pub use frame::{DecodeError, FrameSummary, ReplicaEntity, ReplicaTable};

/// A stable entity id, chosen by the simulation.
pub type EntityId = u64;

/// A component id: the game decides what each one holds.
pub type ComponentId = u8;

/// One replicated entity.
#[derive(Debug, Clone, PartialEq)]
pub struct Entity {
    pub pos: [f32; 3],
    /// Change counter value of the last position change.
    pub pos_seq: u64,
    /// Components. A removed component stays as `bytes: None` so readers
    /// that last saw it can be told it is gone.
    pub components: BTreeMap<ComponentId, Component>,
    /// Change counter value of the last change of any kind.
    pub seq: u64,
    /// Change counter value of this entity's spawn. An id despawned and
    /// spawned again gets a new value, so readers can tell a new entity
    /// from the old one.
    pub spawn_seq: u64,
}

impl Entity {
    /// True when anything changed after change counter value `since`.
    pub fn changed_since(&self, since: u64) -> bool {
        self.seq > since
    }

    /// A component's bytes, if present.
    pub fn component(&self, id: ComponentId) -> Option<&[u8]> {
        self.components.get(&id).and_then(|c| c.bytes.as_deref())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Component {
    pub bytes: Option<Vec<u8>>,
    pub seq: u64,
}

/// The replicated state of a simulation.
#[derive(Debug, Clone, PartialEq)]
pub struct Replicated {
    entities: BTreeMap<EntityId, Entity>,
    seq: u64,
    log: Option<Vec<u8>>,
    /// Bytes held in components.
    component_bytes: usize,
    /// Unique per store made in this process, so a WebAssembly module can
    /// tell when the game swapped in a new store.
    store_id: u64,
}

impl Default for Replicated {
    fn default() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Self {
            entities: BTreeMap::new(),
            seq: 0,
            log: None,
            component_bytes: 0,
            store_id: NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        }
    }
}

mod op {
    pub const SPAWN: u8 = 1;
    pub const DESPAWN: u8 = 2;
    pub const POS: u8 = 3;
    pub const SET: u8 = 4;
    pub const REMOVE: u8 = 5;
}

/// Why a change log did not apply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeError(pub String);

impl std::fmt::Display for ChangeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ChangeError {}

impl Replicated {
    pub fn new() -> Self {
        Self::default()
    }

    /// The change counter. Every change raises it by one.
    pub fn seq(&self) -> u64 {
        self.seq
    }

    pub fn len(&self) -> usize {
        self.entities.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    /// Bytes held in components, for limits.
    pub fn component_bytes(&self) -> usize {
        self.component_bytes
    }

    /// This store's process-unique id.
    pub fn store_id(&self) -> u64 {
        self.store_id
    }

    /// Remove every entity but keep the change counter running, so an id
    /// spawned again afterwards gets a spawn counter no reader has seen and
    /// reads as a new entity.
    pub fn clear(&mut self) {
        self.entities.clear();
        self.component_bytes = 0;
        self.bump();
    }

    pub fn get(&self, id: EntityId) -> Option<&Entity> {
        self.entities.get(&id)
    }

    pub fn contains(&self, id: EntityId) -> bool {
        self.entities.contains_key(&id)
    }

    /// Entities in id order.
    pub fn iter(&self) -> impl Iterator<Item = (EntityId, &Entity)> {
        self.entities.iter().map(|(id, e)| (*id, e))
    }

    fn bump(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    /// Add an entity. Returns false (and changes nothing) when the id is
    /// taken.
    pub fn spawn(&mut self, id: EntityId, pos: [f32; 3]) -> bool {
        if self.entities.contains_key(&id) {
            return false;
        }
        let seq = self.bump();
        self.entities.insert(
            id,
            Entity {
                pos,
                pos_seq: seq,
                components: BTreeMap::new(),
                seq,
                spawn_seq: seq,
            },
        );
        if let Some(log) = &mut self.log {
            log.push(op::SPAWN);
            varint::write_u64(log, id);
            write_pos(log, pos);
        }
        true
    }

    /// Remove an entity. Returns false when there was none.
    pub fn despawn(&mut self, id: EntityId) -> bool {
        let Some(e) = self.entities.remove(&id) else {
            return false;
        };
        self.component_bytes -= e
            .components
            .values()
            .map(|c| c.bytes.as_ref().map_or(0, Vec::len))
            .sum::<usize>();
        self.bump();
        if let Some(log) = &mut self.log {
            log.push(op::DESPAWN);
            varint::write_u64(log, id);
        }
        true
    }

    /// Move an entity. A position equal to the current one is not a change.
    pub fn set_pos(&mut self, id: EntityId, pos: [f32; 3]) -> bool {
        let Some(current) = self.entities.get(&id).map(|e| e.pos) else {
            return false;
        };
        if current.map(f32::to_bits) == pos.map(f32::to_bits) {
            return true;
        }
        let seq = self.bump();
        let e = self.entities.get_mut(&id).expect("checked above");
        e.pos = pos;
        e.pos_seq = seq;
        e.seq = seq;
        if let Some(log) = &mut self.log {
            log.push(op::POS);
            varint::write_u64(log, id);
            write_pos(log, pos);
        }
        true
    }

    /// Set a component. Bytes equal to the current ones are not a change.
    pub fn set_component(&mut self, id: EntityId, component: ComponentId, bytes: &[u8]) -> bool {
        let Some(e) = self.entities.get(&id) else {
            return false;
        };
        if e.component(component) == Some(bytes) {
            return true;
        }
        let seq = self.bump();
        let e = self.entities.get_mut(&id).expect("checked above");
        let old = e.components.insert(
            component,
            Component {
                bytes: Some(bytes.to_vec()),
                seq,
            },
        );
        e.seq = seq;
        self.component_bytes += bytes.len();
        self.component_bytes -= old.and_then(|c| c.bytes).map_or(0, |b| b.len());
        if let Some(log) = &mut self.log {
            log.push(op::SET);
            varint::write_u64(log, id);
            log.push(component);
            varint::write_u64(log, bytes.len() as u64);
            log.extend_from_slice(bytes);
        }
        true
    }

    /// Remove a component. Returns false when the entity or component is
    /// missing.
    pub fn remove_component(&mut self, id: EntityId, component: ComponentId) -> bool {
        let present = self
            .entities
            .get(&id)
            .is_some_and(|e| e.component(component).is_some());
        if !present {
            return false;
        }
        let seq = self.bump();
        let e = self.entities.get_mut(&id).expect("checked above");
        let old = e
            .components
            .insert(component, Component { bytes: None, seq });
        e.seq = seq;
        self.component_bytes -= old.and_then(|c| c.bytes).map_or(0, |b| b.len());
        if let Some(log) = &mut self.log {
            log.push(op::REMOVE);
            varint::write_u64(log, id);
            log.push(component);
        }
        true
    }

    /// Start or stop keeping a log of changes for [`Replicated::take_changes`].
    /// A WebAssembly shard module turns this on.
    pub fn record_changes(&mut self, on: bool) {
        self.log = on.then(Vec::new);
    }

    /// The changes since the last call, as bytes for
    /// [`Replicated::apply_changes`]. Empty when recording is off.
    pub fn take_changes(&mut self) -> Vec<u8> {
        match &mut self.log {
            Some(log) => std::mem::take(log),
            None => Vec::new(),
        }
    }

    /// Every entity and component as a change log that rebuilds this
    /// store from empty (spawns, then component sets).
    pub fn full_changes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for (id, e) in &self.entities {
            out.push(op::SPAWN);
            varint::write_u64(&mut out, *id);
            write_pos(&mut out, e.pos);
            for (cid, c) in &e.components {
                if let Some(bytes) = &c.bytes {
                    out.push(op::SET);
                    varint::write_u64(&mut out, *id);
                    out.push(*cid);
                    varint::write_u64(&mut out, bytes.len() as u64);
                    out.extend_from_slice(bytes);
                }
            }
        }
        out
    }

    /// Apply changes another store recorded. Stops at the first malformed
    /// or inconsistent change; the changes before it stay applied.
    pub fn apply_changes(&mut self, mut bytes: &[u8]) -> Result<(), ChangeError> {
        let err = |m: &str| ChangeError(m.to_string());
        while let Some((&tag, rest)) = bytes.split_first() {
            bytes = rest;
            let id = varint::read_u64(&mut bytes).ok_or_else(|| err("truncated entity id"))?;
            match tag {
                op::SPAWN => {
                    let pos = read_pos(&mut bytes).ok_or_else(|| err("truncated position"))?;
                    if !self.spawn(id, pos) {
                        return Err(ChangeError(format!("spawn of existing entity {id}")));
                    }
                }
                op::DESPAWN => {
                    self.despawn(id);
                }
                op::POS => {
                    let pos = read_pos(&mut bytes).ok_or_else(|| err("truncated position"))?;
                    if !self.set_pos(id, pos) {
                        return Err(ChangeError(format!("move of missing entity {id}")));
                    }
                }
                op::SET => {
                    let (&cid, rest) = bytes
                        .split_first()
                        .ok_or_else(|| err("truncated component"))?;
                    bytes = rest;
                    let len =
                        varint::read_u64(&mut bytes).ok_or_else(|| err("truncated length"))?;
                    let len = usize::try_from(len).map_err(|_| err("component too large"))?;
                    if bytes.len() < len {
                        return Err(err("component runs past the end"));
                    }
                    let (value, rest) = bytes.split_at(len);
                    bytes = rest;
                    if !self.set_component(id, cid, value) {
                        return Err(ChangeError(format!("component on missing entity {id}")));
                    }
                }
                op::REMOVE => {
                    let (&cid, rest) = bytes
                        .split_first()
                        .ok_or_else(|| err("truncated component"))?;
                    bytes = rest;
                    self.remove_component(id, cid);
                }
                other => return Err(ChangeError(format!("unknown change tag {other}"))),
            }
        }
        Ok(())
    }
}

fn write_pos(out: &mut Vec<u8>, pos: [f32; 3]) {
    for v in pos {
        out.extend_from_slice(&v.to_le_bytes());
    }
}

fn read_pos(bytes: &mut &[u8]) -> Option<[f32; 3]> {
    if bytes.len() < 12 {
        return None;
    }
    let (head, rest) = bytes.split_at(12);
    *bytes = rest;
    let f = |i: usize| f32::from_le_bytes(head[i..i + 4].try_into().unwrap());
    Some([f(0), f(4), f(8)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changes_bump_the_counter_and_no_ops_do_not() {
        let mut r = Replicated::new();
        assert!(r.spawn(1, [0.0, 0.0, 0.0]));
        assert!(!r.spawn(1, [5.0, 0.0, 0.0]));
        let s = r.seq();
        r.set_pos(1, [0.0, 0.0, 0.0]);
        r.set_component(1, 3, b"x");
        let after = r.seq();
        assert_eq!(after, s + 1);
        r.set_component(1, 3, b"x");
        assert_eq!(r.seq(), after);
        assert!(r.get(1).unwrap().changed_since(s));
        assert!(!r.get(1).unwrap().changed_since(after));
        assert!(r.remove_component(1, 3));
        assert_eq!(r.get(1).unwrap().component(3), None);
        assert!(!r.remove_component(1, 3));
    }

    #[test]
    fn a_recorded_log_rebuilds_the_same_entities() {
        let mut guest = Replicated::new();
        guest.record_changes(true);
        guest.spawn(7, [1.0, 2.0, 3.0]);
        guest.spawn(9, [0.0, 0.0, 0.0]);
        guest.set_pos(7, [1.5, 2.0, 3.0]);
        guest.set_component(7, 1, b"hp=10");
        guest.set_component(9, 2, &[]);
        guest.remove_component(9, 2);
        guest.despawn(9);
        let log = guest.take_changes();
        assert!(guest.take_changes().is_empty());

        let mut host = Replicated::new();
        host.apply_changes(&log).unwrap();
        let summary = |r: &Replicated| {
            r.iter()
                .map(|(id, e)| (id, e.pos, e.component(1).map(<[u8]>::to_vec)))
                .collect::<Vec<_>>()
        };
        assert_eq!(summary(&host), summary(&guest));
    }

    #[test]
    fn a_full_dump_rebuilds_the_store() {
        let mut a = Replicated::new();
        a.spawn(1, [1.0, 2.0, 3.0]);
        a.spawn(4, [0.0; 3]);
        a.set_component(1, 2, b"x");
        a.set_component(4, 2, b"y");
        a.remove_component(4, 2);
        let mut b = Replicated::new();
        b.apply_changes(&a.full_changes()).unwrap();
        let view = |r: &Replicated| {
            r.iter()
                .map(|(id, e)| (id, e.pos, e.component(2).map(<[u8]>::to_vec)))
                .collect::<Vec<_>>()
        };
        assert_eq!(view(&a), view(&b));
    }

    #[test]
    fn component_bytes_track_sets_removals_and_despawns() {
        let mut r = Replicated::new();
        r.spawn(1, [0.0; 3]);
        r.set_component(1, 1, &[0; 10]);
        r.set_component(1, 2, &[0; 5]);
        assert_eq!(r.component_bytes(), 15);
        r.set_component(1, 1, &[0; 3]);
        assert_eq!(r.component_bytes(), 8);
        r.remove_component(1, 2);
        assert_eq!(r.component_bytes(), 3);
        r.despawn(1);
        assert_eq!(r.component_bytes(), 0);
    }

    #[test]
    fn clear_keeps_the_counter_so_a_respawned_id_is_new() {
        let mut r = Replicated::new();
        r.spawn(7, [0.0; 3]);
        let first = r.get(7).unwrap().spawn_seq;
        r.clear();
        r.spawn(7, [0.0; 3]);
        assert!(r.get(7).unwrap().spawn_seq > first);
        assert_ne!(Replicated::new().store_id(), Replicated::new().store_id());
    }

    #[test]
    fn bad_logs_are_refused_without_panicking() {
        let mut r = Replicated::new();
        assert!(r.apply_changes(&[op::SPAWN]).is_err());
        assert!(r.apply_changes(&[op::SPAWN, 1, 0, 0]).is_err());
        assert!(r
            .apply_changes(&[op::POS, 5, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
            .is_err());
        assert!(r.apply_changes(&[99, 1]).is_err());
        let mut long = vec![op::SPAWN, 1];
        long.extend_from_slice(&[0; 12]);
        long.extend_from_slice(&[op::SET, 1, 4, 0xff, 0xff, 0xff, 0xff, 0x0f]);
        assert!(r.apply_changes(&long).is_err());
    }
}
