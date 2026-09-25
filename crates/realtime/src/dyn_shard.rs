//! Type-erased shard interface for generic HTTP dispatch.
//!
//! The router receives untyped JSON from HTTP clients and doesn't know the
//! concrete `SimState`. [`DynShard`] is an object-safe view of a [`Shard`]
//! that accepts JSON inputs and delegates snapshot encoding to the shard's
//! configured format.

use std::sync::Arc;

use crate::outbound::OutboundQueue;
use crate::shard::{Shard, ShardAuth, ShardError, SimState};
use crate::snapshot::SnapshotFormat;
use crate::subscriber::{SnapshotSink, Subscriber, SubscriberId};
use crate::wire::{peek_client_seq, InputEnvelope, InputRejection, ShardInput};

// ---------------------------------------------------------------------------
// DynShard — object-safe wrapper over Shard<S>
// ---------------------------------------------------------------------------

/// Type-erased shard operations. Implemented for every `Shard<S>`.
///
/// The router and HTTP layer work exclusively with `Arc<dyn DynShard>` —
/// they never see the concrete simulation type.
pub trait DynShard: Send + Sync {
    fn id(&self) -> &str;
    fn is_running(&self) -> bool;
    fn tick_number(&self) -> u64;
    fn subscriber_count(&self) -> usize;
    fn input_queue_len(&self) -> usize;
    /// The codec of this shard's snapshots (and of binary input frames).
    fn snapshot_format(&self) -> SnapshotFormat;
    /// The highest `client_seq` processed for a subscriber (0 = none).
    fn ack(&self, id: &SubscriberId) -> u64;
    /// True when the shard sends entity replication frames (binary)
    /// instead of snapshots.
    fn replicates(&self) -> bool;

    /// Decode an input envelope `{ input, client_seq? }` encoded in
    /// `format`, authorize it, and queue it. On failure, returns the
    /// rejection to send back to the client.
    fn push_input_envelope(
        &self,
        subscriber_id: SubscriberId,
        format: SnapshotFormat,
        bytes: &[u8],
        auth: &ShardAuth,
    ) -> Result<u64, InputRejection>;

    /// Parse a JSON body as an input and queue it after authorization.
    ///
    /// Always runs `SimState::authorize_input` — there is no
    /// non-authorized variant on purpose. Callers that trust the input
    /// entirely should pass `ShardAuth::admin()` or a custom auth context.
    ///
    /// Returns the assigned server-side sequence number.
    fn push_input_json(
        &self,
        subscriber_id: SubscriberId,
        body: &str,
        client_seq: Option<u64>,
        auth: &ShardAuth,
    ) -> Result<u64, ShardError>;

    /// Subscribe a network transport after running
    /// `SimState::authorize_subscribe`. The shard pushes this subscriber's
    /// frames into the returned queue; the transport drains it from its own
    /// thread or task, so a slow client never stalls the tick.
    fn add_queued_subscriber(
        &self,
        id: SubscriberId,
        auth: &ShardAuth,
    ) -> Result<Arc<OutboundQueue>, ShardError>;

    /// Subscribe through a direct sink, after running
    /// `SimState::authorize_subscribe`. The sink runs on the tick thread and
    /// must not block; see [`SnapshotSink`].
    fn add_subscriber(
        &self,
        id: SubscriberId,
        sink: SnapshotSink,
        auth: &ShardAuth,
    ) -> Result<(), ShardError>;

    /// Remove every subscription with this id.
    fn remove_subscriber(&self, id: &SubscriberId) -> bool;

    /// Remove the one subscription behind `queue` (a transport's own
    /// connection closed). Other connections with the same id stay.
    fn remove_queued_subscriber(&self, queue: &Arc<OutboundQueue>) -> bool;

    /// Stop the shard (no further ticks; tick loop will exit).
    fn stop(&self);
}

impl<S: SimState> DynShard for Shard<S> {
    fn id(&self) -> &str {
        Shard::id(self)
    }
    fn is_running(&self) -> bool {
        Shard::is_running(self)
    }
    fn tick_number(&self) -> u64 {
        Shard::tick_number(self)
    }
    fn subscriber_count(&self) -> usize {
        Shard::subscriber_count(self)
    }
    fn input_queue_len(&self) -> usize {
        Shard::input_queue_len(self)
    }
    fn snapshot_format(&self) -> SnapshotFormat {
        self.config().snapshot_format
    }
    fn ack(&self, id: &SubscriberId) -> u64 {
        Shard::ack(self, id)
    }
    fn replicates(&self) -> bool {
        self.with_state(|s| s.replicated().is_some())
    }

    fn push_input_envelope(
        &self,
        subscriber_id: SubscriberId,
        format: SnapshotFormat,
        bytes: &[u8],
        auth: &ShardAuth,
    ) -> Result<u64, InputRejection> {
        let envelope: InputEnvelope<S::Input> =
            S::Input::decode_envelope(format, self.config().snapshot_format, bytes).map_err(
                |message| InputRejection {
                    client_seq: peek_client_seq(format, bytes),
                    code: "invalid".into(),
                    message,
                },
            )?;
        let client_seq = envelope.client_seq;
        Shard::push_input_authorized(self, subscriber_id, envelope.input, client_seq, auth)
            .map_err(|e| rejection_for(client_seq, &e))
    }

    fn push_input_json(
        &self,
        subscriber_id: SubscriberId,
        body: &str,
        client_seq: Option<u64>,
        auth: &ShardAuth,
    ) -> Result<u64, ShardError> {
        let input = S::Input::decode_json(body, self.config().snapshot_format)
            .map_err(ShardError::Other)?;
        Shard::push_input_authorized(self, subscriber_id, input, client_seq, auth)
    }

    fn add_queued_subscriber(
        &self,
        id: SubscriberId,
        auth: &ShardAuth,
    ) -> Result<Arc<OutboundQueue>, ShardError> {
        Shard::add_queued_subscriber_authorized(self, id, auth)
    }

    fn add_subscriber(
        &self,
        id: SubscriberId,
        sink: SnapshotSink,
        auth: &ShardAuth,
    ) -> Result<(), ShardError> {
        let sub: Subscriber<S::Snapshot> = Subscriber::new(id, sink);
        Shard::add_subscriber_authorized(self, sub, auth)
    }

    fn remove_subscriber(&self, id: &SubscriberId) -> bool {
        Shard::remove_subscriber(self, id)
    }

    fn remove_queued_subscriber(&self, queue: &Arc<OutboundQueue>) -> bool {
        Shard::remove_queued_subscriber(self, queue)
    }

    fn stop(&self) {
        Shard::stop(self);
    }
}

/// The rejection sent to a client for a push error.
pub fn rejection_for(client_seq: Option<u64>, err: &ShardError) -> InputRejection {
    let code = match err {
        ShardError::Unauthorized(_) => "unauthorized",
        ShardError::InputRateLimited => "rate_limited",
        ShardError::InputQueueFull => "queue_full",
        ShardError::Stopped => "stopped",
        ShardError::Full | ShardError::SubscriberNotFound | ShardError::Other(_) => "invalid",
    };
    InputRejection {
        client_seq,
        code: code.into(),
        message: err.to_string(),
    }
}

// ---------------------------------------------------------------------------
// DynShardRegistry — object-safe wrapper over ShardRegistry<S>
// ---------------------------------------------------------------------------

pub trait DynShardRegistry: Send + Sync {
    fn get(&self, id: &str) -> Option<Arc<dyn DynShard>>;
    fn ids(&self) -> Vec<String>;
    fn len(&self) -> usize;
}

impl<S: SimState> DynShardRegistry for crate::registry::ShardRegistry<S> {
    fn get(&self, id: &str) -> Option<Arc<dyn DynShard>> {
        crate::registry::ShardRegistry::<S>::get(self, id).map(|s| s as Arc<dyn DynShard>)
    }

    fn ids(&self) -> Vec<String> {
        crate::registry::ShardRegistry::<S>::ids(self)
    }

    fn len(&self) -> usize {
        crate::registry::ShardRegistry::<S>::len(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shard::{Shard, ShardConfig, SimState};
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
    fn push_input_json_roundtrip() {
        use crate::subscriber::Subscriber;
        let shard: Arc<Shard<Counter>> =
            Shard::new("t", Counter { value: 0 }, ShardConfig::default());
        // Attach a subscriber first — push_input_authorized verifies
        // that the sender is an active subscriber, not a forged id.
        let sub = Subscriber::new(SubscriberId::new("p1"), Box::new(|_t, _b| {}));
        shard.add_subscriber(sub).unwrap();

        let dyn_shard: Arc<dyn DynShard> = shard.clone();
        assert_eq!(dyn_shard.id(), "t");

        let admin = ShardAuth {
            user_id: Some("a".into()),
            is_admin: true,
            ..Default::default()
        };
        let seq = dyn_shard
            .push_input_json(SubscriberId::new("p1"), "5", None, &admin)
            .unwrap();
        assert_eq!(seq, 1);

        shard.run_tick();
    }

    #[test]
    fn push_input_json_rejects_garbage() {
        let shard: Arc<Shard<Counter>> =
            Shard::new("t", Counter { value: 0 }, ShardConfig::default());
        let dyn_shard: Arc<dyn DynShard> = shard;
        let admin = ShardAuth {
            user_id: Some("a".into()),
            is_admin: true,
            ..Default::default()
        };
        let err = dyn_shard
            .push_input_json(SubscriberId::new("p1"), "not json", None, &admin)
            .unwrap_err();
        match err {
            ShardError::Other(msg) => assert!(msg.contains("invalid input")),
            _ => panic!("expected Other error"),
        }
    }
}
