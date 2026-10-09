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
    /// Interest management, and each subscriber's snapshot for a shard
    /// that does not replicate.
    pub interest: Duration,
    /// Building each subscriber's replication frame, for a shard that
    /// replicates.
    pub frames: Duration,
    /// Encoding and queueing frames for subscribers.
    pub encode: Duration,
}

impl Phases {
    pub fn total(&self) -> Duration {
        self.inputs + self.tick + self.interest + self.frames + self.encode
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
        let p50_index = ((v.len() - 1) as f64 * 0.5).round() as usize;
        let p99_index = ((v.len() - 1) as f64 * 0.99).round() as usize;
        if v.is_sorted_by(|a, b| a.total_cmp(b).is_le()) {
            return Self {
                p50: v[p50_index],
                p99: v[p99_index],
                max: v[v.len() - 1],
            };
        }
        if v.is_sorted_by(|a, b| a.total_cmp(b).is_ge()) {
            return Self {
                p50: v[v.len() - 1 - p50_index],
                p99: v[v.len() - 1 - p99_index],
                max: v[0],
            };
        }
        let (lower, p99, upper) = v.select_nth_unstable_by(p99_index, f64::total_cmp);
        let p99 = *p99;
        let max = upper.iter().copied().max_by(f64::total_cmp).unwrap_or(p99);
        let p50 = if p50_index == p99_index {
            p99
        } else {
            *lower.select_nth_unstable_by(p50_index, f64::total_cmp).1
        };
        Self { p50, p99, max }
    }
}

/// Seconds spent in whole ticks and in each phase since the shard started.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct PhaseSeconds {
    pub whole: f64,
    pub inputs: f64,
    pub tick: f64,
    pub interest: f64,
    pub frames: f64,
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
    pub frames_ms: Quantiles,
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

#[derive(Debug, Clone, Copy)]
struct SubscriberSamples {
    bytes: [u64; SUBSCRIBER_SAMPLES_PER_TICK],
    len: usize,
}

/// Records tick samples; owned by the shard behind a lock taken once per
/// tick. Clone it under the lock and call `snapshot` on the clone, so the
/// percentiles are computed without holding the lock.
#[derive(Debug, Default, Clone)]
pub struct StatsRecorder {
    ticks: u64,
    window: VecDeque<WindowTick>,
    /// Per-subscriber byte samples, one group per tick in the window.
    subscriber_bytes: VecDeque<SubscriberSamples>,
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
        p.frames += sample.phases.frames;
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
        let mut picked = SubscriberSamples {
            bytes: [0; SUBSCRIBER_SAMPLES_PER_TICK],
            len: all.len().min(SUBSCRIBER_SAMPLES_PER_TICK),
        };
        if all.len() <= SUBSCRIBER_SAMPLES_PER_TICK {
            picked.bytes[..picked.len].copy_from_slice(&all);
        } else {
            // In turn: consecutive ticks sample different subscribers.
            let n = all.len();
            let start = self.sample_offset % n;
            self.sample_offset = self.sample_offset.wrapping_add(SUBSCRIBER_SAMPLES_PER_TICK);
            for (i, value) in picked.bytes.iter_mut().enumerate() {
                *value = all[(start + i) % n];
            }
        }
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
            frames_ms: Quantiles::of(w.iter().map(|s| ms(s.phases.frames))),
            encode_ms: Quantiles::of(w.iter().map(|s| ms(s.phases.encode))),
            phase_seconds_total: PhaseSeconds {
                whole: self.whole_total.as_secs_f64(),
                inputs: t.inputs.as_secs_f64(),
                tick: t.tick.as_secs_f64(),
                interest: t.interest.as_secs_f64(),
                frames: t.frames.as_secs_f64(),
                encode: t.encode.as_secs_f64(),
            },
            bytes_per_tick: Quantiles::of(w.iter().map(|s| s.bytes as f64)),
            bytes_per_subscriber: Quantiles::of(
                self.subscriber_bytes
                    .iter()
                    .flat_map(|sample| &sample.bytes[..sample.len])
                    .map(|&b| b as f64),
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

    fn sorted_quantiles(values: &[f64]) -> Quantiles {
        let mut values = values.to_vec();
        if values.is_empty() {
            return Quantiles::default();
        }
        values.sort_by(f64::total_cmp);
        let at = |q: f64| values[((values.len() - 1) as f64 * q).round() as usize];
        Quantiles {
            p50: at(0.5),
            p99: at(0.99),
            max: values[values.len() - 1],
        }
    }

    #[test]
    fn selected_quantiles_match_sorting_at_boundaries_and_with_ties() {
        for n in (0..200).chain([WINDOW_TICKS, WINDOW_TICKS * SUBSCRIBER_SAMPLES_PER_TICK]) {
            let mut values: Vec<_> = (0..n).map(|i| ((i * 31337) % 97) as f64).collect();
            for shape in 0..6 {
                match shape {
                    1 | 3 => values.reverse(),
                    2 => values.sort_by(f64::total_cmp),
                    4 => values.fill(1.0),
                    5 => {
                        if let Some(first) = values.first_mut() {
                            *first = -1.0;
                        }
                    }
                    _ => {}
                }
                let got = Quantiles::of(values.iter().copied());
                let expected = sorted_quantiles(&values);
                assert_eq!(got, expected, "length {n}");
            }
        }
        let special = [f64::NAN, f64::NEG_INFINITY, -0.0, 0.0, f64::INFINITY, -1.0];
        let got = Quantiles::of(special.into_iter());
        let expected = sorted_quantiles(&special);
        assert_eq!(
            [got.p50, got.p99, got.max].map(f64::to_bits),
            [expected.p50, expected.p99, expected.max].map(f64::to_bits)
        );
    }

    #[test]
    #[ignore = "release-mode performance benchmark"]
    fn benchmark_quantiles() {
        use std::hint::black_box;
        for n in [
            100,
            WINDOW_TICKS,
            WINDOW_TICKS * SUBSCRIBER_SAMPLES_PER_TICK,
        ] {
            for shape in ["random", "sorted", "reverse", "equal"] {
                let values: Vec<f64> = (0..n)
                    .map(|i| match shape {
                        "sorted" => i as f64,
                        "reverse" => (n - i) as f64,
                        "equal" => 1.0,
                        _ => ((i * 31337) % 997) as f64,
                    })
                    .collect();
                for select in [false, true] {
                    let mut samples = Vec::new();
                    for _ in 0..7 {
                        let start = std::time::Instant::now();
                        for _ in 0..100 {
                            black_box(if select {
                                Quantiles::of(black_box(&values).iter().copied())
                            } else {
                                sorted_quantiles(black_box(&values))
                            });
                        }
                        samples.push(start.elapsed().as_secs_f64() * 1e6 / 100.0);
                    }
                    samples.sort_by(f64::total_cmp);
                    println!(
                        "{n} {shape} quantiles, select={select}: {:.3} us median",
                        samples[3]
                    );
                }
            }
        }
    }

    fn sample(ms_each: u64, bytes: &[u64], dropped: u64) -> TickSample {
        let d = Duration::from_millis(ms_each);
        TickSample {
            whole: d * 6,
            phases: Phases {
                inputs: d,
                tick: d,
                interest: d,
                frames: d,
                encode: d,
            },
            subscriber_bytes: bytes.to_vec(),
            dropped_frames: dropped,
        }
    }

    #[test]
    fn subscriber_samples_preserve_empty_short_and_rotating_groups() {
        let mut recorder = StatsRecorder::new();
        let groups: &[&[u64]] = &[
            &[],
            &[17],
            &[3, 7],
            &[10, 11, 12, 13],
            &[100, 101, 102, 103, 104],
        ];
        for group in groups {
            recorder.record(sample(1, group, 0));
        }
        let snapshot = recorder.snapshot();
        assert_eq!(
            snapshot.bytes_total,
            groups.iter().flat_map(|group| group.iter()).sum::<u64>()
        );
        assert_eq!(
            snapshot.bytes_per_subscriber,
            sorted_quantiles(&[17.0, 3.0, 7.0, 10.0, 11.0, 12.0, 13.0, 100.0, 101.0, 102.0, 103.0])
        );
        recorder.record(sample(1, groups[4], 0));
        let last = recorder.subscriber_bytes.back().unwrap();
        assert_eq!(last.bytes, [104, 100, 101, 102]);
        assert_eq!(recorder.clone().snapshot(), recorder.snapshot());
    }

    #[test]
    #[ignore = "release-mode performance benchmark"]
    fn benchmark_snapshot_clone() {
        use std::hint::black_box;
        let mut recorder = StatsRecorder::new();
        for _ in 0..WINDOW_TICKS {
            recorder.record(sample(1, &[1, 2, 3, 4], 0));
        }
        let nested: VecDeque<Vec<u64>> = recorder
            .subscriber_bytes
            .iter()
            .map(|sample| sample.bytes[..sample.len].to_vec())
            .collect();
        for flat in [false, true] {
            let mut samples = Vec::new();
            for _ in 0..7 {
                let start = std::time::Instant::now();
                for _ in 0..100 {
                    if flat {
                        black_box(recorder.clone());
                    } else {
                        black_box((recorder.window.clone(), nested.clone()));
                    }
                }
                samples.push(start.elapsed().as_secs_f64() * 1e6 / 100.0);
            }
            samples.sort_by(f64::total_cmp);
            println!(
                "1200-tick snapshot clone, flat={flat}: {:.3} us median",
                samples[3]
            );
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
        // Phases of 0..9 ms each; the whole tick is the five together.
        assert_eq!(s.tick_ms.max, 54.0);
        assert_eq!(s.frames_ms.max, 9.0);
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
        assert_eq!(s.tick_ms.max, 6.0);
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
