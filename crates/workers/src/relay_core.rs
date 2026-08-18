//! Sync-relay core — the platform-independent half of the `PylonSync`
//! Durable Object (docs/SYNC_DURABLE_OBJECTS_DESIGN.md, Option C).
//!
//! Ungated so it compiles + tests on the host: the DO class in
//! `sync_do.rs` is thin wasm plumbing around this module.
//!
//! ## Filter parity contract
//!
//! [`RelayFilter::wire_json_for`] MUST match the machine's own WS
//! fan-out — `Shard::broadcast_change` + `WsHub::broadcast` in
//! `crates/runtime/src/ws.rs` — decision for decision:
//!
//! 1. Unscoped admin (`is_unscoped_admin`) bypasses policy.
//! 2. Otherwise `check_entity_read(entity, auth, RAW data)` — the raw
//!    row, so policies referencing `serverOnly` fields still evaluate.
//! 3. Post-state denied + Update + pre-state allowed → a synthesized
//!    Delete tombstone at the same seq (visibility flip).
//! 4. What ships is PROJECTED (`project_row_for_replication_opt`:
//!    User allowlist + `serverOnly` strip + `syncOmit` strip) and
//!    `prev_data` is ALWAYS stripped from the wire.
//!
//! Like the WS hub, this checks the read policy only — NOT
//! `sync_scope` (the HTTP pull path checks both; the live fan-out
//! deliberately doesn't, and the relay must not diverge from the
//! surface it replaces or the dual-write diff never converges).
//!
//! Known divergence, fail-closed: policies using `exists(...)` need a
//! `PolicyDataResolver` (a database). The DO installs none, so those
//! policies deny. Apps with `exists()` read policies are not
//! relay-eligible yet; the dual-write diff surfaces this immediately.

use pylon_auth::AuthContext;
use pylon_kernel::{AppManifest, ManifestAuthUserConfig};
use pylon_policy::{PolicyEngine, PolicyResult};
use pylon_sync::{ChangeEvent, ChangeKind};

/// How many events the DO retains for catch-up. A reconnect whose
/// cursor is older than the ring's tail must fall back to the
/// machine's `/api/sync/pull` (deep history stays on the machine).
pub const RING_CAPACITY: usize = 1024;

// ---------------------------------------------------------------------------
// Filter
// ---------------------------------------------------------------------------

pub struct RelayFilter {
    manifest: AppManifest,
    auth_user: ManifestAuthUserConfig,
    engine: PolicyEngine,
}

impl RelayFilter {
    /// Build from the machine's boot-time manifest push:
    /// `{"manifest": AppManifest, "auth_user": ManifestAuthUserConfig}`.
    pub fn from_manifest_payload(payload: &str) -> Result<Self, String> {
        #[derive(serde::Deserialize)]
        struct Payload {
            manifest: AppManifest,
            #[serde(default)]
            auth_user: Option<ManifestAuthUserConfig>,
        }
        let parsed: Payload =
            serde_json::from_str(payload).map_err(|e| format!("manifest payload: {e}"))?;
        let engine = PolicyEngine::from_manifest(&parsed.manifest);
        let auth_user = parsed
            .auth_user
            .unwrap_or_else(|| parsed.manifest.auth.user.clone());
        Ok(Self {
            manifest: parsed.manifest,
            auth_user,
            engine,
        })
    }

    fn project(&self, entity: &str, row: Option<serde_json::Value>) -> Option<serde_json::Value> {
        pylon_router::project_row_for_replication_opt(&self.manifest, &self.auth_user, entity, row)
    }

    fn projected_wire(&self, event: &ChangeEvent) -> Option<String> {
        let wire = ChangeEvent {
            data: self.project(&event.entity, event.data.clone()),
            prev_data: None,
            ..event.clone()
        };
        serde_json::to_string(&wire).ok()
    }

    fn tombstone_wire(&self, event: &ChangeEvent) -> Option<String> {
        let wire = ChangeEvent {
            seq: event.seq,
            entity: event.entity.clone(),
            row_id: event.row_id.clone(),
            kind: ChangeKind::Delete,
            data: self.project(&event.entity, event.prev_data.clone()),
            prev_data: None,
            timestamp: event.timestamp.clone(),
        };
        serde_json::to_string(&wire).ok()
    }

    /// The frame `auth` should receive for `event`, or `None` when the
    /// event is invisible to them. The input event carries RAW
    /// `data`/`prev_data` (as pushed by the machine's sink); the output
    /// JSON is projected and `prev_data`-stripped.
    pub fn wire_json_for(&self, auth: &AuthContext, event: &ChangeEvent) -> Option<String> {
        if auth.is_unscoped_admin() {
            return self.projected_wire(event);
        }
        let post_allowed = matches!(
            self.engine
                .check_entity_read(&event.entity, auth, event.data.as_ref()),
            PolicyResult::Allowed
        );
        if post_allowed {
            return self.projected_wire(event);
        }
        if matches!(event.kind, ChangeKind::Update) && event.prev_data.is_some() {
            let pre_allowed = matches!(
                self.engine
                    .check_entity_read(&event.entity, auth, event.prev_data.as_ref()),
                PolicyResult::Allowed
            );
            if pre_allowed {
                return self.tombstone_wire(event);
            }
        }
        None
    }
}

// ---------------------------------------------------------------------------
// Ring
// ---------------------------------------------------------------------------

/// Bounded, seq-ordered recent-event buffer. The DO half persists each
/// accepted event to DO storage under `e:{seq:020}` and prunes in step
/// with [`EventRing::push`]'s eviction.
#[derive(Default)]
pub struct EventRing {
    events: std::collections::VecDeque<ChangeEvent>,
}

impl EventRing {
    pub fn hydrate(events: Vec<ChangeEvent>) -> Self {
        let mut events = events;
        events.sort_by_key(|e| e.seq);
        let mut ring = Self {
            events: events.into(),
        };
        while ring.events.len() > RING_CAPACITY {
            ring.events.pop_front();
        }
        ring
    }

    pub fn latest_seq(&self) -> u64 {
        self.events.back().map(|e| e.seq).unwrap_or(0)
    }

    pub fn oldest_seq(&self) -> u64 {
        self.events.front().map(|e| e.seq).unwrap_or(0)
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Accept an event; returns evicted seqs the caller must delete
    /// from storage, or `None` when the event is a duplicate or has
    /// already fallen below the ring floor.
    ///
    /// Inserts in seq order rather than assuming append order. In
    /// cluster mode seqs come from a shared Postgres SEQUENCE allocated
    /// interleaved across machines, and each machine's sink pushes only
    /// its own events — so machine B's seq 6 can arrive before machine
    /// A's seq 4. A `seq <= latest` reject rule would silently drop
    /// those. Dedup is by membership (already present, or below the
    /// evicted floor), not by comparison to the tail.
    pub fn push(&mut self, event: ChangeEvent) -> Option<Vec<u64>> {
        // Below the floor: the client that needs this seq is already
        // being sent to /api/sync/pull, so re-admitting it would just
        // churn storage. Only reject once the ring is full — before
        // that oldest_seq() is 0 and everything is in-window.
        if self.events.len() >= RING_CAPACITY && event.seq < self.oldest_seq() {
            return None;
        }
        // Duplicate (machine retry re-push, or a peer echo).
        if self.events.iter().any(|e| e.seq == event.seq) {
            return None;
        }
        // Insert in seq order. Newest-at-back is the common case
        // (append), so scan from the back.
        let pos = self
            .events
            .iter()
            .rposition(|e| e.seq < event.seq)
            .map(|i| i + 1)
            .unwrap_or(0);
        self.events.insert(pos, event);
        let mut evicted = Vec::new();
        while self.events.len() > RING_CAPACITY {
            if let Some(old) = self.events.pop_front() {
                evicted.push(old.seq);
            }
        }
        Some(evicted)
    }

    /// Events strictly after `since`, oldest first. `None` when the
    /// cursor pre-dates the ring (caller must direct the client to the
    /// machine's `/api/sync/pull` instead — deep history is not ours).
    pub fn since(&self, since: u64) -> Option<Vec<&ChangeEvent>> {
        if self.events.is_empty() {
            return Some(Vec::new());
        }
        // A cursor older than (oldest - 1) has a gap the ring can't
        // cover. oldest-1 is fine: the client already has `oldest-1`.
        if since + 1 < self.oldest_seq() {
            return None;
        }
        Some(self.events.iter().filter(|e| e.seq > since).collect())
    }
}

/// Storage key for a ring event — zero-padded so lexicographic order
/// equals numeric order for prefix listing.
pub fn event_key(seq: u64) -> String {
    format!("e:{seq:020}")
}

pub fn parse_event_key(key: &str) -> Option<u64> {
    key.strip_prefix("e:")?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(name: &str) -> pylon_kernel::ManifestField {
        pylon_kernel::ManifestField {
            name: name.into(),
            field_type: "string".into(),
            ..Default::default()
        }
    }

    fn manifest_payload() -> String {
        let mut owner = field("ownerId");
        owner.server_only = true;
        let mut blob = field("blob");
        blob.sync_omit = true;
        let manifest = AppManifest {
            manifest_version: 1,
            name: "relaytest".into(),
            version: "0".into(),
            entities: vec![pylon_kernel::ManifestEntity {
                name: "Note".into(),
                fields: vec![field("title"), owner, blob],
                ..Default::default()
            }],
            policies: vec![pylon_kernel::ManifestPolicy {
                name: "own-notes".into(),
                entity: Some("Note".into()),
                allow_read: Some("auth.userId != null && data.ownerId == auth.userId".into()),
                ..Default::default()
            }],
            ..Default::default()
        };
        serde_json::json!({ "manifest": manifest }).to_string()
    }

    fn note_event(
        kind: ChangeKind,
        data: serde_json::Value,
        prev: Option<serde_json::Value>,
    ) -> ChangeEvent {
        ChangeEvent {
            seq: 10,
            entity: "Note".into(),
            row_id: "n1".into(),
            kind,
            data: Some(data),
            prev_data: prev,
            timestamp: "2026-01-01T00:00:00Z".into(),
        }
    }

    fn user(id: &str) -> AuthContext {
        AuthContext::authenticated(id.into())
    }

    #[test]
    fn owner_receives_projected_frame_stranger_receives_nothing() {
        let filter = RelayFilter::from_manifest_payload(&manifest_payload()).unwrap();
        let ev = note_event(
            ChangeKind::Insert,
            serde_json::json!({"title": "hi", "ownerId": "u1", "blob": "big"}),
            None,
        );
        let frame = filter
            .wire_json_for(&user("u1"), &ev)
            .expect("owner sees frame");
        let parsed: serde_json::Value = serde_json::from_str(&frame).unwrap();
        // Policy evaluated on the RAW row (ownerId is serverOnly), but
        // the wire frame is projected: serverOnly + syncOmit stripped.
        assert_eq!(parsed["data"]["title"], "hi");
        assert!(parsed["data"].get("ownerId").is_none(), "serverOnly leaked");
        assert!(parsed["data"].get("blob").is_none(), "syncOmit leaked");
        assert!(parsed.get("prev_data").is_none());

        assert!(filter.wire_json_for(&user("u2"), &ev).is_none());
        assert!(filter
            .wire_json_for(&AuthContext::anonymous(), &ev)
            .is_none());
    }

    #[test]
    fn unscoped_admin_bypasses_scoped_admin_does_not() {
        let filter = RelayFilter::from_manifest_payload(&manifest_payload()).unwrap();
        let ev = note_event(
            ChangeKind::Insert,
            serde_json::json!({"title": "hi", "ownerId": "u1"}),
            None,
        );
        let mut admin = AuthContext::anonymous();
        admin.is_admin = true;
        assert!(filter.wire_json_for(&admin, &ev).is_some());
        // Admin with an active tenant is scoped like a member on reads
        // (mirrors PolicyEngine::check_entity + is_unscoped_admin).
        admin.tenant_id = Some("org1".into());
        assert!(filter.wire_json_for(&admin, &ev).is_none());
    }

    #[test]
    fn ownership_transfer_synthesizes_projected_tombstone() {
        let filter = RelayFilter::from_manifest_payload(&manifest_payload()).unwrap();
        let ev = note_event(
            ChangeKind::Update,
            serde_json::json!({"title": "hi", "ownerId": "u2"}),
            Some(serde_json::json!({"title": "hi", "ownerId": "u1", "blob": "big"})),
        );
        // Old owner: post denied, pre allowed → Delete at same seq.
        let frame = filter.wire_json_for(&user("u1"), &ev).expect("tombstone");
        let parsed: serde_json::Value = serde_json::from_str(&frame).unwrap();
        assert_eq!(parsed["kind"], "delete");
        assert_eq!(parsed["seq"], 10);
        assert!(
            parsed["data"].get("ownerId").is_none(),
            "tombstone leaked serverOnly"
        );
        // New owner sees the update; a stranger sees nothing.
        let update = filter.wire_json_for(&user("u2"), &ev).unwrap();
        assert!(update.contains("\"kind\":\"update\""));
        assert!(filter.wire_json_for(&user("u3"), &ev).is_none());
    }

    #[test]
    fn no_policy_means_default_deny() {
        // Entity without a policy: PolicyEngine default-denies, so the
        // relay ships nothing — same as the machine's WS hub.
        let manifest = AppManifest {
            manifest_version: 1,
            name: "t".into(),
            version: "0".into(),
            entities: vec![pylon_kernel::ManifestEntity {
                name: "Bare".into(),
                fields: vec![field("x")],
                ..Default::default()
            }],
            ..Default::default()
        };
        let payload = serde_json::json!({ "manifest": manifest }).to_string();
        let filter = RelayFilter::from_manifest_payload(&payload).unwrap();
        let ev = ChangeEvent {
            seq: 1,
            entity: "Bare".into(),
            row_id: "b1".into(),
            kind: ChangeKind::Insert,
            data: Some(serde_json::json!({"x": "y"})),
            prev_data: None,
            timestamp: "t".into(),
        };
        assert!(filter.wire_json_for(&user("u1"), &ev).is_none());
    }

    #[test]
    fn ring_dedups_evicts_and_serves_since() {
        let mut ring = EventRing::default();
        for seq in 1..=5u64 {
            let mut ev = note_event(ChangeKind::Insert, serde_json::json!({}), None);
            ev.seq = seq;
            assert!(ring.push(ev).is_some());
        }
        // Duplicate / stale seqs are rejected (machine retry re-push).
        let mut dup = note_event(ChangeKind::Insert, serde_json::json!({}), None);
        dup.seq = 5;
        assert!(ring.push(dup).is_none());
        assert_eq!(ring.latest_seq(), 5);

        let tail: Vec<u64> = ring.since(3).unwrap().iter().map(|e| e.seq).collect();
        assert_eq!(tail, vec![4, 5]);
        assert!(ring.since(5).unwrap().is_empty());
        // since=0 with oldest=1 is coverable (client has nothing, ring
        // has everything from seq 1).
        assert_eq!(ring.since(0).unwrap().len(), 5);
    }

    #[test]
    fn interleaved_cluster_seqs_are_kept_in_order() {
        // Cluster mode: machine B's batch (seq 6) can land before
        // machine A's 4,5. An old `seq <= latest` reject would drop
        // them; membership-dedup + sorted insert keeps everything.
        let mut ring = EventRing::default();
        for seq in [1u64, 2, 3, 6, 4, 5] {
            let mut ev = note_event(ChangeKind::Insert, serde_json::json!({}), None);
            ev.seq = seq;
            assert!(ring.push(ev).is_some(), "seq {seq} must be accepted");
        }
        let all: Vec<u64> = ring.since(0).unwrap().iter().map(|e| e.seq).collect();
        assert_eq!(all, vec![1, 2, 3, 4, 5, 6], "kept and re-sorted");
        // A later re-push of an already-held seq is still a dup.
        let mut dup = note_event(ChangeKind::Insert, serde_json::json!({}), None);
        dup.seq = 4;
        assert!(ring.push(dup).is_none());
        // since past the middle returns only the strictly-greater tail.
        assert_eq!(
            ring.since(4)
                .unwrap()
                .iter()
                .map(|e| e.seq)
                .collect::<Vec<_>>(),
            vec![5, 6]
        );
    }

    #[test]
    fn ring_reports_uncoverable_cursor() {
        let mut ring = EventRing::default();
        for seq in 100..=110u64 {
            let mut ev = note_event(ChangeKind::Insert, serde_json::json!({}), None);
            ev.seq = seq;
            ring.push(ev);
        }
        assert!(ring.since(50).is_none(), "gap below the ring tail");
        assert!(ring.since(99).is_some(), "client at oldest-1 is covered");
    }

    #[test]
    fn ring_capacity_evicts_and_reports_keys() {
        let mut ring = EventRing::default();
        let mut evicted_total = 0;
        for seq in 1..=(RING_CAPACITY as u64 + 10) {
            let mut ev = note_event(ChangeKind::Insert, serde_json::json!({}), None);
            ev.seq = seq;
            evicted_total += ring.push(ev).unwrap().len();
        }
        assert_eq!(ring.len(), RING_CAPACITY);
        assert_eq!(evicted_total, 10);
        assert_eq!(ring.oldest_seq(), 11);
    }

    #[test]
    fn event_key_orders_lexicographically() {
        assert!(event_key(2) < event_key(10));
        assert_eq!(parse_event_key(&event_key(42)), Some(42));
    }
}
