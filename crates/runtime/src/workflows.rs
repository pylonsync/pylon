use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Workflow definitions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkflowStatus {
    Pending,
    Running,
    Sleeping,
    WaitingForEvent,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StepStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepResult {
    pub step_id: String,
    pub name: String,
    pub status: StepStatus,
    pub output: Option<serde_json::Value>,
    pub error: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub duration_ms: Option<u64>,
    pub retry_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowInstance {
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
    pub status: WorkflowStatus,
    pub steps: Vec<StepResult>,
    pub output: Option<serde_json::Value>,
    pub error: Option<String>,
    pub created_at: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    /// If sleeping, when to wake up (unix timestamp seconds).
    pub wake_at: Option<u64>,
    /// If waiting for an event, the event name.
    pub waiting_for: Option<String>,
    /// Current step index being executed.
    pub current_step: usize,
    /// Max retries per step.
    pub max_retries: u32,
    /// Lookup key set at start (for example a lead id). At most one
    /// non-terminal run exists per workflow name and key.
    #[serde(default)]
    pub key: Option<String>,
    /// If waiting for an event with a timeout, when the wait expires
    /// (unix timestamp seconds).
    #[serde(default)]
    pub wait_deadline: Option<u64>,
    /// Events sent while the run was not waiting for them, oldest first.
    /// The next matching `waitForEvent` consumes the oldest match.
    #[serde(default)]
    pub pending_events: Vec<BufferedEvent>,
    /// The reason passed to `cancel`, if the run was cancelled.
    #[serde(default)]
    pub cancel_reason: Option<String>,
    /// Sequence numbers of buffered events consumed since the last save.
    /// The Postgres store deletes these rows in the same transaction that
    /// records the step, so an event is never both consumed and kept.
    #[serde(skip)]
    pub consumed_events: Vec<u64>,
}

/// An event delivered to a run that was not waiting for it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BufferedEvent {
    pub seq: u64,
    pub event: String,
    pub data: serde_json::Value,
    pub received_at: String,
}

/// Result of [`WorkflowEngine::send_event`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct EventDelivery {
    /// The run was waiting for this event and resumes now.
    pub delivered: bool,
    /// The run was not waiting for this event; the event is queued for
    /// its next matching `waitForEvent`.
    pub buffered: bool,
}

/// Result of [`WorkflowEngine::start_with_key`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartOutcome {
    pub id: String,
    /// False when a non-terminal run with the same name and key already
    /// existed; `id` is then that run's id.
    pub created: bool,
}

/// Which runs [`WorkflowEngine::list_filtered`] returns.
#[derive(Debug, Clone, Default)]
pub struct WorkflowFilter {
    pub status: Option<StatusFilter>,
    pub name: Option<String>,
    pub key: Option<String>,
    /// Max rows returned. 0 means [`DEFAULT_LIST_LIMIT`].
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusFilter {
    Is(WorkflowStatus),
    /// Any non-terminal status.
    Active,
}

impl StatusFilter {
    /// Parse the lowercase names used by the HTTP API and `ctx.workflows`.
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "active" => StatusFilter::Active,
            "pending" => StatusFilter::Is(WorkflowStatus::Pending),
            "running" => StatusFilter::Is(WorkflowStatus::Running),
            "sleeping" => StatusFilter::Is(WorkflowStatus::Sleeping),
            "waiting" => StatusFilter::Is(WorkflowStatus::WaitingForEvent),
            "completed" => StatusFilter::Is(WorkflowStatus::Completed),
            "failed" => StatusFilter::Is(WorkflowStatus::Failed),
            "cancelled" => StatusFilter::Is(WorkflowStatus::Cancelled),
            _ => return None,
        })
    }

    fn matches(&self, status: &WorkflowStatus) -> bool {
        match self {
            StatusFilter::Is(s) => s == status,
            StatusFilter::Active => !status.is_terminal(),
        }
    }
}

impl WorkflowStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            WorkflowStatus::Completed | WorkflowStatus::Failed | WorkflowStatus::Cancelled
        )
    }

    /// Lowercase name used by the HTTP API and `ctx.workflows`.
    pub fn api_name(&self) -> &'static str {
        match self {
            WorkflowStatus::Pending => "pending",
            WorkflowStatus::Running => "running",
            WorkflowStatus::Sleeping => "sleeping",
            WorkflowStatus::WaitingForEvent => "waiting",
            WorkflowStatus::Completed => "completed",
            WorkflowStatus::Failed => "failed",
            WorkflowStatus::Cancelled => "cancelled",
        }
    }
}

impl WorkflowInstance {
    /// The run's state without step history or buffered event payloads.
    pub fn summary(&self) -> serde_json::Value {
        serde_json::json!({
            "id": self.id,
            "name": self.name,
            "key": self.key,
            "status": self.status.api_name(),
            "input": self.input,
            "output": self.output,
            "error": self.error,
            "cancelReason": self.cancel_reason,
            "waitingFor": self.waiting_for,
            "waitDeadline": self.wait_deadline,
            "wakeAt": self.wake_at,
            "bufferedEvents": self.pending_events.len(),
            "createdAt": stamp_secs(&self.created_at),
            "startedAt": self.started_at.as_deref().and_then(stamp_secs),
            "completedAt": self.completed_at.as_deref().and_then(stamp_secs),
        })
    }

    /// Resolve the current `waitForEvent` and record it as a completed
    /// step: `event:<name>` with the event data, or `timeout:<name>` with
    /// no output. The TS executor replays these records in order.
    fn resolve_wait(&mut self, event: &str, data: Option<serde_json::Value>) {
        let name = match data {
            Some(_) => format!("event:{event}"),
            None => format!("timeout:{event}"),
        };
        self.steps.push(StepResult {
            step_id: format!("step_{}", self.steps.len()),
            name,
            status: StepStatus::Completed,
            output: data,
            error: None,
            started_at: Some(now_iso()),
            completed_at: Some(now_iso()),
            duration_ms: None,
            retry_count: 0,
        });
        self.current_step += 1;
        self.status = WorkflowStatus::Running;
        self.waiting_for = None;
        self.wait_deadline = None;
    }

    /// Remove and return the oldest buffered event named `event`.
    fn take_buffered(&mut self, event: &str) -> Option<serde_json::Value> {
        let pos = self.pending_events.iter().position(|e| e.event == event)?;
        let taken = self.pending_events.remove(pos);
        self.consumed_events.push(taken.seq);
        Some(taken.data)
    }

    /// Whether a paused run has work to do right now.
    fn has_due_work(&self, now: u64) -> bool {
        match self.status {
            WorkflowStatus::Pending | WorkflowStatus::Running => true,
            WorkflowStatus::Sleeping => self.wake_at.is_none_or(|t| t <= now),
            WorkflowStatus::WaitingForEvent => {
                self.wait_deadline.is_some_and(|t| t <= now)
                    || self
                        .waiting_for
                        .as_deref()
                        .is_some_and(|event| self.pending_events.iter().any(|e| e.event == event))
            }
            _ => false,
        }
    }
}

/// Default and max row count for [`WorkflowEngine::list_filtered`].
pub const DEFAULT_LIST_LIMIT: usize = 100;
pub const MAX_LIST_LIMIT: usize = 1000;
/// Max buffered events per run. Further sends fail until the run
/// consumes some.
pub const MAX_BUFFERED_EVENTS: usize = 100;
/// How long finished runs stay listable before `prune_terminal` drops them.
pub const TERMINAL_HISTORY_SECS: u64 = 24 * 3600;
const MAX_KEY_LEN: usize = 256;
const MAX_EVENT_NAME_LEN: usize = 200;
const MAX_CANCEL_REASON_LEN: usize = 1000;

/// A registered workflow definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDef {
    pub name: String,
    pub description: String,
    /// The TypeScript file that defines this workflow.
    pub file: String,
    /// Max retries per step.
    pub max_retries: u32,
    /// Timeout per step in seconds.
    pub step_timeout_secs: u64,
}

// ---------------------------------------------------------------------------
// Workflow Engine
// ---------------------------------------------------------------------------

/// Executes one workflow slice, in-process. Installed by the server: it
/// dispatches the runner request through the Bun function pool as an
/// internal action call (`__pylon_workflow_run`), so workflow steps get
/// the full function runtime — ctx.db, ctx.llm, scheduling, the idle
/// timeout — instead of a bespoke HTTP side-channel.
pub type WorkflowRunnerHook =
    std::sync::Arc<dyn Fn(&serde_json::Value) -> Result<serde_json::Value, String> + Send + Sync>;

/// Nudges the driver: "instance `id` has work in `delay_secs` seconds".
/// Installed by the server to enqueue a `pylon.workflow.advance` job —
/// start/send_event/wake must never execute steps inline on the caller's
/// thread (an HTTP route would block for the whole segment). A delay is
/// used for sleep and wait-timeout deadlines, so they fire on time
/// instead of on the next minute tick.
pub type WorkflowKickHook = Box<dyn Fn(&str, u64) + Send + Sync>;

const DISTRIBUTED_WORKFLOW_LEASE_SECS: u64 = 30;
const DISTRIBUTED_WORKFLOW_HEARTBEAT_SECS: u64 = 10;

pub struct WorkflowEngine {
    /// Registered workflow definitions.
    definitions: Mutex<HashMap<String, WorkflowDef>>,
    /// Active and historical workflow instances.
    instances: Mutex<HashMap<String, WorkflowInstance>>,
    /// URL of an EXTERNAL TypeScript workflow runner. Empty = none; the
    /// in-process hook is the normal path. Kept for operators who set
    /// PYLON_WORKFLOW_RUNNER_URL explicitly.
    runner_url: String,
    /// In-process step executor. See [`WorkflowRunnerHook`].
    runner_hook: Mutex<Option<WorkflowRunnerHook>>,
    /// Driver nudge. See [`WorkflowKickHook`].
    kick_hook: Mutex<Option<WorkflowKickHook>>,
    /// Persistence. Every state transition is mirrored so workflows
    /// survive restart (restore_from at boot). Best-effort like the job
    /// store: a failed write logs, never blocks.
    store: Mutex<Option<std::sync::Arc<crate::workflow_store::WorkflowStore>>>,
    /// Shared workflow state used by Postgres deployments.
    pg_store: Mutex<Option<Arc<crate::pg_workflow_store::PgWorkflowStore>>>,
    /// Lease tokens held by this process. Persistence checks the token so a
    /// stale worker cannot overwrite a newer workflow transition.
    lease_tokens: Mutex<HashMap<String, String>>,
    lease_seq: AtomicU64,
    id_seq: AtomicU64,
    instance_id: String,
    /// Instances currently being driven by run_to_pause. The driver can
    /// receive duplicate kicks (the sweep tick re-kicks every Running
    /// instance); without this guard two concurrent drivers would both
    /// read current_step N and execute the same step twice.
    advancing: Mutex<std::collections::HashSet<String>>,
    /// Max instances to keep in history (unused currently, reserved for GC).
    #[allow(dead_code)]
    max_history: usize,
}

struct DistributedWorkflowLease<'a> {
    engine: &'a WorkflowEngine,
    store: Arc<crate::pg_workflow_store::PgWorkflowStore>,
    id: String,
    token: String,
    stop: Option<std::sync::mpsc::Sender<()>>,
    heartbeat: Option<std::thread::JoinHandle<()>>,
}

impl Drop for DistributedWorkflowLease<'_> {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(handle) = self.heartbeat.take() {
            let _ = handle.join();
        }
        if let Err(e) = self.store.release(&self.id, &self.token) {
            tracing::warn!("[workflows] failed to release lease for {}: {e}", self.id);
        }
        if let Ok(mut tokens) = self.engine.lease_tokens.lock() {
            tokens.remove(&self.id);
        }
    }
}

impl WorkflowEngine {
    pub fn new(runner_url: &str, max_history: usize) -> Self {
        Self {
            definitions: Mutex::new(HashMap::new()),
            instances: Mutex::new(HashMap::new()),
            runner_url: runner_url.to_string(),
            runner_hook: Mutex::new(None),
            kick_hook: Mutex::new(None),
            store: Mutex::new(None),
            pg_store: Mutex::new(None),
            lease_tokens: Mutex::new(HashMap::new()),
            lease_seq: AtomicU64::new(1),
            id_seq: AtomicU64::new(1),
            instance_id: pylon_cluster::new_instance_id(),
            advancing: Mutex::new(std::collections::HashSet::new()),
            max_history,
        }
    }

    /// Install the in-process step executor (wins over `runner_url`).
    pub fn set_runner_hook(&self, hook: WorkflowRunnerHook) {
        *self.runner_hook.lock().unwrap() = Some(hook);
    }

    /// Install the driver nudge called whenever an instance gains work.
    pub fn set_kick_hook(&self, hook: WorkflowKickHook) {
        *self.kick_hook.lock().unwrap() = Some(hook);
    }

    /// Attach a persistent store; call once at startup after restore_from.
    pub fn attach_store(&self, store: std::sync::Arc<crate::workflow_store::WorkflowStore>) {
        *self.store.lock().unwrap() = Some(store);
    }

    pub fn attach_pg_store(&self, store: Arc<crate::pg_workflow_store::PgWorkflowStore>) {
        *self.pg_store.lock().unwrap() = Some(store);
    }

    fn pg_store(&self) -> Option<Arc<crate::pg_workflow_store::PgWorkflowStore>> {
        self.pg_store.lock().unwrap().clone()
    }

    fn persist(&self, instance: &WorkflowInstance) -> Result<(), String> {
        if let Some(store) = self.pg_store() {
            let token = self.lease_tokens.lock().unwrap().get(&instance.id).cloned();
            return match token {
                Some(token) => store.save_owned(instance, &token),
                None => store.save(instance),
            };
        }
        if let Some(store) = self.store.lock().unwrap().as_ref() {
            store.save(instance)?;
        }
        Ok(())
    }

    fn acquire_distributed_lease(
        &self,
        workflow_id: &str,
    ) -> Result<Option<DistributedWorkflowLease<'_>>, String> {
        let Some(store) = self.pg_store() else {
            return Ok(None);
        };
        let token = format!(
            "{}-{}",
            self.instance_id,
            self.lease_seq.fetch_add(1, Ordering::Relaxed)
        );
        if !store.try_acquire(workflow_id, &token, DISTRIBUTED_WORKFLOW_LEASE_SECS)? {
            return Ok(None);
        }
        self.lease_tokens
            .lock()
            .unwrap()
            .insert(workflow_id.to_string(), token.clone());

        let heartbeat_store = Arc::clone(&store);
        let heartbeat_id = workflow_id.to_string();
        let heartbeat_token = token.clone();
        let (stop_tx, stop_rx) = std::sync::mpsc::channel();
        let heartbeat = match std::thread::Builder::new()
            .name("pylon-workflow-heartbeat".into())
            .spawn(move || loop {
                match stop_rx.recv_timeout(Duration::from_secs(DISTRIBUTED_WORKFLOW_HEARTBEAT_SECS))
                {
                    Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        match heartbeat_store.heartbeat(
                            &heartbeat_id,
                            &heartbeat_token,
                            DISTRIBUTED_WORKFLOW_LEASE_SECS,
                        ) {
                            Ok(true) => {}
                            Ok(false) => {
                                tracing::warn!(
                                    "[workflows] lease lost while {} was advancing",
                                    heartbeat_id
                                );
                                break;
                            }
                            Err(e) => tracing::warn!(
                                "[workflows] heartbeat failed for {}: {e}",
                                heartbeat_id
                            ),
                        }
                    }
                }
            }) {
            Ok(handle) => handle,
            Err(e) => {
                self.lease_tokens.lock().unwrap().remove(workflow_id);
                let _ = store.release(workflow_id, &token);
                return Err(format!("failed to start workflow heartbeat: {e}"));
            }
        };
        Ok(Some(DistributedWorkflowLease {
            engine: self,
            store,
            id: workflow_id.to_string(),
            token,
            stop: Some(stop_tx),
            heartbeat: Some(heartbeat),
        }))
    }

    /// Persist `workflow_id`'s in-memory state and return the saved copy.
    ///
    /// The SQLite store writes while the instance map is still locked, so
    /// two writers for the same run cannot reach disk out of order. The
    /// Postgres store writes after the map is unlocked; the lease token
    /// and the Cancelled fence in the store order its writes.
    fn persist_locked(
        &self,
        mut instances: std::sync::MutexGuard<'_, HashMap<String, WorkflowInstance>>,
        workflow_id: &str,
    ) -> Result<WorkflowInstance, String> {
        let snapshot = instances
            .get(workflow_id)
            .cloned()
            .ok_or_else(|| format!("Workflow '{}' not found", workflow_id))?;
        if self.pg_store().is_some() {
            drop(instances);
            self.persist(&snapshot)?;
            if let Some(inst) = self.instances.lock().unwrap().get_mut(workflow_id) {
                inst.consumed_events.clear();
            }
        } else {
            self.persist(&snapshot)?;
            if let Some(inst) = instances.get_mut(workflow_id) {
                inst.consumed_events.clear();
            }
        }
        Ok(snapshot)
    }

    /// After a failed Postgres write, return Cancelled if the run was
    /// cancelled concurrently (the store fences writes to cancelled runs),
    /// otherwise the original error.
    fn cancelled_or(&self, workflow_id: &str, err: String) -> Result<WorkflowStatus, String> {
        if let Some(store) = self.pg_store() {
            if let Ok(Some(latest)) = store.load(workflow_id) {
                if latest.status == WorkflowStatus::Cancelled {
                    self.instances
                        .lock()
                        .unwrap()
                        .insert(workflow_id.to_string(), latest);
                    return Ok(WorkflowStatus::Cancelled);
                }
            }
        }
        Err(err)
    }

    pub(crate) fn kick(&self, workflow_id: &str) {
        self.kick_after(workflow_id, 0);
    }

    fn kick_after(&self, workflow_id: &str, delay_secs: u64) {
        if let Some(hook) = self.kick_hook.lock().unwrap().as_ref() {
            hook(workflow_id, delay_secs);
        }
    }

    /// Schedule a kick for the run's next deadline (sleep wake-up or
    /// wait timeout), if it has one.
    fn kick_at_deadline(&self, instance: &WorkflowInstance) {
        let deadline = match instance.status {
            WorkflowStatus::Sleeping => instance.wake_at,
            WorkflowStatus::WaitingForEvent => instance.wait_deadline,
            _ => None,
        };
        if let Some(at) = deadline {
            self.kick_after(&instance.id, at.saturating_sub(now_secs()));
        }
    }

    /// Whether the run has work right now. Postgres reads the shared row
    /// and inbox, because other processes deliver events there.
    fn needs_drive(&self, workflow_id: &str) -> bool {
        let now = now_secs();
        if let Some(store) = self.pg_store() {
            return store.has_due_work(workflow_id, now).unwrap_or_else(|e| {
                tracing::warn!("[workflows] due-work check failed for {workflow_id}: {e}");
                false
            });
        }
        self.instances
            .lock()
            .unwrap()
            .get(workflow_id)
            .is_some_and(|inst| inst.has_due_work(now))
    }

    /// Register a workflow definition.
    pub fn register(&self, def: WorkflowDef) {
        self.definitions
            .lock()
            .unwrap()
            .insert(def.name.clone(), def);
    }

    /// Start a new workflow instance. Returns the instance ID.
    pub fn start(&self, name: &str, input: serde_json::Value) -> Result<String, String> {
        self.start_with_key(name, input, None).map(|o| o.id)
    }

    /// Start a workflow instance with an optional lookup key. When `key`
    /// is set and a non-terminal run of `name` with that key exists, no
    /// new run starts and the existing run's id is returned with
    /// `created: false`.
    pub fn start_with_key(
        &self,
        name: &str,
        input: serde_json::Value,
        key: Option<&str>,
    ) -> Result<StartOutcome, String> {
        if let Some(k) = key {
            if k.is_empty() || k.len() > MAX_KEY_LEN {
                return Err(format!(
                    "workflow key must be 1-{MAX_KEY_LEN} bytes, got {}",
                    k.len()
                ));
            }
        }
        let max_retries = {
            let defs = self.definitions.lock().unwrap();
            defs.get(name)
                .ok_or_else(|| format!("Workflow '{}' not registered", name))?
                .max_retries
        };

        let id = format!(
            "wf_{}_{}",
            self.instance_id,
            self.id_seq.fetch_add(1, Ordering::Relaxed)
        );
        let instance = WorkflowInstance {
            id: id.clone(),
            name: name.to_string(),
            input,
            status: WorkflowStatus::Pending,
            steps: Vec::new(),
            output: None,
            error: None,
            created_at: now_iso(),
            started_at: None,
            completed_at: None,
            wake_at: None,
            waiting_for: None,
            current_step: 0,
            max_retries,
            key: key.map(str::to_string),
            wait_deadline: None,
            pending_events: Vec::new(),
            cancel_reason: None,
            consumed_events: Vec::new(),
        };

        if let Some(store) = self.pg_store() {
            // The store's partial unique index on (name, key) over
            // non-terminal runs decides which of two concurrent starts wins.
            if let Some(existing) = store.insert_new(&instance)? {
                return Ok(StartOutcome {
                    id: existing,
                    created: false,
                });
            }
            self.instances.lock().unwrap().insert(id.clone(), instance);
        } else {
            let mut instances = self.instances.lock().unwrap();
            if let Some(k) = key {
                if let Some(existing) = instances.values().find(|i| {
                    i.name == name && i.key.as_deref() == Some(k) && !i.status.is_terminal()
                }) {
                    return Ok(StartOutcome {
                        id: existing.id.clone(),
                        created: false,
                    });
                }
            }
            instances.insert(id.clone(), instance);
            if let Err(e) = self.persist_locked(instances, &id) {
                self.instances.lock().unwrap().remove(&id);
                return Err(e);
            }
        }
        // Hand the new instance to the driver — steps never run on the
        // caller's thread.
        self.kick(&id);
        Ok(StartOutcome { id, created: true })
    }

    /// Drive a workflow until it pauses (sleep / wait_event) or reaches a
    /// terminal state, executing at most `max_steps` steps. The driver's
    /// entry point: `step_complete` leaves the status Running, which means
    /// "there is more to do right now".
    pub fn run_to_pause(
        &self,
        workflow_id: &str,
        max_steps: usize,
    ) -> Result<WorkflowStatus, String> {
        let distributed_lease = match self.acquire_distributed_lease(workflow_id)? {
            Some(lease) => Some(lease),
            None if self.pg_store().is_some() => {
                return self
                    .get(workflow_id)
                    .map(|instance| instance.status)
                    .ok_or_else(|| format!("Workflow '{}' not found", workflow_id));
            }
            None => None,
        };
        if let Some(store) = self.pg_store() {
            let latest = store
                .load(workflow_id)?
                .ok_or_else(|| format!("Workflow '{}' not found", workflow_id))?;
            self.instances
                .lock()
                .unwrap()
                .insert(workflow_id.to_string(), latest);
        }
        // Single-driver guard: duplicate kicks (the sweep tick re-kicks
        // every Running instance) must not race two drivers into
        // executing the same step twice. Losing the race is a no-op —
        // the winner drives the instance to its next pause. The guard is
        // a Drop type so a panic inside advance (poisoned lock in the
        // ops plumbing) can't strand the instance behind a permanently
        // held entry.
        struct AdvancingGuard<'a> {
            engine: &'a WorkflowEngine,
            id: String,
        }
        impl Drop for AdvancingGuard<'_> {
            fn drop(&mut self) {
                // `if let Ok` (not unwrap): unwinding with a poisoned
                // mutex here would double-panic into an abort.
                if let Ok(mut advancing) = self.engine.advancing.lock() {
                    advancing.remove(&self.id);
                }
            }
        }
        {
            let mut advancing = self.advancing.lock().unwrap();
            if !advancing.insert(workflow_id.to_string()) {
                return self
                    .get(workflow_id)
                    .map(|i| i.status)
                    .ok_or_else(|| format!("Workflow '{}' not found", workflow_id));
            }
        }
        let _guard = AdvancingGuard {
            engine: self,
            id: workflow_id.to_string(),
        };
        let result = (|| {
            let mut last = self.advance_owned(workflow_id)?;
            let mut steps = 1;
            while last == WorkflowStatus::Running && steps < max_steps {
                last = self.advance_owned(workflow_id)?;
                steps += 1;
            }
            Ok(last)
        })();
        drop(_guard);
        drop(distributed_lease);
        // Re-kick when work remains: the step budget ran out mid-run, or
        // an event or deadline arrived while this driver held the run. A
        // kick sent during that window found the run busy and did
        // nothing, so this check runs after the guard and lease are
        // released.
        if result.is_ok() && self.needs_drive(workflow_id) {
            self.kick(workflow_id);
        }
        result
    }

    /// Execute the next step of a workflow by calling the TS runner.
    ///
    /// The TS runner returns an action object describing what happened:
    /// - `{ "action": "step_complete", "step_name": "...", "output": ... }`
    /// - `{ "action": "sleep", "duration": "24h" }`
    /// - `{ "action": "wait_event", "event": "user_confirmed" }`
    /// - `{ "action": "complete", "output": ... }`
    /// - `{ "action": "fail", "error": "...", "step_name": "..." }`
    pub fn advance(&self, workflow_id: &str) -> Result<WorkflowStatus, String> {
        let lease = match self.acquire_distributed_lease(workflow_id)? {
            Some(lease) => Some(lease),
            None if self.pg_store().is_some() => {
                return Err("Workflow is currently advancing; retry the operation".into());
            }
            None => None,
        };
        self.refresh_distributed(workflow_id)?;
        let result = self.advance_owned(workflow_id);
        drop(lease);
        result
    }

    fn advance_owned(&self, workflow_id: &str) -> Result<WorkflowStatus, String> {
        let instance = {
            let instances = self.instances.lock().unwrap();
            instances
                .get(workflow_id)
                .cloned()
                .ok_or_else(|| format!("Workflow '{}' not found", workflow_id))?
        };

        // Terminal states: nothing to do.
        match instance.status {
            WorkflowStatus::Completed | WorkflowStatus::Failed | WorkflowStatus::Cancelled => {
                return Ok(instance.status);
            }
            WorkflowStatus::Sleeping => {
                if let Some(wake_at) = instance.wake_at {
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    if now < wake_at {
                        return Ok(WorkflowStatus::Sleeping);
                    }
                }
                // Timer expired -- fall through to advance.
            }
            WorkflowStatus::WaitingForEvent => {
                // Resume only when a matching event is buffered or the
                // wait timed out; otherwise there is nothing to run.
                self.refresh_inbox(workflow_id)?;
                let mut instances = self.instances.lock().unwrap();
                let Some(inst) = instances.get_mut(workflow_id) else {
                    return Err(format!("Workflow '{}' not found", workflow_id));
                };
                if inst.status != WorkflowStatus::WaitingForEvent {
                    return Ok(inst.status.clone());
                }
                let event = inst.waiting_for.clone().unwrap_or_default();
                if let Some(data) = inst.take_buffered(&event) {
                    inst.resolve_wait(&event, Some(data));
                } else if inst.wait_deadline.is_some_and(|t| t <= now_secs()) {
                    inst.resolve_wait(&event, None);
                } else {
                    return Ok(WorkflowStatus::WaitingForEvent);
                }
                if let Err(e) = self.persist_locked(instances, workflow_id) {
                    return self.cancelled_or(workflow_id, e);
                }
            }
            _ => {}
        }
        let instance = {
            let instances = self.instances.lock().unwrap();
            instances
                .get(workflow_id)
                .cloned()
                .ok_or_else(|| format!("Workflow '{}' not found", workflow_id))?
        };

        let request = serde_json::json!({
            "workflow_id": workflow_id,
            "workflow_name": instance.name,
            "input": instance.input,
            "current_step": instance.current_step,
            "completed_steps": runner_steps(&instance.steps),
        });

        // A transport failure (runner call error, slice idle-timeout) goes
        // through the SAME retry accounting as a step that reported
        // {action:"fail"}. Propagating it as Err instead let a hanging
        // step retry forever: the advance job dead-lettered, the sweep
        // tick re-kicked, and the instance showed Running while each
        // attempt burned a worker for the full slice timeout.
        let response = match self.call_runner(&request) {
            Ok(r) => r,
            Err(e) => serde_json::json!({
                "action": "fail",
                "error": format!("workflow runner error: {e}"),
                "step_name": "__transport",
            }),
        };
        self.apply_response(workflow_id, &response)
    }

    /// Advance a workflow with a pre-provided response (for testing without
    /// a running TS runner).
    pub fn advance_with_response(
        &self,
        workflow_id: &str,
        response: serde_json::Value,
    ) -> Result<WorkflowStatus, String> {
        let lease = match self.acquire_distributed_lease(workflow_id)? {
            Some(lease) => Some(lease),
            None if self.pg_store().is_some() => {
                return Err("Workflow is currently advancing; retry the operation".into());
            }
            None => None,
        };
        self.refresh_distributed(workflow_id)?;
        // Verify the workflow exists and is advanceable.
        {
            let instances = self.instances.lock().unwrap();
            let instance = instances
                .get(workflow_id)
                .ok_or_else(|| format!("Workflow '{}' not found", workflow_id))?;

            match instance.status {
                WorkflowStatus::Completed | WorkflowStatus::Failed | WorkflowStatus::Cancelled => {
                    return Ok(instance.status.clone());
                }
                _ => {}
            }
        }

        let result = self.apply_response(workflow_id, &response);
        drop(lease);
        result
    }

    /// Send an event to a workflow run.
    ///
    /// If the run is waiting for `event`, it resumes. Otherwise the event
    /// is buffered and the run's next `waitForEvent(event)` consumes it.
    /// Sending never waits for a step in progress. Terminal runs reject
    /// events. Null data is stored as `{}`, so a `waitForEvent` with a
    /// timeout resolves to null only when it timed out.
    pub fn send_event(
        &self,
        workflow_id: &str,
        event: &str,
        data: serde_json::Value,
    ) -> Result<EventDelivery, String> {
        if event.is_empty() || event.len() > MAX_EVENT_NAME_LEN {
            return Err(format!(
                "event name must be 1-{MAX_EVENT_NAME_LEN} bytes, got {}",
                event.len()
            ));
        }
        let data = if data.is_null() {
            serde_json::json!({})
        } else {
            data
        };

        if let Some(store) = self.pg_store() {
            // Lease-free: the event lands in the shared inbox even while
            // another process runs a step. The driver consumes it at the
            // run's next wait (see run_to_pause for the lost-kick check).
            let delivery = store.push_event(workflow_id, event, &data, MAX_BUFFERED_EVENTS)?;
            if delivery.delivered {
                self.kick(workflow_id);
            }
            return Ok(delivery);
        }

        let mut instances = self.instances.lock().unwrap();
        let inst = instances
            .get_mut(workflow_id)
            .ok_or_else(|| format!("Workflow '{}' not found", workflow_id))?;
        if inst.status.is_terminal() {
            return Err(format!(
                "Workflow is {}; it no longer accepts events",
                inst.status.api_name()
            ));
        }
        let waiting = inst.status == WorkflowStatus::WaitingForEvent
            && inst.waiting_for.as_deref() == Some(event);
        if waiting {
            inst.resolve_wait(event, Some(data));
        } else {
            if inst.pending_events.len() >= MAX_BUFFERED_EVENTS {
                return Err(format!(
                    "Workflow has {MAX_BUFFERED_EVENTS} buffered events; it must consume some before more can be sent"
                ));
            }
            let seq = inst.pending_events.iter().map(|e| e.seq).max().unwrap_or(0) + 1;
            inst.pending_events.push(BufferedEvent {
                seq,
                event: event.to_string(),
                data,
                received_at: now_iso(),
            });
        }
        self.persist_locked(instances, workflow_id)?;
        if waiting {
            self.kick(workflow_id);
        }
        Ok(EventDelivery {
            delivered: waiting,
            buffered: !waiting,
        })
    }

    /// Cancel a workflow run. Returns false when the run had already
    /// finished (completed, failed, or cancelled).
    ///
    /// Cancellation never waits for a step in progress. That step may
    /// finish, but its result is discarded and no further step runs.
    pub fn cancel(&self, workflow_id: &str, reason: Option<&str>) -> Result<bool, String> {
        let reason = reason.map(|r| truncate_utf8(r, MAX_CANCEL_REASON_LEN).to_string());
        if let Some(store) = self.pg_store() {
            let cancelled = store.cancel(workflow_id, reason.as_deref(), now_secs())?;
            if let Some(latest) = store.load(workflow_id)? {
                self.instances
                    .lock()
                    .unwrap()
                    .insert(workflow_id.to_string(), latest);
            }
            return Ok(cancelled);
        }
        let mut instances = self.instances.lock().unwrap();
        let inst = instances
            .get_mut(workflow_id)
            .ok_or_else(|| format!("Workflow '{}' not found", workflow_id))?;
        if inst.status.is_terminal() {
            return Ok(false);
        }
        inst.status = WorkflowStatus::Cancelled;
        inst.completed_at = Some(now_iso());
        inst.cancel_reason = reason;
        inst.waiting_for = None;
        inst.wait_deadline = None;
        inst.wake_at = None;
        self.persist_locked(instances, workflow_id)?;
        Ok(true)
    }

    /// Get a workflow instance by ID.
    pub fn get(&self, workflow_id: &str) -> Option<WorkflowInstance> {
        if let Some(store) = self.pg_store() {
            return store.load(workflow_id).unwrap_or_else(|e| {
                tracing::warn!("[workflows] Postgres get failed for {workflow_id}: {e}");
                None
            });
        }
        self.instances.lock().unwrap().get(workflow_id).cloned()
    }

    /// List all workflow instances with optional status filter.
    pub fn list(&self, status: Option<&WorkflowStatus>) -> Vec<WorkflowInstance> {
        if let Some(store) = self.pg_store() {
            return store
                .list(status.map(workflow_status_name))
                .unwrap_or_else(|e| {
                    tracing::warn!("[workflows] Postgres list failed: {e}");
                    Vec::new()
                });
        }
        let instances = self.instances.lock().unwrap();
        instances
            .values()
            .filter(|i| {
                status
                    .map(|s| std::mem::discriminant(&i.status) == std::mem::discriminant(s))
                    .unwrap_or(true)
            })
            .cloned()
            .collect()
    }

    /// List runs matching `filter`, newest first. `include_steps: false`
    /// skips loading step history (Postgres reads it per row).
    pub fn list_filtered(
        &self,
        filter: &WorkflowFilter,
        include_steps: bool,
    ) -> Result<Vec<WorkflowInstance>, String> {
        let limit = match filter.limit {
            0 => DEFAULT_LIST_LIMIT,
            n => n.min(MAX_LIST_LIMIT),
        };
        if let Some(store) = self.pg_store() {
            return store.list_filtered(filter, limit, include_steps);
        }
        let instances = self.instances.lock().unwrap();
        let mut rows: Vec<WorkflowInstance> = instances
            .values()
            .filter(|i| filter.status.as_ref().is_none_or(|s| s.matches(&i.status)))
            .filter(|i| filter.name.as_deref().is_none_or(|n| i.name == n))
            .filter(|i| {
                filter
                    .key
                    .as_deref()
                    .is_none_or(|k| i.key.as_deref() == Some(k))
            })
            .cloned()
            .collect();
        drop(instances);
        rows.sort_by(|a, b| {
            stamp_secs(&b.created_at)
                .cmp(&stamp_secs(&a.created_at))
                .then_with(|| b.id.cmp(&a.id))
        });
        rows.truncate(limit);
        if !include_steps {
            for row in &mut rows {
                row.steps.clear();
            }
        }
        Ok(rows)
    }

    /// Load the Postgres inbox for `workflow_id` into the in-memory copy.
    /// Other processes insert events there without the lease.
    fn refresh_inbox(&self, workflow_id: &str) -> Result<(), String> {
        let Some(store) = self.pg_store() else {
            return Ok(());
        };
        let events = store.load_events(workflow_id)?;
        if let Some(inst) = self.instances.lock().unwrap().get_mut(workflow_id) {
            inst.pending_events = events
                .into_iter()
                .filter(|e| !inst.consumed_events.contains(&e.seq))
                .collect();
        }
        Ok(())
    }

    /// List registered workflow definitions.
    pub fn definitions(&self) -> Vec<WorkflowDef> {
        self.definitions.lock().unwrap().values().cloned().collect()
    }

    /// Return only workflows that need a driver job. The Postgres path reads
    /// IDs from an indexed status query and does not load workflow steps.
    pub fn runnable_ids(&self) -> Vec<String> {
        if let Some(store) = self.pg_store() {
            return store.runnable_ids().unwrap_or_else(|e| {
                tracing::warn!("[workflows] failed to list runnable workflows: {e}");
                Vec::new()
            });
        }
        self.instances
            .lock()
            .unwrap()
            .values()
            .filter(|instance| {
                matches!(
                    instance.status,
                    WorkflowStatus::Pending | WorkflowStatus::Running
                )
            })
            .map(|instance| instance.id.clone())
            .collect()
    }

    /// Prune terminal instances older than `max_age_secs` from memory and
    /// the store. Returns how many were dropped from memory. Without
    /// this, the instance map and the workflows DB grow without bound
    /// now that workflows actually run.
    pub fn prune_terminal(&self, max_age_secs: u64) -> usize {
        if let Some(store) = self.pg_store() {
            return store.cleanup_terminal(max_age_secs).unwrap_or_else(|e| {
                tracing::warn!("[workflows] Postgres cleanup failed: {e}");
                0
            });
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let cutoff = now.saturating_sub(max_age_secs);
        let mut instances = self.instances.lock().unwrap();
        let before = instances.len();
        instances.retain(|_, inst| {
            let terminal = matches!(
                inst.status,
                WorkflowStatus::Completed | WorkflowStatus::Failed | WorkflowStatus::Cancelled
            );
            if !terminal {
                return true;
            }
            // completed_at is epoch-seconds + "Z" (now_iso below).
            let completed = inst
                .completed_at
                .as_deref()
                .and_then(|s| s.trim_end_matches('Z').parse::<u64>().ok());
            match completed {
                // Strictly newer than the cutoff survives; prune(0) means
                // "drop every terminal instance".
                Some(ts) => ts > cutoff,
                // Terminal but unparsable/absent stamp: keep — never
                // delete on ambiguity.
                None => true,
            }
        });
        let dropped = before - instances.len();
        drop(instances);
        if let Some(store) = self.store.lock().unwrap().as_ref() {
            store.cleanup_terminal(max_age_secs);
        }
        dropped
    }

    /// Wake sleeping workflows whose timer has expired. Returns the IDs of
    /// workflows that were woken.
    pub fn wake_sleeping(&self) -> Vec<String> {
        if let Some(store) = self.pg_store() {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let sleeping = store.due_sleeping_ids(now).unwrap_or_else(|e| {
                tracing::warn!("[workflows] failed to find sleeping workflows: {e}");
                Vec::new()
            });
            let mut woken = Vec::new();
            for workflow_id in sleeping {
                let Ok(Some(lease)) = self.acquire_distributed_lease(&workflow_id) else {
                    continue;
                };
                let Ok(Some(mut current)) = store.load(&workflow_id) else {
                    continue;
                };
                if current.status != WorkflowStatus::Sleeping
                    || current.wake_at.is_none_or(|wake_at| wake_at > now)
                {
                    continue;
                }
                current.status = WorkflowStatus::Running;
                current.wake_at = None;
                self.instances
                    .lock()
                    .unwrap()
                    .insert(current.id.clone(), current.clone());
                if let Err(e) = self.persist(&current) {
                    tracing::warn!("[workflows] failed to wake {}: {e}", current.id);
                    continue;
                }
                drop(lease);
                self.kick(&current.id);
                woken.push(current.id);
            }
            // Waits that timed out or have a matching buffered event. The
            // driver resolves them; kicking is enough.
            match store.due_wait_ids(now) {
                Ok(ids) => {
                    for id in ids {
                        self.kick(&id);
                        woken.push(id);
                    }
                }
                Err(e) => tracing::warn!("[workflows] failed to find due waits: {e}"),
            }
            return woken;
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let mut woken = Vec::new();
        let mut instances = self.instances.lock().unwrap();

        for (id, inst) in instances.iter_mut() {
            if inst.status == WorkflowStatus::Sleeping {
                if let Some(wake_at) = inst.wake_at {
                    if now >= wake_at {
                        inst.status = WorkflowStatus::Running;
                        inst.wake_at = None;
                        woken.push(id.clone());
                    }
                }
            }
        }
        for id in &woken {
            if let Some(snapshot) = instances.get(id) {
                if let Err(e) = self.persist(snapshot) {
                    tracing::warn!("[workflows] failed to persist wake for {id}: {e}");
                }
            }
        }
        // Waits that timed out or have a matching buffered event. The
        // driver resolves them; kicking is enough.
        let due_waits: Vec<String> = instances
            .values()
            .filter(|i| i.status == WorkflowStatus::WaitingForEvent && i.has_due_work(now))
            .map(|i| i.id.clone())
            .collect();
        drop(instances);
        woken.extend(due_waits);
        for id in &woken {
            self.kick(id);
        }

        woken
    }

    fn refresh_distributed(&self, workflow_id: &str) -> Result<(), String> {
        let Some(store) = self.pg_store() else {
            return Ok(());
        };
        let workflow = store
            .load(workflow_id)?
            .ok_or_else(|| format!("Workflow '{}' not found", workflow_id))?;
        self.instances
            .lock()
            .unwrap()
            .insert(workflow_id.to_string(), workflow);
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Persistence
    // -----------------------------------------------------------------------

    /// Restore active and sleeping workflows from a persistent store.
    ///
    /// Loads all non-terminal workflows and inserts them into the in-memory
    /// instance map. Returns the number of workflows restored.
    ///
    /// Call this once at startup, before the engine begins processing.
    pub fn restore_from(&self, store: &crate::workflow_store::WorkflowStore) -> usize {
        let mut count = 0;

        let active = store.load_active().unwrap_or_default();
        let sleeping = store.load_sleeping().unwrap_or_default();

        let mut instances = self.instances.lock().unwrap();

        for wf in active {
            instances.insert(wf.id.clone(), wf);
            count += 1;
        }
        for wf in sleeping {
            // Avoid double-counting if load_active and load_sleeping overlap
            // (they shouldn't given the status filters, but guard anyway).
            if !instances.contains_key(&wf.id) {
                instances.insert(wf.id.clone(), wf);
                count += 1;
            }
        }
        // Recent finished runs, for `list`. Not counted: they need no
        // driver. prune_terminal drops them after the same window.
        for wf in store
            .load_recent_terminal(TERMINAL_HISTORY_SECS)
            .unwrap_or_default()
        {
            instances.entry(wf.id.clone()).or_insert(wf);
        }

        count
    }

    // -----------------------------------------------------------------------
    // Internal
    // -----------------------------------------------------------------------

    /// Apply a runner response to the workflow, updating state accordingly.
    fn apply_response(
        &self,
        workflow_id: &str,
        response: &serde_json::Value,
    ) -> Result<WorkflowStatus, String> {
        let action = response
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("fail");

        // A wait timeout that doesn't parse fails the run at once instead
        // of defaulting to zero (an immediate timeout). Retrying the same
        // code cannot fix it.
        let mut wait_timeout: Option<u64> = None;
        let mut invalid_timeout: Option<String> = None;
        if action == "wait_event" {
            if let Some(raw) = response.get("timeout").filter(|v| !v.is_null()) {
                match raw.as_str().and_then(parse_duration_strict) {
                    Some(secs) => wait_timeout = Some(secs),
                    None => {
                        invalid_timeout = Some(format!(
                            "invalid waitForEvent timeout {raw}; use a duration like \"60s\", \"5m\", or \"24h\""
                        ))
                    }
                }
            }
            self.refresh_inbox(workflow_id)?;
        }

        let mut instances = self.instances.lock().unwrap();
        let inst = instances
            .get_mut(workflow_id)
            .ok_or_else(|| format!("Workflow '{}' not found", workflow_id))?;

        // A response for a terminal instance is DROPPED, not applied. The
        // race this closes: cancel() lands while a slice is executing;
        // without this check the slice's step_complete flips the status
        // back to Running and the cancelled run keeps going. Postgres
        // cancels from other processes are fenced in the store instead.
        if inst.status.is_terminal() {
            return Ok(inst.status.clone());
        }

        if inst.started_at.is_none() {
            inst.started_at = Some(now_iso());
        }

        let result = match action {
            "step_complete" => {
                let step_name = response
                    .get("step_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                let output = response.get("output").cloned();

                inst.steps.push(StepResult {
                    step_id: format!("step_{}", inst.steps.len()),
                    name: step_name.to_string(),
                    status: StepStatus::Completed,
                    output,
                    error: None,
                    started_at: Some(now_iso()),
                    completed_at: Some(now_iso()),
                    duration_ms: response.get("duration_ms").and_then(|v| v.as_u64()),
                    retry_count: 0,
                });
                inst.current_step += 1;
                inst.status = WorkflowStatus::Running;

                Ok(WorkflowStatus::Running)
            }
            "sleep" => {
                let duration_str = response
                    .get("duration")
                    .and_then(|v| v.as_str())
                    .unwrap_or("0s");
                let secs = parse_duration_str(duration_str);
                let wake_at = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
                    + secs;

                inst.status = WorkflowStatus::Sleeping;
                inst.wake_at = Some(wake_at);
                inst.current_step += 1;

                Ok(WorkflowStatus::Sleeping)
            }
            "wait_event" if invalid_timeout.is_some() => {
                inst.status = WorkflowStatus::Failed;
                inst.error = invalid_timeout;
                inst.completed_at = Some(now_iso());
                Ok(WorkflowStatus::Failed)
            }
            "wait_event" => {
                let event = response
                    .get("event")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if let Some(data) = inst.take_buffered(&event) {
                    // Sent before the run reached this wait.
                    inst.resolve_wait(&event, Some(data));
                    Ok(WorkflowStatus::Running)
                } else if wait_timeout == Some(0) {
                    inst.resolve_wait(&event, None);
                    Ok(WorkflowStatus::Running)
                } else {
                    inst.status = WorkflowStatus::WaitingForEvent;
                    inst.waiting_for = Some(event);
                    inst.wait_deadline = wait_timeout.map(|secs| now_secs().saturating_add(secs));
                    Ok(WorkflowStatus::WaitingForEvent)
                }
            }
            "complete" => {
                inst.status = WorkflowStatus::Completed;
                inst.output = response.get("output").cloned();
                inst.completed_at = Some(now_iso());

                Ok(WorkflowStatus::Completed)
            }
            "fail" => {
                let error = response
                    .get("error")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown error")
                    .to_string();

                let step_name = response
                    .get("step_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");

                // Count previous failures for the same step to decide retry.
                let retry_count = inst
                    .steps
                    .iter()
                    .filter(|s| s.name == step_name && s.status == StepStatus::Failed)
                    .count() as u32;

                if retry_count < inst.max_retries {
                    inst.steps.push(StepResult {
                        step_id: format!("step_{}", inst.steps.len()),
                        name: step_name.to_string(),
                        status: StepStatus::Failed,
                        output: None,
                        error: Some(error),
                        started_at: Some(now_iso()),
                        completed_at: Some(now_iso()),
                        duration_ms: None,
                        retry_count: retry_count + 1,
                    });
                    // Don't advance current_step -- retry the same step.
                    Ok(WorkflowStatus::Running)
                } else {
                    inst.status = WorkflowStatus::Failed;
                    inst.error = Some(error);
                    inst.completed_at = Some(now_iso());
                    Ok(WorkflowStatus::Failed)
                }
            }
            _ => Err(format!("Unknown action: {action}")),
        };
        let status = result?;
        match self.persist_locked(instances, workflow_id) {
            Ok(snapshot) => {
                self.kick_at_deadline(&snapshot);
                Ok(status)
            }
            Err(e) => self.cancelled_or(workflow_id, e),
        }
    }

    /// Execute one workflow slice: the in-process hook (the Bun function
    /// pool) when installed, else an explicitly configured external HTTP
    /// runner. No hook and no URL is a hard error — the old code
    /// defaulted to 127.0.0.1:9876, which nothing has ever served, so TS
    /// workflows silently never executed.
    fn call_runner(&self, request: &serde_json::Value) -> Result<serde_json::Value, String> {
        // Clone the Arc OUT of the mutex before calling: a slice can run
        // for minutes (long agent step), and holding the lock across it
        // would serialize every workflow in the process behind one slow
        // step — while its advance job occupies a worker doing nothing.
        let hook = self.runner_hook.lock().unwrap().clone();
        if let Some(hook) = hook {
            return hook(request);
        }
        if self.runner_url.is_empty() {
            return Err(
                "no workflow runner available: declare workflows in workflows/ (runs \
                 in-process) or set PYLON_WORKFLOW_RUNNER_URL for an external runner"
                    .into(),
            );
        }
        self.call_runner_http(request)
    }

    /// Call an external TypeScript workflow runner via HTTP.
    fn call_runner_http(&self, request: &serde_json::Value) -> Result<serde_json::Value, String> {
        use std::io::{Read, Write};
        use std::net::TcpStream;

        let url = &self.runner_url;
        let host = url.strip_prefix("http://").unwrap_or(url);
        let (host_port, path) = match host.find('/') {
            Some(i) => (&host[..i], &host[i..]),
            None => (host, "/"),
        };

        let body = request.to_string();
        let http_request = format!(
            "POST {} HTTP/1.1\r\n\
             Host: {}\r\n\
             Content-Type: application/json\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\
             \r\n\
             {}",
            path,
            host_port,
            body.len(),
            body
        );

        let mut stream = TcpStream::connect(host_port)
            .map_err(|e| format!("Failed to connect to workflow runner: {e}"))?;
        stream.set_read_timeout(Some(Duration::from_secs(30))).ok();
        stream
            .write_all(http_request.as_bytes())
            .map_err(|e| format!("Write failed: {e}"))?;

        let mut response = String::new();
        stream.read_to_string(&mut response).ok();

        let body = response.split("\r\n\r\n").nth(1).unwrap_or("{}");
        serde_json::from_str(body).map_err(|e| format!("Failed to parse runner response: {e}"))
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Step records in the runner wire format (`WorkflowStepResult` in
/// packages/functions/src/workflows.ts), which uses lowercase statuses.
/// The admin API serializes `StepStatus` with serde's variant names
/// ("Completed"); the TS executor matches on "completed".
fn runner_steps(steps: &[StepResult]) -> serde_json::Value {
    serde_json::Value::Array(
        steps
            .iter()
            .map(|s| {
                serde_json::json!({
                    "step_id": s.step_id,
                    "name": s.name,
                    "status": match s.status {
                        StepStatus::Pending => "pending",
                        StepStatus::Running => "running",
                        StepStatus::Completed => "completed",
                        StepStatus::Failed => "failed",
                        StepStatus::Skipped => "skipped",
                    },
                    "output": s.output,
                    "error": s.error,
                    "started_at": s.started_at,
                    "completed_at": s.completed_at,
                    "duration_ms": s.duration_ms,
                    "retry_count": s.retry_count,
                })
            })
            .collect(),
    )
}

/// Parse a duration like "24h", "30m", "5s", "1d", or bare seconds
/// ("60"). Returns None for anything else.
fn parse_duration_strict(s: &str) -> Option<u64> {
    let s = s.trim();
    let (digits, unit) = match s.char_indices().last()? {
        (i, c) if c.is_ascii_alphabetic() => (&s[..i], c),
        _ => (s, 's'),
    };
    let digits = digits.trim();
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: u64 = digits.parse().ok()?;
    let mult = match unit {
        's' => 1,
        'm' => 60,
        'h' => 3600,
        'd' => 86400,
        _ => return None,
    };
    n.checked_mul(mult)
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Seconds from an engine timestamp ("<epoch-secs>Z").
fn stamp_secs(s: &str) -> Option<u64> {
    s.trim_end_matches('Z').parse().ok()
}

fn truncate_utf8(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Parse a human-readable duration like "24h", "30m", "5s", "1d".
fn parse_duration_str(s: &str) -> u64 {
    let s = s.trim();
    if let Some(n) = s.strip_suffix('s') {
        n.parse().unwrap_or(0)
    } else if let Some(n) = s.strip_suffix('m') {
        n.parse::<u64>().unwrap_or(0) * 60
    } else if let Some(n) = s.strip_suffix('h') {
        n.parse::<u64>().unwrap_or(0) * 3600
    } else if let Some(n) = s.strip_suffix('d') {
        n.parse::<u64>().unwrap_or(0) * 86400
    } else {
        s.parse().unwrap_or(0)
    }
}

fn workflow_status_name(status: &WorkflowStatus) -> &'static str {
    match status {
        WorkflowStatus::Pending => "Pending",
        WorkflowStatus::Running => "Running",
        WorkflowStatus::Sleeping => "Sleeping",
        WorkflowStatus::WaitingForEvent => "WaitingForEvent",
        WorkflowStatus::Completed => "Completed",
        WorkflowStatus::Failed => "Failed",
        WorkflowStatus::Cancelled => "Cancelled",
    }
}

#[cfg(test)]
fn generate_workflow_id() -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);

    let mut hasher = DefaultHasher::new();
    ts.as_nanos().hash(&mut hasher);
    count.hash(&mut hasher);

    format!("wf_{:016x}", hasher.finish())
}

fn now_iso() -> String {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("{ts}Z")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> WorkflowEngine {
        let e = WorkflowEngine::new("http://127.0.0.1:19999/run", 100);
        e.register(WorkflowDef {
            name: "onboarding".into(),
            description: "User onboarding flow".into(),
            file: "workflows/onboarding.ts".into(),
            max_retries: 3,
            step_timeout_secs: 30,
        });
        e
    }

    // -- Registration & start -----------------------------------------------

    #[test]
    fn register_and_list_definitions() {
        let e = engine();
        let defs = e.definitions();
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "onboarding");
    }

    #[test]
    fn start_creates_pending_instance() {
        let e = engine();
        let id = e
            .start("onboarding", serde_json::json!({"user": "alice"}))
            .unwrap();
        let inst = e.get(&id).unwrap();
        assert_eq!(inst.status, WorkflowStatus::Pending);
        assert_eq!(inst.name, "onboarding");
        assert_eq!(inst.input, serde_json::json!({"user": "alice"}));
        assert_eq!(inst.current_step, 0);
    }

    #[test]
    fn start_unknown_workflow_errors() {
        let e = engine();
        let err = e.start("nonexistent", serde_json::json!({})).unwrap_err();
        assert!(err.contains("not registered"));
    }

    // -- Step recording via advance_with_response ---------------------------

    #[test]
    fn step_complete_advances_workflow() {
        let e = engine();
        let id = e.start("onboarding", serde_json::json!({})).unwrap();

        let status = e
            .advance_with_response(
                &id,
                serde_json::json!({
                    "action": "step_complete",
                    "step_name": "create_account",
                    "output": {"account_id": 42},
                    "duration_ms": 120
                }),
            )
            .unwrap();

        assert_eq!(status, WorkflowStatus::Running);
        let inst = e.get(&id).unwrap();
        assert_eq!(inst.current_step, 1);
        assert_eq!(inst.steps.len(), 1);
        assert_eq!(inst.steps[0].name, "create_account");
        assert_eq!(inst.steps[0].status, StepStatus::Completed);
        assert_eq!(
            inst.steps[0].output,
            Some(serde_json::json!({"account_id": 42}))
        );
        assert_eq!(inst.steps[0].duration_ms, Some(120));
        assert!(inst.started_at.is_some());
    }

    #[test]
    fn multiple_steps_advance_sequentially() {
        let e = engine();
        let id = e.start("onboarding", serde_json::json!({})).unwrap();

        e.advance_with_response(
            &id,
            serde_json::json!({"action": "step_complete", "step_name": "step_a"}),
        )
        .unwrap();

        e.advance_with_response(
            &id,
            serde_json::json!({"action": "step_complete", "step_name": "step_b"}),
        )
        .unwrap();

        let inst = e.get(&id).unwrap();
        assert_eq!(inst.current_step, 2);
        assert_eq!(inst.steps.len(), 2);
        assert_eq!(inst.steps[0].name, "step_a");
        assert_eq!(inst.steps[1].name, "step_b");
    }

    // -- Sleep & wake -------------------------------------------------------

    #[test]
    fn sleep_sets_wake_at_and_status() {
        let e = engine();
        let id = e.start("onboarding", serde_json::json!({})).unwrap();

        let status = e
            .advance_with_response(
                &id,
                serde_json::json!({"action": "sleep", "duration": "1h"}),
            )
            .unwrap();

        assert_eq!(status, WorkflowStatus::Sleeping);
        let inst = e.get(&id).unwrap();
        assert!(inst.wake_at.is_some());
        // wake_at should be roughly now + 3600
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let delta = inst.wake_at.unwrap().abs_diff(now + 3600);
        assert!(delta < 5, "wake_at should be ~1h from now, delta={delta}");
    }

    #[test]
    fn wake_sleeping_wakes_expired_workflows() {
        let e = engine();
        let id = e.start("onboarding", serde_json::json!({})).unwrap();

        // Sleep for 0 seconds (immediately expired).
        e.advance_with_response(
            &id,
            serde_json::json!({"action": "sleep", "duration": "0s"}),
        )
        .unwrap();

        let woken = e.wake_sleeping();
        assert!(woken.contains(&id));

        let inst = e.get(&id).unwrap();
        assert_eq!(inst.status, WorkflowStatus::Running);
        assert!(inst.wake_at.is_none());
    }

    #[test]
    fn wake_sleeping_does_not_wake_future_timers() {
        let e = engine();
        let id = e.start("onboarding", serde_json::json!({})).unwrap();

        e.advance_with_response(
            &id,
            serde_json::json!({"action": "sleep", "duration": "24h"}),
        )
        .unwrap();

        let woken = e.wake_sleeping();
        assert!(woken.is_empty());

        let inst = e.get(&id).unwrap();
        assert_eq!(inst.status, WorkflowStatus::Sleeping);
    }

    // -- Event sending ------------------------------------------------------

    #[test]
    fn wait_event_and_send_event() {
        let e = engine();
        let id = e.start("onboarding", serde_json::json!({})).unwrap();

        let status = e
            .advance_with_response(
                &id,
                serde_json::json!({"action": "wait_event", "event": "user_confirmed"}),
            )
            .unwrap();
        assert_eq!(status, WorkflowStatus::WaitingForEvent);

        e.send_event(
            &id,
            "user_confirmed",
            serde_json::json!({"confirmed": true}),
        )
        .unwrap();

        let inst = e.get(&id).unwrap();
        assert_eq!(inst.status, WorkflowStatus::Running);
        assert!(inst.waiting_for.is_none());
        assert_eq!(inst.steps.last().unwrap().name, "event:user_confirmed");
        assert_eq!(
            inst.steps.last().unwrap().output,
            Some(serde_json::json!({"confirmed": true}))
        );
    }

    #[test]
    fn send_event_other_name_is_buffered() {
        let e = engine();
        let id = e.start("onboarding", serde_json::json!({})).unwrap();

        e.advance_with_response(
            &id,
            serde_json::json!({"action": "wait_event", "event": "user_confirmed"}),
        )
        .unwrap();

        // A different event is buffered; the run keeps waiting.
        let delivery = e
            .send_event(&id, "wrong_event", serde_json::json!({}))
            .unwrap();
        assert_eq!(
            delivery,
            EventDelivery {
                delivered: false,
                buffered: true
            }
        );
        let inst = e.get(&id).unwrap();
        assert_eq!(inst.status, WorkflowStatus::WaitingForEvent);
        assert_eq!(inst.pending_events.len(), 1);
    }

    #[test]
    fn send_event_to_finished_run_errors() {
        let e = engine();
        let id = e.start("onboarding", serde_json::json!({})).unwrap();
        e.cancel(&id, None).unwrap();

        let err = e
            .send_event(&id, "anything", serde_json::json!({}))
            .unwrap_err();
        assert!(err.contains("no longer accepts events"), "{err}");
    }

    // -- Cancel -------------------------------------------------------------

    #[test]
    fn cancel_sets_status_and_completed_at() {
        let e = engine();
        let id = e.start("onboarding", serde_json::json!({})).unwrap();

        e.cancel(&id, None).unwrap();

        let inst = e.get(&id).unwrap();
        assert_eq!(inst.status, WorkflowStatus::Cancelled);
        assert!(inst.completed_at.is_some());
    }

    #[test]
    fn cancel_unknown_workflow_errors() {
        let e = engine();
        let err = e.cancel("wf_nonexistent", None).unwrap_err();
        assert!(err.contains("not found"));
    }

    // -- Completion ---------------------------------------------------------

    #[test]
    fn complete_sets_output_and_status() {
        let e = engine();
        let id = e.start("onboarding", serde_json::json!({})).unwrap();

        let status = e
            .advance_with_response(
                &id,
                serde_json::json!({"action": "complete", "output": {"result": "done"}}),
            )
            .unwrap();

        assert_eq!(status, WorkflowStatus::Completed);
        let inst = e.get(&id).unwrap();
        assert_eq!(inst.output, Some(serde_json::json!({"result": "done"})));
        assert!(inst.completed_at.is_some());
    }

    #[test]
    fn advance_completed_workflow_returns_status() {
        let e = engine();
        let id = e.start("onboarding", serde_json::json!({})).unwrap();

        e.advance_with_response(
            &id,
            serde_json::json!({"action": "complete", "output": null}),
        )
        .unwrap();

        let status = e
            .advance_with_response(
                &id,
                serde_json::json!({"action": "step_complete", "step_name": "ignored"}),
            )
            .unwrap();
        assert_eq!(status, WorkflowStatus::Completed);
    }

    // -- Retry on failure ---------------------------------------------------

    #[test]
    fn failure_retries_up_to_max() {
        let e = engine(); // max_retries = 3
        let id = e.start("onboarding", serde_json::json!({})).unwrap();

        // First 3 failures should retry (not mark workflow as Failed).
        for i in 0..3 {
            let status = e
                .advance_with_response(
                    &id,
                    serde_json::json!({
                        "action": "fail",
                        "step_name": "flaky_step",
                        "error": format!("attempt {i}")
                    }),
                )
                .unwrap();
            assert_eq!(
                status,
                WorkflowStatus::Running,
                "retry {i} should keep running"
            );
        }

        // 4th failure exceeds max_retries, workflow should fail.
        let status = e
            .advance_with_response(
                &id,
                serde_json::json!({
                    "action": "fail",
                    "step_name": "flaky_step",
                    "error": "final failure"
                }),
            )
            .unwrap();
        assert_eq!(status, WorkflowStatus::Failed);

        let inst = e.get(&id).unwrap();
        assert_eq!(inst.error, Some("final failure".into()));
        assert!(inst.completed_at.is_some());
        // current_step should not have advanced (all retries on same step).
        assert_eq!(inst.current_step, 0);
    }

    #[test]
    fn failure_then_success_works() {
        let e = engine();
        let id = e.start("onboarding", serde_json::json!({})).unwrap();

        // Fail once.
        e.advance_with_response(
            &id,
            serde_json::json!({"action": "fail", "step_name": "flakey", "error": "oops"}),
        )
        .unwrap();

        // Succeed on retry.
        e.advance_with_response(
            &id,
            serde_json::json!({"action": "step_complete", "step_name": "flakey", "output": "ok"}),
        )
        .unwrap();

        let inst = e.get(&id).unwrap();
        assert_eq!(inst.current_step, 1);
        assert_eq!(inst.steps.len(), 2);
        assert_eq!(inst.steps[0].status, StepStatus::Failed);
        assert_eq!(inst.steps[1].status, StepStatus::Completed);
    }

    // -- parse_duration_str -------------------------------------------------

    #[test]
    fn parse_duration_seconds() {
        assert_eq!(parse_duration_str("30s"), 30);
    }

    #[test]
    fn parse_duration_minutes() {
        assert_eq!(parse_duration_str("5m"), 300);
    }

    #[test]
    fn parse_duration_hours() {
        assert_eq!(parse_duration_str("24h"), 86400);
    }

    #[test]
    fn parse_duration_days() {
        assert_eq!(parse_duration_str("7d"), 604800);
    }

    #[test]
    fn parse_duration_bare_number() {
        assert_eq!(parse_duration_str("60"), 60);
    }

    #[test]
    fn parse_duration_invalid() {
        assert_eq!(parse_duration_str("abc"), 0);
    }

    #[test]
    fn parse_duration_with_whitespace() {
        assert_eq!(parse_duration_str("  10s  "), 10);
    }

    // -- List by status -----------------------------------------------------

    #[test]
    fn list_all_instances() {
        let e = engine();
        e.start("onboarding", serde_json::json!({})).unwrap();
        e.start("onboarding", serde_json::json!({})).unwrap();

        let all = e.list(None);
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn list_filters_by_status() {
        let e = engine();
        let id1 = e.start("onboarding", serde_json::json!({})).unwrap();
        let _id2 = e.start("onboarding", serde_json::json!({})).unwrap();

        // Complete one.
        e.advance_with_response(
            &id1,
            serde_json::json!({"action": "complete", "output": null}),
        )
        .unwrap();

        let completed = e.list(Some(&WorkflowStatus::Completed));
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].id, id1);

        let pending = e.list(Some(&WorkflowStatus::Pending));
        assert_eq!(pending.len(), 1);
    }

    // -- Unknown action returns error ---------------------------------------

    #[test]
    fn unknown_action_returns_error() {
        let e = engine();
        let id = e.start("onboarding", serde_json::json!({})).unwrap();

        let err = e
            .advance_with_response(&id, serde_json::json!({"action": "bogus"}))
            .unwrap_err();
        assert!(err.contains("Unknown action"));
    }

    // -- ID generation uniqueness -------------------------------------------

    #[test]
    fn generated_ids_are_unique() {
        let mut ids = std::collections::HashSet::new();
        for _ in 0..100 {
            let id = generate_workflow_id();
            assert!(ids.insert(id), "duplicate workflow ID generated");
        }
    }

    // -- Restore from store -------------------------------------------------

    #[test]
    fn restore_from_store() {
        let store = crate::workflow_store::WorkflowStore::in_memory().unwrap();

        // Save a pending workflow.
        let wf_pending = WorkflowInstance {
            id: "wf_aaa".into(),
            name: "onboarding".into(),
            input: serde_json::json!({"user": "bob"}),
            status: WorkflowStatus::Pending,
            steps: Vec::new(),
            output: None,
            error: None,
            created_at: "1000Z".into(),
            started_at: None,
            completed_at: None,
            wake_at: None,
            waiting_for: None,
            current_step: 0,
            max_retries: 3,
            key: None,
            wait_deadline: None,
            pending_events: Vec::new(),
            cancel_reason: None,
            consumed_events: Vec::new(),
        };

        // Save a sleeping workflow.
        let wf_sleeping = WorkflowInstance {
            id: "wf_bbb".into(),
            name: "onboarding".into(),
            input: serde_json::json!({}),
            status: WorkflowStatus::Sleeping,
            steps: vec![StepResult {
                step_id: "step_0".into(),
                name: "init".into(),
                status: StepStatus::Completed,
                output: Some(serde_json::json!({"ok": true})),
                error: None,
                started_at: Some("1000Z".into()),
                completed_at: Some("1001Z".into()),
                duration_ms: Some(50),
                retry_count: 0,
            }],
            output: None,
            error: None,
            created_at: "1000Z".into(),
            started_at: Some("1000Z".into()),
            completed_at: None,
            wake_at: Some(99999999),
            waiting_for: None,
            current_step: 1,
            max_retries: 3,
            key: None,
            wait_deadline: None,
            pending_events: Vec::new(),
            cancel_reason: None,
            consumed_events: Vec::new(),
        };

        // Save a completed workflow (should NOT be restored).
        let wf_completed = WorkflowInstance {
            id: "wf_ccc".into(),
            name: "onboarding".into(),
            input: serde_json::json!({}),
            status: WorkflowStatus::Completed,
            steps: Vec::new(),
            output: Some(serde_json::json!({"done": true})),
            error: None,
            created_at: "500Z".into(),
            started_at: Some("500Z".into()),
            completed_at: Some("600Z".into()),
            wake_at: None,
            waiting_for: None,
            current_step: 0,
            max_retries: 3,
            key: None,
            wait_deadline: None,
            pending_events: Vec::new(),
            cancel_reason: None,
            consumed_events: Vec::new(),
        };

        store.save(&wf_pending).unwrap();
        store.save(&wf_sleeping).unwrap();
        store.save(&wf_completed).unwrap();

        let e = WorkflowEngine::new("http://127.0.0.1:19999/run", 100);
        let restored = e.restore_from(&store);
        assert_eq!(restored, 2);

        // Verify the pending workflow is present.
        let inst = e.get("wf_aaa").unwrap();
        assert_eq!(inst.status, WorkflowStatus::Pending);
        assert_eq!(inst.input, serde_json::json!({"user": "bob"}));

        // Verify the sleeping workflow is present with its step.
        let inst = e.get("wf_bbb").unwrap();
        assert_eq!(inst.status, WorkflowStatus::Sleeping);
        assert_eq!(inst.steps.len(), 1);
        assert_eq!(inst.wake_at, Some(99999999));

        // Verify the completed workflow was NOT restored.
        assert!(e.get("wf_ccc").is_none());
    }

    #[test]
    fn cancel_mid_slice_is_not_reverted_by_the_slice_response() {
        // Race: cancel() lands while a slice executes; the slice's
        // step_complete used to flip the status back to Running.
        let e = engine();
        let id = e.start("onboarding", serde_json::json!({})).unwrap();
        e.cancel(&id, None).unwrap();
        let status = e
            .advance_with_response(
                &id,
                serde_json::json!({"action": "step_complete", "step_name": "late", "output": null}),
            )
            .unwrap();
        assert_eq!(status, WorkflowStatus::Cancelled);
        let inst = e.get(&id).unwrap();
        assert_eq!(inst.status, WorkflowStatus::Cancelled);
        assert!(inst.steps.is_empty());
    }

    #[test]
    fn prune_terminal_drops_old_finished_instances_only() {
        let e = engine_with_scripted_hook();
        let done_id = e.start("onboarding", serde_json::json!({})).unwrap();
        e.run_to_pause(&done_id, 50).unwrap();
        let live_id = e.start("onboarding", serde_json::json!({})).unwrap();

        // Age 0: nothing old enough (completed_at is "now").
        assert_eq!(e.prune_terminal(3600), 0);
        // Cutoff in the future relative to completion: the terminal one
        // goes, the running one stays.
        assert_eq!(e.prune_terminal(0), 1);
        assert!(e.get(&done_id).is_none());
        assert!(e.get(&live_id).is_some());
    }

    // -----------------------------------------------------------------
    // In-process runner hook + driver (the wiring that replaced the
    // phantom 127.0.0.1:9876 HTTP runner).
    // -----------------------------------------------------------------

    /// Engine whose runner hook simulates the TS slice executor for a
    /// two-step workflow: slice 0 and 1 complete a step, slice 2
    /// completes the run.
    fn engine_with_scripted_hook() -> WorkflowEngine {
        let e = engine();
        e.set_runner_hook(std::sync::Arc::new(|request| {
            let current = request
                .get("current_step")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            Ok(match current {
                0 => serde_json::json!({
                    "action": "step_complete", "step_name": "one", "output": {"n": 1}
                }),
                1 => serde_json::json!({
                    "action": "step_complete", "step_name": "two", "output": {"n": 2}
                }),
                _ => serde_json::json!({ "action": "complete", "output": {"done": true} }),
            })
        }));
        e
    }

    #[test]
    fn runner_hook_drives_run_to_pause_to_completion() {
        let e = engine_with_scripted_hook();
        let id = e.start("onboarding", serde_json::json!({"u": 1})).unwrap();

        let status = e.run_to_pause(&id, 50).unwrap();
        assert_eq!(status, WorkflowStatus::Completed);

        let inst = e.get(&id).unwrap();
        assert_eq!(inst.steps.len(), 2);
        assert_eq!(inst.steps[0].name, "one");
        assert_eq!(inst.steps[1].name, "two");
        assert_eq!(inst.output, Some(serde_json::json!({"done": true})));
    }

    #[test]
    fn no_hook_and_no_url_fails_the_instance_after_transport_retries() {
        // The old default POSTed to 127.0.0.1:9876, which nothing served —
        // and transport errors used to propagate as Err, which the driver
        // retried FOREVER (dead-letter → tick re-kick loop) while the
        // instance showed Running. Now a transport failure burns a step
        // retry like any {action:"fail"}, so the instance lands in Failed
        // with the diagnosis after max_retries.
        let e = WorkflowEngine::new("", 100);
        e.register(WorkflowDef {
            name: "onboarding".into(),
            description: String::new(),
            file: "workflows/".into(),
            max_retries: 2,
            step_timeout_secs: 600,
        });
        let id = e.start("onboarding", serde_json::json!({})).unwrap();
        let status = e.run_to_pause(&id, 50).unwrap();
        assert_eq!(status, WorkflowStatus::Failed);
        let inst = e.get(&id).unwrap();
        assert!(
            inst.error
                .as_deref()
                .unwrap_or("")
                .contains("no workflow runner available"),
            "{:?}",
            inst.error
        );
        // Each attempt is a recorded __transport failure — visible in the
        // step history, not an invisible retry loop.
        assert!(inst.steps.iter().all(|s| s.name == "__transport"));
        assert_eq!(inst.steps.len(), 2);
    }

    #[test]
    fn kicks_fire_on_start_send_event_and_wake() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let e = engine();
        let kicks = std::sync::Arc::new(AtomicUsize::new(0));
        let kicks_ref = std::sync::Arc::clone(&kicks);
        e.set_kick_hook(Box::new(move |_id, _delay| {
            kicks_ref.fetch_add(1, Ordering::SeqCst);
        }));

        // start() hands the new instance to the driver.
        let id = e.start("onboarding", serde_json::json!({})).unwrap();
        assert_eq!(kicks.load(Ordering::SeqCst), 1);

        // A delivered event resumes the run — another kick.
        e.advance_with_response(
            &id,
            serde_json::json!({"action": "wait_event", "event": "go"}),
        )
        .unwrap();
        e.send_event(&id, "go", serde_json::json!({"ok": true}))
            .unwrap();
        assert_eq!(kicks.load(Ordering::SeqCst), 2);

        // Entering a sleep schedules a kick for its wake time.
        e.advance_with_response(
            &id,
            serde_json::json!({"action": "sleep", "duration": "0s"}),
        )
        .unwrap();
        assert_eq!(kicks.load(Ordering::SeqCst), 3);

        // The minute sweep also wakes it — another kick.
        let woken = e.wake_sleeping();
        assert_eq!(woken, vec![id.clone()]);
        assert_eq!(kicks.load(Ordering::SeqCst), 4);
    }

    #[test]
    fn run_to_pause_respects_its_step_budget_and_rekicks() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let e = engine();
        // A hook that never finishes: every slice completes another step.
        e.set_runner_hook(std::sync::Arc::new(|request| {
            let n = request
                .get("current_step")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            Ok(serde_json::json!({
                "action": "step_complete", "step_name": format!("step-{n}"), "output": null
            }))
        }));
        let kicks = std::sync::Arc::new(AtomicUsize::new(0));
        let kicks_ref = std::sync::Arc::clone(&kicks);
        e.set_kick_hook(Box::new(move |_id, _delay| {
            kicks_ref.fetch_add(1, Ordering::SeqCst);
        }));

        let id = e.start("onboarding", serde_json::json!({})).unwrap();
        let kicks_after_start = kicks.load(Ordering::SeqCst);

        let status = e.run_to_pause(&id, 3).unwrap();
        // Budget exhausted mid-run: still Running, exactly 3 steps
        // executed, and the driver was re-kicked to continue on a fresh
        // job instead of monopolizing this worker.
        assert_eq!(status, WorkflowStatus::Running);
        assert_eq!(e.get(&id).unwrap().steps.len(), 3);
        assert_eq!(kicks.load(Ordering::SeqCst), kicks_after_start + 1);
    }

    #[test]
    fn engine_persists_transitions_through_the_attached_store() {
        let store = std::sync::Arc::new(crate::workflow_store::WorkflowStore::in_memory().unwrap());
        let e = engine_with_scripted_hook();
        e.attach_store(std::sync::Arc::clone(&store));

        let id = e.start("onboarding", serde_json::json!({"u": 1})).unwrap();
        // start() persisted the Pending instance.
        assert!(store.load(&id).unwrap().is_some());

        e.run_to_pause(&id, 50).unwrap();
        let persisted = store.load(&id).unwrap().unwrap();
        assert_eq!(persisted.status, WorkflowStatus::Completed);
        assert_eq!(persisted.steps.len(), 2);
    }

    // -----------------------------------------------------------------
    // Buffered events, wait timeouts, keys, cancel from code.
    // -----------------------------------------------------------------

    fn wait(e: &WorkflowEngine, id: &str, event: &str, timeout: Option<&str>) -> WorkflowStatus {
        let mut resp = serde_json::json!({"action": "wait_event", "event": event});
        if let Some(t) = timeout {
            resp["timeout"] = serde_json::json!(t);
        }
        e.advance_with_response(id, resp).unwrap()
    }

    #[test]
    fn event_sent_before_the_wait_is_consumed_by_the_wait() {
        let e = engine();
        let id = e.start("onboarding", serde_json::json!({})).unwrap();
        let d = e
            .send_event(&id, "seller_replied", serde_json::json!({"body": "yes"}))
            .unwrap();
        assert!(d.buffered && !d.delivered);

        let status = wait(&e, &id, "seller_replied", Some("60s"));
        assert_eq!(status, WorkflowStatus::Running);
        let inst = e.get(&id).unwrap();
        assert!(inst.pending_events.is_empty());
        assert_eq!(inst.current_step, 1);
        let last = inst.steps.last().unwrap();
        assert_eq!(last.name, "event:seller_replied");
        assert_eq!(last.output, Some(serde_json::json!({"body": "yes"})));
    }

    #[test]
    fn buffered_events_are_consumed_oldest_first_per_name() {
        let e = engine();
        let id = e.start("onboarding", serde_json::json!({})).unwrap();
        e.send_event(&id, "a", serde_json::json!({"n": 1})).unwrap();
        e.send_event(&id, "b", serde_json::json!({"n": 2})).unwrap();
        e.send_event(&id, "a", serde_json::json!({"n": 3})).unwrap();

        wait(&e, &id, "a", None);
        wait(&e, &id, "a", None);
        let inst = e.get(&id).unwrap();
        let outputs: Vec<_> = inst
            .steps
            .iter()
            .map(|s| s.output.clone().unwrap())
            .collect();
        assert_eq!(
            outputs,
            vec![serde_json::json!({"n": 1}), serde_json::json!({"n": 3})]
        );
        assert_eq!(inst.pending_events.len(), 1);
        assert_eq!(inst.pending_events[0].event, "b");
    }

    #[test]
    fn null_event_data_is_stored_as_an_empty_object() {
        let e = engine();
        let id = e.start("onboarding", serde_json::json!({})).unwrap();
        wait(&e, &id, "go", Some("1h"));
        e.send_event(&id, "go", serde_json::Value::Null).unwrap();
        let inst = e.get(&id).unwrap();
        assert_eq!(
            inst.steps.last().unwrap().output,
            Some(serde_json::json!({}))
        );
    }

    #[test]
    fn wait_timeout_sets_a_deadline_and_schedules_a_kick() {
        use std::sync::atomic::{AtomicU64, Ordering};
        let e = engine();
        let last_delay = std::sync::Arc::new(AtomicU64::new(u64::MAX));
        let d = std::sync::Arc::clone(&last_delay);
        e.set_kick_hook(Box::new(move |_id, delay| d.store(delay, Ordering::SeqCst)));
        let id = e.start("onboarding", serde_json::json!({})).unwrap();

        assert_eq!(
            wait(&e, &id, "reply", Some("60s")),
            WorkflowStatus::WaitingForEvent
        );
        let inst = e.get(&id).unwrap();
        let deadline = inst.wait_deadline.expect("deadline set");
        assert!(deadline.abs_diff(now_secs() + 60) <= 1);
        let delay = last_delay.load(Ordering::SeqCst);
        assert!((59..=60).contains(&delay), "delay {delay}");
    }

    #[test]
    fn expired_wait_resolves_as_a_timeout_and_the_run_continues() {
        let e = engine();
        e.set_runner_hook(std::sync::Arc::new(|request| {
            let steps = request["completed_steps"].as_array().unwrap();
            Ok(match steps.len() {
                0 => {
                    serde_json::json!({"action": "wait_event", "event": "reply", "timeout": "60s"})
                }
                _ => serde_json::json!({"action": "complete", "output": {"saw": steps[0]["name"]}}),
            })
        }));
        let id = e.start("onboarding", serde_json::json!({})).unwrap();
        assert_eq!(
            e.run_to_pause(&id, 10).unwrap(),
            WorkflowStatus::WaitingForEvent
        );

        // Not due yet: advancing does nothing.
        assert_eq!(
            e.run_to_pause(&id, 10).unwrap(),
            WorkflowStatus::WaitingForEvent
        );
        assert!(e.wake_sleeping().is_empty());

        // Move the deadline into the past, as if 60 s elapsed.
        e.instances
            .lock()
            .unwrap()
            .get_mut(&id)
            .unwrap()
            .wait_deadline = Some(now_secs() - 1);
        assert_eq!(e.wake_sleeping(), vec![id.clone()]);
        assert_eq!(e.run_to_pause(&id, 10).unwrap(), WorkflowStatus::Completed);
        let inst = e.get(&id).unwrap();
        assert_eq!(inst.steps[0].name, "timeout:reply");
        assert_eq!(inst.steps[0].output, None);
        assert_eq!(
            inst.output,
            Some(serde_json::json!({"saw": "timeout:reply"}))
        );
    }

    #[test]
    fn zero_timeout_resolves_immediately() {
        let e = engine();
        let id = e.start("onboarding", serde_json::json!({})).unwrap();
        assert_eq!(wait(&e, &id, "reply", Some("0s")), WorkflowStatus::Running);
        assert_eq!(e.get(&id).unwrap().steps[0].name, "timeout:reply");
    }

    #[test]
    fn invalid_timeout_fails_the_run_without_retries() {
        let e = engine();
        let id = e.start("onboarding", serde_json::json!({})).unwrap();
        assert_eq!(wait(&e, &id, "reply", Some("soon")), WorkflowStatus::Failed);
        let inst = e.get(&id).unwrap();
        assert!(inst.error.unwrap().contains("invalid waitForEvent timeout"));
    }

    #[test]
    fn event_sent_while_a_step_runs_is_not_lost() {
        // The Miles case: a seller replies while the run is inside a step.
        let e = std::sync::Arc::new(engine());
        let sender = std::sync::Arc::downgrade(&e);
        e.set_runner_hook(std::sync::Arc::new(move |request| {
            let id = request["workflow_id"].as_str().unwrap().to_string();
            let steps = request["completed_steps"].as_array().unwrap().len();
            Ok(match steps {
                0 => {
                    // The reply lands mid-step, before the run waits.
                    let e = sender.upgrade().unwrap();
                    let d = e.send_event(&id, "reply", serde_json::json!({"t": 1})).unwrap();
                    assert!(d.buffered);
                    serde_json::json!({"action": "step_complete", "step_name": "send_sms", "output": null})
                }
                1 => serde_json::json!({"action": "wait_event", "event": "reply", "timeout": "60s"}),
                _ => serde_json::json!({"action": "complete", "output": null}),
            })
        }));
        let id = e.start("onboarding", serde_json::json!({})).unwrap();
        assert_eq!(e.run_to_pause(&id, 10).unwrap(), WorkflowStatus::Completed);
        let names: Vec<_> = e
            .get(&id)
            .unwrap()
            .steps
            .iter()
            .map(|s| s.name.clone())
            .collect();
        assert_eq!(names, vec!["send_sms", "event:reply"]);
    }

    #[test]
    fn has_due_work_covers_buffered_matches_and_expired_waits() {
        let e = engine();
        let id = e.start("onboarding", serde_json::json!({})).unwrap();
        wait(&e, &id, "reply", Some("1h"));
        let now = now_secs();
        let mut inst = e.get(&id).unwrap();
        assert!(!inst.has_due_work(now));
        inst.pending_events.push(BufferedEvent {
            seq: 1,
            event: "other".into(),
            data: serde_json::json!({}),
            received_at: now_iso(),
        });
        assert!(!inst.has_due_work(now));
        inst.pending_events[0].event = "reply".into();
        assert!(inst.has_due_work(now));
        inst.pending_events.clear();
        inst.wait_deadline = Some(now);
        assert!(inst.has_due_work(now));
    }

    #[test]
    fn buffer_is_bounded() {
        let e = engine();
        let id = e.start("onboarding", serde_json::json!({})).unwrap();
        for i in 0..MAX_BUFFERED_EVENTS {
            e.send_event(&id, "x", serde_json::json!({"i": i})).unwrap();
        }
        let err = e.send_event(&id, "x", serde_json::json!({})).unwrap_err();
        assert!(err.contains("buffered events"), "{err}");
    }

    #[test]
    fn cancel_records_reason_and_reports_whether_it_changed_anything() {
        let e = engine();
        let id = e.start("onboarding", serde_json::json!({})).unwrap();
        wait(&e, &id, "reply", Some("1h"));
        assert!(e.cancel(&id, Some("seller texted STOP")).unwrap());
        let inst = e.get(&id).unwrap();
        assert_eq!(inst.status, WorkflowStatus::Cancelled);
        assert_eq!(inst.cancel_reason.as_deref(), Some("seller texted STOP"));
        assert_eq!(inst.wait_deadline, None);
        assert!(!e.cancel(&id, Some("again")).unwrap());
        assert_eq!(
            e.get(&id).unwrap().cancel_reason.as_deref(),
            Some("seller texted STOP")
        );
    }

    #[test]
    fn key_dedupes_active_runs_and_frees_up_when_the_run_ends() {
        let e = engine();
        let a = e
            .start_with_key("onboarding", serde_json::json!({}), Some("lead_1"))
            .unwrap();
        assert!(a.created);
        let b = e
            .start_with_key("onboarding", serde_json::json!({}), Some("lead_1"))
            .unwrap();
        assert_eq!(
            b,
            StartOutcome {
                id: a.id.clone(),
                created: false
            }
        );
        let other = e
            .start_with_key("onboarding", serde_json::json!({}), Some("lead_2"))
            .unwrap();
        assert!(other.created);

        e.cancel(&a.id, None).unwrap();
        let c = e
            .start_with_key("onboarding", serde_json::json!({}), Some("lead_1"))
            .unwrap();
        assert!(c.created);
        assert_ne!(c.id, a.id);
        assert!(e
            .start_with_key("onboarding", serde_json::json!({}), Some(""))
            .is_err());
    }

    #[test]
    fn list_filters_by_key_name_and_active_status() {
        let e = engine();
        let a = e
            .start_with_key("onboarding", serde_json::json!({}), Some("lead_1"))
            .unwrap();
        let _b = e
            .start_with_key("onboarding", serde_json::json!({}), Some("lead_2"))
            .unwrap();
        e.cancel(&a.id, None).unwrap();
        let c = e
            .start_with_key("onboarding", serde_json::json!({}), Some("lead_1"))
            .unwrap();

        let by_key = e
            .list_filtered(
                &WorkflowFilter {
                    key: Some("lead_1".into()),
                    ..Default::default()
                },
                false,
            )
            .unwrap();
        assert_eq!(by_key.len(), 2);
        let active = e
            .list_filtered(
                &WorkflowFilter {
                    key: Some("lead_1".into()),
                    status: Some(StatusFilter::Active),
                    ..Default::default()
                },
                false,
            )
            .unwrap();
        assert_eq!(
            active.iter().map(|i| i.id.clone()).collect::<Vec<_>>(),
            vec![c.id]
        );
        let none = e
            .list_filtered(
                &WorkflowFilter {
                    name: Some("other".into()),
                    ..Default::default()
                },
                false,
            )
            .unwrap();
        assert!(none.is_empty());
        assert_eq!(StatusFilter::parse("bogus"), None);
    }

    #[test]
    fn buffered_events_deadline_and_key_survive_restart() {
        let store = std::sync::Arc::new(crate::workflow_store::WorkflowStore::in_memory().unwrap());
        let e = engine();
        e.attach_store(std::sync::Arc::clone(&store));
        let id = e
            .start_with_key("onboarding", serde_json::json!({}), Some("lead_9"))
            .unwrap()
            .id;
        wait(&e, &id, "reply", Some("24h"));
        e.send_event(&id, "rep_took_over", serde_json::json!({"rep": "r1"}))
            .unwrap();
        let before = e.get(&id).unwrap();

        let restored = engine();
        assert_eq!(restored.restore_from(&store), 1);
        let after = restored.get(&id).unwrap();
        assert_eq!(after.status, WorkflowStatus::WaitingForEvent);
        assert_eq!(after.key.as_deref(), Some("lead_9"));
        assert_eq!(after.wait_deadline, before.wait_deadline);
        assert_eq!(after.pending_events, before.pending_events);

        e.cancel(&id, Some("booked")).unwrap();
        let loaded = store.load(&id).unwrap().unwrap();
        assert_eq!(loaded.cancel_reason.as_deref(), Some("booked"));

        // A recently finished run is listable after another restart.
        let again = engine();
        assert_eq!(again.restore_from(&store), 0);
        let listed = again
            .list_filtered(
                &WorkflowFilter {
                    key: Some("lead_9".into()),
                    ..Default::default()
                },
                false,
            )
            .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].status, WorkflowStatus::Cancelled);
    }

    #[test]
    fn parse_duration_strict_rejects_garbage() {
        assert_eq!(parse_duration_strict("60s"), Some(60));
        assert_eq!(parse_duration_strict("5m"), Some(300));
        assert_eq!(parse_duration_strict("24h"), Some(86400));
        assert_eq!(parse_duration_strict("7d"), Some(604800));
        assert_eq!(parse_duration_strict("90"), Some(90));
        assert_eq!(parse_duration_strict("abc"), None);
        assert_eq!(parse_duration_strict("5x"), None);
        assert_eq!(parse_duration_strict("-5s"), None);
        assert_eq!(parse_duration_strict(""), None);
        assert_eq!(parse_duration_strict("99999999999999999999d"), None);
    }

    fn pg_engine() -> Option<std::sync::Arc<WorkflowEngine>> {
        let url = std::env::var("PYLON_TEST_PG_URL").ok()?;
        let pool = pylon_storage::pg_datastore::PgPool::connect(
            &url,
            4,
            std::time::Duration::from_secs(5),
        )
        .expect("test Postgres pool");
        let store = crate::pg_workflow_store::PgWorkflowStore::open(
            pool,
            format!("engine_{}", pylon_cluster::new_instance_id()),
        )
        .expect("pg workflow store");
        let e = engine();
        e.attach_pg_store(std::sync::Arc::new(store));
        Some(std::sync::Arc::new(e))
    }

    #[test]
    fn postgres_event_sent_during_a_step_is_consumed_at_the_next_wait() {
        let Some(e) = pg_engine() else {
            eprintln!("skipping: PYLON_TEST_PG_URL not set");
            return;
        };
        let sender = std::sync::Arc::downgrade(&e);
        e.set_runner_hook(std::sync::Arc::new(move |request| {
            let id = request["workflow_id"].as_str().unwrap().to_string();
            let steps = request["completed_steps"].as_array().unwrap().len();
            Ok(match steps {
                0 => {
                    // The driver holds the lease here; the send must not fail.
                    let e = sender.upgrade().unwrap();
                    let d = e.send_event(&id, "reply", serde_json::json!({"t": 1})).unwrap();
                    assert!(d.buffered);
                    serde_json::json!({"action": "step_complete", "step_name": "send_sms", "output": null})
                }
                1 => serde_json::json!({"action": "wait_event", "event": "reply", "timeout": "60s"}),
                _ => serde_json::json!({"action": "complete", "output": null}),
            })
        }));
        let id = e.start("onboarding", serde_json::json!({})).unwrap();
        assert_eq!(e.run_to_pause(&id, 10).unwrap(), WorkflowStatus::Completed);
        let inst = e.get(&id).unwrap();
        let names: Vec<_> = inst.steps.iter().map(|s| s.name.clone()).collect();
        assert_eq!(names, vec!["send_sms", "event:reply"]);
        assert!(inst.pending_events.is_empty());
    }

    #[test]
    fn postgres_cancel_during_a_step_discards_the_step_result() {
        let Some(e) = pg_engine() else {
            eprintln!("skipping: PYLON_TEST_PG_URL not set");
            return;
        };
        let canceller = std::sync::Arc::downgrade(&e);
        e.set_runner_hook(std::sync::Arc::new(move |request| {
            let id = request["workflow_id"].as_str().unwrap().to_string();
            let e = canceller.upgrade().unwrap();
            assert!(e.cancel(&id, Some("STOP")).unwrap());
            Ok(serde_json::json!({"action": "step_complete", "step_name": "call", "output": null}))
        }));
        let id = e.start("onboarding", serde_json::json!({})).unwrap();
        assert_eq!(e.run_to_pause(&id, 10).unwrap(), WorkflowStatus::Cancelled);
        let inst = e.get(&id).unwrap();
        assert_eq!(inst.status, WorkflowStatus::Cancelled);
        assert!(inst.steps.is_empty());
        assert_eq!(inst.cancel_reason.as_deref(), Some("STOP"));
    }

    #[test]
    fn postgres_expired_wait_times_out() {
        let Some(e) = pg_engine() else {
            eprintln!("skipping: PYLON_TEST_PG_URL not set");
            return;
        };
        e.set_runner_hook(std::sync::Arc::new(|request| {
            let steps = request["completed_steps"].as_array().unwrap();
            Ok(match steps.len() {
                0 => serde_json::json!({"action": "wait_event", "event": "reply", "timeout": "0s"}),
                _ => serde_json::json!({"action": "complete", "output": steps[0]["name"]}),
            })
        }));
        let id = e.start("onboarding", serde_json::json!({})).unwrap();
        assert_eq!(e.run_to_pause(&id, 10).unwrap(), WorkflowStatus::Completed);
        assert_eq!(
            e.get(&id).unwrap().output,
            Some(serde_json::json!("timeout:reply"))
        );
    }

    #[test]
    fn runner_request_uses_the_ts_wire_format_for_step_status() {
        // The TS executor replays a step only when its record has
        // status "completed" (lowercase). Serde's "Completed" made every
        // multi-slice workflow fail with a replay mismatch.
        let seen = std::sync::Arc::new(Mutex::new(Vec::<serde_json::Value>::new()));
        let seen_ref = std::sync::Arc::clone(&seen);
        let e = engine();
        e.set_runner_hook(std::sync::Arc::new(move |request| {
            seen_ref.lock().unwrap().push(request.clone());
            let n = request["completed_steps"].as_array().unwrap().len();
            Ok(if n == 0 {
                serde_json::json!({"action": "step_complete", "step_name": "one", "output": 1})
            } else {
                serde_json::json!({"action": "complete", "output": null})
            })
        }));
        let id = e.start("onboarding", serde_json::json!({})).unwrap();
        e.run_to_pause(&id, 10).unwrap();
        let seen = seen.lock().unwrap();
        assert_eq!(seen[1]["completed_steps"][0]["status"], "completed");
        assert_eq!(seen[1]["completed_steps"][0]["name"], "one");
        assert_eq!(seen[1]["completed_steps"][0]["output"], 1);
    }
}
