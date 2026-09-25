//! Per-subscriber outbound queues.
//!
//! The tick thread must never wait on a client. A subscriber that delivers
//! through a queue gets its frames pushed here by the tick thread, and a
//! writer (a thread or an async task owned by the transport) takes them off
//! and writes them to the socket. A slow or stalled client only fills its
//! own queue.
//!
//! Drop policy: a snapshot frame is a complete state view (or a delta
//! against the previous frame, see [`crate::Subscriber`]). When the queue is
//! full, the queued snapshot frames are dropped and the new one takes their
//! place, so the client gets the newest state as soon as it catches up.
//! Other frames (input rejections) are never dropped. A queue closes when it fills and the
//! writer then takes no frame for [`OutboundConfig::disconnect_after`], or
//! when it fills with control frames alone; the transport then disconnects
//! the client. A slow writer that still takes frames stays connected and
//! gets the newest snapshots.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// Limits for one subscriber's queue.
#[derive(Debug, Clone, Copy)]
pub struct OutboundConfig {
    /// Frames the queue holds before it drops snapshots. Minimum 1.
    pub max_frames: usize,
    /// Close the queue when it has filled and the writer has taken no frame
    /// for this long.
    pub disconnect_after: Duration,
}

impl Default for OutboundConfig {
    fn default() -> Self {
        Self {
            max_frames: 64,
            disconnect_after: Duration::from_secs(10),
        }
    }
}

/// What a frame carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameKind {
    /// A snapshot (or snapshot delta). Replaced by a newer one when the
    /// queue is full.
    Snapshot,
    /// An [`crate::wire::InputRejection`]. Never dropped.
    InputRejected,
    /// An entity replication frame (see `pylon_replication::frame`).
    /// Dropped like a snapshot when the queue is full; the shard then sends
    /// the next one as a full baseline.
    Replication,
    /// A [`crate::wire::TransferNotice`]: the subscriber moved to another
    /// shard. The last frame on a queue; never dropped.
    Transfer,
}

/// One frame for the transport to write.
#[derive(Debug, Clone)]
pub struct Frame {
    /// The shard tick the frame belongs to.
    pub tick: u64,
    pub kind: FrameKind,
    /// The highest `client_seq` the shard has processed for this
    /// subscriber (0 = none). See [`crate::wire`].
    pub ack: u64,
    /// The encoded payload, without transport framing.
    pub bytes: Arc<[u8]>,
}

/// The result of a push.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushOutcome {
    Queued,
    /// The queue was full: queued snapshots were dropped for this frame.
    Coalesced,
    /// The queue is closed; the frame was discarded.
    Closed,
    /// A replication delta was refused: the queue is full, or dropped
    /// frames since the delta's baseline was chosen. Nothing was queued or
    /// dropped; send a full baseline instead.
    NeedsBaseline,
}

struct State {
    frames: VecDeque<Frame>,
    /// When a push last found the queue full, if the writer has not taken a
    /// frame since.
    full_since: Option<Instant>,
    dropped_snapshots: u64,
}

type Notifier = Box<dyn Fn() + Send + Sync>;

/// A bounded frame queue between the tick thread and one client's writer.
pub struct OutboundQueue {
    config: OutboundConfig,
    state: Mutex<State>,
    ready: Condvar,
    closed: AtomicBool,
    /// Called after every push and on close, for writers that do not block
    /// on [`OutboundQueue::pop_blocking`] (an async task's waker).
    notifier: Mutex<Option<Notifier>>,
}

impl OutboundQueue {
    pub fn new(config: OutboundConfig) -> Arc<Self> {
        Arc::new(Self {
            config: OutboundConfig {
                max_frames: config.max_frames.max(1),
                ..config
            },
            state: Mutex::new(State {
                frames: VecDeque::new(),
                full_since: None,
                dropped_snapshots: 0,
            }),
            ready: Condvar::new(),
            closed: AtomicBool::new(false),
            notifier: Mutex::new(None),
        })
    }

    /// Register a callback that runs after each push and on close. It must
    /// not block: it runs on the tick thread.
    pub fn set_notifier(&self, notify: impl Fn() + Send + Sync + 'static) {
        *self.notifier.lock().unwrap() = Some(Box::new(notify));
    }

    pub fn config(&self) -> OutboundConfig {
        self.config
    }

    pub fn len(&self) -> usize {
        self.state.lock().unwrap().frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// True when the next snapshot push would drop queued snapshots.
    pub fn is_full(&self) -> bool {
        self.len() >= self.config.max_frames
    }

    /// Snapshot frames dropped so far because the queue was full.
    pub fn dropped_snapshots(&self) -> u64 {
        self.state.lock().unwrap().dropped_snapshots
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    /// Close the queue. Writers see `None` once it is drained; pushes are
    /// discarded.
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        self.ready.notify_all();
        self.notify();
    }

    fn notify(&self) {
        if let Some(n) = &*self.notifier.lock().unwrap() {
            n();
        }
    }

    /// Queue a snapshot frame, dropping older snapshots when full.
    pub fn push_snapshot(&self, tick: u64, ack: u64, bytes: Arc<[u8]>) -> PushOutcome {
        self.push(Frame {
            tick,
            kind: FrameKind::Snapshot,
            ack,
            bytes,
        })
    }

    /// Queue a replication frame, dropping older snapshot and replication
    /// frames when full. The caller must make a frame pushed into a full
    /// queue a full baseline: the dropped deltas are gone.
    ///
    /// `delta_of`: `None` for a full baseline, which is always queued
    /// (dropping older frames when full). For a delta, the dropped-frame
    /// count its baseline assumed: the delta is queued only if no frame was
    /// dropped since and the queue has room, checked under the queue lock
    /// so a concurrent push cannot drop the baseline in between.
    pub fn push_replication(
        &self,
        tick: u64,
        ack: u64,
        bytes: Arc<[u8]>,
        delta_of: Option<u64>,
    ) -> PushOutcome {
        if let Some(expected_dropped) = delta_of {
            let st = self.state.lock().unwrap();
            if st.dropped_snapshots != expected_dropped || st.frames.len() >= self.config.max_frames
            {
                return PushOutcome::NeedsBaseline;
            }
            // Room, and nothing dropped: `push` will not coalesce. Keep the
            // lock so no other push fills the queue first.
            return self.push_locked(
                st,
                Frame {
                    tick,
                    kind: FrameKind::Replication,
                    ack,
                    bytes,
                },
            );
        }
        self.push(Frame {
            tick,
            kind: FrameKind::Replication,
            ack,
            bytes,
        })
    }

    /// Queue an input-rejected frame. It is never dropped; a queue that
    /// cannot take one closes.
    pub fn push_rejection(&self, tick: u64, ack: u64, bytes: Arc<[u8]>) -> PushOutcome {
        self.push(Frame {
            tick,
            kind: FrameKind::InputRejected,
            ack,
            bytes,
        })
    }

    /// Queue a transfer frame as the last frame, then close. Snapshots and
    /// replication frames still waiting are dropped: the client leaves this
    /// shard and gets a baseline from the next one.
    pub fn push_transfer_and_close(&self, tick: u64, ack: u64, bytes: Arc<[u8]>) -> PushOutcome {
        let outcome = {
            let mut st = self.state.lock().unwrap();
            if self.is_closed() {
                return PushOutcome::Closed;
            }
            let before = st.frames.len();
            st.frames
                .retain(|f| !matches!(f.kind, FrameKind::Snapshot | FrameKind::Replication));
            st.dropped_snapshots += (before - st.frames.len()) as u64;
            st.frames.push_back(Frame {
                tick,
                kind: FrameKind::Transfer,
                ack,
                bytes,
            });
            PushOutcome::Queued
        };
        self.close();
        outcome
    }

    fn push(&self, frame: Frame) -> PushOutcome {
        let st = self.state.lock().unwrap();
        self.push_locked(st, frame)
    }

    fn push_locked(&self, st: std::sync::MutexGuard<'_, State>, frame: Frame) -> PushOutcome {
        if self.is_closed() {
            return PushOutcome::Closed;
        }
        let outcome = {
            let mut st = st;
            let mut outcome = PushOutcome::Queued;
            let now = Instant::now();
            // `full_since` is set when a push found the queue full, and only
            // a pop (the writer making progress) clears it. Coalescing
            // shrinks the queue without clearing it, so a writer that makes
            // no progress is caught even though the queue is no longer full.
            if st
                .full_since
                .is_some_and(|since| now.duration_since(since) >= self.config.disconnect_after)
            {
                drop(st);
                self.close();
                return PushOutcome::Closed;
            }
            if st.frames.len() >= self.config.max_frames {
                st.full_since.get_or_insert(now);
                let before = st.frames.len();
                st.frames
                    .retain(|f| matches!(f.kind, FrameKind::InputRejected | FrameKind::Transfer));
                st.dropped_snapshots += (before - st.frames.len()) as u64;
                if st.frames.len() >= self.config.max_frames {
                    // Full of frames that cannot be dropped: the client is
                    // not reading at all.
                    drop(st);
                    self.close();
                    return PushOutcome::Closed;
                }
                outcome = PushOutcome::Coalesced;
            }
            st.frames.push_back(frame);
            outcome
        };
        self.ready.notify_one();
        self.notify();
        outcome
    }

    /// Take the next frame, if any.
    pub fn pop(&self) -> Option<Frame> {
        let mut st = self.state.lock().unwrap();
        let frame = st.frames.pop_front();
        if frame.is_some() {
            st.full_since = None;
        }
        frame
    }

    /// Wait up to `timeout` for a frame. Returns `None` on timeout, or when
    /// the queue is closed and empty.
    pub fn pop_blocking(&self, timeout: Duration) -> Option<Frame> {
        let deadline = Instant::now() + timeout;
        let mut st = self.state.lock().unwrap();
        loop {
            if let Some(frame) = st.frames.pop_front() {
                st.full_since = None;
                return Some(frame);
            }
            if self.is_closed() {
                return None;
            }
            let now = Instant::now();
            if now >= deadline {
                return None;
            }
            st = self.ready.wait_timeout(st, deadline - now).unwrap().0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(s: &str) -> Arc<[u8]> {
        Arc::from(s.as_bytes())
    }

    #[test]
    fn full_queue_keeps_only_the_newest_snapshot() {
        let q = OutboundQueue::new(OutboundConfig {
            max_frames: 3,
            disconnect_after: Duration::from_secs(60),
        });
        for t in 1..=3 {
            assert_eq!(q.push_snapshot(t, 0, bytes("s")), PushOutcome::Queued);
        }
        assert_eq!(
            q.push_snapshot(4, 0, bytes("newest")),
            PushOutcome::Coalesced
        );
        assert_eq!(q.len(), 1);
        assert_eq!(q.dropped_snapshots(), 3);
        let f = q.pop().unwrap();
        assert_eq!(f.tick, 4);
        assert_eq!(&*f.bytes, b"newest");
        assert!(q.pop().is_none());
    }

    #[test]
    fn rejection_frames_survive_coalescing() {
        let q = OutboundQueue::new(OutboundConfig {
            max_frames: 3,
            disconnect_after: Duration::from_secs(60),
        });
        q.push_snapshot(1, 0, bytes("s1"));
        q.push_rejection(1, 0, bytes("ack"));
        q.push_snapshot(2, 0, bytes("s2"));
        assert_eq!(q.push_snapshot(3, 0, bytes("s3")), PushOutcome::Coalesced);
        let kinds: Vec<_> = std::iter::from_fn(|| q.pop())
            .map(|f| (f.kind, f.tick))
            .collect();
        assert_eq!(
            kinds,
            vec![(FrameKind::InputRejected, 1), (FrameKind::Snapshot, 3)]
        );
    }

    #[test]
    fn a_queue_full_of_rejections_closes() {
        let q = OutboundQueue::new(OutboundConfig {
            max_frames: 2,
            disconnect_after: Duration::from_secs(60),
        });
        q.push_rejection(1, 0, bytes("a"));
        q.push_rejection(2, 0, bytes("b"));
        assert_eq!(q.push_rejection(3, 0, bytes("c")), PushOutcome::Closed);
        assert!(q.is_closed());
    }

    #[test]
    fn a_queue_that_stays_full_closes_after_the_timeout() {
        let q = OutboundQueue::new(OutboundConfig {
            max_frames: 1,
            disconnect_after: Duration::from_millis(30),
        });
        q.push_snapshot(1, 0, bytes("a"));
        assert_eq!(q.push_snapshot(2, 0, bytes("b")), PushOutcome::Coalesced);
        std::thread::sleep(Duration::from_millis(40));
        assert_eq!(q.push_snapshot(3, 0, bytes("c")), PushOutcome::Closed);
        assert!(q.is_closed());
        // The writer still drains what was queued, then sees the close.
        assert!(q.pop_blocking(Duration::from_millis(10)).is_some());
        assert!(q.pop_blocking(Duration::from_millis(10)).is_none());
    }

    #[test]
    fn draining_resets_the_full_timer() {
        let q = OutboundQueue::new(OutboundConfig {
            max_frames: 1,
            disconnect_after: Duration::from_millis(30),
        });
        q.push_snapshot(1, 0, bytes("a"));
        q.push_snapshot(2, 0, bytes("b")); // full → timer starts
        std::thread::sleep(Duration::from_millis(40));
        q.pop(); // the writer caught up
        q.push_snapshot(3, 0, bytes("c"));
        assert_eq!(q.push_snapshot(4, 0, bytes("d")), PushOutcome::Coalesced);
        assert!(!q.is_closed());
    }

    #[test]
    fn pop_blocking_wakes_on_push_and_notifier_fires() {
        let q = OutboundQueue::new(OutboundConfig::default());
        let fired = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let f2 = Arc::clone(&fired);
        q.set_notifier(move || {
            f2.fetch_add(1, Ordering::Relaxed);
        });
        let q2 = Arc::clone(&q);
        let h = std::thread::spawn(move || q2.pop_blocking(Duration::from_secs(5)));
        std::thread::sleep(Duration::from_millis(20));
        q.push_snapshot(7, 0, bytes("x"));
        assert_eq!(h.join().unwrap().unwrap().tick, 7);
        assert_eq!(fired.load(Ordering::Relaxed), 1);
    }
}
