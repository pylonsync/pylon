//! Helpers for persisting shard state and restoring it on startup.
//!
//! Two primitives:
//!
//! - [`persist_every_ticks`] — wire a shard's `on_tick` callback to call a
//!   user-provided persistence function every N ticks.
//! - [`restore_or_init`] — look up or create a shard in a registry, using
//!   a restore function that loads saved state from wherever the caller put it.
//!
//! The persistence *backend* is left to the caller — statecraft's DataStore,
//! a file, Redis, whatever — because the state type `S` is user-defined
//! and the right persistence strategy varies.

use std::sync::Arc;

use crate::registry::ShardRegistry;
use crate::shard::{Shard, ShardConfig, SimState};

/// Install an `on_tick` hook that invokes `persist_fn` every `every_n` ticks.
///
/// Pass `0` for `every_n` to disable. The persist function receives the
/// shard ID, the current state (by reference — you decide whether to
/// serialize/clone), and the tick number.
///
/// # Example
/// ```ignore
/// persist_every_ticks(&shard, 60, |id, state, tick| {
///     let json = serde_json::to_value(state)?;
///     store.update("ShardState", id, &serde_json::json!({
///         "data": json,
///         "tick": tick,
///     }))?;
///     Ok(())
/// });
/// ```
pub fn persist_every_ticks<S, F>(shard: &Arc<Shard<S>>, every_n: u64, persist_fn: F)
where
    S: SimState,
    F: Fn(&str, &S, u64) + Send + Sync + 'static,
{
    if every_n == 0 {
        return;
    }
    let shard_id = shard.id().to_string();
    shard.set_on_tick(move |state, tick| {
        if tick % every_n == 0 {
            persist_fn(&shard_id, state, tick);
        }
    });
}

/// Look up an existing shard in a registry, or create one — with support
/// for restoring previously-persisted state.
///
/// If a shard with `shard_id` already exists, return it. Otherwise:
/// 1. Call `restore(shard_id)`.
/// 2. If it returns `Some(state)`, use that state.
/// 3. Otherwise, call `init()` to create fresh state.
/// 4. Wrap in a `Shard` and register it; the tick loop starts automatically.
///
/// # Example
/// ```ignore
/// let shard = restore_or_init(
///     &registry,
///     "match_42",
///     ShardConfig::default(),
///     |id| load_state_from_db(id).ok(),
///     || GameState::default(),
/// );
/// ```
pub fn restore_or_init<S, R, I>(
    registry: &ShardRegistry<S>,
    shard_id: &str,
    config: ShardConfig,
    restore: R,
    init: I,
) -> Arc<Shard<S>>
where
    S: SimState,
    R: FnOnce(&str) -> Option<S>,
    I: FnOnce() -> S,
{
    registry.get_or_create(shard_id, || {
        let state = restore(shard_id).unwrap_or_else(init);
        Shard::new(shard_id, state, config)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subscriber::SubscriberId;
    use std::sync::atomic::{AtomicU64, Ordering};
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
            _s: &SubscriberId,
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
    fn persist_every_ticks_fires_on_interval() {
        let shard = Shard::new("t", Counter { value: 0 }, ShardConfig::default());

        let call_count = Arc::new(AtomicU64::new(0));
        let call_count_clone = Arc::clone(&call_count);
        persist_every_ticks(&shard, 3, move |id, _state, _tick| {
            assert_eq!(id, "t");
            call_count_clone.fetch_add(1, Ordering::Relaxed);
        });

        // Tick 1, 2: no persist. Tick 3: persist. Tick 4, 5: no. Tick 6: persist.
        for _ in 0..6 {
            shard.run_tick();
        }
        assert_eq!(call_count.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn restore_or_init_uses_restored_state() {
        let reg: ShardRegistry<Counter> = ShardRegistry::new();
        let shard = restore_or_init(
            &reg,
            "m1",
            ShardConfig {
                // Use event-driven mode so the auto-spawned tick loop doesn't
                // race the assertions below.
                tick_rate_hz: 0,
                ..Default::default()
            },
            |_id| Some(Counter { value: 999 }),
            || Counter { value: 0 },
        );
        // Snapshot reads the restored state directly.
        assert_eq!(shard.snapshot(), 999);

        // And mutations apply on top of that state, not on top of `init()`.
        shard.push_input(SubscriberId::new("p1"), 1, None).unwrap();
        shard.run_tick();
        assert_eq!(shard.snapshot(), 1000);

        reg.remove("m1");
    }

    #[test]
    fn restore_or_init_falls_back_to_init() {
        let reg: ShardRegistry<Counter> = ShardRegistry::new();
        let shard = restore_or_init(
            &reg,
            "m2",
            ShardConfig {
                tick_rate_hz: 20,
                ..Default::default()
            },
            |_id| None,
            || Counter { value: 42 },
        );
        assert_eq!(shard.id(), "m2");
        reg.remove("m2");
    }
}
