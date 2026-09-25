//! At-most-once envelopes (see [`crate::ClusterBus::try_publish`]): a queue
//! of their own, so they never wait in front of, or behind, committed
//! changes. The queue is bounded by count and by bytes; one thread sends
//! each envelope once, and a failure drops it (a retry after an ambiguous
//! failure could deliver it twice).

use crate::Envelope;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::Arc;
use std::thread;
use tracing::debug;

/// Envelopes waiting to be sent.
pub(crate) const MAX_QUEUED: usize = 4096;
/// Bytes of JSON waiting to be sent.
pub(crate) const MAX_QUEUED_BYTES: usize = 32 * 1024 * 1024;

pub(crate) struct BestEffort {
    sender: SyncSender<(Envelope, usize)>,
    queued_bytes: Arc<AtomicUsize>,
}

impl BestEffort {
    /// Start the sending thread. `send` delivers one envelope, once.
    pub(crate) fn spawn(
        name: &str,
        mut send: impl FnMut(&Envelope) -> Result<(), String> + Send + 'static,
    ) -> Result<Self, String> {
        let (sender, receiver) = sync_channel::<(Envelope, usize)>(MAX_QUEUED);
        let queued_bytes = Arc::new(AtomicUsize::new(0));
        let bytes = Arc::clone(&queued_bytes);
        thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                while let Ok((envelope, size)) = receiver.recv() {
                    if let Err(e) = send(&envelope) {
                        debug!("[cluster] {} envelope dropped: {e}", envelope.kind);
                    }
                    bytes.fetch_sub(size, Ordering::AcqRel);
                }
            })
            .map_err(|e| format!("spawn {name}: {e}"))?;
        Ok(Self {
            sender,
            queued_bytes,
        })
    }

    /// Queue `envelope`; false (dropped) when the queue is full by count or
    /// by bytes.
    pub(crate) fn try_send(&self, envelope: &Envelope) -> bool {
        let size = serde_json::to_string(&envelope.payload)
            .map(|s| s.len())
            .unwrap_or(usize::MAX);
        let before = self.queued_bytes.fetch_add(size, Ordering::AcqRel);
        if before.saturating_add(size) > MAX_QUEUED_BYTES
            || self.sender.try_send((envelope.clone(), size)).is_err()
        {
            self.queued_bytes.fetch_sub(size, Ordering::AcqRel);
            return false;
        }
        true
    }

    #[cfg(test)]
    pub(crate) fn queued_bytes(&self) -> usize {
        self.queued_bytes.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;
    use std::sync::Mutex;
    use std::time::Duration;

    fn envelope(bytes: usize) -> Envelope {
        Envelope {
            instance_id: "me".into(),
            kind: "shard-message".into(),
            payload: serde_json::json!({ "data_b64": "x".repeat(bytes) }),
        }
    }

    /// A stalled transport fills the queue by bytes, then drops; each
    /// envelope is sent once, and failures are not retried.
    #[test]
    fn the_queue_is_bounded_by_bytes_and_sends_each_envelope_once() {
        let (release_tx, release_rx) = channel::<()>();
        let release_rx = Mutex::new(release_rx);
        let sent = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&sent);
        let queue = BestEffort::spawn("test-best-effort", move |_| {
            release_rx.lock().unwrap().recv().ok();
            counted.fetch_add(1, Ordering::SeqCst);
            Err("the transport failed".into())
        })
        .unwrap();
        let big = MAX_QUEUED_BYTES / 4;
        let mut taken = 0;
        while queue.try_send(&envelope(big)) {
            taken += 1;
            assert!(taken < 10, "the byte limit never refused");
        }
        assert!(taken <= 5, "{taken} envelopes of {big} bytes queued");
        assert!(queue.queued_bytes() <= MAX_QUEUED_BYTES + big);
        for _ in 0..taken {
            release_tx.send(()).unwrap();
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while queue.queued_bytes() > 0 {
            assert!(std::time::Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(sent.load(Ordering::SeqCst), taken);
        // Room again.
        assert!(queue.try_send(&envelope(10)));
    }
}
