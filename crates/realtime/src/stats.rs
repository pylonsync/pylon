//! Per-shard numbers for operators: how long ticks take and where the time
//! goes, how many bytes go out, and what is dropped.
//!
//! The shard records one [`TickSample`] per tick. Percentiles come from the
//! last [`WINDOW_TICKS`] ticks (a minute at 20 Hz). Bytes per subscriber
//! come from up to [`SUBSCRIBER_SAMPLES_PER_TICK`] subscribers a tick over
//! the same ticks, taken in turn so every subscriber is sampled. Totals
//! count from the shard's start.

use std::collections::VecDeque;
use std::time::Duration;

use serde::Serialize;

/// Ticks the percentiles cover.
pub const WINDOW_TICKS: usize = 1200;
/// Subscribers sampled per tick for the per-subscriber byte percentiles.
pub const SUBSCRIBER_SAMPLES_PER_TICK: usize = 4;

/// The parts of a tick, timed separately.
#[derive(Debug, Clone, Copy, Default)]
pub struct Phases {
    /// Applying the queued inputs.
    pub inputs: Duration,
    /// The simulation's `tick` (and the `on_tick` hook).
    pub tick: Duration,
    /// Interest management and building each subscriber's snapshot or
    /// replication frame.
    pub interest: Duration,
    /// Encoding and queueing frames for subscribers.
    pub encode: Duration,
}

impl Phases {
    pub fn total(&self) -> Duration {
        self.inputs + self.tick + self.interest + self.encode
    }
}

/// What one tick did.
#[derive(Debug, Clone, Default)]
pub struct TickSample {
    /// The whole tick, from the start of `run_tick` to the end: the phases
    /// plus waiting for locks, draining inputs, and cleanup.
    pub whole: Duration,
    pub phases: Phases,
    /// Frame bytes queued for each subscription this tick.
    pub subscriber_bytes: Vec<u64>,
    /// Frames the subscribers' outbound queues dropped this tick.
    pub dropped_frames: u64,
}

/// Percentiles of a window.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct Quantiles {
    pub p50: f64,
    pub p99: f64,
    pub max: f64,
}

impl Quantiles {
    fn of(values: impl Iterator<Item = f64>) -> Self {
        let mut v: Vec<f64> = values.collect();
        if v.is_empty() {
            return Self::default();
        }
        v.sort_by(|a, b| a.total_cmp(b));
        let at = |q: f64| v[((v.len() - 1) as f64 * q).round() as usize];
        Self {
            p50: at(0.5),
            p99: at(0.99),
            max: v[v.len() - 1],
        }
    }
}

/// Seconds spent in whole ticks and in each phase since the shard started.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct PhaseSeconds {
    pub whole: f64,
    pub inputs: f64,
    pub tick: f64,
    pub interest: f64,
    pub encode: f64,
}

/// Inputs refused before they were queued, by reason.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct DroppedInputs {
    /// The shard's input queue was full.
    pub queue_full: u64,
    /// The subscriber sent faster than its rate limit.
    pub rate_limited: u64,
}

impl DroppedInputs {
    pub fn total(&self) -> u64 {
        self.queue_full + self.rate_limited
    }
}

/// A shard's numbers at one moment.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ShardStats {
    pub ticks: u64,
    /// Ticks the loop could not run on time and skipped.
    pub overruns: u64,
    pub subscribers: usize,
    pub input_queue: usize,
    /// Whole-tick duration, in milliseconds, over the window.
    pub tick_ms: Quantiles,
    /// Each phase's duration, in milliseconds, over the window.
    pub inputs_ms: Quantiles,
    pub sim_ms: Quantiles,
    pub interest_ms: Quantiles,
    pub encode_ms: Quantiles,
    pub phase_seconds_total: PhaseSeconds,
    /// Frame bytes queued per tick, for all subscribers together.
    pub bytes_per_tick: Quantiles,
    /// Frame bytes queued per subscriber per tick (sampled; see the module
    /// docs).
    pub bytes_per_subscriber: Quantiles,
    pub bytes_total: u64,
    pub dropped_frames_total: u64,
    pub dropped_inputs_total: DroppedInputs,
}

#[derive(Debug, Clone, Copy)]
struct WindowTick {
    whole: Duration,
    phases: Phases,
    bytes: u64,
}

/// Records tick samples; owned by the shard behind a lock taken once per
/// tick. Clone it under the lock and call `snapshot` on the clone, so the
/// percentiles are computed without holding the lock.
#[derive(Debug, Default, Clone)]
pub struct StatsRecorder {
    ticks: u64,
    window: VecDeque<WindowTick>,
    /// Per-subscriber byte samples, one group per tick in the window.
    subscriber_bytes: VecDeque<Vec<u64>>,
    /// Where the next tick's subscriber sample starts.
    sample_offset: usize,
    whole_total: Duration,
    phase_totals: Phases,
    bytes_total: u64,
    dropped_frames_total: u64,
    dropped_inputs: DroppedInputs,
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

impl StatsRecorder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&mut self, sample: TickSample) {
        self.ticks += 1;
        self.whole_total += sample.whole;
        let bytes: u64 = sample.subscriber_bytes.iter().sum();
        self.bytes_total += bytes;
        self.dropped_frames_total += sample.dropped_frames;
        let p = &mut self.phase_totals;
        p.inputs += sample.phases.inputs;
        p.tick += sample.phases.tick;
        p.interest += sample.phases.interest;
        p.encode += sample.phases.encode;
        self.window.push_back(WindowTick {
            whole: sample.whole,
            phases: sample.phases,
            bytes,
        });
        while self.window.len() > WINDOW_TICKS {
            self.window.pop_front();
        }
        let all = sample.subscriber_bytes;
        let picked = if all.len() <= SUBSCRIBER_SAMPLES_PER_TICK {
            all
        } else {
            // In turn: consecutive ticks sample different subscribers.
            let n = all.len();
            let start = self.sample_offset % n;
            self.sample_offset = self.sample_offset.wrapping_add(SUBSCRIBER_SAMPLES_PER_TICK);
            (0..SUBSCRIBER_SAMPLES_PER_TICK)
                .map(|i| all[(start + i) % n])
                .collect()
        };
        self.subscriber_bytes.push_back(picked);
        while self.subscriber_bytes.len() > WINDOW_TICKS {
            self.subscriber_bytes.pop_front();
        }
    }

    pub fn input_queue_full(&mut self) {
        self.dropped_inputs.queue_full += 1;
    }

    pub fn input_rate_limited(&mut self) {
        self.dropped_inputs.rate_limited += 1;
    }

    /// The numbers so far. The caller fills in what the recorder does not
    /// see (overruns, subscribers, input queue).
    pub fn snapshot(&self) -> ShardStats {
        let w = &self.window;
        let t = &self.phase_totals;
        ShardStats {
            ticks: self.ticks,
            tick_ms: Quantiles::of(w.iter().map(|s| ms(s.whole))),
            inputs_ms: Quantiles::of(w.iter().map(|s| ms(s.phases.inputs))),
            sim_ms: Quantiles::of(w.iter().map(|s| ms(s.phases.tick))),
            interest_ms: Quantiles::of(w.iter().map(|s| ms(s.phases.interest))),
            encode_ms: Quantiles::of(w.iter().map(|s| ms(s.phases.encode))),
            phase_seconds_total: PhaseSeconds {
                whole: self.whole_total.as_secs_f64(),
                inputs: t.inputs.as_secs_f64(),
                tick: t.tick.as_secs_f64(),
                interest: t.interest.as_secs_f64(),
                encode: t.encode.as_secs_f64(),
            },
            bytes_per_tick: Quantiles::of(w.iter().map(|s| s.bytes as f64)),
            bytes_per_subscriber: Quantiles::of(
                self.subscriber_bytes.iter().flatten().map(|&b| b as f64),
            ),
            bytes_total: self.bytes_total,
            dropped_frames_total: self.dropped_frames_total,
            dropped_inputs_total: self.dropped_inputs,
            ..ShardStats::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(ms_each: u64, bytes: &[u64], dropped: u64) -> TickSample {
        let d = Duration::from_millis(ms_each);
        TickSample {
            whole: d * 5,
            phases: Phases {
                inputs: d,
                tick: d,
                interest: d,
                encode: d,
            },
            subscriber_bytes: bytes.to_vec(),
            dropped_frames: dropped,
        }
    }

    #[test]
    fn percentiles_totals_and_windows() {
        let mut r = StatsRecorder::new();
        for i in 1..=100u64 {
            r.record(sample(i % 10, &[i, 2 * i], u64::from(i == 50)));
        }
        r.input_queue_full();
        r.input_rate_limited();
        r.input_rate_limited();
        let s = r.snapshot();
        assert_eq!(s.ticks, 100);
        // Phases of 0..9 ms each; the whole tick is longer than the four.
        assert_eq!(s.tick_ms.max, 45.0);
        assert_eq!(s.sim_ms.p50, 5.0);
        assert_eq!(s.bytes_total, (1..=100u64).map(|i| 3 * i).sum::<u64>());
        assert_eq!(s.bytes_per_tick.max, 300.0);
        assert_eq!(s.bytes_per_subscriber.max, 200.0);
        assert_eq!(s.dropped_frames_total, 1);
        assert_eq!(s.dropped_inputs_total.total(), 3);
        assert!((s.phase_seconds_total.tick - 0.45).abs() < 1e-9);

        // The window forgets old ticks; the totals do not.
        for _ in 0..WINDOW_TICKS {
            r.record(sample(1, &[7], 0));
        }
        let s = r.snapshot();
        assert_eq!(s.tick_ms.max, 5.0);
        assert_eq!(s.bytes_per_subscriber.max, 7.0);
        assert_eq!(s.dropped_frames_total, 1);
        assert_eq!(s.ticks, 100 + WINDOW_TICKS as u64);
    }

    #[test]
    fn many_subscribers_are_sampled_in_turn_over_the_whole_window() {
        let mut r = StatsRecorder::new();
        // 1000 subscribers; subscriber i sends i bytes every tick.
        let bytes: Vec<u64> = (0..1000).collect();
        for _ in 0..WINDOW_TICKS {
            r.record(sample(1, &bytes, 0));
        }
        let s = r.snapshot();
        // 4800 samples cover all 1000 subscribers 4.8 times (4 a tick, in
        // turn); the partial last pass weights some a little more, so the
        // percentiles are within a few percent of the true 500 and 990.
        let q = s.bytes_per_subscriber;
        assert!((q.p50 - 500.0).abs() <= 30.0, "{q:?}");
        assert!((q.p99 - 990.0).abs() <= 10.0, "{q:?}");
        assert_eq!(s.bytes_per_subscriber.max, 999.0);
        assert_eq!(s.bytes_per_tick.p50, (0..1000u64).sum::<u64>() as f64);
    }

    #[test]
    fn an_empty_window_is_zero() {
        let s = StatsRecorder::new().snapshot();
        assert_eq!(s.tick_ms, Quantiles::default());
        assert_eq!(s.bytes_per_subscriber, Quantiles::default());
    }
}
