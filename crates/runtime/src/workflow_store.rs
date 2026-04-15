//! SQLite-backed persistent storage for workflow instances.
//!
//! Persists workflow state (including step history) to a SQLite database so
//! active workflows survive server restarts. The in-memory engine remains the
//! source of truth at runtime; this store is written to on state changes and
//! read from at startup to restore unfinished work.

use rusqlite::Connection;
use std::sync::Mutex;

use crate::workflows::{StepResult, StepStatus, WorkflowInstance, WorkflowStatus};

/// SQLite-backed persistent storage for workflow instances.
pub struct WorkflowStore {
    conn: Mutex<Connection>,
}

impl WorkflowStore {
    /// Open or create the workflow store database at `path`.
    pub fn open(path: &str) -> Result<Self, String> {
        let conn = Connection::open(path)
            .map_err(|e| format!("Failed to open workflow store: {e}"))?;
        let store = Self {
            conn: Mutex::new(conn),
        };
        store.init_schema()?;
        Ok(store)
    }

    /// Create an in-memory store (useful for tests).
    pub fn in_memory() -> Result<Self, String> {
        let conn = Connection::open_in_memory()
            .map_err(|e| format!("Failed to open in-memory store: {e}"))?;
        let store = Self {
            conn: Mutex::new(conn),
        };
        store.init_schema()?;
        Ok(store)
    }

    fn init_schema(&self) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "
            PRAGMA journal_mode=WAL;
            CREATE TABLE IF NOT EXISTS workflows (
                id TEXT PRIMARY KEY NOT NULL,
                name TEXT NOT NULL,
                input TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'Pending',
                output TEXT,
                error TEXT,
                created_at TEXT NOT NULL,
                started_at TEXT,
                completed_at TEXT,
                wake_at INTEGER,
                waiting_for TEXT,
                current_step INTEGER NOT NULL DEFAULT 0,
                max_retries INTEGER NOT NULL DEFAULT 3
            );
            CREATE INDEX IF NOT EXISTS idx_wf_status ON workflows(status);

            CREATE TABLE IF NOT EXISTS workflow_steps (
                workflow_id TEXT NOT NULL,
                step_index INTEGER NOT NULL,
                step_id TEXT NOT NULL,
                name TEXT NOT NULL,
                status TEXT NOT NULL,
                output TEXT,
                error TEXT,
                started_at TEXT,
                completed_at TEXT,
                duration_ms INTEGER,
                retry_count INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (workflow_id, step_index),
                FOREIGN KEY (workflow_id) REFERENCES workflows(id) ON DELETE CASCADE
            );
        ",
        )
        .map_err(|e| format!("Schema init failed: {e}"))
    }

    /// Save a workflow instance (insert or update), including all steps.
    pub fn save(&self, wf: &WorkflowInstance) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();

        conn.execute(
            "INSERT OR REPLACE INTO workflows \
             (id, name, input, status, output, error, created_at, started_at, \
              completed_at, wake_at, waiting_for, current_step, max_retries) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            rusqlite::params![
                wf.id,
                wf.name,
                wf.input.to_string(),
                wf_status_to_str(&wf.status),
                wf.output.as_ref().map(|v| v.to_string()),
                wf.error,
                wf.created_at,
                wf.started_at,
                wf.completed_at,
                wf.wake_at.map(|v| v as i64),
                wf.waiting_for,
                wf.current_step as i64,
                wf.max_retries,
            ],
        )
        .map_err(|e| format!("Save workflow failed: {e}"))?;

        // Replace all steps: delete then re-insert.
        conn.execute(
            "DELETE FROM workflow_steps WHERE workflow_id = ?1",
            rusqlite::params![wf.id],
        )
        .map_err(|e| format!("Delete steps failed: {e}"))?;

        let mut stmt = conn
            .prepare(
                "INSERT INTO workflow_steps \
                 (workflow_id, step_index, step_id, name, status, output, error, \
                  started_at, completed_at, duration_ms, retry_count) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            )
            .map_err(|e| format!("Prepare step insert failed: {e}"))?;

        for (i, step) in wf.steps.iter().enumerate() {
            stmt.execute(rusqlite::params![
                wf.id,
                i as i64,
                step.step_id,
                step.name,
                step_status_to_str(&step.status),
                step.output.as_ref().map(|v| v.to_string()),
                step.error,
                step.started_at,
                step.completed_at,
                step.duration_ms.map(|v| v as i64),
                step.retry_count,
            ])
            .map_err(|e| format!("Insert step failed: {e}"))?;
        }

        Ok(())
    }

    /// Load a workflow instance by ID, including its steps.
    pub fn load(&self, id: &str) -> Result<Option<WorkflowInstance>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT id, name, input, status, output, error, created_at, \
                 started_at, completed_at, wake_at, waiting_for, current_step, max_retries \
                 FROM workflows WHERE id = ?1",
            )
            .map_err(|e| format!("Prepare failed: {e}"))?;

        let wf = stmt
            .query_row(rusqlite::params![id], |row| Ok(row_to_workflow(row)))
            .ok();

        match wf {
            Some(mut wf) => {
                wf.steps = load_steps(&conn, &wf.id)?;
                Ok(Some(wf))
            }
            None => Ok(None),
        }
    }

    /// Load all active workflows (Pending, Running, WaitingForEvent).
    pub fn load_active(&self) -> Result<Vec<WorkflowInstance>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT id, name, input, status, output, error, created_at, \
                 started_at, completed_at, wake_at, waiting_for, current_step, max_retries \
                 FROM workflows \
                 WHERE status IN ('Pending', 'Running', 'WaitingForEvent') \
                 ORDER BY created_at ASC",
            )
            .map_err(|e| format!("Prepare failed: {e}"))?;

        let rows = stmt
            .query_map([], |row| Ok(row_to_workflow(row)))
            .map_err(|e| format!("Query failed: {e}"))?;

        let mut workflows = Vec::new();
        for row in rows {
            if let Ok(mut wf) = row {
                wf.steps = load_steps(&conn, &wf.id)?;
                workflows.push(wf);
            }
        }
        Ok(workflows)
    }

    /// Load sleeping workflows.
    pub fn load_sleeping(&self) -> Result<Vec<WorkflowInstance>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT id, name, input, status, output, error, created_at, \
                 started_at, completed_at, wake_at, waiting_for, current_step, max_retries \
                 FROM workflows \
                 WHERE status = 'Sleeping' \
                 ORDER BY wake_at ASC",
            )
            .map_err(|e| format!("Prepare failed: {e}"))?;

        let rows = stmt
            .query_map([], |row| Ok(row_to_workflow(row)))
            .map_err(|e| format!("Query failed: {e}"))?;

        let mut workflows = Vec::new();
        for row in rows {
            if let Ok(mut wf) = row {
                wf.steps = load_steps(&conn, &wf.id)?;
                workflows.push(wf);
            }
        }
        Ok(workflows)
    }

    /// Count workflow instances by status.
    pub fn count_by_status(&self, status: &str) -> usize {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM workflows WHERE status = ?1",
            rusqlite::params![status],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0) as usize
    }
}

// ---------------------------------------------------------------------------
// Row mapping helpers
// ---------------------------------------------------------------------------

fn row_to_workflow(row: &rusqlite::Row<'_>) -> WorkflowInstance {
    WorkflowInstance {
        id: row.get(0).unwrap_or_default(),
        name: row.get(1).unwrap_or_default(),
        input: serde_json::from_str(&row.get::<_, String>(2).unwrap_or_default())
            .unwrap_or(serde_json::json!({})),
        status: str_to_wf_status(&row.get::<_, String>(3).unwrap_or_default()),
        output: row
            .get::<_, String>(4)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok()),
        error: row.get(5).ok(),
        created_at: row.get(6).unwrap_or_default(),
        started_at: row.get(7).ok(),
        completed_at: row.get(8).ok(),
        wake_at: row.get::<_, i64>(9).ok().map(|v| v as u64),
        waiting_for: row.get(10).ok(),
        current_step: row.get::<_, i64>(11).unwrap_or(0) as usize,
        max_retries: row.get(12).unwrap_or(3),
        steps: Vec::new(), // filled in by caller
    }
}

fn load_steps(conn: &Connection, workflow_id: &str) -> Result<Vec<StepResult>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT step_id, name, status, output, error, started_at, \
             completed_at, duration_ms, retry_count \
             FROM workflow_steps \
             WHERE workflow_id = ?1 \
             ORDER BY step_index ASC",
        )
        .map_err(|e| format!("Prepare steps failed: {e}"))?;

    let rows = stmt
        .query_map(rusqlite::params![workflow_id], |row| {
            Ok(StepResult {
                step_id: row.get(0).unwrap_or_default(),
                name: row.get(1).unwrap_or_default(),
                status: str_to_step_status(&row.get::<_, String>(2).unwrap_or_default()),
                output: row
                    .get::<_, String>(3)
                    .ok()
                    .and_then(|s| serde_json::from_str(&s).ok()),
                error: row.get(4).ok(),
                started_at: row.get(5).ok(),
                completed_at: row.get(6).ok(),
                duration_ms: row.get::<_, i64>(7).ok().map(|v| v as u64),
                retry_count: row.get(8).unwrap_or(0),
            })
        })
        .map_err(|e| format!("Query steps failed: {e}"))?;

    let mut steps = Vec::new();
    for row in rows {
        if let Ok(step) = row {
            steps.push(step);
        }
    }
    Ok(steps)
}

fn wf_status_to_str(s: &WorkflowStatus) -> &'static str {
    match s {
        WorkflowStatus::Pending => "Pending",
        WorkflowStatus::Running => "Running",
        WorkflowStatus::Sleeping => "Sleeping",
        WorkflowStatus::WaitingForEvent => "WaitingForEvent",
        WorkflowStatus::Completed => "Completed",
        WorkflowStatus::Failed => "Failed",
        WorkflowStatus::Cancelled => "Cancelled",
    }
}

fn str_to_wf_status(s: &str) -> WorkflowStatus {
    match s {
        "Pending" => WorkflowStatus::Pending,
        "Running" => WorkflowStatus::Running,
        "Sleeping" => WorkflowStatus::Sleeping,
        "WaitingForEvent" => WorkflowStatus::WaitingForEvent,
        "Completed" => WorkflowStatus::Completed,
        "Failed" => WorkflowStatus::Failed,
        "Cancelled" => WorkflowStatus::Cancelled,
        _ => WorkflowStatus::Pending,
    }
}

fn step_status_to_str(s: &StepStatus) -> &'static str {
    match s {
        StepStatus::Pending => "Pending",
        StepStatus::Running => "Running",
        StepStatus::Completed => "Completed",
        StepStatus::Failed => "Failed",
        StepStatus::Skipped => "Skipped",
    }
}

fn str_to_step_status(s: &str) -> StepStatus {
    match s {
        "Pending" => StepStatus::Pending,
        "Running" => StepStatus::Running,
        "Completed" => StepStatus::Completed,
        "Failed" => StepStatus::Failed,
        "Skipped" => StepStatus::Skipped,
        _ => StepStatus::Pending,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_workflow(id: &str, status: WorkflowStatus) -> WorkflowInstance {
        WorkflowInstance {
            id: id.to_string(),
            name: "onboarding".to_string(),
            input: serde_json::json!({"user": "alice"}),
            status,
            steps: Vec::new(),
            output: None,
            error: None,
            created_at: "1000Z".to_string(),
            started_at: None,
            completed_at: None,
            wake_at: None,
            waiting_for: None,
            current_step: 0,
            max_retries: 3,
        }
    }

    fn make_step(name: &str, status: StepStatus) -> StepResult {
        StepResult {
            step_id: format!("step_{name}"),
            name: name.to_string(),
            status,
            output: Some(serde_json::json!({"result": name})),
            error: None,
            started_at: Some("1000Z".into()),
            completed_at: Some("1001Z".into()),
            duration_ms: Some(42),
            retry_count: 0,
        }
    }

    #[test]
    fn in_memory_opens_without_error() {
        let store = WorkflowStore::in_memory().unwrap();
        assert_eq!(store.count_by_status("Pending"), 0);
    }

    #[test]
    fn save_and_load_roundtrip_without_steps() {
        let store = WorkflowStore::in_memory().unwrap();

        let mut wf = make_workflow("wf_1", WorkflowStatus::Running);
        wf.started_at = Some("1500Z".into());
        wf.current_step = 2;
        wf.max_retries = 5;

        store.save(&wf).unwrap();

        let loaded = store.load("wf_1").unwrap().unwrap();
        assert_eq!(loaded.id, "wf_1");
        assert_eq!(loaded.name, "onboarding");
        assert_eq!(loaded.input, serde_json::json!({"user": "alice"}));
        assert_eq!(loaded.status, WorkflowStatus::Running);
        assert_eq!(loaded.current_step, 2);
        assert_eq!(loaded.max_retries, 5);
        assert_eq!(loaded.started_at, Some("1500Z".into()));
        assert!(loaded.steps.is_empty());
    }

    #[test]
    fn save_and_load_roundtrip_with_steps() {
        let store = WorkflowStore::in_memory().unwrap();

        let mut wf = make_workflow("wf_2", WorkflowStatus::Running);
        wf.steps = vec![
            make_step("create_account", StepStatus::Completed),
            make_step("send_email", StepStatus::Failed),
        ];
        wf.steps[1].error = Some("SMTP timeout".into());
        wf.steps[1].retry_count = 2;
        wf.current_step = 1;

        store.save(&wf).unwrap();

        let loaded = store.load("wf_2").unwrap().unwrap();
        assert_eq!(loaded.steps.len(), 2);

        assert_eq!(loaded.steps[0].name, "create_account");
        assert_eq!(loaded.steps[0].status, StepStatus::Completed);
        assert_eq!(loaded.steps[0].output, Some(serde_json::json!({"result": "create_account"})));
        assert_eq!(loaded.steps[0].duration_ms, Some(42));

        assert_eq!(loaded.steps[1].name, "send_email");
        assert_eq!(loaded.steps[1].status, StepStatus::Failed);
        assert_eq!(loaded.steps[1].error, Some("SMTP timeout".into()));
        assert_eq!(loaded.steps[1].retry_count, 2);
    }

    #[test]
    fn save_updates_existing_workflow() {
        let store = WorkflowStore::in_memory().unwrap();

        let mut wf = make_workflow("wf_3", WorkflowStatus::Pending);
        store.save(&wf).unwrap();

        wf.status = WorkflowStatus::Running;
        wf.started_at = Some("2000Z".into());
        wf.steps.push(make_step("step_a", StepStatus::Completed));
        store.save(&wf).unwrap();

        let loaded = store.load("wf_3").unwrap().unwrap();
        assert_eq!(loaded.status, WorkflowStatus::Running);
        assert_eq!(loaded.steps.len(), 1);
    }

    #[test]
    fn load_nonexistent_returns_none() {
        let store = WorkflowStore::in_memory().unwrap();
        assert!(store.load("nonexistent").unwrap().is_none());
    }

    #[test]
    fn load_active_returns_pending_running_waiting() {
        let store = WorkflowStore::in_memory().unwrap();

        store.save(&make_workflow("wf_pending", WorkflowStatus::Pending)).unwrap();
        store.save(&make_workflow("wf_running", WorkflowStatus::Running)).unwrap();
        store.save(&make_workflow("wf_waiting", WorkflowStatus::WaitingForEvent)).unwrap();
        store.save(&make_workflow("wf_sleeping", WorkflowStatus::Sleeping)).unwrap();
        store.save(&make_workflow("wf_completed", WorkflowStatus::Completed)).unwrap();
        store.save(&make_workflow("wf_failed", WorkflowStatus::Failed)).unwrap();
        store.save(&make_workflow("wf_cancelled", WorkflowStatus::Cancelled)).unwrap();

        let active = store.load_active().unwrap();
        assert_eq!(active.len(), 3);
        let ids: Vec<&str> = active.iter().map(|w| w.id.as_str()).collect();
        assert!(ids.contains(&"wf_pending"));
        assert!(ids.contains(&"wf_running"));
        assert!(ids.contains(&"wf_waiting"));
    }

    #[test]
    fn load_sleeping_returns_only_sleeping() {
        let store = WorkflowStore::in_memory().unwrap();

        let mut sleeping = make_workflow("wf_sleep", WorkflowStatus::Sleeping);
        sleeping.wake_at = Some(99999);
        store.save(&sleeping).unwrap();

        store.save(&make_workflow("wf_run", WorkflowStatus::Running)).unwrap();

        let result = store.load_sleeping().unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "wf_sleep");
        assert_eq!(result[0].wake_at, Some(99999));
    }

    #[test]
    fn count_by_status_counts_correctly() {
        let store = WorkflowStore::in_memory().unwrap();

        store.save(&make_workflow("w1", WorkflowStatus::Pending)).unwrap();
        store.save(&make_workflow("w2", WorkflowStatus::Pending)).unwrap();
        store.save(&make_workflow("w3", WorkflowStatus::Running)).unwrap();
        store.save(&make_workflow("w4", WorkflowStatus::Completed)).unwrap();

        assert_eq!(store.count_by_status("Pending"), 2);
        assert_eq!(store.count_by_status("Running"), 1);
        assert_eq!(store.count_by_status("Completed"), 1);
        assert_eq!(store.count_by_status("Failed"), 0);
    }

    #[test]
    fn sleeping_workflow_with_output_roundtrips() {
        let store = WorkflowStore::in_memory().unwrap();

        let mut wf = make_workflow("wf_out", WorkflowStatus::Completed);
        wf.output = Some(serde_json::json!({"final": "result"}));
        wf.error = Some("partial failure".into());
        wf.completed_at = Some("5000Z".into());
        wf.waiting_for = Some("user_confirmed".into());

        store.save(&wf).unwrap();

        let loaded = store.load("wf_out").unwrap().unwrap();
        assert_eq!(loaded.output, Some(serde_json::json!({"final": "result"})));
        assert_eq!(loaded.error, Some("partial failure".into()));
        assert_eq!(loaded.completed_at, Some("5000Z".into()));
        assert_eq!(loaded.waiting_for, Some("user_confirmed".into()));
    }

    #[test]
    fn all_statuses_roundtrip() {
        let store = WorkflowStore::in_memory().unwrap();
        let statuses = [
            WorkflowStatus::Pending,
            WorkflowStatus::Running,
            WorkflowStatus::Sleeping,
            WorkflowStatus::WaitingForEvent,
            WorkflowStatus::Completed,
            WorkflowStatus::Failed,
            WorkflowStatus::Cancelled,
        ];
        for (i, status) in statuses.iter().enumerate() {
            let wf = make_workflow(&format!("wf_{i}"), status.clone());
            store.save(&wf).unwrap();
            let loaded = store.load(&format!("wf_{i}")).unwrap().unwrap();
            assert_eq!(loaded.status, *status);
        }
    }

    #[test]
    fn all_step_statuses_roundtrip() {
        let store = WorkflowStore::in_memory().unwrap();
        let step_statuses = [
            StepStatus::Pending,
            StepStatus::Running,
            StepStatus::Completed,
            StepStatus::Failed,
            StepStatus::Skipped,
        ];

        let mut wf = make_workflow("wf_steps", WorkflowStatus::Running);
        for (i, status) in step_statuses.iter().enumerate() {
            wf.steps.push(make_step(&format!("s{i}"), status.clone()));
        }
        store.save(&wf).unwrap();

        let loaded = store.load("wf_steps").unwrap().unwrap();
        for (i, status) in step_statuses.iter().enumerate() {
            assert_eq!(loaded.steps[i].status, *status);
        }
    }
}
