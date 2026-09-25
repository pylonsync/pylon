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
            // Tick only when there is something to apply: inputs, messages
            // from other shards, or call results. A tick applies at most 64
            // messages; the rest keep the shard ticking.
            if shard.has_pending_work() {
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
        // The property is the constant dt below. The count only shows the
        // loop ran: on a loaded runner the 80 ms sleeps run long, so a count
        // near the target rate (24) fails for reasons outside the tick loop.
        assert!(
            state.dts.len() >= 6,
            "ran {} ticks in 1.2 s",
            state.dts.len()
        );
        assert!(
            state.dts.iter().all(|dt| *dt == Duration::from_millis(50)),
            "dts: {:?}",
            state.dts
        );
    }

    #[test]
    fn catch_up_runs_overdue_ticks_up_to_the_cap_and_counts_the_rest() {
        let config = ShardConfig {
            tick_rate_hz: 20,
            max_catch_up_ticks: 5,
            idle_ticks_before_shutdown: 0,
            ..Default::default()
        };
        let interval = Duration::from_millis(50);

        // Two ticks overdue, plus the one due now: all three run.
        let shard = Shard::new("a", Noop, config.clone());
        let mut next = Instant::now() - interval * 2;
        catch_up(&shard, &mut next, interval);
        assert_eq!(shard.tick_number(), 3);
        assert_eq!(shard.overrun_ticks(), 0);
        assert!(next > Instant::now());

        // Ten overdue: five run, the rest are skipped and counted.
        let shard = Shard::new("b", Noop, config);
        let mut next = Instant::now() - interval * 10;
        catch_up(&shard, &mut next, interval);
        assert_eq!(shard.tick_number(), 5);
        assert!(shard.overrun_ticks() >= 5, "{}", shard.overrun_ticks());
        assert!(next > Instant::now());
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
        // On a slow machine the last input may not have ticked yet: stop
        // only once every input is applied (and so recorded).
        let deadline = Instant::now() + Duration::from_secs(10);
        while log.len() < 5 {
            assert!(
                Instant::now() < deadline,
                "only {} inputs applied",
                log.len()
            );
            std::thread::sleep(Duration::from_millis(5));
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

    /// Counts the messages and call results it gets.
    #[derive(Default)]
    struct Mailbox {
        messages: u64,
        results: u64,
    }
    impl SimState for Mailbox {
        type Input = ();
        type Snapshot = u64;
        type Error = String;
        fn apply_input(&mut self, _: &SubscriberId, _: (), _: Instant) -> Result<(), String> {
            Ok(())
        }
        fn tick(&mut self, _dt: Duration) {}
        fn snapshot(&self) -> u64 {
            self.messages
        }
        fn on_message(&mut self, _message: &crate::shard::ShardMessage) {
            self.messages += 1;
        }
        fn on_call_result(&mut self, _result: &crate::shard::CallResult) {
            self.results += 1;
        }
    }

    /// An event-driven shard with no inputs still ticks for messages (more
    /// than one tick's worth) and call results.
    #[test]
    fn an_event_driven_shard_wakes_for_messages_and_call_results() {
        let shard = Arc::new(Shard::new(
            "turns",
            Mailbox::default(),
            ShardConfig {
                tick_rate_hz: 0,
                ..Default::default()
            },
        ));
        for n in 0..70u8 {
            assert!(shard.push_message(crate::shard::ShardMessage {
                from: String::new(),
                topic: "t".into(),
                data: Arc::from(vec![n]),
            }));
        }
        assert!(shard.push_call_result(crate::shard::CallResult {
            key: "k".into(),
            ok: true,
            data: Arc::from(b"1".to_vec()),
        }));
        let _loop = TickLoop::spawn(Arc::clone(&shard));
        let deadline = Instant::now() + Duration::from_secs(5);
        while shard.with_state(|m| (m.messages, m.results)) != (70, 1) {
            assert!(
                Instant::now() < deadline,
                "{:?}",
                shard.with_state(|m| (m.messages, m.results))
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        shard.stop();
    }
}
