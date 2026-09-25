//! Interest management cost per tick (issue #26 target: under 2 ms for 300
//! subscribers and 1,000 entities on one core).
//!
//! ```sh
//! cargo bench -p pylon-realtime --bench interest_bench
//! ```
//!
//! Each tick moves every entity, rebuilds the grid, and updates all 300
//! views (the grid query, distance checks with hysteresis, the filter, and
//! the entered/left lists). Two worlds: entities spread over 2,000 x 2,000
//! units, and a crowd in 400 x 400 where each subscriber sees most of them.

use std::time::{Duration, Instant};

use pylon_realtime::{EntityPos, InterestArea, InterestConfig, InterestManager, SubscriberId};

/// A small xorshift, so the bench needs no dependencies and runs the same
/// every time.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }
}

fn run(name: &str, world: f32, radius: f32) {
    const SUBSCRIBERS: usize = 300;
    const ENTITIES: usize = 1000;
    const TICKS: usize = 400;
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let mut entities: Vec<EntityPos> = (0..ENTITIES)
        .map(|i| EntityPos {
            id: i as u64,
            x: rng.next() * world,
            y: rng.next() * world,
        })
        .collect();
    let subs: Vec<SubscriberId> = (0..SUBSCRIBERS)
        .map(|i| SubscriberId::new(format!("p{i}")))
        .collect();
    let mut manager = InterestManager::new(InterestConfig {
        cell_size: radius,
        margin: radius * 0.1,
    });

    let mut times: Vec<Duration> = Vec::with_capacity(TICKS);
    let mut visible_total = 0usize;
    for tick in 0..TICKS + 20 {
        // Everyone walks a little (10 units per tick at most).
        for e in &mut entities {
            e.x = (e.x + (rng.next() - 0.5) * 20.0).clamp(0.0, world);
            e.y = (e.y + (rng.next() - 0.5) * 20.0).clamp(0.0, world);
        }
        let start = Instant::now();
        manager.rebuild(&entities);
        for (i, sid) in subs.iter().enumerate() {
            // Subscriber i controls entity i and cannot see entity ids that
            // are multiples of 97 (stealth).
            let me = entities[i];
            let area = InterestArea {
                x: me.x,
                y: me.y,
                radius,
            };
            let view = manager.update(sid, Some(area), |ids| {
                ids.retain(|id| id % 97 != 0 || *id == i as u64)
            });
            visible_total += view.visible.len();
        }
        let took = start.elapsed();
        if tick >= 20 {
            times.push(took);
        }
    }
    times.sort();
    let p = |q: f64| times[((times.len() - 1) as f64 * q) as usize];
    println!(
        "{name:<34} p50 {:>7.3} ms   p99 {:>7.3} ms   ({} visible per subscriber on average)",
        p(0.5).as_secs_f64() * 1000.0,
        p(0.99).as_secs_f64() * 1000.0,
        visible_total / ((TICKS + 20) * SUBSCRIBERS),
    );
}

fn main() {
    println!("interest management, 300 subscribers, 1,000 moving entities, per tick:");
    run("spread (2000 x 2000, radius 150)", 2000.0, 150.0);
    run("crowd (400 x 400, radius 150)", 400.0, 150.0);
}
