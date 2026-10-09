//! Run with `cargo bench -p pylon-replication --bench view_history`.
use std::hint::black_box;
use std::time::{Duration, Instant};

use pylon_replication::{view::Sent, ViewHistory};

fn history(entities: u64) -> ViewHistory {
    let mut history = ViewHistory::new(10);
    let spawned: Vec<_> = (0..entities)
        .map(|id| Sent {
            id,
            q: [id as i64, 0, 0],
            components_changed: false,
        })
        .collect();
    history.record(1, 1.0, true, &spawned, &[], &[]);
    history
}

fn report(name: &str, mut times: Vec<Duration>) {
    let mean = times.iter().sum::<Duration>().as_secs_f64() * 1e6 / times.len() as f64;
    times.sort_unstable();
    println!(
        "{name}: p50 {:.2} us, mean {mean:.2} us",
        times[times.len() / 2].as_secs_f64() * 1e6
    );
}

fn updates(entities: u64, changed: u64) {
    let mut history = history(entities);
    let mut updated: Vec<_> = (0..changed)
        .map(|i| Sent {
            id: (i + 1) * entities / changed - 1,
            q: [0; 3],
            components_changed: false,
        })
        .collect();
    let mut times = Vec::new();
    for tick in 2..=1000 {
        for sent in &mut updated {
            sent.q[1] = tick as i64;
        }
        let start = Instant::now();
        history.record(tick, 1.0, false, &[], &updated, &[]);
        black_box(&history);
        if tick > 100 {
            times.push(start.elapsed());
        }
    }
    report(&format!("{entities} entities, {changed} updates"), times);
}

fn main() {
    updates(100_000, 1);
    updates(100_000, 100);
    updates(1_000, 1_000);
    let history = history(10_000);
    let mut times = Vec::new();
    for i in 0..300 {
        let start = Instant::now();
        black_box(history.entities_near([5_000.0, 0.0, 0.0], 20.0, 1.0));
        if i >= 100 {
            times.push(start.elapsed());
        }
    }
    report("nearby query, 10,000 entities", times);
}
