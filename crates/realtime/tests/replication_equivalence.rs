//! `Replicator::frame` produces the same frames as a straightforward
//! reference implementation: a hash map of what each subscriber has, a
//! despawn pass, then a pass over the visible ids.
//!
//! The replicator is written for speed (one merge of sorted lists, reused
//! buffers, ranking only when the budget must choose). This test drives
//! both through the same random store and view changes, for many ticks and
//! subscribers, and compares every frame byte for byte.

use std::collections::HashMap;

use pylon_realtime::replication::{FrameInput, FrameOutput, Plane, ReplicationConfig, Replicator};
use pylon_realtime::{InterestArea, Replicated};
use pylon_replication::frame::{encode_update_body, quantize3, ComponentChange, FrameBuilder};
use pylon_replication::EntityId;

// ---------------------------------------------------------------------------
// The reference implementation
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct Sent {
    spawn_seq: u64,
    q: [i64; 3],
    seq: u64,
    tick: u64,
}

#[derive(Default)]
struct Baseline {
    entities: HashMap<EntityId, Sent>,
    precision: f32,
    dropped: u64,
    started: bool,
}

#[derive(Default)]
struct Reference {
    baselines: HashMap<u64, Baseline>,
    all_ids: Vec<EntityId>,
    store_id: Option<u64>,
}

struct Candidate {
    id: EntityId,
    priority: f64,
    body: Vec<u8>,
    delta: [i64; 3],
    seq: u64,
}

fn project(plane: Plane, p: [f32; 3]) -> (f32, f32) {
    match plane {
        Plane::XY => (p[0], p[1]),
        Plane::XZ => (p[0], p[2]),
    }
}

impl Reference {
    fn begin_tick(&mut self, store: &Replicated) {
        if self.store_id != Some(store.store_id()) {
            self.baselines.clear();
            self.store_id = Some(store.store_id());
        }
        self.all_ids.clear();
        self.all_ids.extend(store.iter().map(|(id, _)| id));
    }

    fn frame(
        &mut self,
        store: &Replicated,
        config: &ReplicationConfig,
        tick: u64,
        input: FrameInput<'_>,
    ) -> FrameOutput {
        let precision = config.precision;
        let visible: &[EntityId] = input.visible.unwrap_or(&self.all_ids);
        let base = self.baselines.entry(input.key).or_default();
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
        let mut candidates = Vec::new();
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
                        base.entities.get_mut(&id).unwrap().seq = store.seq();
                        continue;
                    }
                    let staleness = (tick.saturating_sub(sent.tick) + 1) as f64;
                    let distance = input.area.map_or(0.0, |a| {
                        let (x, y) = project(config.plane, e.pos);
                        let (dx, dy) = (x as f64 - a.x as f64, y as f64 - a.y as f64);
                        (dx * dx + dy * dy).sqrt() / (a.radius as f64).max(1e-6)
                    });
                    candidates.push(Candidate {
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
        let budget = config.max_bytes_per_tick;
        if budget > 0 {
            candidates.sort_by(|a, b| {
                b.priority
                    .partial_cmp(&a.priority)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(a.id.cmp(&b.id))
            });
            let mut used = 0usize;
            let mut first = true;
            candidates.retain(|c| {
                let cost = c.body.len() + pylon_replication::varint::len_u64(c.id);
                if first || used + cost <= budget {
                    first = false;
                    used += cost;
                    true
                } else {
                    false
                }
            });
            candidates.sort_by_key(|c| c.id);
        }
        for c in &candidates {
            frame.update(c.id, &c.body);
            let sent = base.entities.get_mut(&c.id).unwrap();
            for i in 0..3 {
                sent.q[i] = sent.q[i].wrapping_add(c.delta[i]);
            }
            sent.seq = c.seq;
            sent.tick = tick;
        }
        FrameOutput {
            bytes: frame.finish(),
            delta_of: (!full).then_some(input.dropped),
            datagrams: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// The scenario
// ---------------------------------------------------------------------------

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
    fn f(&mut self, max: f32) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32 * max
    }
}

struct Sub {
    key: u64,
    dropped: u64,
}

fn run(seed: u64, budget: usize, with_area: bool) {
    let mut rng = Rng(seed);
    let mut store = Replicated::new();
    let mut fast = Replicator::new();
    let mut slow = Reference::default();
    let mut precision = 0.05f32;
    let mut subs: Vec<Sub> = (0..6).map(|k| Sub { key: k, dropped: 0 }).collect();
    for id in 0..40 {
        store.spawn(id, [rng.f(100.0), rng.f(100.0), 0.0]);
    }
    for tick in 1..=300u64 {
        // Change the store.
        let ids: Vec<EntityId> = store.iter().map(|(id, _)| id).collect();
        for &id in &ids {
            if rng.chance(60) {
                let p = store.get(id).unwrap().pos;
                // Some moves are smaller than the precision.
                let step = if rng.chance(20) { 0.01 } else { 2.0 };
                store.set_pos(
                    id,
                    [
                        p[0] + (rng.f(2.0) - 1.0) * step,
                        p[1] + (rng.f(2.0) - 1.0) * step,
                        p[2],
                    ],
                );
            }
            if rng.chance(8) {
                let c = rng.below(3) as u8;
                let v = rng.below(1000).to_le_bytes();
                store.set_component(id, c, &v[..1 + rng.below(4) as usize]);
            }
            if rng.chance(3) {
                store.remove_component(id, rng.below(3) as u8);
            }
            if rng.chance(2) {
                store.despawn(id);
            }
        }
        // Spawns, some reusing ids so an id names a new entity.
        for _ in 0..rng.below(3) {
            let id = rng.below(60);
            if rng.chance(50) && store.contains(id) {
                store.despawn(id);
            }
            store.spawn(id, [rng.f(100.0), rng.f(100.0), rng.f(10.0)]);
        }
        if rng.chance(1) {
            precision = if precision == 0.05 { 0.1 } else { 0.05 };
        }
        // Now and then the game swaps in a new store.
        if rng.chance(1) {
            let mut fresh = Replicated::new();
            for (id, e) in store.iter() {
                fresh.spawn(id, e.pos);
            }
            store = fresh;
        }
        // A subscription can end and a new one start.
        if rng.chance(2) {
            let i = rng.below(subs.len() as u64) as usize;
            subs[i] = Sub {
                key: 100 + tick,
                dropped: 0,
            };
        }

        let config = ReplicationConfig {
            precision,
            max_bytes_per_tick: budget,
            plane: if seed.is_multiple_of(2) {
                Plane::XY
            } else {
                Plane::XZ
            },
        };
        fast.begin_tick(&store);
        slow.begin_tick(&store);
        let keys: Vec<u64> = subs.iter().map(|s| s.key).collect();
        fast.retain(|k| keys.contains(&k));
        slow.baselines.retain(|k, _| keys.contains(k));
        let all: Vec<EntityId> = store.iter().map(|(id, _)| id).collect();
        for sub in &mut subs {
            if rng.chance(3) {
                sub.dropped += 1;
            }
            let queue_full = rng.chance(2);
            // A view: everything, or a sorted subset that may name ids the
            // store no longer has.
            let view: Option<Vec<EntityId>> = if rng.chance(30) {
                None
            } else {
                let mut v: Vec<EntityId> = Vec::new();
                for &id in &all {
                    if rng.chance(70) {
                        v.push(id);
                    }
                }
                for _ in 0..rng.below(3) {
                    v.push(60 + rng.below(10));
                }
                v.sort_unstable();
                v.dedup();
                Some(v)
            };
            let area = with_area.then(|| InterestArea {
                x: rng.f(100.0),
                y: rng.f(100.0),
                radius: 5.0 + rng.f(50.0),
            });
            let input = FrameInput {
                key: sub.key,
                visible: view.as_deref(),
                area,
                dropped: sub.dropped,
                queue_full,
                datagram: None,
            };
            let a = fast.frame(&store, &config, tick, input);
            let input = FrameInput {
                key: sub.key,
                visible: view.as_deref(),
                area,
                dropped: sub.dropped,
                queue_full,
                datagram: None,
            };
            let b = slow.frame(&store, &config, tick, input);
            assert_eq!(
                a.bytes, b.bytes,
                "seed {seed} budget {budget} tick {tick} key {}: frames differ",
                sub.key
            );
            assert_eq!(a.delta_of, b.delta_of, "seed {seed} tick {tick}");
        }
    }
}

#[test]
fn frames_match_the_reference_without_a_budget() {
    for seed in 1..=12 {
        run(0x9e37_79b9_7f4a_7c15 ^ seed, 0, seed % 3 != 0);
    }
}

#[test]
fn frames_match_the_reference_with_a_budget() {
    for seed in 1..=12 {
        for budget in [1, 24, 60, 200] {
            run(0x2545_f491_4f6c_dd1d ^ seed, budget, seed % 3 != 0);
        }
    }
}
