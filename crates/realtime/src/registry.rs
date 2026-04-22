//! Registry of active shards, keyed by shard ID.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::shard::{Shard, SimState};
use crate::tick::TickLoop;

/// Tracks all live shards keyed by ID.
///
/// Used by matchmaking / lobby / auction routes to look up or create a
/// shard for a given match, room, lot, or document.
pub struct ShardRegistry<S: SimState> {
    shards: RwLock<HashMap<String, Entry<S>>>,
}

struct Entry<S: SimState> {
    shard: Arc<Shard<S>>,
    // TickLoop is kept alive here; when the entry is removed, the loop exits.
    _tick_loop: TickLoop,
}

impl<S: SimState> ShardRegistry<S> {
    pub fn new() -> Self {
        Self {
            shards: RwLock::new(HashMap::new()),
        }
    }

    /// Insert a new shard and spawn its tick loop.
    ///
    /// Replaces any existing shard with the same ID (which stops its tick loop).
    pub fn insert(&self, shard: Arc<Shard<S>>) {
        let id = shard.id().to_string();
        let tick_loop = TickLoop::spawn(Arc::clone(&shard));
        let entry = Entry {
            shard,
            _tick_loop: tick_loop,
        };
        self.shards.write().unwrap().insert(id, entry);
    }

    /// Look up an existing shard.
    pub fn get(&self, id: &str) -> Option<Arc<Shard<S>>> {
        self.shards
            .read()
            .unwrap()
            .get(id)
            .map(|e| Arc::clone(&e.shard))
    }

    /// Get an existing shard or create a new one by invoking `factory`.
    pub fn get_or_create(
        &self,
        id: &str,
        factory: impl FnOnce() -> Arc<Shard<S>>,
    ) -> Arc<Shard<S>> {
        // Fast path: already present.
        if let Some(existing) = self.shards.read().unwrap().get(id) {
            return Arc::clone(&existing.shard);
        }
        // Slow path: upgrade to write lock and double-check.
        let mut map = self.shards.write().unwrap();
        if let Some(existing) = map.get(id) {
            return Arc::clone(&existing.shard);
        }
        let shard = factory();
        let tick_loop = TickLoop::spawn(Arc::clone(&shard));
        let result = Arc::clone(&shard);
        map.insert(
            id.to_string(),
            Entry {
                shard,
                _tick_loop: tick_loop,
            },
        );
        result
    }

    /// Remove a shard (stops its tick loop on drop).
    pub fn remove(&self, id: &str) -> bool {
        let mut map = self.shards.write().unwrap();
        if let Some(entry) = map.remove(id) {
            entry.shard.stop();
            true
        } else {
            false
        }
    }

    /// List shard IDs.
    pub fn ids(&self) -> Vec<String> {
        self.shards.read().unwrap().keys().cloned().collect()
    }

    /// Number of live shards.
    pub fn len(&self) -> usize {
        self.shards.read().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Sweep: remove shards that are no longer running (finished or idle-shutdown).
    /// Call periodically from a scheduler.
    pub fn sweep_finished(&self) -> usize {
        let mut map = self.shards.write().unwrap();
        let dead: Vec<String> = map
            .iter()
            .filter_map(|(id, e)| {
                if e.shard.is_running() {
                    None
                } else {
                    Some(id.clone())
                }
            })
            .collect();
        let count = dead.len();
        for id in dead {
            map.remove(&id);
        }
        count
    }
}

impl<S: SimState> Default for ShardRegistry<S> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shard::{Shard, ShardConfig, SimState};
    use crate::subscriber::SubscriberId;
    use std::time::{Duration, Instant};

    struct Counter {
        value: u64,
    }

    impl SimState for Counter {
        type Input = i64;
        type Snapshot = u64;
        type Error = String;
        fn apply_input(
            &mut self,
            _sub: &SubscriberId,
            input: Self::Input,
            _now: Instant,
        ) -> Result<(), Self::Error> {
            if input >= 0 {
                self.value += input as u64;
            }
            Ok(())
        }
        fn tick(&mut self, _dt: Duration) {}
        fn snapshot(&self) -> Self::Snapshot {
            self.value
        }
    }

    #[test]
    fn get_or_create_is_idempotent() {
        let reg: ShardRegistry<Counter> = ShardRegistry::new();
        let a = reg.get_or_create("match1", || {
            Shard::new(
                "match1",
                Counter { value: 0 },
                ShardConfig {
                    tick_rate_hz: 20,
                    ..Default::default()
                },
            )
        });
        let b = reg.get_or_create("match1", || {
            Shard::new(
                "match1",
                Counter { value: 99 },
                ShardConfig::default(),
            )
        });
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!(reg.len(), 1);
        reg.remove("match1");
    }

    #[test]
    fn remove_stops_shard() {
        let reg: ShardRegistry<Counter> = ShardRegistry::new();
        let shard = Shard::new(
            "x",
            Counter { value: 0 },
            ShardConfig {
                tick_rate_hz: 20,
                ..Default::default()
            },
        );
        reg.insert(Arc::clone(&shard));
        assert!(shard.is_running());
        reg.remove("x");
        assert!(!shard.is_running());
    }
}
