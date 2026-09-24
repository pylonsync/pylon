//! Shard replay — record all inputs applied to a shard and later reconstruct
//! match state by replaying them from the initial state.
//!
//! How this works:
//! - [`ReplayLog<I>`] is a bounded in-memory buffer of `(tick, subscriber_id, input)` triples.
//! - [`ReplayLog::attach`] makes a shard record every input with the tick
//!   it is applied on.
//! - [`replay_to`] re-runs the recorded inputs against a fresh state in the
//!   same order as the live shard: on tick `n`, apply tick `n`'s inputs,
//!   then advance time by `dt`.
//!
//! A replay reproduces the live run when the shard uses `fixed_timestep`
//! (every tick gets the same `dt`) and the simulation is deterministic: no
//! wall-clock reads (including `apply_input`'s `now` argument) and no
//! unseeded randomness. Seed RNGs from the tick number.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::shard::{Shard, SimState};
use crate::subscriber::SubscriberId;

// ---------------------------------------------------------------------------
// ReplayLog
// ---------------------------------------------------------------------------

/// Bounded in-memory log of inputs for replay.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayEntry<I> {
    pub tick: u64,
    pub subscriber_id: String,
    pub input: I,
}

pub struct ReplayLog<I> {
    entries: Mutex<Vec<ReplayEntry<I>>>,
    capacity: usize,
}

impl<I: Clone> ReplayLog<I> {
    pub fn new(capacity: usize) -> Arc<Self> {
        Arc::new(Self {
            entries: Mutex::new(Vec::new()),
            capacity,
        })
    }

    pub fn record(&self, tick: u64, subscriber_id: &SubscriberId, input: I) {
        let mut es = self.entries.lock().unwrap();
        if self.capacity > 0 && es.len() >= self.capacity {
            es.remove(0);
        }
        es.push(ReplayEntry {
            tick,
            subscriber_id: subscriber_id.as_str().to_string(),
            input,
        });
    }

    pub fn len(&self) -> usize {
        self.entries.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn entries(&self) -> Vec<ReplayEntry<I>> {
        self.entries.lock().unwrap().clone()
    }

    pub fn clear(&self) {
        self.entries.lock().unwrap().clear();
    }
}

impl<I: Clone + Send + Sync + 'static> ReplayLog<I> {
    /// Record every input `shard` applies, with its tick number.
    pub fn attach<S>(self: &Arc<Self>, shard: &Shard<S>)
    where
        S: SimState<Input = I>,
    {
        let log = Arc::clone(self);
        shard.set_input_recorder(move |tick, id, input| log.record(tick, id, input.clone()));
    }
}

// ---------------------------------------------------------------------------
// Replay helper
// ---------------------------------------------------------------------------

/// Replay recorded inputs against a fresh initial state through tick
/// `final_tick`, in the live shard's order: for each tick `n` from 1, apply
/// the inputs recorded for tick `n`, then call `state.tick(dt_per_tick)`.
/// Pass the shard's [`Shard::fixed_dt`] as `dt_per_tick` and its
/// [`Shard::tick_number`] as `final_tick`.
///
/// An `apply_input` error is not fatal, as in the live shard (the input is
/// rejected and the tick goes on).
pub fn replay_to<S: SimState>(
    initial: S,
    entries: &[ReplayEntry<S::Input>],
    dt_per_tick: Duration,
    final_tick: u64,
) -> S
where
    S::Input: Clone,
{
    let mut state = initial;
    let now = Instant::now();
    let mut next = 0;
    for tick in 1..=final_tick {
        while next < entries.len() && entries[next].tick <= tick {
            let entry = &entries[next];
            let sid = SubscriberId::new(entry.subscriber_id.clone());
            let _ = state.apply_input(&sid, entry.input.clone(), now);
            next += 1;
        }
        state.tick(dt_per_tick);
    }
    state
}

/// [`replay_to`] through the tick of the last entry.
pub fn replay<S: SimState>(
    initial: S,
    entries: &[ReplayEntry<S::Input>],
    dt_per_tick: Duration,
) -> S
where
    S::Input: Clone,
{
    let last = entries.last().map(|e| e.tick).unwrap_or(0);
    replay_to(initial, entries, dt_per_tick, last)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
    struct Counter {
        value: i64,
    }

    impl SimState for Counter {
        type Input = i64;
        type Snapshot = i64;
        type Error = String;
        fn apply_input(
            &mut self,
            _s: &SubscriberId,
            input: Self::Input,
            _now: Instant,
        ) -> Result<(), Self::Error> {
            self.value += input;
            Ok(())
        }
        fn tick(&mut self, _dt: Duration) {}
        fn snapshot(&self) -> Self::Snapshot {
            self.value
        }
    }

    #[test]
    fn log_records_and_returns_entries() {
        let log: Arc<ReplayLog<i64>> = ReplayLog::new(100);
        log.record(1, &SubscriberId::new("a"), 5);
        log.record(2, &SubscriberId::new("b"), 3);
        assert_eq!(log.len(), 2);
        let entries = log.entries();
        assert_eq!(entries[0].input, 5);
        assert_eq!(entries[1].subscriber_id, "b");
    }

    #[test]
    fn log_respects_capacity() {
        let log: Arc<ReplayLog<i64>> = ReplayLog::new(2);
        log.record(1, &SubscriberId::new("a"), 1);
        log.record(2, &SubscriberId::new("a"), 2);
        log.record(3, &SubscriberId::new("a"), 3);
        assert_eq!(log.len(), 2);
        let entries = log.entries();
        assert_eq!(entries[0].input, 2); // oldest (tick 1) evicted
        assert_eq!(entries[1].input, 3);
    }

    #[test]
    fn replay_reconstructs_state() {
        let entries = vec![
            ReplayEntry {
                tick: 1,
                subscriber_id: "p".to_string(),
                input: 10i64,
            },
            ReplayEntry {
                tick: 2,
                subscriber_id: "p".to_string(),
                input: 5i64,
            },
            ReplayEntry {
                tick: 5,
                subscriber_id: "p".to_string(),
                input: -3i64,
            },
        ];
        let replayed = replay(Counter { value: 0 }, &entries, Duration::from_millis(50));
        assert_eq!(replayed.value, 12);
    }

    #[test]
    fn replay_serializes_entries() {
        let e = ReplayEntry {
            tick: 42,
            subscriber_id: "p".to_string(),
            input: 7i64,
        };
        let s = serde_json::to_string(&e).unwrap();
        assert!(s.contains("\"tick\":42"));
        let e2: ReplayEntry<i64> = serde_json::from_str(&s).unwrap();
        assert_eq!(e2.input, 7);
    }
}
