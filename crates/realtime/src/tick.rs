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
        } else {
            shard.run_tick();
        }

        drop(shard);

        // Sleep until the next tick, correcting for drift.
        precise_sleep_until(next_tick);
        next_tick += interval;

        // If we've fallen badly behind, reset so we don't spin.
        if next_tick + interval < Instant::now() {
            next_tick = Instant::now() + interval;
        }
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
