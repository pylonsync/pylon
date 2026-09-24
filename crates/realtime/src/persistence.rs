//! Helpers for persisting shard state and restoring it on startup.
//!
//! Two primitives:
//!
//! - [`persist_every_ticks`] — every N ticks, copy the data to save while
//!   the tick holds the state lock, and write it from a separate thread.
//! - [`restore_or_init`] — look up or create a shard in a registry, using
//!   a restore function that loads saved state from wherever the caller put it.
//!
//! The persistence *backend* is left to the caller — pylon's DataStore,
//! a file, Redis, whatever — because the state type `S` is user-defined
//! and the right persistence strategy varies.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use crate::registry::ShardRegistry;
use crate::shard::{Shard, ShardConfig, SimState};

/// Save a shard's state every `every_n` ticks without stopping the shard
/// for the write.
///
/// - `capture` runs on the tick thread while the state lock is held. It
///   must only copy what you want to save (clone a struct, serialize to
///   bytes) and return it. No I/O.
/// - `write` runs on a dedicated writer thread with the shard id, the
///   captured record, and its tick number. Do the database or file write
///   here. It may take as long as it needs.
///
/// While a write runs, the tick keeps going. A record captured meanwhile
/// waits in a one-record slot; a newer record replaces a waiting one, so
/// the writer always saves the latest state and the tick never blocks.
/// When the shard is dropped, the writer saves the waiting record (if any)
/// and exits.
///
/// Pass `0` for `every_n` to disable; the function then returns `None`.
///
/// # Example
/// ```ignore
/// persist_every_ticks(
///     &shard,
///     60,
///     |state| serde_json::to_value(state).unwrap(),
///     move |id, json, tick| {
///         let _ = store.update("ShardState", id, &serde_json::json!({
///             "data": json,
///             "tick": tick,
///         }));
///     },
/// );
/// ```
pub fn persist_every_ticks<S, R, C, W>(
    shard: &Arc<Shard<S>>,
    every_n: u64,
    capture: C,
    mut write: W,
) -> Option<PersistHandle>
where
    S: SimState,
    R: Send + 'static,
    C: Fn(&S) -> R + Send + Sync + 'static,
    W: FnMut(&str, R, u64) + Send + 'static,
{
    if every_n == 0 {
        return None;
    }
    let mailbox: Arc<Mailbox<R>> = Arc::new(Mailbox {
        slot: Mutex::new(None),
        ready: Condvar::new(),
        closed: AtomicBool::new(false),
        replaced: AtomicU64::new(0),
        written: AtomicU64::new(0),
    });

    let shard_id = shard.id().to_string();
    let reader = Arc::clone(&mailbox);
    std::thread::Builder::new()
        .name(format!("shard-persist-{shard_id}"))
        .spawn(move || loop {
            let next = {
                let mut slot = reader.slot.lock().unwrap();
                loop {
                    if let Some(record) = slot.take() {
                        break Some(record);
                    }
                    if reader.closed.load(Ordering::Acquire) {
                        break None;
                    }
                    slot = reader.ready.wait(slot).unwrap();
                }
            };
            match next {
                Some((record, tick)) => {
                    write(&shard_id, record, tick);
                    reader.written.fetch_add(1, Ordering::Relaxed);
                }
                None => return,
            }
        })
        .expect("failed to spawn shard persistence writer");

    let sender = MailboxSender(Arc::clone(&mailbox));
    shard.set_on_tick(move |state, tick| {
        if tick % every_n == 0 {
            sender.put(capture(state), tick);
        }
    });
    Some(PersistHandle { mailbox })
}

/// Counters for a [`persist_every_ticks`] writer.
pub struct PersistHandle {
    mailbox: Arc<dyn MailboxStats>,
}

impl PersistHandle {
    /// Records written so far.
    pub fn written(&self) -> u64 {
        self.mailbox.written()
    }

    /// Records dropped because a newer one replaced them before the writer
    /// was free.
    pub fn replaced(&self) -> u64 {
        self.mailbox.replaced()
    }
}

struct Mailbox<R> {
    slot: Mutex<Option<(R, u64)>>,
    ready: Condvar,
    closed: AtomicBool,
    replaced: AtomicU64,
    written: AtomicU64,
}

trait MailboxStats: Send + Sync {
    fn written(&self) -> u64;
    fn replaced(&self) -> u64;
}

impl<R: Send> MailboxStats for Mailbox<R> {
    fn written(&self) -> u64 {
        self.written.load(Ordering::Relaxed)
    }
    fn replaced(&self) -> u64 {
        self.replaced.load(Ordering::Relaxed)
    }
}

/// The tick side of the mailbox. Dropping it (with the shard's `on_tick`
/// hook) tells the writer to finish and exit.
struct MailboxSender<R>(Arc<Mailbox<R>>);

impl<R> MailboxSender<R> {
    fn put(&self, record: R, tick: u64) {
        let mut slot = self.0.slot.lock().unwrap();
        if slot.replace((record, tick)).is_some() {
            self.0.replaced.fetch_add(1, Ordering::Relaxed);
        }
        drop(slot);
        self.0.ready.notify_one();
    }
}

impl<R> Drop for MailboxSender<R> {
    fn drop(&mut self) {
        self.0.closed.store(true, Ordering::Release);
        self.0.ready.notify_all();
    }
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

        let (tx, rx) = std::sync::mpsc::channel();
        persist_every_ticks(
            &shard,
            3,
            |state| state.value,
            move |id, value, tick| {
                assert_eq!(id, "t");
                tx.send((value, tick)).unwrap();
            },
        )
        .unwrap();

        // Tick 1, 2: no persist. Tick 3: persist. Tick 4, 5: no. Tick 6: persist.
        for _ in 0..6 {
            shard.run_tick();
            // Let the writer take each record before the next one arrives.
            std::thread::sleep(Duration::from_millis(5));
        }
        let got: Vec<(u64, u64)> = rx.iter().take(2).collect();
        assert_eq!(got, vec![(0, 3), (0, 6)]);
    }

    #[test]
    fn a_slow_write_does_not_hold_up_the_tick() {
        let shard = Shard::new("slow", Counter { value: 0 }, ShardConfig::default());
        let handle = persist_every_ticks(
            &shard,
            1,
            |state| state.value,
            |_id, _value, _tick| std::thread::sleep(Duration::from_millis(500)),
        )
        .unwrap();

        // 20 ticks at the 20 Hz target interval (50 ms).
        let interval = Duration::from_millis(50);
        let mut worst = Duration::ZERO;
        for _ in 0..20 {
            let start = Instant::now();
            shard.run_tick();
            let took = start.elapsed();
            worst = worst.max(took);
            if took < interval {
                std::thread::sleep(interval - took);
            }
        }
        assert!(
            worst < Duration::from_millis(5),
            "a tick took {worst:?} while the writer slept 500 ms per record"
        );
        // The writer fell behind, so waiting records were replaced by newer
        // ones instead of queueing up: each of the 20 records was written,
        // replaced, is being written now, or waits in the slot.
        assert!(handle.replaced() > 0);
        let accounted = handle.written() + handle.replaced();
        assert!(
            (18..=20).contains(&accounted),
            "written {} + replaced {} should cover 20 records",
            handle.written(),
            handle.replaced()
        );
    }

    #[test]
    fn the_writer_saves_the_last_record_when_the_shard_is_dropped() {
        let shard = Shard::new("drop", Counter { value: 0 }, ShardConfig::default());
        let (tx, rx) = std::sync::mpsc::channel();
        persist_every_ticks(
            &shard,
            1,
            |state| state.value,
            move |_id, value, tick| {
                std::thread::sleep(Duration::from_millis(50));
                tx.send((value, tick)).unwrap();
            },
        )
        .unwrap();
        shard.push_input(SubscriberId::new("p"), 7, None).unwrap();
        shard.run_tick();
        // Give the writer time to take tick 1 (it then sleeps 50 ms).
        std::thread::sleep(Duration::from_millis(20));
        shard.run_tick(); // tick 2 waits in the slot
        drop(shard);
        let got: Vec<(u64, u64)> = rx.iter().collect();
        assert_eq!(got, vec![(7, 1), (7, 2)]);
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
