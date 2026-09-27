//! Replication over an unreliable transport converges.
//!
//! Stream frames arrive in order, late. Datagrams are lost, delayed (so
//! they come out of order), and duplicated; acks are delayed and lost too.
//! The store keeps changing: moves (some below the precision), component
//! changes and removals, despawns, ids spawned again, changing views, and a
//! byte budget. When the network turns clean and the store holds still, the
//! client's table must equal the store for everything in view.

use std::collections::BTreeMap;

use pylon_realtime::replication::{
    DatagramInput, FrameInput, Plane, ReplicationConfig, Replicator,
};
use pylon_realtime::{ReplicaTable, Replicated};
use pylon_replication::frame::quantize3;
use pylon_replication::EntityId;

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

/// Things in flight, each due at a tick.
struct Wire<T> {
    items: Vec<(u64, u64, T)>,
    order: u64,
}

impl<T> Wire<T> {
    fn new() -> Self {
        Self {
            items: Vec::new(),
            order: 0,
        }
    }
    fn send(&mut self, due: u64, item: T) {
        self.order += 1;
        self.items.push((due, self.order, item));
    }
    /// Everything due by `tick`, by due tick and then send order.
    fn take(&mut self, tick: u64) -> Vec<T> {
        let mut due: Vec<(u64, u64, T)> = Vec::new();
        let mut keep = Vec::new();
        for it in self.items.drain(..) {
            if it.0 <= tick {
                due.push(it);
            } else {
                keep.push(it);
            }
        }
        self.items = keep;
        due.sort_by_key(|(d, o, _)| (*d, *o));
        due.into_iter().map(|(_, _, t)| t).collect()
    }
}

struct Network {
    loss: u64,
    dup: u64,
    max_delay: u64,
}

const KEY: u64 = 7;

/// Everything the client must match: position (quantized) and components
/// of each entity in `visible`.
fn assert_converged(
    table: &ReplicaTable,
    store: &Replicated,
    visible: &[EntityId],
    precision: f32,
    seed: u64,
) {
    let want: Vec<EntityId> = visible
        .iter()
        .copied()
        .filter(|id| store.contains(*id))
        .collect();
    let have: Vec<EntityId> = table.entities.keys().copied().collect();
    assert_eq!(have, want, "seed {seed}: entity sets differ");
    for id in want {
        let e = store.get(id).unwrap();
        let got = &table.entities[&id];
        assert_eq!(
            got.q,
            quantize3(e.pos, precision),
            "seed {seed}: position of {id}"
        );
        let comps: BTreeMap<u8, Vec<u8>> = e
            .components
            .iter()
            .filter_map(|(c, v)| v.bytes.clone().map(|b| (*c, b)))
            .collect();
        assert_eq!(got.components, comps, "seed {seed}: components of {id}");
    }
}

fn run(seed: u64, budget: usize, max_size: usize) {
    let mut rng = Rng(seed);
    let precision = 0.05;
    let config = ReplicationConfig {
        precision,
        max_bytes_per_tick: budget,
        plane: Plane::XY,
    };
    let mut store = Replicated::new();
    for id in 0..30 {
        store.spawn(id, [rng.f(100.0), rng.f(100.0), 0.0]);
        store.set_component(id, 1, &[id as u8]);
    }
    let mut rep = Replicator::new();
    let mut table = ReplicaTable::new();
    let mut stream: Wire<Vec<u8>> = Wire::new();
    let mut datagrams: Wire<Vec<u8>> = Wire::new();
    let mut acks: Wire<(u64, u64)> = Wire::new();
    let bad = Network {
        loss: 20,
        dup: 5,
        max_delay: 5,
    };
    let clean = Network {
        loss: 0,
        dup: 0,
        max_delay: 0,
    };
    let mut visible: Vec<EntityId> = store.iter().map(|(id, _)| id).collect();
    let mut dropped = 0u64;
    // The stream is ordered: a frame never arrives before an earlier one.
    let mut stream_due = 0u64;
    let noisy_ticks = 400u64;
    let quiet_ticks = 120u64;
    for tick in 1..=noisy_ticks + quiet_ticks {
        let noisy = tick <= noisy_ticks;
        let net = if noisy { &bad } else { &clean };
        if noisy {
            let ids: Vec<EntityId> = store.iter().map(|(id, _)| id).collect();
            for &id in &ids {
                if rng.chance(50) {
                    let p = store.get(id).unwrap().pos;
                    let step = if rng.chance(20) { 0.01 } else { 3.0 };
                    store.set_pos(
                        id,
                        [
                            (p[0] + (rng.f(2.0) - 1.0) * step).clamp(0.0, 100.0),
                            (p[1] + (rng.f(2.0) - 1.0) * step).clamp(0.0, 100.0),
                            0.0,
                        ],
                    );
                }
                if rng.chance(5) {
                    let v = rng.below(1000).to_le_bytes();
                    store.set_component(id, rng.below(3) as u8, &v[..1 + rng.below(3) as usize]);
                }
                if rng.chance(2) {
                    store.remove_component(id, rng.below(3) as u8);
                }
                if rng.chance(1) {
                    store.despawn(id);
                }
            }
            for _ in 0..rng.below(2) {
                let id = rng.below(40);
                if store.contains(id) {
                    store.despawn(id);
                }
                store.spawn(id, [rng.f(100.0), rng.f(100.0), 0.0]);
            }
            if rng.chance(10) {
                visible = store
                    .iter()
                    .map(|(id, _)| id)
                    .filter(|_| rng.chance(80))
                    .collect();
            }
            // Now and then the outbound queue drops a frame: a full frame.
            if rng.chance(1) {
                dropped += 1;
            }
        }
        if tick == noisy_ticks + 1 {
            // Hold still, in view of everything.
            visible = store.iter().map(|(id, _)| id).collect();
        }

        rep.begin_tick(&store);
        let out = rep.frame(
            &store,
            &config,
            tick,
            FrameInput {
                key: KEY,
                visible: Some(&visible),
                area: None,
                dropped,
                queue_full: false,
                datagram: Some(DatagramInput {
                    max_size,
                    input_ack: 0,
                }),
            },
        );
        for d in &out.datagrams {
            assert!(d.len() <= max_size, "a datagram of {} bytes", d.len());
        }
        let delay = |rng: &mut Rng| {
            if net.max_delay == 0 {
                0
            } else {
                rng.below(net.max_delay + 1)
            }
        };
        if !out.bytes.is_empty() {
            stream_due = stream_due.max(tick + delay(&mut rng));
            stream.send(stream_due, out.bytes);
        }
        for d in out.datagrams {
            if rng.chance(net.loss) {
                continue;
            }
            if rng.chance(net.dup) {
                let later = tick + delay(&mut rng);
                datagrams.send(later, d.clone());
            }
            datagrams.send(tick + delay(&mut rng), d);
        }

        // The client: the stream first, then datagrams.
        for f in stream.take(tick) {
            table.apply(&f).expect("a stream frame applies");
        }
        let mut applied = Vec::new();
        for d in datagrams.take(tick) {
            let s = table.apply_datagram(&d).expect("a datagram decodes");
            applied.push((s.frame, table.frames_applied));
        }
        for a in applied {
            if rng.chance(net.loss) {
                continue;
            }
            acks.send(tick + delay(&mut rng), a);
        }
        let due = acks.take(tick);
        if !due.is_empty() {
            rep.ack_datagrams(KEY, &due);
        }
    }
    assert_converged(&table, &store, &visible, precision, seed);
}

#[test]
fn converges_through_loss_reorder_and_duplication() {
    for seed in 1..=10 {
        run(0x9e37_79b9_7f4a_7c15 ^ seed, 0, 1200);
    }
}

#[test]
fn converges_with_a_budget_and_small_datagrams() {
    for seed in 1..=10 {
        for (budget, max_size) in [(120, 1200), (400, 80), (60, 60)] {
            run(0x2545_f491_4f6c_dd1d ^ seed, budget, max_size);
        }
    }
}

/// An ack proves the client has that datagram's state or a later one. The
/// client got A then B, the acks for B were lost, and the entity moved back
/// to where it was in A: the replicator must still send it.
#[test]
fn an_ack_of_an_older_datagram_does_not_hide_a_newer_state() {
    let precision = 1.0;
    let config = ReplicationConfig {
        precision,
        max_bytes_per_tick: 0,
        plane: Plane::XY,
    };
    let mut store = Replicated::new();
    store.spawn(1, [0.0, 0.0, 0.0]);
    let mut rep = Replicator::new();
    let mut table = ReplicaTable::new();
    let dg = Some(DatagramInput {
        max_size: 1200,
        input_ack: 0,
    });
    let frame = |rep: &mut Replicator, store: &Replicated, tick: u64| {
        rep.begin_tick(store);
        rep.frame(
            store,
            &config,
            tick,
            FrameInput {
                key: KEY,
                visible: None,
                area: None,
                dropped: 0,
                queue_full: false,
                datagram: dg,
            },
        )
    };
    let spawn = frame(&mut rep, &store, 1);
    table.apply(&spawn.bytes).unwrap();

    store.set_pos(1, [5.0, 0.0, 0.0]);
    let a = frame(&mut rep, &store, 2).datagrams.remove(0);
    store.set_pos(1, [9.0, 0.0, 0.0]);
    let b = frame(&mut rep, &store, 3).datagrams.remove(0);
    // B arrives first, then A (skipped: older). Only A's ack gets back.
    let sb = table.apply_datagram(&b).unwrap();
    let sa = table.apply_datagram(&a).unwrap();
    assert_eq!(table.entities[&1].q[0], 9);
    rep.ack_datagrams(KEY, &[(sa.frame, table.frames_applied)]);
    let _ = sb;

    // Back to A's position: the client is at 9, so this must go out.
    store.set_pos(1, [5.0, 0.0, 0.0]);
    let c = frame(&mut rep, &store, 4);
    assert_eq!(c.datagrams.len(), 1, "the move back is sent");
    table.apply_datagram(&c.datagrams[0]).unwrap();
    assert_eq!(table.entities[&1].q[0], 5);
}

/// A client that stops acking does not make the replicator keep states
/// without bound: after enough unacked updates the entity spawns again on
/// the stream, which the client applies in order.
#[test]
fn an_entity_with_too_many_unacked_updates_spawns_again() {
    let config = ReplicationConfig {
        precision: 1.0,
        max_bytes_per_tick: 0,
        plane: Plane::XY,
    };
    let mut store = Replicated::new();
    store.spawn(1, [0.0, 0.0, 0.0]);
    let mut rep = Replicator::new();
    let mut table = ReplicaTable::new();
    let mut respawns = 0;
    for tick in 1..=100u64 {
        store.set_pos(1, [tick as f32, 0.0, 0.0]);
        rep.begin_tick(&store);
        let out = rep.frame(
            &store,
            &config,
            tick,
            FrameInput {
                key: KEY,
                visible: None,
                area: None,
                dropped: 0,
                queue_full: false,
                datagram: Some(DatagramInput {
                    max_size: 1200,
                    input_ack: 0,
                }),
            },
        );
        if !out.bytes.is_empty() {
            let s = table.apply(&out.bytes).unwrap();
            if !s.full && s.spawned == vec![1] {
                respawns += 1;
            }
        }
        for d in &out.datagrams {
            table.apply_datagram(d).unwrap();
        }
        assert_eq!(table.entities[&1].q[0], tick as i64);
    }
    assert!(respawns >= 2, "respawned {respawns} times in 100 ticks");
}

/// A datagram that arrives before the stream frame with its entity's spawn
/// changes nothing, and its ack must not count: the client acks it with the
/// number of stream frames it had then.
#[test]
fn an_update_that_beat_its_spawn_is_sent_again() {
    let config = ReplicationConfig {
        precision: 1.0,
        max_bytes_per_tick: 0,
        plane: Plane::XY,
    };
    let mut store = Replicated::new();
    let mut rep = Replicator::new();
    let mut table = ReplicaTable::new();
    let frame = |rep: &mut Replicator, store: &Replicated, tick: u64| {
        rep.begin_tick(store);
        rep.frame(
            store,
            &config,
            tick,
            FrameInput {
                key: KEY,
                visible: None,
                area: None,
                dropped: 0,
                queue_full: false,
                datagram: Some(DatagramInput {
                    max_size: 1200,
                    input_ack: 0,
                }),
            },
        )
    };
    let empty = frame(&mut rep, &store, 1);
    table.apply(&empty.bytes).unwrap();

    store.spawn(1, [0.0, 0.0, 0.0]);
    let spawn = frame(&mut rep, &store, 2);
    store.set_pos(1, [0.0, 7.0, 0.0]);
    let update = frame(&mut rep, &store, 3).datagrams.remove(0);

    // The update arrives first: skipped, and acked with the frames the
    // client had then.
    let s = table.apply_datagram(&update).unwrap();
    assert_eq!(s.skipped, 1);
    rep.ack_datagrams(KEY, &[(s.frame, table.frames_applied)]);
    table.apply(&spawn.bytes).unwrap();
    assert_eq!(table.entities[&1].q[1], 0);

    // Nothing changes now, but the client is not where the store is.
    let next = frame(&mut rep, &store, 4);
    assert_eq!(next.datagrams.len(), 1, "the skipped update goes out again");
    table.apply_datagram(&next.datagrams[0]).unwrap();
    assert_eq!(table.entities[&1].q[1], 7);
}
