//! `pylon-cluster` — cross-machine fanout for horizontally-scaled
//! deployments.
//!
//! Pylon's WebSocket + SSE broadcast paths fan changes to every client
//! connected to the SAME pylon process. For a single-machine deploy
//! that's enough. As soon as the deploy spreads across multiple
//! machines (Fly autoscale, K8s replicas, blue/green rollouts), a
//! mutation on machine A is invisible to a WS subscriber on machine B
//! unless something carries the event between them. This crate is
//! that something.
//!
//! ## Design
//!
//! The [`ClusterBus`] trait is a pure pubsub abstraction. Producers
//! (mutation paths, presence relays, CRDT frames) call [`publish`].
//! Consumers (one per machine, started at boot) call [`subscribe`]
//! with a handler that re-applies the inbound envelope to local hubs.
//!
//! Two impls ship:
//!
//! - [`NoopBus`] — the default. publish is a no-op, subscribe never
//!   delivers. Single-machine deploys pay zero overhead.
//! - [`RedisBus`] (feature `redis-bus`) — Redis PUB/SUB. PUBLISH per
//!   envelope on a channel namespaced by the operator's config; a
//!   dedicated background thread holds the SUBSCRIBE connection and
//!   dispatches messages to the handler.
//!
//! ## Self-event filtering
//!
//! Pubsub backends deliver every published message to every
//! subscriber, including the publisher itself. Without de-dup that
//! produces a feedback loop: A publishes → A's subscriber receives →
//! A re-broadcasts locally → already shipped via the originating
//! path → double-delivery to client.
//!
//! Every [`Envelope`] carries the publisher's `instance_id`. Each
//! pylon process mints one at startup; the bus's [`instance_id`]
//! method exposes it so subscribers can compare and skip.
//!
//! ## Authz isn't on the bus
//!
//! The bus carries raw events. Per-client read-policy filtering
//! happens at the LOCAL WS hub when the inbound event fans out —
//! identical to the in-process path. The bus is trusted infra
//! shared between pylon machines; clients never see envelopes that
//! shouldn't reach their auth context.

use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Wire envelope. `event` carries the JSON payload for change events
/// + presence relays; `binary` carries CRDT frames (Loro snapshots /
/// updates) base64-encoded so the same JSON pubsub channel can carry
/// both shapes without a parallel binary channel. Binary frames are
/// typically small (few KB), so the base64 overhead is acceptable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    /// Pylon process that originated this event. Subscribers compare
    /// against their own [`ClusterBus::instance_id`] and skip self-
    /// originated messages to avoid the publish→subscribe feedback
    /// loop.
    pub instance_id: String,
    /// Discriminator for the payload shape. Kept as a string rather
    /// than an enum so adding new event types doesn't require a
    /// coordinated rolling restart — older subscribers will see an
    /// unknown kind and skip.
    pub kind: String,
    /// JSON payload. Shape depends on `kind`:
    ///  - `"change"` → a [`pylon_sync::ChangeEvent`]
    ///  - `"presence"` → opaque presence message (mirrors WS shape)
    ///  - `"crdt"` → `{entity, row_id, snapshot_b64}` for binary CRDT
    pub payload: serde_json::Value,
}

impl Envelope {
    pub fn change(instance_id: &str, event: &pylon_sync::ChangeEvent) -> Self {
        Self {
            instance_id: instance_id.to_string(),
            kind: "change".to_string(),
            payload: serde_json::to_value(event).unwrap_or(serde_json::Value::Null),
        }
    }

    pub fn presence(instance_id: &str, payload: serde_json::Value) -> Self {
        Self {
            instance_id: instance_id.to_string(),
            kind: "presence".to_string(),
            payload,
        }
    }

    pub fn crdt(instance_id: &str, entity: &str, row_id: &str, snapshot: &[u8]) -> Self {
        use base64::Engine;
        Self {
            instance_id: instance_id.to_string(),
            kind: "crdt".to_string(),
            payload: serde_json::json!({
                "entity": entity,
                "row_id": row_id,
                "snapshot_b64": base64::engine::general_purpose::STANDARD.encode(snapshot),
            }),
        }
    }

    /// Session-changed envelope. Carries the user_id whose session
    /// mutated plus the post-mutation tenant_id (None for "no active
    /// org" / revoked). Subscribers on other machines fan a
    /// `session-changed` WS message to that user's local connections
    /// so multi-tab + multi-machine deployments stay in sync without
    /// the app calling notifySessionChanged.
    pub fn session_changed(
        instance_id: &str,
        user_id: &str,
        tenant_id: Option<&str>,
    ) -> Self {
        Self {
            instance_id: instance_id.to_string(),
            kind: "session-changed".to_string(),
            payload: serde_json::json!({
                "user_id": user_id,
                "tenant_id": tenant_id,
            }),
        }
    }

    /// Decode a session-changed payload to `(user_id, tenant_id)`.
    /// None if kind doesn't match or payload is malformed.
    pub fn as_session_changed(&self) -> Option<(String, Option<String>)> {
        if self.kind != "session-changed" {
            return None;
        }
        let user_id = self.payload.get("user_id")?.as_str()?.to_string();
        let tenant_id = self
            .payload
            .get("tenant_id")
            .and_then(|v| v.as_str().map(String::from));
        Some((user_id, tenant_id))
    }

    /// Decode the change-event payload. Returns None if `kind` isn't
    /// `"change"` or the payload doesn't deserialize. Caller handles
    /// the None as "skip this envelope" — unknown / malformed events
    /// shouldn't kill the subscriber thread.
    pub fn as_change(&self) -> Option<pylon_sync::ChangeEvent> {
        if self.kind != "change" {
            return None;
        }
        serde_json::from_value(self.payload.clone()).ok()
    }

    /// Decode the CRDT payload back to raw snapshot bytes. None if
    /// `kind` isn't `"crdt"` or the base64 is invalid.
    pub fn as_crdt(&self) -> Option<CrdtPayload> {
        use base64::Engine;
        if self.kind != "crdt" {
            return None;
        }
        let entity = self.payload.get("entity")?.as_str()?.to_string();
        let row_id = self.payload.get("row_id")?.as_str()?.to_string();
        let b64 = self.payload.get("snapshot_b64")?.as_str()?;
        let snapshot = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
        Some(CrdtPayload {
            entity,
            row_id,
            snapshot,
        })
    }
}

pub struct CrdtPayload {
    pub entity: String,
    pub row_id: String,
    pub snapshot: Vec<u8>,
}

/// Subscriber handler. Runs on the bus's background thread; must be
/// fast (no DB calls, no blocking, no panic). Bus impls catch panics
/// and log them rather than crashing the subscriber thread, but a
/// crashed handler is still a crashed handler — keep it boring.
pub type SubscriberHandler = Arc<dyn Fn(Envelope) + Send + Sync>;

/// Cross-machine pubsub abstraction. See module docs for the contract.
pub trait ClusterBus: Send + Sync {
    fn publish(&self, envelope: &Envelope);
    fn subscribe(&self, handler: SubscriberHandler);
    fn instance_id(&self) -> &str;
}

// ---------------------------------------------------------------------------
// NoopBus — single-machine default. publish + subscribe are no-ops.
// ---------------------------------------------------------------------------

/// Default bus impl: drops every publish, never delivers a subscribe.
/// Used when no cluster transport is configured (single-machine
/// deploys). Costs nothing.
pub struct NoopBus {
    instance_id: String,
}

impl Default for NoopBus {
    fn default() -> Self {
        Self::new()
    }
}

impl NoopBus {
    pub fn new() -> Self {
        Self {
            instance_id: new_instance_id(),
        }
    }
}

impl ClusterBus for NoopBus {
    fn publish(&self, _envelope: &Envelope) {}
    fn subscribe(&self, _handler: SubscriberHandler) {}
    fn instance_id(&self) -> &str {
        &self.instance_id
    }
}

/// Mint a per-process instance id. Format: `pylon-<8-hex>` — short
/// enough to scan in logs, unique enough that two pylon processes in
/// the same minute won't collide in practice. UUID v4 + truncation
/// instead of a counter so two processes that boot simultaneously
/// can't pick the same id.
pub fn new_instance_id() -> String {
    let uuid = uuid::Uuid::new_v4().simple().to_string();
    format!("pylon-{}", &uuid[..8])
}

// ---------------------------------------------------------------------------
// RedisBus — feature-gated cross-machine transport via Redis PUB/SUB.
// ---------------------------------------------------------------------------

#[cfg(feature = "redis-bus")]
pub mod redis_bus;

#[cfg(feature = "redis-bus")]
pub use redis_bus::RedisBus;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use pylon_sync::{ChangeEvent, ChangeKind};

    fn sample_change() -> ChangeEvent {
        ChangeEvent {
            seq: 42,
            entity: "Recording".to_string(),
            row_id: "r_1".to_string(),
            kind: ChangeKind::Update,
            data: Some(serde_json::json!({"id": "r_1", "title": "hi"})),
            timestamp: "2026-05-17T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn change_envelope_round_trip() {
        let inst = new_instance_id();
        let ev = sample_change();
        let env = Envelope::change(&inst, &ev);
        assert_eq!(env.instance_id, inst);
        assert_eq!(env.kind, "change");
        let back = env.as_change().expect("change decodes");
        assert_eq!(back.seq, 42);
        assert_eq!(back.entity, "Recording");
    }

    #[test]
    fn change_decodes_to_none_for_wrong_kind() {
        let env = Envelope::presence("inst", serde_json::json!({}));
        assert!(env.as_change().is_none());
    }

    #[test]
    fn crdt_envelope_round_trip() {
        let inst = new_instance_id();
        let snap = b"\x00\xff\x42loro-snapshot-bytes";
        let env = Envelope::crdt(&inst, "Doc", "d1", snap);
        assert_eq!(env.kind, "crdt");
        let back = env.as_crdt().expect("crdt decodes");
        assert_eq!(back.entity, "Doc");
        assert_eq!(back.row_id, "d1");
        assert_eq!(back.snapshot, snap);
    }

    #[test]
    fn instance_ids_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..100 {
            seen.insert(new_instance_id());
        }
        assert_eq!(seen.len(), 100, "100 fresh instance ids should not collide");
    }

    #[test]
    fn noop_bus_publish_subscribe_are_silent() {
        let bus = NoopBus::new();
        bus.publish(&Envelope::change(bus.instance_id(), &sample_change()));
        // No assertions — just verifying no panic. The "subscribe never
        // delivers" contract is checked by the absence of any
        // handler-callback machinery in NoopBus.
        let called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let called_clone = std::sync::Arc::clone(&called);
        bus.subscribe(Arc::new(move |_env| {
            called_clone.store(true, std::sync::atomic::Ordering::SeqCst);
        }));
        bus.publish(&Envelope::change(bus.instance_id(), &sample_change()));
        // Handler should never fire — Noop is a sink, not a loopback.
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(!called.load(std::sync::atomic::Ordering::SeqCst));
    }
}
