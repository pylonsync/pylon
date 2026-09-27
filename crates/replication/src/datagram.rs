//! The replication datagram: entity updates that may be lost, duplicated,
//! or reordered (a WebTransport datagram, or a relay of one).
//!
//! A subscription on an unreliable transport gets spawns, despawns, and
//! full frames as ordinary replication frames ([`crate::frame`]) on a
//! reliable, ordered stream, and its updates in datagrams. Every datagram
//! stands alone: an update carries the entity's **absolute** quantized
//! position, not a difference, so a lost datagram only delays what it held.
//! The client acks the frame numbers it receives; the server keeps sending
//! whatever changed since the last acked state until an ack covers it.
//!
//! ```text
//! u8      version (2)
//! varint  frame number (per subscription, from 1)
//! varint  tick
//! varint  input ack: the highest client_seq the shard has processed
//! f32 LE  precision
//! varint  update count, then per entity (ids ascend, delta-coded):
//!           id, u8 generation, u8 mask (1 x, 2 y, 4 z, 8 components),
//!           the axes in the mask (zigzag, absolute quantized),
//!           components when bit 8 is set (as in a frame)
//! ```
//!
//! **Generation.** Both sides count the spawns of each id (mod 256) on the
//! stream. An update names the generation it is for, so a late datagram for
//! an entity that has since been despawned and spawned again under the same
//! id changes nothing.
//!
//! **Order.** The client keeps, per entity, the number of the last datagram
//! it applied, and skips an update from an older one.

use crate::frame::{mask, ComponentChange, DecodeError, ReplicaTable};
use crate::varint;
use crate::EntityId;

pub const VERSION: u8 = 2;

/// Bytes before the first update, at most: version, three varints, the
/// precision, and the update count.
pub const MAX_HEADER_LEN: usize = 1 + 10 + 10 + 10 + 4 + 3;

/// Encode one update body (everything after the id): generation, mask,
/// the axes given, and the component changes.
pub fn encode_entry_into(
    out: &mut Vec<u8>,
    generation: u8,
    axes: [Option<i64>; 3],
    components: &[ComponentChange<'_>],
) {
    out.push(generation);
    let mut m = 0u8;
    for (i, bit) in [mask::X, mask::Y, mask::Z].into_iter().enumerate() {
        if axes[i].is_some() {
            m |= bit;
        }
    }
    if !components.is_empty() {
        m |= mask::COMPONENTS;
    }
    out.push(m);
    for v in axes.into_iter().flatten() {
        varint::write_i64(out, v);
    }
    if !components.is_empty() {
        varint::write_u64(out, components.len() as u64);
        for (id, bytes) in components {
            out.push(*id);
            match bytes {
                Some(b) => {
                    varint::write_u64(out, b.len() as u64 + 1);
                    out.extend_from_slice(b);
                }
                None => varint::write_u64(out, 0),
            }
        }
    }
}

/// Builds one datagram. Add updates in ascending id order.
pub struct DatagramBuilder {
    header: Vec<u8>,
    body: Vec<u8>,
    count: u64,
    last: Option<EntityId>,
}

impl DatagramBuilder {
    pub fn new(frame: u64, tick: u64, ack: u64, precision: f32) -> Self {
        let mut header = Vec::with_capacity(MAX_HEADER_LEN);
        header.push(VERSION);
        varint::write_u64(&mut header, frame);
        varint::write_u64(&mut header, tick);
        varint::write_u64(&mut header, ack);
        header.extend_from_slice(&precision.to_le_bytes());
        Self {
            header,
            body: Vec::new(),
            count: 0,
            last: None,
        }
    }

    /// The datagram's length if an update with `entry` (from
    /// [`encode_entry_into`]) for `id` were added.
    pub fn len_with(&self, id: EntityId, entry: &[u8]) -> usize {
        let id_len = match self.last {
            Some(prev) => varint::len_u64(id.wrapping_sub(prev)),
            None => varint::len_u64(id),
        };
        self.header.len() + varint::len_u64(self.count + 1) + self.body.len() + id_len + entry.len()
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Add an update. Ids must ascend across calls.
    pub fn update(&mut self, id: EntityId, entry: &[u8]) {
        match self.last {
            Some(prev) => {
                debug_assert!(id > prev, "ids must ascend");
                varint::write_u64(&mut self.body, id.wrapping_sub(prev));
            }
            None => varint::write_u64(&mut self.body, id),
        }
        self.last = Some(id);
        self.body.extend_from_slice(entry);
        self.count += 1;
    }

    pub fn finish(mut self) -> Vec<u8> {
        varint::write_u64(&mut self.header, self.count);
        self.header.extend_from_slice(&self.body);
        self.header
    }
}

/// What one datagram did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DatagramSummary {
    pub frame: u64,
    pub tick: u64,
    pub ack: u64,
    /// Entities it updated.
    pub updated: Vec<EntityId>,
    /// Updates it skipped: an unknown entity, another generation, an older
    /// datagram than the entity's last, or a precision the table is not in.
    pub skipped: usize,
}

/// Longest update list a datagram may declare.
const MAX_COUNT: u64 = 1 << 16;

impl ReplicaTable {
    /// Apply one datagram. It changes nothing it cannot apply exactly (see
    /// [`DatagramSummary::skipped`]). An error means the bytes are not a
    /// datagram; the table then may hold part of it.
    pub fn apply_datagram(&mut self, datagram: &[u8]) -> Result<DatagramSummary, DecodeError> {
        let err = |m: &str| DecodeError(m.to_string());
        let mut b = datagram;
        let (&version, rest) = b.split_first().ok_or_else(|| err("datagram is empty"))?;
        b = rest;
        if version != VERSION {
            return Err(DecodeError(format!("datagram version {version}")));
        }
        let frame = varint::read_u64(&mut b).ok_or_else(|| err("bad frame number"))?;
        let tick = varint::read_u64(&mut b).ok_or_else(|| err("bad tick"))?;
        let ack = varint::read_u64(&mut b).ok_or_else(|| err("bad ack"))?;
        if b.len() < 4 {
            return Err(err("datagram ends early"));
        }
        let precision = f32::from_le_bytes(b[..4].try_into().unwrap());
        b = &b[4..];
        let n = varint::read_u64(&mut b).ok_or_else(|| err("bad count"))?;
        if n > MAX_COUNT {
            return Err(err("count too large"));
        }
        let mut summary = DatagramSummary {
            frame,
            tick,
            ack,
            ..DatagramSummary::default()
        };
        // Positions in another precision mean nothing to this table: the
        // full frame of the new precision is still on its way.
        let usable = precision == self.precision;
        let mut last: Option<EntityId> = None;
        for _ in 0..n {
            let v = varint::read_u64(&mut b).ok_or_else(|| err("bad id"))?;
            let id = match last {
                Some(prev) => prev
                    .checked_add(v)
                    .filter(|_| v > 0)
                    .ok_or_else(|| err("ids do not ascend"))?,
                None => v,
            };
            last = Some(id);
            let (&generation, rest) = b.split_first().ok_or_else(|| err("datagram ends early"))?;
            let (&m, rest) = rest
                .split_first()
                .ok_or_else(|| err("datagram ends early"))?;
            b = rest;
            let mut axes = [None; 3];
            for (i, bit) in [mask::X, mask::Y, mask::Z].into_iter().enumerate() {
                if m & bit != 0 {
                    axes[i] = Some(varint::read_i64(&mut b).ok_or_else(|| err("bad position"))?);
                }
            }
            let mut changes: Vec<(u8, Option<&[u8]>)> = Vec::new();
            if m & mask::COMPONENTS != 0 {
                let count = varint::read_u64(&mut b).ok_or_else(|| err("bad component count"))?;
                if count > 256 {
                    return Err(err("more than 256 components"));
                }
                for _ in 0..count {
                    let (&c, rest) = b.split_first().ok_or_else(|| err("datagram ends early"))?;
                    b = rest;
                    let len =
                        varint::read_u64(&mut b).ok_or_else(|| err("bad component length"))?;
                    if len == 0 {
                        changes.push((c, None));
                        continue;
                    }
                    let len = usize::try_from(len - 1).map_err(|_| err("component too large"))?;
                    if b.len() < len {
                        return Err(err("component runs past the end"));
                    }
                    let (value, rest) = b.split_at(len);
                    b = rest;
                    changes.push((c, Some(value)));
                }
            }
            let current = usable
                && self.generations.get(&id) == Some(&generation)
                && self.datagram_frames.get(&id).is_none_or(|&f| frame > f);
            let Some(entity) = self.entities.get_mut(&id).filter(|_| current) else {
                summary.skipped += 1;
                continue;
            };
            for (i, v) in axes.into_iter().enumerate() {
                if let Some(v) = v {
                    entity.q[i] = v;
                }
            }
            for (c, bytes) in changes {
                match bytes {
                    Some(v) => {
                        entity.components.insert(c, v.to_vec());
                    }
                    None => {
                        entity.components.remove(&c);
                    }
                }
            }
            self.datagram_frames.insert(id, frame);
            summary.updated.push(id);
        }
        if !b.is_empty() {
            return Err(err("trailing bytes"));
        }
        Ok(summary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::FrameBuilder;

    /// A table holding entities 1 and 2 (generation 1), precision 0.5.
    fn table() -> ReplicaTable {
        let mut t = ReplicaTable::new();
        let mut f = FrameBuilder::new(true, 0.5);
        f.spawn(1, [0, 0, 0], std::iter::empty());
        f.spawn(2, [10, 10, 0], [(7u8, &b"a"[..])].into_iter());
        t.apply(&f.finish()).unwrap();
        t
    }

    fn datagram(
        frame: u64,
        precision: f32,
        updates: &[(EntityId, u8, [Option<i64>; 3])],
    ) -> Vec<u8> {
        let mut d = DatagramBuilder::new(frame, 40 + frame, 3, precision);
        for (id, generation, axes) in updates {
            let mut e = Vec::new();
            encode_entry_into(&mut e, *generation, *axes, &[]);
            d.update(*id, &e);
        }
        d.finish()
    }

    #[test]
    fn an_update_sets_absolute_axes_and_components() {
        let mut t = table();
        assert_eq!(t.generations.get(&2), Some(&1));
        let mut d = DatagramBuilder::new(1, 41, 9, 0.5);
        let mut e = Vec::new();
        encode_entry_into(
            &mut e,
            1,
            [Some(12), None, Some(-3)],
            &[(7, None), (8, Some(b"xy"))],
        );
        let predicted = d.len_with(2, &e);
        d.update(2, &e);
        let bytes = d.finish();
        assert_eq!(bytes.len(), predicted);
        let s = t.apply_datagram(&bytes).unwrap();
        assert_eq!((s.frame, s.tick, s.ack), (1, 41, 9));
        assert_eq!(s.updated, vec![2]);
        let e = &t.entities[&2];
        assert_eq!(e.q, [12, 10, -3]);
        assert_eq!(e.components.get(&7), None);
        assert_eq!(e.components.get(&8).map(Vec::as_slice), Some(&b"xy"[..]));
    }

    #[test]
    fn an_older_datagram_for_an_entity_changes_nothing() {
        let mut t = table();
        t.apply_datagram(&datagram(5, 0.5, &[(1, 1, [Some(50), None, None])]))
            .unwrap();
        let s = t
            .apply_datagram(&datagram(
                4,
                0.5,
                &[
                    (1, 1, [Some(40), None, None]),
                    (2, 1, [Some(1), None, None]),
                ],
            ))
            .unwrap();
        assert_eq!(s.updated, vec![2], "entity 2 had no newer datagram");
        assert_eq!(s.skipped, 1);
        assert_eq!(t.entities[&1].q[0], 50);
    }

    #[test]
    fn another_generation_unknown_entity_and_other_precision_are_skipped() {
        let mut t = table();
        // Entity 1 despawns and spawns again: generation 2.
        let mut f = FrameBuilder::new(false, 0.5);
        f.despawn(1);
        f.spawn(1, [3, 3, 0], std::iter::empty());
        t.apply(&f.finish()).unwrap();
        assert_eq!(t.generations.get(&1), Some(&2));
        let s = t
            .apply_datagram(&datagram(
                1,
                0.5,
                &[
                    (1, 1, [Some(99), None, None]),
                    (9, 1, [Some(1), None, None]),
                ],
            ))
            .unwrap();
        assert!(s.updated.is_empty());
        assert_eq!(s.skipped, 2);
        assert_eq!(t.entities[&1].q[0], 3);
        let s = t
            .apply_datagram(&datagram(2, 0.25, &[(1, 2, [Some(99), None, None])]))
            .unwrap();
        assert_eq!(s.skipped, 1);
        assert_eq!(t.entities[&1].q[0], 3);
    }

    #[test]
    fn a_respawn_starts_the_entity_over_for_datagram_order() {
        let mut t = table();
        t.apply_datagram(&datagram(9, 0.5, &[(1, 1, [Some(9), None, None])]))
            .unwrap();
        let mut f = FrameBuilder::new(false, 0.5);
        f.despawn(1);
        f.spawn(1, [0, 0, 0], std::iter::empty());
        t.apply(&f.finish()).unwrap();
        // A datagram numbered below 9 but for the new generation applies.
        let s = t
            .apply_datagram(&datagram(10, 0.5, &[(1, 2, [Some(4), None, None])]))
            .unwrap();
        assert_eq!(s.updated, vec![1]);
    }

    #[test]
    fn malformed_datagrams_are_errors_not_panics() {
        let mut t = table();
        let good = datagram(1, 0.5, &[(1, 1, [Some(1), Some(2), None])]);
        for cut in 0..good.len() {
            let _ = t.clone().apply_datagram(&good[..cut]);
        }
        assert!(
            t.apply_datagram(&[1, 0, 0]).is_err(),
            "a frame, not a datagram"
        );
        let mut extra = good.clone();
        extra.push(0);
        assert!(t.apply_datagram(&extra).is_err());
    }
}
