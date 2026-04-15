use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
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
}

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

pub struct WorkflowEngine {
    /// Registered workflow definitions.
    definitions: Mutex<HashMap<String, WorkflowDef>>,
    /// Active and historical workflow instances.
    instances: Mutex<HashMap<String, WorkflowInstance>>,
    /// URL of the TypeScript workflow runner.
    runner_url: String,
    /// Max instances to keep in history (unused currently, reserved for GC).
    #[allow(dead_code)]
    max_history: usize,
}

impl WorkflowEngine {
    pub fn new(runner_url: &str, max_history: usize) -> Self {
        Self {
            definitions: Mutex::new(HashMap::new()),
            instances: Mutex::new(HashMap::new()),
            runner_url: runner_url.to_string(),
            max_history,
        }
    }

    /// Register a workflow definition.
    pub fn register(&self, def: WorkflowDef) {
        self.definitions
            .lock()
            .unwrap()
            .insert(def.name.clone(), def);
    }

    /// Start a new workflow instance. Returns the instance ID.
    pub fn start(
        &self,
        name: &str,
        input: serde_json::Value,
    ) -> Result<String, String> {
        let defs = self.definitions.lock().unwrap();
        let def = defs
            .get(name)
            .ok_or_else(|| format!("Workflow '{}' not registered", name))?;

        let id = generate_workflow_id();
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
            max_retries: def.max_retries,
        };

        self.instances.lock().unwrap().insert(id.clone(), instance);
        Ok(id)
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
        let instance = {
            let instances = self.instances.lock().unwrap();
            instances
                .get(workflow_id)
                .cloned()
                .ok_or_else(|| format!("Workflow '{}' not found", workflow_id))?
        };

        // Terminal states: nothing to do.
        match instance.status {
            WorkflowStatus::Completed
            | WorkflowStatus::Failed
            | WorkflowStatus::Cancelled => {
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
            _ => {}
        }

        let request = serde_json::json!({
            "workflow_id": workflow_id,
            "workflow_name": instance.name,
            "input": instance.input,
            "current_step": instance.current_step,
            "completed_steps": instance.steps,
        });

        let response = self.call_runner(&request)?;
        self.apply_response(workflow_id, &response)
    }

    /// Advance a workflow with a pre-provided response (for testing without
    /// a running TS runner).
    pub fn advance_with_response(
        &self,
        workflow_id: &str,
        response: serde_json::Value,
    ) -> Result<WorkflowStatus, String> {
        // Verify the workflow exists and is advanceable.
        {
            let instances = self.instances.lock().unwrap();
            let instance = instances
                .get(workflow_id)
                .ok_or_else(|| format!("Workflow '{}' not found", workflow_id))?;

            match instance.status {
                WorkflowStatus::Completed
                | WorkflowStatus::Failed
                | WorkflowStatus::Cancelled => {
                    return Ok(instance.status.clone());
                }
                _ => {}
            }
        }

        self.apply_response(workflow_id, &response)
    }

    /// Send an event to a waiting workflow.
    pub fn send_event(
        &self,
        workflow_id: &str,
        event: &str,
        data: serde_json::Value,
    ) -> Result<(), String> {
        let mut instances = self.instances.lock().unwrap();
        let inst = instances
            .get_mut(workflow_id)
            .ok_or("Workflow not found")?;

        if inst.status != WorkflowStatus::WaitingForEvent {
            return Err("Workflow is not waiting for an event".into());
        }

        if inst.waiting_for.as_deref() != Some(event) {
            return Err(format!(
                "Workflow is waiting for '{}', not '{event}'",
                inst.waiting_for.as_deref().unwrap_or("")
            ));
        }

        inst.steps.push(StepResult {
            step_id: format!("step_{}", inst.steps.len()),
            name: format!("event:{event}"),
            status: StepStatus::Completed,
            output: Some(data),
            error: None,
            started_at: Some(now_iso()),
            completed_at: Some(now_iso()),
            duration_ms: None,
            retry_count: 0,
        });
        inst.current_step += 1;
        inst.status = WorkflowStatus::Running;
        inst.waiting_for = None;

        Ok(())
    }

    /// Cancel a workflow.
    pub fn cancel(&self, workflow_id: &str) -> Result<(), String> {
        let mut instances = self.instances.lock().unwrap();
        let inst = instances
            .get_mut(workflow_id)
            .ok_or("Workflow not found")?;
        inst.status = WorkflowStatus::Cancelled;
        inst.completed_at = Some(now_iso());
        Ok(())
    }

    /// Get a workflow instance by ID.
    pub fn get(&self, workflow_id: &str) -> Option<WorkflowInstance> {
        self.instances.lock().unwrap().get(workflow_id).cloned()
    }

    /// List all workflow instances with optional status filter.
    pub fn list(&self, status: Option<&WorkflowStatus>) -> Vec<WorkflowInstance> {
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

    /// List registered workflow definitions.
    pub fn definitions(&self) -> Vec<WorkflowDef> {
        self.definitions.lock().unwrap().values().cloned().collect()
    }

    /// Wake sleeping workflows whose timer has expired. Returns the IDs of
    /// workflows that were woken.
    pub fn wake_sleeping(&self) -> Vec<String> {
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

        woken
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

        let mut instances = self.instances.lock().unwrap();
        let inst = instances
            .get_mut(workflow_id)
            .ok_or_else(|| format!("Workflow '{}' not found", workflow_id))?;

        if inst.started_at.is_none() {
            inst.started_at = Some(now_iso());
        }

        match action {
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
                    duration_ms: response
                        .get("duration_ms")
                        .and_then(|v| v.as_u64()),
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
            "wait_event" => {
                let event = response
                    .get("event")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                inst.status = WorkflowStatus::WaitingForEvent;
                inst.waiting_for = Some(event);

                Ok(WorkflowStatus::WaitingForEvent)
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
        }
    }

    /// Call the TypeScript workflow runner via HTTP.
    fn call_runner(
        &self,
        request: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
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
        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .ok();
        stream
            .write_all(http_request.as_bytes())
            .map_err(|e| format!("Write failed: {e}"))?;

        let mut response = String::new();
        stream.read_to_string(&mut response).ok();

        let body = response.split("\r\n\r\n").nth(1).unwrap_or("{}");
        serde_json::from_str(body)
            .map_err(|e| format!("Failed to parse runner response: {e}"))
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

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
        let err = e
            .start("nonexistent", serde_json::json!({}))
            .unwrap_err();
        assert!(err.contains("not registered"));
    }

    // -- Step recording via advance_with_response ---------------------------

    #[test]
    fn step_complete_advances_workflow() {
        let e = engine();
        let id = e
            .start("onboarding", serde_json::json!({}))
            .unwrap();

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
        assert_eq!(inst.steps[0].output, Some(serde_json::json!({"account_id": 42})));
        assert_eq!(inst.steps[0].duration_ms, Some(120));
        assert!(inst.started_at.is_some());
    }

    #[test]
    fn multiple_steps_advance_sequentially() {
        let e = engine();
        let id = e
            .start("onboarding", serde_json::json!({}))
            .unwrap();

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
        let id = e
            .start("onboarding", serde_json::json!({}))
            .unwrap();

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
        let id = e
            .start("onboarding", serde_json::json!({}))
            .unwrap();

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
        let id = e
            .start("onboarding", serde_json::json!({}))
            .unwrap();

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
        let id = e
            .start("onboarding", serde_json::json!({}))
            .unwrap();

        let status = e
            .advance_with_response(
                &id,
                serde_json::json!({"action": "wait_event", "event": "user_confirmed"}),
            )
            .unwrap();
        assert_eq!(status, WorkflowStatus::WaitingForEvent);

        e.send_event(&id, "user_confirmed", serde_json::json!({"confirmed": true}))
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
    fn send_event_wrong_name_errors() {
        let e = engine();
        let id = e
            .start("onboarding", serde_json::json!({}))
            .unwrap();

        e.advance_with_response(
            &id,
            serde_json::json!({"action": "wait_event", "event": "user_confirmed"}),
        )
        .unwrap();

        let err = e
            .send_event(&id, "wrong_event", serde_json::json!({}))
            .unwrap_err();
        assert!(err.contains("waiting for 'user_confirmed'"));
    }

    #[test]
    fn send_event_not_waiting_errors() {
        let e = engine();
        let id = e
            .start("onboarding", serde_json::json!({}))
            .unwrap();

        let err = e
            .send_event(&id, "anything", serde_json::json!({}))
            .unwrap_err();
        assert!(err.contains("not waiting"));
    }

    // -- Cancel -------------------------------------------------------------

    #[test]
    fn cancel_sets_status_and_completed_at() {
        let e = engine();
        let id = e
            .start("onboarding", serde_json::json!({}))
            .unwrap();

        e.cancel(&id).unwrap();

        let inst = e.get(&id).unwrap();
        assert_eq!(inst.status, WorkflowStatus::Cancelled);
        assert!(inst.completed_at.is_some());
    }

    #[test]
    fn cancel_unknown_workflow_errors() {
        let e = engine();
        let err = e.cancel("wf_nonexistent").unwrap_err();
        assert!(err.contains("not found"));
    }

    // -- Completion ---------------------------------------------------------

    #[test]
    fn complete_sets_output_and_status() {
        let e = engine();
        let id = e
            .start("onboarding", serde_json::json!({}))
            .unwrap();

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
        let id = e
            .start("onboarding", serde_json::json!({}))
            .unwrap();

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
        let id = e
            .start("onboarding", serde_json::json!({}))
            .unwrap();

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
            assert_eq!(status, WorkflowStatus::Running, "retry {i} should keep running");
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
        let id = e
            .start("onboarding", serde_json::json!({}))
            .unwrap();

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
        let id1 = e
            .start("onboarding", serde_json::json!({}))
            .unwrap();
        let _id2 = e
            .start("onboarding", serde_json::json!({}))
            .unwrap();

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
        let id = e
            .start("onboarding", serde_json::json!({}))
            .unwrap();

        let err = e
            .advance_with_response(
                &id,
                serde_json::json!({"action": "bogus"}),
            )
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
}
