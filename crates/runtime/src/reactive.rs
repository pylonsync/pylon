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
//! ## Client-scoped subscription keys
//!
//! `sub_id` is client-minted. Two clients picking the same id
//! (browser back-button, two tabs, app bugs) would collide if we
//! keyed `subs` by sub_id alone — the second register overwrites
//! the first; the first client's re-runs push to the second client's
//! socket; data leaks across sessions. Internal keys are
//! `(client_id, sub_id)` tuples; the protocol still exposes sub_id
//! to the client.
//!
//! ## Off-WS-thread handler execution
//!
//! Both the initial subscribe run AND every re-run go through the
//! single re-runner thread. The WS reader thread NEVER blocks on
//! `fn_ops.call`. Without that discipline, a slow handler (a query
//! touching 10k rows, a TS handler doing `await` against an external
//! API) would hold the per-client socket mutex for the duration of
//! the call — ping/pong stalls, broadcast back-pressure breaks,
//! and the client looks frozen.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use pylon_functions::deps::DepSet;
use pylon_functions::protocol::AuthInfo;
use pylon_sync::{ChangeEvent, ChangeKind};

use crate::ws::WsHub;

/// Internal key. Client-scoped so two clients picking the same
/// `sub_id` don't collide. The protocol still exposes just `sub_id`
/// to the client; we tuple it with `client_id` here.
pub type SubKey = (u64, String);

/// Cap on outstanding subscriptions per client. A buggy client that
/// loops `subscribeReactive` with new sub_ids could otherwise leak
/// registry slots until disconnect. Beyond this cap, new subscribes
/// are refused with a REACTIVE_LIMIT error.
const PER_CLIENT_SUB_CAP: usize = 256;

/// Cap on the dirty queue length. Prevents an explosive
/// change-event burst (1k mutations in a tick) from holding
/// megabytes of pending work indefinitely. Once exceeded we drop
/// the OLDEST entries — they'll get rescheduled on the next event
/// touching the sub anyway. Bounded staleness rather than
/// unbounded memory.
const DIRTY_QUEUE_CAP: usize = 10_000;

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
    /// Hash of the last result we sent. `None` means "never sent" —
    /// the next run pushes regardless of hash. Distinguishes "first
    /// push pending" from "value happens to hash to 0".
    last_hash: Option<u64>,
    /// Monotonic version bumped on every `register_pending` for this
    /// (client_id, sub_id) pair. The runner snapshots this with the
    /// sub before invoking the handler and re-checks before pushing /
    /// updating state — if the sub was unsubscribed and re-registered
    /// during the run, the old run's result MUST NOT push to the new
    /// logical sub (it would carry stale deps + stale args + a result
    /// computed for a different question). Without versioning, a
    /// rapid resubscribe (React StrictMode double-effect, hook arg
    /// flip during a slow handler) silently delivers the wrong value.
    version: u64,
}

impl Subscription {
    fn key(&self) -> SubKey {
        (self.client_id, self.sub_id.clone())
    }
}

/// Per-subscription state + indexes for fast change-event matching.
/// All mutations go through the single `inner` mutex — never held
/// across `fn_ops.call` or socket I/O.
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
    subs: HashMap<SubKey, Subscription>,
    by_entity: HashMap<String, HashSet<SubKey>>,
    by_row: HashMap<(String, String), HashSet<SubKey>>,
    by_client: HashMap<u64, HashSet<SubKey>>,
    /// Sub keys waiting to be run (initial or re-run). VecDeque so
    /// we drain in insertion order — slightly fairer when one chatty
    /// entity dominates the bus.
    dirty: VecDeque<SubKey>,
    /// Dedup the dirty queue: a sub that's already pending doesn't
    /// get enqueued twice in the same tick.
    pending: HashSet<SubKey>,
    /// Subs whose handler is CURRENTLY executing on the re-runner
    /// thread. Indexed deps for these subs may be stale (the run is
    /// computing fresh deps right now). on_change MUST dirty any
    /// sub in here regardless of dep match — without that, a write
    /// that lands between "handler reads" and "deps get indexed"
    /// is invisible to the sub forever. The runner clears the entry
    /// in `update_deps_and_hash` once new deps are committed; if a
    /// write dirtied during the run, the next iteration picks it up.
    running: HashSet<SubKey>,
}

/// Result of running a reactive handler. Captured by the re-runner;
/// the registry consumes deps + hash for state updates and pushes
/// `value` to the client.
pub struct ReactiveOutcome {
    pub value: serde_json::Value,
    pub deps: DepSet,
    pub hash: u64,
}

/// Outcome of `register_pending` — exposed so the WS handler can
/// surface back-pressure to the client.
#[derive(Debug, PartialEq, Eq)]
pub enum RegisterOutcome {
    /// Sub registered; initial run is queued on the re-runner.
    Queued,
    /// Per-client cap exceeded. Client should drop the call /
    /// surface an error in the UI.
    OverLimit,
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
                running: HashSet::new(),
            }),
            dirty_notify: Condvar::new(),
            fn_ops: Mutex::new(None),
            ws_hub,
            runner_started: AtomicBool::new(false),
        })
    }

    /// Refresh `tenant_id` on every active reactive subscription whose
    /// captured `auth.user_id` matches. Called by the runtime's
    /// session-changed hook so re-runs after a /api/auth/select-org
    /// don't keep executing handlers under the user's pre-flip
    /// tenant.
    ///
    /// Without this, the codex Wave-3 review caught a real security
    /// gap: `handle_reactive_control` snapshots `AuthInfo` into the
    /// Subscription struct at subscribe time; the re-runner re-uses
    /// that snapshot indefinitely. A user who switched orgs would
    /// keep receiving reactive results computed against the OLD
    /// tenant — leaking rows from the org they left.
    ///
    /// Also dirties every touched sub so the re-runner re-evaluates
    /// the handler under the new identity on the next tick — without
    /// the explicit dirty, the cached `last_hash` would short-circuit
    /// the next change-driven re-run if the result happened to hash
    /// the same as before.
    pub fn update_tenant_for_user(&self, user_id: &str, new_tenant: Option<&str>) -> usize {
        let mut inner = match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        let new_tenant = new_tenant.map(|s| s.to_string());
        let mut updated_keys: Vec<SubKey> = Vec::new();
        for (key, sub) in inner.subs.iter_mut() {
            if sub.auth.user_id.as_deref() != Some(user_id) {
                continue;
            }
            sub.auth.tenant_id = new_tenant.clone();
            updated_keys.push(key.clone());
        }
        let count = updated_keys.len();
        for key in updated_keys {
            if inner.pending.insert(key.clone()) {
                inner.dirty.push_back(key);
            }
        }
        if count > 0 {
            self.dirty_notify.notify_all();
        }
        count
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

    /// Register a subscription with empty deps + queue its initial
    /// run on the re-runner thread. Returns [`RegisterOutcome`] so
    /// the WS handler knows whether to surface a back-pressure
    /// error to the client.
    ///
    /// Off-thread by design: the WS reader thread MUST return to
    /// reading immediately so socket I/O (ping/pong, broadcast
    /// back-pressure) keeps flowing. Without this discipline a slow
    /// handler would hold the per-client socket mutex for the
    /// duration of the call.
    pub fn register_pending(
        &self,
        sub_id: String,
        fn_name: String,
        args: serde_json::Value,
        auth: AuthInfo,
        client_id: u64,
    ) -> RegisterOutcome {
        let key: SubKey = (client_id, sub_id.clone());
        let mut inner = self.inner.lock().unwrap();
        // Per-client cap: bounded by PER_CLIENT_SUB_CAP. Replacing an
        // existing sub doesn't count toward the cap (idempotent
        // re-register). Genuine new subs above the cap get refused.
        if !inner.subs.contains_key(&key) {
            let existing = inner
                .by_client
                .get(&client_id)
                .map(|s| s.len())
                .unwrap_or(0);
            if existing >= PER_CLIENT_SUB_CAP {
                return RegisterOutcome::OverLimit;
            }
        }
        // Bump version on every register so any in-flight run for
        // the old logical sub is detected + discarded before its
        // result lands. Without this, a rapid resubscribe (React
        // arg-flip during a slow handler) silently delivers the
        // OLD sub's result to the NEW sub's React subscriber.
        let prev_version = inner.subs.get(&key).map(|s| s.version).unwrap_or(0);
        if inner.subs.contains_key(&key) {
            remove_locked(&mut inner, &key);
        }
        let sub = Subscription {
            sub_id,
            fn_name,
            args,
            auth,
            client_id,
            deps: DepSet::new(),
            last_hash: None,
            version: prev_version.wrapping_add(1),
        };
        // Index by_client immediately so disconnect cleanup works
        // before the first run lands. by_entity / by_row stay empty
        // until the first run captures deps.
        inner
            .by_client
            .entry(client_id)
            .or_default()
            .insert(key.clone());
        inner.subs.insert(key.clone(), sub);
        enqueue_dirty_locked(&mut inner, key);
        self.dirty_notify.notify_one();
        RegisterOutcome::Queued
    }

    pub fn unsubscribe(&self, client_id: u64, sub_id: &str) {
        let mut inner = self.inner.lock().unwrap();
        remove_locked(&mut inner, &(client_id, sub_id.to_string()));
    }

    /// Tear down every subscription owned by `client_id`. Called from
    /// the WS handler when the connection closes — without this,
    /// re-runs would keep firing for a socket nobody reads.
    pub fn disconnect_client(&self, client_id: u64) {
        let mut inner = self.inner.lock().unwrap();
        let keys: Vec<SubKey> = inner
            .by_client
            .get(&client_id)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default();
        for k in keys {
            remove_locked(&mut inner, &k);
        }
    }

    /// Called by `WsSseNotifier::notify` on every change event. Finds
    /// subs whose deps overlap and marks them dirty for re-run.
    ///
    /// Special case: any sub currently in `running` is dirtied
    /// REGARDLESS of dep match. While the handler is executing, its
    /// deps haven't been indexed yet — a write landing in that
    /// window would be invisible to the dep-based match path and
    /// never trigger a re-run. Marking running subs dirty
    /// unconditionally is the conservative fix: at worst it causes
    /// a redundant re-run when the write didn't actually affect the
    /// handler's reads; the alternative is the silent-staleness bug
    /// codex P1.1 caught against the previous design.
    pub fn on_change(&self, event: &ChangeEvent) {
        let mut inner = self.inner.lock().unwrap();
        let mut to_mark: Vec<SubKey> = Vec::new();

        // Row-level matches: exact (entity, row_id) intersection.
        if let Some(row_subs) = inner
            .by_row
            .get(&(event.entity.clone(), event.row_id.clone()))
        {
            for k in row_subs {
                to_mark.push(k.clone());
            }
        }

        // Entity-level matches: subs that opted into entity-only
        // mode (no precise row deps) get matched here. Subs with
        // precise row deps that didn't cover this row are skipped
        // EXCEPT on Delete (the deleted row may not have been read
        // individually but the handler still listed the entity).
        if let Some(entity_subs) = inner.by_entity.get(&event.entity) {
            let candidates: Vec<SubKey> = entity_subs.iter().cloned().collect();
            for k in candidates {
                if let Some(sub) = inner.subs.get(&k) {
                    if sub.deps.entity_only() {
                        to_mark.push(k);
                    } else if matches!(event.kind, ChangeKind::Delete) {
                        to_mark.push(k);
                    }
                }
            }
        }

        // In-flight runs: any sub whose handler is currently
        // executing gets dirtied. The runner hasn't committed deps
        // yet, so by_entity/by_row don't reflect what the handler is
        // reading. Conservatively re-queue every running sub on
        // every change.
        for k in inner.running.iter() {
            to_mark.push(k.clone());
        }

        if to_mark.is_empty() {
            return;
        }
        let mut newly_dirty = false;
        for k in to_mark {
            if enqueue_dirty_locked(&mut inner, k) {
                newly_dirty = true;
            }
        }
        if newly_dirty {
            self.dirty_notify.notify_one();
        }
    }

    /// Re-runner thread body. Sleeps on the condvar until something
    /// goes dirty, then drains the dirty queue, running each sub
    /// (initial or re-run) and pushing changed results.
    fn runner_loop(self: Arc<Self>) {
        loop {
            // Take a snapshot of dirty work + their sub specs. Run
            // outside the lock so FnOps::call doesn't block the
            // change-event hot path. Mark each as `running` so
            // on_change knows to re-dirty if a write lands during
            // execution.
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
                        Some(k) => {
                            inner.pending.remove(&k);
                            if let Some(sub) = inner.subs.get(&k).cloned() {
                                inner.running.insert(k);
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
                let key = sub.key();
                match outcome {
                    HandlerResult::Ok(outcome) => {
                        // Re-check that the sub still exists AND has
                        // the same version we ran against. A stale
                        // run (unsubscribed mid-run, or re-registered
                        // with new args mid-run) must NOT push its
                        // result — it would deliver the OLD answer
                        // to either nobody or, worse, to the NEW
                        // logical sub. Codex P1.2.
                        let mut inner = self.inner.lock().unwrap();
                        inner.running.remove(&key);
                        let still_current = inner
                            .subs
                            .get(&key)
                            .map(|s| s.version == sub.version)
                            .unwrap_or(false);
                        if !still_current {
                            drop(inner);
                            continue;
                        }
                        let prev_hash = sub.last_hash;
                        let should_push = prev_hash.map(|h| h != outcome.hash).unwrap_or(true);
                        // Update state INSIDE the lock so the new
                        // deps are visible to the next on_change.
                        update_deps_and_hash_locked(&mut inner, &key, &outcome);
                        drop(inner);
                        if should_push {
                            self.push_result(&sub.sub_id, &outcome.value, sub.client_id);
                        }
                    }
                    HandlerResult::Err { code, message } => {
                        // Same version + currency check before
                        // pushing an error frame — don't surface a
                        // stale error to a freshly re-registered sub.
                        let mut inner = self.inner.lock().unwrap();
                        inner.running.remove(&key);
                        let still_current = inner
                            .subs
                            .get(&key)
                            .map(|s| s.version == sub.version)
                            .unwrap_or(false);
                        drop(inner);
                        if !still_current {
                            continue;
                        }
                        // Initial-run errors get surfaced as a
                        // reactive-error frame so the React hook can
                        // stop spinning. Re-run errors (where we'd
                        // already pushed a successful result) get
                        // logged + dropped — the client keeps showing
                        // the last good value rather than flipping to
                        // an error state on a transient handler glitch.
                        if sub.last_hash.is_none() {
                            self.push_error(&sub.sub_id, sub.client_id, &code, &message);
                        }
                    }
                    HandlerResult::RuntimeUnavailable => {
                        let mut inner = self.inner.lock().unwrap();
                        inner.running.remove(&key);
                        let still_current = inner
                            .subs
                            .get(&key)
                            .map(|s| s.version == sub.version)
                            .unwrap_or(false);
                        drop(inner);
                        if !still_current {
                            continue;
                        }
                        if sub.last_hash.is_none() {
                            self.push_error(
                                &sub.sub_id,
                                sub.client_id,
                                "REACTIVE_UNAVAILABLE",
                                "function runtime not configured",
                            );
                        }
                    }
                }
            }
        }
    }

    fn run_handler(&self, fn_name: &str, args: serde_json::Value, auth: AuthInfo) -> HandlerResult {
        let fn_ops = {
            let guard = self.fn_ops.lock().unwrap();
            guard.as_ref().map(Arc::clone)
        };
        let Some(fn_ops) = fn_ops else {
            return HandlerResult::RuntimeUnavailable;
        };
        let guard = pylon_functions::deps::enter();
        let result = fn_ops.call(fn_name, args, auth, None, None);
        let deps = guard.take();
        match result {
            Ok((value, _trace)) => {
                let hash = hash_value(&value);
                HandlerResult::Ok(ReactiveOutcome { value, deps, hash })
            }
            Err(e) => {
                tracing::warn!(
                    "[reactive] handler {fn_name} failed: {} {}",
                    e.code,
                    e.message
                );
                HandlerResult::Err {
                    code: e.code,
                    message: e.message,
                }
            }
        }
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

    fn push_error(&self, sub_id: &str, client_id: u64, code: &str, message: &str) {
        let frame = serde_json::json!({
            "type": "reactive-error",
            "sub_id": sub_id,
            "code": code,
            "message": message,
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

enum HandlerResult {
    Ok(ReactiveOutcome),
    Err { code: String, message: String },
    RuntimeUnavailable,
}

fn enqueue_dirty_locked(inner: &mut RegistryInner, key: SubKey) -> bool {
    if !inner.pending.insert(key.clone()) {
        return false;
    }
    // Dirty queue length is naturally bounded by total subs (each
    // sub appears at most once thanks to the pending dedupe set).
    // Total subs is bounded by PER_CLIENT_SUB_CAP × max connections,
    // so even a million-client cluster would max out at ~256M dirty
    // entries — far above DIRTY_QUEUE_CAP. Hitting the cap means
    // something pathological (handler loop generating writes that
    // dirty every sub). Log loudly + accept the entry rather than
    // drop, because dropping would PERMANENTLY desync a sub whose
    // only triggering event came during the overflow window. Codex
    // P1.3: previous "drop oldest" semantic violated the "rerun on
    // every dep change" contract.
    if inner.dirty.len() >= DIRTY_QUEUE_CAP {
        tracing::warn!(
            "[reactive] dirty queue over soft cap ({DIRTY_QUEUE_CAP}) — accepting anyway to preserve re-run contract; investigate handler-induced write storms"
        );
    }
    inner.dirty.push_back(key);
    true
}

/// Commit fresh deps + hash for `key`. Caller must hold the inner
/// mutex. Runs the standard "drop old indexes, install new" dance.
/// Preserves the version field (which only `register_pending`
/// mutates) so the runner's version check can detect mid-run
/// resubscribes.
fn update_deps_and_hash_locked(inner: &mut RegistryInner, key: &SubKey, outcome: &ReactiveOutcome) {
    let Some(sub_cloned) = inner.subs.get(key).cloned() else {
        return;
    };
    remove_locked(inner, key);
    let new_sub = Subscription {
        deps: outcome.deps.clone(),
        last_hash: Some(outcome.hash),
        ..sub_cloned
    };
    index_locked(inner, &new_sub);
    inner.subs.insert(key.clone(), new_sub);
}

fn index_locked(inner: &mut RegistryInner, sub: &Subscription) {
    let key = sub.key();
    for entity in &sub.deps.entities {
        inner
            .by_entity
            .entry(entity.clone())
            .or_default()
            .insert(key.clone());
    }
    for row in &sub.deps.rows {
        inner
            .by_row
            .entry(row.clone())
            .or_default()
            .insert(key.clone());
    }
    inner
        .by_client
        .entry(sub.client_id)
        .or_default()
        .insert(key);
}

fn remove_locked(inner: &mut RegistryInner, key: &SubKey) {
    let Some(sub) = inner.subs.remove(key) else {
        return;
    };
    for entity in &sub.deps.entities {
        if let Some(s) = inner.by_entity.get_mut(entity) {
            s.remove(key);
            if s.is_empty() {
                inner.by_entity.remove(entity);
            }
        }
    }
    for row in &sub.deps.rows {
        if let Some(s) = inner.by_row.get_mut(row) {
            s.remove(key);
            if s.is_empty() {
                inner.by_row.remove(row);
            }
        }
    }
    if let Some(s) = inner.by_client.get_mut(&sub.client_id) {
        s.remove(key);
        if s.is_empty() {
            inner.by_client.remove(&sub.client_id);
        }
    }
    inner.pending.remove(key);
    // dirty VecDeque keeps the stale entry; the runner skips it
    // when subs.get returns None.
}

fn hash_value(value: &serde_json::Value) -> u64 {
    use std::hash::Hasher;
    let mut h = std::collections::hash_map::DefaultHasher::new();
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
            roles: Vec::new(),
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

    /// Directly install a sub at "ready" state for change-matching tests
    /// (bypasses the runner since there's no fn_ops in unit tests).
    fn install_sub_with_deps(
        reg: &Arc<ReactiveRegistry>,
        client_id: u64,
        sub_id: &str,
        deps: DepSet,
    ) {
        let mut inner = reg.inner.lock().unwrap();
        let sub = Subscription {
            sub_id: sub_id.to_string(),
            fn_name: "f".into(),
            args: serde_json::json!({}),
            auth: make_auth(),
            client_id,
            deps,
            last_hash: Some(0),
            version: 1,
        };
        index_locked(&mut inner, &sub);
        inner.subs.insert(sub.key(), sub);
    }

    #[test]
    fn entity_only_sub_matches_any_row_change() {
        let reg = ReactiveRegistry::new(make_hub());
        install_sub_with_deps(&reg, 42, "s1", dep_set(&["Recording"], &[]));
        reg.on_change(&dummy_change("Recording", "r_99", ChangeKind::Insert));
        let inner = reg.inner.lock().unwrap();
        assert!(inner.pending.contains(&(42, "s1".to_string())));
    }

    #[test]
    fn precise_row_sub_skips_unrelated_row_change() {
        let reg = ReactiveRegistry::new(make_hub());
        install_sub_with_deps(
            &reg,
            42,
            "s1",
            dep_set(&["Recording"], &[("Recording", "r_1")]),
        );
        reg.on_change(&dummy_change("Recording", "r_2", ChangeKind::Update));
        let inner = reg.inner.lock().unwrap();
        assert!(!inner.pending.contains(&(42, "s1".to_string())));
    }

    #[test]
    fn precise_row_sub_fires_for_matching_row() {
        let reg = ReactiveRegistry::new(make_hub());
        install_sub_with_deps(
            &reg,
            42,
            "s1",
            dep_set(&["Recording"], &[("Recording", "r_1")]),
        );
        reg.on_change(&dummy_change("Recording", "r_1", ChangeKind::Update));
        let inner = reg.inner.lock().unwrap();
        assert!(inner.pending.contains(&(42, "s1".to_string())));
    }

    #[test]
    fn delete_fires_precise_sub_even_for_unread_row() {
        let reg = ReactiveRegistry::new(make_hub());
        install_sub_with_deps(
            &reg,
            42,
            "s1",
            dep_set(&["Recording"], &[("Recording", "r_a")]),
        );
        reg.on_change(&dummy_change("Recording", "r_b", ChangeKind::Delete));
        let inner = reg.inner.lock().unwrap();
        assert!(inner.pending.contains(&(42, "s1".to_string())));
    }

    #[test]
    fn unsubscribe_removes_indexes() {
        let reg = ReactiveRegistry::new(make_hub());
        install_sub_with_deps(
            &reg,
            42,
            "s1",
            dep_set(&["Recording", "Org"], &[("Recording", "r_1")]),
        );
        assert_eq!(reg.len(), 1);
        reg.unsubscribe(42, "s1");
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
            install_sub_with_deps(&reg, 42, &format!("s{i}"), dep_set(&["R"], &[]));
        }
        install_sub_with_deps(&reg, 99, "other", dep_set(&["R"], &[]));
        assert_eq!(reg.len(), 6);
        reg.disconnect_client(42);
        assert_eq!(reg.len(), 1);
        assert!(reg
            .inner
            .lock()
            .unwrap()
            .subs
            .contains_key(&(99, "other".to_string())));
    }

    #[test]
    fn dedupe_pending_dirties_only_enqueue_once() {
        let reg = ReactiveRegistry::new(make_hub());
        install_sub_with_deps(&reg, 42, "s1", dep_set(&["Recording"], &[]));
        reg.on_change(&dummy_change("Recording", "r_1", ChangeKind::Insert));
        reg.on_change(&dummy_change("Recording", "r_2", ChangeKind::Update));
        reg.on_change(&dummy_change("Recording", "r_3", ChangeKind::Insert));
        let inner = reg.inner.lock().unwrap();
        assert_eq!(inner.dirty.len(), 1);
        assert_eq!(inner.pending.len(), 1);
    }

    /// Two clients pick the same sub_id. Without client-scoped keys
    /// they'd collide in `subs` and the second register would
    /// overwrite the first, sending re-runs for the first client's
    /// state to the second client's socket — cross-client data leak.
    #[test]
    fn same_sub_id_on_two_clients_does_not_collide() {
        let reg = ReactiveRegistry::new(make_hub());
        install_sub_with_deps(&reg, 1, "shared_id", dep_set(&["A"], &[]));
        install_sub_with_deps(&reg, 2, "shared_id", dep_set(&["B"], &[]));
        assert_eq!(reg.len(), 2);
        // Unsubscribing client 1's "shared_id" must not remove
        // client 2's "shared_id" — keys are tuples now.
        reg.unsubscribe(1, "shared_id");
        assert_eq!(reg.len(), 1);
        let inner = reg.inner.lock().unwrap();
        assert!(inner.subs.contains_key(&(2, "shared_id".to_string())));
        assert!(!inner.subs.contains_key(&(1, "shared_id".to_string())));
    }

    #[test]
    fn per_client_sub_cap_refuses_overflow() {
        let reg = ReactiveRegistry::new(make_hub());
        // Stuff a client up to the cap with installs (which bypass
        // the cap since they're test helpers). Now try a real
        // register_pending and confirm it OverLimits.
        for i in 0..PER_CLIENT_SUB_CAP {
            install_sub_with_deps(&reg, 7, &format!("s{i}"), dep_set(&["A"], &[]));
        }
        let outcome = reg.register_pending(
            "overflow".into(),
            "f".into(),
            serde_json::json!({}),
            make_auth(),
            7,
            // client_id matches the stuffed client
        );
        assert_eq!(outcome, RegisterOutcome::OverLimit);
        assert!(!reg
            .inner
            .lock()
            .unwrap()
            .subs
            .contains_key(&(7, "overflow".to_string())));
    }

    #[test]
    fn dirty_queue_over_cap_still_enqueues() {
        // Codex P1.3: previous "drop oldest" semantic silently
        // de-scheduled re-runs whose only triggering event landed
        // during overflow. The fix is to accept the entry + log a
        // warning. This test verifies no re-run is silently lost.
        let reg = ReactiveRegistry::new(make_hub());
        // Pre-fill the dirty queue past the cap WITHOUT triggering
        // dedupe (use unique sub keys).
        for i in 0..(DIRTY_QUEUE_CAP + 50) {
            install_sub_with_deps(&reg, i as u64, "s", dep_set(&["E"], &[]));
        }
        reg.on_change(&dummy_change("E", "r", ChangeKind::Insert));
        let inner = reg.inner.lock().unwrap();
        // Every sub got dirtied — no drops despite exceeding cap.
        assert_eq!(inner.pending.len(), inner.subs.len());
        assert_eq!(inner.dirty.len(), inner.subs.len());
    }

    /// Codex P1.1: a write that lands between "handler reads" and
    /// "deps get indexed" must dirty the in-flight sub. Simulated
    /// here by directly inserting into `running` and verifying
    /// on_change picks the sub up even though by_entity/by_row
    /// don't yet know about its deps.
    #[test]
    fn on_change_dirties_running_subs_regardless_of_deps() {
        let reg = ReactiveRegistry::new(make_hub());
        // Install a sub with NO deps — the dep-based match path
        // would never fire for it.
        install_sub_with_deps(&reg, 7, "s1", dep_set(&[], &[]));
        // Mark it running — handler is mid-flight, deps not yet
        // committed.
        {
            let mut inner = reg.inner.lock().unwrap();
            inner.running.insert((7, "s1".to_string()));
            // Clear what install_sub_with_deps put in by_entity/by_row
            // (it put nothing, since deps is empty). The point of
            // this test is: ZERO dep entries, yet on_change must
            // still queue.
            assert!(inner.by_entity.is_empty());
            assert!(inner.by_row.is_empty());
        }
        reg.on_change(&dummy_change("Anything", "r1", ChangeKind::Insert));
        let inner = reg.inner.lock().unwrap();
        assert!(
            inner.pending.contains(&(7, "s1".to_string())),
            "running sub must be dirtied by any change event"
        );
    }

    /// Codex P1.2: a sub re-registered with a fresh version while
    /// a prior run is in flight must not receive the OLD run's
    /// result. The runner checks `sub.version == cloned.version`
    /// before pushing; this test confirms a version mismatch
    /// blocks the push by exercising the version-bump in
    /// register_pending.
    #[test]
    fn register_pending_bumps_version() {
        let reg = ReactiveRegistry::new(make_hub());
        let _ = reg.register_pending(
            "s1".into(),
            "f".into(),
            serde_json::json!({}),
            make_auth(),
            42,
            // first register
        );
        let v1 = {
            let inner = reg.inner.lock().unwrap();
            inner.subs.get(&(42, "s1".to_string())).unwrap().version
        };
        let _ = reg.register_pending(
            "s1".into(),
            "f".into(),
            serde_json::json!({"changed": true}),
            make_auth(),
            42,
        );
        let v2 = {
            let inner = reg.inner.lock().unwrap();
            inner.subs.get(&(42, "s1".to_string())).unwrap().version
        };
        assert!(v2 > v1, "re-register must bump version (v1={v1}, v2={v2})");
    }
}
