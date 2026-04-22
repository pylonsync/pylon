//! Type-erased shard interface for generic HTTP dispatch.
//!
//! The router receives untyped JSON from HTTP clients and doesn't know the
//! concrete `SimState`. [`DynShard`] is an object-safe view of a [`Shard`]
//! that accepts JSON inputs and delegates snapshot encoding to the shard's
//! configured format.

use std::sync::Arc;

use serde::de::DeserializeOwned;

use crate::shard::{Shard, ShardAuth, ShardError, SimState};
use crate::subscriber::{SnapshotSink, Subscriber, SubscriberId};

// ---------------------------------------------------------------------------
// DynShard — object-safe wrapper over Shard<S>
// ---------------------------------------------------------------------------

/// Type-erased shard operations. Implemented for every `Shard<S>` whose
/// `SimState::Input` is deserializable from JSON.
///
/// The router and HTTP layer work exclusively with `Arc<dyn DynShard>` —
/// they never see the concrete simulation type.
pub trait DynShard: Send + Sync {
    fn id(&self) -> &str;
    fn is_running(&self) -> bool;
    fn tick_number(&self) -> u64;
    fn subscriber_count(&self) -> usize;
    fn input_queue_len(&self) -> usize;

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

    /// Subscribe a transport (WebSocket / SSE writer) to snapshots, after
    /// running `SimState::authorize_subscribe`.
    fn add_subscriber(
        &self,
        id: SubscriberId,
        sink: SnapshotSink,
        auth: &ShardAuth,
    ) -> Result<(), ShardError>;

    /// Remove a subscriber (e.g. on disconnect).
    fn remove_subscriber(&self, id: &SubscriberId) -> bool;

    /// Stop the shard (no further ticks; tick loop will exit).
    fn stop(&self);
}

impl<S: SimState> DynShard for Shard<S>
where
    S::Input: DeserializeOwned,
    S::Snapshot: serde::Serialize + Clone,
{
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

    fn push_input_json(
        &self,
        subscriber_id: SubscriberId,
        body: &str,
        client_seq: Option<u64>,
        auth: &ShardAuth,
    ) -> Result<u64, ShardError> {
        let input: S::Input = serde_json::from_str(body).map_err(|e| {
            ShardError::Other(format!("invalid input JSON: {e}"))
        })?;
        Shard::push_input_authorized(self, subscriber_id, input, client_seq, auth)
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

    fn stop(&self) {
        Shard::stop(self);
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

impl<S: SimState> DynShardRegistry for crate::registry::ShardRegistry<S>
where
    S::Input: DeserializeOwned,
    S::Snapshot: serde::Serialize + Clone,
{
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
        let shard: Arc<Shard<Counter>> = Shard::new(
            "t",
            Counter { value: 0 },
            ShardConfig::default(),
        );
        // Attach a subscriber first — push_input_authorized verifies
        // that the sender is an active subscriber, not a forged id.
        let sub = Subscriber::new(SubscriberId::new("p1"), Box::new(|_t, _b| {}));
        shard.add_subscriber(sub).unwrap();

        let dyn_shard: Arc<dyn DynShard> = shard.clone();
        assert_eq!(dyn_shard.id(), "t");

        let admin = ShardAuth { user_id: Some("a".into()), is_admin: true };
        let seq = dyn_shard
            .push_input_json(SubscriberId::new("p1"), "5", None, &admin)
            .unwrap();
        assert_eq!(seq, 1);

        shard.run_tick();
    }

    #[test]
    fn push_input_json_rejects_garbage() {
        let shard: Arc<Shard<Counter>> = Shard::new(
            "t",
            Counter { value: 0 },
            ShardConfig::default(),
        );
        let dyn_shard: Arc<dyn DynShard> = shard;
        let admin = ShardAuth { user_id: Some("a".into()), is_admin: true };
        let err = dyn_shard
            .push_input_json(SubscriberId::new("p1"), "not json", None, &admin)
            .unwrap_err();
        match err {
            ShardError::Other(msg) => assert!(msg.contains("invalid input")),
            _ => panic!("expected Other error"),
        }
    }
}
