//! Shared Postgres persistence and leases for workflow instances.

use std::sync::Arc;

use pylon_storage::pg_datastore::PgPool;

use crate::workflows::{StepResult, StepStatus, WorkflowInstance, WorkflowStatus};

const WORKFLOW_SCHEMA_LOCK_ID: i64 = 0x5059_5746;

pub struct PgWorkflowStore {
    pool: Arc<PgPool>,
    owner: String,
}

impl PgWorkflowStore {
    pub fn open(pool: Arc<PgPool>, owner: String) -> Result<Self, String> {
        let store = Self { pool, owner };
        store.init_schema()?;
        Ok(store)
    }

    fn init_schema(&self) -> Result<(), String> {
        self.pool.with_client(|client| {
            let mut tx = client.transaction()?;
            tx.execute(
                "SELECT pg_advisory_xact_lock($1)",
                &[&WORKFLOW_SCHEMA_LOCK_ID],
            )?;
            tx.batch_execute(
                "CREATE TABLE IF NOT EXISTS _pylon_workflows (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    input JSONB NOT NULL,
                    status TEXT NOT NULL DEFAULT 'Pending',
                    output JSONB,
                    error TEXT,
                    created_at BIGINT NOT NULL,
                    started_at BIGINT,
                    completed_at BIGINT,
                    wake_at BIGINT,
                    waiting_for TEXT,
                    current_step BIGINT NOT NULL DEFAULT 0,
                    max_retries INTEGER NOT NULL DEFAULT 3,
                    version BIGINT NOT NULL DEFAULT 0,
                    lease_owner TEXT,
                    lease_token TEXT,
                    lease_expires_at BIGINT
                );
                CREATE INDEX IF NOT EXISTS _pylon_workflows_status_idx
                    ON _pylon_workflows (status, wake_at, created_at);
                CREATE INDEX IF NOT EXISTS _pylon_workflows_lease_idx
                    ON _pylon_workflows (lease_expires_at)
                    WHERE lease_token IS NOT NULL;
                CREATE TABLE IF NOT EXISTS _pylon_workflow_steps (
                    workflow_id TEXT NOT NULL REFERENCES _pylon_workflows(id) ON DELETE CASCADE,
                    step_index BIGINT NOT NULL,
                    step_id TEXT NOT NULL,
                    name TEXT NOT NULL,
                    status TEXT NOT NULL,
                    output JSONB,
                    error TEXT,
                    started_at BIGINT,
                    completed_at BIGINT,
                    duration_ms BIGINT,
                    retry_count INTEGER NOT NULL DEFAULT 0,
                    PRIMARY KEY (workflow_id, step_index)
                );",
            )?;
            tx.commit()
        })
    }

    pub fn save(&self, workflow: &WorkflowInstance) -> Result<(), String> {
        self.save_inner(workflow, None)
    }

    pub fn save_owned(&self, workflow: &WorkflowInstance, token: &str) -> Result<(), String> {
        self.save_inner(workflow, Some(token))
    }

    fn save_inner(&self, workflow: &WorkflowInstance, token: Option<&str>) -> Result<(), String> {
        let status = workflow_status_to_str(&workflow.status);
        let created_at = parse_stamp_i64(&workflow.created_at);
        let started_at = workflow.started_at.as_deref().map(parse_stamp_i64);
        let completed_at = workflow.completed_at.as_deref().map(parse_stamp_i64);
        let wake_at = workflow.wake_at.map(|v| v.min(i64::MAX as u64) as i64);
        let current_step = workflow.current_step.min(i64::MAX as usize) as i64;
        let max_retries = workflow.max_retries.min(i32::MAX as u32) as i32;
        let saved = self.pool.with_client(|client| {
            let mut tx = client.transaction()?;
            let changed = if let Some(token) = token {
                tx.execute(
                    "UPDATE _pylon_workflows SET
                         name=$2,input=$3,status=$4,output=$5,error=$6,created_at=$7,
                         started_at=$8,completed_at=$9,wake_at=$10,waiting_for=$11,
                         current_step=$12,max_retries=$13,version=version+1
                     WHERE id=$1 AND lease_token=$14",
                    &[
                        &workflow.id,
                        &workflow.name,
                        &workflow.input,
                        &status,
                        &workflow.output,
                        &workflow.error,
                        &created_at,
                        &started_at,
                        &completed_at,
                        &wake_at,
                        &workflow.waiting_for,
                        &current_step,
                        &max_retries,
                        &token,
                    ],
                )?
            } else {
                tx.execute(
                    "INSERT INTO _pylon_workflows
                     (id,name,input,status,output,error,created_at,started_at,completed_at,
                      wake_at,waiting_for,current_step,max_retries)
                     VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)
                     ON CONFLICT (id) DO UPDATE SET
                         name=EXCLUDED.name,input=EXCLUDED.input,status=EXCLUDED.status,
                         output=EXCLUDED.output,error=EXCLUDED.error,
                         started_at=EXCLUDED.started_at,completed_at=EXCLUDED.completed_at,
                         wake_at=EXCLUDED.wake_at,waiting_for=EXCLUDED.waiting_for,
                         current_step=EXCLUDED.current_step,max_retries=EXCLUDED.max_retries,
                         version=_pylon_workflows.version+1",
                    &[
                        &workflow.id,
                        &workflow.name,
                        &workflow.input,
                        &status,
                        &workflow.output,
                        &workflow.error,
                        &created_at,
                        &started_at,
                        &completed_at,
                        &wake_at,
                        &workflow.waiting_for,
                        &current_step,
                        &max_retries,
                    ],
                )?
            };
            if changed != 1 {
                tx.rollback()?;
                return Ok(false);
            }
            tx.execute(
                "DELETE FROM _pylon_workflow_steps WHERE workflow_id=$1",
                &[&workflow.id],
            )?;
            for (index, step) in workflow.steps.iter().enumerate() {
                insert_step(&mut tx, &workflow.id, index, step)?;
            }
            tx.commit()?;
            Ok(true)
        })?;
        if !saved {
            return Err("workflow lease is no longer owned".into());
        }
        Ok(())
    }

    pub fn load(&self, id: &str) -> Result<Option<WorkflowInstance>, String> {
        self.pool.with_client(|client| {
            let row = client.query_opt(
                "SELECT id,name,input,status,output,error,created_at,started_at,
                        completed_at,wake_at,waiting_for,current_step,max_retries
                 FROM _pylon_workflows WHERE id=$1",
                &[&id],
            )?;
            match row {
                Some(row) => {
                    let mut workflow = row_to_workflow(&row);
                    workflow.steps = load_steps(client, id)?;
                    Ok(Some(workflow))
                }
                None => Ok(None),
            }
        })
    }

    pub fn list(&self, status: Option<&str>) -> Result<Vec<WorkflowInstance>, String> {
        self.pool.with_client(|client| {
            let rows = client.query(
                "SELECT id,name,input,status,output,error,created_at,started_at,
                        completed_at,wake_at,waiting_for,current_step,max_retries
                 FROM _pylon_workflows
                 WHERE ($1::TEXT IS NULL OR status=$1)
                 ORDER BY created_at DESC",
                &[&status],
            )?;
            let mut workflows = Vec::with_capacity(rows.len());
            for row in rows {
                let mut workflow = row_to_workflow(&row);
                workflow.steps = load_steps(client, &workflow.id)?;
                workflows.push(workflow);
            }
            Ok(workflows)
        })
    }

    pub fn due_sleeping_ids(&self, now: u64) -> Result<Vec<String>, String> {
        let now = now.min(i64::MAX as u64) as i64;
        self.pool.with_client(|client| {
            client
                .query(
                    "SELECT id FROM _pylon_workflows
                     WHERE status='Sleeping' AND wake_at IS NOT NULL AND wake_at <= $1",
                    &[&now],
                )
                .map(|rows| rows.into_iter().map(|row| row.get(0)).collect())
        })
    }

    pub fn runnable_ids(&self) -> Result<Vec<String>, String> {
        self.pool.with_client(|client| {
            client
                .query(
                    "SELECT id FROM _pylon_workflows
                     WHERE status IN ('Pending','Running')",
                    &[],
                )
                .map(|rows| rows.into_iter().map(|row| row.get(0)).collect())
        })
    }

    pub fn try_acquire(&self, id: &str, token: &str, lease_secs: u64) -> Result<bool, String> {
        let lease_secs = lease_secs.min(i64::MAX as u64) as i64;
        self.pool.with_client(|client| {
            client
                .execute(
                    "UPDATE _pylon_workflows
                     SET lease_owner=$2,lease_token=$3,
                         lease_expires_at=EXTRACT(EPOCH FROM clock_timestamp())::BIGINT+$4
                     WHERE id=$1 AND (
                         lease_token IS NULL OR
                         lease_expires_at < EXTRACT(EPOCH FROM clock_timestamp())::BIGINT
                     )",
                    &[&id, &self.owner, &token, &lease_secs],
                )
                .map(|n| n == 1)
        })
    }

    pub fn heartbeat(&self, id: &str, token: &str, lease_secs: u64) -> Result<bool, String> {
        let lease_secs = lease_secs.min(i64::MAX as u64) as i64;
        self.pool.with_client(|client| {
            client
                .execute(
                    "UPDATE _pylon_workflows
                     SET lease_expires_at=EXTRACT(EPOCH FROM clock_timestamp())::BIGINT+$3
                     WHERE id=$1 AND lease_token=$2",
                    &[&id, &token, &lease_secs],
                )
                .map(|n| n == 1)
        })
    }

    pub fn release(&self, id: &str, token: &str) -> Result<bool, String> {
        self.pool.with_client(|client| {
            client
                .execute(
                    "UPDATE _pylon_workflows
                     SET lease_owner=NULL,lease_token=NULL,lease_expires_at=NULL
                     WHERE id=$1 AND lease_token=$2",
                    &[&id, &token],
                )
                .map(|n| n == 1)
        })
    }

    pub fn cleanup_terminal(&self, max_age_secs: u64) -> Result<usize, String> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let cutoff = now.saturating_sub(max_age_secs).min(i64::MAX as u64) as i64;
        self.pool.with_client(|client| {
            client
                .execute(
                    "DELETE FROM _pylon_workflows
                     WHERE status IN ('Completed','Failed','Cancelled')
                       AND completed_at IS NOT NULL AND completed_at < $1",
                    &[&cutoff],
                )
                .map(|n| n as usize)
        })
    }
}

fn insert_step(
    tx: &mut postgres::Transaction<'_>,
    workflow_id: &str,
    index: usize,
    step: &StepResult,
) -> Result<(), postgres::Error> {
    let index = index.min(i64::MAX as usize) as i64;
    let status = step_status_to_str(&step.status);
    let started_at = step.started_at.as_deref().map(parse_stamp_i64);
    let completed_at = step.completed_at.as_deref().map(parse_stamp_i64);
    let duration_ms = step.duration_ms.map(|v| v.min(i64::MAX as u64) as i64);
    let retry_count = step.retry_count.min(i32::MAX as u32) as i32;
    tx.execute(
        "INSERT INTO _pylon_workflow_steps
         (workflow_id,step_index,step_id,name,status,output,error,started_at,
          completed_at,duration_ms,retry_count)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)",
        &[
            &workflow_id,
            &index,
            &step.step_id,
            &step.name,
            &status,
            &step.output,
            &step.error,
            &started_at,
            &completed_at,
            &duration_ms,
            &retry_count,
        ],
    )?;
    Ok(())
}

fn load_steps(
    client: &mut postgres::Client,
    workflow_id: &str,
) -> Result<Vec<StepResult>, postgres::Error> {
    client
        .query(
            "SELECT step_id,name,status,output,error,started_at,completed_at,
                    duration_ms,retry_count
             FROM _pylon_workflow_steps WHERE workflow_id=$1 ORDER BY step_index",
            &[&workflow_id],
        )
        .map(|rows| rows.iter().map(row_to_step).collect())
}

fn row_to_workflow(row: &postgres::Row) -> WorkflowInstance {
    WorkflowInstance {
        id: row.get(0),
        name: row.get(1),
        input: row.get(2),
        status: workflow_status_from_str(row.get::<_, String>(3).as_str()),
        output: row.get(4),
        error: row.get(5),
        created_at: stamp(row.get(6)),
        started_at: row.get::<_, Option<i64>>(7).map(stamp),
        completed_at: row.get::<_, Option<i64>>(8).map(stamp),
        wake_at: row.get::<_, Option<i64>>(9).map(|v| v.max(0) as u64),
        waiting_for: row.get(10),
        current_step: row.get::<_, i64>(11).max(0) as usize,
        max_retries: row.get::<_, i32>(12).max(0) as u32,
        steps: Vec::new(),
    }
}

fn row_to_step(row: &postgres::Row) -> StepResult {
    StepResult {
        step_id: row.get(0),
        name: row.get(1),
        status: step_status_from_str(row.get::<_, String>(2).as_str()),
        output: row.get(3),
        error: row.get(4),
        started_at: row.get::<_, Option<i64>>(5).map(stamp),
        completed_at: row.get::<_, Option<i64>>(6).map(stamp),
        duration_ms: row.get::<_, Option<i64>>(7).map(|v| v.max(0) as u64),
        retry_count: row.get::<_, i32>(8).max(0) as u32,
    }
}

fn workflow_status_from_str(value: &str) -> WorkflowStatus {
    match value {
        "Running" => WorkflowStatus::Running,
        "Sleeping" => WorkflowStatus::Sleeping,
        "WaitingForEvent" => WorkflowStatus::WaitingForEvent,
        "Completed" => WorkflowStatus::Completed,
        "Failed" => WorkflowStatus::Failed,
        "Cancelled" => WorkflowStatus::Cancelled,
        _ => WorkflowStatus::Pending,
    }
}

fn workflow_status_to_str(value: &WorkflowStatus) -> &'static str {
    match value {
        WorkflowStatus::Pending => "Pending",
        WorkflowStatus::Running => "Running",
        WorkflowStatus::Sleeping => "Sleeping",
        WorkflowStatus::WaitingForEvent => "WaitingForEvent",
        WorkflowStatus::Completed => "Completed",
        WorkflowStatus::Failed => "Failed",
        WorkflowStatus::Cancelled => "Cancelled",
    }
}

fn step_status_from_str(value: &str) -> StepStatus {
    match value {
        "Running" => StepStatus::Running,
        "Completed" => StepStatus::Completed,
        "Failed" => StepStatus::Failed,
        "Skipped" => StepStatus::Skipped,
        _ => StepStatus::Pending,
    }
}

fn step_status_to_str(value: &StepStatus) -> &'static str {
    match value {
        StepStatus::Pending => "Pending",
        StepStatus::Running => "Running",
        StepStatus::Completed => "Completed",
        StepStatus::Failed => "Failed",
        StepStatus::Skipped => "Skipped",
    }
}

fn parse_stamp_i64(value: &str) -> i64 {
    value
        .trim_end_matches('Z')
        .parse::<u64>()
        .unwrap_or(0)
        .min(i64::MAX as u64) as i64
}

fn stamp(value: i64) -> String {
    format!("{}Z", value.max(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn test_pool() -> Option<Arc<PgPool>> {
        let Ok(url) = std::env::var("PYLON_TEST_PG_URL") else {
            eprintln!("skipping: PYLON_TEST_PG_URL not set");
            return None;
        };
        Some(PgPool::connect(&url, 2, Duration::from_secs(5)).expect("test Postgres pool"))
    }

    fn test_workflow(id: String) -> WorkflowInstance {
        WorkflowInstance {
            id,
            name: "distributed-test".into(),
            input: serde_json::json!({"value": 1}),
            status: WorkflowStatus::Running,
            steps: vec![StepResult {
                step_id: "step_0".into(),
                name: "first".into(),
                status: StepStatus::Completed,
                output: Some(serde_json::json!({"done": true})),
                error: None,
                started_at: Some("1Z".into()),
                completed_at: Some("2Z".into()),
                duration_ms: Some(1000),
                retry_count: 0,
            }],
            output: None,
            error: None,
            created_at: "1Z".into(),
            started_at: Some("1Z".into()),
            completed_at: None,
            wake_at: None,
            waiting_for: None,
            current_step: 1,
            max_retries: 3,
        }
    }

    #[test]
    fn workflow_state_and_steps_survive_and_stale_owners_are_fenced() {
        let Some(pool) = test_pool() else {
            return;
        };
        let suffix = pylon_cluster::new_instance_id();
        let id = format!("wf_test_{suffix}");
        let first = PgWorkflowStore::open(Arc::clone(&pool), format!("first_{suffix}"))
            .expect("first store");
        let second = PgWorkflowStore::open(Arc::clone(&pool), format!("second_{suffix}"))
            .expect("second store");
        let workflow = test_workflow(id.clone());
        first.save(&workflow).expect("initial save");

        let restored = second.load(&id).expect("load").expect("stored workflow");
        assert_eq!(restored.current_step, 1);
        assert_eq!(restored.steps.len(), 1);
        assert_eq!(restored.steps[0].output, workflow.steps[0].output);

        let first_token = format!("first-token-{suffix}");
        let second_token = format!("second-token-{suffix}");
        assert!(first
            .try_acquire(&id, &first_token, 30)
            .expect("first lease"));
        assert!(!second
            .try_acquire(&id, &second_token, 30)
            .expect("contending lease"));
        pool.with_client(|client| {
            client.execute(
                "UPDATE _pylon_workflows SET lease_expires_at=0 WHERE id=$1",
                &[&id],
            )?;
            Ok(())
        })
        .expect("expire workflow lease");
        assert!(second
            .try_acquire(&id, &second_token, 30)
            .expect("recovery lease"));
        assert!(first.save_owned(&workflow, &first_token).is_err());

        let mut completed = workflow.clone();
        completed.status = WorkflowStatus::Completed;
        completed.output = Some(serde_json::json!({"ok": true}));
        completed.completed_at = Some("3Z".into());
        second
            .save_owned(&completed, &second_token)
            .expect("owned transition");
        second.release(&id, &second_token).expect("release");
        assert_eq!(
            first.load(&id).expect("reload").expect("workflow").status,
            WorkflowStatus::Completed
        );

        pool.with_client(|client| {
            client.execute("DELETE FROM _pylon_workflows WHERE id=$1", &[&id])?;
            Ok(())
        })
        .expect("cleanup test workflow");
    }
}
