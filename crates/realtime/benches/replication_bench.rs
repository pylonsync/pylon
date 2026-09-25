//! Replication bandwidth (issue #27 target: under 20 KB/s per client for
//! 300 players in one area, all moving, at 20 Hz).
//!
//! ```sh
//! cargo bench -p pylon-realtime --bench replication_bench
//! ```
//!
//! 300 players walk (1.5 units/s, random heading changes) inside a 60 x 60
//! area, so every player sees every other. Each tick the replicator builds
//! one subscriber's frame and a client table applies it. Reported: bytes
//! per second for that client, the time to build all 300 frames, and how
//! far the client's positions are from the server's (world units), with
//! and without a byte budget.

use std::time::Instant;

use pylon_realtime::replication::{FrameInput, Plane, ReplicationConfig, Replicator};
use pylon_realtime::{ReplicaTable, Replicated};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }
}

fn run(name: &str, precision: f32, budget: usize) {
    const PLAYERS: u64 = 300;
    const TICKS: u64 = 400;
    const HZ: f64 = 20.0;
    let mut rng = Rng(0x2545_f491_4f6c_dd1d);
    let mut store = Replicated::new();
    let mut heading: Vec<f32> = Vec::new();
    for id in 0..PLAYERS {
        store.spawn(id, [rng.next() * 60.0, rng.next() * 60.0, 0.0]);
        heading.push(rng.next() * std::f32::consts::TAU);
    }
    let config = ReplicationConfig {
        precision,
        max_bytes_per_tick: budget,
        plane: Plane::XY,
    };
    let mut rep = Replicator::new();
    let mut table = ReplicaTable::new();
    let mut bytes = 0u64;
    let mut build = std::time::Duration::ZERO;
    let mut errors: Vec<f32> = Vec::new();
    let step = 1.5 / HZ as f32;
    for tick in 1..=TICKS {
        for id in 0..PLAYERS {
            let h = &mut heading[id as usize];
            *h += (rng.next() - 0.5) * 0.6;
            let p = store.get(id).unwrap().pos;
            let x = (p[0] + h.cos() * step).clamp(0.0, 60.0);
            let y = (p[1] + h.sin() * step).clamp(0.0, 60.0);
            store.set_pos(id, [x, y, 0.0]);
        }
        rep.begin_tick(&store);
        let start = Instant::now();
        let mut ours = Vec::new();
        // Every subscriber's frame, as the shard builds them each tick.
        for sub in 0..PLAYERS {
            let p = store.get(sub).unwrap().pos;
            let frame = rep.frame(
                &store,
                &config,
                tick,
                FrameInput {
                    key: sub,
                    visible: None,
                    area: Some(pylon_realtime::InterestArea {
                        x: p[0],
                        y: p[1],
                        radius: 60.0,
                    }),
                    dropped: 0,
                    queue_full: false,
                },
            );
            if sub == 0 {
                ours = frame;
            }
        }
        build += start.elapsed();
        table.apply(&ours).unwrap();
        if tick > 20 {
            bytes += ours.len() as u64 + 18; // plus the v2 frame header
            for (id, e) in store.iter() {
                let got = table.pos(id).unwrap();
                let (dx, dy) = (got[0] - e.pos[0], got[1] - e.pos[1]);
                errors.push((dx * dx + dy * dy).sqrt());
            }
        }
    }
    errors.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let secs = (TICKS - 20) as f64 / HZ;
    println!(
        "{name:<30} {:>6.1} KB/s per client   build {:>5.2} ms/tick (300 frames)   position error p50 {:.3} p99 {:.3} max {:.3}",
        bytes as f64 / secs / 1000.0,
        build.as_secs_f64() * 1000.0 / TICKS as f64,
        errors[errors.len() / 2],
        errors[errors.len() * 99 / 100],
        errors[errors.len() - 1],
    );
}

fn main() {
    println!("replication, 300 players in one 60 x 60 area, all moving, 20 Hz:");
    run("precision 0.01, no budget", 0.01, 0);
    run("precision 0.05, no budget", 0.05, 0);
    run("precision 0.01, 900 B/tick", 0.01, 900);
    run("precision 0.05, 900 B/tick", 0.05, 900);
}
