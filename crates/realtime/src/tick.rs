//! Fixed-timestep tick loop that drives one or many shards.

use std::sync::{Arc, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::shard::{Shard, SimState};

// ---------------------------------------------------------------------------
// TickLoop — drives a single shard
// ---------------------------------------------------------------------------

/// Drives a shard at a fixed tick rate from a dedicated background thread.
///
/// Holds a `Weak` reference to the shard so the loop exits when the shard
/// is dropped. On shard shutdown (`stop()` or `is_finished()` returning
/// true) the loop exits cleanly.
pub struct TickLoop {
    handle: Option<JoinHandle<()>>,
}

impl TickLoop {
    /// Spawn a tick loop for the given shard.
    ///
    /// If `tick_rate_hz == 0`, the loop polls at 100 Hz but only runs ticks
    /// when inputs arrive (event-driven mode, useful for turn-based games).
    pub fn spawn<S: SimState>(shard: Arc<Shard<S>>) -> Self {
        let tick_rate_hz = shard.config().tick_rate_hz;
        let tick_interval = if tick_rate_hz == 0 {
            Duration::from_millis(10) // poll interval for event-driven mode
        } else {
            Duration::from_nanos(1_000_000_000 / tick_rate_hz as u64)
        };
        let event_driven = tick_rate_hz == 0;

        let weak = Arc::downgrade(&shard);
        // Drop the strong reference so the loop exits when the caller drops
        // their Arc.
        drop(shard);

        let handle = std::thread::Builder::new()
            .name(format!(
                "shard-tick-{}",
                if event_driven { "event" } else { "fixed" }
            ))
            .spawn(move || run_loop(weak, tick_interval, event_driven))
            .expect("failed to spawn shard tick loop");

        Self {
            handle: Some(handle),
        }
    }

    /// Wait for the loop to exit. Usually happens when the shard stops
    /// or is dropped.
    pub fn join(mut self) {
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for TickLoop {
    fn drop(&mut self) {
        // The loop exits when the Weak<Shard> can no longer upgrade,
        // so dropping without joining is safe.
    }
}

fn run_loop<S: SimState>(weak: Weak<Shard<S>>, interval: Duration, event_driven: bool) {
    let mut next_tick = Instant::now() + interval;

    loop {
        let shard = match weak.upgrade() {
            Some(s) => s,
            None => return, // shard dropped, exit
        };
        if !shard.is_running() {
            return;
        }

        if event_driven {
            // Only tick if there are inputs OR subscribers need a fresh snapshot.
            // For now, we tick when there are pending inputs; otherwise sleep.
            if shard.input_queue_len() > 0 {
                shard.run_tick();
            }
            next_tick += interval;
        } else {
            shard.run_tick();
            next_tick += interval;
            catch_up(&shard, &mut next_tick, interval);
        }

        drop(shard);

        // Sleep until the next tick, correcting for drift.
        precise_sleep_until(next_tick);
        if event_driven && next_tick + interval < Instant::now() {
            next_tick = Instant::now() + interval;
        }
    }
}

/// When ticks are overdue, run up to `max_catch_up_ticks` of them back to
/// back (each with the same fixed `dt`, so simulated time keeps pace with
/// wall time), then skip the rest and count them as an overrun.
fn catch_up<S: SimState>(shard: &Shard<S>, next_tick: &mut Instant, interval: Duration) {
    let max = shard.config().max_catch_up_ticks;
    let mut ran = 0;
    while *next_tick <= Instant::now() && ran < max && shard.is_running() {
        shard.run_tick();
        *next_tick += interval;
        ran += 1;
    }
    let now = Instant::now();
    if *next_tick <= now {
        let behind = now.duration_since(*next_tick).as_nanos() / interval.as_nanos().max(1) + 1;
        *next_tick += interval * behind as u32;
        shard.record_overrun(behind as u64);
    }
}

/// Sleep until `target`, with sub-millisecond precision.
///
/// `std::thread::sleep` typically has ~1ms granularity on Linux/macOS and
/// can be much worse under load. For tick rates above ~60 Hz, that overshoot
/// causes visible jitter. This combines a coarse sleep (everything but the
/// last 1ms) with a busy spin for the tail. Costs a small amount of CPU per
/// tick but keeps tick timing tight.
fn precise_sleep_until(target: Instant) {
    const SPIN_THRESHOLD: Duration = Duration::from_millis(1);
    let now = Instant::now();
    if target <= now {
        return;
    }
    let remaining = target - now;
    if remaining > SPIN_THRESHOLD {
        std::thread::sleep(remaining - SPIN_THRESHOLD);
    }
    while Instant::now() < target {
        std::hint::spin_loop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shard::{Shard, ShardConfig, SimState};
    use crate::subscriber::SubscriberId;

    struct Noop;
    impl SimState for Noop {
        type Input = ();
        type Snapshot = u64;
        type Error = String;
        fn apply_input(
            &mut self,
            _sub: &SubscriberId,
            _input: Self::Input,
            _now: Instant,
        ) -> Result<(), Self::Error> {
            Ok(())
        }
        fn tick(&mut self, _dt: Duration) {}
        fn snapshot(&self) -> Self::Snapshot {
            0
        }
    }

    /// Records every dt; every third tick sleeps past the 50 ms interval.
    struct SlowTicker {
        dts: Vec<Duration>,
    }
    impl SimState for SlowTicker {
        type Input = ();
        type Snapshot = u64;
        type Error = String;
        fn apply_input(&mut self, _s: &SubscriberId, _i: (), _n: Instant) -> Result<(), String> {
            Ok(())
        }
        fn tick(&mut self, dt: Duration) {
            self.dts.push(dt);
            if self.dts.len() % 3 == 0 {
                std::thread::sleep(Duration::from_millis(80));
            }
        }
        fn snapshot(&self) -> u64 {
            0
        }
    }

    #[test]
    fn fixed_timestep_gives_every_tick_the_same_dt_even_when_ticks_run_late() {
        let config = ShardConfig {
            tick_rate_hz: 20,
            idle_ticks_before_shutdown: 0,
            ..Default::default()
        };
        let shard = Shard::new("t", SlowTicker { dts: Vec::new() }, config);
        let handle = TickLoop::spawn(Arc::clone(&shard));
        std::thread::sleep(Duration::from_millis(1200));
        shard.stop();
        handle.join();

        let state = shard.state_for_test();
        // 24 at the target rate; catch-up ticks keep it close despite the
        // slow ticks. The bound leaves room for a loaded CI runner.
        assert!(
            state.dts.len() >= 15,
            "ran {} ticks in 1.2 s",
            state.dts.len()
        );
        assert!(
            state.dts.iter().all(|dt| *dt == Duration::from_millis(50)),
            "dts: {:?}",
            state.dts
        );
    }

    /// Position integrates velocity over dt; inputs set the velocity.
    #[derive(Clone, PartialEq, Debug)]
    struct Mover {
        pos: f64,
        vel: f64,
    }
    impl SimState for Mover {
        type Input = f64;
        type Snapshot = f64;
        type Error = String;
        fn apply_input(&mut self, _s: &SubscriberId, v: f64, _n: Instant) -> Result<(), String> {
            self.vel = v;
            Ok(())
        }
        fn tick(&mut self, dt: Duration) {
            self.pos += self.vel * dt.as_secs_f64();
        }
        fn snapshot(&self) -> f64 {
            self.pos
        }
    }

    #[test]
    fn a_replay_of_the_recorded_inputs_matches_the_live_run() {
        use crate::replay::{replay_to, ReplayLog};
        let config = ShardConfig {
            tick_rate_hz: 50,
            idle_ticks_before_shutdown: 0,
            ..Default::default()
        };
        let initial = Mover { pos: 0.0, vel: 0.0 };
        let shard = Shard::new("t", initial.clone(), config);
        let log: Arc<ReplayLog<f64>> = ReplayLog::new(0);
        log.attach(&shard);
        let handle = TickLoop::spawn(Arc::clone(&shard));
        for v in [1.0, -0.5, 3.25, 0.0, 2.0] {
            shard.push_input(SubscriberId::new("p"), v, None).unwrap();
            std::thread::sleep(Duration::from_millis(73));
        }
        shard.stop();
        handle.join();

        let live = shard.state_for_test().clone();
        let replayed = replay_to(
            initial,
            &log.entries(),
            shard.fixed_dt().unwrap(),
            shard.tick_number(),
        );
        assert_eq!(log.len(), 5);
        assert!(live.pos != 0.0);
        assert_eq!(replayed, live);
    }

    #[test]
    fn tick_loop_runs_until_shard_stops() {
        let config = ShardConfig {
            tick_rate_hz: 100,
            idle_ticks_before_shutdown: 0,
            ..Default::default()
        };
        let shard = Shard::new("t", Noop, config);
        let loop_handle = TickLoop::spawn(Arc::clone(&shard));
        std::thread::sleep(Duration::from_millis(30));

        let ticks_before_stop = shard.tick_number();
        assert!(ticks_before_stop > 0, "expected some ticks to have run");

        shard.stop();
        loop_handle.join();
    }
}
