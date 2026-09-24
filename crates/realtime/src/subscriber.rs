//! Subscribers — clients connected to a shard that receive snapshots.

use std::sync::{Arc, Mutex};

use crate::outbound::OutboundQueue;
use crate::snapshot::{encode_snapshot, EncodeSnapshot, SnapshotFormat};
use crate::wire::InputRejection;

// ---------------------------------------------------------------------------
// SubscriberId
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SubscriberId(String);

impl SubscriberId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SubscriberId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for SubscriberId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl From<String> for SubscriberId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

// ---------------------------------------------------------------------------
// SnapshotSink — transport-agnostic delivery of encoded snapshots
// ---------------------------------------------------------------------------

/// Delivers encoded snapshot bytes to a connected client.
///
/// `tick` is the shard's tick number; `bytes` is the encoded snapshot.
///
/// The shard calls a sink on its tick thread (after it releases the state
/// lock), once per subscriber per tick. A sink must not block: a network
/// transport that can stall uses an [`OutboundQueue`] instead (see
/// [`Subscriber::with_queue`]). Sinks suit in-process consumers and
/// transports whose send never blocks, such as a Durable Object WebSocket.
pub type SnapshotSink = Box<dyn Fn(u64, &[u8]) + Send + Sync>;

enum Delivery {
    Sink(SnapshotSink),
    Queue(Arc<OutboundQueue>),
}

// ---------------------------------------------------------------------------
// Subscriber
// ---------------------------------------------------------------------------

pub struct Subscriber<T> {
    id: SubscriberId,
    delivery: Delivery,
    /// When `delta_mode` is on, the previous encoded snapshot bytes are kept
    /// here; subsequent `send()` calls emit only the JSON patch from the
    /// previous to current snapshot.
    last_snapshot: Mutex<Option<Vec<u8>>>,
    delta_mode: bool,
    /// The subscriber never holds a `T`, only encodes one passed to `send`,
    /// so it stays Send + Sync whatever the snapshot type is.
    _phantom: std::marker::PhantomData<fn(&T)>,
}

impl<T: EncodeSnapshot> Subscriber<T> {
    /// A subscriber that receives frames through a direct sink.
    pub fn new(id: SubscriberId, sink: SnapshotSink) -> Self {
        Self::build(id, Delivery::Sink(sink))
    }

    /// A subscriber that receives frames through an outbound queue. The
    /// transport drains the queue from its own thread or task.
    pub fn with_queue(id: SubscriberId, queue: Arc<OutboundQueue>) -> Self {
        Self::build(id, Delivery::Queue(queue))
    }

    fn build(id: SubscriberId, delivery: Delivery) -> Self {
        Self {
            id,
            delivery,
            last_snapshot: Mutex::new(None),
            delta_mode: false,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Enable delta mode: after the first (full) snapshot, emit JSON patches
    /// (RFC 6902) containing only the fields that changed since the previous
    /// tick. Reduces bandwidth for sims with large but slowly-changing state.
    pub fn with_delta_mode(mut self, enabled: bool) -> Self {
        self.delta_mode = enabled;
        self
    }

    pub fn id(&self) -> &SubscriberId {
        &self.id
    }

    /// The outbound queue, for a queued subscriber.
    pub fn queue(&self) -> Option<&Arc<OutboundQueue>> {
        match &self.delivery {
            Delivery::Queue(q) => Some(q),
            Delivery::Sink(_) => None,
        }
    }

    /// True when the subscriber's queue has closed (the client fell too far
    /// behind or disconnected). The shard drops closed subscribers.
    pub fn is_closed(&self) -> bool {
        self.queue().is_some_and(|q| q.is_closed())
    }

    fn deliver(&self, tick: u64, ack: u64, frame: Vec<u8>) {
        match &self.delivery {
            Delivery::Sink(sink) => sink(tick, &frame),
            Delivery::Queue(q) => {
                q.push_snapshot(tick, ack, Arc::from(frame));
            }
        }
    }

    /// Tell a queued subscriber that one of its inputs did not take effect.
    /// A sink subscriber gets nothing: a sink only carries snapshots.
    pub fn reject(&self, tick: u64, ack: u64, rejection: &InputRejection, format: SnapshotFormat) {
        let Some(q) = self.queue() else { return };
        match encode_snapshot(rejection, format) {
            Ok(bytes) => {
                q.push_rejection(tick, ack, Arc::from(bytes));
            }
            Err(e) => tracing::warn!("[realtime] rejection encode failed for {}: {}", self.id, e),
        }
    }

    /// Encode a snapshot in the shard's configured format and send it.
    /// `ack` is the highest `client_seq` the shard has processed for this
    /// subscriber (0 = none); queued transports put it in the frame header.
    ///
    /// In delta mode, sends a full snapshot on the first tick, then only
    /// the diff on subsequent ticks. When the queue is full the push drops
    /// the queued frames, so the frame that replaces them is a full one.
    pub fn send(&self, tick: u64, snapshot: &T, format: SnapshotFormat, ack: u64) {
        let encoded = match snapshot.encode_as(format) {
            Ok(bytes) => bytes,
            Err(e) => {
                tracing::warn!("[realtime] snapshot encode failed for {}: {}", self.id, e);
                return;
            }
        };

        if !self.delta_mode {
            self.deliver(tick, ack, encoded);
            return;
        }

        // Delta mode: compute field-level diff against the previous snapshot.
        let mut last = self.last_snapshot.lock().unwrap();
        let resync = self.queue().is_some_and(|q| q.is_full());
        let frame = match &*last {
            Some(prev) if !resync => {
                // Emit a small JSON envelope: {"delta": {...changed fields...}}
                // Falls back to full snapshot if either side isn't JSON-parsable.
                match (
                    serde_json::from_slice(prev),
                    serde_json::from_slice(&encoded),
                ) {
                    (Ok(a), Ok(b)) => {
                        let patch = json_diff(&a, &b);
                        if patch.as_object().map(|o| o.is_empty()).unwrap_or(false) {
                            // Nothing changed — skip send entirely.
                            *last = Some(encoded);
                            return;
                        }
                        let wrapped = serde_json::json!({ "delta": patch, "tick": tick });
                        serde_json::to_vec(&wrapped).unwrap_or_else(|_| encoded.clone())
                    }
                    _ => encoded.clone(),
                }
            }
            // First tick, or a resync after dropped frames: send it whole.
            _ => encoded.clone(),
        };

        *last = Some(encoded);
        drop(last);
        self.deliver(tick, ack, frame);
    }
}

/// Shallow field-level diff between two JSON values. Returns a JSON object
/// containing only the keys whose values changed in `b` relative to `a`.
///
/// - Deleted keys are emitted as `null`.
/// - Nested objects recurse.
/// - Arrays / primitives are replaced wholesale.
fn json_diff(a: &serde_json::Value, b: &serde_json::Value) -> serde_json::Value {
    use serde_json::Value::*;
    match (a, b) {
        (Object(ao), Object(bo)) => {
            let mut out = serde_json::Map::new();
            for (k, bv) in bo {
                match ao.get(k) {
                    Some(av) if av == bv => {} // unchanged
                    Some(av) => {
                        let sub = json_diff(av, bv);
                        if sub.as_object().map(|o| o.is_empty()).unwrap_or(false) {
                            // Changed but diff is empty (nested object equality).
                            // This shouldn't happen given the equality check above,
                            // but guard for safety.
                        } else {
                            out.insert(k.clone(), sub);
                        }
                    }
                    None => {
                        out.insert(k.clone(), bv.clone());
                    }
                }
            }
            for k in ao.keys() {
                if !bo.contains_key(k) {
                    out.insert(k.clone(), Null);
                }
            }
            Object(out)
        }
        // Any other type: emit the new value as-is.
        _ => b.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_empty_when_equal() {
        let a = serde_json::json!({"a": 1, "b": 2});
        let b = serde_json::json!({"a": 1, "b": 2});
        let d = json_diff(&a, &b);
        assert!(d.as_object().unwrap().is_empty());
    }

    #[test]
    fn diff_includes_changed_field() {
        let a = serde_json::json!({"a": 1, "b": 2});
        let b = serde_json::json!({"a": 1, "b": 3});
        let d = json_diff(&a, &b);
        assert_eq!(d, serde_json::json!({"b": 3}));
    }

    #[test]
    fn diff_includes_new_field() {
        let a = serde_json::json!({"a": 1});
        let b = serde_json::json!({"a": 1, "c": 5});
        let d = json_diff(&a, &b);
        assert_eq!(d, serde_json::json!({"c": 5}));
    }

    #[test]
    fn diff_marks_deleted_as_null() {
        let a = serde_json::json!({"a": 1, "b": 2});
        let b = serde_json::json!({"a": 1});
        let d = json_diff(&a, &b);
        assert_eq!(d, serde_json::json!({"b": null}));
    }
}
