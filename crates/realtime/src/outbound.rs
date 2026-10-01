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
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
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
    /// A replication datagram (`pylon_replication::datagram`), for a queue
    /// in datagram mode. Lossy by design: a full queue drops datagrams
    /// first, and a dropped one only delays what it held.
    Datagram,
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
    /// The largest datagram the transport carries; 0 when it carries none.
    datagram_max: AtomicUsize,
    /// Datagram acks from the client, for the shard to take on its next
    /// tick: (datagram number, the client's stream tick then).
    datagram_acks: Mutex<Vec<(u64, u64)>>,
    /// The connection's last round-trip samples, in microseconds.
    rtt_samples: Mutex<VecDeque<u64>>,
    state: Mutex<State>,
    ready: Condvar,
    closed: AtomicBool,
    /// Called after every push and on close, for writers that do not block
    /// on [`OutboundQueue::pop_blocking`] (an async task's waker).
    notifier: Mutex<Option<Notifier>>,
}

/// Round-trip samples a queue keeps (see [`OutboundQueue::rtt`]).
const RTT_SAMPLES: usize = 8;

impl OutboundQueue {
    pub fn new(config: OutboundConfig) -> Arc<Self> {
        Arc::new(Self {
            config: OutboundConfig {
                max_frames: config.max_frames.max(1),
                ..config
            },
            datagram_max: AtomicUsize::new(0),
            datagram_acks: Mutex::new(Vec::new()),
            rtt_samples: Mutex::new(VecDeque::new()),
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

    /// Put the queue in datagram mode (a transport that carries datagrams
    /// of up to `max_size` bytes), or back out of it with 0. A replicating
    /// shard then sends this subscription's updates as datagrams.
    pub fn set_datagram_max(&self, max_size: usize) {
        self.datagram_max.store(max_size, Ordering::Release);
    }

    /// The largest datagram the transport carries, in datagram mode.
    pub fn datagram_max(&self) -> Option<usize> {
        match self.datagram_max.load(Ordering::Acquire) {
            0 => None,
            n => Some(n),
        }
    }

    /// Hand the shard the client's datagram acks.
    pub fn push_datagram_acks(&self, acks: &[(u64, u64)]) {
        // Bounded: a client flooding acks cannot grow this without limit.
        const MAX_PENDING: usize = 4096;
        let mut pending = self.datagram_acks.lock().unwrap();
        let room = MAX_PENDING.saturating_sub(pending.len());
        pending.extend(acks.iter().take(room).copied());
    }

    /// The acks received since the last call.
    pub fn take_datagram_acks(&self) -> Vec<(u64, u64)> {
        std::mem::take(&mut *self.datagram_acks.lock().unwrap())
    }

    /// Add a round-trip sample: a ping's pong, or QUIC's estimate.
    pub fn record_rtt(&self, rtt: Duration) {
        let sample = rtt.as_micros().clamp(1, u64::MAX as u128) as u64;
        let mut samples = self.rtt_samples.lock().unwrap();
        samples.push_back(sample);
        while samples.len() > RTT_SAMPLES {
            samples.pop_front();
        }
    }

    /// The connection's round trip: the least of its recent samples, once
    /// the transport has measured one. The least, because a client can
    /// only add delay to a sample (holding a pong back), never take it
    /// away, and lag compensation lets a longer round trip reach further
    /// back.
    pub fn rtt(&self) -> Option<Duration> {
        self.rtt_samples
            .lock()
            .unwrap()
            .iter()
            .min()
            .map(|us| Duration::from_micros(*us))
    }

    pub fn len(&self) -> usize {
        self.state.lock().unwrap().frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// True when the next snapshot push would drop queued snapshots.
    /// Queued datagrams do not count: a push drops them first.
    pub fn is_full(&self) -> bool {
        let st = self.state.lock().unwrap();
        Self::reliable_len(&st) >= self.config.max_frames
    }

    /// Frames other than datagrams.
    fn reliable_len(st: &State) -> usize {
        st.frames
            .iter()
            .filter(|f| f.kind != FrameKind::Datagram)
            .count()
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
            let mut st = self.state.lock().unwrap();
            if st.dropped_snapshots != expected_dropped
                || Self::reliable_len(&st) >= self.config.max_frames
            {
                return PushOutcome::NeedsBaseline;
            }
            // Datagrams make room for a delta; they are lossy anyway.
            while st.frames.len() >= self.config.max_frames {
                match st.frames.iter().position(|f| f.kind == FrameKind::Datagram) {
                    Some(i) => {
                        st.frames.remove(i);
                    }
                    None => break,
                }
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

    /// Queue a replication datagram. It never closes the queue, never counts
    /// as a dropped frame, and never displaces another kind of frame: a
    /// full queue drops its oldest datagram, or this one when it holds none.
    /// Returns `Queued` when this datagram was queued, `Coalesced` when it
    /// was dropped.
    pub fn push_datagram(&self, tick: u64, bytes: Arc<[u8]>) -> PushOutcome {
        if self.is_closed() {
            return PushOutcome::Closed;
        }
        let mut st = self.state.lock().unwrap();
        if st.frames.len() >= self.config.max_frames {
            match st.frames.iter().position(|f| f.kind == FrameKind::Datagram) {
                Some(i) => {
                    st.frames.remove(i);
                }
                None => return PushOutcome::Coalesced,
            }
        }
        st.frames.push_back(Frame {
            tick,
            kind: FrameKind::Datagram,
            ack: 0,
            bytes,
        });
        drop(st);
        self.ready.notify_one();
        self.notify();
        PushOutcome::Queued
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
            st.frames.retain(|f| f.kind != FrameKind::Datagram);
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
                // Datagrams go first, and do not count as dropped frames.
                st.frames.retain(|f| f.kind != FrameKind::Datagram);
            }
            if st.frames.len() >= self.config.max_frames {
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

    #[test]
    fn the_round_trip_is_the_least_recent_sample() {
        let q = OutboundQueue::new(OutboundConfig::default());
        assert_eq!(q.rtt(), None);
        q.record_rtt(Duration::from_millis(80));
        q.record_rtt(Duration::from_millis(40));
        // A held-back pong cannot raise it.
        q.record_rtt(Duration::from_millis(900));
        assert_eq!(q.rtt(), Some(Duration::from_millis(40)));
        // Only the recent samples count.
        for _ in 0..RTT_SAMPLES {
            q.record_rtt(Duration::from_millis(60));
        }
        assert_eq!(q.rtt(), Some(Duration::from_millis(60)));
    }
    use super::*;

    fn bytes(s: &str) -> Arc<[u8]> {
        Arc::from(s.as_bytes())
    }

    #[test]
    fn datagrams_give_way_and_never_count_as_dropped_frames() {
        let q = OutboundQueue::new(OutboundConfig {
            max_frames: 3,
            disconnect_after: Duration::from_secs(60),
        });
        q.set_datagram_max(1200);
        assert_eq!(q.datagram_max(), Some(1200));
        assert_eq!(
            q.push_replication(1, 0, bytes("base"), None),
            PushOutcome::Queued
        );
        assert_eq!(q.push_datagram(1, bytes("d1")), PushOutcome::Queued);
        assert_eq!(q.push_datagram(2, bytes("d2")), PushOutcome::Queued);
        // Full, but of datagrams: not full for a delta or a snapshot.
        assert!(!q.is_full());
        // A new datagram replaces the oldest datagram.
        assert_eq!(q.push_datagram(3, bytes("d3")), PushOutcome::Queued);
        // A delta makes room by dropping datagrams, and needs no baseline.
        assert_eq!(
            q.push_replication(4, 0, bytes("delta"), Some(0)),
            PushOutcome::Queued
        );
        assert_eq!(q.dropped_snapshots(), 0);
        let kinds: Vec<(FrameKind, Vec<u8>)> = std::iter::from_fn(|| q.pop())
            .map(|f| (f.kind, f.bytes.to_vec()))
            .collect();
        assert_eq!(
            kinds,
            vec![
                (FrameKind::Replication, b"base".to_vec()),
                (FrameKind::Datagram, b"d3".to_vec()),
                (FrameKind::Replication, b"delta".to_vec()),
            ]
        );
        // A queue full of frames that are not datagrams drops the datagram.
        for i in 0..3 {
            q.push_rejection(i, 0, bytes("r"));
        }
        assert_eq!(q.push_datagram(9, bytes("late")), PushOutcome::Coalesced);
        assert!(!q.is_closed());
    }

    #[test]
    fn datagram_acks_are_handed_over_once_and_bounded() {
        let q = OutboundQueue::new(OutboundConfig::default());
        q.push_datagram_acks(&[(1, 1), (2, 1)]);
        q.push_datagram_acks(&[(3, 2)]);
        assert_eq!(q.take_datagram_acks(), vec![(1, 1), (2, 1), (3, 2)]);
        assert!(q.take_datagram_acks().is_empty());
        let flood: Vec<(u64, u64)> = (0..10_000).map(|i| (i, 0)).collect();
        q.push_datagram_acks(&flood);
        assert_eq!(q.take_datagram_acks().len(), 4096);
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
