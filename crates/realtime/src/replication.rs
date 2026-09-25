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

use pylon_replication::frame::{encode_update_body, quantize3, ComponentChange, FrameBuilder};
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
    entities: HashMap<EntityId, Sent>,
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
    all_ids: Vec<EntityId>,
    candidates: Vec<Candidate>,
    /// The store the baselines describe. The game can swap in another
    /// (a clone for a rollback, a new one for a round); its sequence
    /// numbers mean nothing against these baselines.
    store_id: Option<u64>,
}

#[derive(Debug)]
struct Candidate {
    id: EntityId,
    priority: f64,
    body: Vec<u8>,
    delta: [i64; 3],
    seq: u64,
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
        self.all_ids.clear();
        self.all_ids.extend(store.iter().map(|(id, _)| id));
    }

    /// Build one subscription's frame for `tick`.
    pub fn frame(
        &mut self,
        store: &Replicated,
        config: &ReplicationConfig,
        tick: u64,
        input: FrameInput<'_>,
    ) -> FrameOutput {
        let precision = sanitize_precision(config.precision);
        let visible: &[EntityId] = input.visible.unwrap_or(&self.all_ids);
        let base = self.baselines.entry(input.key).or_default();
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
        } else {
            // Despawn what the subscriber has and no longer sees.
            // Despawn what the subscriber has and no longer sees, and ids
            // that now name a new entity (it spawns again below, with its
            // full state, so nothing of the old entity lingers).
            let gone: Vec<EntityId> = base
                .entities
                .iter()
                .filter(|(id, sent)| {
                    visible.binary_search(id).is_err()
                        || store
                            .get(**id)
                            .is_none_or(|e| e.spawn_seq != sent.spawn_seq)
                })
                .map(|(id, _)| *id)
                .collect();
            for id in gone {
                base.entities.remove(&id);
                frame.despawn(id);
            }
        }

        // Spawns (ascending, since `visible` is sorted), and update
        // candidates for entities the subscriber already has.
        self.candidates.clear();
        for &id in visible {
            let Some(e) = store.get(id) else { continue };
            match base.entities.get(&id) {
                None => {
                    let q = quantize3(e.pos, precision);
                    frame.spawn(
                        id,
                        q,
                        e.components
                            .iter()
                            .filter_map(|(c, v)| v.bytes.as_deref().map(|b| (*c, b)))
                            .collect::<Vec<_>>()
                            .into_iter(),
                    );
                    base.entities.insert(
                        id,
                        Sent {
                            spawn_seq: e.spawn_seq,
                            q,
                            seq: store.seq(),
                            tick,
                        },
                    );
                }
                Some(sent) if e.changed_since(sent.seq) => {
                    let q = quantize3(e.pos, precision);
                    // Wrapping, like the decoder's add: any finite position
                    // round-trips, even across the whole i64 range.
                    let delta = [
                        q[0].wrapping_sub(sent.q[0]),
                        q[1].wrapping_sub(sent.q[1]),
                        q[2].wrapping_sub(sent.q[2]),
                    ];
                    let comps: Vec<ComponentChange<'_>> = e
                        .components
                        .iter()
                        .filter(|(_, c)| c.seq > sent.seq)
                        .map(|(id, c)| (*id, c.bytes.as_deref()))
                        .collect();
                    if delta == [0, 0, 0] && comps.is_empty() {
                        // Moved less than the precision: nothing to send, and
                        // the position baseline stays where the client is.
                        let sent = base.entities.get_mut(&id).expect("present");
                        sent.seq = store.seq();
                        continue;
                    }
                    let staleness = (tick.saturating_sub(sent.tick) + 1) as f64;
                    let distance = input.area.map_or(0.0, |a| {
                        let (x, y) = config.plane.project(e.pos);
                        let (dx, dy) = (x as f64 - a.x as f64, y as f64 - a.y as f64);
                        (dx * dx + dy * dy).sqrt() / (a.radius as f64).max(1e-6)
                    });
                    self.candidates.push(Candidate {
                        id,
                        priority: staleness / (1.0 + distance),
                        body: encode_update_body(delta, &comps),
                        delta,
                        seq: store.seq(),
                    });
                }
                Some(_) => {}
            }
        }

        // Choose updates within the budget, highest priority first, then
        // write them in id order.
        let budget = config.max_bytes_per_tick;
        if budget > 0 {
            self.candidates.sort_by(|a, b| {
                b.priority
                    .partial_cmp(&a.priority)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(a.id.cmp(&b.id))
            });
            let mut used = 0usize;
            let mut first = true;
            self.candidates.retain(|c| {
                // The id is delta-coded after sorting; its absolute length
                // bounds that.
                let cost = c.body.len() + pylon_replication::varint::len_u64(c.id);
                // The top candidate always goes, even past the budget, so an
                // update bigger than the budget still arrives.
                if first || used + cost <= budget {
                    first = false;
                    used += cost;
                    true
                } else {
                    false
                }
            });
            self.candidates.sort_by_key(|c| c.id);
        }
        for c in &self.candidates {
            frame.update(c.id, &c.body);
            let sent = base.entities.get_mut(&c.id).expect("present");
            for i in 0..3 {
                sent.q[i] = sent.q[i].wrapping_add(c.delta[i]);
            }
            sent.seq = c.seq;
            sent.tick = tick;
        }
        FrameOutput {
            bytes: frame.finish(),
            delta_of: (!full).then_some(input.dropped),
        }
    }
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
