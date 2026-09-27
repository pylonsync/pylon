//! Entity replication: per-subscriber spawn, update, and despawn frames.
//!
//! A shard whose [`SimState::replicated`](crate::SimState::replicated)
//! returns a store sends replication frames instead of snapshots. For each
//! subscription the shard keeps a baseline: the entities that subscriber
//! has, the quantized position it last got for each, and the store's
//! change counter at that time. Each tick it sends:
//!
//! - despawn for entities that left the subscriber's view or the store,
//! - spawn (full state) for entities that came into view,
//! - update for entities in view that changed since the subscriber's last
//!   update: only the axes whose quantized position moved (as a difference)
//!   and the components that changed or were removed.
//!
//! With interest management on, the view is the subscriber's interest
//! view, built from the store's positions. Without it, every entity is in
//! view.
//!
//! **Budget.** With `max_bytes_per_tick` set, spawns and despawns always go
//! out; updates go in priority order until the budget is spent. Priority
//! grows with the ticks since the entity was last sent and falls with its
//! distance from the subscriber's interest area. An update that does not
//! fit waits; its changes accumulate and go out whole later.
//!
//! **Baselines.** A new subscription, and a subscription whose outbound
//! queue dropped frames or is full, gets a full frame: the client clears
//! its table and receives every entity in view.

use std::collections::HashMap;

use pylon_replication::frame::{encode_update_body_into, quantize3, ComponentChange, FrameBuilder};
use pylon_replication::{EntityId, Replicated};

use crate::interest::{EntityPos, InterestArea};

/// A borrowed entity store: a plain reference, or a `RefCell` borrow (a
/// WebAssembly shard keeps its mirror of the module's store in one).
pub enum ReplicatedRef<'a> {
    Borrowed(&'a Replicated),
    Cell(std::cell::Ref<'a, Replicated>),
}

impl std::ops::Deref for ReplicatedRef<'_> {
    type Target = Replicated;
    fn deref(&self) -> &Replicated {
        match self {
            ReplicatedRef::Borrowed(r) => r,
            ReplicatedRef::Cell(r) => r,
        }
    }
}

impl<'a> From<&'a Replicated> for ReplicatedRef<'a> {
    fn from(r: &'a Replicated) -> Self {
        ReplicatedRef::Borrowed(r)
    }
}

/// Replication settings for a shard.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReplicationConfig {
    /// Position precision in world units: positions travel as
    /// `round(value / precision)`.
    pub precision: f32,
    /// Byte budget per subscriber per tick for updates. 0 = no limit.
    pub max_bytes_per_tick: usize,
    /// Which two axes interest management uses as the ground plane.
    pub plane: Plane,
}

impl Default for ReplicationConfig {
    fn default() -> Self {
        Self {
            precision: 0.01,
            max_bytes_per_tick: 0,
            plane: Plane::XY,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plane {
    /// A 2D game, or a 3D game with z up.
    XY,
    /// A 3D game with y up.
    XZ,
}

impl Plane {
    fn project(self, p: [f32; 3]) -> (f32, f32) {
        match self {
            Plane::XY => (p[0], p[1]),
            Plane::XZ => (p[0], p[2]),
        }
    }
}

/// The store positions as interest-management entities.
pub fn entity_positions(store: &Replicated, plane: Plane, out: &mut Vec<EntityPos>) {
    out.extend(store.iter().map(|(id, e)| {
        let (x, y) = plane.project(e.pos);
        EntityPos { id, x, y }
    }));
}

#[derive(Debug, Clone, Copy)]
struct Sent {
    /// The entity's spawn counter when the subscriber got it; a different
    /// value in the store means the id now names a new entity.
    spawn_seq: u64,
    /// The quantized position the subscriber has.
    q: [i64; 3],
    /// The store's change counter when the subscriber was last updated.
    seq: u64,
    /// The tick of that update.
    tick: u64,
}

#[derive(Debug, Default)]
struct Baseline {
    /// What the subscriber has, sorted by id. Sorted like the visible list,
    /// so one merge of the two finds the despawns, spawns, and updates.
    entities: Vec<(EntityId, Sent)>,
    /// The precision the subscriber's positions are in.
    precision: f32,
    /// The outbound queue's dropped-frame count when this baseline was
    /// last sent to; a higher count means frames were lost.
    dropped: u64,
    started: bool,
}

/// One subscription's frame this tick.
pub struct FrameInput<'a> {
    pub key: u64,
    /// Visible entity ids, sorted. None: every entity in the store.
    pub visible: Option<&'a [EntityId]>,
    pub area: Option<InterestArea>,
    /// The subscription's queue has dropped this many frames so far.
    pub dropped: u64,
    /// The queue is full, so this frame will replace what it holds.
    pub queue_full: bool,
}

/// One subscription's frame, and how to queue it.
pub struct FrameOutput {
    pub bytes: Vec<u8>,
    /// For a delta, the queue's dropped-frame count its baseline assumed
    /// (see `OutboundQueue::push_replication`); None for a full frame.
    pub delta_of: Option<u64>,
}

/// Per-subscription baselines for one shard.
#[derive(Debug, Default)]
pub struct Replicator {
    baselines: HashMap<u64, Baseline>,
    /// The store's entities this tick, sorted by id (see `TickEntity`),
    /// and their ids.
    table: Vec<TickEntity>,
    all_ids: Vec<EntityId>,
    /// The store version the table describes: (store id, change counter).
    table_of: Option<(u64, u64)>,
    /// The precision `TickEntity::q` is in, as bits.
    table_precision: Option<u32>,
    /// The store the baselines describe. The game can swap in another
    /// (a clone for a rollback, a new one for a round); its sequence
    /// numbers mean nothing against these baselines.
    store_id: Option<u64>,
    // Buffers reused across frames, so a frame allocates only its output.
    candidates: Vec<Candidate>,
    /// Every candidate's update body, back to back.
    bodies: Vec<u8>,
    /// The baseline being built this frame; swapped with the old one.
    next: Vec<(EntityId, Sent)>,
    /// Candidates in priority order, when the budget must choose: see
    /// `rank_key`.
    order: Vec<u128>,
}

/// What every subscriber's frame needs from one entity this tick. Built
/// once per tick, so a frame reads a sorted array in place of the store's
/// tree, and quantizes nothing.
#[derive(Debug, Clone, Copy)]
struct TickEntity {
    id: EntityId,
    seq: u64,
    spawn_seq: u64,
    /// The newest change counter among the components (removed ones too):
    /// the store is read for components only when this is newer than
    /// what the subscriber has.
    component_seq: u64,
    pos: [f32; 3],
    /// `pos` quantized to `Replicator::table_precision`.
    q: [i64; 3],
}

/// The first index in `table[from..]` whose id is `id` or greater (a
/// galloping search: the ids a frame looks up ascend), and whether it is
/// `id`.
fn seek(table: &[TickEntity], from: usize, id: EntityId) -> (usize, bool) {
    let rest = &table[from..];
    let mut bound = 1;
    while bound < rest.len() && rest[bound].id < id {
        bound *= 2;
    }
    let lo = bound / 2;
    let hi = (bound + 1).min(rest.len());
    match rest[lo..hi].binary_search_by_key(&id, |t| t.id) {
        Ok(i) => (from + lo + i, true),
        Err(i) => (from + lo + i, false),
    }
}

#[derive(Debug)]
struct Candidate {
    id: EntityId,
    priority: f64,
    /// The update body: `bodies[body.0..body.1]`.
    body: (usize, usize),
    delta: [i64; 3],
    seq: u64,
    /// Where the entity's entry is in the new baseline.
    slot: usize,
    chosen: bool,
}

impl Replicator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Drop baselines of subscriptions `keep` rejects.
    pub fn retain(&mut self, mut keep: impl FnMut(u64) -> bool) {
        self.baselines.retain(|k, _| keep(*k));
    }

    /// Call once per tick before `frame`.
    pub fn begin_tick(&mut self, store: &Replicated) {
        if self.store_id != Some(store.store_id()) {
            // Every subscription gets a full frame of the new store.
            self.baselines.clear();
            self.store_id = Some(store.store_id());
        }
        self.refresh_table(store);
    }

    /// Rebuild the tick table when the store changed since it was built.
    /// `frame` calls it too, so a frame built after a change without a
    /// `begin_tick` (a full frame the queue asked for) is still current.
    fn refresh_table(&mut self, store: &Replicated) {
        let of = (store.store_id(), store.seq());
        if self.table_of == Some(of) {
            return;
        }
        self.table_of = Some(of);
        self.table_precision = None;
        self.table.clear();
        self.table.extend(store.iter().map(|(id, e)| TickEntity {
            id,
            seq: e.seq,
            spawn_seq: e.spawn_seq,
            component_seq: e.components.values().map(|c| c.seq).max().unwrap_or(0),
            pos: e.pos,
            q: [0; 3],
        }));
        self.all_ids.clear();
        self.all_ids.extend(self.table.iter().map(|t| t.id));
    }

    /// Build one subscription's frame for `tick`.
    pub fn frame(
        &mut self,
        store: &Replicated,
        config: &ReplicationConfig,
        tick: u64,
        input: FrameInput<'_>,
    ) -> FrameOutput {
        self.refresh_table(store);
        let precision = sanitize_precision(config.precision);
        if self.table_precision != Some(precision.to_bits()) {
            for t in &mut self.table {
                t.q = quantize3(t.pos, precision);
            }
            self.table_precision = Some(precision.to_bits());
        }
        let Replicator {
            baselines,
            table,
            all_ids,
            candidates,
            bodies,
            next,
            order,
            ..
        } = self;
        let visible: &[EntityId] = input.visible.unwrap_or(all_ids);
        let base = baselines.entry(input.key).or_default();
        // A new precision rescales every position the client has.
        let full = !base.started
            || input.dropped != base.dropped
            || input.queue_full
            || base.precision != precision;
        base.started = true;
        base.dropped = input.dropped;
        base.precision = precision;
        let mut frame = FrameBuilder::new(full, precision);
        if full {
            base.entities.clear();
        }

        // One merge of the visible ids and the baseline, both sorted:
        // - in the baseline only: left the view, so despawn;
        // - visible only: came into view, so spawn;
        // - in both: despawn and spawn again if the id now names a new
        //   entity (or despawn if the entity is gone), else an update
        //   candidate if it changed.
        // Spawns come out in ascending id order, as the frame needs.
        candidates.clear();
        bodies.clear();
        next.clear();
        let seq = store.seq();
        // One entity's changed components; it borrows the store, so it
        // lives for this frame only.
        let mut changes: Vec<ComponentChange<'_>> = Vec::new();
        let mut cursor = 0usize;
        let mut had = base.entities.iter().peekable();
        let mut vis = visible.iter().copied().peekable();
        loop {
            let (id, sent) = match (vis.peek().copied(), had.peek().map(|(id, _)| *id)) {
                (None, None) => break,
                (Some(v), Some(h)) if h < v => {
                    had.next();
                    frame.despawn(h);
                    continue;
                }
                (None, Some(h)) => {
                    had.next();
                    frame.despawn(h);
                    continue;
                }
                (Some(v), Some(h)) if h == v => {
                    vis.next();
                    (v, had.next().map(|(_, s)| *s))
                }
                (Some(v), _) => {
                    vis.next();
                    (v, None)
                }
            };
            let (at, found) = seek(table, cursor, id);
            cursor = at;
            if !found {
                // Not in the store (a view can name an id it no longer has).
                if sent.is_some() {
                    frame.despawn(id);
                }
                continue;
            }
            let t = table[at];
            let sent = match sent {
                Some(sent) if t.spawn_seq != sent.spawn_seq => {
                    // The id names a new entity: nothing of the old one
                    // may linger, so it spawns again with its full state.
                    frame.despawn(id);
                    None
                }
                other => other,
            };
            let Some(mut sent) = sent else {
                // The table is current, so the store has the entity.
                let e = store.get(id).expect("the tick table matches the store");
                frame.spawn(
                    id,
                    t.q,
                    e.components
                        .iter()
                        .filter_map(|(c, v)| v.bytes.as_deref().map(|b| (*c, b)))
                        .collect::<Vec<_>>()
                        .into_iter(),
                );
                next.push((
                    id,
                    Sent {
                        spawn_seq: t.spawn_seq,
                        q: t.q,
                        seq,
                        tick,
                    },
                ));
                continue;
            };
            if t.seq > sent.seq {
                let q = t.q;
                // Wrapping, like the decoder's add: any finite position
                // round-trips, even across the whole i64 range.
                let delta = [
                    q[0].wrapping_sub(sent.q[0]),
                    q[1].wrapping_sub(sent.q[1]),
                    q[2].wrapping_sub(sent.q[2]),
                ];
                changes.clear();
                if t.component_seq > sent.seq {
                    let e = store.get(id).expect("the tick table matches the store");
                    changes.extend(
                        e.components
                            .iter()
                            .filter(|(_, c)| c.seq > sent.seq)
                            .map(|(id, c)| (*id, c.bytes.as_deref())),
                    );
                }
                if delta == [0, 0, 0] && changes.is_empty() {
                    // Moved less than the precision: nothing to send, and
                    // the position baseline stays where the client is.
                    sent.seq = seq;
                } else {
                    let staleness = (tick.saturating_sub(sent.tick) + 1) as f64;
                    let distance = input.area.map_or(0.0, |a| {
                        let (x, y) = config.plane.project(t.pos);
                        let (dx, dy) = (x as f64 - a.x as f64, y as f64 - a.y as f64);
                        (dx * dx + dy * dy).sqrt() / (a.radius as f64).max(1e-6)
                    });
                    let priority = staleness / (1.0 + distance);
                    let start = bodies.len();
                    encode_update_body_into(bodies, delta, &changes);
                    candidates.push(Candidate {
                        id,
                        // A non-finite position gives no usable distance;
                        // such an update ranks last.
                        priority: if priority.is_nan() { 0.0 } else { priority },
                        body: (start, bodies.len()),
                        delta,
                        seq,
                        slot: next.len(),
                        chosen: true,
                    });
                }
            }
            next.push((id, sent));
        }

        // Choose updates within the budget, highest priority first. When
        // every update fits, all go and there is nothing to rank.
        let budget = config.max_bytes_per_tick;
        let cost = |c: &Candidate| c.body.1 - c.body.0 + pylon_replication::varint::len_u64(c.id);
        if budget > 0 && candidates.iter().map(cost).sum::<usize>() > budget {
            order.clear();
            order.extend(candidates.iter().map(|c| rank_key(c.priority, c.id)));
            order.sort_unstable();
            let mut used = 0usize;
            let mut first = true;
            for &key in order.iter() {
                // Candidates are sorted by id, so the id finds its entry.
                let i = candidates
                    .binary_search_by_key(&(key as u64), |c| c.id)
                    .expect("a ranked candidate is present");
                let c = &mut candidates[i];
                // The id is delta-coded in the frame; its absolute length
                // bounds that.
                let n = cost(c);
                // The top candidate always goes, even past the budget, so an
                // update bigger than the budget still arrives.
                c.chosen = first || used + n <= budget;
                if c.chosen {
                    first = false;
                    used += n;
                }
            }
        }
        // Candidates are in id order already: the merge made them so.
        for c in candidates.iter().filter(|c| c.chosen) {
            frame.update(c.id, &bodies[c.body.0..c.body.1]);
            let sent = &mut next[c.slot].1;
            for i in 0..3 {
                sent.q[i] = sent.q[i].wrapping_add(c.delta[i]);
            }
            sent.seq = c.seq;
            sent.tick = tick;
        }
        std::mem::swap(&mut base.entities, next);
        FrameOutput {
            bytes: frame.finish(),
            delta_of: (!full).then_some(input.dropped),
        }
    }
}

/// A sort key that puts higher priority first and, among equal priority,
/// the lower id first. Priorities are positive and finite (or 0), and for
/// those the IEEE bit pattern orders like the number, so its complement
/// sorts descending.
fn rank_key(priority: f64, id: EntityId) -> u128 {
    (((!priority.to_bits()) as u128) << 64) | id as u128
}

fn sanitize_precision(p: f32) -> f32 {
    if p.is_finite() && p > 0.0 {
        p
    } else {
        ReplicationConfig::default().precision
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pylon_replication::ReplicaTable;

    fn input<'a>(key: u64, visible: Option<&'a [EntityId]>) -> FrameInput<'a> {
        FrameInput {
            key,
            visible,
            area: None,
            dropped: 0,
            queue_full: false,
        }
    }

    /// Everything the client table holds matches the store, to precision.
    fn assert_matches(table: &ReplicaTable, store: &Replicated, ids: &[EntityId], precision: f32) {
        assert_eq!(table.entities.keys().copied().collect::<Vec<_>>(), ids);
        for id in ids {
            let e = store.get(*id).unwrap();
            let got = table.pos(*id).unwrap();
            for i in 0..3 {
                assert!(
                    (got[i] - e.pos[i]).abs() <= precision,
                    "{id}: {got:?} vs {:?}",
                    e.pos
                );
            }
            let comps: Vec<(u8, Vec<u8>)> = e
                .components
                .iter()
                .filter_map(|(c, v)| v.bytes.clone().map(|b| (*c, b)))
                .collect();
            let have: Vec<(u8, Vec<u8>)> = table.entities[id]
                .components
                .iter()
                .map(|(c, b)| (*c, b.clone()))
                .collect();
            assert_eq!(have, comps, "components of {id}");
        }
    }

    #[test]
    fn a_client_table_tracks_the_store_through_spawn_move_and_despawn() {
        let config = ReplicationConfig {
            precision: 0.1,
            ..Default::default()
        };
        let mut store = Replicated::new();
        let mut rep = Replicator::new();
        let mut table = ReplicaTable::new();
        store.spawn(1, [0.0, 0.0, 0.0]);
        store.spawn(2, [5.0, 5.0, 0.0]);
        store.set_component(2, 7, b"red");
        for tick in 1..=30u64 {
            if tick == 10 {
                store.spawn(3, [1.0, 1.0, 1.0]);
            }
            if tick == 20 {
                store.despawn(1);
                store.remove_component(2, 7);
            }
            for id in [1u64, 2, 3] {
                if let Some(e) = store.get(id) {
                    let p = e.pos;
                    store.set_pos(id, [p[0] + 0.37, p[1] - 0.21, p[2]]);
                }
            }
            rep.begin_tick(&store);
            let f = rep.frame(&store, &config, tick, input(9, None)).bytes;
            let s = table.apply(&f).unwrap();
            assert_eq!(s.full, tick == 1);
            let ids: Vec<EntityId> = store.iter().map(|(id, _)| id).collect();
            assert_matches(&table, &store, &ids, config.precision);
        }
    }

    #[test]
    fn leaving_view_despawns_and_returning_spawns_again() {
        let config = ReplicationConfig::default();
        let mut store = Replicated::new();
        store.spawn(1, [0.0; 3]);
        store.spawn(2, [1.0, 0.0, 0.0]);
        let mut rep = Replicator::new();
        let mut table = ReplicaTable::new();
        rep.begin_tick(&store);
        table
            .apply(&rep.frame(&store, &config, 1, input(1, Some(&[1, 2]))).bytes)
            .unwrap();
        let s = table
            .apply(&rep.frame(&store, &config, 2, input(1, Some(&[1]))).bytes)
            .unwrap();
        assert_eq!(s.despawned, vec![2]);
        let s = table
            .apply(&rep.frame(&store, &config, 3, input(1, Some(&[1, 2]))).bytes)
            .unwrap();
        assert_eq!(s.spawned, vec![2]);
        assert_eq!(table.entities.len(), 2);
    }

    #[test]
    fn dropped_frames_force_a_full_baseline() {
        let config = ReplicationConfig::default();
        let mut store = Replicated::new();
        store.spawn(1, [0.0; 3]);
        let mut rep = Replicator::new();
        rep.begin_tick(&store);
        let mut t = ReplicaTable::new();
        assert!(
            t.apply(&rep.frame(&store, &config, 1, input(1, None)).bytes)
                .unwrap()
                .full
        );
        assert!(
            !t.apply(&rep.frame(&store, &config, 2, input(1, None)).bytes)
                .unwrap()
                .full
        );
        let mut dropped = input(1, None);
        dropped.dropped = 3;
        store.set_pos(1, [4.0, 0.0, 0.0]);
        let mut fresh = ReplicaTable::new(); // a client that missed frames
        let s = fresh
            .apply(&rep.frame(&store, &config, 3, dropped).bytes)
            .unwrap();
        assert!(s.full);
        assert_eq!(fresh.pos(1), Some([4.0, 0.0, 0.0]));
        let mut full_queue = input(1, None);
        full_queue.dropped = 3;
        full_queue.queue_full = true;
        assert!(
            fresh
                .apply(&rep.frame(&store, &config, 4, full_queue).bytes)
                .unwrap()
                .full
        );
    }

    #[test]
    fn the_budget_sends_near_and_stale_entities_first_and_catches_up_later() {
        let config = ReplicationConfig {
            precision: 0.1,
            max_bytes_per_tick: 30,
            plane: Plane::XY,
        };
        let mut store = Replicated::new();
        for id in 0..40u64 {
            store.spawn(id, [id as f32 * 10.0, 0.0, 0.0]);
        }
        let mut rep = Replicator::new();
        let mut table = ReplicaTable::new();
        let area = Some(InterestArea {
            x: 0.0,
            y: 0.0,
            radius: 50.0,
        });
        rep.begin_tick(&store);
        let mut first = input(1, None);
        first.area = area;
        table
            .apply(&rep.frame(&store, &config, 1, first).bytes)
            .unwrap();
        for id in 0..40u64 {
            store.set_pos(id, [id as f32 * 10.0 + 1.0, 0.0, 0.0]);
        }
        rep.begin_tick(&store);
        let mut next = input(1, None);
        next.area = area;
        let f = rep.frame(&store, &config, 2, next).bytes;
        let s = table.apply(&f).unwrap();
        // The frame fits the budget (plus the fixed header and counts)...
        assert!(f.len() <= 30 + 12, "{} bytes", f.len());
        // ...and holds the nearest entities.
        assert!(!s.updated.is_empty() && s.updated.len() < 40);
        assert!(s.updated.iter().all(|id| *id < 20), "{:?}", s.updated);
        // With no more movement, later ticks send the rest; the table ends
        // up matching the store.
        for tick in 3..40 {
            let mut i = input(1, None);
            i.area = area;
            rep.begin_tick(&store);
            table
                .apply(&rep.frame(&store, &config, tick, i).bytes)
                .unwrap();
        }
        let ids: Vec<EntityId> = (0..40).collect();
        assert_matches(&table, &store, &ids, config.precision);
    }

    #[test]
    fn a_reused_id_arrives_as_a_new_entity_without_the_old_components() {
        let config = ReplicationConfig::default();
        let mut store = Replicated::new();
        store.spawn(7, [0.0; 3]);
        store.set_component(7, 1, b"old");
        let mut rep = Replicator::new();
        let mut table = ReplicaTable::new();
        rep.begin_tick(&store);
        table
            .apply(&rep.frame(&store, &config, 1, input(1, None)).bytes)
            .unwrap();
        store.despawn(7);
        store.spawn(7, [5.0, 0.0, 0.0]);
        rep.begin_tick(&store);
        let s = table
            .apply(&rep.frame(&store, &config, 2, input(1, None)).bytes)
            .unwrap();
        assert_eq!((s.despawned, s.spawned), (vec![7], vec![7]));
        assert!(table.entities[&7].components.is_empty());
        assert_eq!(table.pos(7), Some([5.0, 0.0, 0.0]));
    }

    #[test]
    fn a_swapped_store_is_sent_in_full() {
        let config = ReplicationConfig::default();
        let mut store = Replicated::new();
        store.spawn(1, [0.0; 3]);
        store.set_component(1, 1, b"template");
        let template = store.clone();
        let mut rep = Replicator::new();
        let mut table = ReplicaTable::new();
        for tick in 1..=5u64 {
            store.set_pos(1, [tick as f32, 0.0, 0.0]);
            store.set_component(1, 1, &[tick as u8]);
            rep.begin_tick(&store);
            table
                .apply(&rep.frame(&store, &config, tick, input(1, None)).bytes)
                .unwrap();
        }
        // A rollback to the template: same ids, older sequence numbers.
        store = template.clone();
        rep.begin_tick(&store);
        let s = table
            .apply(&rep.frame(&store, &config, 6, input(1, None)).bytes)
            .unwrap();
        assert!(s.full);
        assert_matches(&table, &store, &[1], config.precision);
        // A new round in a new store: its counter starts again at zero.
        store = Replicated::new();
        store.spawn(1, [9.0, 0.0, 0.0]);
        rep.begin_tick(&store);
        let s = table
            .apply(&rep.frame(&store, &config, 7, input(1, None)).bytes)
            .unwrap();
        assert!(s.full);
        assert_matches(&table, &store, &[1], config.precision);
    }

    #[test]
    fn a_precision_change_sends_a_full_frame() {
        let mut store = Replicated::new();
        store.spawn(1, [10.0, 0.0, 0.0]);
        let mut rep = Replicator::new();
        let mut table = ReplicaTable::new();
        rep.begin_tick(&store);
        let coarse = ReplicationConfig {
            precision: 1.0,
            ..Default::default()
        };
        table
            .apply(&rep.frame(&store, &coarse, 1, input(1, None)).bytes)
            .unwrap();
        let fine = ReplicationConfig {
            precision: 0.5,
            ..Default::default()
        };
        let out = rep.frame(&store, &fine, 2, input(1, None));
        assert!(out.delta_of.is_none());
        assert!(table.apply(&out.bytes).unwrap().full);
        assert_eq!(table.pos(1), Some([10.0, 0.0, 0.0]));
    }

    #[test]
    fn extreme_positions_round_trip_without_overflow() {
        let config = ReplicationConfig {
            precision: 1.0e-30,
            ..Default::default()
        };
        let mut store = Replicated::new();
        store.spawn(1, [-f32::MAX, 0.0, 0.0]);
        let mut rep = Replicator::new();
        let mut table = ReplicaTable::new();
        rep.begin_tick(&store);
        table
            .apply(&rep.frame(&store, &config, 1, input(1, None)).bytes)
            .unwrap();
        store.set_pos(1, [f32::MAX, 0.0, 0.0]);
        rep.begin_tick(&store);
        table
            .apply(&rep.frame(&store, &config, 2, input(1, None)).bytes)
            .unwrap();
        assert_eq!(table.entities[&1].q[0], i64::MAX);
    }

    #[test]
    fn an_update_bigger_than_the_budget_still_arrives() {
        let config = ReplicationConfig {
            precision: 0.1,
            max_bytes_per_tick: 10,
            plane: Plane::XY,
        };
        let mut store = Replicated::new();
        store.spawn(u64::MAX, [0.0; 3]);
        let mut rep = Replicator::new();
        let mut table = ReplicaTable::new();
        rep.begin_tick(&store);
        table
            .apply(&rep.frame(&store, &config, 1, input(1, None)).bytes)
            .unwrap();
        store.set_component(u64::MAX, 3, &[9u8; 100]);
        rep.begin_tick(&store);
        let s = table
            .apply(&rep.frame(&store, &config, 2, input(1, None)).bytes)
            .unwrap();
        assert_eq!(s.updated, vec![u64::MAX]);
        assert_eq!(table.entities[&u64::MAX].components[&3], vec![9u8; 100]);
    }

    #[test]
    fn sub_precision_moves_send_nothing_and_do_not_drift() {
        let config = ReplicationConfig {
            precision: 1.0,
            ..Default::default()
        };
        let mut store = Replicated::new();
        store.spawn(1, [0.0; 3]);
        let mut rep = Replicator::new();
        let mut table = ReplicaTable::new();
        rep.begin_tick(&store);
        table
            .apply(&rep.frame(&store, &config, 1, input(1, None)).bytes)
            .unwrap();
        // 0.3 per tick: the quantized value only changes every few ticks,
        // and the client ends where the store is.
        for tick in 2..=20 {
            let x = store.get(1).unwrap().pos[0] + 0.3;
            store.set_pos(1, [x, 0.0, 0.0]);
            rep.begin_tick(&store);
            table
                .apply(&rep.frame(&store, &config, tick, input(1, None)).bytes)
                .unwrap();
        }
        assert_matches(&table, &store, &[1], 1.0);
    }
}
