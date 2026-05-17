//! Reactive query subscriptions — Convex-style auto-rerunning handlers.
//!
//! A `query()` handler that's mounted via `useReactiveQuery(fnName, args)`
//! on the client gets registered here. During the handler's first run
//! the function runner records every entity (and row id, when known)
//! it touched via `ctx.db.*`. We store that dep set alongside the
//! subscription. On every subsequent change event, we look up which
//! subscriptions depend on the touched entity / row, re-run their
//! handlers under the original subscriber's auth, hash the result,
//! and push it to the subscribed client if the hash changed.
//!
//! ## Why this lives in the runtime
//!
//! Subscriptions need three things to compose:
//!  - `FnOps` to re-invoke the handler.
//!  - `WsHub` to push the new result to the right client.
//!  - The change-event stream (already routed through `WsSseNotifier`).
//! All three live in the runtime, so this module bridges them.
//!
//! ## Auth isolation across re-runs
//!
//! Re-runs use the auth context captured at subscription time, NOT
//! the auth of whoever wrote the mutation that triggered the re-run.
//! That's the only way the handler can run the same policy + tenant
//! gates it ran originally. A Stripe webhook updating an Org row
//! must NOT cause re-evaluation under the webhook's elevated admin
//! auth — the subscriber's view is the subscriber's auth.
//!
//! ## Coalescing
//!
//! Multiple change events touching the same subscription within one
//! tick coalesce into a single re-run. The dirty set buffers
//! sub_ids; a dedicated re-runner thread drains it on a short
//! cadence (16ms default — matches a 60Hz UI frame). Without
//! coalescing, a batch insert of 100 rows would trigger 100 re-runs
//! per subscriber.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use pylon_functions::deps::DepSet;
use pylon_functions::protocol::AuthInfo;
use pylon_sync::{ChangeEvent, ChangeKind};

use crate::ws::WsHub;

/// One reactive subscription. Cheap to clone (Arc'd at the registry
/// layer, not here) but we keep ownership simple by storing by value.
#[derive(Clone)]
struct Subscription {
    sub_id: String,
    fn_name: String,
    args: serde_json::Value,
    auth: AuthInfo,
    /// WsHub client id of the subscriber. Used by the re-runner to
    /// push the new result back to the right socket.
    client_id: u64,
    deps: DepSet,
    /// Hash of the last result we sent. Skip the push if the new
    /// result hashes the same — saves a frame on every change that
    /// doesn't actually move the handler's output.
    last_hash: u64,
}

/// Per-subscription state + indexes for fast change-event matching.
///
/// Three locks (we never hold more than one at a time to dodge
/// deadlocks):
///  - `subs` — sub_id → Subscription
///  - `by_entity` — entity → Set<sub_id>
///  - `by_row` — (entity, row_id) → Set<sub_id>
///  - `by_client` — client_id → Set<sub_id> (so disconnect can clean
///    up every sub for a client in one pass)
///  - `dirty` — pending re-runs, drained by the re-runner thread
pub struct ReactiveRegistry {
    inner: Mutex<RegistryInner>,
    /// Wake the re-runner thread when the dirty set transitions
    /// from empty → non-empty. The thread blocks on this condvar
    /// when there's no work — no busy-loop.
    dirty_notify: Condvar,
    fn_ops: Mutex<Option<Arc<dyn pylon_router::FnOps>>>,
    ws_hub: Arc<WsHub>,
    runner_started: AtomicBool,
}

struct RegistryInner {
    subs: HashMap<String, Subscription>,
    by_entity: HashMap<String, HashSet<String>>,
    by_row: HashMap<(String, String), HashSet<String>>,
    by_client: HashMap<u64, HashSet<String>>,
    /// Sub_ids waiting to be re-run. VecDeque so we drain in
    /// insertion order — slightly fairer when one chatty entity
    /// dominates the bus.
    dirty: VecDeque<String>,
    /// Dedup the dirty queue: a sub that's already pending doesn't
    /// get enqueued twice in the same tick.
    pending: HashSet<String>,
}

/// Result of running a reactive handler for a fresh subscription or
/// a re-run. Encapsulates the value the runtime needs to push and
/// the dep set the registry needs to store.
pub struct ReactiveOutcome {
    pub value: serde_json::Value,
    pub deps: DepSet,
    pub hash: u64,
}

impl ReactiveRegistry {
    pub fn new(ws_hub: Arc<WsHub>) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(RegistryInner {
                subs: HashMap::new(),
                by_entity: HashMap::new(),
                by_row: HashMap::new(),
                by_client: HashMap::new(),
                dirty: VecDeque::new(),
                pending: HashSet::new(),
            }),
            dirty_notify: Condvar::new(),
            fn_ops: Mutex::new(None),
            ws_hub,
            runner_started: AtomicBool::new(false),
        })
    }

    /// Late-binding for `FnOps` — the function runtime is started
    /// after the registry (the registry feeds into the notifier which
    /// the runtime constructs). Caller wires this once at boot.
    pub fn set_fn_ops(&self, fn_ops: Arc<dyn pylon_router::FnOps>) {
        *self.fn_ops.lock().unwrap() = Some(fn_ops);
    }

    /// Spawn the re-runner thread. Idempotent — calling twice does
    /// nothing. Called once from `start_server` after the registry is
    /// constructed and `set_fn_ops` is wired.
    pub fn start_runner(self: &Arc<Self>) {
        if self.runner_started.swap(true, Ordering::SeqCst) {
            return;
        }
        let me = Arc::clone(self);
        thread::Builder::new()
            .name("pylon-reactive-rerunner".into())
            .spawn(move || me.runner_loop())
            .expect("spawn reactive re-runner");
    }

    /// Run a query handler with dep tracking active. Used by the
    /// initial subscribe path AND the re-runner.
    ///
    /// Returns None if the runner reports an error — caller decides
    /// whether to surface that to the client or just skip the push
    /// (re-runs swallow errors so a temporarily-failing handler
    /// doesn't keep firing error messages at the UI).
    pub fn run_handler(
        &self,
        fn_name: &str,
        args: serde_json::Value,
        auth: AuthInfo,
    ) -> Option<ReactiveOutcome> {
        let fn_ops = {
            let guard = self.fn_ops.lock().unwrap();
            guard.as_ref().map(Arc::clone)
        };
        let fn_ops = fn_ops?;
        let guard = pylon_functions::deps::enter();
        let result = fn_ops.call(fn_name, args, auth, None, None);
        let deps = guard.take();
        match result {
            Ok((value, _trace)) => {
                let hash = hash_value(&value);
                Some(ReactiveOutcome { value, deps, hash })
            }
            Err(e) => {
                tracing::warn!(
                    "[reactive] handler {fn_name} failed: {} {}",
                    e.code,
                    e.message
                );
                None
            }
        }
    }

    /// Register a new subscription and index its deps. Caller must
    /// have just produced `outcome` from [`run_handler`].
    pub fn register(
        &self,
        sub_id: String,
        fn_name: String,
        args: serde_json::Value,
        auth: AuthInfo,
        client_id: u64,
        outcome: &ReactiveOutcome,
    ) {
        let mut inner = self.inner.lock().unwrap();
        // If a sub with this id already exists (client retry), drop
        // the old indexes before overwriting.
        if inner.subs.contains_key(&sub_id) {
            remove_locked(&mut inner, &sub_id);
        }
        let sub = Subscription {
            sub_id: sub_id.clone(),
            fn_name,
            args,
            auth,
            client_id,
            deps: outcome.deps.clone(),
            last_hash: outcome.hash,
        };
        index_locked(&mut inner, &sub);
        inner.subs.insert(sub_id, sub);
    }

    pub fn unsubscribe(&self, sub_id: &str) {
        let mut inner = self.inner.lock().unwrap();
        remove_locked(&mut inner, sub_id);
    }

    /// Tear down every subscription owned by `client_id`. Called from
    /// the WS handler when the connection closes — without this,
    /// re-runs would keep firing for a socket nobody reads.
    pub fn disconnect_client(&self, client_id: u64) {
        let mut inner = self.inner.lock().unwrap();
        let sub_ids: Vec<String> = inner
            .by_client
            .get(&client_id)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default();
        for sub_id in sub_ids {
            remove_locked(&mut inner, &sub_id);
        }
    }

    /// Called by `WsSseNotifier::notify` on every change event. Finds
    /// subs whose deps overlap and marks them dirty for re-run.
    pub fn on_change(&self, event: &ChangeEvent) {
        let mut inner = self.inner.lock().unwrap();
        let mut to_mark: Vec<String> = Vec::new();

        // Row-level matches: exact (entity, row_id) intersection.
        // Subscriptions with empty `rows` (entity-only, or row cap
        // blown) fall through to the entity index below — we don't
        // suppress entity matches when row matches are empty.
        if let Some(row_subs) = inner
            .by_row
            .get(&(event.entity.clone(), event.row_id.clone()))
        {
            for sid in row_subs {
                to_mark.push(sid.clone());
            }
        }

        // Entity-level matches: only subs that opted into
        // entity-only mode (no precise row deps) get matched here.
        // Subs with precise row deps that DIDN'T cover this row
        // shouldn't re-run on changes to other rows.
        if let Some(entity_subs) = inner.by_entity.get(&event.entity) {
            // Snapshot to drop the borrow before mutating dirty set.
            let candidates: Vec<String> = entity_subs.iter().cloned().collect();
            for sid in candidates {
                if let Some(sub) = inner.subs.get(&sid) {
                    if sub.deps.entity_only() {
                        to_mark.push(sid);
                    } else {
                        // Sub has precise row deps; skip unless the
                        // change touches a row in its read set, which
                        // the by_row branch above already handled.
                        // Special-case: delete events for entities
                        // the sub LISTED via `ctx.db.list/query` (no
                        // row id read) need to dirty too — the sub's
                        // row set wouldn't include the deleted row,
                        // and `entity_only` is false because some
                        // other path read individual rows. Treat any
                        // entity in the dep set as a match for
                        // delete events to stay safe.
                        if matches!(event.kind, ChangeKind::Delete) {
                            to_mark.push(sid);
                        }
                    }
                }
            }
        }

        if to_mark.is_empty() {
            return;
        }
        let mut newly_dirty = false;
        for sid in to_mark {
            if inner.pending.insert(sid.clone()) {
                inner.dirty.push_back(sid);
                newly_dirty = true;
            }
        }
        if newly_dirty {
            self.dirty_notify.notify_one();
        }
    }

    /// Re-runner thread body. Sleeps on the condvar until something
    /// goes dirty, then drains the dirty queue, re-running each sub
    /// and pushing changed results.
    fn runner_loop(self: Arc<Self>) {
        loop {
            // Take a snapshot of dirty work + their sub specs. Run
            // outside the lock so FnOps::call doesn't block the
            // change-event hot path.
            let batch: Vec<Subscription> = {
                let mut inner = self.inner.lock().unwrap();
                while inner.dirty.is_empty() {
                    inner = self
                        .dirty_notify
                        .wait_timeout(inner, Duration::from_secs(5))
                        .unwrap()
                        .0;
                }
                // Coalesce: drain at most N at a time so a flood
                // doesn't starve fresh subscribes. 64 is enough for
                // any realistic per-tick burst.
                let mut take = Vec::new();
                for _ in 0..64 {
                    match inner.dirty.pop_front() {
                        Some(sid) => {
                            inner.pending.remove(&sid);
                            if let Some(sub) = inner.subs.get(&sid).cloned() {
                                take.push(sub);
                            }
                        }
                        None => break,
                    }
                }
                take
            };

            for sub in batch {
                let outcome = self.run_handler(&sub.fn_name, sub.args.clone(), sub.auth.clone());
                let Some(outcome) = outcome else {
                    continue;
                };
                if outcome.hash == sub.last_hash {
                    // No-op for the client. Update deps in case the
                    // handler's reads shifted (rare; happens when the
                    // result is value-equal but the handler took a
                    // different code path).
                    self.update_deps(&sub.sub_id, &outcome);
                    continue;
                }
                self.push_result(&sub.sub_id, &outcome.value, sub.client_id);
                self.update_deps_and_hash(&sub.sub_id, &outcome);
            }
        }
    }

    fn update_deps(&self, sub_id: &str, outcome: &ReactiveOutcome) {
        let mut inner = self.inner.lock().unwrap();
        let Some(sub_cloned) = inner.subs.get(sub_id).cloned() else {
            return;
        };
        // Remove old indexes, then re-index with new deps.
        remove_locked(&mut inner, sub_id);
        let new_sub = Subscription {
            deps: outcome.deps.clone(),
            ..sub_cloned
        };
        index_locked(&mut inner, &new_sub);
        inner.subs.insert(sub_id.to_string(), new_sub);
    }

    fn update_deps_and_hash(&self, sub_id: &str, outcome: &ReactiveOutcome) {
        let mut inner = self.inner.lock().unwrap();
        let Some(sub_cloned) = inner.subs.get(sub_id).cloned() else {
            return;
        };
        remove_locked(&mut inner, sub_id);
        let new_sub = Subscription {
            deps: outcome.deps.clone(),
            last_hash: outcome.hash,
            ..sub_cloned
        };
        index_locked(&mut inner, &new_sub);
        inner.subs.insert(sub_id.to_string(), new_sub);
    }

    fn push_result(&self, sub_id: &str, value: &serde_json::Value, client_id: u64) {
        let frame = serde_json::json!({
            "type": "reactive-result",
            "sub_id": sub_id,
            "result": value,
        })
        .to_string();
        self.ws_hub.send_text_to(client_id, &frame);
    }

    /// Count subs — diagnostic.
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().subs.len()
    }

    /// Whether anything is registered — diagnostic.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

fn index_locked(inner: &mut RegistryInner, sub: &Subscription) {
    for entity in &sub.deps.entities {
        inner
            .by_entity
            .entry(entity.clone())
            .or_default()
            .insert(sub.sub_id.clone());
    }
    for row in &sub.deps.rows {
        inner
            .by_row
            .entry(row.clone())
            .or_default()
            .insert(sub.sub_id.clone());
    }
    inner
        .by_client
        .entry(sub.client_id)
        .or_default()
        .insert(sub.sub_id.clone());
}

fn remove_locked(inner: &mut RegistryInner, sub_id: &str) {
    let Some(sub) = inner.subs.remove(sub_id) else {
        return;
    };
    for entity in &sub.deps.entities {
        if let Some(s) = inner.by_entity.get_mut(entity) {
            s.remove(sub_id);
            if s.is_empty() {
                inner.by_entity.remove(entity);
            }
        }
    }
    for row in &sub.deps.rows {
        if let Some(s) = inner.by_row.get_mut(row) {
            s.remove(sub_id);
            if s.is_empty() {
                inner.by_row.remove(row);
            }
        }
    }
    if let Some(s) = inner.by_client.get_mut(&sub.client_id) {
        s.remove(sub_id);
        if s.is_empty() {
            inner.by_client.remove(&sub.client_id);
        }
    }
    inner.pending.remove(sub_id);
    // dirty VecDeque is allowed to retain a stale entry — when the
    // runner pops it the subs lookup returns None and we skip. Cheaper
    // than O(n) scanning the deque on every unsubscribe.
}

fn hash_value(value: &serde_json::Value) -> u64 {
    use std::hash::Hasher;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    // Serialize via the standard `to_string` — produces deterministic
    // output for the same value (object key order is preserved by
    // serde_json's Map). Slightly more expensive than walking the
    // tree, but correct under all the corner cases a hand-rolled
    // hash would have to enumerate.
    let s = value.to_string();
    h.write(s.as_bytes());
    h.finish()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use pylon_policy::PolicyEngine;

    fn make_hub() -> Arc<WsHub> {
        let manifest = pylon_kernel::AppManifest::default();
        WsHub::new(Arc::new(PolicyEngine::from_manifest(&manifest)))
    }

    fn dep_set(entities: &[&str], rows: &[(&str, &str)]) -> DepSet {
        let mut d = DepSet::new();
        for e in entities {
            d.entities.insert((*e).to_string());
        }
        for (e, r) in rows {
            d.rows.insert(((*e).to_string(), (*r).to_string()));
        }
        d
    }

    fn make_outcome(deps: DepSet, value: serde_json::Value) -> ReactiveOutcome {
        let hash = hash_value(&value);
        ReactiveOutcome { value, deps, hash }
    }

    fn make_auth() -> AuthInfo {
        AuthInfo {
            user_id: Some("u1".into()),
            is_admin: false,
            tenant_id: Some("t1".into()),
        }
    }

    fn dummy_change(entity: &str, row_id: &str, kind: ChangeKind) -> ChangeEvent {
        ChangeEvent {
            seq: 1,
            entity: entity.into(),
            row_id: row_id.into(),
            kind,
            data: None,
            timestamp: "".into(),
        }
    }

    #[test]
    fn entity_only_sub_matches_any_row_change() {
        let reg = ReactiveRegistry::new(make_hub());
        let outcome = make_outcome(
            dep_set(&["Recording"], &[]),
            serde_json::json!({"count": 0}),
        );
        reg.register(
            "s1".into(),
            "getCount".into(),
            serde_json::json!({}),
            make_auth(),
            42,
            &outcome,
        );
        reg.on_change(&dummy_change("Recording", "r_99", ChangeKind::Insert));
        let inner = reg.inner.lock().unwrap();
        assert!(inner.pending.contains("s1"));
    }

    #[test]
    fn precise_row_sub_skips_unrelated_row_change() {
        let reg = ReactiveRegistry::new(make_hub());
        let outcome = make_outcome(
            dep_set(&["Recording"], &[("Recording", "r_1")]),
            serde_json::json!({}),
        );
        reg.register(
            "s1".into(),
            "getOne".into(),
            serde_json::json!({"id": "r_1"}),
            make_auth(),
            42,
            &outcome,
        );
        reg.on_change(&dummy_change("Recording", "r_2", ChangeKind::Update));
        let inner = reg.inner.lock().unwrap();
        assert!(
            !inner.pending.contains("s1"),
            "row-precise sub should not fire on unrelated row update"
        );
    }

    #[test]
    fn precise_row_sub_fires_for_matching_row() {
        let reg = ReactiveRegistry::new(make_hub());
        let outcome = make_outcome(
            dep_set(&["Recording"], &[("Recording", "r_1")]),
            serde_json::json!({}),
        );
        reg.register(
            "s1".into(),
            "getOne".into(),
            serde_json::json!({"id": "r_1"}),
            make_auth(),
            42,
            &outcome,
        );
        reg.on_change(&dummy_change("Recording", "r_1", ChangeKind::Update));
        let inner = reg.inner.lock().unwrap();
        assert!(inner.pending.contains("s1"));
    }

    #[test]
    fn delete_fires_precise_sub_even_for_unread_row() {
        // A handler that listed Org rows (no row ids read) needs to
        // re-run when an Org is deleted, even though its dep set has
        // no precise row entries. Verified via the
        // ChangeKind::Delete special case in on_change.
        let reg = ReactiveRegistry::new(make_hub());
        let outcome = make_outcome(
            dep_set(&["Recording"], &[("Recording", "r_a")]),
            serde_json::json!({}),
        );
        reg.register(
            "s1".into(),
            "getAll".into(),
            serde_json::json!({}),
            make_auth(),
            42,
            &outcome,
        );
        reg.on_change(&dummy_change("Recording", "r_b", ChangeKind::Delete));
        let inner = reg.inner.lock().unwrap();
        assert!(
            inner.pending.contains("s1"),
            "delete should dirty precise subs that didn't read the deleted row"
        );
    }

    #[test]
    fn unsubscribe_removes_indexes() {
        let reg = ReactiveRegistry::new(make_hub());
        let outcome = make_outcome(
            dep_set(&["Recording", "Org"], &[("Recording", "r_1")]),
            serde_json::json!({}),
        );
        reg.register(
            "s1".into(),
            "getOne".into(),
            serde_json::json!({}),
            make_auth(),
            42,
            &outcome,
        );
        assert_eq!(reg.len(), 1);
        reg.unsubscribe("s1");
        let inner = reg.inner.lock().unwrap();
        assert_eq!(inner.subs.len(), 0);
        assert!(inner.by_entity.is_empty());
        assert!(inner.by_row.is_empty());
        assert!(inner.by_client.is_empty());
    }

    #[test]
    fn disconnect_client_tears_down_all_their_subs() {
        let reg = ReactiveRegistry::new(make_hub());
        for i in 0..5 {
            let outcome = make_outcome(dep_set(&["Recording"], &[]), serde_json::json!({"i": i}));
            reg.register(
                format!("s{i}"),
                "f".into(),
                serde_json::json!({"i": i}),
                make_auth(),
                42,
                &outcome,
            );
        }
        // Also register one sub for a different client — it should
        // survive the disconnect.
        let outcome = make_outcome(
            dep_set(&["Recording"], &[]),
            serde_json::json!({"other": true}),
        );
        reg.register(
            "other".into(),
            "f".into(),
            serde_json::json!({}),
            make_auth(),
            99,
            &outcome,
        );
        assert_eq!(reg.len(), 6);
        reg.disconnect_client(42);
        assert_eq!(reg.len(), 1);
        assert!(reg.inner.lock().unwrap().subs.contains_key("other"));
    }

    #[test]
    fn dedupe_pending_dirties_only_enqueue_once() {
        let reg = ReactiveRegistry::new(make_hub());
        let outcome = make_outcome(dep_set(&["Recording"], &[]), serde_json::json!({}));
        reg.register(
            "s1".into(),
            "f".into(),
            serde_json::json!({}),
            make_auth(),
            42,
            &outcome,
        );
        // Trigger three changes back-to-back; pending set should
        // dedupe so the dirty deque only holds one entry.
        reg.on_change(&dummy_change("Recording", "r_1", ChangeKind::Insert));
        reg.on_change(&dummy_change("Recording", "r_2", ChangeKind::Update));
        reg.on_change(&dummy_change("Recording", "r_3", ChangeKind::Insert));
        let inner = reg.inner.lock().unwrap();
        assert_eq!(inner.dirty.len(), 1);
        assert_eq!(inner.pending.len(), 1);
    }
}
