//! The replication frame: what one subscriber receives per tick.
//!
//! Integers are LEB128 varints ([`crate::varint`]); signed values are
//! zigzag varints. Positions travel quantized: `round(value / precision)`.
//!
//! ```text
//! u8      version (1)
//! u8      flags: bit 0 FULL: the client clears its table first
//! f32 LE  precision
//! varint  despawn count, then ids: the first absolute, each next as the
//!         difference from the previous (ids ascend)
//! varint  spawn count, then per entity (ids ascend, delta-coded):
//!           id, x, y, z (zigzag, absolute quantized), components
//! varint  update count, then per entity (ids ascend, delta-coded):
//!           id, u8 mask (1 x, 2 y, 4 z, 8 components),
//!           the changed axes (zigzag, quantized difference from the value
//!           last sent), components when bit 8 is set
//! components: varint count, then per component:
//!           u8 id, varint (length + 1), bytes; length field 0 = removed
//! ```
//!
//! A spawn carries every present component. An update carries the ones
//! that changed or were removed since that subscriber was last updated.

use std::collections::BTreeMap;

use crate::varint;
use crate::{ComponentId, EntityId};

pub const VERSION: u8 = 1;
pub const FLAG_FULL: u8 = 1;

/// Bits of an update's mask byte.
pub mod mask {
    pub const X: u8 = 1;
    pub const Y: u8 = 2;
    pub const Z: u8 = 4;
    pub const COMPONENTS: u8 = 8;
}

/// `value / precision`, rounded. Non-finite values quantize to 0.
pub fn quantize(value: f32, precision: f32) -> i64 {
    if !value.is_finite() {
        return 0;
    }
    let q = (value as f64 / precision as f64).round();
    q.clamp(i64::MIN as f64, i64::MAX as f64) as i64
}

pub fn quantize3(pos: [f32; 3], precision: f32) -> [i64; 3] {
    pos.map(|v| quantize(v, precision))
}

/// A component change inside an update: new bytes, or removed.
pub type ComponentChange<'a> = (ComponentId, Option<&'a [u8]>);

/// Encode an update body (everything after the id): the mask, the changed
/// axes, and the component changes. Returns the body.
pub fn encode_update_body(delta: [i64; 3], components: &[ComponentChange<'_>]) -> Vec<u8> {
    let mut body = Vec::with_capacity(8);
    let mut m = 0u8;
    for (i, bit) in [mask::X, mask::Y, mask::Z].into_iter().enumerate() {
        if delta[i] != 0 {
            m |= bit;
        }
    }
    if !components.is_empty() {
        m |= mask::COMPONENTS;
    }
    body.push(m);
    for d in delta {
        if d != 0 {
            varint::write_i64(&mut body, d);
        }
    }
    if !components.is_empty() {
        write_components(&mut body, components.iter().copied());
    }
    body
}

fn write_components<'a>(
    out: &mut Vec<u8>,
    components: impl ExactSizeIterator<Item = ComponentChange<'a>>,
) {
    varint::write_u64(out, components.len() as u64);
    for (id, bytes) in components {
        out.push(id);
        match bytes {
            Some(b) => {
                varint::write_u64(out, b.len() as u64 + 1);
                out.extend_from_slice(b);
            }
            None => varint::write_u64(out, 0),
        }
    }
}

/// Builds one frame. Add despawns in any order; add spawns and updates in
/// ascending id order.
#[derive(Debug)]
pub struct FrameBuilder {
    full: bool,
    precision: f32,
    despawns: Vec<EntityId>,
    spawns: Vec<u8>,
    spawn_count: u64,
    last_spawn: Option<EntityId>,
    updates: Vec<u8>,
    update_count: u64,
    last_update: Option<EntityId>,
}

impl FrameBuilder {
    pub fn new(full: bool, precision: f32) -> Self {
        Self {
            full,
            precision,
            despawns: Vec::new(),
            spawns: Vec::new(),
            spawn_count: 0,
            last_spawn: None,
            updates: Vec::new(),
            update_count: 0,
            last_update: None,
        }
    }

    pub fn despawn(&mut self, id: EntityId) {
        self.despawns.push(id);
    }

    /// Add a spawn. Ids must ascend across calls.
    pub fn spawn<'a>(
        &mut self,
        id: EntityId,
        q: [i64; 3],
        components: impl ExactSizeIterator<Item = (ComponentId, &'a [u8])>,
    ) {
        write_delta_id(&mut self.spawns, &mut self.last_spawn, id);
        for v in q {
            varint::write_i64(&mut self.spawns, v);
        }
        write_components(&mut self.spawns, components.map(|(c, b)| (c, Some(b))));
        self.spawn_count += 1;
    }

    /// Add an update with a body from [`encode_update_body`]. Ids must
    /// ascend across calls.
    pub fn update(&mut self, id: EntityId, body: &[u8]) {
        write_delta_id(&mut self.updates, &mut self.last_update, id);
        self.updates.extend_from_slice(body);
        self.update_count += 1;
    }

    pub fn is_empty(&self) -> bool {
        !self.full && self.despawns.is_empty() && self.spawn_count == 0 && self.update_count == 0
    }

    /// The frame bytes.
    pub fn finish(mut self) -> Vec<u8> {
        self.despawns.sort_unstable();
        self.despawns.dedup();
        let mut out = Vec::with_capacity(16 + self.spawns.len() + self.updates.len());
        out.push(VERSION);
        out.push(if self.full { FLAG_FULL } else { 0 });
        out.extend_from_slice(&self.precision.to_le_bytes());
        varint::write_u64(&mut out, self.despawns.len() as u64);
        let mut last = None;
        for id in &self.despawns {
            write_delta_id(&mut out, &mut last, *id);
        }
        varint::write_u64(&mut out, self.spawn_count);
        out.extend_from_slice(&self.spawns);
        varint::write_u64(&mut out, self.update_count);
        out.extend_from_slice(&self.updates);
        out
    }
}

fn write_delta_id(out: &mut Vec<u8>, last: &mut Option<EntityId>, id: EntityId) {
    match *last {
        Some(prev) => {
            debug_assert!(id > prev, "ids must ascend");
            varint::write_u64(out, id.wrapping_sub(prev));
        }
        None => varint::write_u64(out, id),
    }
    *last = Some(id);
}

// ---------------------------------------------------------------------------
// Decoding
// ---------------------------------------------------------------------------

/// Why a frame did not decode or apply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeError(pub String);

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DecodeError {}

/// An entity as a client sees it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReplicaEntity {
    /// The quantized position; deltas apply to it exactly.
    pub q: [i64; 3],
    pub components: BTreeMap<ComponentId, Vec<u8>>,
}

impl ReplicaEntity {
    pub fn pos(&self, precision: f32) -> [f32; 3] {
        self.q.map(|v| (v as f64 * precision as f64) as f32)
    }
}

/// What one frame did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FrameSummary {
    pub full: bool,
    pub spawned: Vec<EntityId>,
    pub updated: Vec<EntityId>,
    pub despawned: Vec<EntityId>,
}

/// A client's table of the entities it has been told about.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReplicaTable {
    pub entities: BTreeMap<EntityId, ReplicaEntity>,
    pub precision: f32,
}

/// Longest id list a frame may declare, so a hostile count cannot make a
/// decoder allocate without bound.
const MAX_COUNT: u64 = 1 << 24;

impl ReplicaTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn pos(&self, id: EntityId) -> Option<[f32; 3]> {
        self.entities.get(&id).map(|e| e.pos(self.precision))
    }

    /// Apply one frame. On an error the table may hold part of the frame;
    /// the caller should treat the connection as broken.
    pub fn apply(&mut self, frame: &[u8]) -> Result<FrameSummary, DecodeError> {
        let err = |m: &str| DecodeError(m.to_string());
        let mut b = frame;
        let take = |b: &mut &[u8], n: usize| -> Result<Vec<u8>, DecodeError> {
            if b.len() < n {
                return Err(DecodeError("frame ends early".into()));
            }
            let (head, rest) = b.split_at(n);
            *b = rest;
            Ok(head.to_vec())
        };
        let head = take(&mut b, 6)?;
        if head[0] != VERSION {
            return Err(DecodeError(format!("replication version {}", head[0])));
        }
        let full = head[1] & FLAG_FULL != 0;
        let precision = f32::from_le_bytes(head[2..6].try_into().unwrap());
        if !(precision.is_finite() && precision > 0.0) {
            return Err(err("bad precision"));
        }
        if full {
            self.entities.clear();
        }
        self.precision = precision;
        let mut summary = FrameSummary {
            full,
            ..FrameSummary::default()
        };
        let count = |b: &mut &[u8]| -> Result<u64, DecodeError> {
            let n = varint::read_u64(b).ok_or_else(|| err("bad count"))?;
            if n > MAX_COUNT {
                return Err(err("count too large"));
            }
            Ok(n)
        };
        let next_id =
            |b: &mut &[u8], last: &mut Option<EntityId>| -> Result<EntityId, DecodeError> {
                let v = varint::read_u64(b).ok_or_else(|| err("bad id"))?;
                let id = match *last {
                    Some(prev) => prev
                        .checked_add(v)
                        .filter(|_| v > 0)
                        .ok_or_else(|| err("ids do not ascend"))?,
                    None => v,
                };
                *last = Some(id);
                Ok(id)
            };

        let n = count(&mut b)?;
        let mut last = None;
        for _ in 0..n {
            let id = next_id(&mut b, &mut last)?;
            self.entities.remove(&id);
            summary.despawned.push(id);
        }

        let n = count(&mut b)?;
        let mut last = None;
        for _ in 0..n {
            let id = next_id(&mut b, &mut last)?;
            let mut q = [0i64; 3];
            for v in &mut q {
                *v = varint::read_i64(&mut b).ok_or_else(|| err("bad position"))?;
            }
            let mut entity = ReplicaEntity {
                q,
                components: BTreeMap::new(),
            };
            read_components(&mut b, &mut entity.components)?;
            self.entities.insert(id, entity);
            summary.spawned.push(id);
        }

        let n = count(&mut b)?;
        let mut last = None;
        for _ in 0..n {
            let id = next_id(&mut b, &mut last)?;
            let (&m, rest) = b.split_first().ok_or_else(|| err("frame ends early"))?;
            b = rest;
            let entity = self
                .entities
                .get_mut(&id)
                .ok_or_else(|| DecodeError(format!("update for unknown entity {id}")))?;
            for (i, bit) in [mask::X, mask::Y, mask::Z].into_iter().enumerate() {
                if m & bit != 0 {
                    let d = varint::read_i64(&mut b).ok_or_else(|| err("bad delta"))?;
                    entity.q[i] = entity.q[i].wrapping_add(d);
                }
            }
            if m & mask::COMPONENTS != 0 {
                read_components(&mut b, &mut entity.components)?;
            }
            summary.updated.push(id);
        }
        if !b.is_empty() {
            return Err(err("trailing bytes"));
        }
        Ok(summary)
    }
}

fn read_components(
    b: &mut &[u8],
    into: &mut BTreeMap<ComponentId, Vec<u8>>,
) -> Result<(), DecodeError> {
    let err = |m: &str| DecodeError(m.to_string());
    let n = varint::read_u64(b).ok_or_else(|| err("bad component count"))?;
    if n > 256 {
        return Err(err("more than 256 components"));
    }
    for _ in 0..n {
        let (&id, rest) = b.split_first().ok_or_else(|| err("frame ends early"))?;
        *b = rest;
        let len = varint::read_u64(b).ok_or_else(|| err("bad component length"))?;
        if len == 0 {
            into.remove(&id);
            continue;
        }
        let len = usize::try_from(len - 1).map_err(|_| err("component too large"))?;
        if b.len() < len {
            return Err(err("component runs past the end"));
        }
        let (value, rest) = b.split_at(len);
        *b = rest;
        into.insert(id, value.to_vec());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_update_despawn_round_trip() {
        let p = 0.1;
        let mut f = FrameBuilder::new(true, p);
        f.spawn(
            3,
            quantize3([1.0, 2.0, 0.0], p),
            [(1u8, &b"hp"[..])].into_iter(),
        );
        f.spawn(10, quantize3([-5.0, 0.0, 0.0], p), std::iter::empty());
        let mut t = ReplicaTable::new();
        let s = t.apply(&f.finish()).unwrap();
        assert!(s.full);
        assert_eq!(s.spawned, vec![3, 10]);
        assert_eq!(t.pos(3), Some([1.0, 2.0, 0.0]));
        assert_eq!(t.entities[&3].components[&1], b"hp");

        let mut f = FrameBuilder::new(false, p);
        f.update(
            3,
            &encode_update_body([5, 0, 0], &[(1, None), (2, Some(b"new"))]),
        );
        f.update(10, &encode_update_body([0, -20, 0], &[]));
        f.despawn(99);
        let s = t.apply(&f.finish()).unwrap();
        assert_eq!(s.updated, vec![3, 10]);
        let pos3 = t.pos(3).unwrap();
        assert!((pos3[0] - 1.5).abs() < 1e-6);
        assert!(!t.entities[&3].components.contains_key(&1));
        assert_eq!(t.entities[&3].components[&2], b"new");
        assert_eq!(t.pos(10), Some([-5.0, -2.0, 0.0]));

        let mut f = FrameBuilder::new(false, p);
        f.despawn(10);
        t.apply(&f.finish()).unwrap();
        assert!(!t.entities.contains_key(&10));
    }

    #[test]
    fn an_unchanged_entity_update_is_two_bytes_plus_the_id() {
        let body = encode_update_body([3, -2, 0], &[]);
        assert_eq!(body.len(), 3); // mask + two one-byte deltas
    }

    #[test]
    fn hostile_frames_are_refused_without_panicking() {
        let mut t = ReplicaTable::new();
        assert!(t.apply(&[]).is_err());
        assert!(t.apply(&[2, 0, 0, 0, 0x80, 0x3f]).is_err()); // version 2
        assert!(t.apply(&[1, 0, 0, 0, 0, 0]).is_err()); // precision 0
        let head = |extra: &[u8]| {
            let mut v = vec![1, 0];
            v.extend_from_slice(&1.0f32.to_le_bytes());
            v.extend_from_slice(extra);
            v
        };
        // A huge declared count.
        assert!(t.apply(&head(&[0xff, 0xff, 0xff, 0xff, 0x0f])).is_err());
        // An update for an entity the table never saw.
        assert!(t.apply(&head(&[0, 0, 1, 7, 0])).is_err());
        // Ids that do not ascend.
        assert!(t.apply(&head(&[2, 5, 0, 0, 0])).is_err());
        // Trailing bytes.
        assert!(t.apply(&head(&[0, 0, 0, 9])).is_err());
        // A component that runs past the end.
        assert!(t.apply(&head(&[0, 1, 1, 0, 0, 0, 1, 4, 50, 1])).is_err());
    }

    #[test]
    fn quantize_handles_bad_values() {
        assert_eq!(quantize(f32::NAN, 0.1), 0);
        assert_eq!(quantize(f32::INFINITY, 0.1), 0);
        assert_eq!(quantize(f32::MAX, 1e-30), i64::MAX);
        assert_eq!(quantize(-0.04, 0.1), 0);
        assert_eq!(quantize(0.26, 0.1), 3);
    }
}
