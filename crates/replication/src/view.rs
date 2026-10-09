//! What one subscriber was sent, kept so the server can see what that
//! subscriber drew at an earlier tick (lag compensation).
//!
//! A [`ViewHistory`] records the frames sent to one subscriber the way the
//! clients' entity interpolators record them (`EntityInterpolator` in
//! packages/realtime and packages/csharp), and [`ViewHistory::position_at`]
//! places an entity at a fractional tick with the same rule. The two agree
//! exactly for an interpolator with its default options: `holdWhenQuiet`
//! on and no `snapDistance`.
//!
//! The rules, as in the clients:
//!
//! - A spawn starts a life with one sample at the spawn tick. A despawn ends
//!   the open life at its tick. An entity is drawn from its spawn tick until
//!   the tick its life ended.
//! - An update adds a sample when the position or a component changed. A
//!   second update in the same tick replaces the tick's sample. When the
//!   entity moved and its last sample is older than the tick before, a copy
//!   of the last sample is added at the tick before: it stood still until
//!   then.
//! - A full frame lists every entity the subscriber has. Open lives of
//!   entities it does not list end at its tick.
//! - At a render tick, the entity is between the last sample at or before
//!   it and the next sample, linearly, and holds at the last sample after
//!   it. There is no extrapolation.
//!
//! Positions are the quantized positions times the frame precision, in
//! f64, as the clients compute them.

use std::collections::{BTreeMap, VecDeque};

use crate::EntityId;

#[derive(Debug, Clone, Copy, PartialEq)]
struct Sample {
    tick: u64,
    pos: [f64; 3],
    /// Counts component changes, so a sample knows whether its components
    /// differ from the one before.
    components: u64,
}

#[derive(Debug, Clone)]
struct Life {
    spawn_tick: u64,
    despawn_tick: Option<u64>,
    samples: VecDeque<Sample>,
}

/// One entity in a frame sent to the subscriber.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sent {
    pub id: EntityId,
    /// The quantized position the subscriber has after the frame.
    pub q: [i64; 3],
    /// True when the frame changed the entity's components for the subscriber.
    pub components_changed: bool,
}

/// The entities sent to one subscriber over the last ticks, as its
/// interpolator holds them. See the module docs.
#[derive(Debug, Clone)]
pub struct ViewHistory {
    keep_ticks: u64,
    lives: BTreeMap<EntityId, Vec<Life>>,
    last_tick: Option<u64>,
    /// The tick of the last prune: pruning every few ticks is enough to
    /// bound what the history holds.
    pruned_at: u64,
}

/// How many ticks between prunes.
const PRUNE_EVERY: u64 = 8;

impl ViewHistory {
    /// Keep enough to place entities at ticks down to `keep_ticks` before
    /// the newest frame.
    pub fn new(keep_ticks: u64) -> Self {
        Self {
            keep_ticks,
            lives: BTreeMap::new(),
            last_tick: None,
            pruned_at: 0,
        }
    }

    /// How many ticks back [`position_at`](Self::position_at) can see.
    pub fn keep_ticks(&self) -> u64 {
        self.keep_ticks
    }

    /// The newest tick recorded.
    pub fn last_tick(&self) -> Option<u64> {
        self.last_tick
    }

    /// Forget everything (a new connection, or the subscriber's baseline was
    /// reset).
    pub fn clear(&mut self) {
        self.lives.clear();
        self.last_tick = None;
        self.pruned_at = 0;
    }

    /// Record a frame sent for `tick`. `full` is a frame that lists every
    /// entity the subscriber has, as `spawned`. Positions are quantized; the
    /// subscriber places them at `q * precision`.
    pub fn record(
        &mut self,
        tick: u64,
        precision: f32,
        full: bool,
        spawned: &[Sent],
        updated: &[Sent],
        despawned: &[EntityId],
    ) {
        if self.last_tick.is_some_and(|last| tick < last) {
            // Ticks went back: a restarted or different shard.
            self.clear();
        }
        self.last_tick = Some(tick);
        let p = precision as f64;
        let pos = |q: [i64; 3]| [q[0] as f64 * p, q[1] as f64 * p, q[2] as f64 * p];

        if full {
            let present: std::collections::HashSet<EntityId> =
                spawned.iter().map(|s| s.id).collect();
            for (id, lives) in self.lives.iter_mut() {
                if let Some(open) = lives.last_mut().filter(|l| l.despawn_tick.is_none()) {
                    if !present.contains(id) {
                        open.despawn_tick = Some(tick);
                    }
                }
            }
            for s in spawned {
                match self.open_life(s.id) {
                    Some(life) => Self::push(life, tick, pos(s.q), s.components_changed),
                    None => self.spawn(s.id, tick, pos(s.q)),
                }
            }
        } else {
            for id in despawned {
                if let Some(open) = self.open_life(*id) {
                    open.despawn_tick = Some(tick);
                }
            }
            for s in spawned {
                self.spawn(s.id, tick, pos(s.q));
            }
            // Use lookups for sparse updates. Reserve the map walk for
            // frames that update at least one in eight stored entities.
            if updated.len() >= self.lives.len().div_ceil(8)
                && updated.windows(2).all(|w| w[0].id < w[1].id)
            {
                // Dense ascending ids: one walk instead of repeated lookups.
                let mut lives = self.lives.iter_mut().peekable();
                for s in updated {
                    while lives.next_if(|(id, _)| **id < s.id).is_some() {}
                    if let Some((_, l)) = lives.next_if(|(id, _)| **id == s.id) {
                        if let Some(life) = l.last_mut().filter(|l| l.despawn_tick.is_none()) {
                            Self::push(life, tick, pos(s.q), s.components_changed);
                        }
                    }
                }
            } else {
                for s in updated {
                    if let Some(life) = self.open_life(s.id) {
                        Self::push(life, tick, pos(s.q), s.components_changed);
                    }
                }
            }
        }
        if tick >= self.pruned_at + PRUNE_EVERY || tick < self.pruned_at {
            self.prune(tick);
            self.pruned_at = tick;
        }
    }

    /// Where the subscriber drew `id` at `render_tick` (fractional), or None
    /// when the subscriber did not draw it then (not spawned yet, despawned,
    /// or never sent).
    pub fn position_at(&self, id: EntityId, render_tick: f64) -> Option<[f64; 3]> {
        place_entity(self.lives.get(&id)?, render_tick)
    }

    /// Every entity the subscriber drew at `render_tick` within `radius` of
    /// `center` (3D distance), with its position then, in id order.
    pub fn entities_near(
        &self,
        center: [f64; 3],
        radius: f64,
        render_tick: f64,
    ) -> Vec<(EntityId, [f64; 3])> {
        let r2 = radius * radius;
        let mut out = Vec::new();
        for (id, lives) in &self.lives {
            if let Some(p) = place_entity(lives, render_tick) {
                let d = [p[0] - center[0], p[1] - center[1], p[2] - center[2]];
                if d[0] * d[0] + d[1] * d[1] + d[2] * d[2] <= r2 {
                    out.push((*id, p));
                }
            }
        }
        out
    }

    /// Ids the history holds a life for (open or recently ended).
    pub fn ids(&self) -> impl Iterator<Item = EntityId> + '_ {
        self.lives.keys().copied()
    }

    fn open_life(&mut self, id: EntityId) -> Option<&mut Life> {
        self.lives
            .get_mut(&id)?
            .last_mut()
            .filter(|l| l.despawn_tick.is_none())
    }

    fn spawn(&mut self, id: EntityId, tick: u64, pos: [f64; 3]) {
        let lives = self.lives.entry(id).or_default();
        if let Some(open) = lives.last_mut().filter(|l| l.despawn_tick.is_none()) {
            open.despawn_tick = Some(tick);
        }
        let mut samples = VecDeque::with_capacity(4);
        samples.push_back(Sample {
            tick,
            pos,
            components: 0,
        });
        lives.push(Life {
            spawn_tick: tick,
            despawn_tick: None,
            samples,
        });
    }

    /// Add an update to an open life (see the module docs).
    fn push(life: &mut Life, tick: u64, pos: [f64; 3], components_changed: bool) {
        let samples = &mut life.samples;
        let last = *samples.back().expect("a life has a sample");
        let moved = last.pos != pos;
        if !moved && !components_changed {
            return;
        }
        let components = if components_changed {
            last.components + 1
        } else {
            last.components
        };
        if last.tick == tick {
            *samples.back_mut().expect("checked") = Sample {
                tick,
                pos,
                components,
            };
            return;
        }
        if moved && last.tick + 1 < tick {
            // No update since `last`: it stood still until the tick before.
            samples.push_back(Sample {
                tick: tick - 1,
                ..last
            });
        }
        samples.push_back(Sample {
            tick,
            pos,
            components,
        });
    }

    /// Drop what no render tick at or after `tick - keep_ticks` needs: ended
    /// lives, and samples older than the last one at or before that tick.
    fn prune(&mut self, tick: u64) {
        let oldest = tick.saturating_sub(self.keep_ticks);
        self.lives.retain(|_, lives| {
            lives.retain(|l| l.despawn_tick.is_none_or(|d| d > oldest));
            for life in lives.iter_mut() {
                while life.samples.len() > 1 && life.samples[1].tick <= oldest {
                    life.samples.pop_front();
                }
            }
            !lives.is_empty()
        });
    }
}

fn place_entity(lives: &[Life], render_tick: f64) -> Option<[f64; 3]> {
    let life = lives
        .iter()
        .find(|l| l.despawn_tick.is_none_or(|d| (d as f64) > render_tick))?;
    if (life.spawn_tick as f64) > render_tick {
        return None;
    }
    Some(place(&life.samples, render_tick))
}

/// The client interpolators' rule: between the last sample at or before
/// the render tick and the next one, linearly; held at the last sample
/// after it.
fn place(samples: &VecDeque<Sample>, render_tick: f64) -> [f64; 3] {
    let mut i = samples.len() - 1;
    while i > 0 && (samples[i].tick as f64) > render_tick {
        i -= 1;
    }
    let from = samples[i];
    let to = samples.get(i + 1).copied().unwrap_or(from);
    let t = if to.tick == from.tick {
        0.0
    } else {
        ((render_tick - from.tick as f64) / (to.tick as f64 - from.tick as f64)).clamp(0.0, 1.0)
    };
    let d = [
        to.pos[0] - from.pos[0],
        to.pos[1] - from.pos[1],
        to.pos[2] - from.pos[2],
    ];
    [
        from.pos[0] + d[0] * t,
        from.pos[1] + d[1] * t,
        from.pos[2] + d[2] * t,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(id: EntityId, q: [i64; 3]) -> Sent {
        Sent {
            id,
            q,
            components_changed: false,
        }
    }

    #[test]
    fn an_entity_is_placed_between_samples_and_held_after_the_last() {
        let mut v = ViewHistory::new(20);
        v.record(1, 1.0, true, &[s(7, [0, 0, 0])], &[], &[]);
        v.record(2, 1.0, false, &[], &[s(7, [10, 0, 0])], &[]);
        v.record(3, 1.0, false, &[], &[s(7, [30, 0, 0])], &[]);
        assert_eq!(v.position_at(7, 1.0), Some([0.0, 0.0, 0.0]));
        assert_eq!(v.position_at(7, 1.5), Some([5.0, 0.0, 0.0]));
        assert_eq!(v.position_at(7, 2.25), Some([15.0, 0.0, 0.0]));
        assert_eq!(v.position_at(7, 9.0), Some([30.0, 0.0, 0.0]));
        // Before its spawn, the subscriber did not draw it.
        assert_eq!(v.position_at(7, 0.5), None);
        assert_eq!(v.position_at(8, 2.0), None);
    }

    #[test]
    fn a_quiet_entity_stands_still_until_the_tick_before_it_moves() {
        let mut v = ViewHistory::new(20);
        v.record(10, 0.5, false, &[s(1, [0, 0, 0])], &[], &[]);
        // No update for ticks 11-14: deferred, or it did not move.
        v.record(15, 0.5, false, &[], &[s(1, [20, 0, 0])], &[]);
        assert_eq!(v.position_at(1, 13.0), Some([0.0, 0.0, 0.0]));
        assert_eq!(v.position_at(1, 14.0), Some([0.0, 0.0, 0.0]));
        assert_eq!(v.position_at(1, 14.5), Some([5.0, 0.0, 0.0]));
        assert_eq!(v.position_at(1, 15.0), Some([10.0, 0.0, 0.0]));
    }

    #[test]
    fn a_despawn_ends_the_life_and_a_respawn_starts_another() {
        let mut v = ViewHistory::new(20);
        v.record(1, 1.0, false, &[s(4, [1, 1, 1])], &[], &[]);
        v.record(3, 1.0, false, &[], &[], &[4]);
        v.record(5, 1.0, false, &[s(4, [9, 9, 9])], &[], &[]);
        assert_eq!(v.position_at(4, 2.9), Some([1.0, 1.0, 1.0]));
        assert_eq!(v.position_at(4, 3.0), None);
        assert_eq!(v.position_at(4, 4.0), None);
        assert_eq!(v.position_at(4, 5.0), Some([9.0, 9.0, 9.0]));
    }

    #[test]
    fn a_full_frame_ends_lives_it_does_not_list() {
        let mut v = ViewHistory::new(20);
        v.record(1, 1.0, true, &[s(1, [0, 0, 0]), s(2, [5, 0, 0])], &[], &[]);
        v.record(4, 1.0, true, &[s(1, [3, 0, 0])], &[], &[]);
        assert_eq!(v.position_at(2, 3.5), Some([5.0, 0.0, 0.0]));
        assert_eq!(v.position_at(2, 4.0), None);
        // Entity 1 moved: it stood still until tick 3, then moved to 3.
        assert_eq!(v.position_at(1, 3.0), Some([0.0, 0.0, 0.0]));
        assert_eq!(v.position_at(1, 4.0), Some([3.0, 0.0, 0.0]));
    }

    #[test]
    fn a_component_change_without_a_move_adds_a_sample() {
        let mut v = ViewHistory::new(20);
        v.record(1, 1.0, false, &[s(1, [0, 0, 0])], &[], &[]);
        let hp = Sent {
            components_changed: true,
            ..s(1, [0, 0, 0])
        };
        v.record(5, 1.0, false, &[], &[hp], &[]);
        v.record(6, 1.0, false, &[], &[s(1, [10, 0, 0])], &[]);
        // The tick-5 sample holds it still until 5; it moves from there.
        assert_eq!(v.position_at(1, 5.0), Some([0.0, 0.0, 0.0]));
        assert_eq!(v.position_at(1, 5.5), Some([5.0, 0.0, 0.0]));
    }

    #[test]
    fn old_samples_and_lives_are_pruned() {
        let mut v = ViewHistory::new(5);
        v.record(1, 1.0, false, &[s(1, [0, 0, 0]), s(2, [0, 0, 0])], &[], &[]);
        v.record(2, 1.0, false, &[], &[], &[2]);
        for t in 3..=30 {
            v.record(t, 1.0, false, &[], &[s(1, [t as i64, 0, 0])], &[]);
        }
        assert!(!v.lives.contains_key(&2), "an ended life long gone");
        let samples = &v.lives[&1][0].samples;
        // At most the window, the prune interval, and a sample either side.
        assert!(
            samples.len() <= 5 + PRUNE_EVERY as usize + 2,
            "{} samples",
            samples.len()
        );
        // What the window covers is still exact.
        assert_eq!(v.position_at(1, 25.5), Some([25.5, 0.0, 0.0]));
    }

    #[test]
    fn updates_in_any_order_land_on_their_entities() {
        let mut v = ViewHistory::new(20);
        v.record(
            1,
            1.0,
            false,
            &[s(1, [0, 0, 0]), s(2, [0, 0, 0]), s(3, [0, 0, 0])],
            &[],
            &[],
        );
        v.record(2, 1.0, false, &[], &[s(3, [3, 0, 0]), s(1, [1, 0, 0])], &[]);
        v.record(3, 1.0, false, &[], &[s(1, [2, 0, 0]), s(3, [6, 0, 0])], &[]);
        assert_eq!(v.position_at(1, 3.0), Some([2.0, 0.0, 0.0]));
        assert_eq!(v.position_at(2, 3.0), Some([0.0, 0.0, 0.0]));
        assert_eq!(v.position_at(3, 2.0), Some([3.0, 0.0, 0.0]));
        assert_eq!(v.position_at(3, 3.0), Some([6.0, 0.0, 0.0]));
    }

    #[test]
    fn near_finds_the_entities_within_the_radius_at_the_tick() {
        let mut v = ViewHistory::new(20);
        v.record(
            1,
            1.0,
            false,
            &[s(1, [0, 0, 0]), s(2, [100, 0, 0])],
            &[],
            &[],
        );
        v.record(2, 1.0, false, &[], &[s(2, [10, 0, 0])], &[]);
        assert_eq!(
            v.entities_near([0.0, 0.0, 0.0], 20.0, 1.0),
            vec![(1, [0.0, 0.0, 0.0])]
        );
        assert_eq!(
            v.entities_near([0.0, 0.0, 0.0], 20.0, 2.0),
            vec![(1, [0.0, 0.0, 0.0]), (2, [10.0, 0.0, 0.0])]
        );
    }

    #[test]
    fn sparse_and_dense_updates_match_individual_lookups() {
        let spawned: Vec<_> = (0..256).map(|id| s(id, [id as i64, 0, 0])).collect();
        let mut history = ViewHistory::new(10);
        history.record(1, 0.5, true, &spawned, &[], &[]);
        let mut expected = history.clone();
        for tick in 2..=40 {
            // Include both sides of the density threshold and a missing id.
            let count = [1, 2, 31, 32, 33, 128, 256][tick as usize % 7];
            let updated: Vec<_> = (0..count)
                .map(|i| Sent {
                    components_changed: tick % 3 == 0,
                    ..s(256 - count + i, [i as i64, tick as i64, -1])
                })
                .chain([s(999, [0; 3])])
                .collect();
            let despawned = if tick == 5 { vec![255] } else { vec![] };
            let respawned = if tick == 10 {
                vec![s(255, [0; 3])]
            } else {
                vec![]
            };
            history.record(tick, 0.5, false, &respawned, &updated, &despawned);
            // Reverse order forces the lookup path, including for dense frames.
            let reversed: Vec<_> = updated.iter().rev().copied().collect();
            expected.record(tick, 0.5, false, &respawned, &reversed, &despawned);
            assert_eq!(
                history.ids().collect::<Vec<_>>(),
                expected.ids().collect::<Vec<_>>()
            );
            for (id, lives) in &history.lives {
                let wanted = &expected.lives[id];
                assert_eq!(lives.len(), wanted.len());
                for (actual, expected) in lives.iter().zip(wanted) {
                    assert_eq!(actual.spawn_tick, expected.spawn_tick);
                    assert_eq!(actual.despawn_tick, expected.despawn_tick);
                    assert_eq!(actual.samples, expected.samples);
                }
            }
        }
    }

    #[test]
    fn nearby_queries_match_positions_across_entity_lifetimes() {
        let mut history = ViewHistory::new(20);
        history.record(1, 1.0, true, &[s(1, [0; 3]), s(2, [10, 0, 0])], &[], &[]);
        history.record(3, 1.0, false, &[], &[s(2, [0, 10, 0])], &[1]);
        history.record(5, 1.0, false, &[s(1, [0, 0, 10]), s(3, [20; 3])], &[], &[]);
        for tick in [0.0, 1.0, 2.5, 3.0, 4.0, 5.0, 6.0] {
            for radius in [0.0, 10.0, 30.0, -10.0, f64::INFINITY, f64::NAN] {
                let expected: Vec<_> = history
                    .ids()
                    .filter_map(|id| {
                        let p = history.position_at(id, tick)?;
                        let distance = p[0] * p[0] + p[1] * p[1] + p[2] * p[2];
                        (distance <= radius * radius).then_some((id, p))
                    })
                    .collect();
                assert_eq!(history.entities_near([0.0; 3], radius, tick), expected);
            }
        }
    }
}
