//! Shard replay — record all inputs applied to a shard and later reconstruct
//! match state by replaying them from the initial state.
//!
//! How this works:
//! - [`ReplayLog<I>`] is a bounded in-memory buffer of `(tick, subscriber_id, input)` triples.
//! - `ReplayLog::record()` is wired into `SimState::apply_input` by the user
//!   (or automatically via [`RecordingState`]).
//! - [`replay`] takes an initial state + the recorded inputs and re-applies
//!   them deterministically against a fresh simulation.
//!
//! Determinism is the caller's responsibility — if your `SimState::tick` uses
//! `rand::random()` you won't reproduce the same state. Use seeded RNGs
//! anchored to the tick number.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::shard::SimState;
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

// ---------------------------------------------------------------------------
// Replay helper
// ---------------------------------------------------------------------------

/// Replay a sequence of recorded inputs against a fresh initial state.
///
/// Between entries, calls `state.tick(dt_per_tick)` for each tick that
/// elapsed — this mirrors the live simulation's cadence.
pub fn replay<S: SimState>(
    initial: S,
    entries: &[ReplayEntry<S::Input>],
    dt_per_tick: Duration,
) -> Result<S, String>
where
    S::Input: Clone,
{
    let mut state = initial;
    let now = Instant::now();
    let mut last_tick = 0u64;

    for entry in entries {
        // Tick forward to the entry's tick number.
        while last_tick < entry.tick {
            state.tick(dt_per_tick);
            last_tick += 1;
        }
        let sid = SubscriberId::new(entry.subscriber_id.clone());
        state
            .apply_input(&sid, entry.input.clone(), now)
            .map_err(|e| format!("replay apply_input failed at tick {}: {:?}", entry.tick, e))?;
    }

    Ok(state)
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
        let replayed = replay(Counter { value: 0 }, &entries, Duration::from_millis(50)).unwrap();
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
